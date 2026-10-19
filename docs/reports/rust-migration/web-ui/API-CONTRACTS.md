# UI-001 — Screens and API contracts

Status: prototype contract (UI-001), implemented by RUST-008A with documented
differences in [`docs/reference/rust-008a-browser-evidence.md`](../../reference/rust-008a-browser-evidence.md).
The UI is no longer mock-only.

Records the schemas the UI-002 prototype was built against. Every response is now
served by the real Axum API (`src/bin/api.rs`); the browser still calls only
same-origin Next.js `/api/*` endpoints, which the Next.js server proxies.

## Screens and state transitions

Single-page workspace (`/`) with four areas: System status → Document upload →
Ingestion result → Ask/Answer/Sources.

| Area | States |
| --- | --- |
| System status | `mock` · `not-checked` · `up` · `down` (never fabricated percentages) |
| Document upload | `idle` → file selected → `uploading` → `processing` → `done` \| `error` |
| Ingestion result | `complete` · `incomplete` (pages listed) · `invalid-file` · `dependency-down` |
| Question | `idle` → `loading` → `answered` · `insufficient-evidence` · `error` |

## Endpoints (proposed; subject to contract review in UI-003)

| Endpoint | Purpose |
| --- | --- |
| `GET /api/health` | Service and dependency status, no secrets. |
| `POST /api/documents/ingest` | Multipart file -> completed ingestion result or structured error. |
| `POST /api/query` | Question -> answer with structured citations. |

### GET /api/health

    { "status": "ok", "service": "rust-api", "database": "ok", "model": "ok" }

`200` when healthy, `503` with `"status": "degraded"` when a dependency is down.
Never returns credentials or internal error text.

### POST /api/documents/ingest

Request: `multipart/form-data`, one field `file` (`.md`, `.markdown`, `.txt`,
`.pdf`). Response `200`:

    { "ok": true, "source": "loan-policy.md", "chunks": 12, "incomplete_pages": [] }

An incomplete PDF returns `200` with the readable chunks **and** the affected
1-based pages: `"incomplete_pages": [2]` (mirrors `PdfExtraction.incomplete_pages`).
Errors: `400` invalid type or over the size limit; `422` a fully scanned PDF
(all pages lack text; OCR is not offered); `503` a dependency is unavailable.
A rejected ingestion must not write to the database or call the embedder.

### POST /api/query

Request `application/json`:

    { "question": "What happens if someone misses a payment?", "top_k": 4 }

Response `200`:

    {
      "answer": "A late fee is assessed...",
      "insufficient": false,
      "sources": [
        { "source": "loan-policy.md", "section": "Late payments", "page": 1,
          "text": "A late fee is assessed on the first day after the due date." }
      ]
    }

`insufficient: true` (empty `sources`) means the retrieved evidence does not answer
the question — the baseline refusal behavior. Errors: `400` empty question;
`503` dependency unavailable. `page` is `null` when the source carries no page
metadata and must not be invented.

## Supported formats, limits, timeouts

- Formats: `.md`, `.markdown`, `.txt`, `.pdf` — matching implemented loaders. **OCR
  is not offered.**
- Upload size limit: **10 MiB** (selected before implementation; enforced by the API
  and by the Next.js proxy).
- Timeouts: browser->Next.js proxy `30s`; ingestion request `120s`; query request
  `REQUEST_TIMEOUT` (default `5m`). Proxy and request timeouts are defined together.

## Citation fields

`source`, `section`, `page` (nullable), `text` — matching available retrieval
metadata. A page reference must not imply a viewer exists.

## Intentional differences from the `inventory` reference

- No navigation rail and no authentication (this scope has no accounts); `AppShell`
  is a minimal wrapper.
- Domain routes are RAG-specific (`health`/`documents/ingest`/`query`) rather than
  inventory's catalog/sourcing.
- Both servers bind `127.0.0.1` (loopback-only); `API_URL` stays server-side.

## Unresolved contract questions

- Whether a job/status API is needed — deferred until measured ingestion duration
  requires it (no fabricated percentage progress).
- Whether `/api/query` should reuse the baseline `rag-retrieval/v1` JSON envelope
  verbatim; recorded for UI-003 review.
