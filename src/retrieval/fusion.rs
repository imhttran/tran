//! Rank fusion of vector and full-text candidate lists.
//!
//! Ported from the Go baseline fusion step. Uses reciprocal rank fusion (RRF)
//! with `k = 60`: each candidate contributes `1 / (k + rank)` from each list it
//! appears in (rank is 1-based), contributions sum per candidate id, and the
//! fused list is ordered by fused score descending, then id ascending.

use super::Candidate;
use std::collections::HashMap;

/// The RRF constant, matching the Go baseline.
pub const RRF_K: f64 = 60.0;

/// Fuses `lists` with reciprocal rank fusion.
///
pub fn fuse(lists: &[Vec<Candidate>]) -> Vec<Candidate> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    let mut by_id: HashMap<String, Candidate> = HashMap::new();

    for list in lists {
        for (pos, candidate) in list.iter().enumerate() {
            let rank = (pos + 1) as f64;
            *scores.entry(candidate.id.clone()).or_insert(0.0) += 1.0 / (RRF_K + rank);
            by_id
                .entry(candidate.id.clone())
                .or_insert_with(|| candidate.clone());
        }
    }

    let mut fused: Vec<Candidate> = scores
        .into_iter()
        .map(|(id, score)| {
            let base = by_id.remove(&id).expect("id present in by_id");
            Candidate {
                id: base.id,
                document: crate::document::RetrievedDocument::new(base.document.chunk, score),
                score,
            }
        })
        .collect();

    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    fused
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Chunk;

    fn chunk(section: &str) -> Chunk {
        Chunk {
            source: "doc.md".to_string(),
            section: section.to_string(),
            language: "english".to_string(),
            chunk_index: 0,
            content: section.to_string(),
            page: None,
        }
    }

    #[test]
    fn agreement_across_lists_ranks_first() {
        let vector = vec![
            Candidate::new(chunk("A"), 0.9),
            Candidate::new(chunk("B"), 0.8),
        ];
        let fts = vec![
            Candidate::new(chunk("B"), 5.0),
            Candidate::new(chunk("C"), 3.0),
        ];
        let fused = fuse(&[vector, fts]);
        // B appears in both lists, so it wins.
        assert_eq!(fused[0].document.chunk.section, "B");
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn single_list_preserves_order() {
        let vector = vec![
            Candidate::new(chunk("A"), 0.9),
            Candidate::new(chunk("B"), 0.8),
        ];
        let fused = fuse(&[vector]);
        let titles: Vec<&str> = fused
            .iter()
            .map(|c| c.document.chunk.section.as_str())
            .collect();
        assert_eq!(titles, vec!["A", "B"]);
    }

    #[test]
    fn empty_lists_fuse_empty() {
        assert!(fuse(&[]).is_empty());
        assert!(fuse(&[vec![], vec![]]).is_empty());
    }
}
