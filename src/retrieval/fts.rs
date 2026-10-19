//! PostgreSQL full-text search read operation over an in-memory corpus.
//!
//! Ported from the Go baseline FTS path: tokenize the query, score each chunk
//! by the number of query terms it contains (term frequency over the lowercased
//! content), and return scored candidates highest first. Ordering is
//! deterministic: score descending, then candidate id ascending. Chunks with no
//! matching term are excluded.

use super::Candidate;
use crate::document::Chunk;

/// Tokenizes `text` into lowercased alphanumeric terms.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// Returns the number of query terms present in `content`.
fn term_frequency(content: &str, terms: &[String]) -> usize {
    if terms.is_empty() {
        return 0;
    }
    let content_terms: Vec<String> = tokenize(content);
    terms
        .iter()
        .filter(|t| content_terms.iter().any(|c| c == *t))
        .count()
}

/// Searches `corpus` for chunks matching `query`.
///
/// Returns candidates ordered by match count descending, then id ascending.
/// Chunks that share no term with the query are excluded.
pub fn search(corpus: &[Chunk], query: &str) -> Vec<Candidate> {
    let terms = tokenize(query);
    let mut candidates: Vec<Candidate> = corpus
        .iter()
        .filter_map(|chunk| {
            let hits = term_frequency(&chunk.content, &terms);
            if hits == 0 {
                None
            } else {
                Some(Candidate::new(chunk.clone(), hits as f64))
            }
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(section: &str, index: u32, content: &str) -> Chunk {
        Chunk {
            source: "doc.md".to_string(),
            section: section.to_string(),
            language: "english".to_string(),
            chunk_index: index,
            content: content.to_string(),
            page: None,
        }
    }

    #[test]
    fn tokenize_lowercases_and_splits() {
        assert_eq!(tokenize("Hello, World! 42"), vec!["hello", "world", "42"]);
    }

    #[test]
    fn search_scores_by_matching_terms() {
        let corpus = vec![
            chunk("A", 0, "rust retrieval search"),
            chunk("B", 1, "rust only"),
            chunk("C", 2, "unrelated content"),
        ];
        let results = search(&corpus, "rust search");
        let ids: Vec<String> = results
            .iter()
            .map(|c| c.document.chunk.section.clone())
            .collect();
        assert_eq!(ids, vec!["A", "B"]);
        assert_eq!(results[0].score, 2.0);
        assert_eq!(results[1].score, 1.0);
    }

    #[test]
    fn empty_query_returns_nothing() {
        let corpus = vec![chunk("A", 0, "rust")];
        assert!(search(&corpus, "   ").is_empty());
    }
}
