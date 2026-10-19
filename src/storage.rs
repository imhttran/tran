//! Storage foundation: schema contract, provenance metadata, and the
//! embedding-dimension guard that runs before any write.
//!
//! This module ports the schema contract encoded by the SQL migrations under
//! `migrations/` into Rust types. It is deliberately DB-free: it carries no
//! database client and no connection. The write boundary is represented as the
//! [`DocumentStore`] trait, which the future database client will implement.
//!
//! Field provenance (migration file defining the column):
//!
//! | Field            | Column          | Migration                       |
//! |------------------|-----------------|---------------------------------|
//! | `content`        | `content`       | `001_init.sql`                  |
//! | `source`         | `source`        | `001_init.sql`                  |
//! | `section`        | `section`       | `001_init.sql`                  |
//! | `chunk_index`    | `chunk_index`   | `001_init.sql`                  |
//! | `embedding`      | `embedding`     | `001_init.sql`                  |
//! | `embedding_dim`  | `embedding_dim` | `002_ingestion_metadata.sql`    |
//! | `embed_model`    | `embed_model`   | `002_ingestion_metadata.sql`    |
//! | `content_hash`   | `content_hash`  | `002_ingestion_metadata.sql`    |
//! | `chunker_config` | `chunker_config`| `002_ingestion_metadata.sql`    |
//! | `ingested_at`    | `ingested_at`   | `002_ingestion_metadata.sql`    |
//! | `language`       | `language`      | `003_add_language.sql`          |
//! | `page`           | `page`          | `004_page.sql`                  |
//! | `corpus`         | `corpus`        | `005_add_corpus.sql`            |
//!
//! `id` is the `bigserial PRIMARY KEY` from `001_init.sql`; it is assigned by
//! the database and is therefore absent from the write-side type
//! ([`StoredChunk`]).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The embedding dimension assumed by `migrations/001_init.sql`, which declares
/// `embedding vector(768)`.
pub const EMBEDDING_DIM: usize = 768;

/// Errors raised by the storage layer before any write is attempted.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("embedding dimension mismatch: expected {expected}, got {actual}")]
pub struct DimensionMismatchError {
    /// The dimension declared by the schema / configuration.
    pub expected: usize,
    /// The dimension of the supplied embedding.
    pub actual: usize,
}

/// A row to be written to the `documents` table.
///
/// The field names mirror the SQL column contract above. `page` is `None` for
/// page-less sources (Markdown, plain text); `language` and `corpus` are
/// optional metadata, matching the nullable columns added by migrations 003
/// and 005.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredChunk {
    /// `content` (`001_init.sql`): the chunk body.
    pub content: String,
    /// `source` (`001_init.sql`): originating document source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `section` (`001_init.sql`): section within the source document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// `chunk_index` (`001_init.sql`): document-global chunk number.
    pub chunk_index: i32,
    /// `language` (`003_add_language.sql`): BCP-47 language tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// `page` (`004_page.sql`): 1-based source page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<i32>,
    /// `corpus` (`005_add_corpus.sql`): trusted corpus identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corpus: Option<String>,
    /// `content_hash` (`002_ingestion_metadata.sql`): fingerprint of the file
    /// bytes combined with the chunker configuration.
    pub content_hash: String,
    /// `embed_model` (`002_ingestion_metadata.sql`): model that produced the
    /// embedding.
    pub embed_model: String,
    /// `embedding_dim` (`002_ingestion_metadata.sql`): width of the embedding.
    pub embedding_dim: i32,
    /// `chunker_config` (`002_ingestion_metadata.sql`): canonical serialization
    /// of the chunker settings.
    pub chunker_config: String,
    /// `ingested_at` (`002_ingestion_metadata.sql`): when the chunk was last
    /// written, as an RFC 3339 timestamp.
    pub ingested_at: String,
    /// `embedding` (`001_init.sql`): the embedding vector.
    pub embedding: Vec<f64>,
}

impl StoredChunk {
    /// Returns the number of dimensions in the stored embedding.
    pub fn embedding_dim(&self) -> usize {
        self.embedding.len()
    }
}

/// Validates the supplied embedding dimension against `expected`.
///
/// Call this before constructing any [`StoredChunk`] destined for the store.
/// A mismatch returns [`DimensionMismatchError`] and the caller must not enter
/// the write path.
///
/// The guard is deterministic and does not touch the network or a database.
pub fn validate_embedding_dim(
    expected: usize,
    supplied: usize,
) -> Result<(), DimensionMismatchError> {
    if expected != supplied {
        return Err(DimensionMismatchError {
            expected,
            actual: supplied,
        });
    }
    Ok(())
}

/// The storage write boundary.
///
/// The future database client implements this trait. The foundation stage
/// defines the contract but provides no implementation, so no database client
/// dependency is required.
pub trait DocumentStore {
    /// The error type returned by [`DocumentStore::write_chunk`].
    type Error;

