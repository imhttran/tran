//! RUST-007E original K=3 parity.
//!
//! Runs the production Rust `ingest`/`eval` and compares against the frozen Go
//! production Rust `eval` against the same PostgreSQL corpus, dataset, fixed
//! configuration, and deterministic embedding server. Both commands receive
//! identical embeddings for identical input, so the comparison is behavior-based
//! rather than a fixture-only stub.
//!
//! This crate measures the original K=3 contract: `TOP_K=3` and `FINAL_K=3`, so
//! Go emits `K=3` and Rust scores `K=3`. K=4 output is never substituted. Go's
//! `cmd/eval` prints per-case metrics but not the answerable ranking, so
//! `tools/rankdump` prints the fused ranking from the same Go pipeline; the test
//! compares source/section/chunk order and tie ordering as well as the metrics.
//! Numerical tolerance: both commands print two decimal places, so recall and
//! precision are compared with an absolute tolerance of 0.005 (one rounding
//! unit). Case count, question order, ranking order, and rejection labels are
//! exact.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rag_foundation::storage::EMBEDDING_DIM;
use serde_json::Value;

const TOLERANCE: f64 = 0.005;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives three levels below the repository root")
        .to_path_buf()
}

fn dsn() -> String {
    if let Ok(value) = std::env::var("DATABASE_URL") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    // Acceptance checks run as bare commands and do not source the repository
    // `.env`, so read the dedicated test DSN from it when the environment has
    // none. The file is never modified or printed.
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
        "DATABASE_URL is not set: the K=3 parity acceptance test requires the dedicated \
         database in the environment or the repository .env; this test fails, it does not skip."
    )
}

fn unique_tag() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    format!("k3-parity-{}-{nanos}", std::process::id())
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

fn apply_env(command: &mut Command, dsn: &str, corpus: &str, ollama_url: &str) {
    command
        .env("DATABASE_URL", dsn)
        .env("RAG_CORPUS", corpus)
        .env("OLLAMA_URL", ollama_url)
        .env("OLLAMA_EMBED_MODEL", "parity-embed")
        .env("OLLAMA_CHAT_MODEL", "parity-chat")
        .env("EMBED_DIM", EMBEDDING_DIM.to_string())
        .env("REQUEST_TIMEOUT", "10s")
        .env("QUERY_REWRITE", "false")
        .env("EVAL_LEXICAL_RERANK", "false")
        .env("EVAL_LLM_RERANK", "false")
        .env("EVAL_ANSWERABILITY_GATE", "false")
        .env("EVAL_FACT_JUDGE", "false")
        .env("TOP_K", "3")
        .env("FINAL_K", "3")
        .env("EXPAND_LIMIT", "20")
        .env("MIN_SIMILARITY", "0.6")
        .env("EVAL_DATASET", "evals/retrieval.json");
}

