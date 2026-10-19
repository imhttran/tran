//! Domain types for sections, chunks, and retrieved documents.
//!
//! Field semantics and terminology mirror the Go baseline in
//! `internal/document/document.go`: a [`Section`] is one `## ` section of a
//! document, `page` is the 1-based source page (`None` when the source carries
//! no page information), and a [`Chunk`] carries its source, section, language,
//! and document-global chunk index. A [`RetrievedDocument`] augments a chunk
//! with an optional similarity score from retrieval.

use serde::{Deserialize, Serialize};

/// One `## ` section of a document.
///
/// `page` is the 1-based source page the section was extracted from (PDF
/// loaders emit one section per page); it is `None` when the source has no page
/// information (Markdown, plain text), matching Go's `Page == 0` sentinel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// Section heading text (the `## ` title line), trimmed.
    pub title: String,
    /// Section body content, trimmed.
    pub content: String,
    /// 1-based source page, or `None` when the source has no page metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

impl Section {
    /// Builds a section with no page metadata.
    pub fn new(title: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            content: content.into(),
            page: None,
        }
    }

    /// Builds a section carrying explicit page metadata.
    pub fn with_page(title: impl Into<String>, content: impl Into<String>, page: u32) -> Self {
        Self {
            title: title.into(),
            content: content.into(),
            page: Some(page),
        }
    }
}

/// A chunk of a [`Section`], the unit ingestion stores and retrieval returns.
///
/// `source` names the originating document, `section` names the section within
/// it, `language` is the corpus language, and `chunk_index` is the
/// document-global chunk number (the Go baseline numbers chunks
/// document-globally, not per section). `page` carries the optional page
/// metadata inherited from the section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    /// Originating document source (file name or URL).
    pub source: String,
    /// Section within the source document.
    pub section: String,
    /// Corpus language used for full-text search configuration.
    pub language: String,
    /// Document-global chunk index (0-based).
    pub chunk_index: u32,
    /// Chunk content.
    pub content: String,
    /// 1-based source page, or `None` when unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

/// A chunk returned by retrieval, with the similarity score that retrieved it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievedDocument {
    /// The retrieved chunk, preserving its source/section/language/index/page
    /// metadata.
    pub chunk: Chunk,
    /// Cosine similarity (or fused) score that selected this document.
    pub score: f64,
}

impl RetrievedDocument {
    /// Wraps a chunk with its retrieval score.
    pub fn new(chunk: Chunk, score: f64) -> Self {
        Self { chunk, score }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_preserves_page_metadata() {
        let section = Section::with_page("Intro", "body", 3);
        assert_eq!(section.page, Some(3));
    }

    #[test]
    fn section_without_page_has_none() {
        let section = Section::new("Intro", "body");
        assert_eq!(section.page, None);
    }

    #[test]
    fn chunk_preserves_required_metadata() {
        let chunk = Chunk {
            source: "doc.md".to_string(),
            section: "Intro".to_string(),
            language: "english".to_string(),
            chunk_index: 2,
            content: "body".to_string(),
            page: Some(1),
        };

        assert_eq!(chunk.source, "doc.md");
        assert_eq!(chunk.section, "Intro");
        assert_eq!(chunk.language, "english");
        assert_eq!(chunk.chunk_index, 2);
        assert_eq!(chunk.page, Some(1));
    }

    #[test]
    fn optional_page_round_trips() {
        let with_page = Chunk {
            source: "doc.md".to_string(),
            section: "Intro".to_string(),
            language: "english".to_string(),
            chunk_index: 0,
            content: "body".to_string(),
            page: Some(5),
        };
        let encoded = serde_json::to_string(&with_page).unwrap();
        let decoded: Chunk = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, with_page);

        let without_page = Chunk {
            page: None,
            ..with_page
        };
        let encoded = serde_json::to_string(&without_page).unwrap();
        let decoded: Chunk = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, without_page);
        assert_eq!(decoded.page, None);
    }

    #[test]
    fn retrieved_document_round_trips() {
        let doc = RetrievedDocument::new(
            Chunk {
                source: "doc.md".to_string(),
                section: "Intro".to_string(),
                language: "english".to_string(),
                chunk_index: 7,
                content: "body".to_string(),
                page: None,
            },
            0.75,
        );

        let encoded = serde_json::to_string(&doc).unwrap();
        let decoded: RetrievedDocument = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, doc);
        assert_eq!(decoded.chunk.chunk_index, 7);
    }
}
