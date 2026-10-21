//! Database schema round-trip test (real database behaviour).
//!
//! This test **fails clearly** when credentials or pgvector are unavailable; it
//! never skips to a pass, so a missing database can never yield a green suite.
//! It writes a chunk through the production Rust store, reads it back, and then
//! verifies only its own rows so existing data is preserved.

use rag_foundation::corpus::Corpus;
use rag_foundation::storage::{DocumentStore, StoredChunk, EMBEDDING_DIM};
use rag_foundation::storage_pg::PgStore;
use rag_foundation::transaction::SourceReplacer;

fn dsn() -> String {
    match std::env::var("DATABASE_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => panic!(
            "DATABASE_URL is not set: database acceptance requires credentials. \
             Load the repository .env and export DATABASE_URL. This test fails (never skips)."
        ),
    }
}

fn probe_pgvector(dsn: &str) -> String {
    let mut client = postgres::Client::connect(dsn, postgres::NoTls)
        .unwrap_or_else(|e| panic!("connect to the migration database failed: {e}"));
    let row = client
        .query_opt(
            "SELECT extversion FROM pg_extension WHERE extname = 'vector'",
            &[],
        )
        .unwrap_or_else(|e| panic!("pgvector probe failed: {e}"));
    match row {
        Some(r) => r.get::<_, String>(0),
        None => panic!("pgvector is not available in this database; the acceptance check fails, it does not skip"),
    }
}

#[test]
fn rust_store_round_trips_and_reads_schema_columns() {
    let dsn = dsn();
    let version = probe_pgvector(&dsn);
    assert!(!version.is_empty(), "pgvector must report a version");

    let store = PgStore::connect(&dsn).expect("open the Rust store");

    let tag = format!("{}{}", std::process::id(), "-rt");
    let source = format!("interop://rust/{tag}");
    let hash = format!("interop-rust-{tag}");
    let corpus = Corpus::new("interop").expect("valid interop corpus");

    let mut embedding = vec![0.0_f64; EMBEDDING_DIM];
    embedding[0] = 0.5;
    embedding[EMBEDDING_DIM - 1] = 1.0;

    let chunk = StoredChunk {
        content: "interop row written by Rust".to_string(),
        source: Some(source.clone()),
        section: Some("Interop".to_string()),
        chunk_index: 0,
        language: Some("english".to_string()),
        page: Some(1),
        corpus: Some("interop".to_string()),
        content_hash: hash.clone(),
        embed_model: "test-model".to_string(),
        embedding_dim: EMBEDDING_DIM as i32,
        chunker_config: "{}".to_string(),
        ingested_at: "2026-10-17T00:00:00Z".to_string(),
        embedding,
    };
    store.write_chunk(&chunk).expect("write the interop row");

    // Rust reads back the full schema column set.
    let rows = store
        .find_by_content_hash(&corpus, &hash, Some(source.as_str()))
        .expect("read back by content hash");
    assert_eq!(rows.len(), 1, "exactly one row must round-trip");
    let back = &rows[0];
    assert_eq!(back.chunk.source.as_deref(), Some(source.as_str()));
    assert_eq!(back.chunk.section.as_deref(), Some("Interop"));
    assert_eq!(back.chunk.page, Some(1));
    assert_eq!(back.chunk.language.as_deref(), Some("english"));
    assert_eq!(back.chunk.chunk_index, 0);
    assert_eq!(back.chunk.embedding_dim, EMBEDDING_DIM as i32);
    assert_eq!(back.chunk.embedding_dim(), EMBEDDING_DIM);

    let by_id = store
        .find_by_id(&corpus, back.id)
        .expect("read by id")
        .expect("the row exists");
    assert_eq!(by_id.chunk.content, "interop row written by Rust");

    // The dimension guard must reject without inserting anything.
    let bad = StoredChunk {
        content: "should never be stored".to_string(),
        source: Some(source.clone()),
        section: Some("Bad".to_string()),
        chunk_index: 0,
        language: None,
        page: None,
        corpus: None,
        content_hash: format!("{hash}-bad"),
        embed_model: "test-model".to_string(),
        embedding_dim: 3,
        chunker_config: "{}".to_string(),
        ingested_at: "2026-10-17T00:00:00Z".to_string(),
        embedding: vec![1.0, 2.0, 3.0],
    };
    assert!(
        store.write_chunk(&bad).is_err(),
        "a dimension mismatch must error before any write"
    );
    let bad_rows = store
        .find_by_content_hash(&corpus, &format!("{hash}-bad"), Some(source.as_str()))
        .expect("read the rejected hash");
    assert!(bad_rows.is_empty(), "a rejected write must insert no row");

    // Preserve existing data: remove only this test's rows.
    let mut client = postgres::Client::connect(&dsn, postgres::NoTls).expect("cleanup connect");
    let removed = client
        .execute(
            "DELETE FROM documents WHERE corpus = $1 AND source = $2",
            &[&corpus.as_str(), &source],
        )
        .expect("cleanup");
    assert!(removed >= 1, "cleanup removed this test's rows");
}

