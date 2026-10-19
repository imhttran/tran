//! Caller-level integration check for RUST-005B: Rust ingestion must reject an
//! incomplete PDF extraction (and a fully scanned PDF) *before* any embedding
//! call or database write, so existing stored data is preserved.
//!
//! The guard under test is `rag_foundation::ingest::gate_pdf`. This test drives it
//! through a caller that would otherwise embed every chunk and write to storage,
//! and asserts the effects never happen for a mixed text/scanned PDF.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use rag_foundation::embedding::{Embedder, EmbeddingError};
use rag_foundation::ingest::{gate_pdf, IngestRejection};
use rag_foundation::loader::{load_pdf, PdfError, PdfExtraction};

/// Records embedding calls, to prove none happen on rejection. Atomics keep it
/// `Send + Sync`, as the `Embedder` trait requires.
struct RecordingEmbedder {
    calls: AtomicUsize,
}

impl Embedder for RecordingEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f64>, EmbeddingError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(vec![0.0; 3])
    }
}

/// Records database writes, to prove none happen on rejection.
#[derive(Default)]
struct RecordingStore {
    writes: AtomicUsize,
}

impl RecordingStore {
    fn replace_source(&self, _chunks: usize) {
        self.writes.fetch_add(1, Ordering::Relaxed);
    }
}

/// Models the ingest caller: resolve chunks through the gate first, then (only on
/// success) embed each chunk and write them.
fn ingest(
    loaded: Result<PdfExtraction, PdfError>,
    embedder: &RecordingEmbedder,
    store: &RecordingStore,
) -> Result<usize, IngestRejection> {
    let chunks = gate_pdf(loaded, "doc.pdf", "english", 50, 20)?;
    for chunk in &chunks {
        let _ = embedder.embed(&chunk.content);
    }
    store.replace_source(chunks.len());
    Ok(chunks.len())
}

fn inputs() -> PathBuf {
    PathBuf::from("fixtures/parity/inputs")
}

#[test]
fn mixed_text_scanned_pdf_causes_no_embedding_calls_or_writes() {
    let embedder = RecordingEmbedder {
        calls: AtomicUsize::new(0),
    };
    let store = RecordingStore::default();

    let result = ingest(
        load_pdf(inputs().join("mixed-text-scan.pdf")),
        &embedder,
        &store,
    );

    match result {
        Err(IngestRejection::Incomplete { pages }) => {
            assert_eq!(pages, vec![2], "affected pages must be reported");
        }
        other => panic!("mixed PDF must be rejected as incomplete, got {other:?}"),
    }
    assert_eq!(
        embedder.calls.load(Ordering::Relaxed),
        0,
        "no embedding calls on rejection"
    );
    assert_eq!(
        store.writes.load(Ordering::Relaxed),
        0,
        "no database writes on rejection"
    );
}

#[test]
fn fully_scanned_pdf_causes_no_embedding_calls_or_writes() {
    let embedder = RecordingEmbedder {
        calls: AtomicUsize::new(0),
    };
    let store = RecordingStore::default();

    let result = ingest(
        load_pdf(inputs().join("no-text-layer.pdf")),
        &embedder,
        &store,
    );

    assert!(matches!(result, Err(IngestRejection::Loader(_))));
    assert_eq!(embedder.calls.load(Ordering::Relaxed), 0);
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
}

#[test]
fn complete_pdf_embeds_and_writes_control_case() {
    // Proves the recording harness is not vacuously zero.
    let embedder = RecordingEmbedder {
        calls: AtomicUsize::new(0),
    };
    let store = RecordingStore::default();

    let written = ingest(load_pdf(inputs().join("text-layer.pdf")), &embedder, &store).unwrap();
    assert_eq!(written, 2);
    assert_eq!(
        embedder.calls.load(Ordering::Relaxed),
        2,
        "one embedding per chunk"
    );
    assert_eq!(
        store.writes.load(Ordering::Relaxed),
        1,
        "one replace on success"
    );
}
