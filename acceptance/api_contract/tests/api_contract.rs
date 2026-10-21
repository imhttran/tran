//! RUST-008A deterministic API contract.
//!
//! Starts the real Axum API with a deterministic Ollama server and a dedicated
//! PostgreSQL/pgvector database, then exercises upload validation, incomplete
//! PDF rejection, ingestion status, query citations, health, loopback defaults,
//! and the missing-context refusal. No endpoint is mocked.

use std::fs;
use std::path::{Path, PathBuf};

use api_contract_acceptance::{
    dsn, repo_root, seed_api_contract_admin, unique_tag, ApiProcess, CorpusCleanup, FakeOllama,
    OperatorIngest, TestAuth,
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
        .post(format!("{base}/api/documents/ingest"))
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

fn db_count(dsn: &str, corpus: &str, source: &str) -> i64 {
    let mut client =
        postgres::Client::connect(dsn, postgres::NoTls).expect("connect to the database");
    client
        .query_one(
            "SELECT count(*) FROM documents WHERE corpus = $1 AND source = $2",
            &[&corpus, &source],
        )
        .expect("count rows")
        .get(0)
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

    // A whitespace-only source is accepted as a pending submission: extraction,
    // the "no sections" rejection, and chunking all happen after review, so
    // submission itself writes nothing and approves nothing.
    let (status, body) = upload(&client, &base, &fixture("empty.txt"));
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["review_status"], "pending_review");
    assert_eq!(body["processing_status"], "unprocessed");
    assert_eq!(body["chunks"], 0);
    assert!(body["version_id"].as_i64().is_some());
    assert_eq!(db_count(&dsn, &corpus, "empty.txt"), 0);

    // Upload validation: size limit.
    let oversize = temp_file("oversize.md", &vec![b'a'; MAX_UPLOAD_BYTES + 1]);
    let (status, body) = upload(&client, &base, &oversize);
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "file_too_large");

    // An incomplete PDF is also accepted for review: it writes no chunks, so it
    // cannot enter retrieval, and the extraction failure is a processing
    // concern rather than a submission concern.
    let (status, body) = upload(&client, &base, &fixture("mixed-text-scan.pdf"));
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["review_status"], "pending_review");
    assert_eq!(body["chunks"], 0);
    assert_eq!(db_count(&dsn, &corpus, "mixed-text-scan.pdf"), 0);

    // A valid markdown submission is pending, private, and unchunked.
    let markdown = repo_root().join("examples/loan-policy.md");
    let (status, body) = upload(&client, &base, &markdown);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["review_status"], "pending_review");
    assert_eq!(body["chunks"], 0);
    let first_version = body["version_id"].as_i64().expect("submission version id");
    assert_eq!(db_count(&dsn, &corpus, "loan-policy.md"), 0);

    // A pending submission is not a stored source yet.
    let (status, _) = get_json(
        &client,
        &format!("{base}/api/documents/loan-policy.md/status"),
    );
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A repeated submission creates a new private version and still writes no
    // chunks or embedding.
    let (status, duplicate) = upload(&client, &base, &markdown);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(duplicate["review_status"], "pending_review");
    assert_eq!(duplicate["chunks"], 0);
    assert_ne!(
        duplicate["version_id"].as_i64().expect("second version id"),
        first_version,
        "a repeated submission is a new version, not an implicit edit"
    );
    assert_eq!(db_count(&dsn, &corpus, "loan-policy.md"), 0);

    // Nothing pending is retrievable: the approved-only path stays empty.
    let (status, pending) = post_json(
        &client,
        &format!("{base}/api/query"),
        &serde_json::json!({
            "question": "What happens if someone misses a payment?",
            "top_k": 4
        }),
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        pending["insufficient"], true,
        "a pending submission must never answer a query"
    );
    assert_eq!(pending["sources"], serde_json::json!([]));

    // The trusted local operator boundary publishes explicitly: it writes the
    // chunks and records an approved, ready, family-scoped version.
    let operator = OperatorIngest::new();
    operator.publish(&dsn, &corpus, ollama.url(), &markdown);

    let (status, stored) = get_json(
        &client,
        &format!("{base}/api/documents/loan-policy.md/status"),
    );
    assert_eq!(status, StatusCode::OK);
    let chunks = stored["chunks"].as_i64().expect("chunk count");
    assert!(chunks > 0);
    assert_eq!(db_count(&dsn, &corpus, "loan-policy.md"), chunks);
    assert_eq!(stored["corpus"], corpus.as_str());
    assert!(!stored["content_hash"]
        .as_str()
        .unwrap_or_default()
        .is_empty());
    let sections = stored["sections"].as_array().expect("sections");
    assert_eq!(sections.len() as i64, chunks);
    assert_eq!(sections[0]["section"], "Payments");
    assert!(sections.iter().all(|section| section["page"].is_null()));

    // PDF publication preserves page provenance.
    operator.publish(&dsn, &corpus, ollama.url(), &fixture("text-layer.pdf"));
    let (status, stored) = get_json(
        &client,
        &format!("{base}/api/documents/text-layer.pdf/status"),
    );
    assert_eq!(status, StatusCode::OK);
    let pages: Vec<i64> = stored["sections"]
        .as_array()
        .expect("pdf sections")
        .iter()
        .map(|section| section["page"].as_i64().expect("page"))
        .collect();
    assert_eq!(pages, vec![1, 2]);

    // Plain text publication.
    let note = temp_file("api-contract-note.txt", b"The support code is ZX-431.");
    operator.publish(&dsn, &corpus, ollama.url(), &note);
    let (status, stored) = get_json(
        &client,
        &format!("{base}/api/documents/api-contract-note.txt/status"),
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["sections"][0]["page"], Value::Null);

    // Query: cited answer whose sources resolve to stored rows.
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
            "citation {source_name}/{section}/{page:?} does not resolve to a stored row"
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
