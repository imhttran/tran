//! RUST-007E production Markdown/plain-text ingestion acceptance.
//!
//! Drives the real `ingest` binary against the dedicated PostgreSQL/pgvector
//! database with a deterministic embedding server. Proves the production path
//! loads `.md`/`.txt`, preserves provenance and ordering, skips unchanged
//! sources, rejects empty/malformed/unknown inputs before any effect, and keeps
//! previously stored rows when an embedding or dimension failure occurs.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ingest_md_text_acceptance::repo_root;
use rag_foundation::storage::EMBEDDING_DIM;

fn dsn() -> String {
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
        "DATABASE_URL is not set: the Markdown/plain-text acceptance test requires the \
         dedicated database in the environment or the repository .env; it fails, it does not skip."
    )
}

fn unique_tag() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    format!("ingest-md-text-{}-{nanos}", std::process::id())
}

struct CorpusCleanup {
    dsn: String,
    corpus: String,
}

impl Drop for CorpusCleanup {
    fn drop(&mut self) {
        if let Ok(mut client) = postgres::Client::connect(&self.dsn, postgres::NoTls) {
            let _ = client.execute("DELETE FROM documents WHERE corpus = $1", &[&self.corpus]);
        }
    }
}

fn build_ingest() -> PathBuf {
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

#[derive(Clone, Copy)]
struct RunEnv<'a> {
    dsn: &'a str,
    corpus: &'a str,
    source: &'a str,
    url: &'a str,
    embed_dim: usize,
}

fn run_ingest(binary: &Path, input: &Path, env: &RunEnv<'_>) -> Output {
    Command::new(binary)
        .arg(input)
        .env("DATABASE_URL", env.dsn)
        .env("RAG_CORPUS", env.corpus)
        .env("RAG_SOURCE", env.source)
        .env("OLLAMA_URL", env.url)
        .env("OLLAMA_EMBED_MODEL", "parity-embed")
        .env("OLLAMA_CHAT_MODEL", "parity-chat")
        .env("EMBED_DIM", env.embed_dim.to_string())
        .env("REQUEST_TIMEOUT", "5s")
        .env("CORPUS_LANGUAGE", "english")
        .env("CHUNK_SIZE", "50")
        .env("CHUNK_OVERLAP", "10")
        .output()
        .expect("spawn the production ingest binary")
}

#[derive(Debug)]
struct Row {
    section: Option<String>,
    chunk_index: i32,
    page: Option<i32>,
    language: Option<String>,
    content_hash: String,
    corpus: Option<String>,
    content: String,
}

fn rows_for(dsn: &str, corpus: &str, source: &str) -> Vec<Row> {
    let mut client =
        postgres::Client::connect(dsn, postgres::NoTls).expect("connect to the database");
    client
        .query(
            "SELECT section, chunk_index, page, language, content_hash, corpus, content \
             FROM documents WHERE corpus = $1 AND source = $2 ORDER BY chunk_index",
            &[&corpus, &source],
        )
        .expect("read the ingested rows")
        .into_iter()
        .map(|row| Row {
            section: row.get(0),
            chunk_index: row.get(1),
            page: row.get(2),
            language: row.get(3),
            content_hash: row.get(4),
            corpus: row.get(5),
            content: row.get(6),
        })
        .collect()
}