    /// Validates the embedding dimension and then writes `chunk`.
    ///
    /// Implementations must call [`validate_embedding_dim`] before performing
    /// any effect. A mismatch returns an error and no write occurs.
    fn write_chunk(&self, chunk: &StoredChunk) -> Result<(), Self::Error>;
}

/// Validates the embedding dimension of `chunk` against `expected` and, only
/// when the dimension matches, forwards the chunk to `store`.
///
/// This is the fail-fast entry point: a dimension mismatch returns a
/// [`DimensionMismatchError`] and `store` is never called, so no write path is
/// entered.
pub fn write_validated<S, E>(
    store: &S,
    expected_dim: usize,
    chunk: &StoredChunk,
) -> Result<Result<(), E>, DimensionMismatchError>
where
    S: DocumentStore<Error = E>,
{
    validate_embedding_dim(expected_dim, chunk.embedding_dim())?;
    Ok(store.write_chunk(chunk))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn chunk_with_dim(dim: usize, page: Option<i32>) -> StoredChunk {
        StoredChunk {
            content: "body".to_string(),
            source: Some("doc.md".to_string()),
            section: Some("Intro".to_string()),
            chunk_index: 0,
            language: Some("english".to_string()),
            page,
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
    fn matching_dimension_passes() {
        assert_eq!(validate_embedding_dim(768, 768), Ok(()));
    }

    #[test]
    fn mismatched_dimension_names_expected_and_actual() {
        let err = validate_embedding_dim(768, 3).unwrap_err();
        assert_eq!(err.expected, 768);
        assert_eq!(err.actual, 3);
        assert!(err.to_string().contains("expected 768"));
        assert!(err.to_string().contains("got 3"));
    }

    /// A store that records whether its write path was entered.
    struct RecordingStore {
        written: Cell<u32>,
    }

    impl DocumentStore for RecordingStore {
        type Error = ();

        fn write_chunk(&self, _chunk: &StoredChunk) -> Result<(), Self::Error> {
            self.written.set(self.written.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn mismatch_fails_before_the_write_path_is_entered() {
        let store = RecordingStore {
            written: Cell::new(0),
        };
        let chunk = chunk_with_dim(3, None);

        let result = write_validated(&store, EMBEDDING_DIM, &chunk);

        assert!(result.is_err());
        assert_eq!(
            store.written.get(),
            0,
            "the write path must not be entered on a dimension mismatch"
        );
    }

    #[test]
    fn match_reaches_the_write_path() {
        let store = RecordingStore {
            written: Cell::new(0),
        };
        let chunk = chunk_with_dim(EMBEDDING_DIM, None);

        write_validated(&store, EMBEDDING_DIM, &chunk)
            .unwrap()
            .unwrap();

        assert_eq!(store.written.get(), 1);
    }

    #[test]
    fn provenance_language_page_and_corpus_match_the_migration_contract() {
        // `ingested_at` maps to a nullable-with-default column; the rest of the
        // provenance fields are required (`002_ingestion_metadata.sql`).
        let chunk = chunk_with_dim(EMBEDDING_DIM, Some(4));

        assert_eq!(chunk.source.as_deref(), Some("doc.md"));
        assert_eq!(chunk.section.as_deref(), Some("Intro"));
        assert_eq!(chunk.chunk_index, 0);
        assert_eq!(chunk.language.as_deref(), Some("english"));
        assert_eq!(chunk.page, Some(4));
        assert_eq!(chunk.corpus.as_deref(), Some("trusted"));
        assert_eq!(chunk.embed_model, "nomic-embed-text");
        assert_eq!(chunk.embedding_dim, EMBEDDING_DIM as i32);
        assert_eq!(chunk.embedding_dim(), EMBEDDING_DIM);
    }

    #[test]
    fn page_less_sources_read_as_none() {
        let chunk = chunk_with_dim(EMBEDDING_DIM, None);
        assert_eq!(chunk.page, None);
    }

    #[test]
    fn optional_metadata_round_trips_without_null_fields() {
        let mut chunk = chunk_with_dim(EMBEDDING_DIM, None);
        // The helper populates optional metadata; this test covers the
        // absent-optional case, so clear every optional field.
        chunk.source = None;
        chunk.section = None;
        chunk.language = None;
        chunk.corpus = None;
        let encoded = serde_json::to_string(&chunk).unwrap();

        assert!(!encoded.contains("\"page\""));
        assert!(!encoded.contains("\"corpus\""));
        assert!(!encoded.contains("\"source\""));
        assert!(!encoded.contains("\"section\""));
        assert!(!encoded.contains("\"language\""));

        let decoded: StoredChunk = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, chunk);
    }
}
