//! Vector similarity search over fixed embeddings.
//!
//! Ported from the Go baseline vector path: score each stored chunk by cosine
//! similarity against the query embedding, keep candidates at or above the
//! minimum similarity, and return them highest score first. Ties break by
//! candidate id so ordering is deterministic.

use super::Candidate;
use crate::document::Chunk;

/// A stored chunk with its embedding, the search input.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexedVector {
    /// The stored chunk with provenance.
    pub chunk: Chunk,
    /// The chunk embedding.
    pub embedding: Vec<f64>,
}

/// Returns the cosine similarity of `a` and `b`.
///
/// Returns `0.0` when either vector has zero magnitude or lengths differ, so a
/// degenerate embedding never ranks above a real match.
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Searches `index` for chunks similar to `query`.
///
/// Candidates at or above `min_similarity` are kept and returned highest score
/// first, ties broken by candidate id ascending.
pub fn search(index: &[IndexedVector], query: &[f64], min_similarity: f64) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = index
        .iter()
        .map(|entry| {
            Candidate::new(
                entry.chunk.clone(),
                cosine_similarity(query, &entry.embedding),
            )
        })
        .filter(|c| c.score >= min_similarity)
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

    fn chunk(section: &str, index: u32) -> Chunk {
        Chunk {
            source: "doc.md".to_string(),
            section: section.to_string(),
            language: "english".to_string(),
            chunk_index: index,
            content: format!("{section} body"),
            page: None,
        }
    }

    #[test]
    fn cosine_of_identical_vectors_is_one() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn cosine_of_orthogonal_vectors_is_zero() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    }

    #[test]
    fn empty_or_mismatched_vectors_score_zero() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn search_ranks_by_similarity_and_filters() {
        let index = vec![
            IndexedVector {
                chunk: chunk("A", 0),
                embedding: vec![1.0, 0.0],
            },
            IndexedVector {
                chunk: chunk("B", 1),
                embedding: vec![0.9, 0.1],
            },
            IndexedVector {
                chunk: chunk("C", 2),
                embedding: vec![0.0, 1.0],
            },
        ];
        let results = search(&index, &[1.0, 0.0], 0.5);
        let ids: Vec<String> = results
            .iter()
            .map(|c| c.document.chunk.section.clone())
            .collect();
        assert_eq!(ids, vec!["A", "B"]);
        assert!(results[0].score >= results[1].score);
    }

    #[test]
    fn ties_break_by_id() {
        let index = vec![
            IndexedVector {
                chunk: chunk("B", 1),
                embedding: vec![1.0, 0.0],
            },
            IndexedVector {
                chunk: chunk("A", 0),
                embedding: vec![1.0, 0.0],
            },
        ];
        let results = search(&index, &[1.0, 0.0], 0.0);
        let ids: Vec<String> = results
            .iter()
            .map(|c| c.document.chunk.section.clone())
            .collect();
        assert_eq!(ids, vec!["A", "B"]);
    }
}
