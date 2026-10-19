//! Pre-effect ingest guard.
//!
//! Ports the Go caller's decision (`cmd/ingest.loadChunks` + `run`): a source
//! must be resolved to chunks **before** any embedding call or database write,
//! and a loader outcome that cannot be ingested must be rejected first so stored
//! data is never touched. Specifically, matching the reference:
//!
//! * a fatal loader error (unreadable/malformed PDF) is rejected;
//! * a fully scanned PDF (every page missing a text layer) is rejected;
//! * a partial PDF extraction is rejected, reporting the pages that need OCR;
//! * a source that yields no chunks is rejected (the baseline's
//!   "no sections found in <path>").
//!
//! This module is deliberately DB-free: it holds only the decision, so the
//! embedding and database effects remain in the caller (RUST-005B's storage port).

use std::fmt;

use crate::chunker::chunk_sections;
use crate::document::{Chunk, Section};
use crate::loader::{PdfError, PdfExtraction};

/// Why an ingest was refused before any embedding or database write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestRejection {
    /// A fatal loader error (unreadable/malformed PDF, or a fully scanned one).
    Loader(String),
    /// A partial PDF extraction: the listed 1-based pages have no text layer.
    Incomplete { pages: Vec<u32> },
    /// The source yielded no chunks.
    NoChunks { source: String },
}

impl fmt::Display for IngestRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IngestRejection::Loader(message) => write!(f, "load failed: {message}"),
            IngestRejection::Incomplete { pages } => write!(
                f,
                "incomplete PDF extraction: page(s) {} have no text layer; OCR is required",
                pages
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            IngestRejection::NoChunks { source } => write!(f, "no sections found in {source}"),
        }
    }
}

impl std::error::Error for IngestRejection {}

/// The ingest caller's pre-effect gate for a PDF source.
///
/// Returns the chunks to embed and store, or a rejection. A partial extraction is
/// rejected with its affected pages, matching Go's caller, which fails on the
/// loader's `*IncompleteExtractionError` and does not ingest the readable pages.
pub fn gate_pdf(
    loaded: Result<PdfExtraction, PdfError>,
    source: &str,
    language: &str,
    chunk_size: usize,
    overlap: usize,
) -> Result<Vec<Chunk>, IngestRejection> {
    let extraction = loaded.map_err(|err| IngestRejection::Loader(err.to_string()))?;
    if !extraction.incomplete_pages.is_empty() {
        return Err(IngestRejection::Incomplete {
            pages: extraction.incomplete_pages,
        });
    }
    gate_sections(extraction.sections, source, language, chunk_size, overlap)
}

/// The ingest caller's pre-effect gate for a page-less source (Markdown/text).
///
/// Rejects a source that yields no chunks, mirroring the baseline's explicit
/// "no sections found" error rather than letting an empty ingest reach the
/// database and delete a source's rows.
pub fn gate_sections(
    sections: Vec<Section>,
    source: &str,
    language: &str,
    chunk_size: usize,
    overlap: usize,
) -> Result<Vec<Chunk>, IngestRejection> {
    let chunks = chunk_sections(&sections, source, language, chunk_size, overlap);
    if chunks.is_empty() {
        return Err(IngestRejection::NoChunks {
            source: source.to_string(),
        });
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::load_pdf;

    fn inputs() -> std::path::PathBuf {
        std::path::PathBuf::from("fixtures/parity/inputs")
    }

    #[test]
    fn complete_pdf_passes_the_gate() {
        let chunks = gate_pdf(
            load_pdf(inputs().join("text-layer.pdf")),
            "text-layer.pdf",
            "english",
            50,
            20,
        )
        .expect("complete PDF should pass");
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn partial_pdf_is_rejected_with_its_pages() {
        let err = gate_pdf(
            load_pdf(inputs().join("mixed-text-scan.pdf")),
            "mixed.pdf",
            "english",
            50,
            20,
        )
        .unwrap_err();
        assert_eq!(err, IngestRejection::Incomplete { pages: vec![2] });
        assert!(err.to_string().contains("page(s) 2"));
    }

    #[test]
    fn fully_scanned_pdf_is_a_fatal_rejection() {
        let err = gate_pdf(
            load_pdf(inputs().join("no-text-layer.pdf")),
            "scanned.pdf",
            "english",
            50,
            20,
        )
        .unwrap_err();
        assert!(matches!(err, IngestRejection::Loader(_)));
    }

    #[test]
    fn empty_source_is_rejected() {
        let err = gate_sections(Vec::new(), "empty.md", "english", 50, 20).unwrap_err();
        assert_eq!(
            err,
            IngestRejection::NoChunks {
                source: "empty.md".to_string()
            }
        );
    }
}
