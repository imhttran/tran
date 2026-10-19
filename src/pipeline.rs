//! Ingest pipeline caller wiring: guard before any effect.
//!
//! Composes the ordered ingest flow the Go caller (`cmd/ingest`) implements:
//!
//! 1. load the source (PDF through the text-layer loader);
//! 2. run the pre-effect guard ([`crate::ingest::gate_pdf`]) so an incomplete,
//!    fully scanned, or unreadable PDF is rejected **before** any embedding
//!    call or database write;
//! 3. compute the source fingerprint ([`crate::fingerprint::fingerprint`]);
//! 4. read the stored rows and skip when the source is unchanged;
//! 5. embed each chunk through the [`Embedder`] port;
//! 6. replace the source's rows in one transaction through the
//!    [`SourceReplacer`] port.
//!
//! The caller holds no database client and no HTTP client: both effects go
//! through ports, so a caller-level test records every call and proves that a
//! rejected source causes zero embedding calls and zero writes.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use thiserror::Error;

use crate::document::Chunk;
use crate::embedding::{Embedder, EmbeddingError};
use crate::fingerprint::{
    fingerprint, unchanged_source, ChunkerConfig, FingerprintReader, UnchangedSource,
};
use crate::ingest::{gate_pdf, gate_sections, IngestRejection};
use crate::loader::{load_markdown, load_pdf, load_text, PdfError, PdfExtraction};
use crate::storage::StoredChunk;
use crate::transaction::{validate_all_dims, ReplaceError, SourceReplacer};

/// Errors raised by the ingest pipeline after the guard passed.
#[derive(Debug, Error)]
pub enum PipelineError {
    /// The source was rejected by the pre-effect guard: no effect occurred.
    #[error(transparent)]
    Rejected(#[from] IngestRejection),
    /// The unchanged-source read failed. No effect occurred.
    #[error("unchanged-source read failed: {0}")]
    Fingerprint(String),
    /// An embedding call failed. No database write occurred.
    #[error(transparent)]
    Embed(#[from] EmbeddingError),
    /// The transactional replacement failed and rolled back.
    #[error(transparent)]
    Replace(#[from] ReplaceError),
}

/// The outcome of one ingest run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    /// The guard rejected the source; no embedding call or write occurred.
    Rejected(IngestRejection),
    /// The stored fingerprint already matched; no embedding call or write
    /// occurred and the existing rows are preserved.
    Unchanged { fingerprint: String, rows: usize },
    /// Every chunk was embedded and the source was replaced in one transaction.
    Replaced { fingerprint: String, chunks: usize },
}

/// Settings the pipeline needs: language, chunker configuration, embedding
/// model name, and the corpus identity written with each chunk.
pub struct PipelineConfig {
    /// Corpus language passed to the chunker and stored per chunk.
    pub language: String,
    /// Chunker configuration qualified into the fingerprint.
    pub chunker: ChunkerConfig,
    /// Embedding model name stored per chunk (`embed_model`).
    pub embed_model: String,
    /// Trusted corpus identity stored per chunk (`corpus`).
    pub corpus: String,
}

impl PipelineConfig {
    /// Builds pipeline settings.
    pub fn new(
        language: impl Into<String>,
        chunker: ChunkerConfig,
        embed_model: impl Into<String>,
        corpus: impl Into<String>,
    ) -> Self {
        Self {
            language: language.into(),
            chunker,
            embed_model: embed_model.into(),
            corpus: corpus.into(),
        }
    }
}

/// The ingest caller.
///
/// `E` is the embedding port, `R` the unchanged-source read port, and `S` the
/// transactional replacement port. Tests substitute recording fakes.
pub struct IngestPipeline<'a, E, R, S> {
    embedder: &'a E,
    reader: &'a R,
    replacer: &'a S,
    config: PipelineConfig,
}

impl<'a, E, R, S> IngestPipeline<'a, E, R, S>
where
    E: Embedder,
    R: FingerprintReader,
    S: SourceReplacer,
{
    /// Builds a pipeline over the three ports and the settings.
    pub fn new(embedder: &'a E, reader: &'a R, replacer: &'a S, config: PipelineConfig) -> Self {
        Self {
            embedder,
            reader,
            replacer,
            config,
        }
    }

    /// Ingests one PDF source from its loaded extraction.
    ///
    /// The guard runs first. A rejection returns
    /// [`IngestOutcome::Rejected`] without any embedding call or store write.
    pub fn ingest_pdf(
        &self,
        loaded: Result<PdfExtraction, PdfError>,
        source: &str,
        content_hash: &str,
    ) -> Result<IngestOutcome, PipelineError> {
        let chunks = match gate_pdf(
            loaded,
            source,
            &self.config.language,
            self.config.chunker.size,
            self.config.chunker.overlap,
        ) {
            Ok(chunks) => chunks,
            Err(rejection) => return Ok(IngestOutcome::Rejected(rejection)),
        };
        self.after_guard(source, content_hash, chunks)
    }

    /// Ingests one PDF source from disk.
    pub fn ingest_pdf_path(
        &self,
        path: &std::path::Path,
        source: &str,
        content_hash: &str,
    ) -> Result<IngestOutcome, PipelineError> {
        self.ingest_pdf(load_pdf(path), source, content_hash)
    }

    /// Ingests one source from disk, dispatching on the loader the baseline's
    /// registry selects: `.pdf` through the PDF text-layer loader, `.md` and
    /// `.markdown` through the Markdown section parser, and `.txt` through the
    /// plain-text loader. An unknown extension is rejected before any effect.
    ///
    /// Markdown and plain text are page-less, so they pass through
    /// [`gate_sections`]; the PDF path keeps [`gate_pdf`] and its incomplete-page
    /// rejection. Both paths converge on [`Self::after_guard`], so chunking,
    /// embedding, fingerprinting, and transactional replacement are identical.
    pub fn ingest_path(
        &self,
        path: &Path,
        source: &str,
        content_hash: &str,
    ) -> Result<IngestOutcome, PipelineError> {
        let extension = path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "pdf" => self.ingest_pdf_path(path, source, content_hash),
            "md" | "markdown" => match std::fs::read_to_string(path) {
                Ok(text) => {
                    let sections = load_markdown(&text);
                    self.ingest_sections(sections, source, content_hash)
                }
                Err(err) => Ok(IngestOutcome::Rejected(IngestRejection::Loader(format!(
                    "read {}: {err}",
                    path.display()
                )))),
            },
            "txt" => match load_text(path) {
                Ok(sections) => self.ingest_sections(sections, source, content_hash),
                Err(err) => Ok(IngestOutcome::Rejected(IngestRejection::Loader(format!(
                    "read {}: {err}",
                    path.display()
                )))),
            },
            _ => Ok(IngestOutcome::Rejected(IngestRejection::Loader(format!(
                "no loader for {}",
                path.display()
            )))),
        }
    }

