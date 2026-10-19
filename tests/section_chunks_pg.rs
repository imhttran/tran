//! RUST-007D section-chunks read against the real database.
//!
//! Fails (never skips) when `DATABASE_URL` is unset. Writes rows under two
//! run-unique corpus tags, reads them back through
//! [`PgStore::section_chunks`], and deletes only those tags afterwards.

use rag_foundation::storage::{DocumentStore, StoredChunk, EMBEDDING_DIM};
use rag_foundation::storage_pg::{PgStore, SectionKey};

fn dsn() -> String {
    match std::env::var("DATABASE_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => panic!(
            "DATABASE_URL is not set: the section-chunks test requires the dedicated database. \
             Load the repository .env. This test fails (never skips)."
        ),
    }
}

struct Cleanup {
    dsn: String,
    corpora: Vec<String>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Ok(mut client) = postgres::Client::connect(&self.dsn, postgres::NoTls) {
            for corpus in &self.corpora {
                let _ = client.execute("DELETE FROM documents WHERE corpus = $1", &[corpus]);
            }
        }
    }
}

fn chunk(corpus: &str, source: &str, section: &str, index: i32) -> StoredChunk {
    let mut embedding = vec![0.0_f64; EMBEDDING_DIM];
    embedding[index as usize % EMBEDDING_DIM] = 1.0;
    StoredChunk {
        content: format!("{section} chunk {index}"),
        source: Some(source.to_string()),
        section: Some(section.to_string()),
        chunk_index: index,
        language: Some("english".to_string()),
        page: None,
        corpus: Some(corpus.to_string()),
        content_hash: format!("{corpus}-{section}-{index}"),
        embed_model: "test-model".to_string(),
        embedding_dim: EMBEDDING_DIM as i32,
        chunker_config: "{}".to_string(),
        ingested_at: "2026-10-18T00:00:00Z".to_string(),
        embedding,
    }
}

#[test]
fn section_chunks_caps_per_section_orders_and_scopes_to_the_corpus() {
    let dsn = dsn();
    let tag = format!(
        "section-chunks-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let corpus = format!("{tag}-run");
    let foreign = format!("{tag}-other");
    let _cleanup = Cleanup {
        dsn: dsn.clone(),
        corpora: vec![corpus.clone(), foreign.clone()],
    };
    let source = format!("section-chunks://{tag}");
    let store = PgStore::connect(&dsn).expect("open the store");

    // Inserted out of order so the ORDER BY is what sorts them.
    for index in [4, 2, 0, 3, 1] {
        store
            .write_chunk(&chunk(&corpus, &source, "Beta", index))
            .unwrap();
    }
    for index in [2, 0, 1] {
        store
            .write_chunk(&chunk(&corpus, &source, "Alpha", index))
            .unwrap();
    }
    store
        .write_chunk(&chunk(&corpus, &source, "Gamma", 0))
        .unwrap();
    // Same source/section in another corpus must never be returned.
    store
        .write_chunk(&chunk(&foreign, &source, "Alpha", 0))
        .unwrap();

    let keys = vec![
        SectionKey {
            source: source.clone(),
            section: "Beta".to_string(),
        },
        SectionKey {
            source: source.clone(),
            section: "Alpha".to_string(),
        },
    ];
    // limit 6 over 2 keys => 3 chunks per section.
    let rows = store.section_chunks(&corpus, &keys, 6).unwrap();
    let got: Vec<(String, i32)> = rows
        .iter()
        .map(|row| (row.chunk.section.clone().unwrap(), row.chunk.chunk_index))
        .collect();
    assert_eq!(
        got,
        vec![
            ("Alpha".to_string(), 0),
            ("Alpha".to_string(), 1),
            ("Alpha".to_string(), 2),
            ("Beta".to_string(), 0),
            ("Beta".to_string(), 1),
            ("Beta".to_string(), 2),
        ]
    );
    for row in &rows {
        assert_eq!(row.chunk.corpus.as_deref(), Some(corpus.as_str()));
        assert_eq!(row.chunk.source.as_deref(), Some(source.as_str()));
    }

    // A limit smaller than the key count still returns one chunk per section.
    let rows = store.section_chunks(&corpus, &keys, 1).unwrap();
    assert_eq!(rows.len(), 2);

    assert!(store.section_chunks(&corpus, &[], 20).unwrap().is_empty());
}
