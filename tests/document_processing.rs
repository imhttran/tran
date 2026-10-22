//! Synthetic PostgreSQL integration tests for durable document processing
//! (FAM-004).
//!
//! The test uses a disposable RAG database and a deterministic fake embedder.
//! It proves idempotent processing, concurrent claim exclusion, failure/retry,
//! stale-approval rejection, and approved-only retrieval.

use std::fs;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use postgres::{Client, NoTls};
use rag_foundation::config::Config;
use rag_foundation::document_lifecycle::{
    create_submission, queue_processing, review, AccessScope, Category, DocumentStorage,
    NewSubmission,
};
use rag_foundation::document_processing::{
    claim_next_job, process_job, renew_lease, retry_job, run_due_jobs, ProcessingError,
};
use rag_foundation::embedding::{Embedder, EmbeddingError};
use rag_foundation::storage::EMBEDDING_DIM;
use rag_foundation::storage_pg::PgStore;

const MIGRATIONS: [&str; 10] = [
    "migrations/001_init.sql",
    "migrations/002_ingestion_metadata.sql",
    "migrations/003_add_language.sql",
    "migrations/004_page.sql",
    "migrations/005_add_corpus.sql",
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
    "migrations/009_document_lifecycle.sql",
    "migrations/010_document_security_hardening.sql",
];

static DB_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn lock() -> MutexGuard<'static, ()> {
    DB_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn dsn() -> Option<String> {
    let dsn = std::env::var("DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    if dsn.is_empty() {
        return None;
    }
    let config = postgres::Config::from_str(&dsn).expect("DATABASE_URL parses");
    let database = config.get_dbname().unwrap_or_default();
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable RAG database {database:?}"
    );
    Some(dsn)
}

fn apply(client: &mut Client) {
    for path in MIGRATIONS {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

fn reset(client: &mut Client) {
    client
        .batch_execute(
            "TRUNCATE documents, document_audit_event, document_processing_job, document_version \
             RESTART IDENTITY CASCADE",
        )
        .expect("reset lifecycle tables");
}

struct FakeEmbedder {
    fail_next: AtomicBool,
}

impl FakeEmbedder {
    fn accepting() -> Self {
        Self {
            fail_next: AtomicBool::new(false),
        }
    }

    fn failing_once() -> Self {
        Self {
            fail_next: AtomicBool::new(true),
        }
    }
}

impl Embedder for FakeEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f64>, EmbeddingError> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(EmbeddingError::NoEmbeddings);
        }
        Ok(vec![0.01; EMBEDDING_DIM])
    }
}

fn config(dsn: &str) -> Config {
    Config::load_from(|key| match key {
        "DATABASE_URL" => Some(dsn.to_string()),
        _ => None,
    })
    .expect("test configuration")
}

fn submission(client: &mut Client, storage: &DocumentStorage, title: &str, body: &str) -> i64 {
    create_submission(
        client,
        storage,
        NewSubmission {
            actor: "editor-subject",
            corpus: "family-docs",
            title,
            description: Some("Synthetic processing fixture"),
            category: Category::Story,
            access_scope: AccessScope::Family,
            owner_subject: Some("editor-subject"),
            owner_issuer: Some("synthetic-issuer"),
            original_filename: "synthetic.md",
            bytes: body.as_bytes(),
            document_key: None,
        },
    )
    .expect("create submission")
    .version_id
}

