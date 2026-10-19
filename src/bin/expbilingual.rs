//! EVALUATION-ONLY bilingual retrieval experiment command.
//!
//! Ported from the Go baseline `cmd/expbilingual` (RAG-024 RQ-006). Runs the
//! retrieval arms in [`rag_foundation::bilingual`] over a frozen evaluation
//! split against an already-ingested database and prints the `HEADER`, `GEN`,
//! `DIAG`, `CASE` and `FOOTER` lines of the Go harness. It does not modify
//! production retrieval and never writes to the `documents` table.
//!
//! Env: `ARM=A|B|C|D1|D2|E|all`, `REPEATS`, `DIAG=1`, `QUERY_CACHE=<path>`,
//! `GEN_TIMEOUT=<seconds>`, `OLLAMA_CHAT_MODEL`, `OLLAMA_EMBED_MODEL`,
//! `EVAL_DATASET`, `MIN_SIMILARITY`, plus `OLLAMA_URL`, `DATABASE_URL`,
//! `EMBED_DIM` and `REQUEST_TIMEOUT` from [`Config`]. `RAG_CORPUS` (default
//! `trusted`) scopes every read; the Go harness searches unfiltered, so the two
//! agree on a database holding a single corpus.

use std::env;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use rag_foundation::bilingual::{
    self, BilingualError, CorpusSearcher, Harness, OllamaQueryGenerator, QueryCache, Settings,
};
use rag_foundation::config::Config;
use rag_foundation::embedding::OllamaEmbedder;
use rag_foundation::ollama::{HttpTransport, OllamaClient};
use rag_foundation::storage_pg::PgStore;

const DEFAULT_CORPUS: &str = "trusted";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            let _ = io::stdout().flush();
            eprintln!("expbilingual: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let config = Config::load().map_err(|err| format!("configuration error: {err}"))?;
    let settings = Settings::from_env();
    let corpus = env::var("RAG_CORPUS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CORPUS.to_string());

    let stdout = io::stdout();
    let mut out = stdout.lock();
    let write_err = |err: io::Error| format!("write output: {err}");

    writeln!(out, "{}", settings.header_line(config.min_similarity)).map_err(write_err)?;

    let cases = bilingual::load_cases(Path::new(&settings.dataset))
        .map_err(|err| format!("{err}  (EVAL_DATASET={})", settings.dataset))?;

    let database = |err: &dyn std::fmt::Display| {
        format!(
            "document store: {err}  (DATABASE_URL={})",
            config.database_url
        )
    };
    let store = PgStore::connect(&config.database_url).map_err(|err| database(&err))?;
    store.ping().map_err(|err| database(&err))?;

    let embed_transport = Arc::new(HttpTransport::new(config.request_timeout));
    let embed_client = OllamaClient::new(
        config.ollama_url.clone(),
        embed_transport,
        config.request_timeout,
    );
    let embedder = OllamaEmbedder::new(embed_client, settings.embed_model.clone())
        .with_embed_dim(usize::try_from(config.embed_dim).unwrap_or(0));

    // Go bounds each generation call by GEN_TIMEOUT on a client whose HTTP
    // timeout is REQUEST_TIMEOUT, so the effective bound is the smaller one.
    let gen_timeout = settings.gen_timeout.min(config.request_timeout);
    let gen_client = OllamaClient::new(
        config.ollama_url.clone(),
        Arc::new(HttpTransport::new(gen_timeout)),
        gen_timeout,
    );
    let generator = OllamaQueryGenerator::new(gen_client);
    let searcher = CorpusSearcher::new(&store, corpus);

    let cache_path = Path::new(&settings.cache_path);
    let mut cache =
        QueryCache::load(cache_path, &settings.chat_model, &mut out).map_err(write_err)?;

    let harness = Harness {
        searcher: &searcher,
        embedder: &embedder,
        generator: &generator,
        chat_model: &settings.chat_model,
        min_similarity: config.min_similarity,
        diag: settings.diag,
    };

    let start = Instant::now();
    let generated = harness
        .run_all(
            &settings.arms,
            settings.repeats,
            &cases,
            &mut cache,
            &mut out,
        )
        .map_err(|err| match err {
            BilingualError::Store(_) => database(&err),
            BilingualError::Embed { .. } | BilingualError::Generate { .. } => {
                format!("{err}  (OLLAMA_URL={})", config.ollama_url)
            }
            other => other.to_string(),
        })?;

    // Go ignores a failed cache write; the warning goes to stderr only.
    if let Err(err) = cache.save(cache_path, &settings.chat_model) {
        eprintln!(
            "expbilingual: warning: could not write {}: {err}",
            settings.cache_path
        );
    }

    writeln!(
        out,
        "FOOTER wall={} generated_queries={generated} cache={}",
        bilingual::go_duration_ms(start.elapsed()),
        settings.cache_path
    )
    .map_err(write_err)?;
    out.flush().map_err(write_err)?;
    Ok(())
}
