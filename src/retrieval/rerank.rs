//! Reranking of retrieved candidates with a model seam.
//!
//! Ported from the Go baseline rerank step. The model returns a new order for
//! the candidate ids; the reranker applies that order when the response is
//! usable (non-empty and covering known ids). When the model errors or returns
//! an unusable order, [`crate::retrieval::fallback`] supplies the fallback
//! order.

use super::Candidate;
use thiserror::Error;

/// Errors returned by a [`Reranker`].
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RerankError {
    /// The model could not be reached or returned an error.
    #[error("rerank model unavailable")]
    Unavailable,
}

/// A reranking model seam. Tests substitute a stub; production uses the
/// generation port.
pub trait Reranker: Send + Sync {
    /// Returns candidate ids in the model's preferred order for `query`.
    fn rerank(&self, query: &str, candidates: &[Candidate]) -> Result<Vec<String>, RerankError>;
}

/// Applies the model order `order` to `candidates`.
///
pub fn apply_order(candidates: Vec<Candidate>, order: &[String]) -> Vec<Candidate> {
    let mut ranked: Vec<Candidate> = Vec::with_capacity(candidates.len());
    let mut remaining = candidates;
    for id in order {
        if let Some(pos) = remaining.iter().position(|c| &c.id == id) {
            ranked.push(remaining.remove(pos));
        }
    }
    // Candidates the model omitted keep their relative order after those it
    // ranked, matching the baseline's stable append.
    ranked.append(&mut remaining);
    ranked
}

/// Reranks `candidates` with `reranker`.
///
/// Returns the reranked order on a usable response, or the fallback order when
/// the model errors or returns an empty order.
pub fn rerank(reranker: &dyn Reranker, query: &str, candidates: Vec<Candidate>) -> Vec<Candidate> {
    let fallback = super::fallback::order(candidates.clone());
    match reranker.rerank(query, &candidates) {
        Ok(order) if !order.is_empty() => apply_order(candidates, &order),
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Chunk;
    use std::sync::Mutex;

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

    struct MockReranker {
        response: Mutex<Result<Vec<String>, RerankError>>,
    }

    impl Reranker for MockReranker {
        fn rerank(
            &self,
            _query: &str,
            _candidates: &[Candidate],
        ) -> Result<Vec<String>, RerankError> {
            self.response.lock().unwrap().clone()
        }
    }

    fn setup(response: Result<Vec<String>, RerankError>) -> MockReranker {
        MockReranker {
            response: Mutex::new(response),
        }
    }

    #[test]
    fn applies_model_order() {
        let c = vec![
            candidate("A", 0.9),
            candidate("B", 0.8),
            candidate("C", 0.7),
        ];
        let order = vec![
            "doc.md::C::0".to_string(),
            "doc.md::A::0".to_string(),
            "doc.md::B::0".to_string(),
        ];
        let reranker = setup(Ok(order));
        let out = rerank(&reranker, "q", c);
        let titles: Vec<&str> = out
            .iter()
            .map(|x| x.document.chunk.section.as_str())
            .collect();
        assert_eq!(titles, vec!["C", "A", "B"]);
    }

    #[test]
    fn empty_response_falls_back() {
        let c = vec![candidate("A", 0.9), candidate("B", 0.8)];
        let reranker = setup(Ok(vec![]));
        let out = rerank(&reranker, "q", c);
        let titles: Vec<&str> = out
            .iter()
            .map(|x| x.document.chunk.section.as_str())
            .collect();
        assert_eq!(titles, vec!["A", "B"]);
    }

    #[test]
    fn error_falls_back() {
        let c = vec![candidate("A", 0.9), candidate("B", 0.8)];
        let reranker = setup(Err(RerankError::Unavailable));
        let out = rerank(&reranker, "q", c);
        let titles: Vec<&str> = out
            .iter()
            .map(|x| x.document.chunk.section.as_str())
            .collect();
        assert_eq!(titles, vec!["A", "B"]);
    }
}
