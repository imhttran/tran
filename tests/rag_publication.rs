//! FAM-RAG-01 regression matrix: approved-only retrieval, exact citation
//! provenance, and revocation integrity.
//!
//! Synthetic documents only. The suite writes to a disposable RAG database whose
//! name ends in `_test`, uses the real `approved_documents` view and the real
//! Rust retrieval/citation path, and asserts a positive control before every
//! negative exclusion so a broken fixture cannot pass vacuously.

use std::fs;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use postgres::{Client, NoTls};
use rag_foundation::answer::context_budget::PassageIdentity;
use rag_foundation::answer::service::{
    AnswerService, FixedRetrieval, PublicationError, PublicationGuard,
};
use rag_foundation::document::Chunk;
use rag_foundation::generation::{GenerationError, Generator};
use rag_foundation::retrieval::{Candidate, ChunkProvenance};
use rag_foundation::storage_pg::{PgStore, SectionKey};

const CORPUS: &str = "rag-publication-a";
const OTHER_CORPUS: &str = "rag-publication-b";

/// Valid 64-hex fingerprints for the synthetic rows.
const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_STALE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SHA_OLD: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

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

fn apply(client: &mut Client, paths: &[&str]) {
    for path in paths {
        let sql = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|error| panic!("apply {path}: {error}"));
    }
}

fn prepare(client: &mut Client) {
    apply(client, &PRE_LIFECYCLE);
    apply(
        client,
        &[
            "migrations/009_document_lifecycle.sql",
            "migrations/010_document_security_hardening.sql",
        ],
    );
    client
        .batch_execute(
            "TRUNCATE documents, document_audit_event, document_processing_job, document_version \
             RESTART IDENTITY CASCADE",
        )
        .expect("reset lifecycle tables");
}

struct VersionSpec<'a> {
    corpus: &'a str,
    document_key: &'a str,
    version_number: i32,
    source: &'a str,
    review_status: &'a str,
    processing_status: &'a str,
    access_scope: Option<&'a str>,
    removed: bool,
    content_sha256: &'a str,
    approved_sha256: Option<&'a str>,
    approved_scope: Option<&'a str>,
    approved_version_number: Option<i32>,
}

impl<'a> VersionSpec<'a> {
    /// A version that satisfies the `approved_documents` predicate.
    fn approved(corpus: &'a str, key: &'a str, source: &'a str, sha: &'a str) -> Self {
        Self {
            corpus,
            document_key: key,
            version_number: 1,
            source,
            review_status: "approved",
            processing_status: "ready",
            access_scope: Some("family"),
            removed: false,
            content_sha256: sha,
            approved_sha256: Some(sha),
            approved_scope: Some("family"),
            approved_version_number: Some(1),
        }
    }
}

fn insert_version(client: &mut Client, spec: &VersionSpec<'_>) -> i64 {
    client
        .query_one(
            "INSERT INTO document_version
                 (document_key, version_number, corpus, source, title, category, origin,
                  access_scope, owner_subject, owner_issuer, content_sha256, byte_size,
                  mime_type, original_filename, storage_key, review_status,
                  processing_status, approved_sha256, approved_scope,
                  approved_version_number, created_by, removed_at, reviewed_by, reviewed_at)
             VALUES ($1, $2, $3, $4, $4, 'story', 'submission', $5, 'owner', 'issuer',
                     $6, 1, 'text/markdown', $4, $4, $7, $8, $9, $10, $11, 'test',
                     CASE WHEN $12 THEN now() ELSE NULL END,
                     CASE WHEN $7 = 'approved' THEN 'test' ELSE NULL END,
                     CASE WHEN $7 = 'approved' THEN now() ELSE NULL END)
             RETURNING version_id",
            &[
                &spec.document_key,
                &spec.version_number,
                &spec.corpus,
                &spec.source,
                &spec.access_scope,
                &spec.content_sha256,
                &spec.review_status,
                &spec.processing_status,
                &spec.approved_sha256,
                &spec.approved_scope,
                &spec.approved_version_number,
                &spec.removed,
            ],
        )
        .expect("insert version")
        .get(0)
}

