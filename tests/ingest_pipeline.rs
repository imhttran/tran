//! Caller-level verification of the ingest pipeline.
//!
//! Runs fixtures through the public [`rag_foundation::pipeline::IngestPipeline`]
//! with recording embedding and store fakes, proving the PRD requirements:
//!
//! * a mixed text/scanned PDF causes no embedding calls and no database writes;
//! * a repeated ingestion of an unchanged source makes no writes and preserves
//!   the stored rows;
//! * a forced failure during replacement rolls back the complete transaction,
//!   so the existing stored data survives.

use std::cell::RefCell;
use std::path::PathBuf;

use rag_foundation::fingerprint::{fingerprint, ChunkerConfig, FingerprintReader};
use rag_foundation::ingest::IngestRejection;
use rag_foundation::pipeline::{IngestOutcome, IngestPipeline, PipelineConfig, RecordingEmbedder};
use rag_foundation::storage::{StoredChunk, EMBEDDING_DIM};
use rag_foundation::storage_pg::StoredChunkRow;
use rag_foundation::transaction::RecordingSourceReplacer;

fn inputs() -> PathBuf {
    PathBuf::from("fixtures/parity/inputs")
}

fn config() -> PipelineConfig {
    PipelineConfig::new(
        "english",
        ChunkerConfig::new(50, 20),
        "nomic-embed-text",
        "trusted",
    )
}

/// A read port backed by an in-memory row table that records its queries.
#[derive(Default)]
struct FakeReader {
    rows: RefCell<Vec<StoredChunkRow>>,
    queries: RefCell<usize>,
}

impl FakeReader {
    fn with_rows(rows: Vec<StoredChunkRow>) -> Self {
        Self {
            rows: RefCell::new(rows),
            queries: RefCell::new(0),
        }
    }

    fn queries(&self) -> usize {
        *self.queries.borrow()
    }
}

impl FingerprintReader for FakeReader {
    type Error = String;

    fn rows_for_hash(
        &self,
        content_hash: &str,
        source: &str,
    ) -> Result<Vec<StoredChunkRow>, String> {
        *self.queries.borrow_mut() += 1;
        Ok(self
            .rows
            .borrow()
            .iter()
            .filter(|row| {
                row.chunk.content_hash == content_hash
                    && row.chunk.source.as_deref() == Some(source)
            })
            .cloned()
            .collect())
    }
}

