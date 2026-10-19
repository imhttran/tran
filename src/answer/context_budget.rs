//! Context budgeting: assemble retrieved candidates into a bounded context.
//!
//! Ported from the Go baseline budget step. Sections are appended in order
//! until adding the next would exceed `budget` characters; the whole context
//! never exceeds the budget.

use crate::retrieval::Candidate;

/// A budgeted context passage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPassage {
    /// Source document name.
    pub source: String,
    /// Section title.
    pub section: String,
    /// 1-based page, if any.
    pub page: Option<u32>,
    /// Passage body.
    pub content: String,
}

/// Builds a budgeted context from ranked candidates.
///
/// Passages are taken in candidate order. A passage is included only when its
/// content fits within the remaining budget; the returned passages never sum
/// to more than `budget` characters.
pub fn budget(candidates: &[Candidate], budget: usize) -> Vec<ContextPassage> {
    let mut passages = Vec::new();
    let mut used = 0usize;
    for candidate in candidates {
        let chunk = &candidate.document.chunk;
        let len = chunk.content.chars().count();
        if used + len > budget {
            continue;
        }
        used += len;
        passages.push(ContextPassage {
            source: chunk.source.clone(),
            section: chunk.section.clone(),
            page: chunk.page,
            content: chunk.content.clone(),
        });
    }
    passages
}

/// Returns the total character length of `passages`.
pub fn total_len(passages: &[ContextPassage]) -> usize {
    passages.iter().map(|p| p.content.chars().count()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Chunk;

    fn candidate(section: &str, content: &str) -> Candidate {
        Candidate::new(
            Chunk {
                source: "doc.md".to_string(),
                section: section.to_string(),
                language: "english".to_string(),
                chunk_index: 0,
                content: content.to_string(),
                page: Some(1),
            },
            0.5,
        )
    }

    #[test]
    fn keeps_order_and_never_exceeds_budget() {
        let c = vec![
            candidate("A", "12345"),
            candidate("B", "1234567890"),
            candidate("C", "123"),
        ];
        let out = budget(&c, 8);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].section, "A");
        assert_eq!(out[1].section, "C");
        assert!(total_len(&out) <= 8);
    }

    #[test]
    fn empty_budget_yields_no_passages() {
        let c = vec![candidate("A", "12345")];
        assert!(budget(&c, 0).is_empty());
    }

    #[test]
    fn preserves_provenance() {
        let c = vec![candidate("A", "body")];
        let out = budget(&c, 100);
        assert_eq!(out[0].source, "doc.md");
        assert_eq!(out[0].page, Some(1));
    }
}