fn total_rows(dsn: &str, corpus: &str) -> i64 {
    let mut client =
        postgres::Client::connect(dsn, postgres::NoTls).expect("connect to the database");
    client
        .query_one(
            "SELECT count(*) FROM documents WHERE corpus = $1",
            &[&corpus],
        )
        .expect("count the run corpus")
        .get(0)
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(tag);
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("remove the previous temp dir");
    }
    fs::create_dir_all(&dir).expect("create the temp dir");
    dir
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// A deterministic Ollama `/api/embed` server.
struct FakeOllama {
    url: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeOllama {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake embedding server");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let addr = listener.local_addr().expect("fake server address");
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
}

impl Drop for FakeOllama {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(stream: &mut TcpStream) {
    let Some((path, body)) = read_request(stream) else {
        return;
    };
    let response = if path == "/api/embed" {
        let input = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("input")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let values = vector_for(&input);
        format!("{{\"embeddings\":[[{}]]}}", values)
    } else {
        "{}".to_string()
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
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

fn read_request(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    let mut data = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
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
    let path = headers
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .to_string();
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
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..read]);
    }
    Some((path, data[body_start..body_start + length].to_vec()))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[test]
fn production_ingest_handles_markdown_and_plain_text_without_partial_writes() {
    let dsn = dsn();
    let corpus = unique_tag();
    let _cleanup = CorpusCleanup {
        dsn: dsn.clone(),
        corpus: corpus.clone(),
    };
    let server = FakeOllama::start();
    let ingest = build_ingest();
    let dir = temp_dir(&corpus);

    let markdown = dir.join("policy.md");
    fs::write(
        &markdown,
        "## Payments\nLate fees apply after 15 days.\n## Refunds\nRefunds take 5 days.",
    )
    .unwrap();
    let text = dir.join("note.txt");
    fs::write(&text, "The verification code is ZX-431.").unwrap();

    let env = RunEnv {
        dsn: &dsn,
        corpus: &corpus,
        source: "live-policy.md",
        url: &server.url,
        embed_dim: EMBEDDING_DIM,
    };
    let first = run_ingest(&ingest, &markdown, &env);
    assert!(
        first.status.success(),
        "Markdown ingest failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        stderr(&first)
    );
    let rows = rows_for(&dsn, &corpus, "live-policy.md");
    assert_eq!(rows.len(), 2, "two Markdown sections must produce two rows");
    assert_eq!(rows[0].section.as_deref(), Some("Payments"));
    assert_eq!(rows[1].section.as_deref(), Some("Refunds"));
    assert_eq!(rows[0].chunk_index, 0);
    assert_eq!(rows[1].chunk_index, 1);
    assert!(
        rows.iter().all(|row| row.page.is_none()),
        "Markdown is page-less"
    );
    assert_eq!(rows[0].language.as_deref(), Some("english"));
    assert_eq!(rows[0].corpus.as_deref(), Some(corpus.as_str()));
    assert!(!rows[0].content_hash.is_empty());

    let env_text = RunEnv {
        source: "live-note.txt",
        ..env
    };
    let text_run = run_ingest(&ingest, &text, &env_text);
    assert!(
        text_run.status.success(),
        "plain-text ingest failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&text_run.stdout),
        stderr(&text_run)
    );
    let text_rows = rows_for(&dsn, &corpus, "live-note.txt");
    assert_eq!(text_rows.len(), 1, "plain text is one section");
    assert_eq!(text_rows[0].section.as_deref(), Some("note.txt"));
    assert!(text_rows[0].page.is_none());
    assert!(text_rows[0].content.contains("ZX-431"));

    // Duplicate: the same bytes must be unchanged, not re-embedded or duplicated.
    let before_duplicate = total_rows(&dsn, &corpus);
    let duplicate = run_ingest(&ingest, &markdown, &env);
    assert!(duplicate.status.success());
    assert!(
        String::from_utf8_lossy(&duplicate.stdout).contains("unchanged"),
        "duplicate ingest must report unchanged; stdout:\n{}",
        String::from_utf8_lossy(&duplicate.stdout)
    );
    assert_eq!(total_rows(&dsn, &corpus), before_duplicate);

    // Empty Markdown is rejected before any effect.
    let empty = dir.join("empty.md");
    fs::write(&empty, "   \n").unwrap();
    let empty_env = RunEnv {
        source: "empty.md",
        ..env
    };
    let empty_run = run_ingest(&ingest, &empty, &empty_env);
    assert!(!empty_run.status.success());
    assert!(
        stderr(&empty_run).contains("no sections found"),
        "{}",
        stderr(&empty_run)
    );
    assert!(rows_for(&dsn, &corpus, "empty.md").is_empty());
    assert_eq!(total_rows(&dsn, &corpus), before_duplicate);

    // Missing and unknown-format sources are rejected with no rows.
    let missing_env = RunEnv {
        source: "missing.md",
        ..env
    };
    let missing = run_ingest(&ingest, &dir.join("missing.md"), &missing_env);
    assert!(!missing.status.success());
    assert!(
        stderr(&missing).contains("failed to read"),
        "{}",
        stderr(&missing)
    );
    assert!(rows_for(&dsn, &corpus, "missing.md").is_empty());

    let bin = dir.join("document.bin");
    fs::write(&bin, "not a supported format").unwrap();
    let bin_env = RunEnv {
        source: "document.bin",
        ..env
    };
    let bin_run = run_ingest(&ingest, &bin, &bin_env);
    assert!(!bin_run.status.success());
    assert!(
        stderr(&bin_run).contains("no loader for"),
        "{}",
        stderr(&bin_run)
    );
    assert!(rows_for(&dsn, &corpus, "document.bin").is_empty());
    assert_eq!(total_rows(&dsn, &corpus), before_duplicate);

    // Embedding failure: an unreachable model endpoint writes no rows.
    let unreachable_env = RunEnv {
        source: "unreachable.md",
        url: "http://127.0.0.1:1",
        ..env
    };
    let unreachable = run_ingest(&ingest, &markdown, &unreachable_env);
    assert!(!unreachable.status.success());
    assert!(rows_for(&dsn, &corpus, "unreachable.md").is_empty());
    assert_eq!(total_rows(&dsn, &corpus), before_duplicate);

    // Dimension failure after a changed source: the previous rows survive.
    let before_policy = rows_for(&dsn, &corpus, "live-policy.md");
    fs::write(
        &markdown,
        "## Payments\nA different late-fee policy.\n## Refunds\nA different refund policy.",
    )
    .unwrap();
    let mismatch_env = RunEnv {
        embed_dim: 2,
        ..env
    };
    let mismatch = run_ingest(&ingest, &markdown, &mismatch_env);
    assert!(!mismatch.status.success());
    let after_policy = rows_for(&dsn, &corpus, "live-policy.md");
    assert_eq!(
        after_policy.len(),
        before_policy.len(),
        "a failed replacement must keep the previous rows"
    );
    assert_eq!(
        after_policy[0].content, before_policy[0].content,
        "a failed replacement must not partially update content"
    );
    assert_eq!(
        after_policy[0].content_hash, before_policy[0].content_hash,
        "a failed replacement must not update the fingerprint"
    );
}
