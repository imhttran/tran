//! Shared harness for the RUST-008A API contract acceptance.
//!
//! The crate contains no RAG logic. It starts the real `api` binary against a
//! dedicated database and a deterministic Ollama server, then drives the HTTP
//! endpoints.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use rag_foundation::storage::EMBEDDING_DIM;

/// Repository root, two levels above this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives three levels below the repository root")
        .to_path_buf()
}

/// The dedicated test DSN from the environment or the repository `.env`.
pub fn dsn() -> String {
    if let Ok(value) = std::env::var("DATABASE_URL") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    let env_path = repo_root().join(".env");
    if let Ok(text) = fs::read_to_string(&env_path) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "DATABASE_URL" {
                    let value = value.trim().trim_matches('"').trim_matches('\'');
                    if !value.is_empty() {
                        return value.to_string();
                    }
                }
            }
        }
    }
    panic!(
        "DATABASE_URL is not set: the API contract test requires the dedicated database in \
         the environment or the repository .env; it fails, it does not skip."
    )
}

/// A unique run tag for corpus and temp paths.
pub fn unique_tag(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    format!("{prefix}-{}-{nanos}", std::process::id())
}

/// Deletes only the run-owned corpus when dropped.
pub struct CorpusCleanup {
    dsn: String,
    corpus: String,
}

impl CorpusCleanup {
    pub fn new(dsn: impl Into<String>, corpus: impl Into<String>) -> Self {
        Self {
            dsn: dsn.into(),
            corpus: corpus.into(),
        }
    }
}

impl Drop for CorpusCleanup {
    fn drop(&mut self) {
        if let Ok(mut client) = postgres::Client::connect(&self.dsn, postgres::NoTls) {
            let _ = client.execute("DELETE FROM documents WHERE corpus = $1", &[&self.corpus]);
        }
    }
}

/// Builds the production API binary and returns its path.
pub fn build_api() -> PathBuf {
    let root = repo_root();
    let status = Command::new("cargo")
        .current_dir(&root)
        .args([
            "build",
            "--locked",
            "--manifest-path",
            "Cargo.toml",
            "--bin",
            "api",
        ])
        .status()
        .expect("run cargo build for api");
    assert!(status.success(), "cargo build --bin api failed");
    root.join("target/debug/api")
}

/// Reserves a loopback port by binding and immediately releasing it.
pub fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a free loopback port");
    listener.local_addr().expect("read the port").port()
}

/// A local HS256 OIDC issuer for the acceptance test. The API verifies the
/// signed token against this JWKS, so the test exercises the real auth path
/// without an external identity provider and without an auth-bypass switch.
pub struct TestAuth {
    issuer: String,
    audience: String,
    token: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TestAuth {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the test JWKS server");
        listener
            .set_nonblocking(true)
            .expect("nonblocking JWKS listener");
        let addr = listener.local_addr().expect("test JWKS address");
        let issuer = format!("http://{addr}");
        let secret = b"api-contract-test-secret";
        let jwks = format!(
            r#"{{"keys":[{{"kid":"test","alg":"HS256","k":"{}"}}]}}"#,
            String::from_utf8_lossy(secret)
        );
        let shutdown = Arc::new(AtomicBool::new(false));
        let stop = shutdown.clone();
        let handle = thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buffer = [0u8; 4096];
                        let _ = stream.read(&mut buffer);
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            jwks.len(),
                            jwks
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        let audience = "api-contract".to_string();
        let token = mint_hs256(&issuer, &audience, "api-contract-admin", secret);
        Self {
            issuer,
            audience,
            token,
            shutdown,
            handle: Some(handle),
        }
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub fn audience(&self) -> &str {
        &self.audience
    }

    pub fn jwks_url(&self) -> String {
        format!("{}/jwks", self.issuer)
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for TestAuth {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn mint_hs256(issuer: &str, audience: &str, subject: &str, secret: &[u8]) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_secs();
    let header = serde_json::json!({"alg": "HS256", "kid": "test", "typ": "JWT"});
    let payload = serde_json::json!({
        "iss": issuer,
        "aud": audience,
        "sub": subject,
        "iat": now,
        "exp": now + 3600,
    });
    let signing = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string().as_bytes()),
        URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes())
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC key");
    mac.update(signing.as_bytes());
    let signature = mac.finalize().into_bytes();
    format!("{}.{}", signing, URL_SAFE_NO_PAD.encode(signature))
}

/// Seeds the accepted admin identity and corpus grant used by the contract run.
/// The API loads roles from the database; the token only proves the subject.
pub fn seed_api_contract_admin(dsn: &str, issuer: &str, subject: &str, corpus: &str) {
    let mut client =
        postgres::Client::connect(dsn, postgres::NoTls).expect("connect to seed identity");
    client
        .execute(
            "INSERT INTO users (subject, issuer) VALUES ($1, $2) \
             ON CONFLICT (subject) DO UPDATE SET issuer = EXCLUDED.issuer",
            &[&subject, &issuer],
        )
        .expect("seed user");
    let user_id: i64 = client
        .query_one("SELECT id FROM users WHERE subject = $1", &[&subject])
        .expect("read seeded user")
        .get(0);
    client
        .execute(
            "DELETE FROM role_assignments WHERE user_id = $1",
            &[&user_id],
        )
        .expect("clear role assignments");
    client
        .execute(
            "INSERT INTO role_assignments (user_id, role, granted_by) VALUES ($1, 'admin', 'api-contract')",
            &[&user_id],
        )
        .expect("seed admin role");
    client
        .execute("DELETE FROM corpus_grants WHERE user_id = $1", &[&user_id])
        .expect("clear corpus grants");
    client
        .execute(
            "INSERT INTO corpus_grants (user_id, corpus, granted_by) VALUES ($1, $2, 'api-contract')",
            &[&user_id, &corpus],
        )
        .expect("seed corpus grant");
}

/// A running API process. Killed on drop.
pub struct ApiProcess {
    child: Child,
    base_url: String,
}

impl ApiProcess {
    /// Starts the API with the test configuration and waits for health.
    pub fn start(dsn: &str, corpus: &str, ollama_url: &str, auth: &TestAuth) -> Self {
        let binary = build_api();
        let port = free_port();
        let child = Command::new(&binary)
            .env("DATABASE_URL", dsn)
            .env("RAG_CORPUS", corpus)
            .env("API_HOST", "127.0.0.1")
            .env("API_PORT", port.to_string())
            .env("OLLAMA_URL", ollama_url)
            .env("OLLAMA_EMBED_MODEL", "parity-embed")
            .env("OLLAMA_CHAT_MODEL", "parity-chat")
            .env("EMBED_DIM", EMBEDDING_DIM.to_string())
            .env("REQUEST_TIMEOUT", "10s")
            .env("MIN_SIMILARITY", "0.0")
            .env("TOP_K", "20")
            .env("FINAL_K", "10")
            .env("EXPAND_LIMIT", "20")
            .env("CORPUS_LANGUAGE", "english")
            .env("AUTH_ISSUER", auth.issuer())
            .env("AUTH_AUDIENCE", auth.audience())
            .env("JWKS_URL", auth.jwks_url())
            .env("AUTH_RATE_LIMIT", "10000")
            .env("AUTH_CLOCK_SKEW_SECS", "60")
            .env("JWKS_CACHE_TTL_SECS", "60")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the production api binary");
        let base_url = format!("http://127.0.0.1:{port}");
        let mut process = Self { child, base_url };
        process.wait_for_health();
        process
    }

