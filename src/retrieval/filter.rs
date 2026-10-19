//! Similarity filtering of fused candidates.
//!
//! Ported from the Go baseline filter step. Keeps candidates whose score is at
//! or above a configured similarity threshold, preserving order.

use super::Candidate;

/// The default minimum similarity, matching the Go baseline.
pub const DEFAULT_MIN_SIMILARITY: f64 = 0.5;

/// Keeps candidates with `score >= min_similarity`, preserving order.
pub fn apply(candidates: Vec<Candidate>, min_similarity: f64) -> Vec<Candidate> {
    candidates
        .into_iter()
        .filter(|c| c.score >= min_similarity)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Chunk;

    fn candidate(section: &str, score: f64) -> Candidate {
        Candidate::new(
            Chunk {
                source: "doc.md".to_string(),
                section: section.to_string(),
                language: "english".to_string(),
                chunk_index: 0,
                content: section.to_string(),
                page: None,
            },
            score,
        )
    }

    #[test]
    fn drops_below_threshold_and_keeps_at_or_above() {
        let input = vec![
            candidate("A", 0.9),
            candidate("B", 0.5),
            candidate("C", 0.4),
        ];
        let kept = apply(input, DEFAULT_MIN_SIMILARITY);
        let titles: Vec<&str> = kept
            .iter()
            .map(|c| c.document.chunk.section.as_str())
            .collect();
        assert_eq!(titles, vec!["A", "B"]);
    }

    #[test]
    fn empty_input_stays_empty() {
        assert!(apply(vec![], 0.5).is_empty());
    }
}