fn chunk_for(corpus: &Corpus, source: &str, hash: &str, content: &str) -> StoredChunk {
    StoredChunk {
        content: content.to_string(),
        source: Some(source.to_string()),
        section: Some("Isolation".to_string()),
        chunk_index: 0,
        language: Some("english".to_string()),
        page: None,
        corpus: Some(corpus.as_str().to_string()),
        content_hash: hash.to_string(),
        embed_model: "test-model".to_string(),
        embedding_dim: EMBEDDING_DIM as i32,
        chunker_config: "{}".to_string(),
        ingested_at: "2026-10-17T00:00:00Z".to_string(),
        embedding: vec![0.0; EMBEDDING_DIM],
    }
}

#[test]
fn replace_source_is_isolated_by_corpus() {
    let dsn = dsn();
    probe_pgvector(&dsn);
    let store = PgStore::connect(&dsn).expect("open the Rust store");

    let tag = format!("{}{}", std::process::id(), "-corpus");
    let source = format!("interop://corpus/{tag}");
    let first = Corpus::new(format!("interop-a-{tag}")).expect("valid first corpus");
    let second = Corpus::new(format!("interop-b-{tag}")).expect("valid second corpus");
    let first_hash = format!("hash-a-{tag}");
    let second_hash = format!("hash-b-{tag}");

    store
        .replace_source(
            &first,
            &source,
            &[chunk_for(&first, &source, &first_hash, "first-v1")],
        )
        .expect("write the first corpus");
    store
        .replace_source(
            &second,
            &source,
            &[chunk_for(&second, &source, &second_hash, "second-v1")],
        )
        .expect("write the second corpus");

    // Replacing the same source in the first corpus must not touch the second.
    store
        .replace_source(
            &first,
            &source,
            &[
                chunk_for(&first, &source, &first_hash, "first-v2-a"),
                chunk_for(&first, &source, &first_hash, "first-v2-b"),
            ],
        )
        .expect("replace the first corpus");

    let first_rows = store
        .find_by_content_hash(&first, &first_hash, Some(&source))
        .expect("read the first corpus");
    assert_eq!(first_rows.len(), 2);
    assert!(first_rows
        .iter()
        .all(|row| row.chunk.corpus.as_deref() == Some(first.as_str())));

    let leaked = store
        .find_by_content_hash(&second, &first_hash, Some(&source))
        .expect("read the second corpus by the first hash");
    assert!(
        leaked.is_empty(),
        "the first corpus hash must not leak into the second"
    );

    let second_rows = store
        .find_by_content_hash(&second, &second_hash, Some(&source))
        .expect("read the second corpus rows")
        .into_iter()
        .filter(|row| row.chunk.source.as_deref() == Some(source.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(second_rows.len(), 1);
    assert_eq!(second_rows[0].chunk.content, "second-v1");

    let mut client = postgres::Client::connect(&dsn, postgres::NoTls).expect("cleanup connect");
    for corpus in [&first, &second] {
        client
            .execute(
                "DELETE FROM documents WHERE corpus = $1 AND source = $2",
                &[&corpus.as_str(), &source],
            )
            .expect("cleanup");
    }
}
