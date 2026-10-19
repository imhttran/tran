//! Document loaders: read Markdown, plain text, and PDF into [`Section`] values.
//!
//! Ported from the Go baseline loaders in `internal/document/`. Markdown and
//! plain text produce page-less sections (`page: None`); the PDF loader produces
//! one section per page with 1-based `page` provenance and rejects scanned
//! (text-less) PDFs explicitly.

pub mod markdown;
pub mod pdf;
pub mod text;

pub use markdown::load_markdown;
pub use pdf::{load_pdf, PdfError, PdfExtraction};
pub use text::load_text;
