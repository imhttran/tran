//! RUST-006C end-to-end acceptance driver.
//!
//! This crate exercises the *production path* end to end against the dedicated
//! PostgreSQL/pgvector database:
//!
//! * ingestion goes through [`rag_foundation::service::IngestService`] over a
//!   real [`rag_foundation::storage_pg::PgStore`], so the guard, the chunker,
//!   the fingerprint read and the transactional replacement all run in
//!   production code;
//! * retrieval runs the production retrieval stages
//!   ([`rag_foundation::retrieval`]) over rows read back from the database;
//! * the cited answer comes from
//!   [`rag_foundation::answer::service::AnswerService`].
//!
//! Nothing here reimplements ingestion, retrieval or answering logic. Two
//! comparisons are reported **separately** in [`Report`]: the deterministic
//! section compares the fixture plus mocked-model run with the frozen baseline
//! at `fixtures/e2e/baseline.json` (a pass/fail gate that never writes the
//! baseline), and the live-model section records real-model observations. A
//! difference in the live section never fails a deterministic assertion.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use rag_foundation::answer::service::{Answer, AnswerService, RetrievalSource};
use rag_foundation::document::Chunk;
use rag_foundation::embedding::{Embedder, EmbeddingError};
use rag_foundation::fingerprint::{fingerprint, ChunkerConfig};
use rag_foundation::generation::{GenerationError, Generator};
use rag_foundation::ingest::IngestRejection;
use rag_foundation::pipeline::{IngestOutcome, PipelineConfig};
use rag_foundation::retrieval::vector::{self, IndexedVector};
use rag_foundation::retrieval::{dedup, expand, fts, fusion, Candidate};
use rag_foundation::service::IngestService;
use rag_foundation::storage::{DocumentStore, StoredChunk, EMBEDDING_DIM};
use rag_foundation::storage_pg::PgStore;

/// The embedding model name recorded with every fixture row.
pub const EMBED_MODEL: &str = "fixture-embed";
/// The chunker size used by the deterministic run.
pub const CHUNK_SIZE: usize = 50;
/// The chunker overlap used by the deterministic run.
pub const CHUNK_OVERLAP: usize = 20;
/// Path, relative to the repository root, of the frozen baseline.
pub const BASELINE_PATH: &str = "fixtures/e2e/baseline.json";
/// Path, relative to the repository root, of the deterministic corpus document.
pub const CORPUS_PATH: &str = "fixtures/parity/inputs/text-layer.pdf";

