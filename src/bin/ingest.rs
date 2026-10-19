//! Production `ingest` entry point.
//!
//! This binary is thin: it loads [`Config`], builds the real adapters
//! ([`OllamaEmbedder`] over the `reqwest`-backed [`HttpTransport`], the
//! [`PgStore`] as both the fingerprint read port and the transactional
//! source-replacement port), constructs the RUST-005E [`IngestService`], and
//! calls it once per source. There is no second pipeline here: the service
//! delegates to the single [`IngestPipeline`] implementation.
//!
//! Usage: `ingest <source>` where the extension selects the loader: `.pdf`
//! (text layer), `.md`/`.markdown` (section parser), or `.txt` (one plain-text
//! section).
//!
//! `source` (the stored identity) defaults to the file name of the CLI path;
//! override it with `RAG_SOURCE`. The corpus identity defaults to `trusted` and
//! can be overridden with `RAG_CORPUS`. The content hash is computed from the
//! file bytes plus the configured chunker (`CHUNK_SIZE`/`CHUNK_OVERLAP`) through
//! [`fingerprint`].
//!
//! Exit status is non-zero when the configuration is invalid, when the source
//! is rejected (for example an incomplete or fully scanned PDF), or when a
//! fatal loader/other error occurs. In every non-success case the previously
//! stored rows are preserved, because the guard runs before any embedding call
//! or store write.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use rag_foundation::config::Config;
use rag_foundation::embedding::OllamaEmbedder;
use rag_foundation::fingerprint::{fingerprint, ChunkerConfig};
use rag_foundation::ingest::IngestRejection;
use rag_foundation::ollama::{HttpTransport, OllamaClient};
use rag_foundation::pipeline::{IngestOutcome, PipelineConfig};
use rag_foundation::service::IngestService;
use rag_foundation::storage_pg::PgStore;

/// Default corpus identity written with each chunk.
const DEFAULT_CORPUS: &str = "trusted";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("ingest: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let path = source_path()?;

    let config = Config::load().map_err(|err| format!("configuration error: {err}"))?;

    let bytes =
        std::fs::read(&path).map_err(|err| format!("failed to read {}: {err}", path.display()))?;

    let chunker = ChunkerConfig::new(config.chunk_size as usize, config.chunk_overlap as usize);
    let content_hash = fingerprint(&bytes, &chunker);
    let source = source_name(&path);
    let corpus = env::var("RAG_CORPUS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CORPUS.to_string());

    // Real embedding adapter: `reqwest`-backed transport, configured URL,
    // model, dimension, and request timeout.
    let transport = Arc::new(HttpTransport::new(config.request_timeout));
    let client = OllamaClient::new(config.ollama_url.clone(), transport, config.request_timeout);
    let embedder = OllamaEmbedder::new(client, config.embed_model.clone())
        .with_embed_dim(config.embed_dim as usize);

    // Real storage adapter: the same `PgStore` serves the fingerprint read port
    // and the transactional source-replacement port.
    let store = PgStore::connect(&config.database_url)
        .map_err(|err| format!("failed to build the Postgres store: {err}"))?;

    let pipeline_config = PipelineConfig::new(
        config.language.clone(),
        chunker,
        config.embed_model.clone(),
        corpus,
    );

    // Exactly one pipeline: the service composes `IngestPipeline`.
    let service = IngestService::new(&embedder, &store, &store, pipeline_config);

    let outcome = service
        .ingest_source(&path, &source, &content_hash)
        .map_err(|err| format!("ingest failed: {err}"))?;

    Ok(report(&source, &outcome))
}

/// Reads the source path from the single positional CLI argument.
fn source_path() -> Result<PathBuf, String> {
    let mut args = env::args().skip(1);
    match (args.next(), args.next()) {
        (Some(path), None) => Ok(PathBuf::from(path)),
        (None, _) => Err("usage: ingest <source> (PDF, Markdown, or plain text)".to_string()),
        (Some(_), Some(_)) => Err("usage: ingest <source> (exactly one argument)".to_string()),
    }
}

/// Derives the stored `source` identity from the path, unless `RAG_SOURCE`
/// overrides it.
fn source_name(path: &Path) -> String {
    env::var("RAG_SOURCE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        })
}

/// Prints the outcome and returns the process exit code.
fn report(source: &str, outcome: &IngestOutcome) -> ExitCode {
    match outcome {
        IngestOutcome::Replaced { chunks, .. } => {
            println!("ingested {source}: {chunks} chunk(s) replaced");
            ExitCode::SUCCESS
        }
        IngestOutcome::Unchanged { rows, .. } => {
            println!("unchanged {source}: {rows} stored row(s) left in place");
            ExitCode::SUCCESS
        }
        IngestOutcome::Rejected(rejection) => {
            match rejection {
                IngestRejection::Incomplete { pages } => {
                    eprintln!(
                        "rejected {source}: incomplete text layer on page(s) {}",
                        format_pages(pages)
                    );
                }
                IngestRejection::Loader(err) => {
                    eprintln!("rejected {source}: {err}");
                }
                IngestRejection::NoChunks { .. } => {
                    eprintln!("rejected {source}: no sections found");
                }
            }
            eprintln!("stored data for {source} was preserved");
            ExitCode::FAILURE
        }
    }
}

/// Formats affected page numbers as a comma-separated list.
fn format_pages(pages: &[u32]) -> String {
    pages
        .iter()
        .map(|page| page.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
