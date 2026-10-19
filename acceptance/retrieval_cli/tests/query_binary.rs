//! RUST-006B production query-binary acceptance tests.
//!
//! These tests build and run the real `query` binary against the dedicated
//! PostgreSQL/pgvector database and a scripted Ollama HTTP server. They cover
//! cited source/page output, insufficient evidence, and dependency failures,
//! and clean up only test-owned rows.

mod support;

use support::{count_source, dsn, run_query, seed_row, unique_tag, Cleanup, FakeOllama};

#[test]
fn production_query_returns_cited_source_and_page() {
    let dsn = dsn();
    let tag = unique_tag();
    let source = format!("acceptance-query-{tag}.md");
    let corpus = format!("acceptance-query-corpus-{tag}");
    let _cleanup = Cleanup::new(&dsn, &source);
    seed_row(&dsn, &source, &corpus);

    let server = FakeOllama::start(&source, false);
    let binary = support::query_binary();
    let output = run_query(&binary, "What is the answer?", &dsn, server.url(), &corpus);

    assert!(
        output.status.success(),
        "query failed; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("The answer is 42"),
        "answer text missing; stdout:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("[{source} - Intro - p.2]")),
        "cited source/page missing; stdout:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("source: {source} - Intro - p.2")),
        "rendered page reference missing; stdout:\n{stdout}"
    );
    assert_eq!(
        count_source(&dsn, &source),
        1,
        "querying must not write or delete stored rows"
    );
}

#[test]
fn production_query_refuses_without_evidence() {
    let dsn = dsn();
    let tag = unique_tag();
    let corpus = format!("acceptance-empty-corpus-{tag}");
    let server = FakeOllama::start("unused-source.md", false);
    let binary = support::query_binary();
    let output = run_query(&binary, "What is the answer?", &dsn, server.url(), &corpus);

    assert!(
        output.status.success(),
        "refusal should be a successful answer; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("I cannot answer this question from the provided documents."),
        "baseline refusal missing; stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("source:"),
        "a refusal must not invent citations; stdout:\n{stdout}"
    );
}

#[test]
fn production_query_reports_database_failure() {
    let tag = unique_tag();
    let corpus = format!("acceptance-db-failure-{tag}");
    let server = FakeOllama::start("unused-source.md", false);
    let binary = support::query_binary();
    let output = run_query(
        &binary,
        "What is the answer?",
        "postgres://nobody@127.0.0.1:1/nope?sslmode=disable",
        server.url(),
        &corpus,
    );

    assert!(
        !output.status.success(),
        "a database failure must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("connect to the document store") || stderr.contains("query:"),
        "database dependency failure was not reported clearly; stderr:\n{stderr}"
    );
}

#[test]
fn production_query_reports_generation_failure() {
    let dsn = dsn();
    let tag = unique_tag();
    let source = format!("acceptance-gen-failure-{tag}.md");
    let corpus = format!("acceptance-gen-corpus-{tag}");
    let _cleanup = Cleanup::new(&dsn, &source);
    seed_row(&dsn, &source, &corpus);

    let server = FakeOllama::start(&source, true);
    let binary = support::query_binary();
    let output = run_query(&binary, "What is the answer?", &dsn, server.url(), &corpus);

    assert!(
        !output.status.success(),
        "a generation failure must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("answer failed") || stderr.contains("/api/generate"),
        "generation dependency failure was not reported clearly; stderr:\n{stderr}"
    );
}
