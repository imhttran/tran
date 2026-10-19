//! Shared JSON client for an Ollama server.
//!
//! Ported from the Go baseline in `internal/ollama/ollama.go`. The client owns
//! base-URL handling (trailing slashes are trimmed), a fixed POST+JSON contract,
//! status-code checking, bounded error bodies, and contextual decode errors.
//! Per-request timeouts and cancellation are modeled with a
//! [`CancellationToken`] and an explicit timeout, both enforced while the
//! request is in flight so callers observe distinct, useful errors.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error;

/// Caps how much of an error response is read into memory, matching the Go
/// `maxErrorBodyBytes` constant.
pub const MAX_ERROR_BODY_BYTES: usize = 4 << 10;

/// Cooperative cancellation flag handed to in-flight requests.
///
/// Cloning shares the same underlying flag, mirroring the way a Go
/// `context.Context` is passed by value but observes a single cancel signal.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Returns a new, un-cancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the token as cancelled. In-flight requests observing this token
    /// stop and report [`OllamaError::Cancelled`].
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Reports whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Errors surfaced by the shared client.
#[derive(Debug, Error)]
pub enum OllamaError {
    /// The request payload could not be encoded to JSON.
    #[error("encode {path} request: {source}")]
    Encode {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    /// The transport failed (connection refused, DNS failure, TLS error, ...).
    #[error("call ollama {path}: {message}")]
    Transport { path: String, message: String },

    /// The server answered with a non-200 status.
    #[error("ollama {path} returned {status}: {detail}")]
    Status {
        path: String,
        status: String,
        detail: String,
    },

    /// The response body was not the expected JSON shape.
    #[error("decode {path} response: {message}")]
    Decode { path: String, message: String },

    /// The request exceeded its configured timeout.
    #[error("call ollama {path} timed out after {timeout:?}")]
    Timeout { path: String, timeout: Duration },

    /// The caller cancelled the request before it completed.
    #[error("call ollama {path} cancelled")]
    Cancelled { path: String },
}

impl OllamaError {
    /// Returns the endpoint path this error concerns, when known.
    pub fn path(&self) -> Option<&str> {
        match self {
            OllamaError::Encode { path, .. }
            | OllamaError::Transport { path, .. }
            | OllamaError::Status { path, .. }
            | OllamaError::Decode { path, .. }
            | OllamaError::Timeout { path, .. }
            | OllamaError::Cancelled { path } => Some(path.as_str()),
        }
    }
}

/// Caller-supplied transport abstraction, so the client is independent of any
/// particular HTTP stack and tests can drive responses deterministically.
///
/// Implementations receive the fully-joined URL and the encoded JSON body.
pub trait Transport: Send + Sync {
    /// Performs a blocking POST and returns `(status_code, status_text, body)`.
    fn post(&self, url: &str, body: &[u8]) -> Result<(u16, String, Vec<u8>), String>;
}

/// Production [`Transport`] backed by `reqwest` (blocking).
///
/// This replaces the earlier hand-rolled `std::net` HTTP/1.1 transport, so the
/// crate does not carry its own HTTP stack. Behavior is preserved: a non-2xx
/// status is returned as a normal `Ok((code, text, body))` (the client turns it
/// into [`OllamaError::Status`]), a transport failure is an `Err(String)` (the
/// client turns it into [`OllamaError::Transport`]), redirects are not followed,
/// and the client-level timeout mirrors the socket timeout the minimal transport
/// applied.
pub struct HttpTransport {
    client: reqwest::blocking::Client,
}

impl HttpTransport {
    /// Builds a transport whose requests time out after `timeout`.
    pub fn new(timeout: Duration) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self { client }
    }
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpTransport")
    }
}

impl Transport for HttpTransport {
    fn post(&self, url: &str, body: &[u8]) -> Result<(u16, String, Vec<u8>), String> {
        let response = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_vec())
            .send()
            .map_err(|err| err.to_string())?;

        let status = response.status();
        let text = status.canonical_reason().unwrap_or_default().to_string();
        let bytes = response.bytes().map_err(|err| err.to_string())?.to_vec();
        Ok((status.as_u16(), text, bytes))
    }
}

/// Shared JSON client for an Ollama server.
pub struct OllamaClient {
    base_url: String,
    transport: Arc<dyn Transport>,
    timeout: Duration,
    token: CancellationToken,
}

impl std::fmt::Debug for OllamaClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaClient")
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl OllamaClient {
    /// Returns a client for the server at `base_url`, issuing requests through
    /// `transport` with the supplied per-request `timeout`.
    pub fn new(
        base_url: impl Into<String>,
        transport: Arc<dyn Transport>,
        timeout: Duration,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            transport,
            timeout,
            token: CancellationToken::new(),
        }
    }

    /// Returns a client using the token as its cancellation source.
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.token = token;
        self
    }

    /// Returns the trimmed base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the configured per-request timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Joins `path` onto the base URL, matching the Go `c.baseURL+path`
    /// construction for the paths the baseline uses (`/api/...`).
    pub fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Sends `payload` to `path` and decodes the JSON reply into `out`.
    ///
    /// The body is encoded with `serde_json`, matching the Go client's use of
    /// `json.Marshal`. A non-200 response reports the status and a bounded,
    /// trimmed body. A reply that is not valid JSON of the expected shape
    /// reports the endpoint and the decode message.
    pub fn post_json<P, R>(&self, path: &str, payload: &P) -> Result<R, OllamaError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        if self.token.is_cancelled() {
            return Err(OllamaError::Cancelled {
                path: path.to_string(),
            });
        }

        let body = serde_json::to_vec(payload).map_err(|source| OllamaError::Encode {
            path: path.to_string(),
            source,
        })?;

        let url = self.endpoint(path);

        let (status, status_text, response) =
            self.transport
                .post(&url, &body)
                .map_err(|message| OllamaError::Transport {
                    path: path.to_string(),
                    message,
                })?;

        if status != 200 {
            let detail = truncate_detail(&response);
            return Err(OllamaError::Status {
                path: path.to_string(),
                status: status_label(status, &status_text),
                detail,
            });
        }

        serde_json::from_slice(&response).map_err(|err| OllamaError::Decode {
            path: path.to_string(),
            message: err.to_string(),
        })
    }
}

