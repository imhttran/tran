//! Production question-answer entry point.
//!
//! Loads configuration, builds the shared [`QueryService`] (the same retrieval,
//! generation, and citation validation path the Axum API uses), and answers one
//! question. Dependencies are reported distinctly; a retrieval, database,
//! embedding, or generation failure exits non-zero.
//!
//! Usage: `query <question>` (or set `QUESTION`). `RAG_CORPUS` selects the
//! stored corpus (default `trusted`).

use std::env;
use std::process::ExitCode;
use std::sync::Arc;

use rag_foundation::config::Config;
use rag_foundation::query_service::QueryService;
use rag_foundation::storage_pg::PgStore;

const DEFAULT_CORPUS: &str = "trusted";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("query: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let config = Config::load().map_err(|err| format!("configuration error: {err}"))?;
    let question = question(&config)?;
    let corpus = env::var("RAG_CORPUS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CORPUS.to_string());

    let store = Arc::new(
        PgStore::connect(&config.database_url)
            .map_err(|err| format!("connect to the document store: {err}"))?,
    );
    let service = QueryService::from_config(&config, store, corpus)?;
    let answer = service.answer(&question)?;

    println!("{}", answer.text);
    for reference in &answer.references {
        println!("source: {reference}");
    }
    Ok(ExitCode::SUCCESS)
}

fn question(config: &Config) -> Result<String, String> {
    let mut args = env::args().skip(1);
    let from_args = args.next();
    if args.next().is_some() {
        return Err("usage: query <question> (exactly one argument)".to_string());
    }
    let question = from_args
        .or_else(|| {
            let from_config = config.question.trim();
            (!from_config.is_empty()).then(|| from_config.to_string())
        })
        .ok_or_else(|| "usage: query <question>".to_string())?;
    if config.max_question_bytes > 0 && question.len() > config.max_question_bytes as usize {
        return Err(format!(
            "question exceeds MAX_QUESTION_BYTES ({})",
            config.max_question_bytes
        ));
    }
    Ok(question)
}
