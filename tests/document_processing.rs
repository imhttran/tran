//! Synthetic PostgreSQL integration tests for durable document processing
//! (FAM-004).
//!
//! The test uses a disposable RAG database and a deterministic fake embedder.
//! It proves idempotent processing, concurrent claim exclusion, failure/retry,
//! stale-approval rejection, and approved-only retrieval.

use std::fs;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

use postgres::{Client, NoTls};
use rag_foundation::config::Config;
use rag_foundation::document_lifecycle::{
    create_submission, queue_processing, review, AccessScope, Category, DocumentStorage,
    NewSubmission,
};
use rag_foundation::document_processing::{claim_next_job, retry_job, run_due_jobs};
use rag_foundation::embedding::{Embedder, EmbeddingError};
use rag_foundation::storage::EMBEDDING_DIM;
use rag_foundation::storage_pg::PgStore;

const MIGRATIONS: [&str; 9] = [
    "migrations/001_init.sql",
    "migrations/002_ingestion_metadata.sql",
    "migrations/003_add_language.sql",
    "migrations/004_page.sql",
    "migrations/005_add_corpus.sql",
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
    "migrations/009_document_lifecycle.sql",
];

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
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve");
    let job = queue_processing(&mut client, version, "admin").expect("queue");
    let outcomes = run_due_jobs(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        "test-worker",
        10,
    )
    .expect("run jobs");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].status, "ready");
    assert_eq!(store.rows_for_corpus("family-docs").unwrap().len(), 1);
    assert_eq!(
        queue_processing(&mut client, version, "admin").expect("idempotent queue"),
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
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve concurrent");
    queue_processing(&mut client, concurrent, "admin").expect("queue concurrent");
    let mut first = Client::connect(&dsn, NoTls).expect("first claim client");
    let mut second = Client::connect(&dsn, NoTls).expect("second claim client");
    let one = claim_next_job(&mut first, "worker-a").expect("claim a");
    let two = claim_next_job(&mut second, "worker-b").expect("claim b");
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
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve failing");
    let failing_job = queue_processing(&mut client, failing, "admin").expect("queue failing");
    let failed = run_due_jobs(
        &mut client,
        &store,
        &FakeEmbedder::failing_once(),
        &storage,
        &config,
        "test-worker",
        10,
    )
    .expect("run failing");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].status, "failed");
    assert!(store
        .rows_for_corpus("family-docs")
        .unwrap()
        .iter()
        .all(|row| { row.chunk.source.as_deref() != Some("retry-processing") }));
    retry_job(&mut client, failing_job, "admin").expect("retry");
    let recovered = run_due_jobs(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        "test-worker",
        10,
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
        "admin",
        true,
        Some("approved"),
        Some(AccessScope::Family),
    )
    .expect("approve stale");
    queue_processing(&mut client, stale, "admin").expect("queue stale");
    review(&mut client, stale, "admin", false, Some("withdrawn"), None).expect("reject stale");
    let stale_outcomes = run_due_jobs(
        &mut client,
        &store,
        &FakeEmbedder::accepting(),
        &storage,
        &config,
        "test-worker",
        10,
    )
    .expect("run stale");
    let stale_outcome = stale_outcomes
        .iter()
        .find(|outcome| outcome.version_id == stale)
        .expect("stale job attempted");
    assert_eq!(stale_outcome.status, "failed");
    assert!(store
        .rows_for_corpus("family-docs")
        .unwrap()
        .iter()
        .all(|row| { row.chunk.source.as_deref() != Some("stale-processing") }));
}
