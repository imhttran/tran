# Tran RBAC — Phase 0 Preflight and Baseline

- **Plan:** `docs/plans/PLAN-TRAN-RBAC.md`
- **Plan ID:** `plan-tran-rbac`
- **Worktree:** `.worktrees/plan-tran-rbac`
- **Branch:** `sop/plan-tran-rbac`
- **Base:** `main` at `a2f2412`
- **Date:** 2026-10-19

This report records the repository map, the current genealogy-catalog status, the
identity design, and the baseline verification evidence for Phase 0. It does not
approve any Head of Family designation.

## 1. Repository map

### 1.1 Rust HTTP API

The API is built in `src/api.rs`; the binary entry point is `src/bin/api.rs`.

| Route | Method | Source | Notes |
| --- | --- | --- | --- |
| `/api/health` | GET | `src/api.rs:83` (`health` at `:155`) | Public liveness/readiness probe. Must remain non-sensitive. |
| `/api/documents/ingest` | POST (multipart) | `src/api.rs:84` (`ingest` at `:209`) | Accepts `md`, `markdown`, `txt`, `pdf`; 10 MiB upload limit. |
| `/api/documents/{source}/status` | GET | `src/api.rs:85` (`status` at `:421`) | Reads source status for the configured corpus. |
| `/api/query` | POST (JSON) | `src/api.rs:86` (`query` at `:476`) | Runs retrieval, generation, and citation validation. |

Shared state and router construction are in `src/api.rs:48-89`. The server binds
loopback by default and reads `API_HOST`, `API_PORT`, and `RAG_CORPUS` in
`src/api.rs:95-117`. Database and model configuration stay server-side and are
not returned to the browser.

### 1.2 CLI entry points

| Binary | Path | Corpus behavior |
| --- | --- | --- |
| `api` | `src/bin/api.rs` | Uses `RAG_CORPUS` (default `trusted`). |
| `ingest` | `src/bin/ingest.rs` | Defaults to `trusted` (`:40`), reads `RAG_CORPUS` (`:64`). |
| `query` | `src/bin/query.rs` | Reads `RAG_CORPUS` (`:34`, default `trusted`). |
| `eval` | `src/bin/eval.rs` | Reads `RAG_CORPUS` (`:48`). |
| `expbilingual` | `src/bin/expbilingual.rs` | Reads `RAG_CORPUS` (`:47`). |
| `smoke` | `src/bin/smoke.rs` | Local smoke harness. |

The CLI is a local operator interface. It is not a remote family-user API, and
browser RBAC does not protect unrestricted shell or database access.

### 1.3 Next.js proxy

`web/next.config.ts:11-18` rewrites `/api/:path*` to
`${API_URL || "http://127.0.0.1:8080"}/api/:path*`. The browser only calls the
Next.js origin; `web/src/lib/api.ts` uses same-origin `fetch` for health,
ingestion, status, and query. There is no browser CORS path to Axum.

This rewrite is currently unrestricted. Phase 2 must replace it with a
server-side authenticated proxy that forwards only the verified identity
credential and rejects client-supplied identity headers.

### 1.4 Database reads and writes

Schema is defined in `migrations/`:

| Migration | Contents |
| --- | --- |
| `001_init.sql` | `vector` extension, `documents` table, embedding index |
| `002_ingestion_metadata.sql` | ingestion metadata and source index |
| `003_add_language.sql` | language column |
| `004_page.sql` | page column |
| `005_add_corpus.sql` | corpus column and corpus index |

The only catalog table is `documents`. There are no `branches`, `people`,
`relationships`, evidence assertions, or `head_of_family_appointments` tables.

Database access in `src/storage_pg.rs`:

| Method | Line | Scope |
| --- | --- | --- |
| `source_status(corpus, source)` | `:183` | Correctly scopes by `corpus` and `source`. |
| `find_by_content_hash(hash, source)` | `:235` | Scopes by hash and optional source, but **not** corpus. |
| `rows_for_corpus(corpus)` | `:269` | Scopes by corpus. |
| `vector_search(corpus, ...)` | `:287` | Scopes by corpus. |
| `keyword_search(...)` | `:322` | Must be corpus-scoped; verify in Phase 1. |
| `section_chunks(...)` | `:385` | Must be corpus-scoped; verify in Phase 1. |
| `find_by_id(id)` | `:435` | Reads by row id only; callers must not treat it as corpus authorization. |
| `write_chunk(chunk)` | `:491` | Inserts the chunk's corpus. |
| `replace_source(source, chunks)` | `:527` | **Defect:** deletes with `DELETE FROM documents WHERE source = $1` at `:546`, so the same filename in another corpus is destroyed. |

