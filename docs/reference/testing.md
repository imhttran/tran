# Testing

`cargo test --locked` (or `make test` / `make rust-test`) runs the unit and
integration test targets in this package. The runnable procedures — disposable
databases, per-suite isolation, and the real-browser suite — are owned by
[guides/testing.md](../guides/testing.md); this page records the facts the
commands depend on.

## Test tiers

| Tier | Command | Needs | Behaviour when the database is unset |
| --- | --- | --- | --- |
| Unit | `cargo test --locked --lib` | Nothing | Runs; network and model calls use deterministic stubs. |
| RAG schema integration | `cargo test --locked --test ingest_binary --test section_chunks_pg --test storage_interop` | `DATABASE_URL` with pgvector | **Fails** with an explicit message; it never skips. |
| Genealogy integration | `cargo test --locked --test <suite>` | `GENEALOGY_DATABASE_URL` | Most suites skip with a message; each creates and drops its own database. |
| Acceptance | `cargo test --locked --manifest-path acceptance/<crate>/Cargo.toml` | Varies | Separate crates, not part of the default suite. |

## Environment

- `DATABASE_URL` — the RAG database (`documents`, chunks, embeddings). Apply
  `migrations/*.sql` first (`make db-up` does this).
- `GENEALOGY_DATABASE_URL` — the genealogy server DSN. Each suite derives a
  disposable `*_test` database from it, so the role needs `CREATE DATABASE`.
- No `RAG_INTEGRATION` gate exists: the suite is selected by test target, not by
  an environment variable.
- `EMBED_PROVIDER` / `GEN_PROVIDER` are validated at config load, but the
  current command binaries construct the Ollama client directly. Unit tests
  substitute deterministic stub embedder and generator implementations.

## Serialization

Suites that replace shared source rows must not run in parallel against one
database. Run those targets with `-- --test-threads=1`; genealogy suites are
already isolated per suite and do not need it.

## Real browser

`make web-e2e` runs the Playwright suite against disposable databases. Its
selection and refusal rules live in `scripts/e2e-db-env.sh`.
