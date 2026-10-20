//! Transactional source replacement with rollback.
//!
//! Replacement is the ingest writer's unit of work: for one source, the old
//! rows are deleted and the new chunks inserted, all inside **one transaction**.
//! A failure at any point rolls the whole transaction back, so the previous
//! rows stay in place and no new rows survive (PRD: "A forced failure during
//! replacement rolls back the complete transaction").
//!
//! The module is DB-free. It defines the [`SourceReplacer`] port the pipeline
//! depends on and an in-memory [`RecordingSourceReplacer`] used by tests (and
//! by the caller-level verification) to record writes and to inject a failure.
//! [`crate::storage_pg::PgStore`] implements the same port over a Postgres
//! `BEGIN` / `DELETE` / `INSERT` / `COMMIT` sequence.

use std::cell::RefCell;
use std::collections::BTreeMap;

use thiserror::Error;

use crate::corpus::Corpus;
use crate::storage::{validate_embedding_dim, DimensionMismatchError, StoredChunk, EMBEDDING_DIM};

/// Errors raised by a source replacement.
#[derive(Debug, Error)]
pub enum ReplaceError {
    /// The embedding dimension guard rejected a chunk before any statement ran.
    #[error(transparent)]
    Dimension(#[from] DimensionMismatchError),
    /// The store failed during the transaction; the transaction is rolled back.
    #[error("replace source failed: {0}")]
    Store(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// A chunk did not carry the corpus named by the replacement.
    #[error("corpus mismatch: {0}")]
    Corpus(String),
    /// A test-only injected failure, used to prove rollback.
    #[error("injected replacement failure: {0}")]
    Injected(String),
}

/// The transactional replacement port.
///
/// Implementations must run the delete and the inserts as one transaction and
/// must validate every chunk's embedding dimension before the transaction
/// starts.
pub trait SourceReplacer {
    /// Replaces every stored row of `(corpus, source)` with `chunks` atomically.
    fn replace_source(
        &self,
        corpus: &Corpus,
        source: &str,
        chunks: &[StoredChunk],
    ) -> Result<(), ReplaceError>;
}

/// Validates every chunk's embedding dimension before any effect runs.
///
/// Call this at the start of every [`SourceReplacer::replace_source`]
/// implementation: a mismatch returns [`ReplaceError::Dimension`] and no
/// statement executes.
pub fn validate_all_dims(chunks: &[StoredChunk]) -> Result<(), ReplaceError> {
    for chunk in chunks {
        validate_embedding_dim(EMBEDDING_DIM, chunk.embedding_dim())?;
    }
    Ok(())
}

/// Validates that every chunk belongs to `corpus` before any delete or insert.
pub fn validate_chunk_corpora(corpus: &Corpus, chunks: &[StoredChunk]) -> Result<(), ReplaceError> {
    for chunk in chunks {
        match chunk.corpus.as_deref() {
            Some(value) if value == corpus.as_str() => {}
            Some(other) => {
                return Err(ReplaceError::Corpus(format!(
                    "chunk corpus {other:?} does not match replacement corpus {corpus}"
                )))
            }
            None => {
                return Err(ReplaceError::Corpus(format!(
                    "chunk has no corpus; expected {corpus}"
                )))
            }
        }
    }
    Ok(())
}

/// How many writes a [`RecordingSourceReplacer`] applied.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReplaceCounts {
    /// Number of replace calls that committed.
    pub replacements: usize,
    /// Number of replace calls that rolled back.
    pub rollbacks: usize,
    /// Total chunk rows written by committed replacements.
    pub rows_written: usize,
}

/// An in-memory [`SourceReplacer`] that records writes, keeps committed rows,
/// and can inject a failure mid-transaction to prove rollback.
///
/// A replacement stages the delete and the inserts in a copy of the table and
/// commits only when every step succeeds. An error (dimension guard, injected
/// failure, or a forced write error) discards the staged copy, so the previously
/// committed rows remain, exactly as a database rollback would leave them.
#[derive(Debug, Default)]
pub struct RecordingSourceReplacer {
    table: RefCell<BTreeMap<(String, String), Vec<StoredChunk>>>,
    counts: RefCell<ReplaceCounts>,
    fail_after_rows: RefCell<Option<usize>>,
}

impl RecordingSourceReplacer {
    /// Builds an empty recording replacer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Injects a failure after `rows` inserts of the next replacement.
    ///
    /// `0` fails right after the delete and before any insert. The call then
    /// rolls back, so the prior rows survive.
    pub fn inject_failure_after(&self, rows: usize) {
        *self.fail_after_rows.borrow_mut() = Some(rows);
    }