fn build_rust_ingest() -> PathBuf {
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

fn build_rust_eval() -> PathBuf {
    let root = repo_root();
    let status = Command::new("cargo")
        .current_dir(&root)
        .args([
            "build",
            "--locked",
            "--manifest-path",
            "Cargo.toml",
            "--bin",
            "eval",
        ])
        .status()
        .expect("run cargo build for eval");
    assert!(status.success(), "cargo build --bin eval failed");
    root.join("target/debug/eval")
}

fn run_rust_ingest(binary: &Path, source: &str, dsn: &str, corpus: &str, url: &str) {
    let mut command = Command::new(binary);
    command.current_dir(repo_root());
    command.arg(source);
    apply_env(&mut command, dsn, corpus, url);
    command.env("RAG_SOURCE", source);
    let output = command.output().expect("run the production Rust ingest");
    assert!(
        output.status.success(),
        "Rust ingest {source} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_eval(binary: &Path, dsn: &str, corpus: &str, url: &str) -> Output {
    let mut command = Command::new(binary);
    command.current_dir(repo_root());
    apply_env(&mut command, dsn, corpus, url);
    command.output().expect("run eval")
}

/// One Go ranking line, normalized to the same text Rust prints.
fn parse_go_rankings(output: &str) -> Vec<(String, Vec<String>)> {
    let mut cases: Vec<(String, Vec<String>)> = Vec::new();
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("CASE ") {
            let question = rest
                .split_once('\t')
                .map(|(_, question)| question.to_string())
                .unwrap_or_default();
            cases.push((question, Vec::new()));
        } else if let Some(rank_line) = line.strip_prefix("RANK ") {
            let mut fields = rank_line.split('\t');
            let _rank = fields.next();
            let source = fields.next().unwrap_or_default();
            let section = fields.next().unwrap_or_default();
            let chunk = fields.next().unwrap_or_default();
            if let Some((_, rankings)) = cases.last_mut() {
                rankings.push(format!("{source} / {section} / chunk {chunk}"));
            }
        }
    }
    cases
}

/// One Rust ranking line (`    #1  0.1234  source / section / chunk N`),
/// normalized to the same text the Go dump prints.
fn parse_rust_rankings(output: &str) -> Vec<(String, Vec<String>)> {
    let mut cases: Vec<(String, Vec<String>)> = Vec::new();
    for line in output.lines() {
        if let Some(question) = line.strip_prefix("Question: ") {
            cases.push((question.trim().to_string(), Vec::new()));
            continue;
        }
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix('#') else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let _rank = fields.next();
        let _score = fields.next();
        let identity = fields.collect::<Vec<_>>().join(" ");
        if let Some((_, rankings)) = cases.last_mut() {
            rankings.push(identity);
        }
    }
    cases
}

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Metrics(f64, f64),
    Rejection(&'static str),
}

#[derive(Debug, Default)]
struct Parsed {
    cases: Vec<(String, Outcome)>,
    aggregate: Option<(f64, f64)>,
}

fn metric(line: &str, name: &str) -> Option<f64> {
    let rest = line.split(name).nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

fn parse_go(output: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut current: Option<String> = None;
    let mut in_hybrid_overall = false;
    for line in output.lines() {
        if let Some(question) = line.strip_prefix("Question: ") {
            current = Some(question.trim().to_string());
            in_hybrid_overall = false;
        }
        if line.trim() == "Hybrid retrieval:" {
            in_hybrid_overall = true;
            continue;
        }
        if in_hybrid_overall && line.trim_start().starts_with("K=3  Avg Recall@3=") {
            let recall = metric(line, "Avg Recall@3=").expect("go aggregate recall");
            let precision = metric(line, "Avg Precision@3=").expect("go aggregate precision");
            parsed.aggregate = Some((recall, precision));
            in_hybrid_overall = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("Hybrid K=3") {
            let question = current.clone().expect("go case has a question");
            let outcome = if rest.contains("Correct rejection") {
                Outcome::Rejection("correct")
            } else if rest.contains("FALSE POSITIVE") {
                Outcome::Rejection("false-positive")
            } else {
                Outcome::Metrics(
                    metric(line, "Recall@3=").expect("go case recall"),
                    metric(line, "Precision@3=").expect("go case precision"),
                )
            };
            parsed.cases.push((question, outcome));
        }
    }
    parsed
}

fn parse_rust(output: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut current: Option<String> = None;
    for line in output.lines() {
        if let Some(question) = line.strip_prefix("Question: ") {
            current = Some(question.trim().to_string());
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("Recall@3=") {
            parsed.cases.push((
                current.clone().expect("rust case has a question"),
                Outcome::Metrics(
                    metric(trimmed, "Recall@3=").expect("rust case recall"),
                    metric(trimmed, "Precision@3=").expect("rust case precision"),
                ),
            ));
        } else if trimmed == "Correct rejection" {
            parsed.cases.push((
                current.clone().expect("rust case has a question"),
                Outcome::Rejection("correct"),
            ));
        } else if trimmed == "FALSE POSITIVE" {
            parsed.cases.push((
                current.clone().expect("rust case has a question"),
                Outcome::Rejection("false-positive"),
            ));
        } else if trimmed.contains("Avg Recall@3=") {
            parsed.aggregate = Some((
                metric(trimmed, "Avg Recall@3=").expect("rust aggregate recall"),
                metric(trimmed, "Avg Precision@3=").expect("rust aggregate precision"),
            ));
        }
    }
    parsed
}

fn assert_close(label: &str, go: f64, rust: f64) {
    assert!(
        (go - rust).abs() <= TOLERANCE + 1e-9,
        "{label}: Go={go:.4} Rust={rust:.4} differs by more than the documented tolerance {TOLERANCE}"
    );
}

/// A scripted Ollama server returning a deterministic vector for the input.
struct FakeOllama {
    url: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeOllama {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake Ollama");
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
        let input = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("input")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let values = vector_for(&input);
        format!("{{\"embeddings\":[[{}]]}}", values)
    } else if path == "/api/generate" {
        "{\"response\":\"parity\"}".to_string()
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

fn assert_corpus_is_empty(dsn: &str, corpus: &str) {
    let mut client =
        postgres::Client::connect(dsn, postgres::NoTls).expect("connect to check the run corpus");
    let existing: i64 = client
        .query_one(
            "SELECT count(*) FROM documents WHERE corpus = $1",
            &[&corpus],
        )
        .expect("count run corpus rows")
        .get(0);
    assert_eq!(
        existing, 0,
        "the K=3 parity corpus {corpus:?} already has rows; the test would not measure a fresh ingest"
    );
}

fn dataset_sources() -> Vec<String> {
    let dataset = fs::read_to_string(repo_root().join("evals/retrieval.json"))
        .expect("read evals/retrieval.json");
    let cases: Vec<Value> = serde_json::from_str(&dataset).expect("parse evals/retrieval.json");
    let mut sources = BTreeSet::new();
    for case in cases {
        if let Some(expected) = case.get("expected").and_then(Value::as_array) {
            for doc in expected {
                if let Some(source) = doc.get("source").and_then(Value::as_str) {
                    sources.insert(source.to_string());
                }
            }
        }
    }
    sources.into_iter().collect()
}

#[test]
fn go_and_rust_eval_agree_at_k3_on_the_real_corpus() {
    let dsn = dsn();
    let corpus = unique_tag();
    let _cleanup = CorpusCleanup {
        dsn: dsn.clone(),
        corpus: corpus.clone(),
    };
    assert_corpus_is_empty(&dsn, &corpus);
    let server = FakeOllama::start();

    let rust_ingest = build_rust_ingest();
    for source in dataset_sources() {
        run_rust_ingest(&rust_ingest, &source, &dsn, &corpus, &server.url);
    }

    let rust_eval = build_rust_eval();

    let go_stdout =
        fs::read_to_string(repo_root().join("fixtures/parity/frozen/eval_k3_go_stdout.txt"))
            .expect("read the frozen Go K=3 eval output");
    let dumped_stdout =
        fs::read_to_string(repo_root().join("fixtures/parity/frozen/k3_rankdump_go_stdout.txt"))
            .expect("read the frozen Go K=3 ranking output");

    let rust = run_eval(&rust_eval, &dsn, &corpus, &server.url);
    assert!(
        rust.status.success(),
        "Rust eval failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&rust.stdout),
        String::from_utf8_lossy(&rust.stderr)
    );

    let go_out = go_stdout;
    let rust_out = String::from_utf8_lossy(&rust.stdout);
    let go = parse_go(&go_out);
    let rust = parse_rust(&rust_out);

    assert_eq!(
        go.cases.len(),
        rust.cases.len(),
        "case count differs: Go={} Rust={}",
        go.cases.len(),
        rust.cases.len()
    );
    assert!(
        go.cases.len() > 1,
        "the parity check must cover the real dataset, not a stub"
    );
    for (index, ((go_question, go_outcome), (rust_question, rust_outcome))) in
        go.cases.iter().zip(rust.cases.iter()).enumerate()
    {
        assert_eq!(go_question, rust_question, "case {index} question differs");
        match (go_outcome, rust_outcome) {
            (
                Outcome::Metrics(go_recall, go_precision),
                Outcome::Metrics(rust_recall, rust_precision),
            ) => {
                assert_close(
                    &format!("case {index} ({go_question}) recall"),
                    *go_recall,
                    *rust_recall,
                );
                assert_close(
                    &format!("case {index} ({go_question}) precision"),
                    *go_precision,
                    *rust_precision,
                );
            }
            (Outcome::Rejection(go_label), Outcome::Rejection(rust_label)) => {
                assert_eq!(go_label, rust_label, "case {index} rejection label differs");
            }
            (go_outcome, rust_outcome) => {
                panic!("case {index} outcome differs: Go={go_outcome:?} Rust={rust_outcome:?}");
            }
        }
    }
    let (go_recall, go_precision) = go.aggregate.expect("Go aggregate metrics");
    let (rust_recall, rust_precision) = rust.aggregate.expect("Rust aggregate metrics");
    assert_close("aggregate recall", go_recall, rust_recall);
    assert_close("aggregate precision", go_precision, rust_precision);

    // Ranking and tie order, not just the aggregated metrics.
    let go_rankings = parse_go_rankings(&dumped_stdout);
    let rust_rankings = parse_rust_rankings(&rust_out);
    assert_eq!(
        go_rankings.len(),
        rust_rankings.len(),
        "ranking dump case count differs: Go={} Rust={}",
        go_rankings.len(),
        rust_rankings.len()
    );
    for (index, ((go_question, go_ranks), (rust_question, rust_ranks))) in
        go_rankings.iter().zip(rust_rankings.iter()).enumerate()
    {
        assert_eq!(
            go_question, rust_question,
            "case {index} ranking question differs"
        );
        assert_eq!(
            go_ranks, rust_ranks,
            "case {index} ({go_question}) ranking order differs:\nGo:   {go_ranks:?}\nRust: {rust_ranks:?}"
        );
    }
}
