//! RUST-007D bilingual experiment parity.
//!
//! Runs the production Rust `ingest`/`expbilingual` and compares against the frozen Go
//! `expbilingual` against the same PostgreSQL corpus, dataset, configuration,
//! deterministic embeddings, and deterministic generated queries, then compares
//! the normalized output and the written query cache.
//!
//! Two rounds run per command:
//!
//! 1. `ARM=all` (A, D1, D2) from an empty cache, so every variant is generated
//!    (`GEN ... source=model`) and the cache is written from scratch.
//! 2. `ARM=B,C,E` starting from the round-1 cache, so every variant is a cache
//!    hit (`GEN ... source=cache`, `generated_queries=0`) and the remaining arms
//!    are covered.
//!
//! Tolerances (both commands print fixed decimals): `r1`, `r4` and `evidence`
//! within 0.005 (one rounding unit at two decimals) and DIAG `sim` within
//! 0.00005 (one rounding unit at four decimals). Everything else — line kinds
//! and order, HEADER, GEN, DIAG summary lines, candidate order and
//! source/section, arm, rep, id, kind, rejection labels, `generated_queries`
//! and the cache path — must be identical; the FOOTER `wall` time is ignored.
//! The two cache files must be identical byte for byte.
//!
//! The Go harness searches unfiltered while the Rust command is scoped by
//! `RAG_CORPUS`, so the test asserts the database holds no rows outside the
//! run corpus before it starts, and deletes only the run corpus afterwards.

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

const METRIC_TOLERANCE: f64 = 0.005;
const SIMILARITY_TOLERANCE: f64 = 0.00005;
const DATASET: &str = "evals/retrieval-vi-en-expanded.json";
/// Chosen so the stub embeddings (cosine ~0.2-0.7 against the corpus) both
/// clear and miss the floor, exercising the D1/D2/E guards in both directions.
const MIN_SIMILARITY: &str = "0.3";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives three levels below the repository root")
        .to_path_buf()
}

fn dsn() -> String {
    match std::env::var("DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => panic!(
            "DATABASE_URL is not set: the bilingual parity acceptance test requires the dedicated \
             database. Load the repository .env; this test fails, it does not skip."
        ),
    }
}

fn unique_tag() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    format!("bilingual-parity-{}-{nanos}", std::process::id())
}

/// Deletes only the run-owned corpus.
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

