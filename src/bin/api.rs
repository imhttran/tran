//! Production Axum API entry point.
//!
//! Loads the same configuration the CLI uses, then serves the browser-facing
//! API on loopback. Database and Ollama settings stay in this process; the
//! Next.js server proxies same-origin `/api/*` requests here.
//!
//! Environment:
//! - `API_HOST` (default `127.0.0.1`)
//! - `API_PORT` (default `8080`)
//! - `RAG_CORPUS` (default `trusted`)

use std::process::ExitCode;

use rag_foundation::api;
use rag_foundation::config::Config;

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::load() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("api: configuration error: {err}");
            return ExitCode::FAILURE;
        }
    };
    match api::serve(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("api: {message}");
            ExitCode::FAILURE
        }
    }
}