    /// Ingests page-less sections (Markdown/plain text) through the same
    /// pre-effect gate and post-guard path as a PDF.
    fn ingest_sections(
        &self,
        sections: Vec<crate::document::Section>,
        source: &str,
        content_hash: &str,
    ) -> Result<IngestOutcome, PipelineError> {
        match gate_sections(
            sections,
            source,
            &self.config.language,
            self.config.chunker.size,
            self.config.chunker.overlap,
        ) {
            Ok(chunks) => self.after_guard(source, content_hash, chunks),
            Err(rejection) => Ok(IngestOutcome::Rejected(rejection)),
        }
    }

    /// The steps after the guard: unchanged check, embedding, replacement.
    fn after_guard(
        &self,
        source: &str,
        content_hash: &str,
        chunks: Vec<Chunk>,
    ) -> Result<IngestOutcome, PipelineError> {
        let decision = unchanged_source(self.reader, source, content_hash).map_err(|_| {
            PipelineError::Fingerprint(format!("unchanged-source read failed for {source}"))
        })?;
        if let UnchangedSource::Unchanged { rows } = decision {
            return Ok(IngestOutcome::Unchanged {
                fingerprint: content_hash.to_string(),
                rows,
            });
        }

        let ingested_at = ingested_at_now();
        let mut stored = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            let embedding = self.embedder.embed(&chunk.content)?;
            stored.push(StoredChunk {
                content: chunk.content.clone(),
                source: Some(chunk.source.clone()),
                section: Some(chunk.section.clone()),
                chunk_index: chunk.chunk_index as i32,
                language: Some(chunk.language.clone()),
                page: chunk.page.map(|page| page as i32),
                corpus: Some(self.config.corpus.clone()),
                content_hash: content_hash.to_string(),
                embed_model: self.config.embed_model.clone(),
                embedding_dim: embedding.len() as i32,
                chunker_config: self.config.chunker.canonical(),
                ingested_at: ingested_at.clone(),
                embedding,
            });
        }

        // Fail before the transaction starts when an embedding width is wrong.
        validate_all_dims(&stored).map_err(PipelineError::Replace)?;
        self.replacer.replace_source(source, &stored)?;
        Ok(IngestOutcome::Replaced {
            fingerprint: content_hash.to_string(),
            chunks: stored.len(),
        })
    }
}

/// Returns the current UTC time as an RFC 3339 timestamp, or the Unix epoch
/// when the clock is before it.
fn ingested_at_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339(seconds)
}