`src/storage.rs` defines the `DocumentStore` write contract.
`src/transaction.rs` validates embedding dimensions before replacement.

Phase 1 must change `replace_source` to `(corpus, source)` and pass an explicit,
validated corpus through every read and write path, including
`find_by_content_hash`, `find_by_id`, keyword search, expansion, and CLI reads.
Legacy `NULL`-corpus rows must not be exposed automatically.

## 2. Genealogy catalog status

The people/branch catalog is **absent**. No migration or source file defines
branches, people, relationships, cited genealogy assertions, living-status
records, or Head of Family appointments. Phase 3 must add only the minimum stable
schema needed for the designation and must not invent or bulk-assign people.

## 3. Identity and authorization approach

The implementation will use standards-based OIDC for multi-user browser access,
verified in Rust:

- Validate issuer, audience, signature using cached JWKS, expiration, and
  subject.
- Bind accounts by immutable provider subject; never bind by email alone.
- Reject tokens on any verification error; never grant roles from unverified
  token claims.
- Load application roles (`admin`, `editor`, `viewer`) and corpus grants from
  the database, not from client headers or request bodies.
- Require authentication for all endpoints except a non-sensitive `/api/health`.
- Return `401` for missing or invalid login and `403` for authenticated
  insufficient permission.
- Reject unauthorized requests before document staging, database reads/writes,
  embeddings, or model generation.
- Replace the unrestricted Next.js rewrite with a server-side authenticated
  proxy; use CSRF protection and secure, `HttpOnly`, `SameSite` cookies.
- Use isolated, cryptographically valid test identities. Do not add an
  authentication-bypass switch.
- Keep the Head of Family designation separate from the `admin` role.

No OIDC provider, TLS termination, or production secret is configured in this
environment. Phase 0 records that as an environment limitation; it does not
authorize a production or public deployment.

## 4. Baseline evidence

### 4.1 Environment

| Item | Result |
| --- | --- |
| Rust | `cargo 1.99.1`, `rustc 1.99.1` |
| Node/npm | Node `v24.21.0`, npm `11.19.0` via Zed’s bundled toolchain |
| PostgreSQL | User-owned PostgreSQL 16.15 cluster on `127.0.0.1:5434` |
| pgvector | `0.6.0` installed |
| Database URL | `postgres://rag@127.0.0.1:5434/rag?sslmode=disable` (local, trust auth, no production secret) |
| Docker | Not installed; the repository’s `docker compose` path is unavailable |
| Ollama | Reachable at `127.0.0.1:11434`; `llama3.2:3b` available |
| Playwright | Chromium installed; 5 existing E2E tests pass |

The local cluster was initialized with `initdb -D /tmp/tran-pgdata -U rag`,
started on port 5434, and migrated with `make db-schema`. This is a local
verification environment only.

### 4.2 Commands

| Command | Result | Evidence |
| --- | --- | --- |
| `cargo fmt --check` | PASS | exit 0 |
| `cargo build --locked` | PASS | exit 0 |
| `cargo test --locked` | PASS with local `DATABASE_URL` | exit 0 |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS | exit 0 |
| `make rust-check` | PASS | exit 0, including transition-evidence acceptance tests |
| `make web-check` | PASS | exit 0, typecheck and production build |
| `make web-e2e` | PASS | exit 0, 5/5 Playwright tests passed |

`cargo test --locked` without `DATABASE_URL` fails one binary test with:
`DATABASE_URL is not set: this test needs a reachable database...`. This is an
environment requirement, not a code failure. The `.env` file supplies the local
DSN for SOP validation and is gitignored.

## 5. Risks and unresolved decisions

1. The verified corpus-isolation defect in `replace_source` is still present.
2. No genealogy catalog or evidence exists, so no person can be evaluated for
   Head of Family yet.
3. No canonical third-branch identifier or verified living-status evidence has
   been established. Any candidate must remain pending.
4. No succession rule exists for a deceased or ineligible first-born male. Do
   not infer one.
5. OIDC provider, TLS, secure session configuration, and the initial-admin
   bootstrap are not yet configured.
6. The local PostgreSQL cluster is an operator convenience, not a production
   deployment. Do not treat it as a production secret store.

## 6. Phase 0 conclusion

The baseline is green on the available Rust, web, and browser checks. The
repository map and genealogy-catalog gap are recorded. Phase 1 must start with
the `replace_source` corpus-isolation fix before any RBAC work.
