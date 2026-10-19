//! Chunking with overlap and metadata propagation.
//!
//! Ported from the Go baseline chunker (`internal/chunking/chunking.go`).
//! Semantics:
//!
//! * Chunk size and overlap are **word counts**, not bytes (`CHUNK_SIZE` /
//!   `CHUNK_OVERLAP` are documented as word counts).
//! * Chunks are emitted in document order across all sections.
//! * `chunk_index` is document-global and 0-based (not per section).
//! * `source`, `section` (the section title), `language`, and the optional
//!   `page` are copied verbatim from the section.
//! * Each chunk's content is its words joined with a single space, so a run of
//!   whitespace (including newlines) normalizes to one space, exactly as the
//!   baseline's `strings.Join(words, " ")`.
//! * A section whose content has no words produces no chunks.
//! * As in the baseline, a zero size or `overlap >= size` produces no chunks at
//!   all.

use crate::document::{Chunk, Section};

/// Splits `sections` into chunks using the baseline word size/overlap semantics.
///
/// `size` is the maximum chunk length in words and `overlap` is the number of
/// words repeated between consecutive chunks of the same section. A zero `size`
/// or `overlap >= size` yields no chunks, matching the baseline's
/// `splitWords` guard.
pub fn chunk_sections(
    sections: &[Section],
    source: &str,
    language: &str,
    size: usize,
    overlap: usize,
) -> Vec<Chunk> {
    let mut chunks = Vec::new();

    // Baseline guard: chunkSize <= 0 || overlap < 0 || overlap >= chunkSize.
    if size == 0 || overlap >= size {
        return chunks;
    }

    let step = size - overlap;
    let mut index: u32 = 0;

    for section in sections {
        let words: Vec<&str> = section.content.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }

        let parts: Vec<String> = if words.len() <= size {
            vec![words.join(" ")]
        } else {
            let mut parts = Vec::new();
            let mut start = 0usize;
            while start < words.len() {
                let end = (start + size).min(words.len());
                parts.push(words[start..end].join(" "));
                if end == words.len() {
                    break;
                }
                start += step;
            }
            parts
        };

        for content in parts {
            chunks.push(Chunk {
                source: source.to_string(),
                section: section.title.clone(),
                language: language.to_string(),
                chunk_index: index,
                content,
                page: section.page,
            });
            index += 1;
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(prefix: &str, n: usize) -> String {
        (1..=n)
            .map(|i| format!("{prefix}{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn empty_sections_produce_no_chunks() {
        assert!(chunk_sections(&[], "doc.md", "english", 50, 20).is_empty());
    }

    #[test]
    fn empty_section_content_produces_no_chunks() {
        let sections = vec![Section::new("A", "")];
        assert!(chunk_sections(&sections, "doc.md", "english", 50, 20).is_empty());
    }

    #[test]
    fn content_exactly_size_words_is_one_chunk() {
        let content = body("w", 50);
        let sections = vec![Section::new("A", content.clone())];
        let chunks = chunk_sections(&sections, "doc.md", "english", 50, 20);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, content);
        assert_eq!(chunks[0].chunk_index, 0);
    }

    #[test]
    fn content_one_below_size_is_one_chunk() {
        let sections = vec![Section::new("A", body("w", 49))];
        assert_eq!(
            chunk_sections(&sections, "doc.md", "english", 50, 20).len(),
            1
        );
    }

    #[test]
    fn content_one_above_size_overlaps() {
        // 51 words, size 50, overlap 20 -> step 30: chunk0 = w1..w50, chunk1 = w31..w51.
        let sections = vec![Section::new("A", body("w", 51))];
        let chunks = chunk_sections(&sections, "doc.md", "english", 50, 20);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].content, body("w", 50));
        assert_eq!(
            chunks[1].content,
            (31..=51)
                .map(|i| format!("w{i}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(chunks[1].chunk_index, 1);
    }

    #[test]
    fn overlap_is_exact_repeated_words() {
        let sections = vec![Section::new("A", body("w", 100))];
        let chunks = chunk_sections(&sections, "doc.md", "english", 50, 20);
        assert!(chunks.len() >= 2);
        let tail: Vec<&str> = chunks[0].content.split_whitespace().skip(30).collect();
        let head: Vec<&str> = chunks[1].content.split_whitespace().take(20).collect();
        assert_eq!(tail, head);
    }

    #[test]
    fn chunk_index_is_document_global() {
        let sections = vec![
            Section::new("A", body("a", 110)),
            Section::new("B", body("b", 60)),
        ];
        let chunks = chunk_sections(&sections, "doc.md", "english", 50, 20);
        let indices: Vec<u32> = chunks.iter().map(|c| c.chunk_index).collect();
        let expected: Vec<u32> = (0..chunks.len() as u32).collect();
        assert_eq!(indices, expected);
        assert!(chunks.iter().any(|c| c.section == "B"));
    }

    #[test]
    fn whitespace_runs_normalize_to_single_spaces() {
        let sections = vec![Section::new("A", "one\n\n  two\t\tthree")];
        let chunks = chunk_sections(&sections, "doc.md", "english", 50, 20);
        assert_eq!(chunks[0].content, "one two three");
    }

    #[test]
    fn metadata_is_inherited() {
        let sections = vec![Section::with_page("A", body("w", 20), 2)];
        let chunks = chunk_sections(&sections, "doc.pdf", "vietnamese", 50, 20);
        for chunk in &chunks {
            assert_eq!(chunk.source, "doc.pdf");
            assert_eq!(chunk.section, "A");
            assert_eq!(chunk.language, "vietnamese");
            assert_eq!(chunk.page, Some(2));
        }
    }

    #[test]
    fn guard_produces_no_chunks() {
        let sections = vec![Section::new("A", body("w", 12))];
        // overlap >= size, matching the baseline's splitWords guard.
        assert!(chunk_sections(&sections, "doc.md", "english", 5, 5).is_empty());
        assert!(chunk_sections(&sections, "doc.md", "english", 5, 6).is_empty());
        // zero size.
        assert!(chunk_sections(&sections, "doc.md", "english", 0, 0).is_empty());
    }

    #[test]
    fn unicode_words_never_split_a_character() {
        // Unicode "words" (no spaces) stay whole; chunking is word-based.
        let content = (1..=60)
            .map(|i| format!("từ{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let sections = vec![Section::new("A", content)];
        let chunks = chunk_sections(&sections, "doc.md", "vietnamese", 50, 20);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| !c.content.is_empty()));
        assert!(chunks.iter().all(|c| c.content.contains("từ")));
    }
}
