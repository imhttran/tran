//! Live Ollama smoke check for embedding and generation.
//!
//! Skipped (not failed) unless `RUST_OLLAMA_SMOKE=1` is set, so hermetic test
//! runs and CI are unaffected. When enabled it embeds a sample text and
//! generates a sample completion against `OLLAMA_URL`, prints the recorded
//! embed and chat model identifiers, and asserts the live contract from the
//! plan: the embedding length equals `Config::embed_dim` and the completion is
//! non-empty.
//!
//! The HTTP transport is the library's `reqwest`-backed
//! [`rag_foundation::ollama::HttpTransport`]; the crate carries no hand-rolled
//! HTTP stack.

use std::env;
use std::process::ExitCode;
use std::sync::Arc;

use rag_foundation::config::Config;
use rag_foundation::embedding::{Embedder, OllamaEmbedder};
use rag_foundation::generation::{Generator, OllamaGenerator};
use rag_foundation::ollama::{HttpTransport, OllamaClient};

fn main() -> ExitCode {
    if env::var("RUST_OLLAMA_SMOKE").ok().as_deref() != Some("1") {
        println!("RUST_OLLAMA_SMOKE is not set; skipping live Ollama smoke check");
        return ExitCode::SUCCESS;
    }

    let config = match Config::load() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("configuration error: {err}");
            return ExitCode::FAILURE;
        }
    };

    println!("embed model: {}", config.embed_model);
    println!("chat model: {}", config.chat_model);

    let transport = Arc::new(HttpTransport::new(config.request_timeout));

    let embed_client = OllamaClient::new(
        config.ollama_url.clone(),
        transport.clone(),
        config.request_timeout,
    );
    let embedder = OllamaEmbedder::new(embed_client, config.embed_model.clone())
        .with_embed_dim(config.embed_dim as usize);

    let vector = match embedder.embed("RUST-004 live embedding smoke check") {
        Ok(vector) => vector,
        Err(err) => {
            eprintln!("embedding smoke check failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!("embedding length: {}", vector.len());

    if vector.len() != config.embed_dim as usize {
        eprintln!(
            "embedding length {} != configured embed_dim {}",
            vector.len(),
            config.embed_dim
        );
        return ExitCode::FAILURE;
    }

    let gen_client =
        OllamaClient::new(config.ollama_url.clone(), transport, config.request_timeout);
    let generator = OllamaGenerator::new(gen_client, config.chat_model.clone());

    let answer = match generator.generate("Reply with a short greeting.") {
        Ok(answer) => answer,
        Err(err) => {
            eprintln!("generation smoke check failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!("completion length: {}", answer.len());

    if answer.trim().is_empty() {
        eprintln!("generation smoke check returned an empty completion");
        return ExitCode::FAILURE;
    }

    println!("live Ollama smoke check passed");
    ExitCode::SUCCESS
}
