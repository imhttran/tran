# Testing guides

This guide covers how to run the test suite, the disposable-PostgreSQL harness
used by the database integration tests, and the optional real-Ollama smoke test.

## The standard suite (no database, no network, no model)

```sh
make test        # resolve + prepare disposable databases, then cargo test
```

`cargo test --locked` runs the unit tests and every integration target. Unit
tests need no database and no model; deterministic stub providers back the
in-process paths, so identical inputs produce identical results. `make test` calls
`scripts/test-db.sh exec`, which resolves the test databases, creates and
migrates them if needed, exports them, and sets `REQUIRE_TEST_DATABASE=1`, so a
missing database fails the suite instead of skipping green.

Running `cargo test --locked` directly is still supported for a database-free
unit run; database-backed suites skip when their DSN is unset unless
`REQUIRE_TEST_DATABASE=1` is set. The RAG schema suites always fail when
`DATABASE_URL` is unset or lacks pgvector.

## Portable database configuration

Explicit configuration always wins. `scripts/test-db.sh` reads, in order, the
process environment, `.env`, and an explicit test override:

- `DATABASE_URL` / `GENEALOGY_DATABASE_URL` — the application or test DSNs.
- `TEST_DATABASE_URL` / `TEST_GENEALOGY_DATABASE_URL` — explicit disposable test
  targets. When unset, `<name>_test` is derived from the DSN above.

