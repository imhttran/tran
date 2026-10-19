//! Production retrieval-evaluation command.
//!
//! Ported from the Go baseline `cmd/eval`. Loads configuration, builds the real
//! Ollama embedder and the PostgreSQL corpus reader, loads the evaluation
//! dataset selected by `EVAL_DATASET`, and scores every case through
//! [`rag_foundation::evaluation`] and the shared retrieval modules.
//!
//! The fixed deterministic configuration disables every non-deterministic stage
//! (`QUERY_REWRITE=false`, `EVAL_LEXICAL_RERANK=false`, `EVAL_LLM_RERANK=false`,
//! `EVAL_ANSWERABILITY_GATE=false`, `EVAL_FACT_JUDGE=false`), so only vector +
//! full-text + fusion + dedup + expand run.
//!
//! A missing, unreadable, or malformed dataset exits non-zero and names the path
//! or field. An unreachable database or an empty corpus exits non-zero and
//! names the store.

use std::env;
use std::process::ExitCode;
use std::sync::Arc;

use rag_foundation::config::Config;
use rag_foundation::embedding::OllamaEmbedder;
use rag_foundation::evaluation::{self, dataset_path, EvalSettings, RankedChunk};
use rag_foundation::ollama::{HttpTransport, OllamaClient};
use rag_foundation::storage_pg::PgStore;

const DEFAULT_CORPUS: &str = "trusted";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("eval: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    // The dataset is read first, matching the Go baseline, so a mistaken
    // EVAL_DATASET fails before any network or database work.
    let path = dataset_path(env::var("EVAL_DATASET").ok().as_deref());
    let cases = evaluation::load_dataset(&path)
        .map_err(|err| format!("{err}  (EVAL_DATASET={})", path.display()))?;
    println!("Loaded {} evaluation cases", cases.len());

    let config = Config::load().map_err(|err| format!("configuration error: {err}"))?;
    let corpus = env::var("RAG_CORPUS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CORPUS.to_string());

    let transport = Arc::new(HttpTransport::new(config.request_timeout));
    let client = OllamaClient::new(config.ollama_url.clone(), transport, config.request_timeout);
    let embedder = OllamaEmbedder::new(client, config.embed_model.clone())
        .with_embed_dim(config.embed_dim as usize);

    let store = PgStore::connect(&config.database_url).map_err(|err| {
        format!(
            "document store: {err}  (DATABASE_URL={})",
            config.database_url
        )
    })?;

    let (_, _) = evaluation::load_corpus(&store, &corpus)
        .map_err(|err| format!("{err}  (DATABASE_URL={})", config.database_url))?;

    let settings = EvalSettings {
        min_similarity: config.min_similarity,
        expand_limit: u32::try_from(config.expand_limit.max(0)).unwrap_or(u32::MAX),
        final_k: usize::try_from(config.final_k.max(1)).unwrap_or(1),
        candidate_k: usize::try_from(config.top_k.max(1)).unwrap_or(1),
        language: config.language.clone(),
    };

    let results = evaluation::evaluate_sql(&cases, &store, &corpus, &embedder, &settings)
        .map_err(|err| err.to_string())?;

    for result in &results {
        println!();
        println!("Question: {}", result.case.question);
        if result.answerable {
            println!(
                "  Recall@{}={:.2}  Precision@{}={:.2}",
                settings.final_k, result.metrics.recall, settings.final_k, result.metrics.precision
            );
        } else if result.ranking.is_empty() {
            println!("  Correct rejection");
        } else {
            println!("  FALSE POSITIVE");
        }
        print_ranking(&result.ranking);
    }

    let aggregate = evaluation::aggregate(&results);
    println!();
    println!("Overall:");
    println!(
        "  cases={}  answerable={}  Avg Recall@{}={:.2}  Avg Precision@{}={:.2}",
        aggregate.cases,
        aggregate.answerable,
        settings.final_k,
        aggregate.recall,
        settings.final_k,
        aggregate.precision
    );
    println!("  dataset={}", path.display());
    println!("  corpus={corpus}");
    println!(
        "  top_k={}  final_k={}  expand_limit={}",
        config.top_k, config.final_k, config.expand_limit
    );
    Ok(())
}

fn print_ranking(ranking: &[RankedChunk]) {
    if ranking.is_empty() {
        println!("  (no candidates)");
        return;
    }
    for (index, chunk) in ranking.iter().enumerate() {
        println!(
            "    #{}  {:.4}  {} / {} / chunk {}",
            index + 1,
            chunk.score,
            chunk.source,
            chunk.section,
            chunk.chunk_index
        );
    }
}
