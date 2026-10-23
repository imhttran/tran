# TDI-101 — Genealogy integration-test database isolation: root-cause investigation

Status: static investigation only. No `cargo`, `psql`, or other command was run by
this task. Runtime evidence comes from the SOP validation stage.

## 1. Scope and method

This report is a **static** reading of the genealogy integration test suites and
their shared PostgreSQL fixtures. Each source claim cites `file:line`. Anything that
needs a live database appears in section 9 as unverified.

## 2. `tests/genealogy_schema.rs` setup and teardown

- DSN selector: `tests/genealogy_schema.rs:20-21` — `genealogy_dsn()` reads
  `GENEALOGY_DATABASE_URL` via `std::env::var`, trims it, and returns `None` when
  it is unset or empty.
- Suffix guard: `tests/genealogy_schema.rs:30-33` — asserts that the database name
  ends with `_test` or `_disposable`.
- RAG guard: `tests/genealogy_schema.rs:34-47` — refuses a DSN whose
  `(dbname, hosts, ports)` equals `DATABASE_URL`.
- Migrations: `tests/genealogy_schema.rs:11-17` lists five migration files;
  `apply_migrations()` is at `tests/genealogy_schema.rs:79-88`.
- Connection helper: `connect()` at `tests/genealogy_schema.rs:49-51`.
- **Destructive reset function:** `tests/genealogy_schema.rs:53-77` —
  `reset_schema(client: &mut Client)` runs one `batch_execute` of
  `DROP TABLE IF EXISTS ... CASCADE;` for the genealogy tables
  (`assertion_evidence`, `assertion`, `parent_child_link`, `spouse_link`,
  `catalog_entry`, `family_branch`, `source_evidence`, `source_document`,
  `gravesite`, `business_name`, `genealogy_fact_proposal_event`,
  `genealogy_fact_proposal`, `person_name_variant`, `person`,
  `genealogy_schema_version`).
- Call sites in the single test `migrations_constraints_and_rollback_are_enforced`:
  `reset_schema(&mut client);` at `tests/genealogy_schema.rs:91` (setup, before
  `apply_migrations`) and again at `tests/genealogy_schema.rs:200` (teardown).
- Target database: this suite connects to the configured DSN **directly** via
  `connect(&dsn)` at `tests/genealogy_schema.rs:90`. It does **not** derive a
  per-suite name and does **not** call `CREATE DATABASE`. In CI the configured DSN
  names the shared `genealogy_test` database (section 5).

## 3. Every other suite that mutates schema or selects the genealogy DSN

All suites below read `GENEALOGY_DATABASE_URL` and refuse a database name that does
not end with `_test` or `_disposable`. Each defines its own `reset_schema()` with
`DROP TABLE ... CASCADE`. They differ in whether they derive a dedicated database.

| Suite | DSN function | Suffix guard | Create-DB helper | `reset_schema` (DROP ... CASCADE) | Target DB |
|---|---|---|---|---|---|
| `tests/genealogy_schema.rs` | `genealogy_dsn()` :20-21 | :30-33 | none | :53-77 (calls :91, :200) | **base DSN directly** (`genealogy_test` in CI) |
| `tests/genealogy_appointments.rs` | `base_dsn()` :16-24 | :21-24 | `ensure_database()` :41-57; `with_database()` :26-39 | :59-78 (calls :171, :195, :216, :238) | derived `genealogy_appointments_*_test` |
| `tests/genealogy_cli.rs` | `base_dsn()` :15-23 | :20-23 | `ensure_database()` :40-56; `with_database()` :25-38 | :58-76 (calls :101, :174) | derived `genealogy_cli_test` |
| `tests/genealogy_provenance.rs` | `base_dsn()` :20-28 | :26-28 | `ensure_database()` :46-62; `with_database()` :31-44 | :63-81 (call :168) | derived `genealogy_provenance_test` |
| `tests/genealogy_repairs.rs` | `base_dsn()` :20-28 | :26-28 | `ensure_database()` :46-62; `with_database()` :31-44 | :64-83 (calls :160, :345) | derived `genealogy_repair_test`, `genealogy_reimport_test` |
| `tests/genealogy_migration_0011.rs` | `base_dsn()` :59-68 | :62-66 | `ensure_database()` :81-105; `with_database()` :71-79 | :107-142 (calls :191, :225, :256) | derived `genealogy_migration_0011_fresh_test`, `_legacy_test`, `_orphan_test` |
| `tests/genealogy_migration_0012.rs` | `base_dsn()` :30-38 | :34-37 | `ensure_database()` :55-79; `with_database()` :41-52 | :81-116 (calls :131, :187) | derived `genealogy_migration_0012_test` |

