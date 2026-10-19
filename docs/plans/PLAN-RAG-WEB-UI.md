# RAG Web UI Requirements and Implementation Plan

Status: DRAFT — no UI implementation or running browser URL is claimed.
Date: 2026-10-17
Repository: https://github.com/imhttran/rag-template-rust
Target path: `docs/plans/PLAN-RAG-WEB-UI.md`
Parent task: RUST-008 in `docs/plans/PLAN-RUST-MIGRATION.md`

## Project

rag-template-rust — local RAG web UI (Next.js + Axum).

## Summary

Provide a local browser interface for document ingestion and grounded question
answering: a Next.js UI and an Axum API over the shared Rust services, with the
same-origin `/api` proxy and loopback-only binding. UI-001 and UI-002 proceed
while the migration is paused; UI-003..UI-005 require their listed services.
This document was converted to SOP's canonical plan schema (stages as
`- **Objective:**` / `- **Depends on:**` / `- **Acceptance:**` fields) without
changing any criterion.

## Goal

Provide a local browser interface for document ingestion and grounded question
answering. Use Next.js for the UI and Axum for the Rust API. Follow the existing
`inventory` project's visual conventions and same-origin proxy pattern.

## Scope

- Show API, database, and model availability.
- Upload supported documents and display ingestion results.
- Ask questions and display answers with source, section, and page citations.
- Handle loading, empty, insufficient-evidence, connection-failure, and
  extraction-error states.
- Reuse the Rust application services used by the CLI.
- Keep database and model credentials server-side.
- Bind both servers to `127.0.0.1` by default.

Exclude public hosting, remote access, authentication, user accounts, OCR,
PDF-to-Markdown experimentation, genealogy extraction, and conversation
persistence. Do not add a document-management subsystem or PDF viewer unless
separately requested. A page reference must not imply that an unavailable viewer
exists.

## Reference and architecture

Inspect `inventory` before implementation. Reuse its layout and components where
practical: `AppShell`, `PageHeader`, `PageTitle`, `PageFooter`, `FormStatus`, and
`SystemStatus`. Record the reference commit and intentional differences. Do not
copy inventory-specific domain behavior.

The browser sends same-origin `/api` requests to Next.js. Next.js proxies requests
to Axum through a server-side `API_URL`. Axum calls shared Rust ingestion and query
services. The CLI calls those same services. Keep retrieval and generation logic
out of Next.js and route handlers.

Use a `web/` directory for Next.js and the migration's final root Cargo package for
Rust. Before RUST-007 moves Rust to the root, document temporary paths rather than
moving files early.

## Main workspace

| Area | Required behavior |
| --- | --- |
| System status | Show API, database, and configured model availability. Distinguish unavailable from not yet checked. |
| Document upload | Select one supported file, show its name and size, and explicitly start ingestion. |
| Ingestion result | Show completion or a useful error. For incomplete PDFs, show affected page numbers. |
| Question entry | Accept a non-empty question. Prevent repeated submission while a request is pending. |
| Answer | Show the grounded answer or the existing insufficient-evidence response. Preserve readable formatting. |
| Sources | Show source, section, supporting text, and page when available. Missing page metadata is allowed and must not be invented. |

## Tasks

### UI-001 — Define screens and API contracts

- **Objective:** Inspect the reference UI and the current Rust contracts; define the workspace layout and state transitions; define request, response, and error schemas; record supported formats, upload limits, timeout behavior, and citation fields; and record intentional differences from the reference application.
- **Depends on:** none
- **Acceptance:** Contract schemas describe successful responses and all required error states.
- **Acceptance:** Supported file types match implemented loaders; OCR is not offered.
- **Acceptance:** Ingestion states distinguish uploading, processing, completion, and failure where observable.
- **Acceptance:** Citation fields match available retrieval metadata.
- **Acceptance:** A documented upload-size limit is selected before implementation.

Proposed endpoints, subject to contract review:

| Endpoint | Purpose |
| --- | --- |
| `GET /api/health` | Return service and dependency status without secrets. |
| `POST /api/documents/ingest` | Accept a multipart file and return a completed ingestion result or structured error. |
| `POST /api/query` | Accept a question and return an answer with structured citations. |

