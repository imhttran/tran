# Phase 0 Endpoint, CLI, Rewrite, and Database Access Map

This report maps the real Rust API endpoints, CLI entry points, the Next.js
rewrite, and database read and write paths with file paths.

## Rust API routes

Defined in `src/api.rs` in `AppState::router`:

| Route | Method | Handler | File |
| --- | --- | --- | --- |
| `/api/health` | GET | `health` | `src/api.rs` |
| `/api/query` | POST | `query` | `src/api.rs` |
| `/api/documents/ingest` | POST | `ingest` | `src/api.rs` |
| `/api/documents/{source}/status` | GET | `status` | `src/api.rs` |

- Server binding: `serve` in `src/api.rs` (default loopback `127.0.0.1:8080`).
- Router construction: `AppState::router` in `src/api.rs`.
- `/api/health` calls `PgStore::ping` (`src/storage_pg.rs`) and the Ollama
  `/api/tags` probe (`model_reachable` in `src/api.rs`).
- `/api/query` calls `QueryService::from_config` and `answer`
  (`src/query_service.rs`).
- `/api/documents/ingest` calls `IngestService::ingest_source`
  (`src/service.rs`) with `PgStore` as both reader and writer (`src/storage_pg.rs`).
- `/api/documents/{source}/status` calls `PgStore::source_status`
  (`src/storage_pg.rs`).

## CLI entry points

Binary targets under `src/bin/` (each is a `main` entry point):

| Entry point | File | Purpose |
| --- | --- | --- |
| `api` | `src/bin/api.rs` | Serve the Axum API |
| `ingest` | `src/bin/ingest.rs` | Ingest a source through the shared pipeline |
| `query` | `src/bin/query.rs` | Ask a question through the shared query service |
| `eval` | `src/bin/eval.rs` | Run retrieval evaluation |
| `expbilingual` | `src/bin/expbilingual.rs` | Bilingual experiment entry point |
| `smoke` | `src/bin/smoke.rs` | Smoke check entry point |

## Next.js rewrite

`web/next.config.ts` defines `rewrites()`:

- `source`: `/api/:path*`
- `destination`: `${process.env.API_URL || "http://127.0.0.1:8080"}/api/:path*`

The browser talks only to Next.js; `/api/*` is proxied server-side to the Rust
API. `API_URL` stays server-side and no CORS is exposed.

## Database read and write paths

Storage adapter: `src/storage_pg.rs` (`PgStore`). Schema under `migrations/`.

Reads:
- `PgStore::ping` — health probe read (`src/api.rs` `/api/health`).
- `PgStore::source_status` — reads `ingestion_sources` and chunk sections for a
  `(corpus, source)` (`src/api.rs` `/api/documents/{source}/status`).
- Retrieval reads via `src/retrieval_pg.rs`, used by `QueryService`
  (`src/query_service.rs`) for `/api/query` and the `query` CLI.

Writes:
- `PgStore::replace_source` — deletes and inserts document/chunk rows for a
  source (`src/storage_pg.rs`; called from `IngestService::ingest_source` in
  `src/service.rs`). Note: it currently deletes by `source` alone
  (`src/storage_pg.rs`), which is the corpus-scoping gap fixed in Phase 1.
- `ingestion_sources` upsert — ingestion metadata written by the ingest path
  (`src/storage_pg.rs`, schema `migrations/002_ingestion_metadata.sql`).

Schema tables present (`migrations/`): `documents`, `chunks`
(`migrations/001_init.sql`), `ingestion_sources`
(`migrations/002_ingestion_metadata.sql`), `corpora`
(`migrations/005_add_corpus.sql`), plus language and page columns
(`migrations/003_add_language.sql`, `migrations/004_page.sql`).
