# Phase 0 Baseline Tests and Environment Limitations

This report records the baseline test commands and their environment
limitations. It does not add production secrets.

## Prior passing evidence

Prior passing check evidence was searched for in the SOP run artefacts. No
readable prior baseline results were found in the repository during discovery.
Inputs for this stage are unchanged code plus new report files, so the stage
records the required commands as the baseline. Where a command cannot run in
this environment, the concrete limitation is recorded below.

## Required baseline commands

| Command | Purpose |
| --- | --- |
| `cargo build --locked` | Compile the crate with the lockfile |
| `cargo test --locked` | Run Rust unit and integration tests |
| `cargo fmt --check` | Check formatting |
| `cargo clippy --locked --all-targets -- -D warnings` | Lint with warnings as errors |
| `make rust-check` | Aggregate Rust checks |
| `make web-check` | Web type and lint checks |
| `make web-e2e` | Playwright end-to-end tests |
| migrations apply (see below) | Apply `migrations/*.sql` to PostgreSQL |

## Environment limitations

- **PostgreSQL and pgvector**: several tests and the migration apply step need a
  running PostgreSQL with the pgvector extension. `docker-compose.yml` defines
  the service. Where a database is not reachable, database-backed tests and the
  migration apply step cannot run; this is a concrete environmental limitation,
  not a code failure.
- **Ollama model server**: health, ingestion, and query paths call a local Ollama
  server. Where no model server is reachable, those checks report degraded or
  unavailable dependencies. This is a concrete environmental limitation.
- **Playwright browsers**: `make web-e2e` needs the Playwright browser binaries
  installed. Where they are absent, the e2e run reports a concrete environmental
  limitation.

SOP runs the required validation commands independently after this stage and
records the actual results. This report records the commands and the known
environment constraints.

## Secret safety

No production secret is written to this report or to other report files. OIDC
issuer, audience, and TLS settings stay operator-supplied environment values and
are not committed.
