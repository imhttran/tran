//! FAM-RAG-02 worker reliability: lease renewal, crash recovery, and
//! per-attempt fencing.
//!
//! Synthetic documents only. Every case uses a disposable `*_test` database and
//! the deterministic fake embedder; no live model and no real family record.

use std::fs;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::Duration;

use postgres::{Client, NoTls};
use rag_foundation::config::Config;
use rag_foundation::document_lifecycle::{
    create_submission, queue_processing, review, AccessScope, Category, DocumentStorage,
    NewSubmission,
};
use rag_foundation::document_processing::{claim_next_job, process_job};
use rag_foundation::embedding::{Embedder, EmbeddingError};
use rag_foundation::storage::EMBEDDING_DIM;
use rag_foundation::storage_pg::PgStore;

const CORPUS: &str = "family-docs";
const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

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
        let sql = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|error| panic!("apply {path}: {error}"));
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

fn config(dsn: &str) -> Config {
    Config::load_from(|key| match key {
        "DATABASE_URL" => Some(dsn.to_string()),
        _ => None,
    })
    .expect("test configuration")
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

/// Blocks the first embedding call until the test releases it.
struct GatedEmbedder {
    calls: AtomicUsize,
    started: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl Embedder for GatedEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f64>, EmbeddingError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.started.wait();
            self.release.wait();
        }
        Ok(vec![0.01; EMBEDDING_DIM])
    }
}