The other integration tests in `tests/` read `DATABASE_URL` (RAG / identity side),
not `GENEALOGY_DATABASE_URL`, and skip cleanly when it is absent. They do not touch
the genealogy schema. Examples: `tests/admin_members.rs`, `tests/document_lifecycle.rs`,
`tests/document_processing.rs`, `tests/rag_publication.rs`, `tests/worker_reliability.rs`,
`tests/family_profile_media.rs`, `tests/onboarding_link.rs`, `tests/onboarding_flow.rs`.
This report does not enumerate their line ranges because they are outside the
shared genealogy database.

## 4. Exact files, functions, and line references for destructive setup/cleanup

The destructive operation in every genealogy suite is `DROP TABLE IF EXISTS ... CASCADE`:

- `tests/genealogy_schema.rs:53-77` — `reset_schema()`; setup call at :91, teardown
  call at :200. **Uses the base DSN without a derived database name.**
- `tests/genealogy_appointments.rs:59-78` — `reset_schema()`; per-test calls at
  :171, :195, :216, :238 (no teardown reset).
- `tests/genealogy_cli.rs:58-76` — `reset_schema()`; setup call at :101, teardown
  call at :174.
- `tests/genealogy_provenance.rs:63-81` — `reset_schema()`; setup call at :168,
  teardown call follows the final assertions.
- `tests/genealogy_repairs.rs:64-83` — `reset_schema()`; setup calls at :160 and
  :345, teardown calls after the final assertions.
- `tests/genealogy_migration_0011.rs:107-142` — `reset_schema()`; setup calls at
  :191, :225, :256.
- `tests/genealogy_migration_0012.rs:81-116` — `reset_schema()`; setup calls at
  :131, :187.

Database creation (schema-owning, not destructive) is done by `ensure_database()`
in each derived suite via `CREATE DATABASE <name>`:

- `genealogy_appointments.rs:41-57`
- `genealogy_cli.rs:40-56`
- `genealogy_provenance.rs:46-62`
- `genealogy_repairs.rs:46-62`
- `genealogy_migration_0011.rs:81-105`
- `genealogy_migration_0012.rs:55-79`

## 5. How each suite selects its database name and DSN

- **Environment variables**
  - `GENEALOGY_DATABASE_URL` — the base DSN read by every genealogy suite
    (`genealogy_schema.rs:20-21`, `genealogy_appointments.rs:16-17`,
    `genealogy_cli.rs:15-16`, `genealogy_provenance.rs:20-21`,
    `genealogy_repairs.rs:20-21`, `genealogy_migration_0011.rs:59-62`,
    `genealogy_migration_0012.rs:30-34`).
  - `DATABASE_URL` — read by the RAG guard in `genealogy_schema.rs:34-47` and by
    the RAG/identity suites.
- **Suffix rule:** every genealogy suite refuses a target whose database name does
  not end with `_test` or `_disposable` (e.g. `genealogy_schema.rs:30-33`,
  `genealogy_appointments.rs:21-24`, `genealogy_migration_0011.rs:62-66`,
  `genealogy_migration_0012.rs:34-37`).