fn stored_row(hash: &str, source: &str) -> StoredChunkRow {
    StoredChunkRow {
        id: 1,
        chunk: StoredChunk {
            content: "previously stored body".to_string(),
            source: Some(source.to_string()),
            section: Some("Intro".to_string()),
            chunk_index: 0,
            language: Some("english".to_string()),
            page: Some(1),
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

#[test]
fn mixed_text_scan_pdf_makes_no_embedding_calls_and_no_writes() {
    let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
    let reader = FakeReader::default();
    let replacer = RecordingSourceReplacer::new();
    let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

    let outcome = pipeline
        .ingest_pdf_path(&inputs().join("mixed-text-scan.pdf"), "mixed.pdf", "hash")
        .expect("the rejection is an outcome, not an error");

    assert_eq!(
        outcome,
        IngestOutcome::Rejected(IngestRejection::Incomplete { pages: vec![2] }),
        "the affected page is reported"
    );
    assert_eq!(
        embedder.calls(),
        0,
        "no embedding call on a rejected source"
    );
    assert_eq!(reader.queries(), 0, "the unchanged read is not reached");
    let counts = replacer.counts();
    assert_eq!(counts.replacements, 0, "zero database writes");
    assert_eq!(counts.rows_written, 0, "zero rows written");
    assert!(replacer.is_empty());
}

#[test]
fn scanned_pdf_and_fatal_loader_error_make_no_effects() {
    let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
    let reader = FakeReader::default();
    let replacer = RecordingSourceReplacer::new();
    let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

    let scanned = pipeline
        .ingest_pdf_path(&inputs().join("no-text-layer.pdf"), "scanned.pdf", "hash")
        .unwrap();
    assert!(matches!(
        scanned,
        IngestOutcome::Rejected(IngestRejection::Loader(_))
    ));

    let missing = pipeline
        .ingest_pdf_path(&inputs().join("missing.pdf"), "missing.pdf", "hash")
        .unwrap();
    assert!(matches!(
        missing,
        IngestOutcome::Rejected(IngestRejection::Loader(_))
    ));

    assert_eq!(embedder.calls(), 0);
    assert_eq!(replacer.counts().replacements, 0);
    assert!(replacer.is_empty());
}

#[test]
fn repeated_ingestion_of_an_unchanged_source_preserves_stored_data() {
    let bytes = std::fs::read(inputs().join("text-layer.pdf")).unwrap();
    let content_hash = fingerprint(&bytes, &ChunkerConfig::new(50, 20));

    // First run: the store is empty for this source, so the source changed and
    // the pipeline embeds and replaces once.
    let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
    let empty = FakeReader::default();
    let replacer = RecordingSourceReplacer::new();
    let pipeline = IngestPipeline::new(&embedder, &empty, &replacer, config());
    let first = pipeline
        .ingest_pdf_path(
            &inputs().join("text-layer.pdf"),
            "text-layer.pdf",
            &content_hash,
        )
        .unwrap();
    let IngestOutcome::Replaced { chunks, .. } = first else {
        panic!("first ingest must replace, got {first:?}");
    };
    assert!(chunks > 0);
    assert_eq!(embedder.calls(), chunks);
    assert_eq!(replacer.counts().replacements, 1);
    let stored_before = replacer.rows_for("text-layer.pdf");
    assert_eq!(stored_before.len(), chunks);

    // Second run: the stored fingerprint matches, so the pipeline skips
    // re-embedding and does not touch the stored rows.
    let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
    let stored_rows = FakeReader::with_rows(vec![stored_row(&content_hash, "text-layer.pdf")]);
    let pipeline = IngestPipeline::new(&embedder, &stored_rows, &replacer, config());
    let second = pipeline
        .ingest_pdf_path(
            &inputs().join("text-layer.pdf"),
            "text-layer.pdf",
            &content_hash,
        )
        .unwrap();

    assert_eq!(
        second,
        IngestOutcome::Unchanged {
            fingerprint: content_hash.clone(),
            rows: 1
        }
    );
    assert_eq!(embedder.calls(), 0, "unchanged source is not re-embedded");
    assert_eq!(replacer.counts().replacements, 1, "no second replacement");
    assert_eq!(
        replacer.rows_for("text-layer.pdf"),
        stored_before,
        "stored data is preserved by a repeated ingestion"
    );
}

#[test]
fn forced_replacement_failure_rolls_back_and_preserves_stored_data() {
    let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
    let reader = FakeReader::default();
    let replacer = RecordingSourceReplacer::new();
    let pipeline = IngestPipeline::new(&embedder, &reader, &replacer, config());

    // Seed the store with an earlier ingest of the same source.
    pipeline
        .ingest_pdf_path(
            &inputs().join("text-layer.pdf"),
            "text-layer.pdf",
            "old-hash",
        )
        .unwrap();
    let before = replacer.rows_for("text-layer.pdf");
    assert!(!before.is_empty());

    // Force a failure after the first insert of the next replacement.
    replacer.inject_failure_after(1);
    let result = pipeline.ingest_pdf_path(
        &inputs().join("text-layer.pdf"),
        "text-layer.pdf",
        "new-hash",
    );

    assert!(result.is_err(), "the injected failure surfaces as an error");
    assert_eq!(
        replacer.rows_for("text-layer.pdf"),
        before,
        "rollback leaves the prior rows unchanged"
    );
    assert_eq!(replacer.counts().rollbacks, 1);
    assert_eq!(replacer.counts().replacements, 1);
}
