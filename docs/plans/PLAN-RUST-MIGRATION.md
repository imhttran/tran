# Rust Migration and Learning Plan

Repository: https://github.com/imhttran/rag-template-rust
Target path: `docs/plans/PLAN-RUST-MIGRATION.md`
Created: 2026-10-17
Status: ACTIVE — RUST-001 through RUST-007C verified; RUST-008 is pending.

## Project

rag-template-rust — port the implemented Go RAG project to Rust.

## Summary

Convert the implemented Go RAG project to idiomatic Rust. Preserve observable behavior and use each phase to learn Rust. Keep the Go implementation as the reference until Rust passes parity checks. Scope and the execution contract below are canonical; the stages below are the machine plan SOP executes.

## Goal

Convert the implemented Go RAG project to idiomatic Rust. Preserve observable behavior and use each phase to learn Rust. Keep the Go implementation as the reference until Rust passes parity checks.

The existing `docs/requirements/PRD.md` remains the product requirements source. Use `docs/architecture/OVERVIEW.md` and the configuration, commands, capabilities, and testing references to establish the migration contract. Confirm them against code before execution.

## Scope

Port all implemented capabilities used by `cmd/ingest`, `cmd/rag`, `cmd/eval`, and `cmd/expbilingual`. Preserve SQL migrations, environment variable names and defaults, dataset formats, metadata, provider selection, and documented failure behavior.

Exclude OCR, genealogy entity extraction, additional providers, retrieval redesign, and performance optimization. Do not introduce a new RAG framework. Reuse established libraries for infrastructure where they preserve the required contracts.

RUST-008 adds a new Next.js + Axum user interface that is **not** part of the Go baseline (which has no browser UI). It is a deliberate addition, not a port: it calls the same Rust services as the CLI and reuses the reference web pattern from the `inventory` project (Next.js -> Axum -> PostgreSQL; the browser talks only to Next.js; `/api/*` is proxied server-side) plus the baseline's `rag-retrieval/v1` query contract where practical.

## Execution contract

- Work in an isolated migration branch or worktree. Preserve unrelated changes.
- Keep Go runnable during migration. Do not remove it before RUST-007 passes.
- Complete tasks in dependency order. Continue automatically through implementation, verification, and documentation until completion or a real blocker.
- Make routine implementation decisions within scope. Record material tradeoffs.
- Use one Cargo package with library modules and thin command binaries initially. Add crates only when a concrete boundary requires them.
- Pin the Go reference commit and fixtures. Never regenerate expected outputs from Rust to make a failing parity check pass.
- Use mocks and fixed vectors for deterministic checks. Use separate, explicitly selected test databases for writes.
- Never use production data, reset a shared database, or change the schema destructively as part of verification.
- Record task evidence and a short Rust learning note after each task.
- Preserve repository approval rules for publishing and merging. Do not assume this file authorizes either action.

The task contracts below were converted to SOP's canonical plan schema
(stages restated as `- **Objective:**`, `- **Depends on:**`, and
`- **Acceptance:**` fields) on 2026-10-17 without changing any criterion.
RUST-001 through RUST-008 and their dependencies are retained. RUST-005 was
split into RUST-005A (storage foundation) and RUST-005B (ingestion pipeline) on
2026-10-17, preserving all of its acceptance criteria, with RUST-006 depending on
RUST-005B. RUST-005C records the completed storage client and interoperability
work. RUST-005D is a verification-only follow-up added on 2026-10-17 because
RUST-005C's historical outcome remains LOCAL_DONE/PARTIAL and its continuation
budget is exhausted: it re-verifies the storage evidence without reimplementing
it, and RUST-005B depends on it so downstream work cannot advance before the
storage verification passes.

On 2026-10-17 a second repair added bounded follow-ups because RUST-005B and
RUST-006 reached `VERIFIED` from the whole-suite check while their substantive
free-text criteria were marked informational and production wiring was still
missing: **RUST-005E** (production ingestion service seam + acceptance crate) and
**RUST-005F** (production ingest binary/config/adapter wiring), **RUST-006B**
(retrieval and cited-answer CLI/service wiring), and **RUST-006C** (real
end-to-end verification). **RUST-007** keeps its ID and execution record but is
now the production `eval` command and deterministic-parity stage; **RUST-007D**
is the single bilingual-harness follow-up; **RUST-007B** freezes
evidence and updates pre-removal tooling while Go remains; **RUST-007C** performs
the Rust-only transition and final no-Go verification. RUST-008 depends on
RUST-007C. The machine plan was validated with SOP's own
`planner.PlanFromMarkdown` and `Plan.Validate` before execution.

On 2026-10-19 a third repair added the remaining functional closure without
replacing completed history: **RUST-007E** closes the original K=3 parity,
production Markdown/plain-text ingestion, and live Ollama/PostgreSQL
ingest → retrieve → cited-answer gaps while Go remains present; **RUST-008A**
adds the real Axum/Next.js browser integration on top of RUST-007E. RUST-007C
therefore depends on RUST-007B, RUST-007E, and RUST-008A, and RUST-008 remains
the final Rust-only browser acceptance after RUST-007C.

## Verification mapping and the informational-criteria limitation

SOP treats every `Acceptance Checks` command as a **required** check and every
free-text `Acceptance` line as an **informational** criterion whenever checks
exist. Informational criteria are recorded but do not affect the `VERIFIED`
verdict. A generic `cargo test` run therefore cannot verify a missing production
path, database behavior, API contract, or UI state.

The rules for this plan are:

- Every required criterion has a dedicated deterministic check, or a dedicated
  acceptance-crate check whose tests fail when the behavior is absent. The
  acceptance crates named below are part of the deliverable, not optional
  scaffolding.
- A whole-suite `cargo test` check is supplementary. It is never the sole
  evidence for production wiring, a live database path, an API contract, or a
  UI criterion.
- Go reference checks are used only while Go exists (RUST-001 through RUST-007B).
  RUST-007C uses root Rust checks only, after Go removal. Do not remove Go before
  RUST-007 and RUST-007B pass.
- Node.js/browser checks are outside SOP's command policy and remain
  **operator-run evidence**, explicitly separate from SOP verification. The UI
  stays visibly mock-only until the real Rust API and pipeline are wired and
  pass end-to-end verification.


## Known unmet requirements

The following are explicit, unmet requirements. They are not removed, remapped, or
weakened by any passing check in this plan.

- **Markdown and plain-text ingestion.** The production ingestion service is
  currently PDF-only (`IngestPipeline::ingest_pdf_path`). The RUST-006C e2e
  corpus is a text-layer PDF, so it does not satisfy Markdown/text ingestion.
  A later task must add and verify those loaders through the same guard,
  transactional replacement, and provenance path.
- **Live-model end-to-end verification.** RUST-006C uses a fixture embedder and
  a mocked generator. A real Ollama ingest -> retrieval -> cited-answer run is
  not claimed. Live observations must be recorded separately (for example via
  `RAG_E2E_LIVE_OBSERVATIONS`) and must not be inferred from scripted tests.
- **CLI/browser end-to-end.** RUST-006C does not drive the real browser UI. The
  UI remains explicitly mock-only until the real Axum/Next.js flow passes; the
  CLI query-binary path is covered separately by RUST-006B acceptance tests.
