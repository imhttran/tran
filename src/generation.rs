//! Answer prompts with a local model.
//!
//! [`OllamaGenerator`] is ported from the Go baseline in
//! `internal/generation/generation.go`: the endpoint path (`/api/generate`), the
//! JSON request (`model`, `prompt`, `stream: false`), and the JSON response
//! (`response`) mirror the baseline. [`OpenAICompatibleGenerator`] sends the
//! same prompt to any OpenAI-compatible server (for example oMLX) through
//! `/v1/chat/completions`.

use crate::ollama::{OllamaClient, OllamaError};
use thiserror::Error;

/// Produces text from a prompt. Consumers depend on this interface rather than
/// a concrete implementation, so tests can substitute a stub.
pub trait Generator: Send + Sync {
    fn generate(&self, prompt: &str) -> Result<String, GenerationError>;
}

/// Lets a provider chosen at runtime stand wherever a concrete generator is
/// expected.
impl Generator for Box<dyn Generator> {
    fn generate(&self, prompt: &str) -> Result<String, GenerationError> {
        (**self).generate(prompt)
    }
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
    /// An OpenAI-compatible reply carried no message content.
    #[error("generate answer: the reply had no message content")]
    EmptyReply,
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

/// The OpenAI-compatible endpoint used for generation, relative to the server
/// root.
pub const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

#[derive(serde::Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(serde::Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 1],
    stream: bool,
}

#[derive(serde::Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

#[derive(serde::Deserialize)]
struct ChatChoice {
    message: ChatReply,
}

#[derive(serde::Deserialize)]
struct ChatReply {
    #[serde(default)]
    content: Option<String>,
}

/// Produces text with a model on an OpenAI-compatible server (for example
/// oMLX, LM Studio, or llama.cpp's server).
///
/// The prompt is sent as one user message with `stream: false`. A reasoning
/// model's `<think>...</think>` block is removed from the reply, so callers see
/// only the answer and its citations.
pub struct OpenAICompatibleGenerator {
    client: OllamaClient,
    model: String,
}

impl OpenAICompatibleGenerator {
    /// Returns a generator for `model` on the server behind `client`. The
    /// client's base URL is the server root; a trailing `/v1` is accepted.
    pub fn new(client: OllamaClient, model: impl Into<String>) -> Self {
        Self {
            client,
            model: model.into(),
        }
    }

    fn path(&self) -> &'static str {
        if self.client.base_url().ends_with("/v1") {
            "/chat/completions"
        } else {
            CHAT_COMPLETIONS_PATH
        }
    }
}

impl Generator for OpenAICompatibleGenerator {
    fn generate(&self, prompt: &str) -> Result<String, GenerationError> {
        let request = ChatRequest {
            model: &self.model,
            messages: [ChatMessage {
                role: "user",
                content: prompt,
            }],
            stream: false,
        };
        let reply: ChatResponse = self.client.post_json(self.path(), &request)?;
        let content = reply
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .ok_or(GenerationError::EmptyReply)?;
        Ok(strip_thinking(&content))
    }
}

/// Removes `<think>...</think>` blocks a reasoning model may put before its
/// answer. An unclosed block means the model never reached its answer, so
/// nothing after `<think>` is kept.
pub fn strip_thinking(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
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

    fn openai_generator(transport: Arc<MockTransport>, base: &str) -> OpenAICompatibleGenerator {
        OpenAICompatibleGenerator::new(
            OllamaClient::new(base, transport, Duration::from_secs(300)),
            "Qwen3-4B-4bit",
        )
    }

    #[test]
    fn openai_compatible_request_path_and_body_match_contract() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            200,
            "200 OK".into(),
            br#"{"choices":[{"message":{"role":"assistant","content":"hi there"}}]}"#.to_vec(),
        )));
        let g = openai_generator(transport.clone(), "http://127.0.0.1:8000");

        assert_eq!(g.generate("say hi").unwrap(), "hi there");

        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen[0].0, "http://127.0.0.1:8000/v1/chat/completions");
        let body: serde_json::Value = serde_json::from_slice(&seen[0].1).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "model": "Qwen3-4B-4bit",
                "messages": [{"role": "user", "content": "say hi"}],
                "stream": false
            })
        );
    }

    #[test]
    fn openai_compatible_accepts_a_base_url_ending_in_v1() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            200,
            "200 OK".into(),
            br#"{"choices":[{"message":{"content":"ok"}}]}"#.to_vec(),
        )));
        let g = openai_generator(transport.clone(), "http://localhost:8000/v1/");
        assert_eq!(g.generate("x").unwrap(), "ok");
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen[0].0, "http://localhost:8000/v1/chat/completions");
    }

    #[test]
    fn openai_compatible_removes_a_thinking_block() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            200,
            "200 OK".into(),
            br#"{"choices":[{"message":{"content":"<think>\nreasoning here\n</think>\n\nThe answer [a.md]."}}]}"#
                .to_vec(),
        )));
        let g = openai_generator(transport, "http://127.0.0.1:8000");
        assert_eq!(g.generate("q").unwrap(), "The answer [a.md].");
    }

    #[test]
    fn openai_compatible_reply_without_content_is_an_error() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((200, "200 OK".into(), br#"{"choices":[]}"#.to_vec())));
        let g = openai_generator(transport, "http://127.0.0.1:8000");
        assert!(matches!(g.generate("q"), Err(GenerationError::EmptyReply)));
    }

    #[test]
    fn openai_compatible_error_status_is_reported() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            404,
            "Not Found".into(),
            br#"{"error":"model not found"}"#.to_vec(),
        )));
        let g = openai_generator(transport, "http://127.0.0.1:8000");
        let err = g.generate("q").unwrap_err().to_string();
        assert!(err.contains("404"), "{err}");
        assert!(err.contains("model not found"), "{err}");
    }

    #[test]
    fn strip_thinking_keeps_plain_text_and_drops_an_unclosed_block() {
        assert_eq!(strip_thinking("  plain answer "), "plain answer");
        assert_eq!(strip_thinking("<think>a</think>x<think>b</think>y"), "xy");
        assert_eq!(strip_thinking("<think>never finished"), "");
    }

    #[test]
    fn a_boxed_generator_delegates() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            200,
            "200 OK".into(),
            br#"{"response":"boxed"}"#.to_vec(),
        )));
        let boxed: Box<dyn Generator> = Box::new(generator(transport));
        assert_eq!(boxed.generate("q").unwrap(), "boxed");
    }
}
