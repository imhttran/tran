# Testing guides

This guide covers how to run the test suite, the disposable-PostgreSQL harness
used by the database integration tests, and the optional real-Ollama smoke test.

## The standard suite (no database, no network, no model)

```sh
make test        # or: cargo test --locked
```

`cargo test --locked` runs every non-optional test with no database and no model.
Database-backed tests **skip** (they do not fail) unless `RAG_INTEGRATION=1` is
set, so the standard suite stays fast and green on a machine with no PostgreSQL
and no Ollama running. Deterministic, network-free stub providers back the
in-process tests, so identical inputs always produce identical results.

## Disposable PostgreSQL (never `rag_db`)

The integration tests run against a **throwaway** database, never the development
database. The harness conventions are:

- **`RAG_TEST_DATABASE_URL`** points at the disposable database; it defaults to
  `postgres://rag:rag@127.0.0.1:5434/rag_test?sslmode=disable` (the
  `docker-compose` database exposed on port 5434).
- **Never `rag_db`.** The retrieval integration tests truncate the `documents`
  table, so they must never be pointed at `DATABASE_URL` (the dev database).
  The `.githooks/pre-push` hook probes `INTEGRATION_TEST_DB` (bound to
  `RAG_TEST_DATABASE_URL`) with `psql` first, and skips cleanly when the database
  is unreachable, so a stopped Docker daemon never blocks a push.
- **Throwaway name.** Use a clearly disposable name such as `rag_test`. Never
  create, migrate, or truncate a database named `rag_db`.
- **Refusal guard.** The RAG-025B harness in `internal/rag/e2e_db_test.go` parses
  the database name out of `RAG_TEST_DATABASE_URL` (falling back to `DATABASE_URL`
  only when the former is unset) and **refuses to run** — before any migration or
  mutation — when that name is `rag_db`. No integration path in the rag package
  can reach `rag_db`.

### Setup

```sh
make db-up            # start PostgreSQL 16 + pgvector (docker-compose)
```

### Running the database-backed tests

The schema, ingestion, and retrieval tests use the dedicated database in
`DATABASE_URL` and fail (never skip) when it is unavailable or lacks pgvector.

```sh
cargo test --locked --test ingest_binary --test section_chunks_pg --test storage_interop -- --test-threads=1
```

`--test-threads=1` serializes the database tests: some replace whole source
rows, so they must not run in parallel.

The full suite (including the binary and library tests) is:

```sh
cargo test --locked
```

The live production ingest → query → cited-answer check is the explicit
`acceptance/live_rag` crate; it requires real Ollama and a dedicated database and
is not part of the default suite:

```sh
cargo test --locked --manifest-path acceptance/live_rag/Cargo.toml
```

### Schema migration

The database tests connect to `RAG_TEST_DATABASE_URL` and apply every
`migrations/*.sql` file in name order (idempotent), so they work against a fresh
database and pick up migrations added after `001`.

### Teardown

```sh
make db-down          # stop and remove the disposable PostgreSQL container
```

## Genealogy integration-test isolation (TDI-002)

The genealogy suites that mutate the genealogy schema no longer share one database.
Each suite derives its own disposable target from `GENEALOGY_DATABASE_URL`, applies
the full migration set to that target, and drops only what it created. Parallel
runs stay safe, so no suite needs `--test-threads=1` or a cross-suite lock.

### Per-suite target

The shared helper lives at `tests/support/genealogy_isolation.rs`. It builds a
run-unique name from a suite label plus a per-run token (process id, timestamp,
and an atomic counter):

```text
genealogy_<suite_label>_<pid>_<nanos>_<seq>_test
```

Two targets created in the same process with the same base DSN therefore always
get different names. PostgreSQL limits identifiers to 63 bytes, so a long suite
label is truncated to fit; the unique token and the mandatory `_test` suffix are
never truncated.

### Migration initialisation

The helper applies `GENEALOGY_MIGRATIONS`, the sorted list of every
`genealogy/migrations/*.sql` file from `0001` through `0022`, to the target it just
created. Suites no longer keep their own partial migration arrays; the one
exception is the schema-contract suite, which keeps its original `0001`-`0005`
scope so its assertions still describe the same contract.

### Cleanup ownership