#[allow(clippy::too_many_arguments)]
fn insert_chunk(
    client: &mut Client,
    version_id: i64,
    corpus: &str,
    source: &str,
    section: &str,
    page: Option<i32>,
    chunk_index: i32,
    content: &str,
    similarity: f64,
) -> i64 {
    let vector = format!(
        "[{}]",
        std::iter::repeat_n(similarity.to_string(), 768)
            .collect::<Vec<_>>()
            .join(",")
    );
    client
        .query_one(
            "INSERT INTO documents
                 (content, source, section, chunk_index, embedding, content_hash, corpus,
                  language, page, document_version_id)
             VALUES ($1, $2, $3, $4, $5::text::vector, 'hash', $6, 'english', $7, $8)
             RETURNING id",
            &[
                &content,
                &source,
                &section,
                &chunk_index,
                &vector,
                &corpus,
                &page,
                &version_id,
            ],
        )
        .expect("insert chunk")
        .get(0)
}

/// Seeds one approved version with one retrievable chunk and returns
/// `(version_id, chunk_id)`.
#[allow(clippy::too_many_arguments)]
fn seed_approved(
    client: &mut Client,
    corpus: &str,
    key: &str,
    version_number: i32,
    source: &str,
    section: &str,
    content: &str,
    similarity: f64,
) -> (i64, i64) {
    let mut spec = VersionSpec::approved(corpus, key, source, SHA);
    spec.version_number = version_number;
    spec.approved_version_number = Some(version_number);
    let version = insert_version(client, &spec);
    let chunk = insert_chunk(
        client, version, corpus, source, section, None, 0, content, similarity,
    );
    (version, chunk)
}

fn identity(row: &rag_foundation::storage_pg::PublicationRow) -> PassageIdentity {
    PassageIdentity {
        chunk_id: Some(row.chunk_id),
        document_version_id: Some(row.document_version_id),
        corpus: row.corpus.clone(),
        content_hash: row.content_hash.clone(),
    }
}

