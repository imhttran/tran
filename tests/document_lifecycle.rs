//! Synthetic PostgreSQL integration tests for the family document lifecycle.
//!
//! The test writes only to a disposable RAG database whose name ends in
//! `_test`. It proves additive migration behavior, legacy quarantine, explicit
//! approval binding, approved-only retrieval, and private original storage.

use std::fs;
use std::str::FromStr;

use postgres::{Client, NoTls};
use rag_foundation::document_lifecycle::{
    change_scope, create_submission, get_version, legacy_inventory, promote_legacy,
    queue_processing, read_original, remove, review, AccessScope, Category, DocumentStorage,
    LifecycleError, NewSubmission,
};
use rag_foundation::storage_pg::{PgStore, SectionKey};

const PRE_LIFECYCLE: [&str; 8] = [
    "migrations/001_init.sql",
    "migrations/002_ingestion_metadata.sql",
    "migrations/003_add_language.sql",
    "migrations/004_page.sql",
    "migrations/005_add_corpus.sql",
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
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

fn apply(client: &mut Client, paths: &[&str]) {
    for path in paths {
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

fn insert_legacy(
    client: &mut Client,
    corpus: &str,
    source: &str,
    content: &str,
    chunk_index: i32,
    content_hash: &str,
) -> i64 {
    let row = client
        .query_one(
            "INSERT INTO documents
                 (content, source, section, chunk_index, embedding, content_hash, corpus)
             VALUES ($1, $2, 'Section', $3,
                     (SELECT array_agg(0.1::float8) FROM generate_series(1, 768))::vector,
                     $4, $5)
             RETURNING id",
            &[&content, &source, &chunk_index, &content_hash, &corpus],
        )
        .expect("insert legacy document");
    row.get(0)
}

fn insert_chunk_for_version(
    client: &mut Client,
    corpus: &str,
    source: &str,
    version_id: i64,
    content: &str,
    content_hash: &str,
) {
    client
        .execute(
            "INSERT INTO documents
                 (content, source, section, chunk_index, embedding, content_hash, corpus,
                  document_version_id)
             VALUES ($1, $2, 'Section', 0,
                     (SELECT array_agg(0.2::float8) FROM generate_series(1, 768))::vector,
                     $3, $4, $5)",
            &[&content, &source, &content_hash, &corpus, &version_id],
        )
        .expect("insert document chunk for version");
}

fn mark_ready(client: &mut Client, version_id: i64) {
    client
        .execute(
            "UPDATE document_version SET processing_status = 'ready' WHERE version_id = $1",
            &[&version_id],
        )
        .expect("mark version ready");
    client
        .execute(
            "UPDATE document_processing_job SET status = 'ready', finished_at = now()
              WHERE version_id = $1",
            &[&version_id],
        )
        .expect("mark job ready");
}

#[test]
fn lifecycle_is_additive_quarantined_and_approved_only() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL is unset; skipping document lifecycle integration test");
        return;
    };
    let mut client = Client::connect(&dsn, NoTls).expect("connect to disposable RAG database");
    apply(&mut client, &PRE_LIFECYCLE);
    apply(&mut client, &["migrations/009_document_lifecycle.sql"]);
    reset(&mut client);

    // Legacy rows: two chunks for one source and one for another.
    let legacy_one = insert_legacy(
        &mut client,
        "legacy-corpus",
        "legacy-source",
        "Legacy one",
        0,
        "legacy-hash-1",
    );
    let legacy_two = insert_legacy(
        &mut client,
        "legacy-corpus",
        "legacy-source",
        "Legacy two",
        1,
        "legacy-hash-1",
    );
    let legacy_other = insert_legacy(
        &mut client,
        "legacy-corpus",
        "other-source",
        "Other",
        0,
        "legacy-hash-2",
    );
    let before: Vec<(i64, String, Option<String>, Option<String>)> = client
        .query(
            "SELECT id, content, source, corpus FROM documents ORDER BY id",
            &[],
        )
        .expect("read legacy rows")
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect();
    assert_eq!(before.len(), 3);

    // The additive migration quarantines existing groups without implicit
    // approval or content changes.
    apply(&mut client, &["migrations/009_document_lifecycle.sql"]);
    let after: Vec<(i64, String, Option<String>, Option<String>)> = client
        .query(
            "SELECT id, content, source, corpus FROM documents ORDER BY id",
            &[],
        )
        .expect("read rows after migration")
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect();
    assert_eq!(before, after, "migration must not rewrite documents");
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM document_version WHERE origin = 'legacy_import'",
                &[]
            )
            .unwrap()
            .get::<_, i64>(0),
        2,
        "one quarantine version per legacy (corpus, source) group"
    );
    let legacy_versions = client
        .query(
            "SELECT version_id, review_status, processing_status, approved_sha256, access_scope
               FROM document_version WHERE origin = 'legacy_import' ORDER BY version_id",
            &[],
        )
        .expect("read legacy versions");
    for row in &legacy_versions {
        assert_eq!(row.get::<_, String>(1), "pending_review");
        assert_eq!(row.get::<_, String>(2), "legacy_indexed");
        assert_eq!(row.get::<_, Option<String>>(3), None);
        assert_eq!(row.get::<_, Option<String>>(4), None);
    }
    for id in [legacy_one, legacy_two, legacy_other] {
        let version_id: Option<i64> = client
            .query_one(
                "SELECT document_version_id FROM documents WHERE id = $1",
                &[&id],
            )
            .unwrap()
            .get(0);
        assert!(
            version_id.is_some(),
            "legacy chunk linked to quarantine version"
        );
    }

    let store = PgStore::connect(&dsn).expect("retrieval store");
    assert!(store.rows_for_corpus("legacy-corpus").unwrap().is_empty());
    let approved_count: i64 = client
        .query_one("SELECT count(*) FROM approved_documents", &[])
        .unwrap()
        .get(0);
    assert_eq!(approved_count, 0);

    // Explicit promotion of the legacy version makes its preserved chunks
    // retrievable; removal takes them out again.
    let legacy_id = legacy_versions[0].get::<_, i64>(0);
    review(
        &mut client,
        legacy_id,
        "admin",
        true,
        Some("promote legacy"),
        Some(AccessScope::Family),
    )
    .expect("approve legacy");
    promote_legacy(&mut client, legacy_id, "admin").expect("promote legacy");
    assert_eq!(store.rows_for_corpus("legacy-corpus").unwrap().len(), 2);
    remove(&mut client, legacy_id, "admin").expect("remove legacy");
    assert!(store.rows_for_corpus("legacy-corpus").unwrap().is_empty());

    // A new submission starts pending and writes no chunks.
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let bytes = b"# Synthetic family document\n\nA synthetic paragraph.";
    let submission = create_submission(
        &mut client,
        &storage,
        NewSubmission {
            actor: "editor-subject",
            corpus: "family-docs",
            title: "Synthetic History",
            description: Some("Synthetic fixture"),
            category: Category::Story,
            access_scope: AccessScope::Family,
            owner_subject: Some("editor-subject"),
            original_filename: "synthetic.md",
            bytes,
            document_key: None,
        },
    )
    .expect("create submission");
    assert_eq!(submission.review_status, "pending_review");
    assert_eq!(submission.processing_status, "unprocessed");
    let (_, original) =
        read_original(&mut client, &storage, submission.version_id).expect("read original");
    assert_eq!(original, bytes);
    insert_chunk_for_version(
        &mut client,
        "family-docs",
        &submission.source,
        submission.version_id,
        "Synthetic chunk",
        "synthetic-hash",
    );
    assert!(store.rows_for_corpus("family-docs").unwrap().is_empty());
    assert!(store
        .vector_search("family-docs", &vec![0.2; 768], 5)
        .unwrap()
        .is_empty());

    // Approval binds the content and scope; queueing is idempotent.
    let approved = review(
        &mut client,
        submission.version_id,
        "admin",
        true,
        Some("looks good"),
        Some(AccessScope::Family),
    )
    .expect("approve");
    assert_eq!(approved.review_status, "approved");
    let job_one = queue_processing(&mut client, submission.version_id, "admin").expect("queue");
    let job_two =
        queue_processing(&mut client, submission.version_id, "admin").expect("queue again");
    assert_eq!(job_one, job_two);
    mark_ready(&mut client, submission.version_id);
    assert_eq!(store.rows_for_corpus("family-docs").unwrap().len(), 1);
    assert_eq!(
        store
            .vector_search("family-docs", &vec![0.2; 768], 5)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .section_chunks(
                "family-docs",
                &[SectionKey {
                    source: submission.source.clone(),
                    section: "Section".to_string(),
                }],
                10,
            )
            .unwrap()
            .len(),
        1
    );

    // A scope change invalidates approval and immediately removes retrieval.
    let changed = change_scope(
        &mut client,
        submission.version_id,
        "admin",
        AccessScope::AdminOnly,
    )
    .expect("change scope");
    assert_eq!(changed.review_status, "pending_review");
    assert!(store.rows_for_corpus("family-docs").unwrap().is_empty());

    // Rejection cannot be queued for processing.
    let rejected = review(
        &mut client,
        submission.version_id,
        "admin",
        false,
        Some("not suitable"),
        None,
    )
    .expect("reject");
    assert_eq!(rejected.review_status, "rejected");
    assert_eq!(
        queue_processing(&mut client, submission.version_id, "admin"),
        Err(LifecycleError::NotApproved)
    );

    // The legacy inventory is read-only and reports only synthetic counts.
    let inventory = legacy_inventory(&mut client).expect("inventory");
    assert!(inventory
        .iter()
        .any(|row| row.corpus == "legacy-corpus" && row.chunks == 2));

    let final_version = get_version(&mut client, submission.version_id).expect("version");
    assert_eq!(final_version.review_status, "rejected");
}
