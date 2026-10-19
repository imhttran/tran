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

use crate::answer::service::{Answer, AnswerService, RetrievalSource};
use crate::config::Config;
use crate::document::Chunk;
use crate::embedding::{Embedder, OllamaEmbedder};
use crate::generation::OllamaGenerator;
use crate::ollama::{HttpTransport, OllamaClient};
use crate::retrieval::{dedup, expand, Candidate};
use crate::retrieval_pg;
use crate::storage_pg::PgStore;

/// Default character budget for the model context, matching `bin/query`.
const DEFAULT_CONTEXT_BUDGET: usize = 4000;

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
    fn retrieve(&self, question: &str) -> Result<Vec<Candidate>, String> {
        let rows = self
            .store
            .rows_for_corpus(self.corpus)
            .map_err(|err| format!("load corpus {:?}: {err}", self.corpus))?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let chunks: Vec<Chunk> = rows.iter().map(stored_chunk).collect();

        let query = self
            .embedder
            .embed(question)
            .map_err(|err| format!("embed question: {err}"))?;

        let vector_hits: Vec<_> = self
            .store
            .vector_search(self.corpus, &query, self.candidate_k)
            .map_err(|err| format!("vector search: {err}"))?
            .into_iter()
            .filter(|hit| hit.similarity >= self.min_similarity)
            .collect();
        let keyword_hits = self
            .store
            .keyword_search(self.corpus, question, self.candidate_k, self.language)
            .map_err(|err| format!("keyword search: {err}"))?;
        let mut candidates = retrieval_pg::fuse(&[vector_hits, keyword_hits], self.candidate_k);
        candidates = dedup::dedup(candidates);
        candidates = expand::expand(&candidates, &chunks, self.expand_limit);
        candidates.truncate(self.final_k);
        Ok(candidates)
    }
}

fn stored_chunk(row: &crate::storage_pg::StoredChunkRow) -> Chunk {
    Chunk {
        source: row.chunk.source.clone().unwrap_or_default(),
        section: row.chunk.section.clone().unwrap_or_default(),
        language: row.chunk.language.clone().unwrap_or_default(),
        chunk_index: row.chunk.chunk_index.max(0) as u32,
        content: row.chunk.content.clone(),
        page: row.chunk.page.and_then(|page| u32::try_from(page).ok()),
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
}

impl QueryService {
    /// Builds the service from the loaded configuration and a shared store.
    pub fn from_config(
        config: &Config,
        store: Arc<PgStore>,
        corpus: impl Into<String>,
    ) -> Result<Self, String> {
        let embed_transport = Arc::new(HttpTransport::new(config.request_timeout));
        let embed_client = OllamaClient::new(
            config.ollama_url.clone(),
            embed_transport,
            config.request_timeout,
        );
        let embedder = OllamaEmbedder::new(embed_client, config.embed_model.clone())
            .with_embed_dim(config.embed_dim as usize);

        let gen_transport = Arc::new(HttpTransport::new(config.request_timeout));
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
            corpus: corpus.into(),
            min_similarity: config.min_similarity,
            final_k: config.final_k.max(1) as usize,
            candidate_k: config.top_k.max(1) as usize,
            expand_limit: config.expand_limit.max(0) as u32,
            language: config.language.clone(),
            budget,
            citation_validation: config.rag_citation_validation,
        })
    }

    /// Answers `question` through the shared retrieval, generation, and citation
    /// validation path. Insufficient evidence returns the baseline refusal.
    pub fn answer(&self, question: &str) -> Result<Answer, String> {
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
            .with_citation_validation(self.citation_validation);
        service.answer(question).map_err(|err| err.to_string())
    }

    /// The corpus this service reads.
    pub fn corpus(&self) -> &str {
        &self.corpus
    }
}
