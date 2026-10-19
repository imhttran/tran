//! RUST-007E live production RAG acceptance.
//!
//! Uses the real Rust `ingest` and `query` binaries with real Ollama embedding
//! and generation and a real PostgreSQL+pgvector database. It ingests a PDF, a
//! Markdown file, and a plain-text file into a unique corpus, answers questions
//! with source citations that are validated against stored rows, and proves the
//! missing-context refusal. No mock generation is used.

use std::fs;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use live_rag_acceptance::repo_root;

const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";
const DEFAULT_CHAT_MODEL: &str = "llama3.2:3b";
const REFUSAL: &str = "I cannot answer this question from the provided documents.";
const EVIDENCE_PATH: &str = "docs/reports/rust-migration/RUST-007E-live-evidence.md";

fn env_value(key: &str) -> Option<String> {
    if let Ok(value) = std::env::var(key) {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    let env_path = repo_root().join(".env");
    let text = fs::read_to_string(env_path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            if name.trim() == key {
                let value = value.trim().trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn dsn() -> String {
    env_value("DATABASE_URL").unwrap_or_else(|| {
        panic!(
            "DATABASE_URL is not set: the live RAG acceptance test requires the dedicated \
             database in the environment or the repository .env; it fails, it does not skip."
        )
    })
}

fn ollama_url() -> String {
    env_value("OLLAMA_URL").unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_string())
}

fn embed_model() -> String {
    env_value("OLLAMA_EMBED_MODEL").unwrap_or_else(|| DEFAULT_EMBED_MODEL.to_string())
}

fn chat_model() -> String {
    env_value("OLLAMA_CHAT_MODEL").unwrap_or_else(|| DEFAULT_CHAT_MODEL.to_string())
}

fn unique_tag() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    format!("rust-007e-live-{}-{nanos}", std::process::id())
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

fn ensure_ollama_reachable(url: &str) {
    let without_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().expect("OLLAMA_URL port")),
        None => (authority, 11434),
    };
    let addresses: Vec<_> = (host, port)
        .to_socket_addrs()
        .expect("resolve OLLAMA_URL")
        .collect();
    let reachable = addresses
        .iter()
        .any(|address| TcpStream::connect_timeout(address, Duration::from_secs(3)).is_ok());
    assert!(
        reachable,
        "Ollama is not reachable at {url}; the live RAG test fails instead of skipping"
    );
}

fn ensure_pgvector(dsn: &str) {
    let mut client = postgres::Client::connect(dsn, postgres::NoTls)
        .expect("connect to the dedicated pgvector database");
    let version: Option<String> = client
        .query_opt(
            "SELECT extversion FROM pg_extension WHERE extname = 'vector'",
            &[],
        )
        .expect("probe pgvector")
        .map(|row| row.get(0));
    assert!(
        version.is_some(),
        "pgvector is unavailable; the live RAG test fails instead of skipping"
    );
}

fn build_binary(name: &str) -> PathBuf {
    let root = repo_root();
    let status = Command::new("cargo")
        .current_dir(&root)
        .args([
            "build",
            "--locked",
            "--manifest-path",
            "Cargo.toml",
            "--bin",
            name,
        ])
        .status()
        .expect("run cargo build");
    assert!(status.success(), "cargo build --bin {name} failed");
    root.join(format!("target/debug/{name}"))
}

struct LiveEnv<'a> {
    dsn: &'a str,
    corpus: &'a str,
    source: &'a str,
    url: &'a str,
    embed_model: &'a str,
    chat_model: &'a str,
}

fn apply_env(command: &mut Command, env: &LiveEnv<'_>) {
    command
        .env("DATABASE_URL", env.dsn)
        .env("RAG_CORPUS", env.corpus)
        .env("RAG_SOURCE", env.source)
        .env("OLLAMA_URL", env.url)
        .env("OLLAMA_EMBED_MODEL", env.embed_model)
        .env("OLLAMA_CHAT_MODEL", env.chat_model)
        .env("EMBED_DIM", "768")
        .env("REQUEST_TIMEOUT", "180s")
        .env("CORPUS_LANGUAGE", "english")
        .env("CHUNK_SIZE", "50")
        .env("CHUNK_OVERLAP", "10")
        .env("MIN_SIMILARITY", "0.3")
        .env("TOP_K", "4")
        .env("FINAL_K", "3")
        .env("EXPAND_LIMIT", "20");
}

