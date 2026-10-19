# Project structure

The repository is a single Rust package at the root plus the Next.js UI. The Go
implementation was removed by RUST-007C; its pinned provenance remains in
[`rust-transition-evidence.md`](../reference/rust-transition-evidence.md).

```text
.
├── Cargo.toml                 # rag-foundation package and binary targets
├── Cargo.lock                 # committed dependency lock
├── rust-toolchain.toml        # pinned toolchain/components
├── src/
│   ├── lib.rs                 # library module exports
│   ├── api.rs                 # Axum router, handlers, validation
│   ├── config.rs              # environment configuration
│   ├── document.rs            # Section/Chunk domain types
│   ├── loader/                # Markdown, plain-text, PDF loaders
│   ├── section.rs             # `## ` section parser
│   ├── chunker.rs             # overlapping word chunks
│   ├── ingest.rs              # pre-effect guard (PDF/page-less)
│   ├── fingerprint.rs         # content fingerprint and unchanged check
│   ├── pipeline.rs            # single ingest pipeline (format dispatch)
│   ├── service.rs             # IngestService seam
│   ├── transaction.rs         # transactional source replacement
│   ├── storage.rs             # schema contract and embedding guard
│   ├── storage_pg.rs          # r2d2 + PostgreSQL/pgvector client
│   ├── embedding.rs           # Ollama `/api/embed`
│   ├── generation.rs          # Ollama `/api/generate`
│   ├── ollama.rs              # shared HTTP client/transport
│   ├── provider.rs            # embed/generate provider registry
│   ├── retrieval/             # vector, FTS, fusion, dedup, expand, filters
│   ├── retrieval_pg.rs        # SQL vector/FTS fusion path
│   ├── answer/                # prompt, budget, citation validation, service
│   ├── query_service.rs       # shared CLI/API question path
│   ├── evaluation.rs          # recall/precision harness
│   ├── bilingual.rs           # RAG-024 experiment harness
│   ├── bin/
│   │   ├── ingest.rs          # production ingest CLI
│   │   ├── query.rs           # production question → cited answer CLI
│   │   ├── eval.rs            # production evaluation CLI
│   │   ├── expbilingual.rs    # bilingual experiment CLI
│   │   ├── api.rs             # Axum API binary
│   │   └── smoke.rs           # optional live Ollama smoke check
├── tests/                     # schema, parity, binary, and integration tests
├── acceptance/                # separate acceptance crates
│   ├── api_contract/
│   ├── bilingual_parity/
│   ├── clean_checkout/
│   ├── e2e/
│   ├── eval_parity/
│   ├── evidence/
│   ├── final_layout/
│   ├── ingest_cli/
│   ├── ingest_md_text/
│   ├── k3_parity/
│   ├── live_rag/
│   └── retrieval_cli/
├── fixtures/                  # frozen parity inputs/outputs and the Go reference captures
├── migrations/                # shared PostgreSQL + pgvector schema
├── evals/                     # frozen evaluation datasets
├── examples/                  # sample documents
├── web/                       # Next.js UI and Playwright browser tests
├── scripts/                   # web-control.sh, web-e2e.sh
├── docs/                      # requirements, architecture, reference, guides, evidence
├── Makefile                   # Rust and web convenience targets
├── .githooks/                 # pre-commit / pre-push Rust checks
└── .github/workflows/ci.yml   # Rust, API-contract, and web CI
```

## Entry points

- `src/bin/ingest.rs`, `query.rs`, `eval.rs`, and `expbilingual.rs` are thin CLI
  wrappers over the library modules.
- `src/bin/api.rs` serves the Axum API; it calls the same `IngestService` and
  `QueryService` as the CLI.
- `web/` is a Next.js App Router UI. The browser calls only same-origin
  `/api/*`; the Next.js server proxies those requests to Axum.

## Generated and local state

`target/`, `web/node_modules/`, `web/.next*`, `web/.run/`, `.env`, and
`.agent-sdlc/` are git-ignored. `Cargo.lock` is committed.
