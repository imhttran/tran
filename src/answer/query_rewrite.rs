//! Query rewriting before retrieval, with a model seam.
//!
//! Ported from the Go baseline rewrite step. The model may return a rewritten
//! query; when it errors or returns blank, the original question is used.

use crate::generation::Generator;

/// Rewrites `question` into a retrieval query using `generator`.
///
/// Returns the trimmed model output when it is non-empty, otherwise the
/// original question (trimmed).
pub fn rewrite(generator: &dyn Generator, question: &str) -> String {
    let prompt = format!(
        "Rewrite the question for document search. Reply with the query only.\n\nQuestion: {question}"
    );
    match generator.generate(&prompt) {
        Ok(rewritten) if !rewritten.trim().is_empty() => rewritten.trim().to_string(),
        _ => question.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::GenerationError;

    struct StubGenerator(Result<String, ()>);

    impl Generator for StubGenerator {
        fn generate(&self, _prompt: &str) -> Result<String, GenerationError> {
            match &self.0 {
                Ok(rewritten) => Ok(rewritten.clone()),
                Err(()) => Err(GenerationError::Client(
                    crate::ollama::OllamaError::Timeout {
                        path: "rewrite".to_string(),
                        timeout: std::time::Duration::from_secs(1),
                    },
                )),
            }
        }
    }

    #[test]
    fn uses_model_rewrite_when_present() {
        let g = StubGenerator(Ok("  rust retrieval  ".to_string()));
        assert_eq!(rewrite(&g, "what is rust?"), "rust retrieval");
    }

    #[test]
    fn blank_rewrite_falls_back_to_question() {
        let g = StubGenerator(Ok("   ".to_string()));
        assert_eq!(rewrite(&g, " what is rust? "), "what is rust?");
    }

    #[test]
    fn error_falls_back_to_question() {
        let g = StubGenerator(Err(()));
        assert_eq!(rewrite(&g, "what is rust?"), "what is rust?");
    }
}