/// Removes the per-run scratch directory (binaries and cache files).
struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn base_env(command: &mut Command, dsn: &str, ollama_url: &str) {
    command
        .current_dir(repo_root())
        .env_remove("RAG_CORPUS")
        .env_remove("LANGUAGE")
        .env("DATABASE_URL", dsn)
        .env("OLLAMA_URL", ollama_url)
        .env("OLLAMA_EMBED_MODEL", "parity-embed")
        .env("OLLAMA_CHAT_MODEL", "parity-chat")
        .env("EMBED_DIM", EMBEDDING_DIM.to_string())
        .env("REQUEST_TIMEOUT", "30s")
        .env("MIN_SIMILARITY", MIN_SIMILARITY);
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

fn build_rust_expbilingual() -> PathBuf {
    let root = repo_root();
    let status = Command::new("cargo")
        .current_dir(&root)
        .args([
            "build",
            "--locked",
            "--manifest-path",
            "Cargo.toml",
            "--bin",
            "expbilingual",
        ])
        .status()
        .expect("run cargo build for expbilingual");
    assert!(status.success(), "cargo build --bin expbilingual failed");
    let binary = root.join("target/debug/expbilingual");
    assert!(binary.is_file(), "missing Rust binary {}", binary.display());
    binary
}

fn run_rust_ingest(binary: &Path, source: &str, dsn: &str, corpus: &str, url: &str) {
    let mut command = Command::new(binary);
    base_env(&mut command, dsn, url);
    command
        .env("RAG_CORPUS", corpus)
        .env("RAG_SOURCE", source)
        .arg(source);
    let output = command.output().expect("run the production Rust ingest");
    assert!(
        output.status.success(),
        "Rust ingest {source} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Run<'a> {
    dsn: &'a str,
    url: &'a str,
    arm: &'a str,
    cache: &'a Path,
    corpus: Option<&'a str>,
}

fn run_harness(binary: &Path, run: &Run<'_>) -> String {
    let mut command = Command::new(binary);
    base_env(&mut command, run.dsn, run.url);
    command
        .env("ARM", run.arm)
        .env("REPEATS", "1")
        .env("DIAG", "1")
        .env("EVAL_DATASET", DATASET)
        .env("GEN_TIMEOUT", "30")
        .env("QUERY_CACHE", run.cache);
    // RAG_CORPUS is supplied to the Rust command only.
    if let Some(corpus) = run.corpus {
        command.env("RAG_CORPUS", corpus);
    }
    let output: Output = command.output().expect("run expbilingual");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "{} failed:\nstdout:\n{stdout}\nstderr:\n{}",
        binary.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

// ---------------------------------------------------------------------------
// Output normalization
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Line {
    /// HEADER, GEN, DIAG summary and CACHE lines compare exactly.
    Exact(String),
    /// `DIAG   #n sim=X source / section`.
    Candidate {
        rank: String,
        similarity: f64,
        section: String,
    },
    /// `CASE ...`: exact fields plus the three metrics.
    Case {
        fields: Vec<String>,
        r1: f64,
        r4: f64,
        evidence: f64,
    },
    /// `FOOTER ...` without the wall time.
    Footer(Vec<String>),
}

fn parse(output: &str) -> Vec<Line> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            if let Some(rest) = line.strip_prefix("DIAG   #") {
                let (rank, rest) = rest.split_once(" sim=").expect("DIAG candidate sim");
                let (similarity, section) = rest.split_once(' ').expect("DIAG candidate section");
                Line::Candidate {
                    rank: rank.to_string(),
                    similarity: similarity.parse().expect("DIAG sim value"),
                    section: section.to_string(),
                }
            } else if line.starts_with("CASE ") {
                let mut fields = Vec::new();
                let (mut r1, mut r4, mut evidence) = (None, None, None);
                for token in line.split_whitespace() {
                    match token.split_once('=') {
                        Some(("r1", v)) => r1 = Some(v.parse().expect("r1")),
                        Some(("r4", v)) => r4 = Some(v.parse().expect("r4")),
                        Some(("evidence", v)) => evidence = Some(v.parse().expect("evidence")),
                        _ => fields.push(token.to_string()),
                    }
                }
                Line::Case {
                    fields,
                    r1: r1.expect("CASE r1"),
                    r4: r4.expect("CASE r4"),
                    evidence: evidence.expect("CASE evidence"),
                }
            } else if line.starts_with("FOOTER ") {
                Line::Footer(
                    line.split_whitespace()
                        .filter(|token| !token.starts_with("wall="))
                        .map(|token| {
                            if token.starts_with("cache=") {
                                "cache=<path>".to_string()
                            } else {
                                token.to_string()
                            }
                        })
                        .collect(),
                )
            } else {
                Line::Exact(line.to_string())
            }
        })
        .collect()
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance + 1e-9
}

fn assert_parity(round: &str, go: &str, rust: &str) {
    let go_lines = parse(go);
    let rust_lines = parse(rust);
    let context = || format!("\n--- Go ---\n{go}\n--- Rust ---\n{rust}");
    for (index, (g, r)) in go_lines.iter().zip(rust_lines.iter()).enumerate() {
        let same = match (g, r) {
            (
                Line::Candidate {
                    rank: gr,
                    similarity: gs,
                    section: gsec,
                },
                Line::Candidate {
                    rank: rr,
                    similarity: rs,
                    section: rsec,
                },
            ) => gr == rr && gsec == rsec && close(*gs, *rs, SIMILARITY_TOLERANCE),
            (
                Line::Case {
                    fields: gf,
                    r1: g1,
                    r4: g4,
                    evidence: ge,
                },
                Line::Case {
                    fields: rf,
                    r1: r1v,
                    r4: r4v,
                    evidence: re,
                },
            ) => {
                gf == rf
                    && close(*g1, *r1v, METRIC_TOLERANCE)
                    && close(*g4, *r4v, METRIC_TOLERANCE)
                    && close(*ge, *re, METRIC_TOLERANCE)
            }
            (g, r) => g == r,
        };
        assert!(
            same,
            "{round}: line {index} differs beyond the documented tolerance:\n  Go:   {g:?}\n  Rust: {r:?}{}",
            context()
        );
    }
    assert_eq!(
        go_lines.len(),
        rust_lines.len(),
        "{round}: line count differs{}",
        context()
    );
}