#[test]
fn processing_is_durable_idempotent_and_approved_only() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL is unset; skipping document processing integration test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect to disposable RAG database");
    apply(&mut client);
    reset(&mut client);
    let store = PgStore::connect(&dsn).expect("retrieval store");
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let config = config(&dsn);

    // Happy path: approval alone is not retrieval; processing publishes.
    let version = submission(
        &mut client,
        &storage,
        "Synthetic Processing",
        "## Synthetic\n\nThis is a synthetic family document for processing tests.\n",
    );
    assert!(store.rows_for_corpus("family-docs").unwrap().is_empty());
    review(
        &mut client,
        version,
        "family-docs",
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve");
    let job = queue_processing(&mut client, version, "family-docs", "admin").expect("queue");
    let outcomes = run_due_jobs(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        "test-worker",
        10,
        Some("family-docs"),
        300,
    )
    .expect("run jobs");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].status, "ready");
    assert_eq!(store.rows_for_corpus("family-docs").unwrap().len(), 1);
    assert_eq!(
        queue_processing(&mut client, version, "family-docs", "admin").expect("idempotent queue"),
        job
    );
    assert!(run_due_jobs(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        "test-worker",
        10,
        Some("family-docs"),
        300,
    )
    .expect("no extra jobs")
    .is_empty());

    // Concurrent claim: exactly one worker wins.
    let concurrent = submission(
        &mut client,
        &storage,
        "Concurrent Processing",
        "## Concurrent\n\nAnother synthetic document.\n",
    );
    review(
        &mut client,
        concurrent,
        "family-docs",
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve concurrent");
    queue_processing(&mut client, concurrent, "family-docs", "admin").expect("queue concurrent");
    let mut first = Client::connect(&dsn, NoTls).expect("first claim client");
    let mut second = Client::connect(&dsn, NoTls).expect("second claim client");
    let one = claim_next_job(&mut first, "worker-a", Some("family-docs"), 300).expect("claim a");
    let two = claim_next_job(&mut second, "worker-b", Some("family-docs"), 300).expect("claim b");
    assert!(one.is_some() ^ two.is_some(), "exactly one claim must win");

    // Failure and explicit retry.
    let failing = submission(
        &mut client,
        &storage,
        "Retry Processing",
        "## Retry\n\nA synthetic document that fails once.\n",
    );
    review(
        &mut client,
        failing,
        "family-docs",
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve failing");
    let failing_job =
        queue_processing(&mut client, failing, "family-docs", "admin").expect("queue failing");
    let failed = run_due_jobs(
        &mut client,
        &store,
        &FakeEmbedder::failing_once(),
        &storage,
        &config,
        "test-worker",
        10,
        Some("family-docs"),
        300,
    )
    .expect("run failing");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].status, "failed");
    assert!(store
        .rows_for_corpus("family-docs")
        .unwrap()
        .iter()
        .all(|row| { row.chunk.source.as_deref() != Some("retry-processing") }));
    retry_job(&mut client, failing_job, "admin", "family-docs").expect("retry");
    let recovered = run_due_jobs(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        "test-worker",
        10,
        Some("family-docs"),
        300,
    )
    .expect("run recovery");
    assert_eq!(recovered[0].status, "ready");

    // A rejection after queueing blocks publication.
    let stale = submission(
        &mut client,
        &storage,
        "Stale Processing",
        "## Stale\n\nA synthetic document whose approval is withdrawn.\n",
    );
    review(
        &mut client,
        stale,
        "family-docs",
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve stale");
    queue_processing(&mut client, stale, "family-docs", "admin").expect("queue stale");
    review(
        &mut client,
        stale,
        "family-docs",
        "admin",
        false,
        Some("withdrawn"),
        None,
    )
    .expect("reject stale");
    // The rejection already invalidated the queued job, so an ordinary run
    // finds nothing to do. Force the same job back to `queued` to prove the
    // worker itself refuses to publish a withdrawn approval, rather than only
    // relying on the invalidation that happened when the review changed.
    client
        .execute(
            "UPDATE document_processing_job
                SET status = 'queued', claimed_by = NULL, claimed_at = NULL,
                    lease_expires_at = NULL, finished_at = NULL
              WHERE version_id = $1",
            &[&stale],
        )
        .expect("requeue stale job");
    let stale_outcomes = run_due_jobs(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        "test-worker",
        10,
        Some("family-docs"),
        300,
    )
    .expect("run stale");
    let stale_outcome = stale_outcomes
        .iter()
        .find(|outcome| outcome.version_id == stale)
        .expect("stale job was attempted");
    assert_eq!(stale_outcome.status, "failed");
    assert_eq!(stale_outcome.error.as_deref(), Some("stale approval"));
    let job_status: String = client
        .query_one(
            "SELECT status FROM document_processing_job WHERE version_id = $1",
            &[&stale],
        )
        .expect("read stale job")
        .get(0);
    assert_eq!(job_status, "failed");
    assert!(store
        .rows_for_corpus("family-docs")
        .unwrap()
        .iter()
        .all(|row| { row.chunk.source.as_deref() != Some("stale-processing") }));
    // An abandoned claim is reclaimable after its lease expires, and the old
    // worker cannot publish after the new worker takes over.
    let leased = submission(
        &mut client,
        &storage,
        "Leased Processing",
        "## Lease\n\nA synthetic document with an abandoned claim.\n",
    );
    review(
        &mut client,
        leased,
        "family-docs",
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve leased");
    let leased_job =
        queue_processing(&mut client, leased, "family-docs", "admin").expect("queue leased");
    let leased_source: String = client
        .query_one(
            "SELECT source FROM document_version WHERE version_id = $1",
            &[&leased],
        )
        .expect("read leased source")
        .get(0);
    let old_job = claim_next_job(&mut client, "worker-old", Some("family-docs"), 300)
        .expect("claim leased")
        .expect("leased job available");
    assert_eq!(old_job.job_id, leased_job);
    client
        .execute(
            "UPDATE document_processing_job
                SET lease_expires_at = now() - interval '1 second'
              WHERE job_id = $1",
            &[&leased_job],
        )
        .expect("expire lease");
    let new_job = claim_next_job(&mut client, "worker-new", Some("family-docs"), 300)
        .expect("reclaim leased")
        .expect("expired lease reclaimable");
    assert_eq!(new_job.job_id, leased_job);
    assert_eq!(new_job.claimed_by, "worker-new");
    let old_outcome = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &old_job,
        "worker-old",
    )
    .expect("old worker attempt");
    assert_eq!(old_outcome.status, "stale");
    assert!(store
        .rows_for_corpus("family-docs")
        .expect("retrieval")
        .iter()
        .all(|row| row.chunk.source.as_deref() != Some(leased_source.as_str())));
    let new_outcome = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &new_job,
        "worker-new",
    )
    .expect("new worker attempt");
    assert_eq!(new_outcome.status, "ready");
    assert!(store
        .rows_for_corpus("family-docs")
        .expect("retrieval")
        .iter()
        .any(|row| row.chunk.source.as_deref() == Some(leased_source.as_str())));

    // A job row carried over from before leases existed has a NULL lease. The
    // current claim holder must still be able to publish it; the claimed_by
    // check, not the lease, is what stops a reclaimed job.
    let legacy_lease = submission(
        &mut client,
        &storage,
        "Null Lease Processing",
        "## Null lease\n\nA synthetic document with no lease timestamp.\n",
    );
    review(
        &mut client,
        legacy_lease,
        "family-docs",
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve null lease");
    queue_processing(&mut client, legacy_lease, "family-docs", "admin").expect("queue null lease");
    let null_lease_job = claim_next_job(&mut client, "worker-lease", Some("family-docs"), 300)
        .expect("claim null lease")
        .expect("null lease job available");
    client
        .execute(
            "UPDATE document_processing_job
                SET lease_expires_at = NULL
              WHERE job_id = $1",
            &[&null_lease_job.job_id],
        )
        .expect("clear lease");
    let null_lease_outcome = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &null_lease_job,
        "worker-lease",
    )
    .expect("null lease attempt");
    assert_eq!(null_lease_outcome.status, "ready");

    // A lease may also expire while a long processing run is still healthy. If
    // no other worker reclaimed the job, the holder must still be able to
    // publish; only a changed claimed_by blocks it.
    let long_run = submission(
        &mut client,
        &storage,
        "Long Run Processing",
        "## Long run\n\nA synthetic document that outlives its lease.\n",
    );
    review(
        &mut client,
        long_run,
        "family-docs",
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve long run");
    queue_processing(&mut client, long_run, "family-docs", "admin").expect("queue long run");
    let long_job = claim_next_job(&mut client, "worker-long", Some("family-docs"), 300)
        .expect("claim long run")
        .expect("long run job available");
    client
        .execute(
            "UPDATE document_processing_job
                SET lease_expires_at = now() - interval '1 second'
              WHERE job_id = $1",
            &[&long_job.job_id],
        )
        .expect("expire long lease");
    let long_outcome = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &long_job,
        "worker-long",
    )
    .expect("long run attempt");
    assert_eq!(long_outcome.status, "ready");
}

