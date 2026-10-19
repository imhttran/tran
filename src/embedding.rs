//! Turn text into vectors using Ollama.
//!
//! Ported from the Go baseline in `internal/embedding/embedding.go`. The
//! endpoint path (`/api/embed`), the JSON request (`model`, `input`), the JSON
//! response (`embeddings`), and the "no embeddings" and dimension-mismatch
//! checks all mirror the baseline.

use crate::ollama::{OllamaClient, OllamaError};
use thiserror::Error;

/// Turns text into an embedding vector. Consumers depend on this interface
/// rather than a concrete implementation, so tests can substitute a stub that
/// performs no HTTP.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f64>, EmbeddingError>;
}

/// The Ollama endpoint used for embeddings.
pub const EMBED_PATH: &str = "/api/embed";

/// Request body sent to `/api/embed`.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct EmbedRequest {
    pub model: String,
    pub input: String,
}

impl EmbedRequest {
    /// Builds the request for `model` and `input`.
    pub fn new(model: impl Into<String>, input: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            input: input.into(),
        }
    }
}

/// Response body returned by `/api/embed`.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct EmbedResponse {
    pub embeddings: Vec<Vec<f64>>,
}

/// Errors returned by [`Embedder::embed`].
#[derive(Debug, Error)]
pub enum EmbeddingError {
    /// The transport, status, or decode layer failed.
    #[error("embed text: {0}")]
    Client(#[from] OllamaError),

    /// The provider returned an empty embedding list.
    #[error("ollama returned no embeddings")]
    NoEmbeddings,

    /// The returned vector length did not match the configured dimension.
    #[error("embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },
}

/// Creates embeddings with a specific Ollama model, validating the vector
/// dimension against the configured `embed_dim`.
pub struct OllamaEmbedder {
    client: OllamaClient,
    model: String,
    embed_dim: usize,
}

impl OllamaEmbedder {
    /// Returns an embedder that uses `model` and talks to Ollama through
    /// `client`, checking results against `embed_dim`.
    pub fn new(client: OllamaClient, model: impl Into<String>) -> Self {
        Self {
            client,
            model: model.into(),
            embed_dim: 0,
        }
    }

    /// Sets the expected embedding dimension enforced on each result.
    pub fn with_embed_dim(mut self, embed_dim: usize) -> Self {
        self.embed_dim = embed_dim;
        self
    }
}

impl Embedder for OllamaEmbedder {
    /// Returns the embedding vector for `text`.
    fn embed(&self, text: &str) -> Result<Vec<f64>, EmbeddingError> {
        let request = EmbedRequest::new(self.model.clone(), text);
        let result: EmbedResponse = self.client.post_json(EMBED_PATH, &request)?;

        let vector = result
            .embeddings
            .into_iter()
            .next()
            .ok_or(EmbeddingError::NoEmbeddings)?;

        if self.embed_dim != 0 && vector.len() != self.embed_dim {
            return Err(EmbeddingError::DimensionMismatch {
                expected: self.embed_dim,
                actual: vector.len(),
            });
        }

        Ok(vector)
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

    fn embedder(transport: Arc<MockTransport>, embed_dim: usize) -> OllamaEmbedder {
        OllamaEmbedder::new(
            OllamaClient::new(
                "http://localhost:11434",
                transport,
                Duration::from_secs(300),
            ),
            "nomic-embed-text",
        )
        .with_embed_dim(embed_dim)
    }

    #[test]
    fn request_path_and_body_match_contract() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            200,
            "200 OK".into(),
            br#"{"embeddings":[[0.5,0.25]]}"#.to_vec(),
        )));
        let e = embedder(transport.clone(), 2);

        assert_eq!(e.embed("hello world").unwrap(), vec![0.5, 0.25]);

        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen[0].0, "http://localhost:11434/api/embed");
        let body: serde_json::Value = serde_json::from_slice(&seen[0].1).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"model": "nomic-embed-text", "input": "hello world"})
        );
    }

    #[test]
    fn dimension_mismatch_names_expected_and_actual() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            200,
            "200 OK".into(),
            br#"{"embeddings":[[0.1,0.2,0.3]]}"#.to_vec(),
        )));
        let e = embedder(transport, 768);

        let err = e.embed("hi").unwrap_err();
        assert!(matches!(
            err,
            EmbeddingError::DimensionMismatch {
                expected: 768,
                actual: 3
            }
        ));
        assert!(err.to_string().contains("expected 768"));
        assert!(err.to_string().contains("got 3"));
    }

    #[test]
    fn empty_embeddings_are_rejected() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((200, "200 OK".into(), br#"{"embeddings":[]}"#.to_vec())));
        let e = embedder(transport, 0);

        assert!(matches!(
            e.embed("hi").unwrap_err(),
            EmbeddingError::NoEmbeddings
        ));
    }

    #[test]
    fn malformed_body_is_a_decode_error() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((200, "200 OK".into(), b"nope".to_vec())));
        let e = embedder(transport, 0);

        let err = e.embed("hi").unwrap_err();
        assert!(matches!(
            err,
            EmbeddingError::Client(OllamaError::Decode { .. })
        ));
        assert!(err.to_string().contains("/api/embed"));
    }

    #[test]
    fn http_failure_is_a_status_error() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            503,
            "503 Service Unavailable".into(),
            b"down".to_vec(),
        )));
        let e = embedder(transport, 0);

        let err = e.embed("hi").unwrap_err();
        match err {
            EmbeddingError::Client(OllamaError::Status {
                path,
                status,
                detail,
            }) => {
                assert_eq!(path, "/api/embed");
                assert!(status.contains("503"));
                assert_eq!(detail, "down");
            }
            other => panic!("expected status error, got {other:?}"),
        }
    }
}
