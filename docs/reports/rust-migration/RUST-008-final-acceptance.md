# RUST-008 — final acceptance evidence

Date: 2026-10-19. Source revision: `2f067a2b0dcc13f9306e591f950741ae76f391fd`
(`rust-008-final-acceptance`). Installed SOP:
`7540edc9bc2b3bb5a9c6cfe4f3799021b807e085` (`sop dev`).

This document records the operator-run evidence for the three manual RUST-008
criteria. The deterministic Rust criteria are covered by the commands below;
the manual criteria cover startup documentation and real-browser behavior.

## Environment

| Component | Value |
| --- | --- |
| Rust | `rustc 1.99.1 (58cf8f9c3 2026-10-13)` |
| Cargo | `--locked` with the committed `Cargo.lock` |
| Node / npm | `v24.21.0` / `11.19.0` (declared range `>=20 <25`, `>=10 <12`) |
| PostgreSQL | local PostgreSQL 16 with pgvector, dedicated `rag_migration_test` database |
| Ollama | local runtime; `nomic-embed-text` for embeddings and `llama3.2:3b` for generation |
| Browser | Playwright Chromium (through `make web-e2e`) |

The run used a unique corpus (`rust-008a-e2e-20261019T171356-263813`) and deleted
only that corpus afterwards. No shared data was removed.

## Startup commands and browser URL (criterion 1)

Documented in `README.md` and `web/README.md`:

```sh
cargo build --locked --bin api
target/debug/api                  # Axum API, 127.0.0.1:8080
make web-start                    # Next.js, 127.0.0.1:3000
```

Browser URL: **http://127.0.0.1:3000**

`API_HOST` and `API_PORT` override the Axum bind address. `API_URL` and the
database/model settings stay server-side; the browser receives neither.

## Deterministic and integration gates

| Command | Result |
| --- | --- |
| `make rust-check` | **PASS** — fmt, build, tests, Clippy (`-D warnings`), evidence, final-layout, clean-checkout |
| `make rust-acceptance-api-contract` | **PASS** — 1 contract test |
| `make rust-acceptance-functional` | **PASS** — K=3 parity (1) and Markdown/text ingestion (1) |
| `make rust-acceptance-live-rag` | **PASS** — 2 tests (live evidence + production ingest/query/cited answer) |
| `make web-check` | **PASS** — `tsc --noEmit` and `next build` |
| `make web-e2e` | **PASS** — 5 Playwright tests in 12.9s |

No Go toolchain is required: `acceptance/final_layout` and
`acceptance/clean_checkout` pass, and the repository contains no `go.mod`,
`go.sum`, or Go sources.

## Upload → ingest → question → cited answer (criterion 2)

`make web-e2e` drove the real browser against the real Axum API, PostgreSQL, and
Ollama. Verified in Chromium:

1. Empty state renders and `/api/health` reports `API up`, `Database up`, and
   `Model up`.
2. PDF upload (`fixtures/parity/inputs/text-layer.pdf`) ingests with page
   provenance; the page-two question returns an answer and a `text-layer.pdf —
   Page 2` citation with page 2.
3. Markdown upload (`examples/loan-policy.md`) stores 6 chunks with sections;
   the missed-payment question returns a grounded `loan-policy.md — Missed
   Payments` citation.
4. Every browser `/api/*` request was same-origin `http://127.0.0.1:3000/api/`;
   no request reached the Axum port `127.0.0.1:8080` directly.

Screenshots from the acceptance run (preserved with the mock-only label
visible):

- `docs/reports/rust-migration/web-e2e/rust-008-acceptance/pdf-page-answer.png`
- `docs/reports/rust-migration/web-e2e/rust-008-acceptance/markdown-answer.png`

## UI states (criterion 3)

`make web-e2e` covers each required state in the real browser:

| State | Test | Screenshot |
| --- | --- | --- |
| Loading | pending-query indicator | `web-e2e/rust-008-acceptance/loading-state.png` |
| Empty | initial empty state (test 1) | `web-e2e/rust-008-acceptance/markdown-answer.png` (populated), test assertion `empty-state` |
| Connection failure | `/api/health` aborted; visible error and `down` status | `web-e2e/rust-008-acceptance/connection-failure.png` |
| Extraction error | incomplete PDF rejected; page 2 named | `web-e2e/rust-008-acceptance/extraction-error.png` |
| Insufficient evidence | no-context refusal state | `web-e2e/rust-008-acceptance/insufficient-evidence.png` |

The incomplete-PDF case returns `422 incomplete_pdf` before embedding or any
database write, as asserted by the Rust `api_contract` test.

## Intentional differences

The RUST-008A contract differences remain in force and are recorded in
`docs/reference/rust-008a-browser-evidence.md`: incomplete PDFs return `422`
with `incomplete_pages`, the document-status endpoint is added, `top_k` is
validated but server settings govern retrieval, and `sources[].text` is the
cited passage body.

## Mock-only label removed

The plan required an explicit mock-only label until the real Rust API and
pipeline passed end-to-end verification. That verification passed, the operator
accepted the three manual criteria, and SOP recorded RUST-008 as
`VERIFIED / LOCAL_DONE`; the banner (`web/src/components/MockOnlyBanner.tsx`
and its mount in `web/src/app/layout.tsx`) was then removed. The acceptance
screenshots above were captured while the label was still present; the
post-removal UI is covered by the rerun results below.

## Operator decision (recorded)

The operator reviewed this evidence and recorded the three evidence-bound
acceptance decisions, then approved the SOP gate:

```sh
sop outcome RUST-008 --accept criterion-1 \
  --evidence docs/reports/rust-migration/RUST-008-final-acceptance.md --by operator
sop outcome RUST-008 --accept criterion-2 \
  --evidence docs/reports/rust-migration/RUST-008-final-acceptance.md --by operator
sop outcome RUST-008 --accept criterion-3 \
  --evidence docs/reports/rust-migration/RUST-008-final-acceptance.md --by operator
sop approve RUST-008 --by operator
```

SOP then reported `outcome: VERIFIED`, `validation: passed`, and
`lifecycle: LOCAL_DONE` (22/22 tasks, final gate PASS).

## Post-removal verification

After the banner was deleted and its mount removed from
`web/src/app/layout.tsx`, the affected checks were rerun:

| Command | Result |
| --- | --- |
| `make web-check` | **PASS** — `tsc --noEmit` and `next build` |
| `make web-e2e` | **PASS** — 5 Playwright tests in 13.3s |
| `make rust-check` | **PASS** — fmt, build, tests, Clippy, evidence, final-layout, clean-checkout |
| `sop verify RUST-008` | **VERIFIED**, validation passed, `LOCAL_DONE`; all seven criteria met |

The post-removal browser run used corpus
`rust-008a-e2e-20261019T174725-276684`, regenerated the screenshots at
`docs/reports/rust-migration/web-e2e/*.png` without the banner, and deleted only
that run's corpus. The original acceptance screenshots are preserved under
`docs/reports/rust-migration/web-e2e/rust-008-acceptance/`.
