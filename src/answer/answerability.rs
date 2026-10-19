//! Answerability gate: refuse when evidence is insufficient.
//!
//! Ported from the Go baseline: the question is answerable when the budgeted
//! context has at least one passage. Otherwise the pipeline refuses with the
//! baseline refusal text.

use super::context_budget::ContextPassage;

/// The baseline refusal text used when evidence is insufficient.
pub const REFUSAL: &str = "I cannot answer this question from the provided documents.";

/// Returns `true` when `passages` provides enough evidence to answer.
pub fn is_answerable(passages: &[ContextPassage]) -> bool {
    !passages.is_empty()
}

/// Returns the baseline refusal text.
pub fn refusal() -> String {
    REFUSAL.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage(content: &str) -> ContextPassage {
        ContextPassage {
            source: "doc.md".to_string(),
            section: "Intro".to_string(),
            page: None,
            content: content.to_string(),
        }
    }

    #[test]
    fn empty_evidence_is_not_answerable() {
        assert!(!is_answerable(&[]));
        assert_eq!(refusal(), REFUSAL);
    }

    #[test]
    fn any_passage_is_answerable() {
        assert!(is_answerable(&[passage("body")]));
    }
}