#[test]
fn approved_documents_is_the_only_retrieval_boundary() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping approved-only retrieval test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    prepare(&mut client);
    let store = PgStore::connect(&dsn).expect("store");

    // Positive control: this row must be retrievable by every path.
    let (control_version, control_chunk) = seed_approved(
        &mut client,
        CORPUS,
        "control",
        1,
        "control.md",
        "Control",
        "the sentinel control passage",
        0.1,
    );
    assert!(control_version > 0 && control_chunk > 0);

    let mut excluded = Vec::new();
    for (
        key,
        source,
        section,
        review,
        processing,
        scope,
        removed,
        approved_sha,
        approved_scope,
        approved_version,
    ) in [
        (
            "pending",
            "pending.md",
            "Pending",
            "pending_review",
            "unprocessed",
            Some("family"),
            false,
            Some(SHA),
            Some("family"),
            Some(1),
        ),
        (
            "rejected",
            "rejected.md",
            "Rejected",
            "rejected",
            "unprocessed",
            Some("family"),
            false,
            None,
            None,
            None,
        ),
        (
            "unprocessed",
            "unprocessed.md",
            "Unprocessed",
            "approved",
            "unprocessed",
            Some("family"),
            false,
            Some(SHA),
            Some("family"),
            Some(1),
        ),
        (
            "admin",
            "admin.md",
            "Admin",
            "approved",
            "ready",
            Some("admin_only"),
            false,
            Some(SHA),
            Some("admin_only"),
            Some(1),
        ),
        (
            "removed",
            "removed.md",
            "Removed",
            "approved",
            "ready",
            Some("family"),
            true,
            Some(SHA),
            Some("family"),
            Some(1),
        ),
    ] {
        let version = insert_version(
            &mut client,
            &VersionSpec {
                corpus: CORPUS,
                document_key: key,
                version_number: 1,
                source,
                review_status: review,
                processing_status: processing,
                access_scope: scope,
                removed,
                content_sha256: SHA,
                approved_sha256: approved_sha,
                approved_scope,
                approved_version_number: approved_version,
            },
        );
        // A vector close to the question so an unfiltered search would rank
        // these rows ahead of the control.
        insert_chunk(
            &mut client,
            version,
            CORPUS,
            source,
            section,
            None,
            0,
            "excluded sentinel",
            0.99,
        );
        excluded.push(source);
    }

    let snapshot = store.retrieval_snapshot(CORPUS).expect("snapshot");
    assert_eq!(
        snapshot.len(),
        1,
        "only the approved control may be in the snapshot: {snapshot:?}"
    );
    assert_eq!(snapshot[0].source, "control.md");
    assert_eq!(snapshot[0].chunk_id, control_chunk);
    assert_eq!(snapshot[0].document_version_id, control_version);

    assert_eq!(store.rows_for_corpus(CORPUS).expect("rows").len(), 1);
    assert_eq!(store.chunks_for_corpus(CORPUS).expect("chunks").len(), 1);

    let vector = vec![0.1_f64; 768];
    let hits = store.vector_search(CORPUS, &vector, 20).expect("vector");
    assert_eq!(hits.len(), 1, "vector search must be approved-only");
    assert_eq!(hits[0].row.chunk.source.as_deref(), Some("control.md"));

    let hits = store
        .keyword_search(CORPUS, "sentinel", 20, "english")
        .expect("keyword");
    assert!(
        hits.iter()
            .all(|hit| hit.row.chunk.source.as_deref() == Some("control.md")),
        "keyword search must be approved-only"
    );
    let hits = store
        .keyword_search(CORPUS, "excluded", 20, "english")
        .expect("keyword excluded");
    assert!(hits.is_empty(), "excluded content must not be searchable");

    let control = store
        .section_chunks(
            CORPUS,
            &[SectionKey {
                source: "control.md".to_string(),
                section: "Control".to_string(),
            }],
            10,
        )
        .expect("section control");
    assert_eq!(control.len(), 1);
    for source in &excluded {
        let hits = store
            .section_chunks(
                CORPUS,
                &[SectionKey {
                    source: (*source).to_string(),
                    section: "Control".to_string(),
                }],
                10,
            )
            .expect("section excluded");
        assert!(hits.is_empty(), "{source} must not expand");
    }

    // The schema's approval-binding CHECK prevents these two corrupted states
    // from existing in normal operation. Drop it only inside this disposable
    // test database to prove the view's redundant read-time clauses still
    // exclude them, then remove the fixtures and restore the constraint.
    client
        .batch_execute(
            "ALTER TABLE document_version \
             DROP CONSTRAINT IF EXISTS document_version_approval_binding",
        )
        .expect("drop approval-binding check");
    let stale_fingerprint = insert_version(
        &mut client,
        &VersionSpec {
            corpus: CORPUS,
            document_key: "stale-fingerprint",
            version_number: 1,
            source: "stale-fingerprint.md",
            review_status: "approved",
            processing_status: "ready",
            access_scope: Some("family"),
            removed: false,
            content_sha256: SHA,
            approved_sha256: Some(SHA_STALE),
            approved_scope: Some("family"),
            approved_version_number: Some(1),
        },
    );
    let stale_version = insert_version(
        &mut client,
        &VersionSpec {
            corpus: CORPUS,
            document_key: "stale-version",
            version_number: 1,
            source: "stale-version.md",
            review_status: "approved",
            processing_status: "ready",
            access_scope: Some("family"),
            removed: false,
            content_sha256: SHA,
            approved_sha256: Some(SHA),
            approved_scope: Some("family"),
            approved_version_number: Some(9),
        },
    );
    for (version, source) in [
        (stale_fingerprint, "stale-fingerprint.md"),
        (stale_version, "stale-version.md"),
    ] {
        insert_chunk(
            &mut client,
            version,
            CORPUS,
            source,
            "Stale",
            None,
            0,
            "stale binding sentinel",
            0.99,
        );
    }
    assert_eq!(
        store
            .retrieval_snapshot(CORPUS)
            .expect("snapshot stale")
            .len(),
        1,
        "stale approval bindings must stay out of the snapshot"
    );
    assert!(store
        .keyword_search(CORPUS, "stale", 20, "english")
        .expect("keyword stale")
        .is_empty());
    client
        .execute(
            "DELETE FROM documents WHERE document_version_id IN ($1, $2)",
            &[&stale_fingerprint, &stale_version],
        )
        .expect("delete stale chunks");
    client
        .execute(
            "DELETE FROM document_version WHERE version_id IN ($1, $2)",
            &[&stale_fingerprint, &stale_version],
        )
        .expect("delete stale versions");
    client
        .batch_execute(
            "ALTER TABLE document_version ADD CONSTRAINT document_version_approval_binding CHECK (review_status <> 'approved' OR (reviewed_by IS NOT NULL AND reviewed_at IS NOT NULL AND access_scope IS NOT NULL AND approved_sha256 = content_sha256 AND approved_scope = access_scope AND approved_version_number = version_number))",
        )
        .expect("restore approval-binding check");
}