fn run_ingest(binary: &Path, input: &Path, env: &LiveEnv<'_>) -> Output {
    let mut command = Command::new(binary);
    command.arg(input).current_dir(repo_root());
    apply_env(&mut command, env);
    command
        .output()
        .expect("spawn the production ingest binary")
}

fn run_query(binary: &Path, question: &str, env: &LiveEnv<'_>) -> Output {
    let mut command = Command::new(binary);
    command.arg(question).current_dir(repo_root());
    apply_env(&mut command, env);
    command.output().expect("spawn the production query binary")
}

fn citations(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("source: "))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn citation_exists(dsn: &str, corpus: &str, citation: &str) -> bool {
    let (without_page, page) = match citation.rsplit_once(" - p.") {
        Some((rest, page)) => (rest, Some(page.parse::<i32>().expect("citation page"))),
        None => (citation, None),
    };
    let Some((source, section)) = without_page.split_once(" - ") else {
        return false;
    };
    let mut client =
        postgres::Client::connect(dsn, postgres::NoTls).expect("connect for citation check");
    let count: i64 = match page {
        Some(page) => client
            .query_one(
                "SELECT count(*) FROM documents \
                 WHERE corpus = $1 AND source = $2 AND section = $3 AND page = $4",
                &[&corpus, &source, &section, &page],
            )
            .expect("check the paged citation")
            .get(0),
        None => client
            .query_one(
                "SELECT count(*) FROM documents \
                 WHERE corpus = $1 AND source = $2 AND section = $3",
                &[&corpus, &source, &section],
            )
            .expect("check the citation")
            .get(0),
    };
    count > 0
}

fn assert_valid_citations(dsn: &str, corpus: &str, output: &str) {
    let found = citations(output);
    assert!(
        !found.is_empty(),
        "the answer must cite at least one source:\n{output}"
    );
    for citation in &found {
        assert!(
            citation_exists(dsn, corpus, citation),
            "citation {citation:?} does not resolve to a stored row in corpus {corpus:?}"
        );
    }
}