When `DATABASE_URL` is absent, the script probes a server and never guesses a
password: **Linux tries native PostgreSQL on 5432 first, macOS tries the Docker
compose port first**, and 5434 (this repository's documented default) stays in
the probe list. A reachable server is used as-is. If none answers, the script
fails with setup instructions rather than skipping tests.

```sh
scripts/test-db.sh status    # show the resolved test targets, no credentials
scripts/test-db.sh prepare   # create (if absent) and migrate the test targets
scripts/test-db.sh exec -- cargo test --locked
```

`prepare` only creates databases and applies idempotent migrations. It never
drops, truncates, or resets a database, and it refuses any name that does not end
in `_test` or `_disposable`.

## Disposable PostgreSQL (never `rag_db`)

The integration tests run against **throwaway** databases, never a persistent
application database. The harness conventions are:

- **`DATABASE_URL`** points at the disposable RAG database; `.env.example`
  defaults to `postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable`.
- **`GENEALOGY_DATABASE_URL`** points at the genealogy server DSN. Each suite
  derives a run-unique `*_test` database from it, so the role needs
  `CREATE DATABASE`.
- **Never a shared database.** The RAG schema tests write and delete rows, so
  point them at a disposable database. The genealogy suites already isolate
  themselves per suite. Never point the destructive tests at the development
  database (`rag`) or a database named `rag_db`.
- **Guard for the browser suite.** `scripts/e2e-db-env.sh` refuses an
  application database for `make web-e2e`; `cargo test` does not use it.

### Setup

```sh
make db-up            # Linux: require native PostgreSQL; macOS: docker compose
make test-db-up       # create and migrate only the disposable test databases
```

`make db-up` follows `DB_BACKEND`: `native` requires an already-running server,
`docker` starts `docker compose up -d --wait`, and with it unset an already
reachable server is left alone (macOS falls back to Docker, Linux to native).
Native PostgreSQL is never started by this project on Linux.

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

The test runner connects to the disposable test DSNs and applies every
`migrations/*.sql` (RAG) or `genealogy/migrations/*.sql` (genealogy) file in name
order (idempotent), so it works against a fresh database and picks up migrations
added after `001`.

### Teardown

```sh
make db-down          # stop and remove the disposable PostgreSQL container
```

## Genealogy integration-test isolation (TDI-002)

The genealogy suites that mutate the genealogy schema no longer share one database.
Each suite derives its own disposable target from `GENEALOGY_DATABASE_URL` and
drops only what it created. Parallel runs stay safe, so no suite needs
`--test-threads=1` or a cross-suite lock.

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

The helper hashes the full sorted `GENEALOGY_MIGRATIONS` set and builds one
schema template named `genealogy_schema_<hash>_test` the first time that hash is
used. A marker row records that the template is complete. Each per-suite target
is then created with `CREATE DATABASE ... TEMPLATE`, which is a file copy rather
than a replay of 30 fsync-bound migrations.

The schema-contract suite deliberately uses **an empty target**
(`create_empty_isolated_target`) and applies its original `0001`-`0005` subset, so
its assertions still describe the same contract. A changed migration file
produces a new template name, so a stale template is never served; old templates
are inert and can be dropped by an operator.

### Why the template exists

Replaying the full migration set per test dominated the suite: measured here at
about **24 s per test**, with another **6 s** to drop the database. Fourteen
authority tests took 343 s serially and 228 s in parallel because the work is
DDL-bound. Copying a migrated template takes under a second, and disabling
`synchronous_commit` on the disposable admin connection cuts the drop to about
0.6 s. The same authority suite now runs in about 30 s. This changes only test
infrastructure: every test still gets its own uniquely named database, and no
assertion or migration is skipped.

### Interrupted runs

Suites that shared one base database previously relied on `Drop` to delete their
rows by prefix. A killed process skipped that cleanup, and a later run's
unfiltered review-queue query could see the leftovers. `onboarding_profile_revisions`
now uses the per-run isolated target like the other genealogy suites, so an
interrupted run leaves only an inert `_test` database for that run.

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
server-side errors, and it states that the role must hold `CREATEDB`. It never
falls back to a shared database or a schema, so a run fails loudly instead of
silently touching a shared target. If a template copy is refused (for example a
concurrent connection keeps the template busy), the helper falls back to creating
an empty database and replaying the migrations, which is slower but still
isolated.

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

### Real-browser E2E database selection

`scripts/web-e2e.sh` migrates, seeds, and deletes rows in **both** databases, so
its selection rules are strict. They live in `scripts/e2e-db-env.sh`:

- **Caller values win.** `load_env_defaults` reads `.env` only to fill variables
  that are not already set. `DATABASE_URL=.../rag_e2e_test make web-e2e` keeps
  that database even when `.env` defines a different one. Isolation never works
  by moving `.env` aside.
- **Disposable only.** Both `DATABASE_URL` and `GENEALOGY_DATABASE_URL` must name
  a database whose name ends in `_test` or `_disposable`.
- **Refused before any write.** An application database (for example `rag` or
  `tran_genealogy`) stops the run before the first migration, seed, or delete.
- **Distinct targets.** The RAG and genealogy DSNs must not reach the same
  database.

The guard itself is covered without a database:

```sh
scripts/test-e2e-db-env.sh
```

### Real-browser E2E ports

`scripts/web-e2e.sh` starts three servers on loopback: the test OIDC issuer
(`E2E_AUTH_PORT`, default 9099), the API (`API_PORT`, default 8080), and Next.js
(`WEB_PORT`, default 3000). Before any migration, seed, or delete it checks each
port with `scripts/port-guard.sh` (lsof, then ss, then a connect probe, so it
works on macOS and Linux). A port that something else already holds stops the
run with the listener's pid and the variable to override; nothing already
running is stopped. A Next.js server that `scripts/web-control.sh` manages is
still stopped and restarted, as before.

The readiness checks confirm the run's own process is alive before accepting a
response, and the run pins `API_URL` (the Next.js proxy target) to its own API,
overriding the developer value in `.env`. Without these, a developer API on 8080
connected to the application databases answered the health check and received
the suite's requests. To keep a developer stack running, move the run instead:

```sh
API_PORT=8081 make web-e2e
```

The guard is covered with one throwaway listener and no database:

```sh
scripts/test-port-guard.sh
```

Because the CI role can create databases, the helper works there. An environment
whose role lacks `CREATEDB` fails with the explicit message above; the fix is to
grant the test role `CREATEDB` on the disposable cluster, never to point the suite
at a shared database.

### Deterministic browser fixtures

`scripts/web-e2e.sh` truncates its synthetic narrative tables
(`source_narrative_review_event`, `source_narrative_link`,
`source_narrative_passage`) before seeding the branch-review fixture. The E2E
database is disposable, and an interrupted run can leave rows that the seed's
narrow `DELETE`s do not match; those leftovers changed the review queue from two
pages to three. `TRUNCATE ... RESTART IDENTITY CASCADE` bypasses the append-only
row triggers and guarantees the fixture count on every run without changing any
assertion or expected value.

For a focused rerun, the harness accepts Playwright arguments through
`PLAYWRIGHT_ARGS`; the seed and servers still run, so the fixture is complete:

```sh
PLAYWRIGHT_ARGS='e2e/narrative-review.spec.ts -g filters' make web-e2e
```

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
stored document untouched. This contract is exercised by `tests/ingest_pipeline.rs`
(`forced_replacement_failure_rolls_back_and_preserves_stored_data` and
`repeated_ingestion_of_an_unchanged_source_preserves_stored_data`) and by the
RAG schema tests in `tests/storage_interop.rs`.

## Reliability paths (failure / cancellation / timeout)

The answer-service tests exercise retrieval failure, generation failure, and the
insufficient-evidence refusal path in memory with the deterministic stub
provider; they **do not touch any database**. See the `#[cfg(test)]` module in
`src/answer/service.rs` (`retrieval_dependency_failure_is_reported`,
`generation_dependency_failure_is_reported`,
`insufficient_evidence_refuses_without_generation`). Worker lease, fencing, and
heartbeat failure behaviour is covered by `tests/worker_reliability.rs`.

The generation timeout is a configuration bound (`REQUEST_TIMEOUT`, enforced by
the Ollama HTTP client); there is no separate cancellation test.

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
