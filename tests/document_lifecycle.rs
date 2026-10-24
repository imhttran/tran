//! Synthetic PostgreSQL integration tests for the family document lifecycle.
//!
//! The test writes only to a disposable RAG database whose name ends in
//! `_test`. It proves additive migration behavior, legacy quarantine, explicit
//! approval binding, approved-only retrieval, and private original storage.

use std::fs;
use std::str::FromStr;

use postgres::{Client, NoTls};
use rag_foundation::document_lifecycle::{
    begin_operator_import, change_scope, create_submission, fail_operator_import,
    finish_operator_import, get_version, legacy_inventory, list_submissions, promote_legacy,
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
    apply(
        &mut client,
        &[
            "migrations/009_document_lifecycle.sql",
            "migrations/010_document_security_hardening.sql",
        ],
    );
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
    apply(
        &mut client,
        &[
            "migrations/009_document_lifecycle.sql",
            "migrations/010_document_security_hardening.sql",
        ],
    );
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
        "legacy-corpus",
        "admin",
        true,
        Some("promote legacy"),
        Some(AccessScope::Family),
    )
    .expect("approve legacy");
    promote_legacy(&mut client, legacy_id, "legacy-corpus", "admin").expect("promote legacy");
    assert_eq!(store.rows_for_corpus("legacy-corpus").unwrap().len(), 2);
    remove(&mut client, legacy_id, "legacy-corpus", "admin").expect("remove legacy");
    assert!(store.rows_for_corpus("legacy-corpus").unwrap().is_empty());

    // A new submission starts pending and writes no chunks.
    let root = tempfile::tempdir().expect("temp document root");
    let storage = DocumentStorage::new(root.path());
    let bytes = b"## Synthetic family document\n\nA synthetic paragraph.";
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
            owner_issuer: Some("synthetic-issuer"),
            original_filename: "synthetic.md",
            bytes,
            document_key: None,
        },
    )
    .expect("create submission");
    assert_eq!(submission.review_status, "pending_review");

    // A Markdown original the pipeline could never index is refused at
    // submission, before it can enter the review queue. The loader only treats
    // "## " as a section boundary, so an H1-only file would yield no chunks and
    // fail later as an opaque "extraction rejected".
    let before: i64 = client
        .query_one("SELECT count(*) FROM document_version", &[])
        .expect("count versions")
        .get(0);
    let flat = create_submission(
        &mut client,
        &storage,
        NewSubmission {
            actor: "editor-subject",
            corpus: "family-docs",
            title: "Flat note",
            description: None,
            category: Category::Story,
            access_scope: AccessScope::Family,
            owner_subject: Some("editor-subject"),
            owner_issuer: Some("synthetic-issuer"),
            original_filename: "flat.md",
            bytes: b"# Flat note\n\nA paragraph with no section heading.",
            document_key: None,
        },
    );
    assert!(matches!(flat, Err(LifecycleError::NotIndexable(_))));
    let after: i64 = client
        .query_one("SELECT count(*) FROM document_version", &[])
        .expect("count versions")
        .get(0);
    assert_eq!(after, before, "a rejected original must create no version");
    assert_eq!(submission.processing_status, "unprocessed");
    let (_, original) = read_original(&mut client, &storage, submission.version_id, "family-docs")
        .expect("read original");
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
        "family-docs",
        "admin",
        true,
        Some("looks good"),
        Some(AccessScope::Family),
    )
    .expect("approve");
    assert_eq!(approved.review_status, "approved");
    let job_one = queue_processing(&mut client, submission.version_id, "family-docs", "admin")
        .expect("queue");
    let job_two = queue_processing(&mut client, submission.version_id, "family-docs", "admin")
        .expect("queue again");
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
        "family-docs",
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
        "family-docs",
        "admin",
        false,
        Some("not suitable"),
        None,
    )
    .expect("reject");
    assert_eq!(rejected.review_status, "rejected");
    assert_eq!(
        queue_processing(&mut client, submission.version_id, "family-docs", "admin"),
        Err(LifecycleError::NotApproved)
    );

    // The legacy inventory is read-only and reports only synthetic counts.
    let inventory = legacy_inventory(&mut client, "legacy-corpus").expect("inventory");
    assert!(inventory
        .iter()
        .any(|row| row.corpus == "legacy-corpus" && row.chunks == 2));

    let final_version =
        get_version(&mut client, submission.version_id, "family-docs").expect("version");
    assert_eq!(final_version.review_status, "rejected");
    // Cross-corpus lifecycle operations cannot reach or mutate a foreign version.
    let other_corpus = create_submission(
        &mut client,
        &storage,
        NewSubmission {
            actor: "editor-subject",
            corpus: "other-corpus",
            title: "Other Corpus",
            description: None,
            category: Category::Other,
            access_scope: AccessScope::Family,
            owner_subject: Some("editor-subject"),
            owner_issuer: Some("synthetic-issuer"),
            original_filename: "other.md",
            bytes: b"## Other\n\nSynthetic other corpus.",
            document_key: None,
        },
    )
    .expect("other corpus submission");
    assert_eq!(
        get_version(&mut client, other_corpus.version_id, "family-docs").unwrap_err(),
        LifecycleError::NotFound
    );
    assert_eq!(
        review(
            &mut client,
            other_corpus.version_id,
            "family-docs",
            "admin",
            true,
            Some("wrong corpus"),
            Some(AccessScope::Family),
        )
        .unwrap_err(),
        LifecycleError::NotFound
    );
    assert_eq!(
        queue_processing(&mut client, other_corpus.version_id, "family-docs", "admin").unwrap_err(),
        LifecycleError::NotFound
    );
    assert_eq!(
        remove(&mut client, other_corpus.version_id, "family-docs", "admin").unwrap_err(),
        LifecycleError::NotFound
    );
    assert!(
        list_submissions(&mut client, "editor-subject", false, "family-docs")
            .expect("list family corpus")
            .iter()
            .all(|version| version.version_id != other_corpus.version_id)
    );
    assert!(legacy_inventory(&mut client, "other-corpus")
        .expect("other corpus inventory")
        .is_empty());

    // Approval cannot silently change an admin-only version to family.
    let admin_doc = create_submission(
        &mut client,
        &storage,
        NewSubmission {
            actor: "editor-subject",
            corpus: "family-docs",
            title: "Admin Only",
            description: None,
            category: Category::Other,
            access_scope: AccessScope::AdminOnly,
            owner_subject: Some("editor-subject"),
            owner_issuer: Some("synthetic-issuer"),
            original_filename: "admin-only.md",
            bytes: b"## Admin\n\nSynthetic admin-only document.",
            document_key: None,
        },
    )
    .expect("admin-only submission");
    assert_eq!(
        review(
            &mut client,
            admin_doc.version_id,
            "family-docs",
            "admin",
            true,
            Some("silent scope change"),
            Some(AccessScope::Family),
        )
        .unwrap_err(),
        LifecycleError::ScopeChange
    );
    let admin_approved = review(
        &mut client,
        admin_doc.version_id,
        "family-docs",
        "admin",
        true,
        Some("keep admin-only"),
        Some(AccessScope::AdminOnly),
    )
    .expect("approve admin-only");
    assert_eq!(admin_approved.access_scope.as_deref(), Some("admin_only"));

    // Two submissions with the same filename and bytes but different titles get
    // distinct full document identities and chunk sources.
    let source_a = create_submission(
        &mut client,
        &storage,
        NewSubmission {
            actor: "editor-subject",
            corpus: "family-docs",
            title: "Source A",
            description: None,
            category: Category::Other,
            access_scope: AccessScope::Family,
            owner_subject: Some("editor-subject"),
            owner_issuer: Some("synthetic-issuer"),
            original_filename: "same.md",
            bytes: b"## Same\n\nSame bytes.",
            document_key: None,
        },
    )
    .expect("source a");
    let source_b = create_submission(
        &mut client,
        &storage,
        NewSubmission {
            actor: "editor-subject",
            corpus: "family-docs",
            title: "Source B",
            description: None,
            category: Category::Other,
            access_scope: AccessScope::Family,
            owner_subject: Some("editor-subject"),
            owner_issuer: Some("synthetic-issuer"),
            original_filename: "same.md",
            bytes: b"## Same\n\nSame bytes.",
            document_key: None,
        },
    )
    .expect("source b");
    assert_ne!(source_a.document_key, source_b.document_key);
    assert_ne!(source_a.source, source_b.source);

    // A client-supplied document key is normalized to the generated slug
    // alphabet, so it cannot forge a version suffix or another source shape.
    let crafted = create_submission(
        &mut client,
        &storage,
        NewSubmission {
            actor: "editor-subject",
            corpus: "family-docs",
            title: "Crafted",
            description: None,
            category: Category::Story,
            access_scope: AccessScope::Family,
            owner_subject: Some("editor-subject"),
            owner_issuer: Some("synthetic-issuer"),
            original_filename: "crafted.md",
            bytes: b"## Crafted\n\nA synthetic crafted document key.\n",
            document_key: Some("evil) (v9"),
        },
    )
    .expect("crafted key submission");
    assert!(!crafted.document_key.contains(')'));
    assert!(!crafted.document_key.contains(' '));
    assert!(!crafted.source.contains("(v9)"));

    // Rerunning migration 009 must not attach a post-migration chunk to the
    // already quarantined legacy version.
    let legacy_count_before: i64 = client
        .query_one(
            "SELECT count(*) FROM document_version WHERE origin = 'legacy_import'",
            &[],
        )
        .expect("legacy count")
        .get(0);
    client
        .execute(
            "INSERT INTO documents
                 (content, source, section, chunk_index, embedding, content_hash, corpus)
             VALUES ('post-migration chunk', 'legacy-source', 'Section', 9,
                     (SELECT array_agg(0.3::float8) FROM generate_series(1, 768))::vector,
                     'post-migration', 'legacy-corpus')",
            &[],
        )
        .expect("insert post-migration chunk");
    apply(&mut client, &["migrations/009_document_lifecycle.sql"]);
    let linked: Option<i64> = client
        .query_one(
            "SELECT document_version_id FROM documents WHERE content = 'post-migration chunk'",
            &[],
        )
        .expect("read post-migration chunk")
        .get(0);
    assert_eq!(linked, None);
    let legacy_count_after: i64 = client
        .query_one(
            "SELECT count(*) FROM document_version WHERE origin = 'legacy_import'",
            &[],
        )
        .expect("legacy count after")
        .get(0);
    assert_eq!(legacy_count_after, legacy_count_before);

    // The rerun must stay a no-op even when migration 010's unique
    // (corpus, source) index is not present to catch the insert.
    client
        .execute(
            "DROP INDEX IF EXISTS document_version_corpus_source_uniq",
            &[],
        )
        .expect("drop source index");
    client
        .execute(
            "INSERT INTO documents
                 (content, source, section, chunk_index, embedding, content_hash, corpus)
             VALUES ('second post-migration chunk', 'legacy-source', 'Section', 10,
                     (SELECT array_agg(0.3::float8) FROM generate_series(1, 768))::vector,
                     'post-migration-2', 'legacy-corpus')",
            &[],
        )
        .expect("insert second post-migration chunk");
    apply(&mut client, &["migrations/009_document_lifecycle.sql"]);
    let legacy_count_no_index: i64 = client
        .query_one(
            "SELECT count(*) FROM document_version WHERE origin = 'legacy_import'",
            &[],
        )
        .expect("legacy count without index")
        .get(0);
    assert_eq!(legacy_count_no_index, legacy_count_before);
    let second_linked: Option<i64> = client
        .query_one(
            "SELECT document_version_id FROM documents WHERE content = 'second post-migration chunk'",
            &[],
        )
        .expect("read second post-migration chunk")
        .get(0);
    assert_eq!(second_linked, None);
    client
        .execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS document_version_corpus_source_uniq
                 ON document_version (corpus, source)",
            &[],
        )
        .expect("restore source index");

    // A failed trusted operator import never publishes, and a completed one
    // links its chunks only after the explicit finish step.
    let failed_import = begin_operator_import(
        &mut client,
        &storage,
        "cli-operator",
        "family-docs",
        "operator-failed",
        "Operator Failed",
        "operator-failed.md",
        b"## Operator\n\nSynthetic operator failure.",
    )
    .expect("begin failed import");
    assert!(!failed_import.already_ready);
    // A retry before the first attempt is finished reuses the same version
    // rather than stranding it and creating a duplicate.
    let retried_import = begin_operator_import(
        &mut client,
        &storage,
        "cli-operator",
        "family-docs",
        "operator-failed",
        "Operator Failed",
        "operator-failed.md",
        b"## Operator\n\nSynthetic operator failure.",
    )
    .expect("retry failed import");
    assert_eq!(retried_import.version_id, failed_import.version_id);
    assert!(!retried_import.already_ready);
    assert_eq!(
        get_version(&mut client, failed_import.version_id, "family-docs")
            .expect("failed import version")
            .processing_status,
        "processing"
    );
    fail_operator_import(
        &mut client,
        failed_import.version_id,
        "family-docs",
        "cli-operator",
    )
    .expect("fail operator import");
    assert_eq!(
        get_version(&mut client, failed_import.version_id, "family-docs")
            .expect("failed version")
            .processing_status,
        "failed"
    );

    let finished_import = begin_operator_import(
        &mut client,
        &storage,
        "cli-operator",
        "family-docs",
        "operator-finished",
        "Operator Finished",
        "operator-finished.md",
        b"## Operator\n\nSynthetic operator success.",
    )
    .expect("begin finished import");
    client
        .execute(
            "INSERT INTO documents
                 (content, source, section, chunk_index, embedding, content_hash, corpus)
             VALUES ('operator chunk', $1, 'Operator', 0,
                     (SELECT array_agg(0.4::float8) FROM generate_series(1, 768))::vector,
                     'operator-hash', 'family-docs')",
            &[&finished_import.source],
        )
        .expect("insert operator chunk");
    finish_operator_import(
        &mut client,
        finished_import.version_id,
        "family-docs",
        "cli-operator",
    )
    .expect("finish operator import");
    assert!(store
        .rows_for_corpus("family-docs")
        .expect("retrieval")
        .iter()
        .any(|row| row.chunk.source.as_deref() == Some(finished_import.source.as_str())));

    // A newer operator version supersedes the previous one but must not delete
    // or orphan that version's chunks.
    let replacement = begin_operator_import(
        &mut client,
        &storage,
        "cli-operator",
        "family-docs",
        "operator-finished",
        "Operator Finished",
        "operator-finished.md",
        b"## Operator\n\nSynthetic operator success, revised.",
    )
    .expect("begin replacement import");
    assert_ne!(replacement.version_id, finished_import.version_id);
    assert_ne!(replacement.source, finished_import.source);
    client
        .execute(
            "INSERT INTO documents
                 (content, source, section, chunk_index, embedding, content_hash, corpus)
             VALUES ('operator chunk v2', $1, 'Operator', 0,
                     (SELECT array_agg(0.5::float8) FROM generate_series(1, 768))::vector,
                     'operator-hash-2', 'family-docs')",
            &[&replacement.source],
        )
        .expect("insert replacement chunk");
    finish_operator_import(
        &mut client,
        replacement.version_id,
        "family-docs",
        "cli-operator",
    )
    .expect("finish replacement import");
    let previous = get_version(&mut client, finished_import.version_id, "family-docs")
        .expect("previous version");
    assert!(previous.is_removed());
    let previous_chunks: i64 = client
        .query_one(
            "SELECT count(*) FROM documents WHERE source = $1",
            &[&finished_import.source],
        )
        .expect("count previous chunks")
        .get(0);
    assert_eq!(previous_chunks, 1);
    let previous_linked: i64 = client
        .query_one(
            "SELECT count(*) FROM documents WHERE document_version_id = $1",
            &[&finished_import.version_id],
        )
        .expect("count previous linked chunks")
        .get(0);
    assert_eq!(previous_linked, 1);
    assert!(store
        .rows_for_corpus("family-docs")
        .expect("retrieval")
        .iter()
        .any(|row| row.chunk.source.as_deref() == Some(replacement.source.as_str())));

    // Removing a ready operator version takes it out of retrieval while its
    // chunk provenance stays linked to the version row.
    remove(&mut client, replacement.version_id, "family-docs", "admin")
        .expect("remove operator version");
    let still_linked: i64 = client
        .query_one(
            "SELECT count(*) FROM documents WHERE document_version_id = $1",
            &[&replacement.version_id],
        )
        .expect("count linked chunks")
        .get(0);
    assert_eq!(still_linked, 1);
    assert!(store
        .rows_for_corpus("family-docs")
        .expect("retrieval")
        .iter()
        .all(|row| row.chunk.source.as_deref() != Some(replacement.source.as_str())));

    // A scope change invalidates the job, and reapproval requeues it.
    let requeue = create_submission(
        &mut client,
        &storage,
        NewSubmission {
            actor: "editor-subject",
            corpus: "family-docs",
            title: "Requeue",
            description: None,
            category: Category::Other,
            access_scope: AccessScope::Family,
            owner_subject: Some("editor-subject"),
            owner_issuer: Some("synthetic-issuer"),
            original_filename: "requeue.md",
            bytes: b"## Requeue\n\nSynthetic requeue document.",
            document_key: None,
        },
    )
    .expect("requeue submission");
    review(
        &mut client,
        requeue.version_id,
        "family-docs",
        "admin",
        true,
        Some("approve"),
        Some(AccessScope::Family),
    )
    .expect("approve requeue");
    let requeue_job = queue_processing(&mut client, requeue.version_id, "family-docs", "admin")
        .expect("queue requeue");
    change_scope(
        &mut client,
        requeue.version_id,
        "family-docs",
        "admin",
        AccessScope::AdminOnly,
    )
    .expect("restrict requeue");
    let failed_job_status: String = client
        .query_one(
            "SELECT status FROM document_processing_job WHERE job_id = $1",
            &[&requeue_job],
        )
        .expect("read invalidated job")
        .get(0);
    assert_eq!(failed_job_status, "failed");
    change_scope(
        &mut client,
        requeue.version_id,
        "family-docs",
        "admin",
        AccessScope::Family,
    )
    .expect("restore requeue scope");
    review(
        &mut client,
        requeue.version_id,
        "family-docs",
        "admin",
        true,
        Some("reapprove"),
        Some(AccessScope::Family),
    )
    .expect("reapprove requeue");
    let requeued_job = queue_processing(&mut client, requeue.version_id, "family-docs", "admin")
        .expect("requeue processing");
    assert_eq!(requeued_job, requeue_job);
    let requeued_status: String = client
        .query_one(
            "SELECT status FROM document_processing_job WHERE job_id = $1",
            &[&requeued_job],
        )
        .expect("read requeued job")
        .get(0);
    assert_eq!(requeued_status, "queued");
}