fn approval_ready(client: &mut Client, version: i64) {
    review(
        client,
        version,
        "family-docs",
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve version");
}

fn job_row(client: &mut Client, job_id: i64) -> (String, i32, Option<String>) {
    let row = client
        .query_one(
            "SELECT status, attempts, claimed_by FROM document_processing_job WHERE job_id = $1",
            &[&job_id],
        )
        .expect("read job row");
    (row.get(0), row.get(1), row.get(2))
}

fn version_row(client: &mut Client, version_id: i64) -> (String, Option<String>) {
    let row = client
        .query_one(
            "SELECT processing_status, removed_at::text FROM document_version WHERE version_id = $1",
            &[&version_id],
        )
        .expect("read version row");
    (row.get(0), row.get(1))
}

#[test]
fn attempt_fence_blocks_stale_and_unapproved_publication() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL is unset; skipping attempt fence integration test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect to disposable RAG database");
    apply(&mut client);
    reset(&mut client);
    let store = PgStore::connect(&dsn).expect("retrieval store");
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let config = config(&dsn);

    // Two attempts under the same worker name cannot impersonate each other.
    let version = submission(
        &mut client,
        &storage,
        "Fenced Processing",
        "## Fenced\n\nA synthetic document for the attempt fence.\n",
    );
    approval_ready(&mut client, version);
    queue_processing(&mut client, version, "family-docs", "admin").expect("queue fenced");
    let first = claim_next_job(&mut client, "cli-worker", Some("family-docs"), 300)
        .expect("first claim")
        .expect("first claim available");
    assert_eq!(first.claimed_by, "cli-worker");
    // Simulate a reclaim by another process with the identical worker name.
    let mut other = Client::connect(&dsn, NoTls).expect("second worker client");
    other
        .execute(
            "UPDATE document_processing_job
                SET lease_expires_at = now() - interval '1 second'
              WHERE job_id = $1",
            &[&first.job_id],
        )
        .expect("expire first lease");
    let second = claim_next_job(&mut other, "cli-worker", Some("family-docs"), 300)
        .expect("reclaim")
        .expect("reclaimed job available");
    assert_eq!(second.job_id, first.job_id);
    assert_eq!(second.claimed_by, "cli-worker");
    assert_eq!(second.attempts, first.attempts + 1);

    // The earlier attempt cannot renew, publish, or fail after the reclaim.
    assert!(matches!(
        renew_lease(&mut client, &first, 300),
        Err(ProcessingError::ClaimLost)
    ));
    let fenced = submission(
        &mut client,
        &storage,
        "Fenced Publish",
        "## Fenced publish\n\nA synthetic document for the publish fence.\n",
    );
    approval_ready(&mut client, fenced);
    queue_processing(&mut client, fenced, "family-docs", "admin").expect("queue fenced publish");
    let owner = claim_next_job(&mut client, "cli-worker", Some("family-docs"), 300)
        .expect("claim fenced publish")
        .expect("fenced publish claim available");
    let mut attacker = Client::connect(&dsn, NoTls).expect("attacker client");
    attacker
        .execute(
            "UPDATE document_processing_job
                SET lease_expires_at = now() - interval '1 second'
              WHERE job_id = $1",
            &[&owner.job_id],
        )
        .expect("expire owner lease");
    let reclaimer = claim_next_job(&mut attacker, "cli-worker", Some("family-docs"), 300)
        .expect("reclaim fenced publish")
        .expect("reclaimed fenced publish available");
    assert_eq!(reclaimer.job_id, owner.job_id);
    assert_eq!(reclaimer.attempts, owner.attempts + 1);
    let (owner_status, owner_attempts, _) = job_row(&mut client, owner.job_id);
    let (owner_version_status, owner_removed) = version_row(&mut client, fenced);
    let stale_publish = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &owner,
        "cli-worker",
    )
    .expect("stale publish attempt");
    assert_eq!(stale_publish.status, "stale");
    assert_eq!(
        job_row(&mut client, owner.job_id),
        (
            owner_status,
            owner_attempts,
            Some(reclaimer.claimed_by.clone())
        )
    );
    assert_eq!(
        version_row(&mut client, fenced),
        (owner_version_status, owner_removed)
    );
    assert!(store
        .rows_for_corpus("family-docs")
        .unwrap()
        .iter()
        .all(|row| !row
            .chunk
            .source
            .as_deref()
            .unwrap_or_default()
            .contains("fenced-publish")));

    // The current attempt publishes exactly once; a repeat is stale.
    let current = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &reclaimer,
        "cli-worker",
    )
    .expect("current publish attempt");
    assert_eq!(current.status, "ready");
    let ready_jobs: i64 = client
        .query_one(
            "SELECT count(*) FROM document_processing_job
              WHERE job_id = $1 AND status = 'ready'",
            &[&reclaimer.job_id],
        )
        .expect("count ready jobs")
        .get(0);
    assert_eq!(ready_jobs, 1);
    let repeat = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &reclaimer,
        "cli-worker",
    )
    .expect("repeat publish attempt");
    assert_eq!(repeat.status, "stale");

    // The earlier attempt cannot fail the job either.
    let (before_status, before_attempts, before_owner) = job_row(&mut client, first.job_id);
    let stale_failure = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &first,
        "cli-worker",
    )
    .expect("stale failure attempt");
    assert_eq!(stale_failure.status, "stale");
    assert_eq!(
        job_row(&mut client, first.job_id),
        (before_status, before_attempts, before_owner)
    );

    // Healthy lease renewal works for the current attempt only.
    let renewed = submission(
        &mut client,
        &storage,
        "Renewed Processing",
        "## Renewed\n\nA synthetic document for the lease fence.\n",
    );
    approval_ready(&mut client, renewed);
    queue_processing(&mut client, renewed, "family-docs", "admin").expect("queue renewed");
    let held = claim_next_job(&mut client, "cli-worker", Some("family-docs"), 300)
        .expect("claim renewed")
        .expect("renewed claim available");
    renew_lease(&mut client, &held, 900).expect("renew current claim");
    let (_, held_attempts, _) = job_row(&mut client, held.job_id);
    assert_eq!(held_attempts, held.attempts);

    // Non-family readiness states cannot become ready.
    for (label, tamper) in [
        (
            "cancelled",
            "review_status = 'pending_review', approved_sha256 = NULL",
        ),
        (
            "rejected",
            "review_status = 'rejected', approved_sha256 = NULL",
        ),
        (
            "admin_only",
            "access_scope = 'admin_only', approved_scope = 'admin_only'",
        ),
        (
            "superseded",
            "removed_at = now(), removed_by = 'system:superseded'",
        ),
    ] {
        let blocked = submission(
            &mut client,
            &storage,
            &format!("Blocked {label}"),
            &format!("## Blocked\n\nA synthetic {label} document.\n"),
        );
        approval_ready(&mut client, blocked);
        queue_processing(&mut client, blocked, "family-docs", "admin").expect("queue blocked");
        let claim = claim_next_job(&mut client, "cli-worker", Some("family-docs"), 300)
            .expect("claim blocked")
            .expect("blocked claim available");
        client
            .execute(
                &format!("UPDATE document_version SET {tamper} WHERE version_id = $1"),
                &[&blocked],
            )
            .expect("tamper blocked version");
        let outcome = process_job(
            &mut client,
            &store,
            &FakeEmbedder::accepting(),
            &storage,
            &config,
            300,
            &claim,
            "cli-worker",
        )
        .expect("blocked publish attempt");
        assert_eq!(outcome.status, "failed", "{label} must refuse readiness");
        let (state, _) = version_row(&mut client, blocked);
        assert_ne!(state, "ready", "{label} must stay not ready");
    }
}