- **Helpers:** `with_database(base, name)` rewrites the path segment of the base
  DSN to a new database name while preserving the `?...` query string
  (`genealogy_appointments.rs:26-39` and copies in the other suites).
  `ensure_database(admin_dsn, db)` runs `CREATE DATABASE` when the name is missing.
  The migration suites (`0011`, `0012`) also validate that the derived name is
  alphanumeric or underscore before interpolating it.
- **Base-DSN suite:** `genealogy_schema.rs` connects to whatever
  `GENEALOGY_DATABASE_URL` names. It does not derive a distinct name and does not
  call `ensure_database`. In CI this is the shared `genealogy_test`.
- **Hard-coded database names in code**
  - `genealogy_cli_test` — `tests/genealogy_cli.rs:99,101`.
  - `genealogy_provenance_test` — `tests/genealogy_provenance.rs:158,163`.
  - `genealogy_appointments_approval_test`, `genealogy_appointments_self_approval_test`,
    `genealogy_appointments_ineligible_test`, `genealogy_appointments_concurrent_test` —
    `tests/genealogy_appointments.rs:172,201,229,251`.
  - `genealogy_repair_test`, `genealogy_reimport_test` — `tests/genealogy_repairs.rs:157,342`.
  - `genealogy_migration_0011_fresh_test`, `_legacy_test`, `_orphan_test` —
    `tests/genealogy_migration_0011.rs:15-17`.
  - `genealogy_migration_0012_test` — `tests/genealogy_migration_0012.rs:11`.
- **Hard-coded DSN in CI:** `.github/workflows/ci.yml:34` sets
  `GENEALOGY_DATABASE_URL: postgres://rag:rag@127.0.0.1:5432/genealogy_test?sslmode=disable`.
  `.github/workflows/ci.yml:37-41` (`Create genealogy database and apply its migrations`)
  runs `CREATE DATABASE genealogy_test` and applies every `genealogy/migrations/*.sql`
  with `psql` against that DSN. `DATABASE_URL` is `rag_test` (`ci.yml:32`).

## 6. Shared fixtures, environment variables, mutexes, and hard-coded names

- **Shared fixture files:** the SQL migrations under `genealogy/migrations/`.
  `genealogy_schema.rs:11-17` and `genealogy_provenance.rs:9-15` and
  `genealogy_repairs.rs:9-15` load the same `0001..0005` file set;
  `genealogy_appointments.rs:80-103` uses `include_str!` of `0001..0006`;
  `genealogy_migration_0011.rs:24-36,39-49` and `genealogy_migration_0012.rs:13-25`
  use the longer `0001..0011` / `0001..0012` sets.
- **Shared environment variables:** `GENEALOGY_DATABASE_URL`, `DATABASE_URL`.
- **DB_LOCK mutex:** exactly one suite defines a lock.
  `tests/genealogy_migration_0012.rs:12` declares
  `static DB_LOCK: Mutex<()> = Mutex::new(());` and each of its two tests takes it
  with `let _guard = DB_LOCK.lock()...;` at `:127` and `:183`. No other suite
  defines or acquires a lock. `genealogy_schema.rs` in particular has **no**
  lock, so nothing serializes it against the other base-DSN contenders.
- **Hard-coded database names:** listed in section 5. The single shared name is
  `genealogy_test` from `.github/workflows/ci.yml:34`.

## 7. Interference mechanism (hypothesis grounded in code)