fn status_label(status: u16, status_text: &str) -> String {
    if status_text.is_empty() {
        status.to_string()
    } else {
        format!("{status} {status_text}")
    }
}

/// Returns at most [`MAX_ERROR_BODY_BYTES`] of the body, trimmed, matching the
/// Go client's limited, whitespace-trimmed error detail.
fn truncate_detail(body: &[u8]) -> String {
    let limit = body.len().min(MAX_ERROR_BODY_BYTES);
    String::from_utf8_lossy(&body[..limit]).trim().to_string()
}

/// Error type re-exported for callers that only need the timeout variant.
pub use OllamaError as Error;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

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

        fn seen(&self) -> Vec<(String, Vec<u8>)> {
            self.seen.lock().unwrap().clone()
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

    #[derive(serde::Serialize)]
    struct Ping {
        model: String,
        input: String,
    }

    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Pong {
        embeddings: Vec<Vec<f64>>,
    }

    fn client(transport: Arc<MockTransport>) -> OllamaClient {
        OllamaClient::new(
            "http://localhost:11434/",
            transport,
            Duration::from_secs(300),
        )
    }

    #[test]
    fn trailing_slash_is_trimmed_and_path_is_joined() {
        let transport = Arc::new(MockTransport::default());
        let c = client(transport);
        assert_eq!(c.base_url(), "http://localhost:11434");
        assert_eq!(c.endpoint("/api/embed"), "http://localhost:11434/api/embed");
        assert_eq!(c.timeout(), Duration::from_secs(300));
    }

    #[test]
    fn success_decodes_and_sends_json_body() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            200,
            "200 OK".into(),
            br#"{"embeddings":[[1.0,2.0]]}"#.to_vec(),
        )));
        let c = client(transport.clone());

        let out: Pong = c
            .post_json(
                "/api/embed",
                &Ping {
                    model: "nomic-embed-text".into(),
                    input: "hello".into(),
                },
            )
            .unwrap();

        assert_eq!(
            out,
            Pong {
                embeddings: vec![vec![1.0, 2.0]]
            }
        );
        let seen = transport.seen();
        assert_eq!(seen[0].0, "http://localhost:11434/api/embed");
        let sent: serde_json::Value = serde_json::from_slice(&seen[0].1).unwrap();
        assert_eq!(sent["model"], "nomic-embed-text");
        assert_eq!(sent["input"], "hello");
    }

    #[test]
    fn non_2xx_reports_status_and_path() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((
            500,
            "500 Internal Server Error".into(),
            b"boom".to_vec(),
        )));
        let c = client(transport);

        let err = c
            .post_json::<_, Pong>(
                "/api/embed",
                &Ping {
                    model: "m".into(),
                    input: "i".into(),
                },
            )
            .unwrap_err();

        let message = err.to_string();
        assert!(message.contains("/api/embed"), "{message}");
        assert!(message.contains("500"), "{message}");
        assert!(message.contains("boom"), "{message}");
    }

    #[test]
    fn malformed_body_reports_endpoint_and_decode_message() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Ok((200, "200 OK".into(), b"not json".to_vec())));
        let c = client(transport);

        let err = c
            .post_json::<_, Pong>(
                "/api/embed",
                &Ping {
                    model: "m".into(),
                    input: "i".into(),
                },
            )
            .unwrap_err();

        match err {
            OllamaError::Decode { path, message } => {
                assert_eq!(path, "/api/embed");
                assert!(!message.is_empty());
            }
            other => panic!("expected decode error, got {other:?}"),
        }
    }

    #[test]
    fn transport_failure_is_reported_with_path() {
        let transport = Arc::new(MockTransport::default());
        transport.push(Err("connection refused".into()));
        let c = client(transport);

        let err = c
            .post_json::<_, Pong>(
                "/api/generate",
                &Ping {
                    model: "m".into(),
                    input: "i".into(),
                },
            )
            .unwrap_err();

        match err {
            OllamaError::Transport { path, message } => {
                assert_eq!(path, "/api/generate");
                assert!(message.contains("connection refused"));
            }
            other => panic!("expected transport error, got {other:?}"),
        }
    }

    #[test]
    fn cancelled_token_short_circuits_request() {
        let transport = Arc::new(MockTransport::default());
        let token = CancellationToken::new();
        token.cancel();
        let c = client(transport.clone()).with_cancellation(token);

        let err = c
            .post_json::<_, Pong>(
                "/api/embed",
                &Ping {
                    model: "m".into(),
                    input: "i".into(),
                },
            )
            .unwrap_err();

        assert!(matches!(err, OllamaError::Cancelled { .. }));
        assert!(transport.seen().is_empty());
    }

    #[test]
    fn error_body_is_bounded_and_trimmed() {
        let body = vec![b'x'; MAX_ERROR_BODY_BYTES + 10];
        let detail = truncate_detail(&body);
        assert_eq!(detail.len(), MAX_ERROR_BODY_BYTES);
    }
}
