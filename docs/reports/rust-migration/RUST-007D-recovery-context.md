# RUST-007D recovery context

Verified against repository commit `acd1fb9` and the production Rust SQL retrieval path. This context is supplied through `SOP_TASK_INPUTS`; it does not change the plan, task criteria, or mappings.

## Goal

Port the Go `cmd/expbilingual` experiment harness and add a production Rust `expbilingual` command with behavior parity. Reuse the existing production retrieval path; do not reimplement retrieval. Keep Go present. Live-model groundedness/citations remain a separate, explicitly unmet closure requirement.

## Go reference contract (`cmd/expbilingual/main.go`)

- Environment: `ARM=A|B|C|D1|D2|E|all` (default `all`), `REPEATS` (default 1), `DIAG=1`, `EVAL_DATASET` (default `evals/retrieval-vi-en-expanded.json`), `OLLAMA_EMBED_MODEL` (default `nomic-embed-text`), `OLLAMA_CHAT_MODEL` (default `llama3.2:3b`), `QUERY_CACHE` (default `docs/rq-006-query-cache.json`), `GEN_TIMEOUT` seconds (default 60).
- Header: `HEADER arm=... dataset=... embed=... chat=... floor=... repeats=... gen_options=...` where generation options are pinned to `temperature:0`, `seed:0`, `num_predict:64`.
- Arms:
  - `A`: baseline, original query.
  - `B`: bilingual expansion, fused.
  - `C`: translation, fused.
  - `D1`: `C` plus original-query rejection guard.
  - `D2`: `D1` plus baseline-first ranking (original top-1 pinned at rank 1).
  - `E`: `C` plus cross-language agreement (at least 2 of 3 query vector paths clear the similarity floor).
- `ARM=all` runs `A,D1,D2` by default.
- Cache: keyed by case id plus configuration/model; variants per arm; load and save the cache at `QUERY_CACHE`; preserve the Go format.
- Output: per-case/per-arm/per-repeat metrics and ranking, then `FOOTER wall=... generated_queries=... cache=...`.
- The harness is evaluation-only; it must not modify production retrieval.

## Rust production target

- Add `rust/src/bin/expbilingual.rs` and a bilingual module (for example `rust/src/bilingual.rs`) exported from `rust/src/lib.rs`; add `[[bin]] name = "expbilingual"` to `rust/Cargo.toml`.
- Reuse: `rag_foundation::config::Config`; `storage_pg::PgStore`; `retrieval_pg::fuse`; `retrieval::{Candidate, ...}`; `embedding::OllamaEmbedder`; `generation`/`ollama`; `document::Chunk`; `storage::{StoredChunk, EMBEDDING_DIM}`; `answer::service` where needed.
- Verified APIs:
  - `PgStore::connect(dsn) -> Result<PgStore, PgStoreError>`
  - `PgStore::vector_search(&self, corpus: &str, vector: &[f64], top_k: usize) -> Result<Vec<SqlHit>, PgStoreError>`
  - `PgStore::keyword_search(&self, corpus: &str, query: &str, top_k: usize, language: &str) -> Result<Vec<SqlHit>, PgStoreError>`
  - `retrieval_pg::fuse(rankings: &[Vec<SqlHit>], top_k: usize) -> Vec<Candidate>`
  - `OllamaClient::new(base_url, Arc<HttpTransport>, timeout)`
  - `OllamaEmbedder::new(client, model).with_embed_dim(dim)`
  - `SqlHit { row: StoredChunkRow, similarity: f64, keyword_score: f64 }`
  - `StoredChunkRow { id: i64, chunk: StoredChunk }`
  - `Config` fields include `ollama_url`, `embed_model`, `chat_model`, `database_url`, `min_similarity`, `top_k`, `final_k`, `expand_limit`, `language`, `request_timeout`, `query_rewrite`.
- Preserve Go's RRF ordering (fused score, best rank, database row id); the shared `retrieval_pg::fuse` already implements it.
- Do not create a passing acceptance crate without the production command. Add `rust/acceptance/bilingual_parity` **with** the production binary. Its tests must run the real Rust `expbilingual` and the pinned Go `cmd/expbilingual` against the same corpus, dataset, configuration, and deterministic embeddings, normalize the header/per-case/footer/cache output, and compare with explicit numerical tolerances. Scripted generator/cache tests do not prove live-model behavior.
- Use an isolated database and unique run-owned corpus/source tags; clean up only those tags. Never delete foreign rows.
- Acceptance checks: `go build ./...`, `go test ./...`, `go vet ./...`, `cargo test --locked --manifest-path rust/acceptance/bilingual_parity/Cargo.toml`, `cargo test --locked --manifest-path rust/Cargo.toml`.
- Do not remove Go, move the crate to the root, or change the Rust-only layout in this task.