#[test]
fn superseded_cross_corpus_and_revalidation_are_exact() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping provenance test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    prepare(&mut client);
    let store = PgStore::connect(&dsn).expect("store");

    // Version 1 of one document, superseded by version 2.
    let v1 = insert_version(
        &mut client,
        &VersionSpec::approved(CORPUS, "shared", "shared.md (shared-aaaa1111)", SHA_OLD),
    );
    let v1_chunk = insert_chunk(
        &mut client,
        v1,
        CORPUS,
        "shared.md (shared-aaaa1111)",
        "S",
        None,
        0,
        "old",
        0.1,
    );
    client
        .execute(
            "UPDATE document_version SET removed_at = now(), removed_by = 'system:superseded'
              WHERE version_id = $1",
            &[&v1],
        )
        .expect("supersede v1");
    let (v2, _) = seed_approved(
        &mut client,
        CORPUS,
        "shared",
        2,
        "shared.md (shared-aaaa1111) (v2)",
        "S",
        "new",
        0.1,
    );

    let snapshot = store.retrieval_snapshot(CORPUS).expect("snapshot");
    assert_eq!(snapshot.len(), 1, "only the current version is published");
    assert_eq!(snapshot[0].document_version_id, v2);

    // The superseded identity fails revalidation.
    let old_identity = PassageIdentity {
        chunk_id: Some(v1_chunk),
        document_version_id: Some(v1),
        corpus: CORPUS.to_string(),
        content_hash: SHA_OLD.to_string(),
    };
    assert!(!store
        .revalidate_publication(CORPUS, &[&old_identity])
        .expect("revalidate old"));
    let current_identity = identity(&snapshot[0]);
    assert!(store
        .revalidate_publication(CORPUS, &[&current_identity])
        .expect("revalidate current"));

    // The same filename in a second corpus is a different publication.
    let (other_version, _) = seed_approved(
        &mut client,
        OTHER_CORPUS,
        "shared",
        1,
        "shared.md (shared-aaaa1111)",
        "S",
        "other corpus",
        0.1,
    );
    let other_snapshot = store
        .retrieval_snapshot(OTHER_CORPUS)
        .expect("other snapshot");
    assert_eq!(other_snapshot.len(), 1);
    assert_eq!(other_snapshot[0].document_version_id, other_version);
    assert_ne!(other_snapshot[0].chunk_id, snapshot[0].chunk_id);
    // A chunk from one corpus is not published in the other.
    assert!(!store
        .revalidate_publication(OTHER_CORPUS, &[&current_identity])
        .expect("cross corpus revalidate"));

    // Removal and restriction both revoke immediately.
    client
        .execute(
            "UPDATE document_version SET access_scope = 'admin_only', approved_scope = 'admin_only'
              WHERE version_id = $1",
            &[&v2],
        )
        .expect("restrict");
    assert!(store
        .retrieval_snapshot(CORPUS)
        .expect("snapshot restricted")
        .is_empty());
    assert!(!store
        .revalidate_publication(CORPUS, &[&current_identity])
        .expect("revalidate restricted"));
}

