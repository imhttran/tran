//! RUST-008A deterministic API contract.
//!
//! Starts the real Axum API with a deterministic Ollama server and a dedicated
//! PostgreSQL/pgvector database, then exercises upload validation, pending
//! review, processing, publication, query citations, health, loopback defaults,
//! and the missing-context refusal. No endpoint is mocked.

use std::fs;
use std::path::{Path, PathBuf};

use api_contract_acceptance::{
    dsn, repo_root, seed_api_contract_admin, unique_tag, ApiProcess, CorpusCleanup, FakeOllama,
    TestAuth,
};
use rag_foundation::api::{DEFAULT_HOST, DEFAULT_PORT, MAX_UPLOAD_BYTES};
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::StatusCode;
use serde_json::Value;

fn client(token: &str) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("bearer header"),
    );
    Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("build the HTTP client")
}

fn fixture(name: &str) -> PathBuf {
    repo_root().join("fixtures/parity/inputs").join(name)
}

fn upload(client: &Client, base: &str, path: &Path) -> (StatusCode, Value) {
    let form = reqwest::blocking::multipart::Form::new()
        .file("file", path)
        .expect("build the multipart form");
    let response: Response = client
        .post(format!("{base}/api/documents/submissions"))
        .multipart(form)
        .send()
        .expect("POST /api/documents/ingest");
    json(response)
}

fn json(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let text = response.text().unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    (status, body)
}

fn get_json(client: &Client, url: &str) -> (StatusCode, Value) {
    json(client.get(url).send().expect("GET request"))
}

fn post_json(client: &Client, url: &str, body: &Value) -> (StatusCode, Value) {
    json(client.post(url).json(body).send().expect("POST request"))
}

fn citation_exists(
    dsn: &str,
    corpus: &str,
    source: &str,
    section: &str,
    page: Option<i64>,
) -> bool {
    let mut client = postgres::Client::connect(dsn, postgres::NoTls).expect("connect for citation");
    let count: i64 = match page {
        Some(page) => {
            let page = page as i32;
            client
                .query_one(
                    "SELECT count(*) FROM documents \
                     WHERE corpus = $1 AND source = $2 AND section = $3 AND page = $4",
                    &[&corpus, &source, &section, &page],
                )
                .expect("count paged citation")
                .get(0)
        }
        None => client
            .query_one(
                "SELECT count(*) FROM documents \
                 WHERE corpus = $1 AND source = $2 AND section = $3",
                &[&corpus, &source, &section],
            )
            .expect("count citation")
            .get(0),
    };
    count > 0
}

fn db_count_by_version(dsn: &str, version_id: i64) -> i64 {
    let mut client =
        postgres::Client::connect(dsn, postgres::NoTls).expect("connect for version count");
    client
        .query_one(
            "SELECT count(*) FROM documents WHERE document_version_id = $1",
            &[&version_id],
        )
        .expect("count version rows")
        .get(0)
}

fn db_sections_by_version(dsn: &str, version_id: i64) -> Vec<(String, Option<i64>)> {
    let mut client = postgres::Client::connect(dsn, postgres::NoTls).expect("connect for sections");
    client
        .query(
            "SELECT coalesce(section, ''), page::bigint FROM documents
              WHERE document_version_id = $1 ORDER BY chunk_index, id",
            &[&version_id],
        )
        .expect("read version sections")
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect()
}

fn submit_and_publish(client: &Client, base: &str, path: &Path) -> i64 {
    let (status, body) = upload(client, base, path);
    assert_eq!(status, StatusCode::OK, "submission failed: {body}");
    let version_id = body["version_id"].as_i64().expect("version id");
    assert_eq!(body["review_status"], "pending_review");
    assert_eq!(body["chunks"], 0);
    let (status, _) = post_json(
        client,
        &format!("{base}/api/documents/submissions/{version_id}/review"),
        &serde_json::json!({ "approve": true, "scope": "family" }),
    );
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        client,
        &format!("{base}/api/documents/submissions/{version_id}/process"),
        &serde_json::json!({}),
    );
    assert_eq!(status, StatusCode::OK);
    let (status, run) = post_json(
        client,
        &format!("{base}/api/documents/jobs/run"),
        &serde_json::json!({}),
    );
    assert_eq!(status, StatusCode::OK);
    let processed = run["processed"].as_array().expect("processed outcomes");
    assert!(
        processed.iter().any(|row| {
            row["version_id"].as_i64() == Some(version_id) && row["status"] == "ready"
        }),
        "version {version_id} did not become ready: {run}"
    );
    version_id
}

