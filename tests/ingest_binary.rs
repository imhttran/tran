//! RUST-005F production ingest binary wiring test.
//!
//! Drives the real `ingest` binary (via `CARGO_BIN_EXE_ingest`) against the
//! dedicated PostgreSQL/pgvector database. It proves the production caller
//! rejects incomplete and fully scanned PDFs and fatal loader errors before any
//! embedding call or database write, and that already stored rows survive.
//!
//! The test fails clearly when `DATABASE_URL` or pgvector is unavailable; it
//! never skips to a pass.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use rag_foundation::storage::{DocumentStore, StoredChunk, EMBEDDING_DIM};
use rag_foundation::storage_pg::PgStore;

fn dsn() -> String {
    match std::env::var("DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => panic!(
            "DATABASE_URL is not set: the production ingest binary acceptance test \
             requires the dedicated database. Load the repository .env; this test fails, it does not skip."
        ),
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/parity/inputs")
        .join(name)
}

fn unique_tag() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn count_source(dsn: &str, source: &str) -> i64 {
    let mut client =
        postgres::Client::connect(dsn, postgres::NoTls).expect("connect to the migration database");
    client
        .query_one(
            "SELECT count(*) FROM documents WHERE source = $1",
            &[&source],
        )
        .expect("count the test source")
        .get(0)
}

fn seed_row(dsn: &str, source: &str, hash: &str) {
    let store = PgStore::connect(dsn).expect("open the Rust store");
    let mut embedding = vec![0.0_f64; EMBEDDING_DIM];
    embedding[0] = 0.5;
    store
        .write_chunk(&StoredChunk {
            content: "pre-existing row that a rejected ingest must preserve".to_string(),
            source: Some(source.to_string()),
            section: Some("Existing".to_string()),
            chunk_index: 0,
            language: Some("english".to_string()),
            page: Some(1),
            corpus: Some("interop".to_string()),
            content_hash: hash.to_string(),
            embed_model: "test-model".to_string(),
            embedding_dim: EMBEDDING_DIM as i32,
            chunker_config: "{}".to_string(),
            ingested_at: "2026-10-17T00:00:00Z".to_string(),
            embedding,
        })
        .expect("seed the pre-existing row");
}

fn cleanup(dsn: &str, source: &str) {
    let mut client = postgres::Client::connect(dsn, postgres::NoTls).expect("connect for cleanup");
    client
        .execute("DELETE FROM documents WHERE source = $1", &[&source])
        .expect("delete only this test's rows");
}

fn run_binary(input: &Path, source: &str, dsn: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ingest"))
        .arg(input)
        .env("RAG_SOURCE", source)
        .env("DATABASE_URL", dsn)
        .output()
        .expect("spawn the production ingest binary")
}

#[test]
fn production_binary_rejects_and_preserves_existing_rows() {
    let dsn = dsn();
    let tag = unique_tag();
    let source = format!("interop://rust/bin/{tag}");
    let hash = format!("bin-{tag}");

    // pgvector must be present; the storage layer depends on it.
    let mut client =
        postgres::Client::connect(&dsn, postgres::NoTls).expect("connect to probe pgvector");
    let version: Option<String> = client
        .query_opt(
            "SELECT extversion FROM pg_extension WHERE extname = 'vector'",
            &[],
        )
        .expect("probe pgvector")
        .map(|row| row.get(0));
    assert!(
        version.is_some(),
        "pgvector is unavailable; the binary acceptance test fails instead of skipping"
    );

    seed_row(&dsn, &source, &hash);
    let before = count_source(&dsn, &source);
    assert_eq!(before, 1, "the preserved row must exist before the run");

    let mixed = run_binary(&fixture("mixed-text-scan.pdf"), &source, &dsn);
    assert!(
        !mixed.status.success(),
        "an incomplete PDF must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&mixed.stderr);
    assert!(
        stderr.contains("page(s) 2"),
        "the binary must report affected page 2; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("stored data"),
        "the binary must state that stored data was preserved; stderr:\n{stderr}"
    );
    assert_eq!(
        count_source(&dsn, &source),
        before,
        "a rejected incomplete PDF must write no rows"
    );

    let scanned = run_binary(&fixture("no-text-layer.pdf"), &source, &dsn);
    assert!(
        !scanned.status.success(),
        "a scanned PDF must exit non-zero"
    );
    assert_eq!(
        count_source(&dsn, &source),
        before,
        "a rejected fully scanned PDF must write no rows"
    );

    let missing = run_binary(&fixture("does-not-exist.pdf"), &source, &dsn);
    assert!(
        !missing.status.success(),
        "a fatal loader error must exit non-zero"
    );
    assert_eq!(
        count_source(&dsn, &source),
        before,
        "a fatal loader error must write no rows"
    );

    cleanup(&dsn, &source);
}
