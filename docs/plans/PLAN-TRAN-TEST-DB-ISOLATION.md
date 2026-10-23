# Implementation Plan

## Project

Tran genealogy — integration-test database isolation

## Summary

Remove database interference between genealogy integration tests so the suite is
deterministic and repeatable under parallel `cargo test`, without shared-database
resets, unnecessary global serialization, or any production behavior change. The
SOP agent performs static repository inspection and all code changes; the SOP
orchestrator validation stage executes build, test, lint, and integration
commands and supplies runtime evidence.

## TDI-101 — Investigate and document the root cause

Perform a **static** investigation and write the findings to
`docs/reports/genealogy-test-db-isolation.md`. This task is bounded: read the
relevant setup files once, record what you find, write the report, and finish. Do
not re-read the same files without producing output, and do not attempt to run
`cargo`, `psql`, or any other command — the `tool` harness refuses them, and
runtime evidence comes from the SOP validation stage.

The report must contain:

- The database setup and teardown behaviour in `tests/genealogy_schema.rs`,
  including the exact function that drops or resets tables.
- Every other integration test that shares the same database or schema, and what
  each one creates, truncates, resets, migrates, or drops.
- The exact files, functions, and line references responsible for destructive
  setup or cleanup.
- How each suite selects its database name and DSN (environment variables,
  helpers, suffix rules, hard-coded names).
- The likely interference mechanism, stated as a hypothesis grounded in the code.
- A minimal recommended isolation approach for the next stage.
- What remains unverified until the SOP validation stage runs.

Separate source-code findings (with `file:line`) from anything that needs runtime
confirmation.

### Dependencies

None

### Deliverables

- `docs/reports/genealogy-test-db-isolation.md` containing every item listed above.

### Acceptance Criteria

- The report exists at `docs/reports/genealogy-test-db-isolation.md`.
- It names `tests/genealogy_schema.rs` `reset_schema()` and every other schema-mutating or DSN-selecting test with file and line references.
- It lists the shared fixtures, environment variables, DB_LOCK mutexes, and hard-coded database names it found.
- It distinguishes source-code findings from conclusions that still need validator output, and does not claim to have run commands the harness refuses.
- The task ends after writing the report; it does not loop on repeated inspection.

## TDI-002 — Design the smallest reliable fix

Design the isolation approach before implementation. Prefer a unique disposable
database (or an isolated schema where createdb is unavailable) per independent
suite that mutates schema. Reuse a small helper only where it removes real
duplication. Initialise migrations against the isolated target. Make cleanup
remove only the resources the run owns. Keep fixtures deterministic and independent
of ordering so suites need no cross-suite locking.

Determine whether the DSN role can create databases from the configured validation
commands, not by running `psql` directly. If database creation is unavailable,
choose a safe schema-level strategy and document the tradeoff.

### Dependencies

- TDI-101

### Deliverables

- Isolation design: per-suite target, migration initialisation, cleanup ownership, and failure behaviour.
- List of files to change and the helper (if any) to extract.
- Documented tradeoff when database creation is unavailable.

### Acceptance Criteria

- The design isolates every suite that mutates genealogy schema, or documents with evidence why a suite does not need isolation.
- Cleanup can only remove resources owned by the run; no shared development or production table is dropped, truncated, or reset.
- The design keeps parallel execution and does not rely on `--test-threads=1`.
- No new database-testing framework is introduced unless the existing approach cannot support isolation.

## TDI-003 — Implement isolated test databases

Implement the design in the test infrastructure only. Fix
`tests/genealogy_schema.rs` and every other affected setup path so each mutating
suite owns its target. Keep existing assertions and schema-contract coverage.
Tests must refuse a target that is not a `_test` or `_disposable` database. Add the
regression tests required by TDI-004 so the SOP validator can execute them.

Do not change application logic, APIs, migrations, or genealogy business rules.

### Dependencies

- TDI-002

### Deliverables

- Code changes to the genealogy test setup paths and any shared helper.
- Regression tests for the isolation scenarios in TDI-004.
- Guard that rejects a non-disposable database target.

### Acceptance Criteria

- `tests/genealogy_schema.rs` can no longer reset or drop state owned by another suite.
- No application, API, migration, or genealogy business-rule file is modified.
- A test run cannot target a non-`_test`/`_disposable` database.
- Existing assertions and schema-contract coverage are preserved.

## TDI-004 — Prove isolation under parallel execution

Write the regressions that prove the isolation and record, for each one, the exact
command the SOP validator must run. Prove that two independent genealogy suites run
concurrently without interference; that one suite's cleanup cannot remove another
suite's tables or records; that migrations initialise correctly in the isolated
target; that repeated runs leave no conflicting state; and that a failed test does
not damage a shared database.

Runtime results come from the SOP validation stage. Do not claim a runtime result
that the validator has not produced.

### Dependencies

- TDI-003

### Deliverables

- Executable regression tests for each isolation scenario.
- The exact validator command line for each regression.
- A record of the validator's observed results once available.

### Acceptance Criteria

- Each isolation scenario has an executable test, not only prose.
- The recorded commands run against the designated disposable database only.
- Reported results come from actual validator output; static inspection or compilation alone does not satisfy this criterion.
- A scenario whose command cannot run is reported with the exact command and blocker instead of being marked complete.

## TDI-005 — Run project verification gates

Run the project gates through the SOP validation stage: `cargo fmt --check`,
`cargo clippy --locked --all-targets -- -D warnings`, the targeted integration
suites plus repeated and parallel runs, `sop validate`, and `sop review`. Web or API
checks run only if the changes affect them.

Report actual command output. If the orchestrator cannot execute a required check,
report the exact command and the blocker rather than marking the task complete.

### Dependencies

- TDI-004

### Deliverables

- Validation results for every applicable gate, with the exact command and outcome.
- Recorded parallel and repeated test runs.
- Explicit list of any check that could not run, with its blocker.

### Acceptance Criteria

- Every applicable gate is executed through the validation stage and its actual output recorded.
- No skipped, interrupted, or unavailable check is reported as passing.
- Parallel and repeated runs demonstrate deterministic isolation behaviour.

## TDI-006 — Deliver the focused PR

Commit the verified changes on `fix/genealogy-test-db-isolation`, push the branch,
and open one focused pull request against current `main`. Include the root cause,
the isolation design, the files changed, the validator evidence, the cleanup
guarantees, and the SOP validation and review results.

Do not merge or deploy automatically.

### Dependencies

- TDI-005

### Deliverables

- Pushed branch and one pull request against `main`.
- PR description covering root cause, design, files, evidence, and remaining risks.
- GitHub CI result.

### Acceptance Criteria

- One PR targets `main` from `fix/genealogy-test-db-isolation`.
- The PR reports actual validation and CI results, with no fabricated or skipped checks.
- The PR is not merged or deployed.