Start with request/response ingestion. Do not show fabricated percentage progress.
Add a job/status API only if measured ingestion duration requires it. Define proxy
and request timeouts together.

Work steps:

1. Inspect the reference UI and current Rust contracts.
2. Define the workspace layout and state transitions.
3. Define request, response, and error schemas.
4. Record supported formats, upload limits, timeout behavior, and citation fields.
5. Record intentional differences from the reference application.

Verification: Review schemas against Rust types and reference patterns. Record
unresolved contract questions before UI/API implementation.

### UI-002 — Build the Next.js prototype

- **Objective:** Create the Next.js workspace using the reference visual conventions; implement the upload, ingestion-result, question, answer, and citation areas; add explicit mock scenarios for the required states; add keyboard access, labels, focus handling, and accessible status messages; and support desktop and narrow layouts.
- **Depends on:** UI-001
- **Acceptance:** The prototype starts locally with a documented URL.
- **Acceptance:** Mock mode is visibly labeled and does not imply real ingestion or generated answers.
- **Acceptance:** Loading, empty, success, insufficient-evidence, connection-failure, invalid-file, and incomplete-PDF states can be demonstrated.
- **Acceptance:** Duplicate submissions are prevented while requests are pending.
- **Acceptance:** Forms work by keyboard, errors are associated with inputs, and status changes are announced.

Work steps:

1. Create the Next.js workspace using reference visual conventions.
2. Implement upload, ingestion result, question, answer, and citation areas.
3. Add explicit mock scenarios for required states.
4. Add keyboard access, labels, focus handling, and accessible status messages.
5. Support desktop and narrow layouts.

Verification: Run TypeScript checks and focused component tests. Inspect desktop and
narrow layouts. Walk through all mock scenarios. Choose actual package scripts during
setup and record exact commands.

Policy note: no authorizable acceptance check exists — Node.js/Next.js tooling is
outside the SOP command policy, so these UI checks are operator-run evidence (see
"Policy limitation") and are never counted as SOP-verified criteria.

### UI-003 — Implement the Axum API

- **Objective:** Implement the health, ingestion, and query routes; call the shared Rust application services; enforce file-size and format limits; handle temporary files without trusting client filenames as filesystem paths; return structured errors and bounded request timeouts; and bind the API to loopback by default. Requires the verified Rust storage, ingestion, and query services from RUST-005A, RUST-005B, and RUST-006 (cross-plan dependency; the migration stays paused until those pass).
- **Depends on:** UI-001
- **Acceptance:** Route tests confirm the approved schemas and status codes.
- **Acceptance:** Incomplete PDFs, scanned PDFs, and fatal loader errors are rejected before embedding or database writes.
- **Acceptance:** Incomplete extraction identifies affected pages when known.
- **Acceptance:** Rejected ingestion preserves existing stored records.
- **Acceptance:** Complete-document ingestion succeeds as a positive control.
- **Acceptance:** CLI and API use the same application services; CLI regression checks pass.
- **Acceptance:** Errors and health responses expose no credentials or internal stack traces.
- **Acceptance:** Uploaded files are bounded and cleaned up after success or failure.
- **Acceptance Checks:** `cargo test --manifest-path rust/Cargo.toml`

Work steps:

1. Implement health, ingestion, and query routes.
2. Call shared Rust application services.
3. Enforce file-size and format limits.
4. Handle temporary files without trusting client filenames as filesystem paths.
5. Return structured errors and bounded request timeouts.
6. Bind the API to loopback by default.

Verification: Run locked Rust checks, API contract tests, and dedicated
test-database integration checks. Include failure cases and positive controls. Do
not use shared or production databases.

### UI-004 — Connect Next.js to Axum

- **Objective:** Configure the Next.js same-origin `/api` proxy; keep `API_URL` server-side with no browser-facing backend override; bind Next.js to loopback by default; connect the upload and question forms to the real API; display API errors and source metadata; and make real versus mock mode explicit with no silent fallback to mock data.
- **Depends on:** UI-002, UI-003
- **Acceptance:** Browser requests use same-origin `/api` routes only.
- **Acceptance:** Database and model credentials are absent from client bundles and browser responses.
- **Acceptance:** Both servers listen on `127.0.0.1` by default.
- **Acceptance:** Real upload → ingest → question → cited-answer verification passes.
- **Acceptance:** Source text is rendered safely; uploaded or generated content cannot inject executable HTML.
- **Acceptance:** Connection failures preserve entered questions and allow an explicit retry.

