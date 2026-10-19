# Commands

The project is a Rust package at the repository root. The CLI binaries are thin
wrappers over the library modules, and the Axum API calls the same services.

## Workflow

| Command | What it does |
| --- | --- |
| `ingest <file>` | Read a PDF/Markdown/plain-text document → split into chunks → embed each chunk → store it transactionally |
| `query "<question>"` | Embed a question → retrieve the closest chunks → answer with validated citations |
| `eval` | Score retrieval (recall/precision) against `evals/retrieval.json` |
| `expbilingual` | Run the RAG-024 bilingual evaluation arms over a frozen split |
| `api` | Serve the Axum API for the Next.js UI on `127.0.0.1:8080` |

Run them with `cargo run --locked --bin <name>` or use the Makefile shortcuts.

## Shortcuts

| Target | Runs |
| --- | --- |
| `make db-up` | start the database and apply all migrations |
| `make db-down` | `docker compose down` |
| `make db-schema` | apply all migrations (existing volume only) |
| `make test` | `cargo test --locked` |
| `make fmt-json` | rewrite every tracked JSON file with `jq` |
| `make ingest` | ingest `examples/loan-policy.md` (`FILE=…` to change) |
| `make ask Q="…"` | ask a question (omit `Q` to be prompted) |
| `make eval` | score retrieval against `evals/retrieval.json` |
| `make rust-check` | fmt, build, test, clippy, final-layout, and clean-checkout checks |
| `make check` | the Rust checks plus the web typecheck/build |

## Rust checks and acceptance

| Target | Runs |
| --- | --- |
| `make rust-fmt` | `cargo fmt --check` |
| `make rust-build` | `cargo build --locked` |
| `make rust-test` | `cargo test --locked` |
| `make rust-clippy` | `cargo clippy --locked --all-targets -- -D warnings` |
| `make rust-acceptance-evidence` | `cargo test --locked --manifest-path acceptance/evidence/Cargo.toml` |
| `make rust-acceptance-final-layout` | `cargo test --locked --manifest-path acceptance/final_layout/Cargo.toml` |
| `make rust-acceptance-clean-checkout` | `cargo test --locked --manifest-path acceptance/clean_checkout/Cargo.toml` |
| `make rust-acceptance-k3` | K=3 parity against the frozen Go reference output |
| `make rust-acceptance-ingest-md-text` | production Markdown/plain-text ingestion and transaction safety |
| `make rust-acceptance-live-rag` | live Ollama + PostgreSQL ingest → query → cited-answer verification |
| `make rust-acceptance-api-contract` | deterministic Axum API contract (upload, status, query, health, refusal) |
| `make rust-acceptance-functional` | K=3 parity + Markdown/plain-text ingestion (needs the database) |
| `make transition-evidence` | evidence, final layout, and clean checkout without Go |
| `make web-e2e` | real-browser Playwright upload → ingest → cited-answer verification |

The `clean_checkout` crate copies the package into a temporary directory, removes
Go from `PATH`, and runs `cargo build --locked`, `cargo test --locked --lib`, and
the section/chunk parity test. The full database-backed suite is covered by
`make rust-test`.

## Rust API and web UI

The Axum API binary (`src/bin/api.rs`) serves the browser-facing contract and
calls the same Rust services as the CLI:

| Endpoint | Purpose |
| --- | --- |
| `GET /api/health` | Service, database, and model status; never returns credentials |
| `POST /api/documents/ingest` | Multipart `file` upload for `.md`, `.markdown`, `.txt`, `.pdf` |
| `GET /api/documents/{source}/status` | Stored chunks, sections, pages, corpus, and fingerprint |
| `POST /api/query` | `{question, top_k?}` -> answer, `insufficient`, and cited sources |

`API_HOST` defaults to `127.0.0.1` and `API_PORT` to `8080`. `RAG_CORPUS`
selects the corpus (default `trusted`). The Next.js app in `web/` proxies
same-origin `/api/*` server-side to `API_URL`; the browser never receives the
API URL or credentials. Run `make web-e2e` for the real-browser check.

## Frozen Go provenance

The Go implementation was removed by RUST-007C. The pinned Go commit
`d72fb398e8959eea83ed24be03f6df8cdeca86a2`, its source repository URL, and the
frozen `fixtures/parity/` outputs remain in the repository for audit. The
Rust-only parity tests compare against those frozen outputs; they never
regenerate expected values from Rust.
