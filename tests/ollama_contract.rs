//! Contract tests for the Ollama HTTP client, embedding/generation ports, and
//! provider resolution.
//!
//! These tests are fully hermetic: they drive a controlled in-process transport
//! instead of a live server, asserting the exact endpoint paths and JSON
//! request/response contracts recorded from the Go baseline, and covering the
//! error matrix (malformed response, HTTP failure, dimension mismatch, timeout,
//! and unknown-provider config failure).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rag_foundation::config::{Config, ConfigError};
use rag_foundation::embedding::{Embedder, EmbeddingError};
use rag_foundation::generation::{GenerationError, Generator};
use rag_foundation::ollama::{CancellationToken, OllamaClient, OllamaError, Transport};
use rag_foundation::provider::{self, Client};

type CallQueue = Mutex<VecDeque<Result<(u16, String, Vec<u8>), String>>>;

#[derive(Default)]
struct MockServer {
    responses: CallQueue,
    requests: Mutex<Vec<(String, Vec<u8>)>>,
}

impl MockServer {
    fn enqueue(&self, status: u16, status_text: &str, body: &[u8]) {
        self.responses.lock().unwrap().push_back(Ok((
            status,
            status_text.to_string(),
            body.to_vec(),
        )));
    }

    fn enqueue_timeout(&self) {
        self.responses
            .lock()
            .unwrap()
            .push_back(Err("request timed out".into()));
    }

    fn requests(&self) -> Vec<(String, Vec<u8>)> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for MockServer {
    fn post(&self, url: &str, body: &[u8]) -> Result<(u16, String, Vec<u8>), String> {
        self.requests
            .lock()
            .unwrap()
            .push((url.to_string(), body.to_vec()));
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err("no response queued".into()))
    }
}

fn transport() -> Arc<MockServer> {
    Arc::new(MockServer::default())
}

fn embedder(
    server: Arc<MockServer>,
    embed_dim: usize,
) -> rag_foundation::embedding::OllamaEmbedder {
    rag_foundation::embedding::OllamaEmbedder::new(
        OllamaClient::new("http://localhost:11434", server, Duration::from_secs(300)),
        "nomic-embed-text",
    )
    .with_embed_dim(embed_dim)
}

fn generator(server: Arc<MockServer>) -> rag_foundation::generation::OllamaGenerator {
    rag_foundation::generation::OllamaGenerator::new(
        OllamaClient::new("http://localhost:11434", server, Duration::from_secs(300)),
        "qwen3.8:27b-mlx",
    )
}

#[test]
fn embedding_endpoint_path_and_json_contract() {
    let server = transport();
    server.enqueue(200, "200 OK", br#"{"embeddings":[[0.1,0.2]]}"#);

    let vector = embedder(server.clone(), 2).embed("hello").unwrap();
    assert_eq!(vector, vec![0.1, 0.2]);

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "http://localhost:11434/api/embed");
    let body: serde_json::Value = serde_json::from_slice(&requests[0].1).unwrap();
    assert_eq!(body["model"], "nomic-embed-text");
    assert_eq!(body["input"], "hello");
}

#[test]
fn generation_endpoint_path_and_json_contract() {
    let server = transport();
    server.enqueue(200, "200 OK", br#"{"response":"pong"}"#);

    let answer = generator(server.clone()).generate("ping").unwrap();
    assert_eq!(answer, "pong");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "http://localhost:11434/api/generate");
    let body: serde_json::Value = serde_json::from_slice(&requests[0].1).unwrap();
    assert_eq!(body["model"], "qwen3.8:27b-mlx");
    assert_eq!(body["prompt"], "ping");
    assert_eq!(body["stream"], false);
}

#[test]
fn malformed_embedding_response_is_a_decode_error() {
    let server = transport();
    server.enqueue(200, "200 OK", b"<html>not json</html>");

    let err = embedder(server, 0).embed("hi").unwrap_err();
    match err {
        EmbeddingError::Client(OllamaError::Decode { path, message }) => {
            assert_eq!(path, "/api/embed");
            assert!(!message.is_empty());
        }
        other => panic!("expected decode error, got {other:?}"),
    }
}

