//! rag-foundation: document processing port of the Go baseline.
//!
//! Provides the domain types ([`document`]), configuration ([`config`]), the
//! document-processing pipeline: Markdown/plain-text loaders, `## ` section
//! parsing, chunking with overlap, and PDF text-layer loading with 1-based page
//! provenance, the pre-effect ingest guard ([`ingest`]), the source fingerprint
//! and unchanged-source decision ([`fingerprint`]), transactional source
//! replacement ([`transaction`]), the ingest pipeline caller ([`pipeline`]),
//! and the provider integration: a shared Ollama HTTP client ([`ollama`]),
//! embedding ([`embedding`]) and generation ([`generation`]) request ports, and
//! provider registration/resolution ([`provider`]), and the evaluation-only
//! bilingual experiment harness ([`bilingual`]).

pub mod answer;
pub mod api;
pub mod bilingual;
pub mod chunker;
pub mod config;
pub mod document;
pub mod embedding;
pub mod evaluation;
pub mod fingerprint;
pub mod generation;
pub mod ingest;
pub mod loader;
pub mod ollama;
pub mod pipeline;
pub mod provider;
pub mod query_service;
pub mod retrieval;
pub mod retrieval_pg;
pub mod section;
pub mod service;
pub mod storage;
pub mod storage_pg;
pub mod transaction;