**Hypothesis.** Under parallel `cargo test`, `tests/genealogy_schema.rs` connects to
`GENEALOGY_DATABASE_URL` **directly** (`genealogy_schema.rs:90`) and issues
`DROP TABLE IF EXISTS ... CASCADE` both at setup (`:91`) and teardown (`:200`)
while the DSN in CI is the shared `genealogy_test` (`ci.yml:34`). No other suite uses
the bare `genealogy_test` name after derivation, **but** CI's `Apply genealogy
migrations` step (`ci.yml:37-41`) and the SOP validation `psql` command both pre-load
migrations into `genealogy_test` against the same DSN. Two concrete collision paths
follow from the code:

1. `genealogy_schema.rs:91` drops tables that the CI pre-load step (or any external
   `psql` caller hitting `genealogy_test`) expects to exist, and recreates them via
   `apply_migrations`. If a second connection is reading `genealogy_test` at that
   moment it sees missing relations or in-flight DDL.
2. `genealogy_schema.rs:200` drops the tables again at teardown. A concurrent
   re-application or inspection of `genealogy_test` sees `relation does not exist`.

Inside a single test binary `cargo test` runs tests on threads. The schema suite is
one test and it holds the only connection, so within-binary contention is not the
mechanism. The contention is **cross-binary and cross-process**: the SOP validation
command applies `genealogy/migrations/0001..0005` twice with `psql` against
`genealogy_test`, then runs `cargo test --test genealogy_schema`. Both the `psql`
invocations and the test drop and recreate the same tables on the same database.
`genealogy_migration_0012.rs` protects itself with `DB_LOCK` (:12, :127, :183);
`genealogy_schema.rs` has no such guard.

This is a **hypothesis grounded in the source**. It rests on two code facts:
(a) `genealogy_schema.rs` mutates a database it does not own uniquely, and
(b) no lock serializes it against the `psql` migration step or any other caller.

## 8. Minimal recommended isolation approach (next stage)

Smallest reliable fix, reusing the pattern already present in every derived suite:

1. Change `tests/genealogy_schema.rs` to derive its own database via
   `with_database(base, "genealogy_schema_test")` and create it with
   `ensure_database(&with_database(&base, "postgres"), "genealogy_schema_test")`,
   mirroring `genealogy_appointments.rs:26-57` and `genealogy_cli.rs:25-56`.
2. Keep `reset_schema()` and `apply_migrations()` unchanged, but run them against
   the derived DSN, so the `DROP ... CASCADE` at :91 and :200 no longer touches
   `genealogy_test`.
3. Optionally add `static DB_LOCK: Mutex<()> = Mutex::new(());` as in
   `genealogy_migration_0012.rs:12` only if the derived name is reused across
   invocations; per-invocation `_test` names make the lock unnecessary.
4. Keep the `_test`/`_disposable` suffix guard and the RAG-database guard.
5. Do not change CI or the SOP `psql` step. `genealogy_test` becomes a bootstrap
   database only; every schema-mutating suite owns its own disposable database.

Per-suite databases preserve parallel execution without `--test-threads=1`.

## 9. Unverified pending SOP validation stage

The following are **not** established by this static report and must be confirmed
or refuted by SOP validation output:

- Whether the interference described in section 7 actually shows up under the SOP
  validation run. No suite was executed by this task.
- Which caller of `genealogy_test` actually collides with `genealogy_schema.rs`
  (the `psql` pre-load step, the CI pre-load step, or a parallel developer run).
  The source points to the `psql` step as the strongest candidate; only runtime
  confirms it.
- Teardown timing: whether `genealogy_schema.rs:200` `reset_schema()` runs before,
  during, or after a `psql` migration of the same database.
- The outcome of `cargo build --locked`, `cargo test --locked`, the `psql`
  migration+`genealogy_schema` command, `cargo fmt --check`, and
  `cargo clippy --locked --all-targets -- -D warnings`. The harness refuses these
  commands; their results come only from SOP validation.
- Whether a green `cargo test --locked` requires fixing the pre-existing
  environment issue at `tests/expbilingual_binary.rs:53-58`, which fails when
  `DATABASE_URL` is unset. That is a separate, pre-existing condition unrelated to
  genealogy isolation and is outside this report's scope; it must not be treated
  as evidence for or against the hypothesis in section 7.

Once the validator runs, only the observed evidence should replace these bullets. No
command is claimed that the harness refused.