/// Runs a query up to `attempts` times, accepting the first success that carries
/// valid citations. Live generation is stochastic, so a bounded retry is
/// recorded and never treated as a mock.
fn answer_with_citations(
    binary: &Path,
    question: &str,
    env: &LiveEnv<'_>,
    attempts: usize,
) -> String {
    let mut last = String::new();
    for attempt in 1..=attempts {
        let output = run_query(binary, question, env);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        last = format!(
            "attempt {attempt}: status={}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        );
        if !output.status.success() {
            continue;
        }
        if stdout.trim() == REFUSAL {
            continue;
        }
        let found = citations(&stdout);
        if found.is_empty() {
            continue;
        }
        if found
            .iter()
            .all(|citation| citation_exists(&env.dsn, env.corpus, citation))
        {
            return stdout;
        }
    }
    panic!("no attempt produced a cited answer:\n{last}");
}

fn count_rows(dsn: &str, corpus: &str) -> i64 {
    let mut client =
        postgres::Client::connect(dsn, postgres::NoTls).expect("connect to count rows");
    client
        .query_one(
            "SELECT count(*) FROM documents WHERE corpus = $1",
            &[&corpus],
        )
        .expect("count the live corpus")
        .get(0)
}

#[test]
fn live_production_ingest_query_and_cited_answer() {
    let dsn = dsn();
    let url = ollama_url();
    let embed = embed_model();
    let chat = chat_model();
    ensure_ollama_reachable(&url);
    ensure_pgvector(&dsn);

    let corpus = unique_tag();
    let _cleanup = CorpusCleanup {
        dsn: dsn.clone(),
        corpus: corpus.clone(),
    };
    let ingest = build_binary("ingest");
    let query = build_binary("query");

    let policy = repo_root().join("examples/loan-policy.md");
    let pdf = repo_root().join("fixtures/parity/inputs/text-layer.pdf");
    let note_dir = std::env::temp_dir().join(&corpus);
    fs::create_dir_all(&note_dir).expect("create the live note directory");
    let note = note_dir.join("rust-007e-note.txt");
    fs::write(
        &note,
        "The RUST-007E support code is ZX-431. Escalate it to the migration operator.",
    )
    .expect("write the live note");

    let base = LiveEnv {
        dsn: &dsn,
        corpus: &corpus,
        source: "live-policy.md",
        url: &url,
        embed_model: &embed,
        chat_model: &chat,
    };

    for (input, source) in [
        (&policy, "live-policy.md"),
        (&pdf, "live-text-layer.pdf"),
        (&note, "live-note.txt"),
    ] {
        let env = LiveEnv { source, ..base };
        let output = run_ingest(&ingest, input, &env);
        assert!(
            output.status.success(),
            "live ingest of {source} failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        count_rows(&dsn, &corpus) >= 4,
        "the live corpus must contain the ingested documents"
    );

    let policy_answer = answer_with_citations(
        &query,
        "What happens if someone misses a payment?",
        &base,
        3,
    );
    assert_valid_citations(&dsn, &corpus, &policy_answer);
    assert!(
        policy_answer.contains("live-policy.md - "),
        "the loan-policy answer must cite the Markdown source:\n{policy_answer}"
    );

    let pdf_answer = answer_with_citations(
        &query,
        "What text appears on page two of the PDF document?",
        &base,
        3,
    );
    assert_valid_citations(&dsn, &corpus, &pdf_answer);
    assert!(
        pdf_answer.contains("live-text-layer.pdf - Page 2 - p.2"),
        "the PDF answer must cite page 2 with page provenance:\n{pdf_answer}"
    );

    let note_answer =
        answer_with_citations(&query, "What is the RUST-007E support code?", &base, 3);
    assert_valid_citations(&dsn, &corpus, &note_answer);
    assert!(
        note_answer.contains("ZX-431") || note_answer.contains("live-note.txt - "),
        "the plain-text answer must use the note fact or cite the note:\n{note_answer}"
    );

    // Missing context: an empty corpus must refuse without generation and
    // without citations.
    let empty_corpus = format!("{corpus}-empty");
    let empty_env = LiveEnv {
        corpus: &empty_corpus,
        ..base
    };
    let refusal = run_query(&query, "What is the RUST-007E support code?", &empty_env);
    let stdout = String::from_utf8_lossy(&refusal.stdout);
    assert!(
        refusal.status.success(),
        "an empty corpus must return the baseline refusal, not fail:\n{stdout}\n{}",
        String::from_utf8_lossy(&refusal.stderr)
    );
    assert_eq!(stdout.trim(), REFUSAL, "the empty corpus must refuse");
    assert!(
        citations(&stdout).is_empty(),
        "a refusal must not carry citations:\n{stdout}"
    );

    // Leave a concise record for the evidence document.
    let mut record = fs::File::create(std::env::temp_dir().join(format!("{corpus}.live.txt")))
        .expect("create the live record");
    writeln!(record, "corpus={corpus}").unwrap();
    writeln!(record, "ollama={url} embed={embed} chat={chat}").unwrap();
    writeln!(record, "policy_answer:\n{policy_answer}").unwrap();
    writeln!(record, "pdf_answer:\n{pdf_answer}").unwrap();
    writeln!(record, "note_answer:\n{note_answer}").unwrap();
    writeln!(record, "refusal={}", stdout.trim()).unwrap();
}

#[test]
fn live_evidence_is_recorded() {
    let path = repo_root().join(EVIDENCE_PATH);
    let text = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "missing live evidence {}: {err}; record the live run before the final acceptance",
            path.display()
        )
    });
    for required in [
        "## Run identity",
        "## Configuration",
        "## Commands",
        "## Results",
        "## Citations",
        "## Refusal",
        "## Observations",
        "nomic-embed-text",
        "llama3.2:3b",
        "ingest",
        "query",
    ] {
        assert!(
            text.contains(required),
            "live evidence is missing {required:?} in {}",
            path.display()
        );
    }
}