fn submission(client: &mut Client, storage: &DocumentStorage, title: &str, body: &str) -> i64 {
    create_submission(
        client,
        storage,
        NewSubmission {
            actor: "editor-subject",
            corpus: CORPUS,
            title,
            description: Some("Synthetic worker fixture"),
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

fn approve(client: &mut Client, version: i64) {
    review(
        client,
        version,
        CORPUS,
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve version");
}

fn job_state(client: &mut Client, job_id: i64) -> (String, i32, Option<String>, Option<f64>) {
    let row = client
        .query_one(
            "SELECT status, attempts, claimed_by,
                    EXTRACT(EPOCH FROM (lease_expires_at - now()))::float8
               FROM document_processing_job WHERE job_id = $1",
            &[&job_id],
        )
        .expect("read job");
    (row.get(0), row.get(1), row.get(2), row.get(3))
}

fn version_status(client: &mut Client, version: i64) -> String {
    client
        .query_one(
            "SELECT processing_status FROM document_version WHERE version_id = $1",
            &[&version],
        )
        .expect("read version")
        .get(0)
}

fn seed_legacy(client: &mut Client, key: &str, section: &str) -> (i64, i64) {
    let version: i64 = client
        .query_one(
            "INSERT INTO document_version
                 (document_key, version_number, corpus, source, title, category, origin,
                  access_scope, owner_subject, owner_issuer, content_sha256, byte_size,
                  mime_type, original_filename, storage_key, review_status, processing_status,
                  reviewed_by, reviewed_at, approved_sha256, approved_scope,
                  approved_version_number, created_by)
             VALUES ($1, 1, $2, $1, $1, 'legacy', 'legacy_import', 'family', 'legacy',
                     'synthetic-issuer', $3, 0, 'application/octet-stream', $1, NULL,
                     'approved', 'legacy_indexed', 'test', now(), $3, 'family', 1, 'legacy')
             RETURNING version_id",
            &[&key, &CORPUS, &SHA],
        )
        .expect("insert legacy version")
        .get(0);
    let chunk: i64 = client
        .query_one(
            "INSERT INTO documents
                 (content, source, section, chunk_index, embedding, content_hash, corpus,
                  language, document_version_id)
             VALUES ('legacy body', $1, $2, 0,
                     (SELECT array_agg(0.01::float8) FROM generate_series(1, 768))::vector,
                     'legacy-hash', $3, 'english', $4)
             RETURNING id",
            &[&key, &section, &CORPUS, &version],
        )
        .expect("insert legacy chunk")
        .get(0);
    client
        .execute(
            "INSERT INTO document_processing_job
                 (version_id, job_key, status, attempts, max_attempts, created_by)
             VALUES ($1, $2, 'queued', 0, 3, 'test')",
            &[&version, &format!("legacy:{version}")],
        )
        .expect("insert legacy job");
    (version, chunk)
}

#[test]
fn healthy_long_run_renews_and_blocks_reclaim() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping heartbeat test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    apply(&mut client);
    reset(&mut client);
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let version = submission(
        &mut client,
        &storage,
        "Long Renewal",
        "## Long renewal\n\nA synthetic document that outlives its lease.\n",
    );
    approve(&mut client, version);
    queue_processing(&mut client, version, CORPUS, "admin").expect("queue");
    let job = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 3)
        .expect("claim")
        .expect("claim available");

    let started = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let embedder = GatedEmbedder {
        calls: AtomicUsize::new(0),
        started: Arc::clone(&started),
        release: Arc::clone(&release),
    };
    let worker_dsn = dsn.clone();
    let worker_storage_root = root.path().to_path_buf();
    let worker_config = config(&dsn);
    let worker_job = job.clone();
    let handle = thread::spawn(move || {
        let mut worker = Client::connect(&worker_dsn, NoTls).expect("worker connect");
        let store = PgStore::connect(&worker_dsn).expect("worker store");
        let storage = DocumentStorage::new(worker_storage_root);
        process_job(
            &mut worker,
            &store,
            &embedder,
            &storage,
            &worker_config,
            3,
            &worker_job,
            "cli-worker",
        )
    });

    // The embedding call is inside ingestion. Wait past the original 3s lease.
    started.wait();
    thread::sleep(Duration::from_secs(4));
    let mut other = Client::connect(&dsn, NoTls).expect("second worker");
    let reclaimed =
        claim_next_job(&mut other, "cli-worker", Some(CORPUS), 3).expect("second claim");
    assert!(
        reclaimed.is_none(),
        "a healthy renewed job must not be reclaimable"
    );
    let (state, attempts, _, remaining) = job_state(&mut other, job.job_id);
    assert_eq!(state, "processing");
    assert_eq!(attempts, job.attempts);
    assert!(
        remaining.unwrap_or(-1.0) > 1.0,
        "the lease must have been renewed recently (remaining {remaining:?})"
    );

    release.wait();
    let outcome = handle.join().expect("worker thread").expect("processed");
    assert_eq!(outcome.status, "ready");
    assert_eq!(version_status(&mut client, version), "ready");
}

#[test]
fn abandoned_lease_is_reclaimable() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping crash recovery test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    apply(&mut client);
    reset(&mut client);
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let version = submission(
        &mut client,
        &storage,
        "Crashed Worker",
        "## Crashed\n\nA synthetic abandoned job.\n",
    );
    approve(&mut client, version);
    queue_processing(&mut client, version, CORPUS, "admin").expect("queue");
    let first = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 3)
        .expect("claim")
        .expect("claim available");
    // No heartbeat: simulate a crashed worker whose lease expires.
    client
        .execute(
            "UPDATE document_processing_job
                SET lease_expires_at = now() - interval '1 second'
              WHERE job_id = $1",
            &[&first.job_id],
        )
        .expect("expire lease");
    let second = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 3)
        .expect("reclaim")
        .expect("abandoned job must be reclaimable");
    assert_eq!(second.job_id, first.job_id);
    assert_eq!(second.attempts, first.attempts + 1);
}