/// A generator that flips a shared "revoked" flag before returning its answer.
struct RevokingGenerator {
    revoked: Arc<AtomicBool>,
    answer: String,
}

impl Generator for RevokingGenerator {
    fn generate(&self, _prompt: &str) -> Result<String, GenerationError> {
        self.revoked.store(true, Ordering::SeqCst);
        Ok(self.answer.clone())
    }
}

struct FlagGuard {
    revoked: Arc<AtomicBool>,
}

impl PublicationGuard for FlagGuard {
    fn revalidate(
        &self,
        _passages: &[rag_foundation::answer::context_budget::ContextPassage],
    ) -> Result<bool, PublicationError> {
        Ok(!self.revoked.load(Ordering::SeqCst))
    }
}

fn cited_candidate() -> Candidate {
    Candidate::with_provenance(
        Chunk {
            source: "report.md (report-md-aaaa1111)".to_string(),
            section: "Findings".to_string(),
            language: "english".to_string(),
            chunk_index: 0,
            content: "The finding.".to_string(),
            page: None,
        },
        0.9,
        ChunkProvenance {
            chunk_id: Some(10),
            document_version_id: Some(20),
            corpus: CORPUS.to_string(),
            content_hash: "sha".to_string(),
        },
    )
}

#[test]
fn revocation_during_generation_refuses_the_answer() {
    let revoked = Arc::new(AtomicBool::new(false));
    let retrieval = FixedRetrieval::new(vec![cited_candidate()]);
    let generator = RevokingGenerator {
        revoked: Arc::clone(&revoked),
        answer: "A finding [report.md (report-md-aaaa1111) - Findings].".to_string(),
    };
    let guard = FlagGuard {
        revoked: Arc::clone(&revoked),
    };
    let service = AnswerService::new(&retrieval, &generator, 4000).with_publication_guard(&guard);
    let answer = service.answer("what is the finding?").expect("answer");
    assert!(
        answer.refused,
        "a revocation committed during generation must refuse"
    );
    assert!(answer.text.contains("cannot answer"));
    assert!(answer.references.is_empty());
    assert!(answer.citations.is_empty());

    // Control: without the revocation the same path returns the citation.
    let clean = Arc::new(AtomicBool::new(false));
    let retrieval = FixedRetrieval::new(vec![cited_candidate()]);
    let generator = RevokingGenerator {
        revoked: Arc::new(AtomicBool::new(false)),
        answer: "A finding [report.md (report-md-aaaa1111) - Findings].".to_string(),
    };
    let guard = FlagGuard {
        revoked: Arc::clone(&clean),
    };
    let service = AnswerService::new(&retrieval, &generator, 4000).with_publication_guard(&guard);
    let answer = service.answer("what is the finding?").expect("answer");
    assert!(!answer.refused);
    assert_eq!(answer.citations.len(), 1);
    assert_eq!(answer.citations[0].identity.document_version_id, Some(20));
}

#[test]
fn citations_carry_version_identity_and_never_mix_documents() {
    let one = Candidate::with_provenance(
        Chunk {
            source: "report.md (report-md-aaaa1111)".to_string(),
            section: "Findings".to_string(),
            language: "english".to_string(),
            chunk_index: 0,
            content: "first".to_string(),
            page: None,
        },
        0.9,
        ChunkProvenance {
            chunk_id: Some(1),
            document_version_id: Some(101),
            corpus: CORPUS.to_string(),
            content_hash: "sha-1".to_string(),
        },
    );
    let two = Candidate::with_provenance(
        Chunk {
            source: "report.md (report-md-bbbb2222)".to_string(),
            section: "Findings".to_string(),
            language: "english".to_string(),
            chunk_index: 0,
            content: "second".to_string(),
            page: None,
        },
        0.8,
        ChunkProvenance {
            chunk_id: Some(2),
            document_version_id: Some(202),
            corpus: CORPUS.to_string(),
            content_hash: "sha-2".to_string(),
        },
    );
    let retrieval = FixedRetrieval::new(vec![one, two]);

    // Exact key cites the second document, not the first same-named one.
    struct StaticGenerator(String);
    impl Generator for StaticGenerator {
        fn generate(&self, _prompt: &str) -> Result<String, GenerationError> {
            Ok(self.0.clone())
        }
    }
    let generator =
        StaticGenerator("Cited [report.md (report-md-bbbb2222) - Findings].".to_string());
    let service = AnswerService::new(&retrieval, &generator, 4000);
    let answer = service.answer("which?").expect("answer");
    assert_eq!(answer.citations.len(), 1);
    assert_eq!(answer.citations[0].identity.document_version_id, Some(202));
    assert_eq!(answer.citations[0].content, "second");

    // A human-readable filename that names both documents is ambiguous and is
    // stripped rather than attributed to the first passage.
    let generator = StaticGenerator("Cited [report.md - Findings].".to_string());
    let service = AnswerService::new(&retrieval, &generator, 4000);
    let answer = service.answer("which?").expect("answer");
    assert!(answer.citations.is_empty());
    assert!(answer.references.is_empty());
    assert!(!answer.text.contains("[report.md - Findings]"));
}