/// Locates the repository root from the crate manifest directory.
pub fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/acceptance/e2e`.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .expect("crate lives three levels below the repository root")
        .to_path_buf()
}

/// Returns the chunker configuration used by the deterministic run.
pub fn chunker() -> ChunkerConfig {
    ChunkerConfig::new(CHUNK_SIZE, CHUNK_OVERLAP)
}

/// Reads the dedicated-database DSN.
///
/// The test fails (it never skips) when `DATABASE_URL` is absent, so a missing
/// database can never yield a green suite. Returning the result rather than
/// panicking inside a helper keeps a single test failing cleanly.
pub fn dsn() -> Result<String, String> {
    match std::env::var("DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(
            "DATABASE_URL is not set: the RUST-006C end-to-end acceptance run requires the \
             dedicated PostgreSQL/pgvector database. Load the repository .env; this test fails, \
             it does not skip."
                .to_string(),
        ),
    }
}

/// Returns a unique tag so each run owns its own corpus and rows.
pub fn unique_tag() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    format!("e2e-{}-{nanos}", std::process::id())
}

/// The frozen deterministic baseline record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    /// Chunk count produced by ingesting the corpus document.
    pub chunks: usize,
    /// Stable candidate id of the top retrieval hit.
    pub top_candidate: String,
    /// The cited answer text rendered by the mocked generator.
    pub answer: String,
    /// The rendered page references of the cited answer.
    pub references: Vec<String>,
    /// Baseline rejection kinds, one per input class, in [`InputClass::all`]
    /// order.
    pub rejections: Vec<Rejection>,
}

/// One frozen baseline rejection record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    /// The input class name.
    pub class: String,
    /// The baseline rejection kind for the class.
    pub kind: String,
}

impl Baseline {
    /// Parses the frozen baseline JSON.
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|err| format!("parse baseline JSON: {err}"))
    }

    /// Loads the frozen baseline. It never creates the file: a missing baseline
    /// is an error, so a run can never freeze its own output.
    pub fn load() -> Result<Self, String> {
        let path = repo_root().join(BASELINE_PATH);
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read the frozen baseline {}: {e}", path.display()))?;
        Self::parse(&text)
    }
}

/// The deterministic comparison section of the report.
#[derive(Debug, Clone, Default)]
pub struct Deterministic {
    /// One human-readable line per compared field.
    pub checks: Vec<String>,
    /// `true` when every compared field matched the baseline.
    pub matched: bool,
}

/// The live-model variability section of the report.
#[derive(Debug, Clone)]
pub struct Live {
    /// `true` when a live model run was available.
    pub available: bool,
    /// Observations recorded from the live run.
    pub observations: Vec<String>,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            available: false,
            observations: vec!["live model not exercised in this run".to_string()],
        }
    }
}

/// The two-section report.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Deterministic fixture and mocked-model comparison.
    pub deterministic: Deterministic,
    /// Live-model variability observations, reported separately.
    pub live: Live,
}

impl Report {
    /// Renders the report with the two sections kept apart.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("[deterministic]\n");
        out.push_str(&format!("matched={}\n", self.deterministic.matched));
        for check in &self.deterministic.checks {
            out.push_str(&format!("check={check}\n"));
        }
        out.push_str("[live-model]\n");
        out.push_str(&format!("available={}\n", self.live.available));
        for observation in &self.live.observations {
            out.push_str(&format!("observation={observation}\n"));
        }
        out
    }
}

/// Collects live-model variability observations from the environment.
///
/// A live run is opt-in: `RAG_E2E_LIVE_OBSERVATIONS` supplies newline-separated
/// observations recorded from a real model. Without it the section reports that
/// the live model was not exercised. This section never affects the
/// deterministic verdict.
pub fn live_observations() -> Live {
    match std::env::var("RAG_E2E_LIVE_OBSERVATIONS") {
        Ok(value) if !value.trim().is_empty() => Live {
            available: true,
            observations: value
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
        },
        _ => Live::default(),
    }
}

/// A deterministic fixture embedder: the vector depends only on the text.
#[derive(Debug, Default)]
pub struct FixtureEmbedder;

impl FixtureEmbedder {
    /// Builds the fixture embedder.
    pub fn new() -> Self {
        Self
    }
}

impl Embedder for FixtureEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f64>, EmbeddingError> {
        let mut vector = vec![0.0f64; EMBEDDING_DIM];
        for (index, byte) in text.bytes().enumerate() {
            vector[(byte as usize + index) % EMBEDDING_DIM] += 1.0;
        }
        let norm = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
        if norm > 0.0 {
            for value in &mut vector {
                *value /= norm;
            }
        }
        Ok(vector)
    }
}

/// A deterministic embedder for questions, mirroring [`FixtureEmbedder`].
#[derive(Debug, Default)]
pub struct FixtureQueryEmbedder;

impl Embedder for FixtureQueryEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f64>, EmbeddingError> {
        FixtureEmbedder::new().embed(text)
    }
}

/// A mocked generator that replays a fixed answer and counts calls.
#[derive(Debug)]
pub struct MockGenerator {
    answer: String,
    calls: std::sync::atomic::AtomicUsize,
}

impl MockGenerator {
    /// Builds a mocked generator that returns `answer`.
    pub fn new(answer: impl Into<String>) -> Self {
        Self {
            answer: answer.into(),
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Returns how many generation calls were made.
    pub fn calls(&self) -> usize {
        use std::sync::atomic::Ordering;
        self.calls.load(Ordering::SeqCst)
    }
}

impl Generator for MockGenerator {
    fn generate(&self, _prompt: &str) -> Result<String, GenerationError> {
        use std::sync::atomic::Ordering;
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.answer.clone())
    }
}

/// The retrieval source used by the production answer entry point.
///
/// It reads the corpus rows from the database through the production store and
/// runs the production retrieval stages (vector search, full-text search, rank
/// fusion, deduplication, section expansion) — the same composition the
/// production `query` binary uses. No ranking logic is reimplemented here.
pub struct DbRetrieval<'a, E> {
    store: &'a PgStore,
    embedder: &'a E,
    corpus: &'a str,
    min_similarity: f64,
    final_k: usize,
    expand_limit: u32,
}

impl<'a, E> DbRetrieval<'a, E>
where
    E: Embedder,
{
    /// Builds the retrieval source over a store and a query embedder.
    pub fn new(store: &'a PgStore, embedder: &'a E, corpus: &'a str) -> Self {
        Self {
            store,
            embedder,
            corpus,
            min_similarity: 0.0,
            final_k: 5,
            expand_limit: 1,
        }
    }
}

impl<E> RetrievalSource for DbRetrieval<'_, E>
where
    E: Embedder,
{
    fn retrieve(&self, question: &str) -> Result<Vec<Candidate>, String> {
        let rows = self
            .store
            .rows_for_corpus(self.corpus)
            .map_err(|err| format!("load corpus {:?}: {err}", self.corpus))?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let chunks: Vec<Chunk> = rows.iter().map(stored_chunk).collect();
        let index: Vec<IndexedVector> = rows
            .iter()
            .map(|row| IndexedVector {
                chunk: stored_chunk(row),
                embedding: row.chunk.embedding.clone(),
            })
            .collect();

        let query = self
            .embedder
            .embed(question)
            .map_err(|err| format!("embed question: {err}"))?;

        let vector_hits = vector::search(&index, &query, self.min_similarity);
        let fts_hits = fts::search(&chunks, question);
        let mut candidates = fusion::fuse(&[vector_hits, fts_hits]);
        candidates = dedup::dedup(candidates);
        candidates = expand::expand(&candidates, &chunks, self.expand_limit);
        candidates.truncate(self.final_k);
        Ok(candidates)
    }
}

fn stored_chunk(row: &rag_foundation::storage_pg::StoredChunkRow) -> Chunk {
    Chunk {
        source: row.chunk.source.clone().unwrap_or_default(),
        section: row.chunk.section.clone().unwrap_or_default(),
        language: row.chunk.language.clone().unwrap_or_default(),
        chunk_index: row.chunk.chunk_index.max(0) as u32,
        content: row.chunk.content.clone(),
        page: row.chunk.page.and_then(|page| u32::try_from(page).ok()),
    }
}

/// The deterministic input classes the rejection tests cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputClass {
    /// A document the loader cannot read at all.
    Invalid,
    /// A PDF page without a text layer.
    Incomplete,
    /// A page-less document with no sections.
    Empty,
    /// A page-less document whose content yields no chunks.
    Oversized,
}

impl InputClass {
    /// Returns the baseline rejection class name.
    pub fn name(self) -> &'static str {
        match self {
            InputClass::Invalid => "invalid",
            InputClass::Incomplete => "incomplete",
            InputClass::Empty => "empty",
            InputClass::Oversized => "oversized",
        }
    }

    /// Returns the fixture path for the class, relative to the repository root.
    ///
    /// The `Oversized` class uses an unsupported-format fixture, matching the
    /// frozen baseline's loader rejection for that class.
    pub fn fixture(self) -> &'static str {
        match self {
            InputClass::Invalid => "fixtures/parity/inputs/missing.pdf",
            InputClass::Incomplete => "fixtures/parity/inputs/mixed-text-scan.pdf",
            InputClass::Empty => "fixtures/parity/inputs/empty.txt",
            InputClass::Oversized => "fixtures/parity/inputs/oversized.bin",
        }
    }

    /// Every class, in baseline order.
    pub fn all() -> [InputClass; 4] {
        [
            InputClass::Invalid,
            InputClass::Incomplete,
            InputClass::Empty,
            InputClass::Oversized,
        ]
    }
}

/// Returns the baseline rejection kind for an outcome.
pub fn rejection_kind(outcome: &IngestOutcome) -> String {
    match outcome {
        IngestOutcome::Rejected(IngestRejection::Loader(_)) => "loader".to_string(),
        IngestOutcome::Rejected(IngestRejection::Incomplete { .. }) => "incomplete".to_string(),
        IngestOutcome::Rejected(IngestRejection::NoChunks { .. }) => "no-chunks".to_string(),
        IngestOutcome::Unchanged { .. } => "unchanged".to_string(),
        IngestOutcome::Replaced { .. } => "replaced".to_string(),
    }
}

/// Seeds one row through the production store so a rejection test can prove the
/// stored data is preserved. The dimension guard runs inside the store.
pub fn seed_row(store: &PgStore, source: &str, corpus: &str, content: &str) {
    let mut embedding = vec![0.0_f64; EMBEDDING_DIM];
    embedding[0] = 1.0;
    store
        .write_chunk(&StoredChunk {
            content: content.to_string(),
            source: Some(source.to_string()),
            section: Some("Existing".to_string()),
            chunk_index: 0,
            language: Some("english".to_string()),
            page: Some(1),
            corpus: Some(corpus.to_string()),
            content_hash: format!("seed-{source}"),
            embed_model: EMBED_MODEL.to_string(),
            embedding_dim: EMBEDDING_DIM as i32,
            chunker_config: chunker().canonical(),
            ingested_at: "2026-10-18T00:00:00Z".to_string(),
            embedding,
        })
        .expect("seed the preserved row through the production store");
}

/// Returns a stable snapshot of every stored row for `source`, ordered by id.
///
/// The snapshot is read with a direct query so a rejection test can compare the
/// exact stored rows before and after a failing call, independent of which
/// fingerprint the failing call carried.
pub fn snapshot(source: &str) -> Result<Vec<String>, String> {
    let mut client = connect()?;
    let rows = client
        .query(
            "SELECT id, content, section, chunk_index, page, corpus, content_hash, embed_model, \
             embedding_dim, chunker_config FROM documents WHERE source = $1 ORDER BY id",
            &[&source],
        )
        .map_err(|err| format!("read the snapshot: {err}"))?;
    Ok(rows
        .iter()
        .map(|row| {
            format!(
                "{}|{}|{:?}|{}|{:?}|{:?}|{}|{}|{}|{}",
                row.get::<_, i64>(0),
                row.get::<_, String>(1),
                row.get::<_, Option<String>>(2),
                row.get::<_, i32>(3),
                row.get::<_, Option<i32>>(4),
                row.get::<_, Option<String>>(5),
                row.get::<_, String>(6),
                row.get::<_, String>(7),
                row.get::<_, i32>(8),
                row.get::<_, String>(9),
            )
        })
        .collect())
}

/// Returns the number of stored rows for `source`.
pub fn count_source(source: &str) -> Result<usize, String> {
    let mut client = connect()?;
    let row = client
        .query_one(
            "SELECT count(*) FROM documents WHERE source = $1",
            &[&source],
        )
        .map_err(|err| format!("count the test source: {err}"))?;
    let count: i64 = row.get(0);
    Ok(count as usize)
}

/// Deletes only the rows owned by one test source. Errors are surfaced: a failed
/// cleanup leaves the test-owned rows behind, which the caller must know about.
pub fn cleanup(source: &str) -> Result<(), String> {
    let mut client = connect()?;
    client
        .execute("DELETE FROM documents WHERE source = $1", &[&source])
        .map(|_| ())
        .map_err(|err| format!("clean up test-owned rows for {source}: {err}"))
}

/// Deletes every row owned by one run corpus: rows tagged with the corpus and
/// guard rows whose source is under the corpus prefix. A failed run therefore
/// cannot leave rows that block an isolated parity test later.
pub fn cleanup_corpus(corpus: &str) -> Result<(), String> {
    let mut client = connect()?;
    client
        .execute(
            "DELETE FROM documents WHERE corpus = $1 OR source LIKE $2",
            &[&corpus, &format!("{corpus}/%")],
        )
        .map(|_| ())
        .map_err(|err| format!("clean up corpus rows for {corpus}: {err}"))
}

fn connect() -> Result<postgres::Client, String> {
    let dsn = dsn()?;
    postgres::Client::connect(&dsn, postgres::NoTls).map_err(|err| format!("connect: {err}"))
}

/// Runs the deterministic production path once and returns the observed
/// baseline record together with the two-section report.
///
/// The path is: production [`IngestService`] over a real [`PgStore`] ->
/// production retrieval -> production [`AnswerService`] with a mocked model.
///
/// Rejection classes are exercised against independent test-owned sources so
/// each class independently proves it preserves its own stored rows.
pub fn run_deterministic(corpus: &str) -> Result<(Baseline, Report), String> {
    let dsn = dsn()?;
    let root = repo_root();
    let corpus_doc = root.join(CORPUS_PATH);
    let store = PgStore::connect(&dsn).map_err(|e| format!("connect to the store: {e}"))?;
    let embedder = FixtureEmbedder::new();

    let config = PipelineConfig::new("english", chunker(), EMBED_MODEL, corpus.to_string());

    // 0. Clean any rows a previous aborted run left for this corpus.
    cleanup_corpus(corpus)?;
    cleanup(CORPUS_PATH)?;

    // 1. Production ingestion entry point over the dedicated database.
    let service = IngestService::new(&embedder, &store, &store, config);
    let bytes = fs::read(&corpus_doc).map_err(|e| format!("read corpus: {e}"))?;
    let content_hash = fingerprint(&bytes, &chunker());
    let outcome = service
        .ingest_source(&corpus_doc, CORPUS_PATH, &content_hash)
        .map_err(|e| format!("ingest failed: {e}"))?;
    let chunks = match outcome {
        IngestOutcome::Replaced { chunks, .. } => chunks,
        other => return Err(format!("want a replacement, got {other:?}")),
    };

    // 2. Production retrieval, then the production answer entry point.
    let query_embedder = FixtureQueryEmbedder;
    let retrieval = DbRetrieval::new(&store, &query_embedder, corpus);
    let generator = MockGenerator::new(MOCKED_ANSWER);
    let answer_service = AnswerService::new(&retrieval, &generator, 4096);
    let question = "Which storage engine stores the vectors?";
    let answer = answer_service
        .answer(question)
        .map_err(|e| format!("answer failed: {e}"))?;
    let top_candidate = retrieval
        .retrieve(question)
        .map_err(|e| format!("retrieval failed: {e}"))?
        .first()
        .map(|candidate| candidate.id.clone())
        .ok_or("retrieval returned no candidates")?;

    // 3. Rejection classes: each rejects and preserves its own stored rows.
    let mut rejections = Vec::new();
    for class in InputClass::all() {
        let guard_source = guard_source(corpus, class);
        seed_row(&store, &guard_source, corpus, "pre-existing row");
        let before = snapshot(&guard_source)?;
        let path = root.join(class.fixture());
        let outcome = service
            .ingest_source(&path, &guard_source, &format!("reject-{}", class.name()))
            .map_err(|e| format!("{} ingest failed: {e}", class.name()))?;
        if !matches!(outcome, IngestOutcome::Rejected(_)) {
            return Err(format!("{} was not rejected: {outcome:?}", class.name()));
        }
        rejections.push(Rejection {
            class: class.name().to_string(),
            kind: rejection_kind(&outcome),
        });
        if snapshot(&guard_source)? != before {
            return Err(format!(
                "{} rejection changed the stored rows",
                class.name()
            ));
        }
        cleanup(&guard_source)?;
    }

    let observed = Baseline {
        chunks,
        top_candidate,
        answer: answer.text.clone(),
        references: answer.references.clone(),
        rejections,
    };

    let frozen = Baseline::load()?;
    let mut report = Report::default();
    let mut matched = true;
    matched &= compare(
        &mut report.deterministic.checks,
        "chunks",
        frozen.chunks,
        observed.chunks,
    );
    matched &= compare(
        &mut report.deterministic.checks,
        "top_candidate",
        &frozen.top_candidate,
        &observed.top_candidate,
    );
    matched &= compare(
        &mut report.deterministic.checks,
        "answer",
        &frozen.answer,
        &observed.answer,
    );
    matched &= compare(
        &mut report.deterministic.checks,
        "references",
        &frozen.references,
        &observed.references,
    );
    matched &= compare(
        &mut report.deterministic.checks,
        "rejections",
        &frozen.rejections,
        &observed.rejections,
    );
    report.deterministic.matched = matched;
    report.live = live_observations();

    // 4. Leave only test-owned rows we created for the corpus behind to be
    // cleaned by the caller; never touch other data.
    Ok((observed, report))
}

/// Returns the per-class guard source name, so each rejection class proves its
/// own data preservation on an independent test-owned source.
pub fn guard_source(corpus: &str, class: InputClass) -> String {
    format!("{corpus}/reject-{}", class.name())
}

/// The fixed answer the mocked model returns. It cites the corpus page.
pub const MOCKED_ANSWER: &str =
    "PostgreSQL with the pgvector extension stores the vectors [fixtures/parity/inputs/text-layer.pdf - Page 1 - p.1].";

fn compare<T: PartialEq + std::fmt::Debug>(
    checks: &mut Vec<String>,
    label: &str,
    expected: T,
    actual: T,
) -> bool {
    if expected == actual {
        checks.push(format!("{label}: match"));
        true
    } else {
        checks.push(format!(
            "{label}: mismatch expected={expected:?} actual={actual:?}"
        ));
        false
    }
}

/// Returns the cited answer for a fixed generator answer through the production
/// answer entry point.
pub fn cited_answer(
    store: &PgStore,
    corpus: &str,
    question: &str,
    generator_answer: &str,
) -> Result<Answer, String> {
    let query_embedder = FixtureQueryEmbedder;
    let retrieval = DbRetrieval::new(store, &query_embedder, corpus);
    let generator = MockGenerator::new(generator_answer);
    let answer_service = AnswerService::new(&retrieval, &generator, 4096);
    answer_service.answer(question).map_err(|e| e.to_string())
}

/// Reads a fixture file from the repository.
pub fn read_fixture(relative: &str) -> Result<String, String> {
    let path: PathBuf = repo_root().join(relative);
    fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Returns the fixture directory the crate uses.
pub fn fixtures_dir() -> PathBuf {
    repo_root().join("fixtures/e2e")
}

/// Returns `true` when the path exists under the repository root.
pub fn fixture_exists(relative: &str) -> bool {
    Path::new(&repo_root().join(relative)).exists()
}