/// Formats `seconds` since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_rfc3339(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let secs_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Converts days since the Unix epoch into `(year, month, day)`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A recording [`Embedder`] stub: returns a fixed-width zero vector and counts
/// every call.
#[derive(Debug)]
pub struct RecordingEmbedder {
    dim: usize,
    calls: AtomicUsize,
}

impl RecordingEmbedder {
    /// Builds an embedder that returns `dim`-wide vectors.
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            calls: AtomicUsize::new(0),
        }
    }

    /// Returns how many embedding calls were made.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Embedder for RecordingEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f64>, EmbeddingError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![0.5; self.dim])
    }
}

/// Computes the pipeline fingerprint for a byte payload and configuration.
///
pub fn source_fingerprint(bytes: &[u8], config: &ChunkerConfig) -> String {
    fingerprint(bytes, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StoredChunk;
    use crate::storage::EMBEDDING_DIM;
    use crate::storage_pg::StoredChunkRow;
    use crate::transaction::RecordingSourceReplacer;
    use std::cell::RefCell;

    /// An empty read port: every source reads as changed.
    struct NoRows;

    impl FingerprintReader for NoRows {
        type Error = String;

        fn rows_for_hash(
            &self,
            _content_hash: &str,
            _source: &str,
        ) -> Result<Vec<StoredChunkRow>, String> {
            Ok(Vec::new())
        }
    }

    /// A read port that returns one stored row carrying `hash` for `source`.
    struct StoredRows {
        hash: String,
        source: String,
        queries: RefCell<usize>,
    }

    impl FingerprintReader for StoredRows {
        type Error = String;

        fn rows_for_hash(
            &self,
            content_hash: &str,
            source: &str,
        ) -> Result<Vec<StoredChunkRow>, String> {
            *self.queries.borrow_mut() += 1;
            if content_hash == self.hash && source == self.source {
                Ok(vec![stored_row(&self.hash, &self.source)])
            } else {
                Ok(Vec::new())
            }
        }
    }

    fn stored_row(hash: &str, source: &str) -> StoredChunkRow {
        StoredChunkRow {
            id: 1,
            chunk: StoredChunk {
                content: "old body".to_string(),
                source: Some(source.to_string()),
                section: Some("Intro".to_string()),
                chunk_index: 0,
                language: Some("english".to_string()),
                page: None,
                corpus: Some("trusted".to_string()),
                content_hash: hash.to_string(),
                embed_model: "nomic-embed-text".to_string(),
                embedding_dim: EMBEDDING_DIM as i32,
                chunker_config: "size=50,overlap=20".to_string(),
                ingested_at: "2024-01-01T00:00:00Z".to_string(),
                embedding: vec![0.0; EMBEDDING_DIM],
            },
        }
    }

    fn config() -> PipelineConfig {
        PipelineConfig::new(
            "english",
            ChunkerConfig::new(50, 20),
            "nomic-embed-text",
            "trusted",
        )
    }

    fn inputs() -> std::path::PathBuf {
        std::path::PathBuf::from("fixtures/parity/inputs")
    }

    #[test]
    fn rejected_mixed_pdf_makes_no_embedding_or_store_calls() {
        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = NoRows;
        let replacer = RecordingSourceReplacer::new();
        let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

        let outcome = pipeline
            .ingest_pdf_path(&inputs().join("mixed-text-scan.pdf"), "mixed.pdf", "hash")
            .unwrap();

        assert_eq!(
            outcome,
            IngestOutcome::Rejected(IngestRejection::Incomplete { pages: vec![2] })
        );
        assert_eq!(embedder.calls(), 0, "no embedding call on rejection");
        assert!(replacer.is_empty());
        assert_eq!(replacer.counts().replacements, 0);
        assert_eq!(replacer.counts().rows_written, 0);
    }

    #[test]
    fn scanned_pdf_makes_no_embedding_or_store_calls() {
        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = NoRows;
        let replacer = RecordingSourceReplacer::new();
        let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

        let outcome = pipeline
            .ingest_pdf_path(&inputs().join("no-text-layer.pdf"), "scanned.pdf", "hash")
            .unwrap();

        assert!(matches!(
            outcome,
            IngestOutcome::Rejected(IngestRejection::Loader(_))
        ));
        assert_eq!(embedder.calls(), 0);
        assert_eq!(replacer.counts().replacements, 0);
    }

    #[test]
    fn fatal_loader_error_makes_no_embedding_or_store_calls() {
        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = NoRows;
        let replacer = RecordingSourceReplacer::new();
        let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

        let outcome = pipeline
            .ingest_pdf_path(&inputs().join("missing.pdf"), "missing.pdf", "hash")
            .unwrap();

        assert!(matches!(
            outcome,
            IngestOutcome::Rejected(IngestRejection::Loader(_))
        ));
        assert_eq!(embedder.calls(), 0);
        assert_eq!(replacer.counts().replacements, 0);
    }

    #[test]
    fn unchanged_source_makes_no_embedding_or_replacement_calls() {
        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = StoredRows {
            hash: "hash".to_string(),
            source: "text-layer.pdf".to_string(),
            queries: RefCell::new(0),
        };
        let replacer = RecordingSourceReplacer::new();
        let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

        let outcome = pipeline
            .ingest_pdf_path(&inputs().join("text-layer.pdf"), "text-layer.pdf", "hash")
            .unwrap();

        assert_eq!(
            outcome,
            IngestOutcome::Unchanged {
                fingerprint: "hash".to_string(),
                rows: 1
            }
        );
        assert_eq!(embedder.calls(), 0, "unchanged source is not re-embedded");
        assert_eq!(replacer.counts().replacements, 0);
        assert_eq!(*reader.queries.borrow(), 1);
    }

    #[test]
    fn changed_source_embeds_each_chunk_then_replaces_once() {
        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = NoRows;
        let replacer = RecordingSourceReplacer::new();
        let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

        let outcome = pipeline
            .ingest_pdf_path(&inputs().join("text-layer.pdf"), "text-layer.pdf", "hash")
            .unwrap();

        let IngestOutcome::Replaced { chunks, .. } = outcome else {
            panic!("expected a replacement, got {outcome:?}");
        };
        assert_eq!(embedder.calls(), chunks);
        assert_eq!(replacer.counts().replacements, 1);
        assert_eq!(replacer.counts().rows_written, chunks);
        assert_eq!(replacer.rows_for("text-layer.pdf").len(), chunks);
    }

    #[test]
    fn markdown_source_is_chunked_and_embedded_without_a_page() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.md");
        std::fs::write(&path, "## Intro\nalpha beta\n## Fees\ngamma delta").unwrap();

        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = NoRows;
        let replacer = RecordingSourceReplacer::new();
        let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

        let outcome = pipeline.ingest_path(&path, "policy.md", "hash").unwrap();
        let IngestOutcome::Replaced { chunks, .. } = outcome else {
            panic!("expected a replacement, got {outcome:?}");
        };
        assert_eq!(chunks, 2);
        assert_eq!(embedder.calls(), 2);
        assert_eq!(replacer.counts().replacements, 1);
        let rows = replacer.rows_for("policy.md");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].section.as_deref(), Some("Intro"));
        assert_eq!(rows[1].section.as_deref(), Some("Fees"));
        assert!(
            rows.iter().all(|row| row.page.is_none()),
            "Markdown rows stay page-less"
        );
        assert_eq!(rows[0].chunk_index, 0);
        assert_eq!(rows[1].chunk_index, 1);
        assert_eq!(rows[0].language.as_deref(), Some("english"));
    }

    #[test]
    fn plain_text_source_is_one_page_less_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "## not a heading\nplain body").unwrap();

        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = NoRows;
        let replacer = RecordingSourceReplacer::new();
        let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

        let outcome = pipeline.ingest_path(&path, "note.txt", "hash").unwrap();
        let IngestOutcome::Replaced { chunks, .. } = outcome else {
            panic!("expected a replacement, got {outcome:?}");
        };
        assert_eq!(chunks, 1);
        let rows = replacer.rows_for("note.txt");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].section.as_deref(), Some("note.txt"));
        assert_eq!(rows[0].content, "## not a heading plain body");
        assert!(rows[0].page.is_none());
    }

    #[test]
    fn empty_markdown_is_rejected_without_effects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.md");
        std::fs::write(&path, "   \n").unwrap();

        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = NoRows;
        let replacer = RecordingSourceReplacer::new();
        let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

        let outcome = pipeline.ingest_path(&path, "empty.md", "hash").unwrap();
        assert_eq!(
            outcome,
            IngestOutcome::Rejected(IngestRejection::NoChunks {
                source: "empty.md".to_string()
            })
        );
        assert_eq!(embedder.calls(), 0);
        assert_eq!(replacer.counts().replacements, 0);
    }

    #[test]
    fn unknown_extension_is_rejected_without_effects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("document.bin");
        std::fs::write(&path, "bytes").unwrap();

        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = NoRows;
        let replacer = RecordingSourceReplacer::new();
        let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

        let outcome = pipeline.ingest_path(&path, "document.bin", "hash").unwrap();
        match outcome {
            IngestOutcome::Rejected(IngestRejection::Loader(message)) => {
                assert!(message.contains("no loader for"), "{message}");
            }
            other => panic!("expected a loader rejection, got {other:?}"),
        }
        assert_eq!(embedder.calls(), 0);
        assert_eq!(replacer.counts().replacements, 0);
    }

    #[test]
    fn rfc3339_format_matches_known_epochs() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(1_704_067_200), "2024-01-01T00:00:00Z");
    }
}
