//! Shared production query service for the CLI and the Axum API.
//!
//! The CLI (`query`) and the API must answer through the same retrieval and
//! citation path. This module owns that path once: it wires the PostgreSQL
//! corpus reader, the Ollama embedder/generator, the shared retrieval stages,
//! and [`AnswerService`]. Callers only supply a question.
//!
//! The retrieval adapter is synchronous because the database and Ollama clients
//! are synchronous. Async callers run [`QueryService::answer`] on a blocking
//! task instead of blocking an executor thread.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use thiserror::Error;

use crate::answer::context_budget::{ContextPassage, PassageIdentity};
use crate::answer::service::{
    Answer, AnswerError, AnswerService, PublicationError, PublicationGuard, RetrievalSource,
};
use crate::config::Config;
use crate::corpus::Corpus;
use crate::document::Chunk;
use crate::embedding::{Embedder, OllamaEmbedder};
use crate::generation::OllamaGenerator;
use crate::ollama::{HttpTransport, OllamaClient};
use crate::retrieval::{candidate_id, dedup, expand, Candidate, ChunkProvenance};
use crate::retrieval_pg;
use crate::storage_pg::{PgStore, PublicationRow};

/// Default character budget for the model context, matching `bin/query`.
const DEFAULT_CONTEXT_BUDGET: usize = 4000;
/// The number of question suggestions the Ask page requests.
pub const SUGGESTION_LIMIT: usize = 6;