    /// Returns the committed rows for `(corpus, source)`.
    pub fn rows_for(&self, corpus: &Corpus, source: &str) -> Vec<StoredChunk> {
        self.table
            .borrow()
            .get(&(corpus.as_str().to_string(), source.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// Returns the write counters.
    pub fn counts(&self) -> ReplaceCounts {
        *self.counts.borrow()
    }

    /// Returns `true` when no committed row exists for any source.
    pub fn is_empty(&self) -> bool {
        self.table.borrow().values().all(Vec::is_empty)
    }
}

impl SourceReplacer for RecordingSourceReplacer {
    fn replace_source(
        &self,
        corpus: &Corpus,
        source: &str,
        chunks: &[StoredChunk],
    ) -> Result<(), ReplaceError> {
        // Fail-fast, before any effect: the same ordering the Postgres
        // implementation enforces.
        validate_all_dims(chunks)?;
        validate_chunk_corpora(corpus, chunks)?;

        let fail_after = *self.fail_after_rows.borrow();
        let mut staged = Vec::with_capacity(chunks.len());
        for (index, chunk) in chunks.iter().enumerate() {
            if fail_after == Some(index) {
                self.counts.borrow_mut().rollbacks += 1;
                *self.fail_after_rows.borrow_mut() = None;
                return Err(ReplaceError::Injected(format!(
                    "failed after {index} of {} rows",
                    chunks.len()
                )));
            }
            staged.push(chunk.clone());
        }

        // Commit: the delete and every insert take effect together. The key is
        // the pair, so the same source in another corpus is untouched.
        self.table
            .borrow_mut()
            .insert((corpus.as_str().to_string(), source.to_string()), staged);
        {
            let mut counts = self.counts.borrow_mut();
            counts.replacements += 1;
            counts.rows_written += chunks.len();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(name: &str) -> Corpus {
        Corpus::new(name).unwrap()
    }

    fn chunk_in(corpus: &Corpus, source: &str, index: i32, dim: usize) -> StoredChunk {
        StoredChunk {
            content: format!("body {index}"),
            source: Some(source.to_string()),
            section: Some("Intro".to_string()),
            chunk_index: index,
            language: Some("english".to_string()),
            page: None,
            corpus: Some(corpus.as_str().to_string()),
            content_hash: "hash".to_string(),
            embed_model: "nomic-embed-text".to_string(),
            embedding_dim: dim as i32,
            chunker_config: "size=50,overlap=20".to_string(),
            ingested_at: "2024-01-01T00:00:00Z".to_string(),
            embedding: vec![0.0; dim],
        }
    }

    fn chunk(source: &str, index: i32, dim: usize) -> StoredChunk {
        chunk_in(&corpus("trusted"), source, index, dim)
    }

    #[test]
    fn successful_replacement_replaces_the_old_rows() {
        let store = RecordingSourceReplacer::new();
        let trusted = corpus("trusted");
        store
            .replace_source(&trusted, "doc.md", &[chunk("doc.md", 0, EMBEDDING_DIM)])
            .unwrap();
        assert_eq!(store.rows_for(&trusted, "doc.md").len(), 1);

        store
            .replace_source(
                &trusted,
                "doc.md",
                &[
                    chunk("doc.md", 0, EMBEDDING_DIM),
                    chunk("doc.md", 1, EMBEDDING_DIM),
                ],
            )
            .unwrap();

        let rows = store.rows_for(&trusted, "doc.md");
        assert_eq!(
            rows.len(),
            2,
            "old rows are removed and exactly the new ones kept"
        );
        assert_eq!(rows[1].chunk_index, 1);
        assert_eq!(store.counts().replacements, 2);
    }

    #[test]
    fn successful_replacement_leaves_other_sources_alone() {
        let store = RecordingSourceReplacer::new();
        let trusted = corpus("trusted");
        store
            .replace_source(&trusted, "a.md", &[chunk("a.md", 0, EMBEDDING_DIM)])
            .unwrap();
        store
            .replace_source(&trusted, "b.md", &[chunk("b.md", 0, EMBEDDING_DIM)])
            .unwrap();
        assert_eq!(store.rows_for(&trusted, "a.md").len(), 1);
        assert_eq!(store.rows_for(&trusted, "b.md").len(), 1);
    }

    #[test]
    fn replacement_is_isolated_by_corpus() {
        let store = RecordingSourceReplacer::new();
        let first = corpus("corpus-a");
        let second = corpus("corpus-b");

        store
            .replace_source(
                &first,
                "same.md",
                &[chunk_in(&first, "same.md", 0, EMBEDDING_DIM)],
            )
            .unwrap();
        store
            .replace_source(
                &second,
                "same.md",
                &[chunk_in(&second, "same.md", 0, EMBEDDING_DIM)],
            )
            .unwrap();

        store
            .replace_source(
                &first,
                "same.md",
                &[
                    chunk_in(&first, "same.md", 0, EMBEDDING_DIM),
                    chunk_in(&first, "same.md", 1, EMBEDDING_DIM),
                ],
            )
            .unwrap();

        assert_eq!(store.rows_for(&first, "same.md").len(), 2);
        assert_eq!(store.rows_for(&second, "same.md").len(), 1);
        assert_eq!(
            store.rows_for(&second, "same.md")[0].corpus.as_deref(),
            Some("corpus-b")
        );
    }

    #[test]
    fn corpus_mismatch_is_rejected_before_any_effect() {
        let store = RecordingSourceReplacer::new();
        let first = corpus("corpus-a");
        let second = corpus("corpus-b");

        let err = store
            .replace_source(
                &first,
                "same.md",
                &[chunk_in(&second, "same.md", 0, EMBEDDING_DIM)],
            )
            .unwrap_err();

        assert!(matches!(err, ReplaceError::Corpus(_)));
        assert!(store.is_empty());
        assert_eq!(store.counts().replacements, 0);
    }

    #[test]
    fn forced_failure_rolls_back_the_complete_transaction() {
        let store = RecordingSourceReplacer::new();
        let trusted = corpus("trusted");
        store
            .replace_source(
                &trusted,
                "doc.md",
                &[
                    chunk("doc.md", 0, EMBEDDING_DIM),
                    chunk("doc.md", 1, EMBEDDING_DIM),
                ],
            )
            .unwrap();
        let before = store.rows_for(&trusted, "doc.md");
        assert_eq!(before.len(), 2);

        store.inject_failure_after(1);
        let err = store
            .replace_source(
                &trusted,
                "doc.md",
                &[
                    chunk("doc.md", 0, EMBEDDING_DIM),
                    chunk("doc.md", 1, EMBEDDING_DIM),
                ],
            )
            .unwrap_err();

        assert!(matches!(err, ReplaceError::Injected(_)));
        assert_eq!(
            store.rows_for(&trusted, "doc.md"),
            before,
            "prior rows must survive rollback"
        );
        assert_eq!(store.counts().rollbacks, 1);
        assert_eq!(store.counts().replacements, 1);
        assert_eq!(store.counts().rows_written, 2);
    }

    #[test]
    fn failure_before_any_insert_also_preserves_the_old_rows() {
        let store = RecordingSourceReplacer::new();
        let trusted = corpus("trusted");
        store
            .replace_source(&trusted, "doc.md", &[chunk("doc.md", 0, EMBEDDING_DIM)])
            .unwrap();
        let before = store.rows_for(&trusted, "doc.md");

        store.inject_failure_after(0);
        assert!(store
            .replace_source(&trusted, "doc.md", &[chunk("doc.md", 5, EMBEDDING_DIM)])
            .is_err());
        assert_eq!(store.rows_for(&trusted, "doc.md"), before);
    }

    #[test]
    fn dimension_mismatch_fails_before_any_statement() {
        let store = RecordingSourceReplacer::new();
        let trusted = corpus("trusted");
        let err = store
            .replace_source(&trusted, "doc.md", &[chunk("doc.md", 0, 3)])
            .unwrap_err();
        assert!(matches!(err, ReplaceError::Dimension(_)));
        assert!(store.is_empty());
        assert_eq!(store.counts().replacements, 0);
        assert_eq!(store.counts().rows_written, 0);
    }
}
