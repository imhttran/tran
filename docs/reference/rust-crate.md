# Rust crate reference

The repository is a single Rust package at the root (`rag-foundation`) plus the
Next.js UI under `web/`. The Go reference was removed by RUST-007C; its pinned
commit SHA and source URL remain in
[`rust-transition-evidence.md`](rust-transition-evidence.md) for audit.

## Toolchain

[`rust-toolchain.toml`](../../rust-toolchain.toml) pins the channel and
components used by the build, tests, formatting, and Clippy.

## Layout

| Path | Purpose |
| --- | --- |
| `src/lib.rs` | Library modules: configuration, loaders, chunking, ingestion, retrieval, answering, evaluation, bilingual harness, Axum API |
| `src/bin/ingest.rs` | Production ingestion CLI (PDF, Markdown, plain text) |
| `src/bin/query.rs` | Production question → cited-answer CLI |
| `src/bin/eval.rs` | Production retrieval-evaluation CLI |
| `src/bin/expbilingual.rs` | Bilingual experiment harness |
| `src/bin/api.rs` | Axum HTTP API used by the Next.js UI |
| `tests/` | Integration and schema tests |
| `fixtures/` | Frozen parity fixtures and inputs, including `fixtures/parity/frozen/` captured from the pinned Go reference |
| `acceptance/` | Separate acceptance crates for the API contract, live RAG, parity, final layout, and clean checkout |
| `web/` | Next.js UI and Playwright browser tests |

## Commands

```bash
cargo build --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

The Makefile and `.githooks/` delegate to these commands; see the root
[`README.md`](../../README.md) and
[`commands.md`](commands.md) for the full target list.