The helper records the exact name it created. `cleanup()` drops only that owned
database (`DROP DATABASE ... WITH (FORCE)`), and the target also drops it from
`Drop`, so a failing assertion does not leave the database behind. It never issues
`DROP TABLE` or `TRUNCATE` against a shared table name, so no development or
production table can be reset by a test run. Pre-existing databases outside the
run are never touched.

### Failure behaviour

The helper requires `CREATE DATABASE`. It tries the `postgres` maintenance
database, then `template1`; if neither can create the database it panics with an
explicit message naming the suite, both attempted maintenance databases, and their
server-side errors. It never falls back to a shared database or a schema, so a run
fails loudly instead of silently touching a shared target.

### Database-creation capability (from configured validation commands)

The isolation mode is concluded from the configured commands, not from a direct
`psql` probe:

- `.github/workflows/ci.yml` sets `GENEALOGY_DATABASE_URL` to
  `postgres://rag:rag@127.0.0.1:5432/genealogy_test?sslmode=disable` and its
  `Create genealogy database and apply its migrations` step runs
  `psql "$DATABASE_URL" ... -c "CREATE DATABASE genealogy_test"`. That step is
  itself evidence that the CI role can create databases; locally, the isolated
  suites create and drop their own databases on the disposable cluster.
- The `Makefile` targets `rust-build`, `rust-test`, `rust-fmt`, `rust-clippy` run
  `cargo build|test|fmt|clippy` and never rely on a pre-created genealogy database;
  the suites create their own.
- `scripts/web-e2e.sh` requires `GENEALOGY_DATABASE_URL` and applies the migrations
  with `psql`, but it does not create the database, so a fresh disposable database
  is provided by the run environment.

Because the CI role can create databases, the helper works there. An environment
whose role lacks `CREATEDB` fails with the explicit message above; the fix is to
grant the test role `CREATEDB` on the disposable cluster, never to point the suite
at a shared database.

### Why there is no schema fallback

Genealogy migrations `0010_onboarding_foreign_keys.sql` and
`0011_onboarding_foreign_key_reconciliation.sql` resolve constraints through
`pg_constraint`, which is database-global rather than schema-scoped. Two suites
sharing one database under different schemas would therefore see each other's
constraints and produce false passes and false fails. Database-per-suite is the
only isolation the migration set supports.

### Validation commands

These commands exercise the isolated genealogy suites:

```sh
cargo build --locked
cargo test --locked
cargo test --locked --test genealogy_schema -- --nocapture
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

## Atomic ingestion

Replacement ingest is atomic: `ReplaceDocument` embeds every chunk **before**
the transaction opens and runs all batched `INSERT`s on a single transaction.
If any batch fails, the transaction rolls back the whole replacement — including
the `DELETE` of the previous document — so a failing batch leaves the previously
stored document untouched. This contract is exercised by
`internal/ingestion`'s integration tests (`TestReplaceDocumentIntegration*`,
including `TestReplaceDocumentIntegrationLargeReplacementRollsBackOnLaterBatchFailure`)
and by `internal/rag`'s `TestE2EIngestReplacementIsAtomicDBIntegration`.

## Reliability paths (failure / cancellation / timeout)

The `internal/rag` reliability tests exercise the generation failure, request
cancellation, and request timeout paths. They run entirely in memory with the
deterministic stub generator and **do not touch any database** — so they can
never reach `rag_db`. See `internal/rag/reliability_test.go`.

## Optional: real-Ollama smoke testing (non-gating)

Real-Ollama smoke testing is **optional and reported separately**. It is **not**
required by the standard suite and **never** gates a commit, push, or merge.

When you want to exercise the real provider (a running Ollama server), run it by
hand with the network model available and record the result separately from the
suite's pass/fail:

```sh
# Example: point the commands at a running Ollama and exercise the path.
OLLAMA_URL=http://127.0.0.1:11434 cargo run --locked --bin query
```

Because it needs a live model, it is excluded from `cargo test --locked`; the stub
provider/generator covers the deterministic, network-free path in the suite.

## Provider selection

The commands resolve the embedder and generator through the provider registry
(`EMBED_PROVIDER` / `GEN_PROVIDER`). An unregistered provider name fails fast at
`config.Load`, naming the variable and value; an unset value keeps the default
(`ollama`).
