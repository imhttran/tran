//! Deduplication of fused candidates by stable identity.
//!
//! Ported from the Go baseline dedup step. Keeps the first occurrence of each
//! candidate id, preserving order, so the highest-ranked duplicate wins.

use super::Candidate;
use std::collections::HashSet;

/// Removes later duplicates, keeping the first occurrence of each id.
pub fn dedup(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen: HashSet<String> = HashSet::new();
    candidates
        .into_iter()
        .filter(|c| seen.insert(c.id.clone()))
        .collect()
}

/// Removes later candidates from the same source and section, keeping the
/// first occurrence. This is the Go baseline's `DeduplicateSections` step used
/// by the hybrid pipeline and the evaluation fused ranking.
pub fn dedup_sections(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    candidates
        .into_iter()
        .filter(|c| {
            seen.insert((
                c.document.chunk.source.clone(),
                c.document.chunk.section.clone(),
            ))
        })
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
    fn keeps_first_occurrence_and_order() {
        let input = vec![
            candidate("A", 0.9),
            candidate("B", 0.8),
            candidate("A", 0.7),
        ];
        let out = dedup(input);
        let titles: Vec<&str> = out
            .iter()
            .map(|c| c.document.chunk.section.as_str())
            .collect();
        assert_eq!(titles, vec!["A", "B"]);
        assert_eq!(out[0].score, 0.9);
    }

    #[test]
    fn empty_stays_empty() {
        assert!(dedup(vec![]).is_empty());
    }

    #[test]
    fn section_dedup_keeps_the_first_chunk_of_a_section() {
        let input = vec![
            candidate("A", 0.9),
            candidate("A", 0.8),
            candidate("B", 0.7),
        ];
        let out = dedup_sections(input);
        let titles: Vec<&str> = out
            .iter()
            .map(|c| c.document.chunk.section.as_str())
            .collect();
        assert_eq!(titles, vec!["A", "B"]);
        assert_eq!(out[0].score, 0.9);
    }

    #[test]
    fn section_dedup_distinguishes_sources() {
        let mut second = candidate("A", 0.8);
        second.document.chunk.source = "other.md".to_string();
        let out = dedup_sections(vec![candidate("A", 0.9), second]);
        assert_eq!(out.len(), 2);
    }
}