- **Go parity.** The frozen Go fixtures and provenance remain the reference.
  RUST-006C adds a local deterministic regression baseline; it does not replace
  or extend Go parity evidence.
- **K=3 parity.** RUST-007's fixed parity run compares `FINAL_K=4` because the Go
  reference emits `K=1,2,TOP_K`; the original `FINAL_K=3` behavior remains
  explicitly open and is not waived by approval of the RUST-007 gate.

These are closure requirements for final project completion. RUST-007B/C must
not claim final completion while they remain unmet, and RUST-008 must replace
the mock-only UI with the real combined CLI/browser flow before final completion.
RUST-007D keeps the live-model groundedness/citation criterion as manual/unmet;
scripted parity does not satisfy it.

## RUST-001 — Establish the Go baseline

Establish and freeze the Go reference baseline: read repository instructions and inspect implemented behavior; record the reference commit, toolchain, commands, configuration, and dependencies; run the existing Go unit and required integration checks; map every implemented capability to a Rust task and verification method; freeze representative English, Vietnamese, and page-provenance fixtures; and record evaluation settings and baseline metrics when services are available.

### Dependencies

None

### Acceptance Criteria

- The reference SHA and environment are recorded in migration evidence.
- Baseline checks pass, or pre-existing failures are isolated and documented.
- Every implemented capability has an assigned task and verification method.
- Fixtures capture outputs from the pinned Go implementation.
- Exact-match fields, float tolerances, tie ordering, and evaluation thresholds are set before Rust comparisons. Deterministic retrieval metrics must not regress on the fixed corpus.

### Acceptance Checks

- `go build ./...`
- `go test ./...`

### Work Steps

1. Read repository instructions and inspect implemented behavior.
2. Record the reference commit, toolchain, commands, configuration, and dependencies.
3. Run the existing Go unit and required integration checks.
4. Map every implemented capability to a Rust task and verification method.
5. Freeze representative English, Vietnamese, and page-provenance fixtures.
6. Record evaluation settings and baseline metrics when services are available.

Verification: Run `go test ./...` and repository-prescribed checks. Record exact integration and evaluation commands discovered from the repository. Mark unavailable checks BLOCKED; do not count them as passed.

Learning focus: Map Go packages, interfaces, and errors to Rust modules, traits, and `Result`.

## RUST-002 — Create the Rust foundation

Create a Cargo package with library modules and thin binaries; define section, chunk, and retrieved-document types; port configuration parsing, defaults, and validation; select compatible maintained dependencies from official documentation; and add Rust checks alongside the existing Go checks. Includes the Rust toolchain and Zed development-environment setup below.

### Dependencies

- RUST-001

### Acceptance Criteria

- Rust builds without removing the Go commands.
- Configuration names, defaults, and invalid-value behavior match the baseline.
- Dependency choices and toolchain requirements are documented and locked.
- Domain types preserve source, section, language, chunk index, and optional page metadata.
- The selected toolchain and components work in a normal shell and Zed's integrated terminal.
- Completion, go-to-definition, hover information, and diagnostics work in Rust files.
- Saving a Rust file formats it. A temporary lint issue produces a Clippy diagnostic; remove the issue after verification.
- A unit test runs from Zed, and a debugging smoke check succeeds.
- Setup instructions support the user's macOS and Linux environments. Record each platform as verified or unverified; do not infer success on an untested machine.
- Existing Zed and agent settings remain intact. Missing access to the user's desktop is recorded as pending manual verification, not a successful check.

### Acceptance Checks

- `cargo build --manifest-path rust/Cargo.toml`

### Work Steps

1. Create a Cargo package with library modules and thin binaries.
2. Define section, chunk, and retrieved-document types.
3. Port configuration parsing, defaults, and validation.
4. Select compatible maintained dependencies from official documentation.
5. Add Rust checks alongside the existing Go checks.