#[test]
fn multi_chunk_section_keeps_all_supporting_chunks() {
    // Section expansion routinely puts several chunks of one section into the
    // context. They are one citeable publication; every chunk is returned so
    // the answer exposes the real supporting text and provenance rather than an
    // arbitrary first chunk.
    let chunk = |index: u32, id: i64, content: &str| {
        Candidate::with_provenance(
            Chunk {
                source: "minutes.md (minutes-md-aaaa1111)".to_string(),
                section: "Decisions".to_string(),
                language: "english".to_string(),
                chunk_index: index,
                content: content.to_string(),
                page: Some(2),
            },
            0.9,
            ChunkProvenance {
                chunk_id: Some(id),
                document_version_id: Some(700),
                corpus: CORPUS.to_string(),
                content_hash: "sha-minutes".to_string(),
            },
        )
    };
    let retrieval = FixedRetrieval::new(vec![
        chunk(0, 10, "Agenda and attendance."),
        chunk(1, 11, "The decision was to approve the 2026 budget."),
    ]);

    struct StaticGenerator(String);
    impl Generator for StaticGenerator {
        fn generate(&self, _prompt: &str) -> Result<String, GenerationError> {
            Ok(self.0.clone())
        }
    }

    let generator = StaticGenerator(
        "The decision was recorded [minutes.md (minutes-md-aaaa1111) - Decisions - p.2]."
            .to_string(),
    );
    let service = AnswerService::new(&retrieval, &generator, 4000);
    let answer = service.answer("what was decided?").expect("answer");
    assert!(!answer.refused);
    assert_eq!(
        answer.citations.len(),
        2,
        "every supporting chunk of the publication is exposed"
    );
    assert_eq!(answer.references.len(), 1);
    assert!(
        answer
            .text
            .contains("[minutes.md (minutes-md-aaaa1111) - Decisions - p.2]"),
        "the valid citation must be preserved: {}",
        answer.text
    );

    let ids: Vec<_> = answer
        .citations
        .iter()
        .map(|passage| passage.identity.chunk_id)
        .collect();
    assert_eq!(ids, vec![Some(10), Some(11)]);
    assert!(answer.citations.iter().all(|passage| {
        passage.identity.document_version_id == Some(700)
            && passage.identity.corpus == CORPUS
            && passage.identity.content_hash == "sha-minutes"
    }));

    // The answer depends on the second chunk; its content and provenance are
    // exposed instead of the unrelated first chunk.
    let support = answer
        .citations
        .iter()
        .find(|passage| passage.identity.chunk_id == Some(11))
        .expect("second chunk is a supporting citation");
    assert!(support.content.contains("approve the 2026 budget"));

    // The human-readable form resolves to the same publication and all chunks.
    let generator = StaticGenerator("Recorded [minutes.md - Decisions - p.2].".to_string());
    let service = AnswerService::new(&retrieval, &generator, 4000);
    let answer = service.answer("what was decided?").expect("answer");
    assert_eq!(answer.citations.len(), 2);
    assert_eq!(answer.references.len(), 1);
}
