//! Section expansion: grow retrieved chunks to their surrounding sections.
//!
//! Ported from the Go baseline expansion step. For each candidate, adds the
//! neighbouring chunks of the same source whose `chunk_index` is within
//! `window` of the candidate, preserving candidate order and provenance
//! (source, section, page). The first occurrence of each id is kept.

use super::Candidate;
use crate::document::Chunk;
use std::collections::{HashMap, HashSet};

/// Expands each candidate to neighbouring chunks of the same source.
///
pub fn expand(candidates: &[Candidate], corpus: &[Chunk], window: u32) -> Vec<Candidate> {
    let mut by_source: HashMap<&str, Vec<&Chunk>> = HashMap::new();
    for chunk in corpus {
        by_source
            .entry(chunk.source.as_str())
            .or_default()
            .push(chunk);
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        let chunk = &candidate.document.chunk;
        if let Some(neighbours) = by_source.get(chunk.source.as_str()) {
            for neighbour in neighbours {
                let distance = chunk.chunk_index.abs_diff(neighbour.chunk_index);
                if distance <= window {
                    let expanded = Candidate::new((*neighbour).clone(), candidate.score);
                    if seen.insert(expanded.id.clone()) {
                        out.push(expanded);
                    }
                }
            }
        } else if seen.insert(candidate.id.clone()) {
            out.push(candidate.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(section: &str, index: u32, page: Option<u32>) -> Chunk {
        Chunk {
            source: "doc.md".to_string(),
            section: section.to_string(),
            language: "english".to_string(),
            chunk_index: index,
            content: format!("{section} {index}"),
            page,
        }
    }

    #[test]
    fn expands_to_neighbouring_chunks_preserving_provenance() {
        let corpus = vec![
            chunk("A", 0, Some(1)),
            chunk("B", 1, Some(1)),
            chunk("C", 2, Some(2)),
            chunk("D", 3, Some(2)),
        ];
        let candidates = vec![Candidate::new(chunk("B", 1, Some(1)), 0.9)];
        let expanded = expand(&candidates, &corpus, 1);
        let indexes: Vec<u32> = expanded
            .iter()
            .map(|c| c.document.chunk.chunk_index)
            .collect();
        assert_eq!(indexes, vec![0, 1, 2]);
        assert_eq!(expanded[1].document.chunk.page, Some(1));
        assert_eq!(expanded[2].document.chunk.page, Some(2));
    }

    #[test]
    fn zero_window_keeps_only_the_candidate() {
        let corpus = vec![chunk("A", 0, None), chunk("B", 1, None)];
        let candidates = vec![Candidate::new(chunk("A", 0, None), 0.9)];
        let expanded = expand(&candidates, &corpus, 0);
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].document.chunk.chunk_index, 0);
    }

    #[test]
    fn empty_candidates_expand_to_empty() {
        let corpus = vec![chunk("A", 0, None)];
        assert!(expand(&[], &corpus, 2).is_empty());
    }
}