#[test]
fn heartbeat_stops_after_failure() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping heartbeat shutdown test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    apply(&mut client);
    reset(&mut client);
    let store = PgStore::connect(&dsn).expect("store");
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let config = config(&dsn);
    let version = submission(
        &mut client,
        &storage,
        "Failed Worker",
        "## Failed\n\nA synthetic failing job.\n",
    );
    approve(&mut client, version);
    queue_processing(&mut client, version, CORPUS, "admin").expect("queue");
    let job = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 3)
        .expect("claim")
        .expect("claim available");
    let outcome = process_job(
        &mut client,
        &store,
        &FakeEmbedder::failing_once(),
        &storage,
        &config,
        3,
        &job,
        "cli-worker",
    )
    .expect("process failure");
    assert_eq!(outcome.status, "failed");
    let (state, _, _, lease) = job_state(&mut client, job.job_id);
    assert_eq!(state, "failed");
    assert!(lease.is_none(), "a finished job must not hold a lease");
    // If the heartbeat thread were still alive it would renew the lease.
    thread::sleep(Duration::from_secs(2));
    let (state, _, _, lease) = job_state(&mut client, job.job_id);
    assert_eq!(state, "failed");
    assert!(lease.is_none(), "no heartbeat may outlive the job");
    assert_ne!(version_status(&mut client, version), "ready");
}

#[test]
fn legacy_fast_path_respects_approval_and_fence() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping legacy path test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    apply(&mut client);
    reset(&mut client);
    let store = PgStore::connect(&dsn).expect("store");
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let config = config(&dsn);

    // Control: an approved legacy version becomes ready.
    let (version, _) = seed_legacy(&mut client, "legacy-control", "Control");
    let job = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 3)
        .expect("claim legacy")
        .expect("legacy claim available");
    let outcome = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        3,
        &job,
        "cli-worker",
    )
    .expect("legacy control");
    assert_eq!(outcome.status, "ready");
    assert_eq!(version_status(&mut client, version), "ready");

    // A removed legacy version is never marked ready.
    let (blocked, _) = seed_legacy(&mut client, "legacy-removed", "Removed");
    let blocked_job = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 3)
        .expect("claim removed")
        .expect("removed claim available");
    client
        .execute(
            "UPDATE document_version SET removed_at = now() WHERE version_id = $1",
            &[&blocked],
        )
        .expect("remove legacy version");
    let outcome = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        3,
        &blocked_job,
        "cli-worker",
    )
    .expect("legacy removed");
    assert_eq!(outcome.status, "failed");
    assert_ne!(version_status(&mut client, blocked), "ready");

    // A stale attempt cannot mark a reclaimed legacy job ready.
    let (fenced, _) = seed_legacy(&mut client, "legacy-fenced", "Fenced");
    let first = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 3)
        .expect("claim fenced")
        .expect("fenced claim available");
    client
        .execute(
            "UPDATE document_processing_job
                SET lease_expires_at = now() - interval '1 second'
              WHERE job_id = $1",
            &[&first.job_id],
        )
        .expect("expire fenced lease");
    let second = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 3)
        .expect("reclaim fenced")
        .expect("fenced reclaim available");
    let stale = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        3,
        &first,
        "cli-worker",
    )
    .expect("stale legacy");
    assert_eq!(stale.status, "stale");
    assert_ne!(version_status(&mut client, fenced), "ready");
    let current = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        3,
        &second,
        "cli-worker",
    )
    .expect("current legacy");
    assert_eq!(current.status, "ready");
    assert_eq!(version_status(&mut client, fenced), "ready");
}

fn source_of(client: &mut Client, version: i64) -> String {
    client
        .query_one(
            "SELECT source FROM document_version WHERE version_id = $1",
            &[&version],
        )
        .expect("read source")
        .get(0)
}

/// Returns `(chunk_id, document_version_id, content)` for one source.
fn documents_for_source(client: &mut Client, source: &str) -> Vec<(i64, Option<i64>, String)> {
    client
        .query(
            "SELECT id, document_version_id, content FROM documents
              WHERE corpus = $1 AND source = $2 ORDER BY id",
            &[&CORPUS, &source],
        )
        .expect("read documents")
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect()
}

