//! Shared helpers for the retrieval/answering acceptance tests.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rag_foundation::storage::{DocumentStore, StoredChunk, EMBEDDING_DIM};
use rag_foundation::storage_pg::PgStore;

pub fn dsn() -> String {
    match std::env::var("DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => panic!(
            "DATABASE_URL is not set: the retrieval acceptance tests require the dedicated database. \
             Load the repository .env; these tests fail, they do not skip."
        ),
    }
}

pub fn unique_tag() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

pub fn seed_row(dsn: &str, source: &str, corpus: &str) {
    let store = PgStore::connect(dsn).expect("open the Rust store");
    store
        .write_chunk(&StoredChunk {
            content: "The answer is 42.".to_string(),
            source: Some(source.to_string()),
            section: Some("Intro".to_string()),
            chunk_index: 0,
            language: Some("english".to_string()),
            page: Some(2),
            corpus: Some(corpus.to_string()),
            content_hash: format!("hash-{source}"),
            embed_model: "test-embed".to_string(),
            embedding_dim: EMBEDDING_DIM as i32,
            chunker_config: "{}".to_string(),
            ingested_at: "2026-10-18T00:00:00Z".to_string(),
            embedding: vec![0.1_f64; EMBEDDING_DIM],
        })
        .expect("seed the acceptance row");
}

pub fn count_source(dsn: &str, source: &str) -> i64 {
    let mut client =
        postgres::Client::connect(dsn, postgres::NoTls).expect("connect to count the test source");
    client
        .query_one(
            "SELECT count(*) FROM documents WHERE source = $1",
            &[&source],
        )
        .expect("count the test source")
        .get(0)
}

/// Deletes only the test-owned rows when dropped (including on panic).
pub struct Cleanup {
    dsn: String,
    source: String,
}

impl Cleanup {
    pub fn new(dsn: &str, source: &str) -> Self {
        Self {
            dsn: dsn.to_string(),
            source: source.to_string(),
        }
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Ok(mut client) = postgres::Client::connect(&self.dsn, postgres::NoTls) {
            let _ = client.execute("DELETE FROM documents WHERE source = $1", &[&self.source]);
        }
    }
}

/// Builds the production `query` binary once and returns its path.
pub fn query_binary() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve the crate root");
    let manifest = root.join("Cargo.toml");
    let status = Command::new("cargo")
        .args([
            "build",
            "--locked",
            "--manifest-path",
            manifest.to_str().expect("utf-8 manifest path"),
            "--bin",
            "query",
        ])
        .status()
        .expect("run cargo build for the query binary");
    assert!(
        status.success(),
        "building the production query binary failed"
    );
    root.join("target/debug/query")
}

/// Runs the production query binary with the test environment.
pub fn run_query(
    binary: &Path,
    question: &str,
    dsn: &str,
    ollama_url: &str,
    corpus: &str,
) -> Output {
    Command::new(binary)
        .arg(question)
        .env("DATABASE_URL", dsn)
        .env("OLLAMA_URL", ollama_url)
        .env("OLLAMA_EMBED_MODEL", "test-embed")
        .env("OLLAMA_CHAT_MODEL", "test-chat")
        .env("EMBED_DIM", EMBEDDING_DIM.to_string())
        .env("MIN_SIMILARITY", "0.0")
        .env("FINAL_K", "1")
        .env("EXPAND_LIMIT", "1")
        .env("CONTEXT_BUDGET", "2000")
        .env("REQUEST_TIMEOUT", "5s")
        .env("QUERY_REWRITE", "false")
        .env("RAG_CORPUS", corpus)
        .output()
        .expect("spawn the production query binary")
}

/// A scripted Ollama HTTP server for `/api/embed` and `/api/generate`.
pub struct FakeOllama {
    url: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeOllama {
    pub fn start(source: &str, fail_generate: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake Ollama server");
        listener
            .set_nonblocking(true)
            .expect("set the fake Ollama server nonblocking");
        let addr = listener.local_addr().expect("fake Ollama address");
        let shutdown = Arc::new(AtomicBool::new(false));
        let stop = shutdown.clone();
        let source = source.to_string();
        let handle = thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => serve(&mut stream, &source, fail_generate),
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

fn serve(stream: &mut TcpStream, source: &str, fail_generate: bool) {
    let Some(path) = read_path(stream) else {
        return;
    };
    let (status, body) = match path.as_str() {
        "/api/embed" => (
            200,
            format!("{{\"embeddings\":[[{}]]}}", embedding_values()),
        ),
        "/api/generate" if fail_generate => (
            500,
            "{\"error\":\"generation dependency unavailable\"}".to_string(),
        ),
        "/api/generate" => (
            200,
            format!("{{\"response\":\"The answer is 42 [{source} - Intro - p.2].\"}}"),
        ),
        _ => (404, "{}".to_string()),
    };
    let reason = if status == 200 {
        "OK"
    } else {
        "Internal Server Error"
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn read_path(stream: &mut TcpStream) -> Option<String> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut data = Vec::new();
    let header_end;
    loop {
        let mut chunk = [0_u8; 4096];
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find(&data, b"\r\n\r\n") {
            header_end = pos;
            break;
        }
    }
    let headers = String::from_utf8_lossy(&data[..header_end]).to_string();
    let first = headers.lines().next()?;
    let path = first.split_whitespace().nth(1)?.to_string();

    let length = headers
        .lines()
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
        let mut chunk = [0_u8; 4096];
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
    }
    Some(path)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn embedding_values() -> String {
    vec!["0.1"; EMBEDDING_DIM].join(",")
}
