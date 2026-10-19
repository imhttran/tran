//! Answer prompts with a local Ollama model.
//!
//! Ported from the Go baseline in `internal/generation/generation.go`. The
//! endpoint path (`/api/generate`), the JSON request (`model`, `prompt`,
//! `stream: false`), and the JSON response (`response`) mirror the baseline.

use crate::ollama::{OllamaClient, OllamaError};
use thiserror::Error;

/// Produces text from a prompt. Consumers depend on this interface rather than
/// a concrete implementation, so tests can substitute a stub.
pub trait Generator: Send + Sync {
    fn generate(&self, prompt: &str) -> Result<String, GenerationError>;
}

/// The Ollama endpoint used for generation.
pub const GENERATE_PATH: &str = "/api/generate";

/// Request body sent to `/api/generate`.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct GenerateRequest {
    pub model: String,
    pub prompt: String,
    pub stream: bool,
}

impl GenerateRequest {
    /// Builds a non-streaming request for `model` and `prompt`, matching the Go
    /// baseline's `Stream: false`.
    pub fn new(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            prompt: prompt.into(),
            stream: false,
        }
    }
}

/// Response body returned by `/api/generate`.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct GenerateResponse {
    pub response: String,
}

/// Errors returned by [`Generator::generate`].
#[derive(Debug, Error)]
pub enum GenerationError {
    /// The transport, status, or decode layer failed.
    #[error("generate answer: {0}")]
    Client(#[from] OllamaError),
}

/// Produces text with a specific Ollama model.
pub struct OllamaGenerator {
    client: OllamaClient,
    model: String,
}

impl OllamaGenerator {
    /// Returns a generator backed by Ollama using `model`.
    pub fn new(client: OllamaClient, model: impl Into<String>) -> Self {
        Self {
            client,
            model: model.into(),
        }
    }
}

impl Generator for OllamaGenerator {
    /// Returns the model's answer to `prompt`.
    fn generate(&self, prompt: &str) -> Result<String, GenerationError> {
        let request = GenerateRequest::new(self.model.clone(), prompt);
        let result: GenerateResponse = self.client.post_json(GENERATE_PATH, &request)?;
        Ok(result.response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ollama::Transport;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    type CallQueue = Mutex<VecDeque<Result<(u16, String, Vec<u8>), String>>>;

    #[derive(Default)]
    struct MockTransport {
        calls: CallQueue,
        seen: Mutex<Vec<(String, Vec<u8>)>>,
    }

    impl MockTransport {
        fn push(&self, response: Result<(u16, String, Vec<u8>), String>) {
            self.calls.lock().unwrap().push_back(response);
        }
    }

    impl Transport for MockTransport {
        fn post(&self, url: &str, body: &[u8]) -> Result<(u16, String, Vec<u8>), String> {
            self.seen
                .lock()
                .unwrap()
                .push((url.to_string(), body.to_vec()));
            self.calls
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err("no response".into()))
        }
    }

    fn generator(transport: Arc<MockTransport>) -> OllamaGenerator {
        OllamaGenerator::new(
            OllamaClient::new(
                "http://localhost:11434",
                transport,
                Duration::from_secs(300),
            ),
            "qwen3.8:27b-mlx",
        )
    }

    #[test]
    fn request_path_and_body_match_contract() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            200,
            "200 OK".into(),
            br#"{"response":"hi there"}"#.to_vec(),
        )));
        let g = generator(transport.clone());

        assert_eq!(g.generate("say hi").unwrap(), "hi there");

        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen[0].0, "http://localhost:11434/api/generate");
        let body: serde_json::Value = serde_json::from_slice(&seen[0].1).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"model": "qwen3.8:27b-mlx", "prompt": "say hi", "stream": false})
        );
    }

    #[test]
    fn malformed_body_is_a_decode_error() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((200, "200 OK".into(), b"not json".to_vec())));
        let g = generator(transport);

        let err = g.generate("hi").unwrap_err();
        assert!(matches!(
            err,
            GenerationError::Client(OllamaError::Decode { .. })
        ));
        assert!(err.to_string().contains("/api/generate"));
    }

    #[test]
    fn http_failure_is_a_status_error() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            500,
            "500 Internal Server Error".into(),
            b"boom".to_vec(),
        )));
        let g = generator(transport);

        let err = g.generate("hi").unwrap_err();
        match err {
            GenerationError::Client(OllamaError::Status {
                path,
                status,
                detail,
            }) => {
                assert_eq!(path, "/api/generate");
                assert!(status.contains("500"));
                assert_eq!(detail, "boom");
            }
            other => panic!("expected status error, got {other:?}"),
        }
    }

    #[test]
    fn timeout_surfaces_the_timeout_error() {
        let err = GenerationError::Client(OllamaError::Timeout {
            path: GENERATE_PATH.to_string(),
            timeout: Duration::from_secs(1),
        });
        assert!(err.to_string().contains("timed out"));
    }
}
