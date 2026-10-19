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

use crate::storage::{validate_embedding_dim, DimensionMismatchError, StoredChunk, EMBEDDING_DIM};

/// Errors raised by a source replacement.
#[derive(Debug, Error)]
pub enum ReplaceError {
    /// The embedding dimension guard rejected a chunk before any statement ran.
    #[error(transparent)]
    Dimension(#[from] DimensionMismatchError),
    /// The store failed during the transaction; the transaction is rolled back.
    #[error("replace source failed: {0}")]
    Store(String),
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
    /// Replaces every stored row of `source` with `chunks` atomically.
    fn replace_source(&self, source: &str, chunks: &[StoredChunk]) -> Result<(), ReplaceError>;
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
    table: RefCell<BTreeMap<String, Vec<StoredChunk>>>,
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

    /// Returns the committed rows for `source`.
    pub fn rows_for(&self, source: &str) -> Vec<StoredChunk> {
        self.table.borrow().get(source).cloned().unwrap_or_default()
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
    fn replace_source(&self, source: &str, chunks: &[StoredChunk]) -> Result<(), ReplaceError> {
        // Fail-fast, before any effect: the same ordering the Postgres
        // implementation enforces.
        validate_all_dims(chunks)?;

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

        // Commit: the delete and every insert take effect together.
        self.table.borrow_mut().insert(source.to_string(), staged);
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

    fn chunk(source: &str, index: i32, dim: usize) -> StoredChunk {
        StoredChunk {
            content: format!("body {index}"),
            source: Some(source.to_string()),
            section: Some("Intro".to_string()),
            chunk_index: index,
            language: Some("english".to_string()),
            page: None,
            corpus: Some("trusted".to_string()),
            content_hash: "hash".to_string(),
            embed_model: "nomic-embed-text".to_string(),
            embedding_dim: dim as i32,
            chunker_config: "size=50,overlap=20".to_string(),
            ingested_at: "2024-01-01T00:00:00Z".to_string(),
            embedding: vec![0.0; dim],
        }
    }

    #[test]
    fn successful_replacement_replaces_the_old_rows() {
        let store = RecordingSourceReplacer::new();
        store
            .replace_source("doc.md", &[chunk("doc.md", 0, EMBEDDING_DIM)])
            .unwrap();
        assert_eq!(store.rows_for("doc.md").len(), 1);

        store
            .replace_source(
                "doc.md",
                &[
                    chunk("doc.md", 0, EMBEDDING_DIM),
                    chunk("doc.md", 1, EMBEDDING_DIM),
                ],
            )
            .unwrap();

        let rows = store.rows_for("doc.md");
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
        store
            .replace_source("a.md", &[chunk("a.md", 0, EMBEDDING_DIM)])
            .unwrap();
        store
            .replace_source("b.md", &[chunk("b.md", 0, EMBEDDING_DIM)])
            .unwrap();
        assert_eq!(store.rows_for("a.md").len(), 1);
        assert_eq!(store.rows_for("b.md").len(), 1);
    }

    #[test]
    fn forced_failure_rolls_back_the_complete_transaction() {
        let store = RecordingSourceReplacer::new();
        store
            .replace_source(
                "doc.md",
                &[
                    chunk("doc.md", 0, EMBEDDING_DIM),
                    chunk("doc.md", 1, EMBEDDING_DIM),
                ],
            )
            .unwrap();
        let before = store.rows_for("doc.md");
        assert_eq!(before.len(), 2);

        store.inject_failure_after(1);
        let err = store
            .replace_source(
                "doc.md",
                &[
                    chunk("doc.md", 0, EMBEDDING_DIM),
                    chunk("doc.md", 1, EMBEDDING_DIM),
                ],
            )
            .unwrap_err();

        assert!(matches!(err, ReplaceError::Injected(_)));
        assert_eq!(
            store.rows_for("doc.md"),
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
        store
            .replace_source("doc.md", &[chunk("doc.md", 0, EMBEDDING_DIM)])
            .unwrap();
        let before = store.rows_for("doc.md");

        store.inject_failure_after(0);
        assert!(store
            .replace_source("doc.md", &[chunk("doc.md", 5, EMBEDDING_DIM)])
            .is_err());
        assert_eq!(store.rows_for("doc.md"), before);
    }

    #[test]
    fn dimension_mismatch_fails_before_any_statement() {
        let store = RecordingSourceReplacer::new();
        let err = store
            .replace_source("doc.md", &[chunk("doc.md", 0, 3)])
            .unwrap_err();
        assert!(matches!(err, ReplaceError::Dimension(_)));
        assert!(store.is_empty());
        assert_eq!(store.counts().replacements, 0);
        assert_eq!(store.counts().rows_written, 0);
    }
}