#[test]
fn stale_attempt_cannot_replace_before_ingestion() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping pre-ingestion fence test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    apply(&mut client);
    reset(&mut client);
    let store = PgStore::connect(&dsn).expect("store");
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let config = config(&dsn);

    let version = submission(
        &mut client,
        &storage,
        "Pre Ingestion Fence",
        "## Fence\n\nA synthetic document for the pre-ingestion fence.\n",
    );
    approve(&mut client, version);
    queue_processing(&mut client, version, CORPUS, "admin").expect("queue");
    let source = source_of(&mut client, version);

    let first = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 300)
        .expect("claim")
        .expect("claim available");
    client
        .execute(
            "UPDATE document_processing_job
                SET lease_expires_at = now() - interval '1 second'
              WHERE job_id = $1",
            &[&first.job_id],
        )
        .expect("expire lease");
    let second = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 300)
        .expect("reclaim")
        .expect("reclaim available");
    assert_eq!(second.job_id, first.job_id);
    assert_eq!(second.attempts, first.attempts + 1);

    // The stale attempt must not reach ingestion at all.
    let stale = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &first,
        "cli-worker",
    )
    .expect("stale attempt");
    assert_eq!(stale.status, "stale");
    assert!(
        documents_for_source(&mut client, &source).is_empty(),
        "a stale attempt must not replace source chunks"
    );
    assert_ne!(version_status(&mut client, version), "ready");

    // The current attempt completes normally.
    let current = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &second,
        "cli-worker",
    )
    .expect("current attempt");
    assert_eq!(current.status, "ready");
    assert_eq!(version_status(&mut client, version), "ready");
    let published = documents_for_source(&mut client, &source);
    assert!(
        !published.is_empty(),
        "the current attempt must publish chunks"
    );
    assert!(
        published
            .iter()
            .all(|(_, linked, _)| *linked == Some(version)),
        "published chunks must be linked to the approved version"
    );
    assert_eq!(
        store.rows_for_corpus(CORPUS).expect("retrieval").len(),
        published.len()
    );
}

#[test]
fn claim_lost_during_embedding_cannot_replace_published_chunks() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping replacement fence test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    apply(&mut client);
    reset(&mut client);
    let store = PgStore::connect(&dsn).expect("store");
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let service_config = config(&dsn);

    let version = submission(
        &mut client,
        &storage,
        "Embedding Fence",
        "## Embedding fence\n\nA synthetic document for the replacement fence.\n",
    );
    approve(&mut client, version);
    queue_processing(&mut client, version, CORPUS, "admin").expect("queue");
    let source = source_of(&mut client, version);

    // A claims with a long lease so its heartbeat cannot fire during the test.
    let first = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 300)
        .expect("claim")
        .expect("claim available");

    let started = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let embedder = GatedEmbedder {
        calls: AtomicUsize::new(0),
        started: Arc::clone(&started),
        release: Arc::clone(&release),
    };
    let worker_dsn = dsn.clone();
    let worker_root = root.path().to_path_buf();
    let worker_config = config(&dsn);
    let worker_job = first.clone();
    let handle = thread::spawn(move || {
        let mut worker = Client::connect(&worker_dsn, NoTls).expect("worker connect");
        let store = PgStore::connect(&worker_dsn).expect("worker store");
        let storage = DocumentStorage::new(worker_root);
        process_job(
            &mut worker,
            &store,
            &embedder,
            &storage,
            &worker_config,
            300,
            &worker_job,
            "cli-worker",
        )
    });

    // A is now blocked after its precheck and before source replacement.
    started.wait();

    // Expire and reclaim as a second process with the same worker name.
    client
        .execute(
            "UPDATE document_processing_job
                SET lease_expires_at = now() - interval '1 second'
              WHERE job_id = $1",
            &[&first.job_id],
        )
        .expect("expire first lease");
    let mut second_client = Client::connect(&dsn, NoTls).expect("second connect");
    let second = claim_next_job(&mut second_client, "cli-worker", Some(CORPUS), 300)
        .expect("reclaim")
        .expect("reclaim available");
    assert_eq!(second.job_id, first.job_id);
    assert_eq!(second.attempts, first.attempts + 1);

    // The current attempt finishes and publishes.
    let current = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &service_config,
        300,
        &second,
        "cli-worker",
    )
    .expect("current attempt");
    assert_eq!(current.status, "ready");
    assert_eq!(version_status(&mut client, version), "ready");
    let published = documents_for_source(&mut client, &source);
    assert!(!published.is_empty());
    assert!(
        published
            .iter()
            .all(|(_, linked, _)| *linked == Some(version)),
        "the current attempt's chunks must be linked"
    );

    // Release the stale attempt. Its ingestion must not replace the published
    // chunks, and it must not change the current attempt's ready state.
    release.wait();
    let stale = handle
        .join()
        .expect("worker thread")
        .expect("stale attempt");
    assert_eq!(stale.status, "stale");
    let after = documents_for_source(&mut client, &source);
    assert_eq!(
        after, published,
        "a stale attempt must not delete or replace published chunks"
    );
    let (state, attempts, owner, lease) = job_state(&mut client, second.job_id);
    assert_eq!(state, "ready");
    assert_eq!(attempts, second.attempts);
    assert_eq!(owner, Some(second.claimed_by.clone()));
    assert!(lease.is_none());
    assert_eq!(
        store.rows_for_corpus(CORPUS).expect("retrieval").len(),
        published.len()
    );
}

