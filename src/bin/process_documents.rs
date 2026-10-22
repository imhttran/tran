//! Durable document processing worker.
//!
//! Runs one batch of queued document jobs, or watches continuously with
//! `--watch`. The worker uses the same ingestion service and private original
//! storage as the API; it never commits a version without the approval and
//! stale-work checks in `document_processing`.

use std::sync::Arc;
use std::time::Duration;

use rag_foundation::config::Config;
use rag_foundation::document_lifecycle::DocumentStorage;
use rag_foundation::document_processing::run_due_jobs;
use rag_foundation::embedding::OllamaEmbedder;
use rag_foundation::ollama::{HttpTransport, OllamaClient};
use rag_foundation::storage_pg::PgStore;

fn main() {
    let watch = std::env::args().any(|arg| arg == "--watch");
    loop {
        match run_once() {
            Ok(outcomes) => {
                if outcomes.is_empty() {
                    println!("no queued document jobs");
                }
                for outcome in outcomes {
                    println!(
                        "job {} version {}: {}{}",
                        outcome.job_id,
                        outcome.version_id,
                        outcome.status,
                        outcome
                            .error
                            .as_deref()
                            .map(|error| format!(" ({error})"))
                            .unwrap_or_default()
                    );
                }
            }
            Err(message) => {
                eprintln!("process-documents: {message}");
                std::process::exit(1);
            }
        }
        if !watch {
            return;
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

fn run_once() -> Result<Vec<rag_foundation::document_processing::JobOutcome>, String> {
    let config = Config::load().map_err(|err| format!("configuration error: {err}"))?;
    let mut client = postgres::Client::connect(&config.database_url, postgres::NoTls)
        .map_err(|err| format!("document database unavailable: {err}"))?;
    let store = PgStore::connect(&config.database_url)
        .map_err(|err| format!("failed to build the Postgres store: {err}"))?;
    let storage = DocumentStorage::new(config.document_root.clone());
    let transport =
        Arc::new(HttpTransport::new(config.request_timeout).map_err(|err| err.to_string())?);
    let model = OllamaClient::new(config.ollama_url.clone(), transport, config.request_timeout);
    let embedder = OllamaEmbedder::new(model, config.embed_model.clone())
        .with_embed_dim(config.embed_dim as usize);
    let corpus = std::env::var("RAG_CORPUS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    run_due_jobs(
        &mut client,
        &store,
        &embedder,
        &storage,
        &config,
        "cli-worker",
        100,
        corpus.as_deref(),
        300,
    )
    .map_err(|err| err.to_string())
}
