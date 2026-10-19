//! Fallback ordering when reranking is unavailable or unusable.
//!
//! Ported from the Go baseline fallback step: order candidates by their
//! existing score descending, ties broken by id ascending, so the input order
//! is stable and deterministic.

use super::Candidate;

/// Returns `candidates` ordered by score descending, then id ascending.
pub fn order(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
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
    fn orders_by_score_descending() {
        let input = vec![
            candidate("A", 0.5),
            candidate("B", 0.9),
            candidate("C", 0.7),
        ];
        let out = order(input);
        let titles: Vec<&str> = out
            .iter()
            .map(|c| c.document.chunk.section.as_str())
            .collect();
        assert_eq!(titles, vec!["B", "C", "A"]);
    }

    #[test]
    fn empty_stays_empty() {
        assert!(order(vec![]).is_empty());
    }
}
