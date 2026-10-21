//! Shared harness for the RUST-008A API contract acceptance.
//!
//! The crate contains no RAG logic. It starts the real `api` binary against a
//! dedicated database and a deterministic Ollama server, then drives the HTTP
//! endpoints.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};

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

/// Builds the operator ingest binary and returns its path.
pub fn build_ingest() -> PathBuf {
    let root = repo_root();
    let status = Command::new("cargo")
        .current_dir(&root)
        .args([
            "build",
            "--locked",
            "--manifest-path",
            "Cargo.toml",
            "--bin",
            "ingest",
        ])
        .status()
        .expect("run cargo build for ingest");
    assert!(status.success(), "cargo build --bin ingest failed");
    root.join("target/debug/ingest")
}

/// The trusted local operator ingest, used to publish documents explicitly.
///
/// API submissions stay pending and write no chunks, so the only publication
/// path in this stage is this operator boundary. It records an approved
/// `operator_import` version and links the chunks it wrote.
pub struct OperatorIngest {
    binary: PathBuf,
    document_root: PathBuf,
}

impl OperatorIngest {
    pub fn new() -> Self {
        let binary = build_ingest();
        let document_root =
            std::env::temp_dir().join(unique_tag("api-contract-operator-documents"));
        fs::create_dir_all(&document_root).expect("create the operator document root");
        Self {
            binary,
            document_root,
        }
    }

    /// Publishes one source file and panics if the operator boundary refuses it.
    pub fn publish(&self, dsn: &str, corpus: &str, ollama_url: &str, path: &Path) {
        let output = Command::new(&self.binary)
            .env("DATABASE_URL", dsn)
            .env("RAG_CORPUS", corpus)
            .env("OLLAMA_URL", ollama_url)
            .env("OLLAMA_EMBED_MODEL", "parity-embed")
            .env("OLLAMA_CHAT_MODEL", "parity-chat")
            .env("EMBED_DIM", EMBEDDING_DIM.to_string())
            .env("REQUEST_TIMEOUT", "10s")
            .env("CORPUS_LANGUAGE", "english")
            .env("DOCUMENT_ROOT", &self.document_root)
            .arg(path)
            .output()
            .expect("run the operator ingest CLI");
        assert!(
            output.status.success(),
            "operator ingest failed for {}: {}{}",
            path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for OperatorIngest {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.document_root);
    }
}

/// Reserves a loopback port by binding and immediately releasing it.
pub fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a free loopback port");
    listener.local_addr().expect("read the port").port()
}

