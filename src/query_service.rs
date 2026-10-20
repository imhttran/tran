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

use std::sync::Arc;

use thiserror::Error;

use crate::answer::service::{Answer, AnswerError, AnswerService, RetrievalSource};
use crate::config::Config;
use crate::corpus::Corpus;
use crate::embedding::{Embedder, OllamaEmbedder};
use crate::generation::OllamaGenerator;
use crate::ollama::{HttpTransport, OllamaClient};
use crate::retrieval::{dedup, expand, Candidate};
use crate::retrieval_pg;
use crate::storage_pg::PgStore;

/// Default character budget for the model context, matching `bin/query`.
const DEFAULT_CONTEXT_BUDGET: usize = 4000;

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
        let chunks = self
            .store
            .chunks_for_corpus(self.corpus)
            .map_err(|source| DbRetrievalError::Corpus {
                corpus: self.corpus.to_string(),
                source,
            })?;
        if chunks.is_empty() {
            return Ok(Vec::new());
        }

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
        let mut candidates = retrieval_pg::fuse(&[vector_hits, keyword_hits], self.candidate_k);
        candidates = dedup::dedup(candidates);
        candidates = expand::expand(&candidates, &chunks, self.expand_limit);
        candidates.truncate(self.final_k);
        Ok(candidates)
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
        let service = AnswerService::new(&retrieval, &self.generator, self.budget)
            .with_citation_validation(self.citation_validation)
            .with_answerability_gate(self.answerability_gate);
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
