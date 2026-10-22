//! Retrieval layer: vector search, full-text search, rank fusion,
//! deduplication, and section expansion.
//!
//! Ported from the Go baseline under `internal/retrieval/`. The search
//! operations read from an in-memory candidate source that mirrors the
//! storage boundary; the fusion and dedup steps are pure functions
//! over candidate lists so they are deterministic and testable with fixed
//! vectors.

pub mod dedup;
pub mod expand;
pub mod fts;
pub mod fusion;
pub mod vector;

use crate::document::{Chunk, RetrievedDocument};
use serde::{Deserialize, Serialize};

/// Authoritative provenance for one retrieved chunk.
///
/// The retrieval layer observes this from `approved_documents` at query time.
/// It is what a citation must resolve to; a human-readable filename alone is not
/// sufficient source identity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkProvenance {
    /// `documents.id` of the approved chunk.
    pub chunk_id: Option<i64>,
    /// `document_version.version_id` of the approved version.
    pub document_version_id: Option<i64>,
    /// Corpus the chunk was published in.
    pub corpus: String,
    /// Content fingerprint bound to the approval.
    pub content_hash: String,
}

/// A retrieval candidate shared by every search stage.
///
/// `id` is a stable identity (`source::section::chunk_index`) used by
/// fusion, filter and dedup so the same chunk from different search paths
/// collapses to one entry. `score` is the path-local score; fusion rewrites
/// it with the fused score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Stable identity for dedup and fusion.
    pub id: String,
    /// The retrieved chunk with provenance.
    pub document: RetrievedDocument,
    /// Path-local score (cosine similarity, FTS rank, fused score, ...).
    pub score: f64,
    /// Authoritative chunk and version identity, when the candidate came from
    /// the approved database snapshot.
    #[serde(default)]
    pub provenance: ChunkProvenance,
}

impl Candidate {
    /// Builds a candidate from a chunk and a score, deriving `id` from
    /// provenance so equality across search paths holds.
    pub fn new(chunk: Chunk, score: f64) -> Self {
        Self::with_provenance(chunk, score, ChunkProvenance::default())
    }

    /// Builds a candidate that carries the approved database identity.
    pub fn with_provenance(chunk: Chunk, score: f64, provenance: ChunkProvenance) -> Self {
        let id = candidate_id(&chunk);
        Self {
            id,
            document: RetrievedDocument::new(chunk, score),
            score,
            provenance,
        }
    }
}

/// Returns the stable identity of `chunk`: `source::section::chunk_index`.
pub fn candidate_id(chunk: &Chunk) -> String {
    format!("{}::{}::{}", chunk.source, chunk.section, chunk.chunk_index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(source: &str, section: &str, index: u32) -> Chunk {
        Chunk {
            source: source.to_string(),
            section: section.to_string(),
            language: "english".to_string(),
            chunk_index: index,
            content: format!("{section} body"),
            page: None,
        }
    }

    #[test]
    fn candidate_id_is_stable_across_paths() {
        let a = Candidate::new(chunk("doc.md", "Intro", 1), 0.9);
        let b = Candidate::new(chunk("doc.md", "Intro", 1), 0.4);
        assert_eq!(a.id, b.id);
        assert_eq!(a.id, "doc.md::Intro::1");
    }
}