    /// Base URL of the running API.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn wait_for_health(&mut self) {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("build the health client");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(response) = client.get(format!("{}/api/health", self.base_url)).send() {
                if response.status().is_success() {
                    return;
                }
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                let mut stderr = String::new();
                if let Some(mut pipe) = self.child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                panic!("api exited before health ({status}):\n{stderr}");
            }
            if Instant::now() >= deadline {
                panic!("api did not become healthy within 30s at {}", self.base_url);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for ApiProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Deterministic Ollama server used by the API contract test.
pub struct FakeOllama {
    url: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeOllama {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake Ollama server");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let addr = listener.local_addr().expect("fake Ollama address");
        let shutdown = Arc::new(AtomicBool::new(false));
        let stop = shutdown.clone();
        let handle = thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => serve(&mut stream),
                    Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url: format!("http://{addr}"),
            shutdown,
            handle: Some(handle),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for FakeOllama {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn serve(stream: &mut TcpStream) {
    let Some(request) = read_request(stream) else {
        return;
    };
    let (status, response) = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/tags") => ("200 OK", "{\"models\":[]}".to_string()),
        ("POST", "/api/embed") => {
            let input = serde_json::from_slice::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|value| {
                    value
                        .get("input")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            (
                "200 OK",
                format!("{{\"embeddings\":[[{}]]}}", vector_for(&input)),
            )
        }
        ("POST", "/api/generate") => {
            let prompt = serde_json::from_slice::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|value| {
                    value
                        .get("prompt")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            let answer = answer_for(&prompt);
            (
                "200 OK",
                serde_json::json!({ "response": answer }).to_string(),
            )
        }
        _ => ("404 Not Found", "{}".to_string()),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Returns a citation to the first retrieved passage in the prompt.
///
/// Passages are separated by `---`; parse each block on its own so a `Page:`
/// line from a later passage is never attached to an earlier unpaginated one.
fn answer_for(prompt: &str) -> String {
    for block in prompt.split("\n\n---\n\n") {
        let mut source: Option<String> = None;
        let mut section: Option<String> = None;
        let mut page: Option<u32> = None;
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("Source: ") {
                if source.is_none() {
                    source = Some(value.trim().to_string());
                }
            } else if let Some(value) = line.strip_prefix("Section: ") {
                if section.is_none() {
                    section = Some(value.trim().to_string());
                }
            } else if let Some(value) = line.strip_prefix("Page: ") {
                if page.is_none() {
                    page = value.trim().parse().ok();
                }
            }
        }
        if let (Some(source), Some(section)) = (source, section) {
            return match page {
                Some(page) => format!(
                    "Deterministic answer from the retrieved context. [{source} - {section} - p.{page}]"
                ),
                None => {
                    format!("Deterministic answer from the retrieved context. [{source} - {section}]")
                }
            };
        }
    }
    "I do not have enough information.".to_string()
}

fn vector_for(input: &str) -> String {
    let mut vector = vec![0.0_f64; EMBEDDING_DIM];
    for (index, byte) in input.bytes().enumerate() {
        vector[(byte as usize + index) % EMBEDDING_DIM] += 1.0;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
        .iter()
        .map(|value| format!("{value:.8}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut data = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 8192];
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        data.extend_from_slice(&chunk[..read]);
        if let Some(position) = find(&data, b"\r\n\r\n") {
            break position;
        }
    };
    let headers = String::from_utf8_lossy(&data[..header_end]).to_string();
    let mut lines = headers.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let length = lines
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
        })
        .unwrap_or(0);
    let body_start = header_end + 4;
    while data.len() < body_start + length {
        let mut chunk = [0_u8; 8192];
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..read]);
    }
    Some(Request {
        method,
        path,
        body: data[body_start..body_start + length].to_vec(),
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