#[test]
fn crash_and_retry_preserves_published_content() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping crash retry test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    apply(&mut client);
    reset(&mut client);
    let store = PgStore::connect(&dsn).expect("store");
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let config = config(&dsn);

    let version = submission(
        &mut client,
        &storage,
        "Crash Retry",
        "## Crash retry\n\nA synthetic document for crash recovery.\n",
    );
    approve(&mut client, version);
    queue_processing(&mut client, version, CORPUS, "admin").expect("queue");
    let source = source_of(&mut client, version);

    let first = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 300)
        .expect("claim")
        .expect("claim available");
    // Simulate an abandoned attempt that wrote unfinished, unlinked chunks.
    client
        .execute(
            "INSERT INTO documents
                 (content, source, section, chunk_index, embedding, content_hash, corpus,
                  language, document_version_id)
             VALUES ('unfinished chunk', $1, 'Partial', 0,
                     (SELECT array_agg(0.02::float8) FROM generate_series(1, 768))::vector,
                     'partial-hash', $2, 'english', NULL)",
            &[&source, &CORPUS],
        )
        .expect("insert unfinished chunk");
    assert!(
        store.rows_for_corpus(CORPUS).expect("retrieval").is_empty(),
        "unfinished chunks must not be retrievable"
    );
    client
        .execute(
            "UPDATE document_processing_job
                SET lease_expires_at = now() - interval '1 second'
              WHERE job_id = $1",
            &[&first.job_id],
        )
        .expect("expire lease");

    let retry = claim_next_job(&mut client, "cli-worker", Some(CORPUS), 300)
        .expect("reclaim")
        .expect("reclaim available");
    let outcome = process_job(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        300,
        &retry,
        "cli-worker",
    )
    .expect("retry");
    assert_eq!(outcome.status, "ready");
    let published = documents_for_source(&mut client, &source);
    assert!(!published.is_empty());
    assert!(
        published
            .iter()
            .all(|(_, linked, content)| *linked == Some(version) && content != "unfinished chunk"),
        "the retry must replace unfinished chunks with linked, processed chunks"
    );
    assert_eq!(
        store.rows_for_corpus(CORPUS).expect("retrieval").len(),
        published.len(),
        "retrieval must expose exactly the final chunk set"
    );
}
