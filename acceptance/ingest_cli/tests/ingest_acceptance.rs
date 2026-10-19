//! Acceptance tests for the production ingestion service seam (RUST-005E).
//!
//! These tests drive the callable production service
//! [`rag_foundation::service::IngestService`] with the mixed text/scanned PDF
//! and the fully scanned PDF. They assert zero embedding calls, zero database
//! writes, the reported affected pages, and that previously stored data is
//! preserved. No pipeline is reimplemented here.

use std::sync::atomic::{AtomicUsize, Ordering};

use ingest_cli_acceptance::fixture;
use rag_foundation::embedding::{Embedder, EmbeddingError};
use rag_foundation::fingerprint::{ChunkerConfig, FingerprintReader};
use rag_foundation::ingest::IngestRejection;
use rag_foundation::pipeline::{IngestOutcome, PipelineConfig};
use rag_foundation::service::IngestService;
use rag_foundation::storage::EMBEDDING_DIM;
use rag_foundation::storage_pg::StoredChunkRow;
use rag_foundation::transaction::RecordingSourceReplacer;

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

/// An embedder that counts calls and returns fixed-width zero vectors.
struct CountingEmbedder {
    dim: usize,
    calls: AtomicUsize,
}

impl CountingEmbedder {
    fn new(dim: usize) -> Self {
        Self {
            dim,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Embedder for CountingEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f64>, EmbeddingError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![0.5; self.dim])
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

#[test]
fn mixed_text_scanned_pdf_rejects_with_reported_pages_and_zero_effects() {
    let embedder = CountingEmbedder::new(EMBEDDING_DIM);
    let reader = NoRows;
    let replacer = RecordingSourceReplacer::new();
    let service = IngestService::new(&embedder, &reader, &replacer, config());

    let outcome = service
        .ingest_source(&fixture("mixed-text-scan.pdf"), "mixed.pdf", "hash")
        .expect("rejection is not a pipeline error");

    assert_eq!(
        outcome,
        IngestOutcome::Rejected(IngestRejection::Incomplete { pages: vec![2] }),
        "the mixed PDF must report affected page 2"
    );
    assert_eq!(embedder.calls(), 0, "zero embedding calls on rejection");
    assert_eq!(replacer.counts().replacements, 0, "zero replacements");
    assert_eq!(replacer.counts().rows_written, 0, "zero rows written");
    assert!(replacer.is_empty(), "no stored data was touched");
}

#[test]
fn fully_scanned_pdf_rejects_as_loader_with_zero_effects() {
    let embedder = CountingEmbedder::new(EMBEDDING_DIM);
    let reader = NoRows;
    let replacer = RecordingSourceReplacer::new();
    let service = IngestService::new(&embedder, &reader, &replacer, config());

    let outcome = service
        .ingest_source(&fixture("no-text-layer.pdf"), "scanned.pdf", "hash")
        .expect("rejection is not a pipeline error");

    assert!(
        matches!(outcome, IngestOutcome::Rejected(IngestRejection::Loader(_))),
        "the scanned PDF must be a loader rejection, got {outcome:?}"
    );
    assert_eq!(embedder.calls(), 0, "zero embedding calls on rejection");
    assert_eq!(replacer.counts().replacements, 0, "zero replacements");
    assert_eq!(replacer.counts().rows_written, 0, "zero rows written");
    assert!(replacer.is_empty(), "no stored data was touched");
}