Work steps:

1. Configure the Next.js same-origin `/api` proxy.
2. Keep `API_URL` server-side; do not introduce a browser-facing backend override.
3. Bind Next.js to loopback by default.
4. Connect upload and question forms to the real API.
5. Display API errors and source metadata.
6. Make real versus mock mode explicit; never silently fall back to mock data.

Verification: Inspect browser network requests and listening addresses. Run UI/API
integration tests. Exercise model, database, and API failure states. Inspect
generated client assets for configured secrets without printing secret values.

### UI-005 — Verify and document completion

- **Objective:** Run the complete browser workflow against test services; verify accessibility and responsive layouts; run the Next.js build, TypeScript checks, focused UI tests, and the required Rust checks; verify startup from a clean checkout without Go installed; document prerequisites, environment settings, startup and shutdown commands, and the actual browser URL; and update the parent migration plan and final evidence report. Depends on RUST-007 for the final layout and Go-free verification (cross-plan).
- **Depends on:** UI-004
- **Acceptance:** A supported document ingests and produces a grounded answer with usable citations.
- **Acceptance:** A mixed PDF reports incomplete pages and produces no embedding calls or database writes through the production path.
- **Acceptance:** Insufficient evidence and dependency failures display the approved states.
- **Acceptance:** Keyboard navigation, visible focus, and narrow-screen layout checks pass.
- **Acceptance:** Required checks pass; skipped or pending checks are stated explicitly.
- **Acceptance:** A clean checkout starts using Rust and Node.js tooling without Go.
- **Acceptance:** The documented browser URL opens the implemented application. Do not claim a running URL before startup is verified.

Work steps:

1. Run the complete browser workflow against test services.
2. Verify accessibility and responsive layouts.
3. Run Next.js build, TypeScript checks, focused UI tests, and required Rust checks.
4. Verify startup from a clean checkout without Go installed.
5. Document prerequisites, environment settings, startup commands, shutdown commands, and actual browser URL.
6. Update the parent migration plan and final evidence report.

Verification: Record exact commands, exit results, browser observations, and
evidence paths. Use a test corpus and a dedicated test database.

## Policy limitation

The Next.js UI checks (TypeScript, component tests, build) are outside the SOP
command policy, which authorizes only `go`, read-only `git`, and the scoped `cargo`
shapes. UI-001 and UI-002 therefore carry **no SOP-authorizable acceptance check**;
their verification is **operator-run evidence**, recorded separately and never
counted as SOP-verified criteria. Do not disguise Node.js commands as Cargo or Go
commands to bypass command policy.

## Scheduling and SOP execution

UI-001 and UI-002 may proceed independently while the migration is paused. UI-003
and later tasks require their listed services. RUST-008 completion remains dependent
on RUST-007. Do not create a backward dependency from an earlier migration task to
UI work.

Preserve existing restrictions on SOP execution, installation, commits, pushes, and
merges. Creating this plan does not lift those restrictions.

## Evidence and tracking

Store UI evidence under `docs/reports/rust-migration/web-ui/`. Update
`docs/reports/rust-migration/sop-development-process.md` at task boundaries.

Track user turns, manual continuations, SOP invocations, task attempts, approvals,
plan repairs, and SOP-internal agent calls separately. External development-agent
calls remain UNKNOWN unless instrumented. Distinguish manual prototype work from
SOP-managed execution. Do not claim time savings without a measured comparison.

| Task | Status | Evidence |
| --- | --- | --- |
| UI-001 | PLANNED | Pending |
| UI-002 | PLANNED | Pending |
| UI-003 | PLANNED | Pending |
| UI-004 | PLANNED | Pending |
| UI-005 | PLANNED | Pending |

## Stop conditions

Stop the affected work when a required backend contract is unavailable, behavior
conflicts with the migration baseline, a required check cannot run, or
implementation requires a scope change. Record the evidence and smallest decision
needed. Continue independent authorized tasks where possible. Never mark mock
behavior, skipped checks, or unfinished production wiring as complete integration.
