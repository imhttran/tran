//! PDF text-layer loader.
//!
//! Ported from the Go baseline PDF loader (`internal/loader/file_pdf.go`). The
//! baseline emits one [`Section`] per page with 1-based page provenance
//! (`page: Some(n)`) and title `Page N`, and it never silently treats a missing
//! text layer as empty content:
//!
//! * a hard failure (unreadable/malformed PDF, or a page whose object is null)
//!   is [`PdfError::Invalid`];
//! * a document whose pages all lack a text layer is [`PdfError::Scanned`];
//! * a partial extraction returns the readable sections **and** names the pages
//!   that require OCR in [`PdfExtraction::incomplete_pages`], matching Go's
//!   `*IncompleteExtractionError`.
//!
//! Extraction differences (whitespace, ordering) are normalized by trimming each
//! page's text and dropping whitespace-only pages, in line with the baseline.

use std::path::Path;

use thiserror::Error;

use crate::document::Section;

/// Errors that reject a PDF outright (no readable sections are returned).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PdfError {
    /// The input is not a readable PDF document, or a page's text could not be
    /// extracted (the baseline's open/parse/extract failures).
    #[error("pdf: not a valid PDF document: {0}")]
    Invalid(String),
    /// Every page lacks an extractable text layer (scanned / image-only).
    #[error("pdf: no extractable text layer (scanned PDF is not supported)")]
    Scanned,
}

/// The result of a successful extraction.
///
/// `incomplete_pages` carries the 1-based pages whose text layer was empty while
/// other pages had text — the baseline's `IncompleteExtractionError` case. It is
/// empty for a fully readable document. The readable sections are always
/// preserved, so a partially scanned PDF yields its readable pages rather than
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfExtraction {
    /// One section per page that had text, titled `Page N`, in page order.
    pub sections: Vec<Section>,
    /// 1-based pages that had no text layer (empty for a complete extraction).
    pub incomplete_pages: Vec<u32>,
}

impl PdfExtraction {
    /// Reports whether every page was extracted (no page needs OCR).
    pub fn is_complete(&self) -> bool {
        self.incomplete_pages.is_empty()
    }
}

/// Loads a text-layer PDF from `path`.
///
/// Each returned section is titled `Page N` and carries `page: Some(n)` with a
/// 1-based page number in document order, matching the Go reference. Pages with
/// no extractable text are reported in [`PdfExtraction::incomplete_pages`]; a
/// document whose text layer is entirely empty is rejected with
/// [`PdfError::Scanned`].
pub fn load_pdf(path: impl AsRef<Path>) -> Result<PdfExtraction, PdfError> {
    let bytes = std::fs::read(path.as_ref()).map_err(|err| PdfError::Invalid(err.to_string()))?;
    load_pdf_bytes(&bytes)
}

/// Loads a text-layer PDF from an in-memory byte buffer.
pub fn load_pdf_bytes(bytes: &[u8]) -> Result<PdfExtraction, PdfError> {
    if bytes.is_empty() {
        return Err(PdfError::Invalid("empty input".to_string()));
    }

    let doc = lopdf::Document::load_mem(bytes).map_err(|err| PdfError::Invalid(err.to_string()))?;

    let pages = doc.get_pages();
    if pages.is_empty() {
        return Err(PdfError::Invalid("document has no pages".to_string()));
    }

    let mut sections = Vec::new();
    let mut incomplete_pages: Vec<u32> = Vec::new();

    for (page_number, _page_id) in pages {
        let text = doc
            .extract_text(&[page_number])
            .map_err(|err| PdfError::Invalid(err.to_string()))?;
        let text = normalize_text(&text);
        if text.is_empty() {
            incomplete_pages.push(page_number);
            continue;
        }
        sections.push(Section::with_page(
            format!("Page {page_number}"),
            text,
            page_number,
        ));
    }

    // Every page missing a text layer: reject outright, matching the baseline's
    // "has no extractable text layer: OCR is required".
    if sections.is_empty() {
        return Err(PdfError::Scanned);
    }

    Ok(PdfExtraction {
        sections,
        incomplete_pages,
    })
}

/// Normalizes extracted page text: trims surrounding whitespace, resolving common
/// extraction differences against the baseline fixtures.
fn normalize_text(raw: &str) -> String {
    raw.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_rejected() {
        assert_eq!(
            load_pdf_bytes(&[]),
            Err(PdfError::Invalid("empty input".to_string()))
        );
    }

    #[test]
    fn non_pdf_input_is_rejected() {
        let err = load_pdf_bytes(b"this is not a pdf").unwrap_err();
        assert!(matches!(err, PdfError::Invalid(_)));
    }

    #[test]
    fn full_text_layer_pdf_is_complete() {
        let extraction = load_pdf("fixtures/parity/inputs/text-layer.pdf").unwrap();
        assert!(extraction.is_complete());
        assert_eq!(extraction.sections.len(), 2);
        assert_eq!(extraction.sections[0].title, "Page 1");
        assert_eq!(extraction.sections[0].page, Some(1));
    }

    #[test]
    fn partially_scanned_pdf_reports_incomplete_pages() {
        let extraction = load_pdf("fixtures/parity/inputs/mixed-text-scan.pdf").unwrap();
        assert_eq!(extraction.sections.len(), 1);
        assert_eq!(extraction.sections[0].title, "Page 1");
        assert_eq!(extraction.incomplete_pages, vec![2]);
    }

    #[test]
    fn fully_scanned_pdf_is_rejected() {
        assert_eq!(
            load_pdf("fixtures/parity/inputs/no-text-layer.pdf"),
            Err(PdfError::Scanned)
        );
    }

    #[test]
    fn scanned_pdf_rejection_message_matches_baseline() {
        assert_eq!(
            PdfError::Scanned.to_string(),
            "pdf: no extractable text layer (scanned PDF is not supported)"
        );
    }
}
