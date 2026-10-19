//! Source fingerprint and unchanged-source detection.
//!
//! The storage contract ([`crate::storage::StoredChunk::content_hash`]) defines
//! `content_hash` as the "fingerprint of the file bytes combined with the
//! chunker configuration". This module ports that meaning to a deterministic
//! function ([`fingerprint`]) plus the unchanged-source decision the ingest
//! caller needs ([`UnchangedSource`]), so a repeated ingestion of an unchanged
//! source skips re-embedding and replacement, matching the Go baseline.
//!
//! The module is DB-free: the decision reads existing rows through the small
//! [`FingerprintReader`] port, which [`crate::storage_pg::PgStore`] implements
//! over its existing `find_by_content_hash` read path.

use crate::storage_pg::{PgStore, PgStoreError, StoredChunkRow};

/// The canonical chunker configuration that qualifies a fingerprint.
///
/// Serializes to a stable `size=<n>,overlap=<n>` form, matching the
/// `chunker_config` column contents documented in [`crate::storage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkerConfig {
    /// Maximum chunk length in words (`CHUNK_SIZE`).
    pub size: usize,
    /// Repeated words between consecutive chunks (`CHUNK_OVERLAP`).
    pub overlap: usize,
}

impl ChunkerConfig {
    /// Builds a chunker configuration from the word size and overlap.
    pub fn new(size: usize, overlap: usize) -> Self {
        Self { size, overlap }
    }

    /// Returns the canonical serialization stored in `chunker_config`.
    pub fn canonical(&self) -> String {
        format!("size={},overlap={}", self.size, self.overlap)
    }
}

/// Computes a deterministic fingerprint of `bytes` combined with `config`.
///
/// The same bytes and configuration always produce the same string across
/// runs. Changing the bytes or any chunker setting changes the fingerprint.
/// The result does not depend on any chunk order: it hashes the raw source
/// bytes and the canonical configuration only.
pub fn fingerprint(bytes: &[u8], config: &ChunkerConfig) -> String {
    let mut hash = Fnv1a::new();
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
    let canonical = config.canonical();
    hash.update(canonical.as_bytes());
    format!("{:016x}", hash.finish())
}

/// A 64-bit FNV-1a hasher: deterministic, dependency-free, and stable across
/// platforms and runs.
struct Fnv1a {
    state: u64,
}

impl Fnv1a {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self {
            state: Self::OFFSET_BASIS,
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    fn finish(&self) -> u64 {
        self.state
    }
}

/// The read port the unchanged-source decision depends on.
///
/// Implemented for [`PgStore`] over `find_by_content_hash`. Tests substitute a
/// recording fake so the decision runs without a database.
pub trait FingerprintReader {
    /// The error type returned when the read fails.
    type Error;

    /// Returns every stored row for `source` whose `content_hash` is
    /// `content_hash`.
    fn rows_for_hash(
        &self,
        content_hash: &str,
        source: &str,
    ) -> Result<Vec<StoredChunkRow>, Self::Error>;
}

impl FingerprintReader for PgStore {
    type Error = PgStoreError;

    fn rows_for_hash(
        &self,
        content_hash: &str,
        source: &str,
    ) -> Result<Vec<StoredChunkRow>, PgStoreError> {
        self.find_by_content_hash(content_hash, Some(source))
    }
}

/// The unchanged-source decision for one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnchangedSource {
    /// No stored row carries the computed fingerprint, so the source changed.
    Changed,
    /// Stored rows already carry the computed fingerprint for this source, so
    /// the source is unchanged and must be skipped.
    Unchanged { rows: usize },
}

impl UnchangedSource {
    /// Returns `true` when the source is unchanged and ingestion must skip.
    pub fn is_unchanged(&self) -> bool {
        matches!(self, UnchangedSource::Unchanged { .. })
    }
}