/// `(arm, label)` for every CASE line.
fn case_labels(output: &str) -> Vec<(String, String)> {
    parse(output)
        .into_iter()
        .filter_map(|line| match line {
            Line::Case { fields, .. } => {
                let arm = fields
                    .iter()
                    .find_map(|f| f.strip_prefix("arm="))
                    .expect("CASE arm")
                    .to_string();
                Some((arm, fields.last().expect("CASE label").clone()))
            }
            _ => None,
        })
        .collect()
}

fn footer(output: &str) -> Vec<String> {
    parse(output)
        .into_iter()
        .find_map(|line| match line {
            Line::Footer(fields) => Some(fields),
            _ => None,
        })
        .expect("FOOTER line")
}

// ---------------------------------------------------------------------------
// Deterministic Ollama stub
// ---------------------------------------------------------------------------

/// A scripted Ollama server: `/api/embed` returns an input-derived normalized
/// vector and `/api/generate` a prompt-derived response, so both commands see
/// identical embeddings and identical generated queries.
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
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        serve(&mut stream);
                    }
                    Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
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
    let request = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let field = |name: &str| {
        request
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let response = if path == "/api/embed" {
        format!("{{\"embeddings\":[[{}]]}}", vector_for(&field("input")))
    } else if path == "/api/generate" {
        serde_json::json!({ "response": generated_query(&field("prompt")) }).to_string()
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

/// A deterministic "generated" query: the question from the prompt, decorated
/// per prompt kind. Padding exercises the harness trim; `&` and `<>` exercise
/// the Go HTML escaping of the cache file.
fn generated_query(prompt: &str) -> String {
    let question = prompt
        .split_once("QUESTION:\n")
        .and_then(|(_, rest)| rest.split_once("\n\n"))
        .map(|(question, _)| question)
        .unwrap_or_default();
    if prompt.starts_with("Rewrite") {
        format!("{question} & <chính sách> policy")
    } else if prompt.contains("into Vietnamese") {
        format!("  {question} tiếng Việt  ")
    } else {
        question.to_string()
    }
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

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

fn assert_test_database_is_isolated(dsn: &str, corpus: &str) {
    let mut client = postgres::Client::connect(dsn, postgres::NoTls)
        .expect("connect to check test-database isolation");
    let foreign: i64 = client
        .query_one(
            "SELECT count(*) FROM documents WHERE corpus IS DISTINCT FROM $1",
            &[&corpus],
        )
        .expect("count foreign corpus rows")
        .get(0);
    assert_eq!(
        foreign, 0,
        "the bilingual parity test requires an isolated database: Go cmd/expbilingual searches \
         unfiltered. Found {foreign} rows outside the run corpus {corpus:?}. Use a dedicated test \
         database; this test does not delete other rows."
    );
}

fn dataset() -> Vec<Value> {
    let raw = fs::read_to_string(repo_root().join(DATASET)).expect("read the bilingual dataset");
    serde_json::from_str(&raw).expect("parse the bilingual dataset")
}

fn dataset_sources(cases: &[Value]) -> Vec<String> {
    let mut sources = BTreeSet::new();
    for case in cases {
        for doc in case
            .get("expected")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(source) = doc.get("source").and_then(Value::as_str) {
                sources.insert(source.to_string());
            }
        }
    }
    sources.into_iter().collect()
}

#[test]
fn go_and_rust_expbilingual_agree_on_the_real_corpus() {
    let dsn = dsn();
    let corpus = unique_tag();
    let _cleanup = CorpusCleanup {
        dsn: dsn.clone(),
        corpus: corpus.clone(),
    };
    assert_test_database_is_isolated(&dsn, &corpus);

    let scratch = std::env::temp_dir().join(&corpus);
    fs::create_dir_all(&scratch).expect("create scratch dir");
    let _scratch = ScratchDir(scratch.clone());
    let server = FakeOllama::start();

    let cases = dataset();
    let sources = dataset_sources(&cases);
    assert_eq!(cases.len(), 39, "the frozen dataset has 39 cases");
    assert_eq!(sources.len(), 8, "the dataset references eight sources");
    assert!(
        sources.iter().any(|source| source.contains("vi")),
        "the Vietnamese sources must be ingested: {sources:?}"
    );

    let rust_ingest = build_rust_ingest();
    for source in &sources {
        run_rust_ingest(&rust_ingest, source, &dsn, &corpus, &server.url);
    }
    assert_test_database_is_isolated(&dsn, &corpus);

    let rust_harness = build_rust_expbilingual();
    let frozen = repo_root().join("fixtures/parity/frozen");
    let go_all = fs::read_to_string(frozen.join("bilingual_go_all_stdout.txt"))
        .expect("read the frozen Go bilingual all output");
    let go_cache = fs::read(frozen.join("bilingual_go_all_cache.json"))
        .expect("read the frozen Go bilingual all cache");

    // The cache path is identical for both commands (it is printed in the
    // FOOTER); each command starts from the same cache state.
    let cache = scratch.join("query-cache.json");
    let run = |binary: &Path, arm: &str, rust: bool| {
        run_harness(
            binary,
            &Run {
                dsn: &dsn,
                url: &server.url,
                arm,
                cache: &cache,
                corpus: rust.then_some(corpus.as_str()),
            },
        )
    };

    // Round 1: ARM=all from an empty cache, compared with the frozen Go output.
    if cache.exists() {
        fs::remove_file(&cache).expect("reset the cache for the Rust run");
    }
    let rust_all = run(&rust_harness, "all", true);
    let rust_cache = fs::read(&cache).expect("Rust wrote the query cache");

    assert_parity("ARM=all", &go_all, &rust_all);
    assert_eq!(
        String::from_utf8_lossy(&go_cache),
        String::from_utf8_lossy(&rust_cache),
        "the written query caches differ"
    );
    assert_eq!(
        go_cache, rust_cache,
        "the written query caches differ byte-wise"
    );

    let labels = case_labels(&rust_all);
    for arm in ["A", "D1", "D2"] {
        assert_eq!(
            labels.iter().filter(|(a, _)| a == arm).count(),
            39,
            "arm {arm} must cover all 39 cases"
        );
    }
    for label in ["n/a", "reject", "FALSE_POSITIVE"] {
        assert!(
            labels.iter().any(|(_, l)| l == label),
            "rejection label {label} must be exercised"
        );
    }
    for guard in ["guardOK=true", "guardOK=false"] {
        assert!(
            rust_all.contains(guard),
            "the D1/D2 guard must be exercised with {guard}"
        );
    }
    assert!(
        rust_all.contains(" r4=1.00 "),
        "some case must retrieve its expected section"
    );
    assert_eq!(
        footer(&rust_all),
        vec![
            "FOOTER".to_string(),
            "generated_queries=117".to_string(),
            format!("cache=<path>"),
        ],
        "every case generates exp/vi/en once, then D2 reads them from the cache"
    );

    // Round 2: B, C, E from the round-1 cache (all cache hits), compared with
    // the frozen Go output and cache.
    let go_rest = fs::read_to_string(frozen.join("bilingual_go_rest_stdout.txt"))
        .expect("read the frozen Go bilingual rest output");
    let go_cache_2 = fs::read(frozen.join("bilingual_go_rest_cache.json"))
        .expect("read the frozen Go bilingual rest cache");
    fs::write(&cache, &go_cache).expect("seed the cache for Rust");
    let rust_rest = run(&rust_harness, "B,C,E", true);
    let rust_cache_2 = fs::read(&cache).expect("Rust rewrote the query cache");

    assert_parity("ARM=B,C,E", &go_rest, &rust_rest);
    assert_eq!(
        go_cache_2, rust_cache_2,
        "the rewritten query caches differ"
    );
    assert!(
        !rust_rest.contains("source=model"),
        "round 2 must be served from the cache"
    );
    let labels = case_labels(&rust_rest);
    for arm in ["B", "C", "E"] {
        assert_eq!(
            labels.iter().filter(|(a, _)| a == arm).count(),
            39,
            "arm {arm} must cover all 39 cases"
        );
    }
    assert!(footer(&rust_rest).contains(&"generated_queries=0".to_string()));
}