/// Errors raised while building or running the shared query service.
#[derive(Debug, Error)]
pub enum QueryServiceError {
    /// The corpus identity is invalid.
    #[error(transparent)]
    Corpus(#[from] crate::corpus::CorpusError),
    /// The Ollama HTTP client could not be built.
    #[error(transparent)]
    Client(#[from] crate::ollama::OllamaError),
    /// Answering failed.
    #[error(transparent)]
    Answer(#[from] AnswerError),
}

/// Errors from one retrieval pass, with the stage that failed.
#[derive(Debug, Error)]
enum DbRetrievalError {
    #[error("load corpus {corpus:?}: {source}")]
    Corpus {
        corpus: String,
        #[source]
        source: crate::storage_pg::PgStoreError,
    },
    #[error("embed question: {0}")]
    Embedding(#[source] crate::embedding::EmbeddingError),
    #[error("vector search: {0}")]
    Vector(#[source] crate::storage_pg::PgStoreError),
    #[error("keyword search: {0}")]
    Keyword(#[source] crate::storage_pg::PgStoreError),
}

/// Retrieves ranked candidates for a question from the PostgreSQL corpus.
pub struct DbRetrieval<'a> {
    store: &'a PgStore,
    embedder: &'a OllamaEmbedder,
    corpus: &'a str,
    min_similarity: f64,
    final_k: usize,
    candidate_k: usize,
    expand_limit: u32,
    language: &'a str,
}

impl RetrievalSource for DbRetrieval<'_> {
    fn retrieve(
        &self,
        question: &str,
    ) -> Result<Vec<Candidate>, crate::answer::service::RetrievalError> {
        let snapshot = self
            .store
            .retrieval_snapshot(self.corpus)
            .map_err(|source| DbRetrievalError::Corpus {
                corpus: self.corpus.to_string(),
                source,
            })?;
        if snapshot.is_empty() {
            return Ok(Vec::new());
        }

        // The snapshot is both the expansion corpus and the authoritative
        // provenance source. A candidate that is not in it has no approved
        // identity and is dropped rather than answered.
        let chunks: Vec<Chunk> = snapshot.iter().map(publication_chunk).collect();
        let provenance: HashMap<String, ChunkProvenance> = snapshot
            .iter()
            .map(|row| {
                (
                    candidate_id(&publication_chunk(row)),
                    publication_provenance(row),
                )
            })
            .collect();

        let query = self
            .embedder
            .embed(question)
            .map_err(DbRetrievalError::Embedding)?;

        let vector_hits: Vec<_> = self
            .store
            .vector_search(self.corpus, &query, self.candidate_k)
            .map_err(DbRetrievalError::Vector)?
            .into_iter()
            .filter(|hit| hit.similarity >= self.min_similarity)
            .collect();
        let keyword_hits = self
            .store
            .keyword_search(self.corpus, question, self.candidate_k, self.language)
            .map_err(DbRetrievalError::Keyword)?;
        let fused = retrieval_pg::fuse(&[vector_hits, keyword_hits], self.candidate_k);
        // Only a candidate with an approved snapshot identity may consume a
        // `final_k` slot. Filtering here (not after expansion) keeps the bound
        // counting real, published sections even if a row was withdrawn between
        // the snapshot read and the searches.
        let fused: Vec<Candidate> = fused
            .into_iter()
            .filter(|candidate| provenance.contains_key(&candidate.id))
            .collect();
        let mut candidates = finalize_candidates(fused, &chunks, self.final_k, self.expand_limit);
        for candidate in &mut candidates {
            if let Some(found) = provenance.get(&candidate.id) {
                candidate.provenance = found.clone();
            }
        }
        candidates.retain(|candidate| candidate.provenance.chunk_id.is_some());
        Ok(candidates)
    }
}

/// Selects the top fused sections, then expands those sections to nearby chunks.
///
/// `final_k` counts sections, not chunks, and only the sections that survive
/// selection are expanded. Applying the bound after expansion (the old order)
/// let an earlier same-section chunk crowd the actual match out of the context
/// before context budgeting ran. The context budget remains the only length
/// bound on the answer prompt.
pub fn finalize_candidates(
    candidates: Vec<Candidate>,
    chunks: &[Chunk],
    final_k: usize,
    expand_limit: u32,
) -> Vec<Candidate> {
    let candidates = dedup::dedup(candidates);
    let mut candidates = dedup::dedup_sections(candidates);
    candidates.truncate(final_k);
    expand::expand(&candidates, chunks, expand_limit)
}

/// Builds the expansion chunk for one approved snapshot row.
fn publication_chunk(row: &PublicationRow) -> Chunk {
    Chunk {
        source: row.source.clone(),
        section: row.section.clone(),
        language: row.language.clone(),
        chunk_index: row.chunk_index.max(0) as u32,
        content: row.content.clone(),
        page: row.page.and_then(|page| u32::try_from(page).ok()),
    }
}

/// Builds the authoritative provenance for one approved snapshot row.
fn publication_provenance(row: &PublicationRow) -> ChunkProvenance {
    ChunkProvenance {
        chunk_id: Some(row.chunk_id),
        document_version_id: Some(row.document_version_id),
        corpus: row.corpus.clone(),
        content_hash: row.content_hash.clone(),
    }
}

/// Revalidates the answer's evidence against the live approved view.
struct StorePublicationGuard<'a> {
    store: &'a PgStore,
    corpus: &'a str,
}

impl PublicationGuard for StorePublicationGuard<'_> {
    fn revalidate(&self, passages: &[ContextPassage]) -> Result<bool, PublicationError> {
        let identities: Vec<&PassageIdentity> =
            passages.iter().map(|passage| &passage.identity).collect();
        self.store
            .revalidate_publication(self.corpus, &identities)
            .map_err(|error| PublicationError(error.to_string()))
    }
}

/// The shared production query service.
pub struct QueryService {
    store: Arc<PgStore>,
    embedder: OllamaEmbedder,
    generator: OllamaGenerator,
    corpus: String,
    min_similarity: f64,
    final_k: usize,
    candidate_k: usize,
    expand_limit: u32,
    language: String,
    budget: usize,
    citation_validation: bool,
    query_rewrite: bool,
    answerability_gate: bool,
}

impl QueryService {
    /// Builds the service from the loaded configuration and a shared store.
    pub fn from_config(
        config: &Config,
        store: Arc<PgStore>,
        corpus: impl Into<String>,
    ) -> Result<Self, QueryServiceError> {
        let corpus: String = corpus.into();
        let corpus = Corpus::new(&corpus)?;
        let embed_transport = Arc::new(HttpTransport::new(config.request_timeout)?);
        let embed_client = OllamaClient::new(
            config.ollama_url.clone(),
            embed_transport,
            config.request_timeout,
        );
        let embedder = OllamaEmbedder::new(embed_client, config.embed_model.clone())
            .with_embed_dim(config.embed_dim as usize);

        let gen_transport = Arc::new(HttpTransport::new(config.request_timeout)?);
        let gen_client = OllamaClient::new(
            config.ollama_url.clone(),
            gen_transport,
            config.request_timeout,
        );
        let generator = OllamaGenerator::new(gen_client, config.chat_model.clone());

        let budget = if config.context_budget > 0 {
            config.context_budget as usize
        } else {
            DEFAULT_CONTEXT_BUDGET
        };
        Ok(Self {
            store,
            embedder,
            generator,
            corpus: corpus.as_str().to_string(),
            min_similarity: config.min_similarity,
            final_k: config.final_k.max(1) as usize,
            candidate_k: config.top_k.max(1) as usize,
            expand_limit: config.expand_limit.max(0) as u32,
            language: config.language.clone(),
            budget,
            citation_validation: config.rag_citation_validation,
            query_rewrite: config.query_rewrite,
            answerability_gate: config.rag_answerability_gate,
        })
    }

    /// Answers `question` through the shared retrieval, generation, and citation
    /// validation path. Insufficient evidence returns the baseline refusal.
    pub fn answer(&self, question: &str) -> Result<Answer, QueryServiceError> {
        let retrieval = DbRetrieval {
            store: &self.store,
            embedder: &self.embedder,
            corpus: &self.corpus,
            min_similarity: self.min_similarity,
            final_k: self.final_k,
            candidate_k: self.candidate_k,
            expand_limit: self.expand_limit,
            language: &self.language,
        };
        let guard = StorePublicationGuard {
            store: &self.store,
            corpus: &self.corpus,
        };
        let service = AnswerService::new(&retrieval, &self.generator, self.budget)
            .with_citation_validation(self.citation_validation)
            .with_answerability_gate(self.answerability_gate)
            .with_publication_guard(&guard);
        let retrieval_query = if self.query_rewrite {
            crate::answer::query_rewrite::rewrite(&self.generator, question)
        } else {
            question.to_string()
        };
        Ok(service.answer_with_retrieval_query(question, &retrieval_query)?)
    }

    /// The corpus this service reads.
    pub fn corpus(&self) -> &str {
        &self.corpus
    }
}

/// Builds deterministic questions from approved archive topics.
///
/// The suggestions name only content that is already inside the publication
/// boundary. They never invent a fact or expose a passage body. The caller
/// supplies distinct section titles (or source names as fallbacks).
pub fn suggested_questions(topics: &[String], limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let mut seen = BTreeSet::new();
    let mut questions = Vec::new();
    for topic in topics {
        let topic = topic.trim();
        if topic.is_empty() {
            continue;
        }
        let question = format!("What does the archive say about \"{topic}\"?");
        if seen.insert(question.clone()) {
            questions.push(question);
            if questions.len() >= limit {
                break;
            }
        }
    }
    questions
}

#[cfg(test)]
mod tests {
    use super::{finalize_candidates, suggested_questions};
    use crate::document::Chunk;
    use crate::retrieval::Candidate;

    fn topic(value: &str) -> String {
        value.to_string()
    }

    fn chunk(section: &str, index: u32) -> Chunk {
        Chunk {
            source: "family.md".to_string(),
            section: section.to_string(),
            language: "english".to_string(),
            chunk_index: index,
            content: format!("{section} chunk {index}"),
            page: None,
        }
    }

    #[test]
    fn suggestions_are_derived_from_approved_topics() {
        let questions = suggested_questions(&[topic("Early years"), topic("Migration")], 6);
        assert_eq!(
            questions,
            vec![
                "What does the archive say about \"Early years\"?",
                "What does the archive say about \"Migration\"?",
            ]
        );
    }

    #[test]
    fn duplicates_and_blank_topics_are_removed() {
        let questions = suggested_questions(
            &[
                topic("family.md"),
                topic("Same title"),
                topic("Same title"),
                topic("  "),
            ],
            6,
        );
        assert_eq!(
            questions,
            vec![
                "What does the archive say about \"family.md\"?",
                "What does the archive say about \"Same title\"?",
            ]
        );
    }

    #[test]
    fn no_approved_topics_produce_no_suggestions() {
        assert!(suggested_questions(&[], 6).is_empty());
        assert!(suggested_questions(&[topic("")], 6).is_empty());
        assert!(suggested_questions(&[topic("Section")], 0).is_empty());
    }

    #[test]
    fn expansion_cannot_crowd_out_the_selected_section() {
        let mut chunks: Vec<Chunk> = (0..24)
            .map(|index| chunk("Editorial method", index))
            .collect();
        chunks.push(chunk("Target section", 24));
        let candidate = Candidate::new(chunk("Target section", 24), 1.0);
        let expanded = finalize_candidates(vec![candidate], &chunks, 1, 20);
        assert!(
            expanded
                .iter()
                .any(|candidate| candidate.document.chunk.section == "Target section"),
            "the matched section must survive expansion and final_k"
        );
    }

    #[test]
    fn final_k_counts_sections_before_expansion() {
        let chunks = vec![
            chunk("Section A", 0),
            chunk("Section A", 1),
            chunk("Section A", 2),
            chunk("Section B", 3),
        ];
        // Two chunks of section A outrank one chunk of section B.
        let candidates = vec![
            Candidate::new(chunk("Section A", 0), 0.9),
            Candidate::new(chunk("Section A", 1), 0.85),
            Candidate::new(chunk("Section B", 3), 0.8),
        ];
        let selected = finalize_candidates(candidates, &chunks, 1, 2);
        assert_eq!(selected[0].document.chunk.section, "Section A");
        assert!(
            selected
                .iter()
                .all(|candidate| candidate.document.chunk.section == "Section A"),
            "one section slot must not admit a second section"
        );
    }
}
