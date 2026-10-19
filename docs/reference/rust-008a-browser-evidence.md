# RUST-008A — browser integration evidence

Real Axum + Next.js integration over the Rust services verified by RUST-007E.
No mock-only UI remains: the browser talks only to same-origin Next.js `/api/*`,
which proxies server-side to Axum, which calls the shared Rust ingestion,
retrieval, generation, and citation-validation services.

## Implemented endpoints

| Endpoint | Behavior |
| --- | --- |
| `GET /api/health` | `{status, service, database, model}`; `200` healthy, `503` degraded; no credentials |
| `POST /api/documents/ingest` | Multipart `file`; `.md`, `.markdown`, `.txt`, `.pdf`; 10 MiB limit |
| `GET /api/documents/{source}/status` | Stored chunks, sections with chunk index and page, corpus, fingerprint, timestamp |
| `POST /api/query` | `{question, top_k?}` -> `{answer, insufficient, sources[]}` with `source`, `section`, `page`, `text` |

The API binds `127.0.0.1:8080` by default (`API_HOST`/`API_PORT` override).
Database and model settings stay in the process environment. Every handler runs
synchronous database/Ollama work on `spawn_blocking`, never on an async executor
thread.

## Shared services

- Ingestion calls `IngestService`/`IngestPipeline`, the same path the CLI uses,
  including the pre-effect gate, fingerprint, embedding, and transactional
  replacement.
- Query calls `QueryService`, extracted from `bin/query.rs` so the CLI and API
  share one retrieval/generation/citation implementation.
- `AnswerService` now returns the cited `ContextPassage` list used by the API's
  structured `sources`; the CLI still prints the same rendered references.

## Intentional differences from the UI-001 contract

- **Incomplete PDFs return `422` with `incomplete_pages` and write nothing.**
  The UI-001 draft proposed `200` with the readable chunks; the verified Rust
  pipeline rejects incomplete PDFs before embedding or database writes, matching
  the RUST-005B contract and the RUST-008A requirements. The UI renders the
  affected pages as an extraction error.
- **`GET /api/documents/{source}/status` was added.** The UI-001 draft deferred a
  status API; RUST-008A requires ingestion status, so the endpoint returns stored
  chunks, sections, page provenance, corpus, and fingerprint.
- **`top_k` is accepted and validated but does not override server settings.**
  The established request field is preserved; retrieval depth follows the
  configured `TOP_K`/`FINAL_K`, matching the CLI. Documented in the handler.
- **`sources[].text`** is the cited passage body, not the full stored chunk set.

## Deterministic API contract

```bash
cargo test --locked --manifest-path acceptance/api_contract/Cargo.toml
```

Result: **PASS** (1 test). It starts the real `api` binary against a dedicated
PostgreSQL/pgvector database and a deterministic Ollama server and verifies:

- loopback defaults and the 10 MiB limit;
- health `200` with `status/database/model = ok` and no credential strings;
- unsupported type, zero-byte, and oversize uploads rejected;
- incomplete PDF -> `422 incomplete_pdf`, pages `[2]`, no rows and status `404`;
- Markdown ingestion -> stored chunks, ordered sections, page-less rows, corpus,
  fingerprint; duplicate upload -> `unchanged`;
- PDF ingestion -> page provenance `[1, 2]`;
- plain-text ingestion -> one page-less section;
- query -> cited answer whose every source resolves to a stored row (source,
  section, and page);
- empty question -> `400`;
- empty corpus -> baseline refusal, `insufficient: true`, no sources.

## Real browser E2E

Runner: `scripts/web-e2e.sh` (starts Axum + Next.js, runs Playwright, deletes
only the run corpus). `web/playwright.config.ts` drives Chromium.

```bash
make web-e2e
```

Result: **5 passed** (12.2s). Verified in the real browser:

1. Empty state, then PDF upload -> `text-layer.pdf` with pages 1 and 2.
2. Page-two question -> answer citing `text-layer.pdf — Page 2` with page 2.
3. Markdown upload -> 6 stored chunks with sections and a grounded
   `loan-policy.md — Missed Payments` citation.
4. Loading indicator while the question is pending.
5. Connection failure -> visible error and `down` status.
6. Incomplete PDF -> extraction error naming page 2.
7. Insufficient-evidence UI state.
8. All `/api/*` requests were same-origin `http://127.0.0.1:3000/api/`; no
   browser request reached `127.0.0.1:8080`.

Screenshots:

- `docs/reports/rust-migration/web-e2e/markdown-answer.png`
- `docs/reports/rust-migration/web-e2e/pdf-page-answer.png`
- `docs/reports/rust-migration/web-e2e/loading-state.png`
- `docs/reports/rust-migration/web-e2e/connection-failure.png`
- `docs/reports/rust-migration/web-e2e/extraction-error.png`
- `docs/reports/rust-migration/web-e2e/insufficient-evidence.png`

The browser checks are operator-run verification (Node.js/browser execution is
outside SOP's command policy). The deterministic Rust-side contract is the
`api_contract` crate. Operator acceptance of the browser criteria is recorded
through SOP.

## Verification

- `cargo test --locked` — Rust suite
- `cargo test --locked --manifest-path acceptance/api_contract/Cargo.toml` — contract
- `go build ./...`, `go test ./...`, `go vet ./...` — Go reference
- `make web-typecheck`, `make web-build` — Next.js typecheck and production build
- `make web-e2e` — real browser suite
- `make transition-evidence` — RUST-007B transition/clean-checkout checks

Go remains present and operational; RUST-007C was not started.

> Historical note: RUST-007C later completed the Rust-only transition (the Go
tree and Go tooling were removed and the Rust package moved to the repository
root), and RUST-008 recorded the final `VERIFIED / LOCAL_DONE` acceptance. See
`docs/reports/rust-migration/RUST-008-final-acceptance.md`.