#[test]
fn malformed_generation_response_is_a_decode_error() {
    let server = transport();
    server.enqueue(200, "200 OK", b"not json");

    let err = generator(server).generate("hi").unwrap_err();
    match err {
        GenerationError::Client(OllamaError::Decode { path, .. }) => {
            assert_eq!(path, "/api/generate");
        }
        other => panic!("expected decode error, got {other:?}"),
    }
}

#[test]
fn http_failure_reports_status_and_endpoint() {
    let server = transport();
    server.enqueue(500, "500 Internal Server Error", b"kaboom");

    let err = generator(server).generate("hi").unwrap_err();
    match err {
        GenerationError::Client(OllamaError::Status {
            path,
            status,
            detail,
        }) => {
            assert_eq!(path, "/api/generate");
            assert!(status.contains("500"), "{status}");
            assert_eq!(detail, "kaboom");
        }
        other => panic!("expected status error, got {other:?}"),
    }
}

#[test]
fn dimension_mismatch_names_expected_and_actual() {
    let server = transport();
    server.enqueue(200, "200 OK", br#"{"embeddings":[[1.0,2.0,3.0]]}"#);

    let err = embedder(server, 768).embed("hi").unwrap_err();
    assert!(matches!(
        err,
        EmbeddingError::DimensionMismatch {
            expected: 768,
            actual: 3
        }
    ));
    let message = err.to_string();
    assert!(message.contains("expected 768"), "{message}");
    assert!(message.contains("got 3"), "{message}");
}

#[test]
fn timeout_surfaces_a_timeout_error() {
    let server = transport();
    server.enqueue_timeout();

    let err = generator(server).generate("hi").unwrap_err();
    match err {
        GenerationError::Client(OllamaError::Transport { path, message }) => {
            assert_eq!(path, "/api/generate");
            assert!(message.contains("timed out"), "{message}");
        }
        other => panic!("expected transport timeout error, got {other:?}"),
    }
}

#[test]
fn cancellation_interrupts_an_in_flight_request() {
    let server = transport();
    let token = CancellationToken::new();
    token.cancel();

    let client = OllamaClient::new(
        "http://localhost:11434",
        server.clone(),
        Duration::from_secs(5),
    )
    .with_cancellation(token);
    let err = client
        .post_json::<_, serde_json::Value>(
            "/api/embed",
            &serde_json::json!({"model": "m", "input": "i"}),
        )
        .unwrap_err();

    assert!(matches!(err, OllamaError::Cancelled { .. }));
    assert!(server.requests().is_empty());
}

#[test]
fn unknown_provider_fails_during_configuration_loading() {
    let err = Config::load_from(|key| match key {
        "EMBED_PROVIDER" => Some("bogus".to_string()),
        _ => None,
    })
    .unwrap_err();

    assert_eq!(
        err,
        ConfigError::Invalid("EMBED_PROVIDER=\"bogus\" is not a registered provider".into())
    );
}

#[test]
fn registered_provider_resolves_to_an_implementation() {
    let server = transport();
    let client = Client {
        ollama_url: "http://localhost:11434".into(),
        embed_model: "nomic-embed-text".into(),
        chat_model: "qwen3.8:27b-mlx".into(),
        request_timeout: Duration::from_secs(300),
        transport: server.clone(),
    };

    assert!(provider::has_embedder("ollama"));
    assert!(provider::has_generator("ollama"));

    server.enqueue(200, "200 OK", br#"{"embeddings":[[1.0]]}"#);
    let embedder = provider::embedder("", &client).unwrap();
    assert_eq!(embedder.embed("hi").unwrap(), vec![1.0]);

    server.enqueue(200, "200 OK", br#"{"response":"ok"}"#);
    let generator = provider::generator("ollama", &client).unwrap();
    assert_eq!(generator.generate("hi").unwrap(), "ok");
}
