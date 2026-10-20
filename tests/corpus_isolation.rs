//! Corpus isolation for replacement and the unchanged-source decision.
//!
//! Fails (never skips) when `DATABASE_URL` is unset. Writes rows under
//! run-unique corpus tags, replaces one corpus' copy of a source, and deletes
//! only those tags afterwards.

use rag_foundation::fingerprint::{unchanged_source, UnchangedSource};
use rag_foundation::storage::{DocumentStore, StoredChunk, EMBEDDING_DIM};
use rag_foundation::storage_pg::PgStore;
use rag_foundation::transaction::SourceReplacer;

fn dsn() -> String {
    match std::env::var("DATABASE_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => panic!(
            "DATABASE_URL is not set: the corpus isolation test requires the dedicated database. \
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

fn chunk(corpus: &str, source: &str, hash: &str, index: i32) -> StoredChunk {
    let mut embedding = vec![0.0_f64; EMBEDDING_DIM];
    embedding[index as usize % EMBEDDING_DIM] = 1.0;
    StoredChunk {
        content: format!("{corpus} chunk {index}"),
        source: Some(source.to_string()),
        section: Some("Intro".to_string()),
        chunk_index: index,
        language: Some("english".to_string()),
        page: None,
        corpus: Some(corpus.to_string()),
        content_hash: hash.to_string(),
        embed_model: "test-model".to_string(),
        embedding_dim: EMBEDDING_DIM as i32,
        chunker_config: "{}".to_string(),
        ingested_at: "2026-10-20T00:00:00Z".to_string(),
        embedding,
    }
}

#[test]
fn replacement_and_unchanged_check_are_scoped_to_the_corpus() {
    let dsn = dsn();
    let tag = format!(
        "corpus-isolation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let corpus_a = format!("{tag}-a");
    let corpus_b = format!("{tag}-b");
    let _cleanup = Cleanup {
        dsn: dsn.clone(),
        corpora: vec![corpus_a.clone(), corpus_b.clone()],
    };
    let source = format!("corpus-isolation://{tag}/README.md");
    let store = PgStore::connect(&dsn).expect("open the store");

    // The same filename and bytes exist in two corpora.
    store
        .write_chunk(&chunk(&corpus_a, &source, "shared-hash", 0))
        .unwrap();
    store
        .write_chunk(&chunk(&corpus_b, &source, "shared-hash", 0))
        .unwrap();

    // Both corpora read as unchanged for the same (source, hash).
    for corpus in [&corpus_a, &corpus_b] {
        assert_eq!(
            unchanged_source(&store, corpus, &source, "shared-hash").expect("read unchanged"),
            UnchangedSource::Unchanged { rows: 1 }
        );
    }

    // Replacing corpus A must not delete corpus B's copy of the same filename.
    let replacement = vec![
        chunk(&corpus_a, &source, "new-hash", 0),
        chunk(&corpus_a, &source, "new-hash", 1),
    ];
    store
        .replace_source(&corpus_a, &source, &replacement)
        .expect("replace corpus A");

    let rows_a = store.rows_for_corpus(&corpus_a).expect("read corpus A");
    let rows_b = store.rows_for_corpus(&corpus_b).expect("read corpus B");
    assert_eq!(rows_a.len(), 2, "corpus A got exactly the new rows");
    assert_eq!(rows_b.len(), 1, "corpus B kept its row");
    assert_eq!(rows_b[0].chunk.content_hash, "shared-hash");
    assert_eq!(
        rows_b[0].chunk.corpus.as_deref(),
        Some(corpus_b.as_str()),
        "corpus B's row still belongs to corpus B"
    );

    // A corpus with no rows for the source is still considered changed.
    assert_eq!(
        unchanged_source(&store, &format!("{tag}-empty"), &source, "shared-hash")
            .expect("read empty corpus"),
        UnchangedSource::Changed
    );
}
