//! Provider registry mapping `EMBED_PROVIDER` / `GEN_PROVIDER` names to
//! constructors for embedding and generation implementations.
//!
//! Ported from the Go baseline in `internal/provider/provider.go`. The registry
//! is the only module that knows how a vendor client is built, so the rest of
//! the crate resolves implementations by name. An empty name resolves to
//! [`DEFAULT`], while an unregistered name is an error — never a silent
//! success — which is what makes unknown providers fail during configuration
//! loading.

use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;

use crate::embedding::{Embedder, OllamaEmbedder};
use crate::generation::{Generator, OllamaGenerator, OpenAICompatibleGenerator};
use crate::ollama::{OllamaClient, Transport};

/// Provider name used when none is configured, matching the Go `Default`.
pub const DEFAULT: &str = "ollama";

/// Generate provider for any OpenAI-compatible server (oMLX, LM Studio,
/// llama.cpp's server). Answers only: embeddings stay on [`DEFAULT`].
pub const OPENAI_COMPATIBLE: &str = "openai_compatible";

/// Settings a registered constructor needs in order to build a vendor client,
/// mirroring the Go `provider.Client` struct.
#[derive(Clone)]
pub struct Client {
    pub ollama_url: String,
    pub embed_model: String,
    pub chat_model: String,
    pub request_timeout: Duration,
    /// Transport used for the constructed client.
    pub transport: Arc<dyn Transport>,
    /// Server root for [`OPENAI_COMPATIBLE`] (a trailing `/v1` is accepted).
    pub openai_compatible_url: String,
    /// Model name for [`OPENAI_COMPATIBLE`], as the server lists it.
    pub openai_compatible_model: String,
}

/// Error returned when a name does not resolve to a registered provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProviderError {
    #[error("unknown embed provider \"{0}\"")]
    UnknownEmbedder(String),
    #[error("unknown generate provider \"{0}\"")]
    UnknownGenerator(String),
}

/// Returns `name`, or [`DEFAULT`] when `name` is empty.
pub fn resolve_name(name: &str) -> &str {
    if name.is_empty() {
        DEFAULT
    } else {
        name
    }
}

/// Reports whether `name` resolves to a registered embed provider. An empty
/// name resolves to [`DEFAULT`], matching [`resolve_name`]. Lets callers (for
/// example [`crate::config`]) validate a configured provider name against the
/// registry without constructing a client.
pub fn has_embedder(name: &str) -> bool {
    matches!(resolve_name(name), DEFAULT)
}

/// Reports whether `name` resolves to a registered generate provider. An empty
/// name resolves to [`DEFAULT`], matching [`resolve_name`].
pub fn has_generator(name: &str) -> bool {
    matches!(resolve_name(name), DEFAULT | OPENAI_COMPATIBLE)
}

/// Returns an [`Embedder`] for the named provider. An empty name resolves to
/// [`DEFAULT`]. An unregistered name is an error, never a nil success.
pub fn embedder(name: &str, client: &Client) -> Result<Box<dyn Embedder>, ProviderError> {
    match resolve_name(name) {
        DEFAULT => Ok(Box::new(OllamaEmbedder::new(
            OllamaClient::new(
                client.ollama_url.clone(),
                Arc::clone(&client.transport),
                client.request_timeout,
            ),
            client.embed_model.clone(),
        ))),
        other => Err(ProviderError::UnknownEmbedder(other.to_string())),
    }
}

/// Returns a [`Generator`] for the named provider. An empty name resolves to
/// [`DEFAULT`]. An unregistered name is an error, never a nil success.
pub fn generator(name: &str, client: &Client) -> Result<Box<dyn Generator>, ProviderError> {
    match resolve_name(name) {
        DEFAULT => Ok(Box::new(OllamaGenerator::new(
            OllamaClient::new(
                client.ollama_url.clone(),
                Arc::clone(&client.transport),
                client.request_timeout,
            ),
            client.chat_model.clone(),
        ))),
        OPENAI_COMPATIBLE => Ok(Box::new(OpenAICompatibleGenerator::new(
            OllamaClient::new(
                client.openai_compatible_url.clone(),
                Arc::clone(&client.transport),
                client.request_timeout,
            ),
            client.openai_compatible_model.clone(),
        ))),
        other => Err(ProviderError::UnknownGenerator(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct NoopTransport {
        _lock: Mutex<()>,
    }

    impl Transport for NoopTransport {
        fn post(&self, _url: &str, _body: &[u8]) -> Result<(u16, String, Vec<u8>), String> {
            Err("unused".into())
        }
    }

    fn client() -> Client {
        Client {
            ollama_url: "http://localhost:11434".into(),
            embed_model: "nomic-embed-text".into(),
            chat_model: "qwen3.8:27b-mlx".into(),
            request_timeout: Duration::from_secs(300),
            transport: Arc::new(NoopTransport::default()),
            openai_compatible_url: "http://127.0.0.1:8000".into(),
            openai_compatible_model: "Qwen3-4B-4bit".into(),
        }
    }

    #[test]
    fn empty_name_resolves_to_default() {
        assert_eq!(resolve_name(""), DEFAULT);
        assert_eq!(resolve_name("ollama"), "ollama");
        assert!(has_embedder(""));
        assert!(has_generator(""));
    }

    #[test]
    fn default_providers_resolve() {
        assert!(has_embedder("ollama"));
        assert!(has_generator("ollama"));
        assert!(embedder("ollama", &client()).is_ok());
        assert!(generator("ollama", &client()).is_ok());
    }

    #[test]
    fn openai_compatible_generates_but_does_not_embed() {
        assert!(has_generator(OPENAI_COMPATIBLE));
        assert!(generator(OPENAI_COMPATIBLE, &client()).is_ok());
        // Embeddings stay on Ollama; this provider only answers.
        assert!(!has_embedder(OPENAI_COMPATIBLE));
        assert!(embedder(OPENAI_COMPATIBLE, &client()).is_err());
    }

    #[test]
    fn unknown_providers_are_rejected() {
        assert!(!has_embedder("bogus"));
        assert!(!has_generator("bogus"));
        assert_eq!(
            embedder("bogus", &client()).err().unwrap(),
            ProviderError::UnknownEmbedder("bogus".into())
        );
        assert_eq!(
            generator("bogus", &client()).err().unwrap(),
            ProviderError::UnknownGenerator("bogus".into())
        );
    }
}
