//! RUST-007D `expbilingual` failure exits.
//!
//! A missing dataset, an unreachable database, and an unreachable model
//! endpoint must each exit non-zero and name the dataset path, the database
//! URL, or the endpoint. `QUERY_CACHE` always points into a temporary
//! directory so the repository cache is never written.

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_expbilingual");

fn command(cache_dir: &tempfile::TempDir) -> Command {
    let mut command = Command::new(BIN);
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("QUERY_CACHE", cache_dir.path().join("cache.json"))
        .env("ARM", "A")
        .env_remove("EVAL_DATASET");
    command
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn missing_dataset_exits_non_zero_naming_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let output = command(&dir)
        .env("EVAL_DATASET", "/nonexistent/bilingual.json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("/nonexistent/bilingual.json"),
        "{}",
        stderr(&output)
    );
    assert!(!dir.path().join("cache.json").exists());
}

#[test]
fn unreachable_database_exits_non_zero_naming_the_url() {
    let dir = tempfile::tempdir().unwrap();
    let url = "postgres://nobody:nothing@127.0.0.1:1/unreachable?sslmode=disable";
    let output = command(&dir).env("DATABASE_URL", url).output().unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains(url), "{}", stderr(&output));
}

#[test]
fn unreachable_model_endpoint_exits_non_zero_naming_the_endpoint() {
    let dsn = match std::env::var("DATABASE_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => panic!(
            "DATABASE_URL is not set: this test needs a reachable database so the failure is \
             the model endpoint. Load the repository .env. This test fails (never skips)."
        ),
    };
    let dir = tempfile::tempdir().unwrap();
    let endpoint = "http://127.0.0.1:1";
    let output = command(&dir)
        .env("DATABASE_URL", dsn)
        .env("OLLAMA_URL", endpoint)
        .env("RAG_CORPUS", "expbilingual-unreachable-endpoint")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains(endpoint), "{message}");
    assert!(message.contains("/api/embed"), "{message}");
}