/// Test-only RS256 private key (PKCS#8 DER, base64). This is a synthetic key
/// generated for the acceptance suite; it is never used outside tests.
const TEST_KEY_DER_B64: &str = "MIIEugIBADANBgkqhkiG9w0BAQEFAASCBKQwggSgAgEAAoIBAQC+V5EZ9MtnqFbydmFC46NtOIuNaQcyBxm2Vupukx+Wm6w8aZ2mLWJ0GodTU00VyuLfnh2901eNqGaG62f2m4DXCi7AziILWGXBgxnKbqSYK2P+0hG++JJMaqjX5qN5VN0f5UmmSlAKoKkInkePpe/GDsVnox6lsEFMQKtU8bkkqHcOhaOu4QbWKzgStIoey5qkVIulu6pbVbLnbO5xpubJneQiP1PkpG1Um/A20ppQaBNyr6a+Eu7riZv9TsGeGqwQx9ic5JAEar9ZR+yoUHFY19X9Vbj54SLo3wPhvdZaHKwVQviX5dqpFwWGUYUrEVESaB7zpvraH35q0vTS8fPZAgMBAAECgf91TSx82xLDtViKrYvcpH4sYk6veUYxqB19ORMvZyZuTKPP7JwQgF8jx74Vk1op2Dy0krRbHR9X4LzoNbOPcYoOt8jUCWgTyQiGQcUSFZUmuqft0TLJMUT3wfbDW3lSMyUkAu412adHB4ibz5oXKIn5nocrgf16uD4OHe6rt39ZY82Ae5F1AvQVWlPk1IvqUQn9fGFnHCDuqykl4h3QlPVP0bUrldaQcr8kRfM8J/LPKyICxxrfl3kE/G9hsKaPy2zaF43gDMJF8J2HrH7POMLPLmO/bEO+3kZqomBMVopWBjOCqW3NSor+zwVcbITwsM7rx2nlbpa3nwBPyTYqkO8CgYEA9gmQGvPWJ6BEr1FOvyQb5qEZ0U9qxSvGYSL+LUKDXHMyJDKHRUWOlNho1un6RXAsjVyDGo55COpmLT6JYQwKqs0K3RwfClo7gATlxTXmQs1sbo7ZLGLwHE65Xnj/uBe5YLWxulzuU1XSBsFh8T8UXhPeAggSNnX9XZd5pnV2GMcCgYEAxgypzg+5PE4Dgesq/MZVqolGIlA4O+3Y1jPkLxhDHh39RRCds9W8lQ2qNu1ef1ZjUYvaXJ0G+Uu9K9YYrM4iofCFv5KBiCtnTSvH2mXyXr21EvPm67pevMvWzBVsb7RNHKOP722rTKlLWec1i8I3O1h0ebjTiLS1fSe/VUH9Ll8CgYBbMkg79U3X+kv7QAQUTWoaHnTXZLyKwSMTNklItXLDfd+EE0aYoqOrAFw77RZbTqz06Omfmle6ukeNVbTOSAxcKwxv0NCgobmURjx00Pm8oTAj+sb88N9XXsbPAcKnlgOcmTx7e57LPPBQmLWZGmyOQobz8dHuZeW4H0F0Ne3EWQKBgDUvsWplTpmjWc3KGgFIQjlKJEuSv4dYmKLtzJnZKi0sYJfrnm5tN/fPc6bMmFQaHxIHPN0iTn0kmejr1jSSJzrWHRUVgzPOdC+FUmaUekJ3L+mnFpy3sDmDhGvKVRSqlwrZ4g2qphYwYMwEC508yxhk6OLHgAd2gEbcJcX/sr6BAoGAWoRdff3xMmzozsEdHnTOKYzf8OxDDYgJ0a+voWJtHUWdHV8qtND8IDCZe5vwnjy8h0MllavKj+14v64vFpdNnI/gQD9GkoAoJ03SoSGHOoloZo87am1EMJyv6ifsWRNOs2Hg9mEl9VLfyeZ4H06s5NEfhIu/Awpp3Z3fiXW3pTM=";
const TEST_KEY_N: &str = "vleRGfTLZ6hW8nZhQuOjbTiLjWkHMgcZtlbqbpMflpusPGmdpi1idBqHU1NNFcri354dvdNXjahmhutn9puA1wouwM4iC1hlwYMZym6kmCtj_tIRvviSTGqo1-ajeVTdH-VJpkpQCqCpCJ5Hj6Xvxg7FZ6MepbBBTECrVPG5JKh3DoWjruEG1is4ErSKHsuapFSLpbuqW1Wy52zucabmyZ3kIj9T5KRtVJvwNtKaUGgTcq-mvhLu64mb_U7BnhqsEMfYnOSQBGq_WUfsqFBxWNfV_VW4-eEi6N8D4b3WWhysFUL4l-XaqRcFhlGFKxFREmge86b62h9-atL00vHz2Q";
const TEST_KEY_E: &str = "AQAB";

/// A local RS256 OIDC issuer for the acceptance test. The API verifies the
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
        let jwks = format!(
            r#"{{"keys":[{{"kty":"RSA","kid":"test","alg":"RS256","n":"{}","e":"{}"}}]}}"#,
            TEST_KEY_N, TEST_KEY_E
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
        let der = STANDARD
            .decode(TEST_KEY_DER_B64)
            .expect("decode the test RSA key");
        let key_pair = RsaKeyPair::from_pkcs8(&der).expect("parse the test RSA key");
        let token = mint_rs256(&issuer, &audience, "api-contract-admin", &key_pair);
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

fn mint_rs256(issuer: &str, audience: &str, subject: &str, key_pair: &RsaKeyPair) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_secs();
    let header = serde_json::json!({"alg": "RS256", "kid": "test", "typ": "JWT"});
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
    let rng = SystemRandom::new();
    let mut signature = vec![0u8; key_pair.public().modulus_len()];
    key_pair
        .sign(&RSA_PKCS1_SHA256, &rng, signing.as_bytes(), &mut signature)
        .expect("sign the test RS256 token");
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
             ON CONFLICT (issuer, subject) DO UPDATE SET updated_at = now(), deactivated_at = NULL",
            &[&subject, &issuer],
        )
        .expect("seed user");
    let user_id: i64 = client
        .query_one(
            "SELECT id FROM users WHERE subject = $1 AND issuer = $2",
            &[&subject, &issuer],
        )
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
    /// Disposable private document root owned by this run only.
    document_root: PathBuf,
}

impl ApiProcess {
    /// Starts the API with the test configuration and waits for health.
    pub fn start(dsn: &str, corpus: &str, ollama_url: &str, auth: &TestAuth) -> Self {
        let binary = build_api();
        let port = free_port();
        // Originals are stored privately. Point the run at its own temp root so
        // the test never writes into the checkout and never touches operator
        // documents.
        let document_root = std::env::temp_dir().join(unique_tag("api-contract-documents"));
        fs::create_dir_all(&document_root).expect("create the run document root");
        let child = Command::new(&binary)
            .env("DATABASE_URL", dsn)
            .env("DOCUMENT_ROOT", &document_root)
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
        let mut process = Self {
            child,
            base_url,
            document_root,
        };
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
        let _ = fs::remove_dir_all(&self.document_root);
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