Verification: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`. Add `--all-features` only if all optional features can run together.

Learning focus: Structs, enums, modules, traits, `Option`, and `Result`.

**Rust toolchain and Zed setup (part of RUST-002).** Use Zed as the Rust development environment. This requirement covers editing, diagnostics, running tests, and debugging this project. Building a Zed extension or changing Zed itself is outside scope.


Setup steps:

1. Check for an existing Rust installation with `rustup show`, `rustc --version`, and `cargo --version`. Preserve existing toolchains.
2. If Rust is absent, install it on the development machine with the official command:

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. Open a new shell after installation. Confirm that Cargo is available in Zed's integrated terminal.
4. Select a stable Rust release compatible with the chosen dependencies. Record its exact version in `rust-toolchain.toml` and use that version in CI. Do not leave the project on a moving `stable` pin.
5. Add the project toolchain components with `rustup component add rustfmt clippy rust-src rust-analyzer` from the project directory after creating the toolchain file.
6. Verify the platform linker and native build prerequisites with a small Cargo build. Install missing platform tools only when required.
7. Open the directory containing `Cargo.toml` in Zed. Confirm rust-analyzer starts. Zed has native Rust support and can use rust-analyzer from PATH or download its own version.
8. Merge project settings into `.zed/settings.json`. Preserve existing settings and agent configuration. Enable format-on-save for Rust. Use the language server formatter and enable Clippy diagnostics through rust-analyzer:

   ```json
   {
     "languages": {
       "Rust": {
         "format_on_save": "on"
       }
     },
     "lsp": {
       "rust-analyzer": {
         "initialization_options": {
           "check": {
             "command": "clippy"
           }
         }
       }
     }
   }
   ```

9. Use Zed's built-in Cargo tasks first. Add project tasks for existing migration checks only when needed. Keep their commands consistent with CI.
10. Run one unit test from Zed. Debug a test or binary with CodeLLDB. Confirm a breakpoint is reached and local variables can be inspected.

References: [Rust installation](https://rust-lang.org/learn/get-started/), [Zed Rust support](https://zed.dev/docs/languages/rust), and [Zed language configuration](https://zed.dev/docs/configuring-languages). Recheck settings against current documentation during implementation.

## RUST-003 — Port document processing

Port Markdown and plain-text loaders; port section parsing and chunk overlap behavior; preserve metadata through all transformations; compare shared fixtures against Go; and port PDF text-layer loading with a compatible existing library.

### Dependencies

- RUST-002

### Acceptance Criteria

- Sections and chunks match the baseline fixtures exactly for deterministic fields.
- Checks cover empty input, Unicode, Vietnamese text, boundary sizes, overlap, and invalid settings.
- Page provenance is preserved through PDF sections and chunks.
- Scanned PDFs retain the baseline explicit rejection behavior.
- Any PDF extraction differences are resolved against representative fixtures before closure.

### Acceptance Checks

- `cargo test --manifest-path rust/Cargo.toml`

### Work Steps

1. Port Markdown and plain-text loaders.
2. Port section parsing and chunk overlap behavior.
3. Preserve metadata through all transformations.
4. Compare shared fixtures against Go.
5. Port PDF text-layer loading with a compatible existing library.

Verification: Run Rust checks and the shared Go/Rust fixture comparisons without requiring Ollama or PostgreSQL for text processing.

Learning focus: Ownership, borrowing, UTF-8 boundaries, iterators, and file I/O.

## RUST-004 — Port Ollama and provider integration

Implement the shared HTTP client; port embedding and generation requests; port provider registration and resolution; and preserve request settings, timeouts, cancellation, and response validation.

### Dependencies

- RUST-003

### Acceptance Criteria

- Mock-server checks confirm endpoint paths and JSON request/response contracts.
- Unknown providers fail during configuration loading as in Go.
- Malformed responses, HTTP failures, dimension mismatches, and timeouts return useful errors.
- Live embedding and generation smoke checks pass with recorded model identifiers.

### Acceptance Checks

- `cargo test --manifest-path rust/Cargo.toml`

### Work Steps

1. Implement the shared HTTP client.
2. Port embedding and generation requests.
3. Port provider registration and resolution.
4. Preserve request settings, timeouts, cancellation, and response validation.

Verification: Run deterministic mock-server tests and Rust checks. Run a separate live Ollama smoke check. Do not require exact generated wording.

Learning focus: Async execution, serialization, trait objects or generics, and error propagation.

## RUST-005A — Storage foundation (schema, migrations, dimension guard)

Reuse the existing SQL migrations and schema; define the storage types and provenance columns; and port the embedding-dimension guard so a mismatch fails fast before any write. This bounded stage is DB-free and carries no database client yet.

### Dependencies

- RUST-004

### Acceptance Criteria

- Dimension validation occurs before writes.
- Provenance and language/page metadata match the baseline.

### Acceptance Checks

- `cargo test --manifest-path rust/Cargo.toml`

### Work Steps

1. Reuse the existing SQL migrations and schema (do not rewrite them).
2. Define the storage types (source, section, language, chunk index, page) and the provenance columns.
3. Port the embedding-dimension guard so a mismatch fails before any write.

Verification: Unit tests exercising the dimension guard and the storage types without a live database. Run Rust checks.

Learning focus: SQL types, error handling, and configuration guards.

## RUST-005C — Storage client and Go/Rust interoperability

Port the database client and connection pool over the schema from RUST-005A, and verify Go/Rust data interoperability at the storage layer against a dedicated disposable PostgreSQL/pgvector database.

### Dependencies

- RUST-005A

### Acceptance Criteria

- Rust reads Go-ingested records and Go reads Rust-ingested records.

### Acceptance Checks

- `cargo test --manifest-path rust/Cargo.toml`

### Execution

- verify-first

### Work Steps

1. Port the database client and connection pool.
2. Verify Go/Rust data interoperability (Rust reads Go-ingested rows; Go reads Rust-ingested rows).

Verification: Integration tests against a dedicated disposable PostgreSQL/pgvector database. Inspect records. Run Rust checks.

Learning focus: Database pools and resource lifetimes.

## RUST-005D — Verify storage client and Go/Rust interoperability

Re-verify the completed RUST-005C storage client and Go/Rust interoperability work without reimplementing it. Preserve RUST-005C's historical LOCAL_DONE/PARTIAL outcome and rerun the storage evidence against the dedicated PostgreSQL/pgvector database.

### Dependencies

- RUST-005A
- RUST-005C

### Acceptance Criteria

- Rust reads Go-ingested records and Go reads Rust-ingested records.

### Acceptance Checks

- `cargo test --manifest-path rust/Cargo.toml`

### Execution

- verify-first

### Work Steps

1. Inspect the existing storage client, migrations, and interoperability tests. Do not reimplement or rewrite them.
2. Run the Rust suite with the repository-root `.env` supplied by SOP, so the storage tests use the dedicated migration database.
3. Confirm the three interoperability cases execute and pass; record the persisted verdict and evidence. A skipped database test is never verification.

Verification: Run the Rust checks against the dedicated disposable PostgreSQL/pgvector database. Report the actual interoperability test evidence. A passing build, an unused fixture, or a skipped database test is not storage verification.

Learning focus: Database pools, resource lifetimes, and cross-language data contracts.

## RUST-005B — Port ingestion pipeline (fingerprints, transactional replacement)

Port fingerprints and unchanged-source detection, and transactional source replacement with rollback on failure, including the pre-effect ingest guard and its caller-level integration check.

### Dependencies

- RUST-005C
- RUST-005D
- RUST-003

### Acceptance Criteria

- Repeated ingestion matches the baseline unchanged-source behavior.
- A forced failure during replacement rolls back the complete transaction.
- Rust ingestion rejects incomplete PDF extraction, fully scanned PDFs, and fatal loader errors before embedding or database writes. For incomplete PDFs it reports the affected pages. Existing stored data is preserved on failure.
- A caller-level test shows a mixed text/scanned PDF causes no embedding calls and no database writes.

### Acceptance Checks

- `cargo test --manifest-path rust/Cargo.toml`

### Work Steps

1. Port fingerprints and unchanged-source detection.
2. Port transactional source replacement.
3. Reject incomplete/scanned/fatal loader outcomes before any embedding or database write (the pre-effect ingest guard), reporting the affected pages for a partial PDF.
4. Add the caller-level integration test (mixed text/scanned PDF -> no embedding calls, no database writes).
5. Verify rollback on forced failure.

Verification: Run integration tests against a dedicated disposable PostgreSQL/pgvector database. Inspect rollback outcomes. Run Rust checks. The integration check depends on RUST-003's loader status contract (`PdfExtraction.incomplete_pages`); completed loader parity is kept separate from this pending caller parity.

Learning focus: Transactions and resource lifetimes.

## RUST-005E — Production ingestion service seam and acceptance crate

Add a callable production ingestion service that composes `IngestPipeline` and `gate_pdf` over the existing ports, and add the `rust/acceptance/ingest_cli` crate that drives that production service. Do not wire the binary/config/real adapters in this stage; that is RUST-005F. This retains the historical RUST-005E ID; its prior zero-mutation attempts are preserved.

### Dependencies

- RUST-005B
- RUST-005D

### Acceptance Criteria

- A callable production service function, not a test-only helper, drives `IngestPipeline` and `gate_pdf` over the ports.
- The acceptance crate invokes that production service with mixed text/scanned and fully scanned PDFs and asserts zero embedding calls, zero database writes, and reported affected pages.
- Existing stored data is preserved on rejection or failure; the service never duplicates the pipeline in a second implementation.

### Acceptance Checks

- `cargo test --locked --manifest-path rust/acceptance/ingest_cli/Cargo.toml`
- `cargo test --locked --manifest-path rust/Cargo.toml`

### Work Steps

1. Create `rust/acceptance/ingest_cli` with a `Cargo.toml` and a test that calls the production service, not a local `ingest` helper.
2. Add the production service seam (for example `rust/src/ingest_service.rs`, exported from `lib.rs`) that is generic over the existing `Embedder`, `FingerprintReader`, and `SourceReplacer` ports and calls `IngestPipeline`.
3. Keep `bin/ingest.rs`, configuration, and real adapters unchanged in this stage; do not create a duplicate pipeline.
4. Run the dedicated acceptance crate and the whole suite with the repository-root `.env` supplied by SOP.

Verification: The dedicated crate must fail if the production service is missing or bypasses the guard. A passing library suite alone is not evidence for this seam.

Learning focus: A testable production seam without duplicating the implementation.

## RUST-005F — Production ingest binary/config/adapter wiring

Wire the production `ingest` binary and configuration to the RUST-005E service, constructing the real embedding, fingerprint-read, and transactional source-replacement adapters. The binary must report rejections and affected pages, exit non-zero, and preserve stored data on failure.

### Dependencies

- RUST-005E

### Acceptance Criteria

- `bin/ingest.rs` constructs and calls the production service with real adapters from configuration; it does not contain a second pipeline.
- The acceptance crate drives the production binary/service with a mixed text/scanned PDF and asserts zero embedding calls and zero database writes.
- Incomplete PDFs report the affected pages; fully scanned and fatal loader errors preserve stored data and exit non-zero.

### Acceptance Checks

- `cargo test --locked --manifest-path rust/acceptance/ingest_cli/Cargo.toml`
- `cargo test --locked --manifest-path rust/Cargo.toml`

### Work Steps

1. Extend `rust/acceptance/ingest_cli` to drive the production binary/service entry point, not only the generic service seam.
2. Wire `bin/ingest.rs`, CLI/config source selection, and the real adapters into the RUST-005E service.
3. Preserve rejection reporting, exit status, and stored data on every failure.
4. Run the dedicated acceptance crate and the whole suite with the repository-root `.env` supplied by SOP.

Verification: The dedicated crate must prove the binary/service path. Do not treat the RUST-005E service-seam test as evidence for the binary wiring.

Learning focus: Adapter construction, configuration, and process exit semantics.

## RUST-005G — Verify production ingest CLI wiring

Re-verify the RUST-005F production binary wiring without reimplementing it. Preserve RUST-005F's historical LOCAL_DONE/PARTIAL verdict. The production binary test drives the real `ingest` binary against the dedicated PostgreSQL/pgvector database and asserts rejection, affected-page reporting, and data preservation.

### Dependencies

- RUST-005F

### Acceptance Criteria

- `bin/ingest.rs` constructs and calls the production service with real adapters from configuration; it does not contain a second pipeline.
- The whole-suite production binary test drives `ingest` with mixed text/scanned and fully scanned PDFs and asserts zero embedding calls and zero database writes.
- Incomplete PDFs report affected pages; fully scanned and fatal loader errors preserve stored data and exit non-zero.

### Acceptance Checks

- `cargo test --locked --manifest-path rust/acceptance/ingest_cli/Cargo.toml`
- `cargo test --locked --manifest-path rust/Cargo.toml`

### Acceptance Mapping

- criterion 1: check 2
- criterion 2: check 2
- criterion 3: check 2

### Execution

- verify-first

### Work Steps

1. Inspect `rust/tests/ingest_binary.rs`; do not reimplement the binary or the service.
2. Run the dedicated acceptance crate and the whole suite with the repository-root `.env` supplied by SOP.
3. Confirm the binary test executed against the dedicated database and report the recorded verdict.

Verification: The binary test must fail if the production binary bypasses the guard or writes on rejection. A passing library suite alone is not evidence for the binary wiring.

Learning focus: Verifying a production entry point without reimplementing it.

## RUST-005H — Verify production ingest CLI wiring (mapped)

Re-verify the RUST-005F production binary wiring under the new SOP acceptance mapping without reimplementing it. Preserve RUST-005G's historical LOCAL_DONE/PARTIAL verdict. The production binary test drives the real `ingest` binary against the dedicated PostgreSQL/pgvector database and asserts rejection, affected-page reporting, and data preservation.

### Dependencies

- RUST-005G

### Acceptance Criteria

- `bin/ingest.rs` constructs and calls the production service with real adapters from configuration; it does not contain a second pipeline.
- The whole-suite production binary test drives `ingest` with mixed text/scanned and fully scanned PDFs and asserts zero embedding calls and zero database writes.
- Incomplete PDFs report affected pages; fully scanned and fatal loader errors preserve stored data and exit non-zero.

### Acceptance Checks

- `cargo test --locked --manifest-path rust/acceptance/ingest_cli/Cargo.toml`
- `cargo test --locked --manifest-path rust/Cargo.toml`

### Acceptance Mapping

- criterion 1: check 2
- criterion 2: check 2
- criterion 3: check 2

### Execution

- verify-first

### Work Steps

1. Inspect the existing binary, service, and `rust/tests/ingest_binary.rs`; do not reimplement them.
2. Run the dedicated acceptance crate and the whole suite with the repository-root `.env` supplied by SOP.
3. Confirm the recorded outcome is VERIFIED under the mapping and preserve RUST-005G's historical verdict.

### Verification

Run the mapped acceptance checks. A passing library suite without the mapped binary test is not evidence.

### Learning Focus

Mapping criteria to production-path evidence without reimplementation.

## RUST-006 — Port retrieval and answering

Port vector search and PostgreSQL full-text search; port rank fusion, similarity filtering, deduplication, and section expansion; port reranking and fallback behavior; port context budgeting and query rewriting; port answerability checks, prompts, and citation validation/repair; and wire the Rust ingest and question-answer commands.

### Dependencies

- RUST-005B

### Acceptance Criteria

- Fixed vectors and mocked model responses produce equivalent ranking, context, and fallback behavior within the frozen comparison rules.
- Both answering and evaluation use one Rust retrieval implementation.
- Prompt construction preserves separation of instructions and untrusted retrieved text.
- Insufficient evidence produces the baseline refusal behavior.
- Citation parsing, validation, repair, and page rendering pass shared cases.
- The live ingestion → retrieval → answer path works on the test corpus.

### Acceptance Checks

- `cargo test --manifest-path rust/Cargo.toml`

### Work Steps

1. Port vector search and PostgreSQL full-text search.
2. Port rank fusion, similarity filtering, deduplication, and section expansion.
3. Port reranking and fallback behavior.
4. Port context budgeting and query rewriting.
5. Port answerability checks, prompts, and citation validation/repair.
6. Wire the Rust ingest and question-answer commands.

Verification: Run parity checks, database integration tests, Rust checks, and a live end-to-end smoke check. Report live model variability separately from deterministic differences.

Learning focus: Composition, module boundaries, collections, and async orchestration.

## RUST-006B — Retrieval and cited-answer CLI/service wiring

Wire the retrieval and answering modules into the production question-answer CLI/service so the real entry point uses the single Rust retrieval implementation and returns cited answers with page references. This is a follow-up to the historical RUST-006 library work; its historical outcome is preserved.

### Dependencies

- RUST-006
- RUST-005H

### Acceptance Criteria

- One retrieval implementation serves both answering and evaluation.
- The production question-answer entry point returns cited answers with page references.
- Prompt construction preserves separation of instructions and untrusted retrieved text.
- Insufficient evidence produces the baseline refusal behavior.
- Citation parsing, validation, repair, and page rendering pass the shared cases.

### Acceptance Checks

- `cargo test --locked --manifest-path rust/acceptance/retrieval_cli/Cargo.toml`
- `cargo test --locked --manifest-path rust/Cargo.toml`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1
- criterion 4: check 1
- criterion 5: check 1

### Execution

- verify-first

### Work Steps

1. Add `rust/acceptance/retrieval_cli` with tests that invoke the production question-answer entry point and fail if it uses a second retrieval path.
2. Wire retrieval, context budgeting, answerability, prompt construction, citation validation/repair, and page rendering into the production service.
3. Run the dedicated acceptance crate and the whole suite with the repository-root `.env` supplied by SOP.

Verification: The dedicated crate must exercise the production entry point. A passing library suite with unwired production code is not verification.

Learning focus: Service composition and one-source-of-truth retrieval.

## RUST-006C — Real end-to-end verification

Exercise the real production path end to end: ingest a real corpus document through the production ingestion entry point, retrieve with the production retrieval implementation, and produce a cited answer through the production question-answer entry point against the dedicated PostgreSQL/pgvector database. Report deterministic fixture comparisons separately from live-model variability.

### Dependencies

- RUST-005H
- RUST-006B

### Acceptance Criteria

- A real ingest -> retrieval -> cited answer runs through the production entry points against the dedicated test database.
- Deterministic fixture and mocked-model comparisons match the frozen baseline; live model variability is reported separately.
- Invalid, incomplete, oversized, and empty inputs fail with the baseline behavior and preserve stored data.

### Acceptance Checks

- `cargo test --locked --manifest-path rust/acceptance/e2e/Cargo.toml`
- `cargo test --locked --manifest-path rust/Cargo.toml`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1

### Work Steps

1. Add `rust/acceptance/e2e` driving the production binaries/services, not library fakes.
2. Use the frozen corpus and the dedicated test database; clean up only test-owned rows.
3. Compare deterministic outputs with the frozen baseline and record live-model variability separately.
4. Run the dedicated crate and the whole suite with the repository-root `.env` supplied by SOP.

Verification: The end-to-end crate must fail if the production paths are stubs, if the retrieval implementation is duplicated, or if a rejected document writes data.

This is **scripted service-level PDF evidence**: a fixture embedder and mocked generator over the real PostgreSQL/pgvector database and production ingestion/retrieval/answer services. It is not live-model evidence, not Markdown/text ingestion evidence, not CLI/browser evidence, and not new Go-parity evidence. Those remain explicit unmet requirements below.

### Execution

- verify-first

Learning focus: Integration boundaries and evidence provenance.

## RUST-007 — Production evaluation command and deterministic parity (retains historical ID)

Port the Go `cmd/eval` production retrieval-evaluation command. This stage is deliberately limited to `eval`; the bilingual experiment harness is the single follow-up RUST-007D. Do not remove Go in this task.

### Dependencies

- RUST-006C

### Acceptance Criteria

- The production `eval` binary (`rust/src/bin/eval.rs` and `rust/src/evaluation.rs`) reads `evals/retrieval.json`, queries the ingested PostgreSQL corpus through the shared retrieval modules, and prints the baseline per-case rankings and aggregate recall/precision metrics.
- For the fixed deterministic configuration (`EVAL_DATASET=evals/retrieval.json`, `QUERY_REWRITE=false`, `EVAL_LEXICAL_RERANK=false`, `EVAL_LLM_RERANK=false`, `EVAL_ANSWERABILITY_GATE=false`, `EVAL_FACT_JUDGE=false`, `TOP_K=4`, `FINAL_K=4`, `EXPAND_LIMIT=20`), Rust and the pinned Go reference agree on per-case source/section ordering and aggregate metrics under the frozen comparison rules.
- The `eval` invocation mapping is documented, and a missing dataset or database fails clearly.
- No required check for this stage remains blocked or unverified. The bilingual command and the live-model groundedness/citation criterion are explicitly deferred to RUST-007D and closure.

### Acceptance Checks

- `go build ./...`
- `go test ./...`
- `go vet ./...`
- `cargo test --locked --manifest-path rust/acceptance/eval_parity/Cargo.toml`
- `cargo test --locked --manifest-path rust/Cargo.toml`

### Acceptance Mapping

- criterion 1: check 4
- criterion 2: check 4
- criterion 3: check 4
- criterion 4: check 5

### Execution

- verify-first

### Work Steps

1. Add the production `eval` binary and evaluation module: `rust/src/bin/eval.rs`, `rust/src/evaluation.rs`, and `[[bin]] eval` in `rust/Cargo.toml`.
2. Read `evals/retrieval.json` (with the `EVAL_DATASET` override); load the ingested corpus from PostgreSQL; use the shared `retrieval` modules and the existing storage/embedding configuration. No fixture-only shortcut.
3. Reproduce the Go baseline's per-case top-K source/section ordering and aggregate vector/hybrid recall and precision, plus evidence recall, for the fixed configuration. The Go baseline emits K=1,2,TOP_K, so the parity run uses TOP_K=4 and FINAL_K=4 as the common emitted K. Keep Go present and run the Go reference during the acceptance check.
4. Add `rust/acceptance/eval_parity` **with the implementation**: its tests must build and run the real Rust `eval` binary and the pinned Go `cmd/eval` against the same corpus/database, normalize the per-case ranking and summary metrics, and compare them. A stub or fixture-only Rust command must fail the crate. Generate its `Cargo.lock`; do not add a passing crate before the production command exists.
5. Document the `eval` invocation mapping under `docs/reports/rust-migration/`.

Verification: Run the Go reference checks, the dedicated eval-parity crate, and the Rust suite. The parity crate must fail if the Rust `eval` command is a stub, misses the dataset/database, or differs in ranking/metrics. Do not remove Go.

Learning focus: Evaluation metrics, deterministic comparison, and honest parity evidence.

## RUST-007D — Bilingual experiment harness and command parity

Port the Go `cmd/expbilingual` experiment harness. This is the single follow-up for the bilingual command split out of RUST-007; it preserves the original experiment-arm, cache, generation, and command-mapping requirements.

### Dependencies

- RUST-007

### Acceptance Criteria

- All four commands have working Rust equivalents and documented invocation mappings (ingest and query are covered by their tasks, `eval` by RUST-007, and `expbilingual` here).
- The bilingual command supports arms A/B/C/D1/D2/E over `evals/retrieval-vi-en-expanded.json`, `ARM`, `REPEATS`, `DIAG`, `QUERY_CACHE`, `GEN_TIMEOUT`, and the configured models.
- Query generation is pinned (temperature 0, seed 0, num_predict 64) and cached by case id + configuration; cache behavior and per-case/aggregate output match the frozen contract.
- Generated answers meet the baseline groundedness and citation criteria — a **live-model closure requirement**, not satisfied by scripted tests.
- No required check for this stage remains blocked or unverified, except the explicit live-model closure evidence.

### Acceptance Checks

- `go build ./...`
- `go test ./...`
- `go vet ./...`
- `cargo test --locked --manifest-path rust/acceptance/bilingual_parity/Cargo.toml`
- `cargo test --locked --manifest-path rust/Cargo.toml`

### Acceptance Mapping

- criterion 1: check 4
- criterion 2: check 4
- criterion 3: check 4
- criterion 4: manual: live-model groundedness and citation evidence (UNMET until a real Ollama run is recorded)
- criterion 5: check 5

### Execution

- verify-first

### Work Steps

1. Add the production `expbilingual` binary and bilingual module; preserve arms A/B/C/D1/D2/E, dataset formats, the cache path/semantics, and the pinned generation settings.
2. Add `rust/acceptance/bilingual_parity` **with the implementation**; its tests must run the real Rust binary with a scripted generator/cache and compare the frozen contract. Do not add a passing crate before the production command exists.
3. Record the live-model groundedness/citation evidence separately; scripted parity does not satisfy criterion 4.
4. Document the bilingual invocation mapping.

Verification: Run the Go reference checks, the bilingual parity crate, and the Rust suite. Criterion 4 remains explicitly unmet until live evidence is recorded.

Learning focus: Experiment arms, cache semantics, and live-versus-scripted evidence.

## RUST-007B — Freeze transition evidence and update pre-removal tooling

Freeze the capability matrix, limitations, rollback reference, baseline provenance, and fixture provenance; update README, command reference, setup, Makefile, hooks, and CI for the transition while Go remains present; and verify a clean checkout with Go present. Do not remove Go.

### Dependencies

- RUST-007
- RUST-007D

### Acceptance Criteria

- Migration evidence records the capability matrix, remaining limitations, rollback reference, pinned Go commit SHA, source repository URL, and fixture provenance.
- Frozen fixtures and shared assets (`migrations/*.sql`, `evals/`, `examples/`, `docker-compose.yml`) are preserved.
- CI, Makefile, hooks, and documentation describe the transition layout while Go remains.
- A clean checkout follows the documented setup with Go present.

### Acceptance Checks

- `go build ./...`
- `go test ./...`
- `go vet ./...`
- `cargo test --locked --manifest-path rust/Cargo.toml`
- `cargo test --locked --manifest-path rust/acceptance/evidence/Cargo.toml`
- `cargo test --locked --manifest-path rust/acceptance/clean_checkout_go/Cargo.toml`

### Acceptance Mapping

- criterion 1: check 5
- criterion 2: check 5
- criterion 3: check 5
- criterion 4: check 6

### Work Steps

1. Add `rust/acceptance/evidence` asserting the required evidence files, provenance fields, frozen fixtures, and shared assets exist and are internally consistent.
2. Write the capability matrix, limitations, rollback reference, and baseline provenance.
3. Update README, command reference, setup, Makefile, hooks, and CI for the pre-removal layout.
4. Verify a disposable clean checkout with Go present.

Verification: Run Go reference checks, the dedicated evidence crate, and the Rust suite. The evidence crate must fail if provenance or shared assets are missing.

Learning focus: Reproducibility and transition documentation.

## RUST-007E — Functional closure: K=3 parity, Markdown/text ingestion, live RAG

Close the remaining functional gaps before Go is removed: reproduce the original
K=3 evaluation contract against the pinned Go reference, ingest Markdown and
plain-text sources through the production pipeline, and verify the real Ollama +
PostgreSQL production path from ingestion to a validated cited answer. Keep Go
present and do not start the Rust-only transition.

### Dependencies

- RUST-007
- RUST-007D
- RUST-007B

### Acceptance Criteria

- With the pinned Go reference and the original evaluation settings (`TOP_K=3`, `FINAL_K=3`, `EXPAND_LIMIT=20`, `MIN_SIMILARITY=0.6`, rewrite/rerank/answerability-gate/fact-judge off), the production Rust `eval` binary and the pinned Go `cmd/eval` agree per case on source/section ranking order and K=3 recall/precision and agree on the aggregate metrics under the frozen comparison rules. The evidence is produced at K=3; K=4 results are not substituted.
- The production `ingest` binary and service load `.md` and `.txt` sources through the existing Markdown/text loaders and store them through the same chunk, embed, and transactional source-replacement path as PDFs, preserving source identity, language, chunk ordering, corpus, and content hash.
- Empty, malformed or unreadable, duplicate (unchanged), and failed-ingestion cases are rejected or skipped without corrupting or partially replacing previously stored rows; page-less Markdown/text rows keep `page = NULL`, and stored chunk order/index and language match the loaded sections.
- A real Ollama embedding/generation model and a real PostgreSQL+pgvector database drive the Rust production `ingest` and `query` commands to ingest representative PDF, Markdown, and plain-text documents, retrieve content, generate an answer with source citations that validate against the retrieved content, and refuse when the retrieved context cannot answer. The commands, configuration identifiers, model versions, corpus identity, results, and observations are recorded in `docs/reports/rust-migration/RUST-007E-live-evidence.md`.
- Existing Go reference checks and the RUST-007B transition/clean-checkout checks still pass. Go remains available and operational, and no completed task, acceptance record, fixture, or historical result is altered.

### Acceptance Checks

- `go build ./...`
- `go test ./...`
- `go vet ./...`
- `cargo test --locked --manifest-path rust/Cargo.toml`
- `cargo test --locked --manifest-path rust/acceptance/k3_parity/Cargo.toml`
- `cargo test --locked --manifest-path rust/acceptance/ingest_md_text/Cargo.toml`
- `cargo test --locked --manifest-path rust/acceptance/live_rag/Cargo.toml`
- `cargo test --locked --manifest-path rust/acceptance/evidence/Cargo.toml`
- `cargo test --locked --manifest-path rust/acceptance/clean_checkout_go/Cargo.toml`

### Acceptance Mapping

- criterion 1: check 5
- criterion 2: check 6
- criterion 3: check 6
- criterion 4: check 7
- criterion 5: check 9

### Execution

- verify-first

### Work Steps

1. Add `rust/acceptance/k3_parity` with the implementation: build the pinned Go `cmd/ingest`/`cmd/eval` and the production Rust `eval`, ingest the frozen dataset sources into a dedicated test database with deterministic embeddings, run both commands with `TOP_K=3` and `FINAL_K=3`, and compare per-case source/section ranking order, K=3 recall/precision, rejection labels, and aggregate metrics. Do not reuse K=4 output or regenerate expected data from Rust.
2. Extend the production ingest path for `.md` and `.txt`: select the existing Markdown/text loader by extension, pass page-less sections through the existing pre-effect gate, and keep the single pipeline, fingerprint, embedding, and transactional replacement path. Do not duplicate chunking or storage logic.
3. Add `rust/acceptance/ingest_md_text` proving production Markdown/plain-text ingestion, provenance (`page = NULL`, source/language/chunk order), duplicate/unchanged behavior, empty/malformed rejection, and failed-embedding/failed-write rollback leaves prior rows intact. Use a dedicated test database and a unique corpus; clean up only that corpus.
4. Add `rust/acceptance/live_rag`: run the production Rust `ingest` and `query` binaries against real Ollama and PostgreSQL+pgvector with a unique corpus; ingest a PDF, Markdown, and plain-text source; ask an answerable question and validate every rendered citation against retrieved content; ask an unanswerable question and assert the baseline refusal; record the model names/versions and results in `docs/reports/rust-migration/RUST-007E-live-evidence.md`; clean up only the run corpus.
5. Record the K=3, ingestion, and live evidence under `docs/reports/rust-migration/`, update the migration capability matrix and limitations, and run the existing Go and RUST-007B checks.

Verification: Run the Go reference checks, the dedicated K=3, Markdown/text, live-RAG, evidence, and clean-checkout crates, and the Rust suite. A generic `cargo test` is not evidence for a missing production path; the live crate must exercise the real binaries and services.

Learning focus: Frozen-contract parity, format-aware production loading, and live end-to-end verification.

## RUST-008A — Browser integration

Add the real Axum API and Next.js upload → ingest → question → cited-answer flow over the existing Rust application services, with browser end-to-end and error-state evidence. This task does not remove Go and does not perform the Rust-only transition.

### Dependencies

- RUST-007E

### Acceptance Criteria

- The Axum API exposes upload, ingestion status, query, and health over the same Rust application services the CLI uses, binds `127.0.0.1` by default, keeps `API_URL` and model/database credentials server-side, and serves only same-origin `/api` requests to the browser.
- The Next.js workflow completes upload → ingest → question → cited-answer end to end, with source citations and page references.
- Loading, empty, connection-failure, and extraction-error states are covered.
- The Rust-side API contract is verified deterministically; browser end-to-end and UI-state checks are recorded as operator-run evidence because Node.js/browser execution is outside the SOP command policy.
- Existing CLI behavior and the Go reference remain operational.

### Acceptance Checks

- `cargo test --locked --manifest-path rust/Cargo.toml`
- `cargo test --locked --manifest-path rust/acceptance/api_contract/Cargo.toml`

### Acceptance Mapping

- criterion 1: check 2
- criterion 2: manual: operator-run browser upload-to-cited-answer evidence
- criterion 3: manual: operator-run UI loading, empty, connection-failure, and extraction-error evidence
- criterion 4: check 2
- criterion 5: check 1

### Execution

- verify-first

### Work Steps

1. Expose the shared Rust application services so the CLI and the Axum API call the same load/ingest/retrieve/answer functions.
2. Add the Axum API with upload, ingestion, query, and health endpoints bound to loopback; keep `API_URL` and credentials server-side.
3. Add the Next.js upload, status, question, and cited-answer views, proxying `/api/*` server-side and binding the Next.js server to loopback.
4. Add `rust/acceptance/api_contract` for the Rust-side contract and record operator-run browser evidence for the full flow and the four UI states.

Verification: Run the Rust checks and the API contract crate; exercise the browser flow and record the four UI states as operator-run evidence. Do not claim the Rust-only layout in this task.

Learning focus: Axum handlers, shared application services, and browser-facing API isolation.

## RUST-007C — Rust-only transition and final verification

Move the Rust package to the repository root; remove Go source, `go.mod`, `go.sum`, and Go-only tooling; update CI, Makefile, `.zed` tasks, SOP validation checks, hooks, and documentation for the Rust-only layout; and verify a clean checkout builds and tests without Go installed. Remove Go only after RUST-007 and RUST-007B pass.

### Dependencies

- RUST-007B
- RUST-007E
- RUST-008A

### Acceptance Criteria

- The final repository is Rust-only: the Rust package sits at the repository root; no Go source, `go.mod`, `go.sum`, or Go-dependent tooling (including the Go fixture generator) remains.
- Frozen fixtures and the baseline provenance (pinned Go commit SHA and source repository URL) are preserved, and the Rust suite runs with no Go toolchain installed.
- Shared assets are preserved: `migrations/*.sql`, `evals/`, `examples/`, and `docker-compose.yml`.
- CI, `Makefile`, `.zed/` tasks, SOP validation checks, and documentation describe the Rust-only layout.
- A clean checkout builds and tests with Go absent.
- Formatting, linting, unit, integration, and end-to-end checks pass.

### Acceptance Checks

- `cargo build --locked`
- `cargo test --locked`
- `cargo clippy --locked --all-targets`
- `cargo fmt --check`
- `cargo test --locked --manifest-path acceptance/final_layout/Cargo.toml`
- `cargo test --locked --manifest-path acceptance/clean_checkout/Cargo.toml`

### Acceptance Mapping

- criterion 1: check 5
- criterion 2: check 5
- criterion 3: check 5
- criterion 4: check 5
- criterion 5: check 6
- criterion 6: check 2

### Execution

- verify-first

### Work Steps

1. Add `acceptance/final_layout` asserting the root layout, absence of Go sources/tooling, preserved fixtures/provenance/shared assets, and Rust-only docs/CI/SOP checks.
2. Move the Rust package from `rust/` to the repository root.
3. Remove Go source, `go.mod`, `go.sum`, and Go-dependent tooling; preserve frozen fixtures, datasets, examples, migrations, and shared assets.
4. Replace the project validation commands with the Rust-only set (build, test, clippy, fmt) and update CI, Makefile, `.zed` tasks, hooks, and documentation.
5. Verify a clean checkout in a disposable directory with Go absent.

Verification: Run the root Rust checks and the final-layout crate. Do not claim the transition from a passing suite alone; the final-layout crate must prove the absence of Go and the presence of provenance/shared assets.

Learning focus: Packaging, toolchain removal, and clean-checkout reproducibility.

## RUST-008 — Next.js UI and Axum API

Add a Next.js front end and an Axum API over the existing Rust RAG services: document upload, ingestion status, question entry, answers with source citations and page references, and health. The CLI and the API call the same Rust application services; ingestion, retrieval, and generation stay in Rust. Both the Next.js server and the Axum API bind to loopback (`127.0.0.1`) by default; `API_URL` remains a server-side setting and the browser only makes same-origin `/api` requests. Keep the database and model credentials server-side. Reuse the layout, interaction patterns, and API contracts of the `inventory` reference (Next.js -> Axum -> PostgreSQL; the browser talks only to Next.js; `/api/*` is proxied server-side), and the baseline's `rag-retrieval/v1` query contract where practical. Record intentional differences. The Rust-side API contract (upload, ingest, query, health, error states, same-origin `/api`, server-side `API_URL`, loopback binding) is the SOP-authorizable evidence. Node.js/browser checks are outside the command policy and are recorded as **operator-run evidence**, not a SOP verdict. The UI must stay explicitly labeled mock-only until the real Rust API and pipeline pass the end-to-end verification.

### Dependencies

- RUST-007C

### Acceptance Criteria

- Startup commands and the browser URL are documented.
- Upload -> ingest -> question -> cited-answer verification completes end to end.
- The UI shows loading, empty, connection-failure, and extraction-error states.
- Incomplete PDFs are rejected before embedding or database writes.
- UI/API contracts are verified and existing CLI behavior is preserved.
- The final project requires no Go toolchain.
- Both the Next.js server and the Axum API bind loopback (`127.0.0.1`) by default; `API_URL` stays server-side and the browser makes only same-origin `/api` requests.

### Acceptance Checks

- `cargo test --locked --manifest-path acceptance/api_contract/Cargo.toml`
- `cargo test --locked --manifest-path Cargo.toml`

### Acceptance Mapping

- criterion 1: manual: operator-run startup and browser URL evidence
- criterion 2: manual: operator-run browser upload-to-cited-answer evidence
- criterion 3: manual: operator-run UI state evidence
- criterion 4: check 1
- criterion 5: check 1
- criterion 6: check 2
- criterion 7: check 1

### Work Steps

1. Expose the shared Rust application services so the CLI and the API call the same functions (load/upload, ingest, retrieve, answer).
2. Add the Axum API: upload, ingestion, query, and health endpoints, bound to `127.0.0.1` by default.
3. Add the Next.js UI: document upload, ingestion status, question entry, answers, source citations, and page references, proxying `/api/*` server-side; bind the Next.js server to loopback (`127.0.0.1`) as well.
4. Keep database and model credentials and `API_URL` server-side; the browser receives neither and makes only same-origin `/api` requests.
5. Verify the end-to-end flow and the loading, empty, connection-failure, and extraction-error states.
6. Record intentional differences from the reference.

Verification: Run Rust checks and the API contract tests; exercise the browser flow (upload -> ingest -> question -> cited answer) and the four UI states; confirm the CLI behavior is unchanged and no Go toolchain is required.

Learning focus: Axum handlers and shared state, server-side proxying, and UI state handling.

Intentional differences from the reference: both servers bind `127.0.0.1` (loopback-only, not `0.0.0.0`); `API_URL` is server-side only and the browser stays on same-origin `/api`; routes are RAG-specific (upload/ingest/query/health); no authentication surface in this scope. The Node.js UI cannot be an SOP acceptance check (Node.js is outside the command policy), so its verification is **operator-run evidence** recorded separately, while the Rust-side API contract is checked by `cargo test --manifest-path rust/Cargo.toml`.


## Verification matrix

Every required criterion added or changed by this repair maps to a dedicated
deterministic check. The whole-suite check is supplementary, never the sole
evidence for a production path.

| Task | Required criterion | Deterministic evidence |
| ---- | ------------------ | ---------------------- |
| RUST-005E | Production service seam drives `IngestPipeline`/`gate_pdf` | `rust/acceptance/ingest_cli` |
| RUST-005F | Production binary/config/adapters drive the service | `rust/acceptance/ingest_cli` + binary test |
| RUST-005G | Verify the binary wiring without reimplementation (legacy shape) | `rust/Cargo.toml` suite (binary test) |
| RUST-005H | Verify the binary wiring under the acceptance mapping | `rust/Cargo.toml` suite (binary test) |
| RUST-005E/F | Mixed/scanned/fatal rejections cause zero embeddings/writes; pages reported | `rust/acceptance/ingest_cli` |
| RUST-005E/F | Stored data preserved; tests use the production service/binary | `rust/acceptance/ingest_cli` |
| RUST-006B | One retrieval implementation for answering and evaluation | `rust/acceptance/retrieval_cli` |
| RUST-006B | Production Q&A returns cited pages; prompt separation; refusal | `rust/acceptance/retrieval_cli` |
| RUST-006B | Citation parse/validate/repair/render cases | `rust/acceptance/retrieval_cli` |
| RUST-006C | Real ingest -> retrieval -> cited answer through production entry points | `rust/acceptance/e2e` |
| RUST-006C | Frozen fixture/model parity; live variability reported separately | `rust/acceptance/e2e` |
| RUST-006C | Invalid/incomplete/oversized/empty inputs preserve data | `rust/acceptance/e2e` |
| RUST-007 | Production `eval` command reads `evals/retrieval.json` and queries PostgreSQL | `rust/acceptance/eval_parity` |
| RUST-007 | Per-case ranking and aggregate metrics match the pinned Go reference | `rust/acceptance/eval_parity` |
| RUST-007 | Go reference remains green while Go exists | `go build`, `go test`, `go vet` |
| RUST-007D | Four command mappings; bilingual arms A/B/C/D1/D2/E | `rust/acceptance/bilingual_parity` |
| RUST-007D | Pinned generation and cache semantics match the frozen contract | `rust/acceptance/bilingual_parity` |
| RUST-007D | Live-model groundedness/citations | manual live evidence (UNMET) |
| RUST-007B | Capability matrix, limitations, rollback, provenance | `rust/acceptance/evidence` |
| RUST-007B | Frozen fixtures and shared assets preserved | `rust/acceptance/evidence` |
| RUST-007B | Clean checkout with Go present | `rust/acceptance/clean_checkout_go` |
| RUST-007C | Root Rust-only layout; no Go sources/tooling | `acceptance/final_layout` |
| RUST-007C | Fixtures, provenance, shared assets preserved | `acceptance/final_layout` |
| RUST-007C | Rust-only docs/CI/Makefile/SOP checks | `acceptance/final_layout` |
| RUST-007C | Clean checkout builds/tests with Go absent | `acceptance/clean_checkout` |
| RUST-007C | Formatting, linting, unit, integration, end-to-end | root Cargo build/test/clippy/fmt |
| RUST-008 | Rust API contract (upload/ingest/query/health/errors) | `acceptance/api_contract` |
| RUST-008 | UI browser states and full browser flow | **operator-run evidence** (Node.js is outside SOP policy) |

## Stop conditions

Pause the affected task when:
- Required behavior cannot be established from code, tests, or product requirements.
- A deterministic parity difference remains unexplained.
- A dependency cannot preserve required behavior without a scope change.
- A destructive schema change or shared-data operation is required.
- Required services or credentials are unavailable and prevent acceptance checks.

Record the blocker, evidence, attempted resolution, and the smallest decision or action needed. Continue independent work when possible. Never mark an unverified task complete.

## Evidence and progress

Keep migration evidence under `docs/reports/rust-migration/`. Record the baseline SHA, toolchains, dependency decisions, fixtures, exact commands, results, and unresolved differences. Avoid secrets and machine-specific credentials.

| Task | Status | Evidence | Learning note |
| --- | --- | --- | --- |
| RUST-001 | PLANNED | Pending | Pending |
| RUST-002 | PLANNED | Pending | Pending |
| RUST-003 | PLANNED | Pending | Pending |
| RUST-004 | PLANNED | Pending | Pending |
| RUST-005A | PLANNED | Pending | Pending |
| RUST-005B | PLANNED | Pending | Pending |
| RUST-005E | PLANNED | Pending | Pending |
| RUST-005F | PLANNED | Pending | Pending |
| RUST-005G | PLANNED | Pending | Pending |
| RUST-005H | PLANNED | Pending | Pending |
| RUST-005D | PLANNED | Pending | Pending |
| RUST-006 | PLANNED | Pending | Pending |
| RUST-006B | PLANNED | Pending | Pending |
| RUST-006C | PLANNED | Pending | Pending |
| RUST-007 | PLANNED | Pending | Pending |
| RUST-007D | PLANNED | Pending | Pending |
| RUST-007B | PLANNED | Pending | Pending |
| RUST-007C | PLANNED | Pending | Pending |
| RUST-008 | PLANNED | Pending | Pending |

First learning milestone: complete RUST-001 through RUST-003, with working Rust parsing and chunking and demonstrated fixture parity. Continue to later tasks unless a stop condition applies.

## Completion report

Report completed tasks, reference and final commits, verification results, behavior differences, dependency tradeoffs, remaining limitations, and the main Rust concepts learned. State publication or merge status separately from local completion.