/// Decides whether `source` is unchanged against `content_hash`.
///
/// Returns [`UnchangedSource::Unchanged`] when at least one stored row for the
/// same source carries the computed fingerprint, and
/// [`UnchangedSource::Changed`] when no such row exists (including the first
/// ingest, where the store is empty for the source).
pub fn unchanged_source<R>(
    reader: &R,
    source: &str,
    content_hash: &str,
) -> Result<UnchangedSource, R::Error>
where
    R: FingerprintReader,
{
    let rows = reader.rows_for_hash(content_hash, source)?;
    if rows.is_empty() {
        Ok(UnchangedSource::Changed)
    } else {
        Ok(UnchangedSource::Unchanged { rows: rows.len() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StoredChunk;
    use std::cell::RefCell;

    fn config() -> ChunkerConfig {
        ChunkerConfig::new(50, 20)
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let bytes = b"# Title\n\nbody text";
        assert_eq!(fingerprint(bytes, &config()), fingerprint(bytes, &config()));
    }

    #[test]
    fn changed_bytes_change_the_fingerprint() {
        assert_ne!(
            fingerprint(b"one", &config()),
            fingerprint(b"two", &config())
        );
    }

    #[test]
    fn changed_chunker_config_changes_the_fingerprint() {
        let bytes = b"same bytes";
        assert_ne!(
            fingerprint(bytes, &ChunkerConfig::new(50, 20)),
            fingerprint(bytes, &ChunkerConfig::new(50, 10))
        );
        assert_ne!(
            fingerprint(bytes, &ChunkerConfig::new(50, 20)),
            fingerprint(bytes, &ChunkerConfig::new(40, 20))
        );
    }

    #[test]
    fn canonical_config_is_stable() {
        assert_eq!(ChunkerConfig::new(50, 20).canonical(), "size=50,overlap=20");
    }

    fn row(hash: &str, source: &str) -> StoredChunkRow {
        StoredChunkRow {
            id: 1,
            chunk: StoredChunk {
                content: "body".to_string(),
                source: Some(source.to_string()),
                section: Some("Intro".to_string()),
                chunk_index: 0,
                language: Some("english".to_string()),
                page: None,
                corpus: Some("trusted".to_string()),
                content_hash: hash.to_string(),
                embed_model: "nomic-embed-text".to_string(),
                embedding_dim: 768,
                chunker_config: config().canonical(),
                ingested_at: "2024-01-01T00:00:00Z".to_string(),
                embedding: vec![0.0; 768],
            },
        }
    }

    /// A read port backed by an in-memory row list that records its queries.
    struct FakeReader {
        rows: Vec<StoredChunkRow>,
        queries: RefCell<Vec<(String, String)>>,
    }

    impl FingerprintReader for FakeReader {
        type Error = String;

        fn rows_for_hash(
            &self,
            content_hash: &str,
            source: &str,
        ) -> Result<Vec<StoredChunkRow>, String> {
            self.queries
                .borrow_mut()
                .push((content_hash.to_string(), source.to_string()));
            Ok(self
                .rows
                .iter()
                .filter(|r| {
                    r.chunk.content_hash == content_hash
                        && r.chunk.source.as_deref() == Some(source)
                })
                .cloned()
                .collect())
        }
    }

    #[test]
    fn empty_store_reads_as_changed() {
        let reader = FakeReader {
            rows: Vec::new(),
            queries: RefCell::new(Vec::new()),
        };
        assert_eq!(
            unchanged_source(&reader, "doc.md", "hash").unwrap(),
            UnchangedSource::Changed
        );
        assert_eq!(reader.queries.borrow().len(), 1);
    }

    #[test]
    fn matching_hash_reads_as_unchanged() {
        let reader = FakeReader {
            rows: vec![row("hash", "doc.md"), row("hash", "doc.md")],
            queries: RefCell::new(Vec::new()),
        };
        assert_eq!(
            unchanged_source(&reader, "doc.md", "hash").unwrap(),
            UnchangedSource::Unchanged { rows: 2 }
        );
    }

    #[test]
    fn matching_hash_for_another_source_reads_as_changed() {
        let reader = FakeReader {
            rows: vec![row("hash", "other.md")],
            queries: RefCell::new(Vec::new()),
        };
        let decision = unchanged_source(&reader, "doc.md", "hash").unwrap();
        assert!(!decision.is_unchanged());
    }

    #[test]
    fn repeated_ingest_matches_unchanged_source_behavior() {
        let bytes = b"# Title\n\nbody";
        let fp = fingerprint(bytes, &config());
        let empty = FakeReader {
            rows: Vec::new(),
            queries: RefCell::new(Vec::new()),
        };
        assert!(!unchanged_source(&empty, "doc.md", &fp)
            .unwrap()
            .is_unchanged());

        let stored = FakeReader {
            rows: vec![row(&fp, "doc.md")],
            queries: RefCell::new(Vec::new()),
        };
        assert!(unchanged_source(&stored, "doc.md", &fp)
            .unwrap()
            .is_unchanged());
    }
}