fn submit_and_fail(client: &Client, base: &str, path: &Path) -> i64 {
    let (status, body) = upload(client, base, path);
    assert_eq!(status, StatusCode::OK, "submission failed: {body}");
    let version_id = body["version_id"].as_i64().expect("version id");
    let (status, _) = post_json(
        client,
        &format!("{base}/api/documents/submissions/{version_id}/review"),
        &serde_json::json!({ "approve": true, "scope": "family" }),
    );
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        client,
        &format!("{base}/api/documents/submissions/{version_id}/process"),
        &serde_json::json!({}),
    );
    assert_eq!(status, StatusCode::OK);
    let (status, run) = post_json(
        client,
        &format!("{base}/api/documents/jobs/run"),
        &serde_json::json!({}),
    );
    assert_eq!(status, StatusCode::OK);
    let processed = run["processed"].as_array().expect("processed outcomes");
    assert!(
        processed.iter().any(|row| {
            row["version_id"].as_i64() == Some(version_id) && row["status"] == "failed"
        }),
        "version {version_id} was not rejected: {run}"
    );
    version_id
}

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(unique_tag("api-contract"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(name);
    fs::write(&path, bytes).expect("write temp file");
    path
}

#[test]
fn api_contract_upload_status_query_health_and_refusal() {
    let dsn = dsn();
    let corpus = unique_tag("api-contract");
    let _cleanup = CorpusCleanup::new(dsn.clone(), corpus.clone());
    let ollama = FakeOllama::start();
    let auth = TestAuth::start();
    seed_api_contract_admin(&dsn, auth.issuer(), "api-contract-admin", &corpus);
    let api = ApiProcess::start(&dsn, &corpus, ollama.url(), &auth);
    let base = api.base_url().to_string();
    let client = client(auth.token());

    // Loopback defaults are part of the contract.
    assert_eq!(DEFAULT_HOST, "127.0.0.1");
    assert_eq!(DEFAULT_PORT, 8080);
    assert_eq!(MAX_UPLOAD_BYTES, 10 * 1024 * 1024);

    // Health: healthy, structured, and free of credentials.
    let (status, health) = get_json(&client, &format!("{base}/api/health"));
    assert_eq!(status, StatusCode::OK);
    assert_eq!(health["status"], "ok");
    assert_eq!(health["service"], "rust-api");
    assert_eq!(health["database"], "ok");
    assert_eq!(health["model"], "ok");
    let health_text = health.to_string();
    for forbidden in ["postgres://", "DATABASE_URL", "OLLAMA_URL", "password"] {
        assert!(
            !health_text.contains(forbidden),
            "health response leaked {forbidden}: {health_text}"
        );
    }

    // Upload validation: unsupported type.
    let exe = temp_file("document.exe", b"not a document");
    let (status, body) = upload(&client, &base, &exe);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "unsupported_type");

    // Upload validation: a zero-byte file is rejected before any effect.
    let zero = temp_file("empty.txt", b"");
    let (status, body) = upload(&client, &base, &zero);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "empty_file");

    // Upload validation: size limit.
    let oversize = temp_file("oversize.md", &vec![b'a'; MAX_UPLOAD_BYTES + 1]);
    let (status, body) = upload(&client, &base, &oversize);
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "file_too_large");

    // A whitespace-only Markdown file is accepted as a pending submission but
    // processing rejects it and publishes nothing.
    let whitespace = fixture("empty.txt");
    let empty_version = submit_and_fail(&client, &base, &whitespace);
    assert_eq!(db_count_by_version(&dsn, empty_version), 0);

    // Incomplete PDF: accepted for review, rejected by processing, never
    // published.
    let pdf_incomplete = fixture("mixed-text-scan.pdf");
    let incomplete_version = submit_and_fail(&client, &base, &pdf_incomplete);
    assert_eq!(db_count_by_version(&dsn, incomplete_version), 0);
    let (status, _) = get_json(
        &client,
        &format!("{base}/api/documents/mixed-text-scan.pdf/status"),
    );
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Markdown: pending first, then explicit approval and processing.
    let markdown = repo_root().join("examples/loan-policy.md");
    let (status, pending) = upload(&client, &base, &markdown);
    assert_eq!(status, StatusCode::OK);
    let markdown_version = pending["version_id"].as_i64().expect("version id");
    assert_eq!(pending["review_status"], "pending_review");
    assert_eq!(pending["chunks"], 0);
    assert_eq!(db_count_by_version(&dsn, markdown_version), 0);
    let (status, _) = post_json(
        &client,
        &format!("{base}/api/documents/submissions/{markdown_version}/review"),
        &serde_json::json!({ "approve": true, "scope": "family" }),
    );
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        &client,
        &format!("{base}/api/documents/submissions/{markdown_version}/process"),
        &serde_json::json!({}),
    );
    assert_eq!(status, StatusCode::OK);
    let (status, run) = post_json(
        &client,
        &format!("{base}/api/documents/jobs/run"),
        &serde_json::json!({}),
    );
    assert_eq!(status, StatusCode::OK);
    assert!(run["processed"]
        .as_array()
        .expect("processed")
        .iter()
        .any(
            |row| row["version_id"].as_i64() == Some(markdown_version) && row["status"] == "ready"
        ));
    let chunks = db_count_by_version(&dsn, markdown_version);
    assert!(chunks > 0);
    let sections = db_sections_by_version(&dsn, markdown_version);
    assert_eq!(sections.len() as i64, chunks);
    assert!(sections.iter().all(|(_, page)| page.is_none()));

    // A second submission of the same file is a new version, not a silent
    // replacement of the published one.
    let (status, duplicate) = upload(&client, &base, &markdown);
    assert_eq!(status, StatusCode::OK);
    let duplicate_version = duplicate["version_id"].as_i64().expect("duplicate id");
    assert_ne!(duplicate_version, markdown_version);
    assert_eq!(duplicate["review_status"], "pending_review");
    let (status, _) = post_json(
        &client,
        &format!("{base}/api/documents/submissions/{duplicate_version}/review"),
        &serde_json::json!({ "approve": true, "scope": "family" }),
    );
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        &client,
        &format!("{base}/api/documents/submissions/{duplicate_version}/process"),
        &serde_json::json!({}),
    );
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        &client,
        &format!("{base}/api/documents/jobs/run"),
        &serde_json::json!({}),
    );
    assert_eq!(status, StatusCode::OK);

    // PDF ingestion preserves page provenance after publication.
    let pdf = fixture("text-layer.pdf");
    let pdf_version = submit_and_publish(&client, &base, &pdf);
    let pages: Vec<i64> = db_sections_by_version(&dsn, pdf_version)
        .into_iter()
        .map(|(_, page)| page.expect("paged PDF section"))
        .collect();
    assert_eq!(pages, vec![1, 2]);

    // Plain text publication.
    let note = temp_file("api-contract-note.txt", b"The support code is ZX-431.");
    let note_version = submit_and_publish(&client, &base, &note);
    assert!(db_count_by_version(&dsn, note_version) > 0);

    // Query: cited answer whose sources resolve to published rows only.
    let (status, answer) = post_json(
        &client,
        &format!("{base}/api/query"),
        &serde_json::json!({
            "question": "What happens if someone misses a payment?",
            "top_k": 4
        }),
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer["insufficient"], false);
    let text = answer["answer"].as_str().expect("answer text");
    assert!(text.contains('['), "answer must carry a citation: {text}");
    let sources = answer["sources"].as_array().expect("sources");
    assert!(!sources.is_empty(), "a cited answer must return sources");
    for source in sources {
        let source_name = source["source"].as_str().expect("source name");
        let section = source["section"].as_str().expect("section");
        let page = source["page"].as_i64();
        assert!(
            citation_exists(&dsn, &corpus, source_name, section, page),
            "citation {source_name}/{section}/{page:?} does not resolve to a published row"
        );
        assert!(
            !source["text"].as_str().unwrap_or_default().is_empty(),
            "a source must carry its passage text"
        );
    }

    // Empty question is rejected.
    let (status, body) = post_json(
        &client,
        &format!("{base}/api/query"),
        &serde_json::json!({ "question": "   " }),
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "empty_question");

    // Missing context: an empty corpus returns the baseline refusal.
    let mut db =
        postgres::Client::connect(&dsn, postgres::NoTls).expect("connect for corpus reset");
    db.execute("DELETE FROM documents WHERE corpus = $1", &[&corpus])
        .expect("delete only the run corpus");
    let (status, refusal) = post_json(
        &client,
        &format!("{base}/api/query"),
        &serde_json::json!({ "question": "What happens if someone misses a payment?" }),
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(refusal["insufficient"], true);
    assert_eq!(
        refusal["answer"],
        "I cannot answer this question from the provided documents."
    );
    assert_eq!(refusal["sources"], serde_json::json!([]));
}
