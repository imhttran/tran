# Architecture overview

The final repository is Rust-only. The library crate `rag-foundation` owns the
RAG behavior; the CLI binaries and the Axum API are thin adapters over it. The
Next.js UI talks only to the Axum API through same-origin `/api/*` requests.

## Entry points

| Entry point | Responsibility |
| --- | --- |
| `src/bin/ingest.rs` | Load, guard, chunk, embed, and transactionally store one source |
| `src/bin/query.rs` | Retrieve evidence and answer one question with citations |
| `src/bin/eval.rs` | Score retrieval against `evals/retrieval.json` |
| `src/bin/expbilingual.rs` | RAG-024 bilingual experiment harness |
| `src/bin/api.rs` | Axum API: health, upload/ingest, status, query |
| `web/` | Next.js UI; the browser calls only same-origin Next.js `/api/*` |

## Ingest path

`ingest` turns a file into stored chunk embeddings:

1. **Load** — `src/loader/` dispatches on the file format: the PDF text-layer
   loader, the Markdown section parser, or the plain-text loader.
2. **Guard** — `src/ingest.rs` rejects fatal loader errors, fully scanned PDFs,
   incomplete PDFs (naming the affected pages), and sources that yield no chunks
   **before** any embedding call or database write.
3. **Chunk** — `src/chunker.rs` splits each section into overlapping word chunks.
4. **Fingerprint** — `src/fingerprint.rs` hashes the bytes plus chunker settings;
   an unchanged source is a no-op.
5. **Embed** — `src/embedding.rs` calls Ollama `/api/embed` for every chunk.
6. **Store** — `src/transaction.rs` replaces the source's rows in one
   transaction, after `src/storage.rs` validates every embedding dimension.

`src/pipeline.rs` is the single pipeline implementation; `src/service.rs` is the
callable seam used by both the CLI and the API.

## Query path

`query` and the API use the shared `src/query_service.rs`:

1. **Retrieve** — `src/storage_pg.rs` runs pgvector vector search and PostgreSQL
   full-text search over the configured corpus.
2. **Fuse** — `src/retrieval/fusion.rs` / `src/retrieval_pg.rs` apply reciprocal
   rank fusion; candidates are deduplicated and expanded.
3. **Budget** — `src/answer/context_budget.rs` assembles a bounded context.
4. **Gate** — `src/answer/answerability.rs` returns the baseline refusal when no
   passage is available, without a generation call.
5. **Generate** — `src/generation.rs` prompts Ollama `/api/generate`.
6. **Cite** — `src/answer/citation.rs` parses, validates, and performs the single
   bounded citation repair; `src/answer/render.rs` renders page references.

The API additionally returns the cited passages as structured `sources`
(`source`, `section`, `page`, `text`); the CLI prints the rendered references.

## Module responsibilities

| Module | Responsibility |
| --- | --- |
| `config` | Read environment settings; build the Ollama client and database URL |
| `loader` | Detect a file's format and extract sections |
| `section` / `document` | Parse `## ` sections; carry page provenance |
| `chunker` | Split sections into overlapping chunks |
| `ingest` | Pre-effect rejection decisions |
| `fingerprint` | Content fingerprint and unchanged-source check |
| `pipeline` / `service` | Single ingest pipeline and service seam |
| `transaction` / `storage*` | Transactional replacement and pgvector storage |
| `embedding` / `generation` / `ollama` | Ollama request ports and shared HTTP client |
| `provider` | Embedder/generator provider registry |
| `retrieval*` | pgvector search, FTS, RRF fusion, dedup, expansion, filters |
| `answer` | Prompt, budget, answerability, generation, citation validation |
| `query_service` | Shared CLI/API question path |
| `evaluation` | Recall/precision evaluation harness |
| `bilingual` | RAG-024 experiment arms, cache, and pinned generation |
| `api` | Axum router, validation, structured errors, loopback binding |
| `web/` | Next.js upload/status/question/cited-answer UI and Playwright tests |

## Boundaries

- **Binaries are thin.** They wire configuration and adapters; they own no
  retrieval, ingestion, or citation logic.
- **One retrieval path.** `query_service` and `eval` use the shared retrieval
  modules; the bilingual harness reuses the same fusion and guards.
- **One ingest path.** `ingest`, the API, and every acceptance crate call the same
  `IngestService`/`IngestPipeline`.
- **Pre-effect safety.** Rejections happen before embedding or writes; source
  replacement is transactional.
- **Provider seam.** `EMBED_PROVIDER` / `GEN_PROVIDER` resolve through
  `provider`; an unregistered provider fails fast.
- **Server-side credentials.** The API binds `127.0.0.1` by default; `API_URL`,
  `DATABASE_URL`, and model settings stay in the server process. The browser only
  sees same-origin Next.js `/api/*`.
- **Frozen parity provenance.** The pinned Go reference SHA
  `d72fb398e8959eea83ed24be03f6df8cdeca86a2` and the captured
  `fixtures/parity/frozen/` outputs remain for audit; expected values are never
  regenerated from Rust.
