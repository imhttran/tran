# RAG Web UI — RUST-008A

A Next.js App Router UI over the real Rust Axum API. The browser calls only
same-origin Next.js `/api/*` endpoints; the Next.js server proxies those
requests to Axum, and Axum calls the same ingestion, retrieval, generation, and
citation-validation services as the Rust CLI.

```text
Browser -> Next.js /api/* -> Axum -> Rust services -> PostgreSQL + Ollama
```

No data is mocked and no credential reaches the browser. `API_URL` is a
server-side setting; both Next.js and Axum bind `127.0.0.1` by default.

## What it does

- Uploads `.md`, `.markdown`, `.txt`, and `.pdf` documents (10 MiB limit).
- Reports ingestion status with stored chunks, sections, and page provenance.
- Rejects incomplete PDFs before embedding or any database write.
- Asks questions and renders grounded answers with source citations.
- Shows PDF page references when the source carries page metadata.
- Shows loading, empty, connection-failure, extraction-error, and
  insufficient-evidence states.

## Run

Start the database and Ollama as for the CLI, then:

```sh
cargo build --locked --bin api
target/debug/api            # 127.0.0.1:8080, RAG_CORPUS default "trusted"
make web-start                   # 127.0.0.1:3000
```

The API reads the same environment as the CLI (`DATABASE_URL`, `OLLAMA_URL`,
`OLLAMA_EMBED_MODEL`, `OLLAMA_CHAT_MODEL`, `EMBED_DIM`, `RAG_CORPUS`,
`TOP_K`/`FINAL_K`/`MIN_SIMILARITY`, `REQUEST_TIMEOUT`). `API_HOST` and
`API_PORT` select the bind address.

## Supported Node and npm

Declared in `package.json` (`engines`); tested with Node 24.21.0 and npm
11.19.0:

| Tool | Supported |
| --- | --- |
| Node | `>=20.0.0 <25.0.0` |
| npm | `>=10.0.0 <12.0.0` |

Every web command validates both versions first. The toolchain is resolved, in
order, from explicit overrides (`NODE_BIN`, `NPM_BIN`), then `PATH`, then Zed's
bundled Node. Nothing is installed or upgraded automatically.

## Commands

```sh
npm run dev          # http://127.0.0.1:3000
npm run typecheck    # tsc --noEmit
npm run build        # next build
npm run e2e          # Playwright (servers must already be running)
```

From the repository root:

```sh
make web-check-env   # resolve and validate the Node/npm toolchain
make web-install     # install web dependencies
make web-start       # start Next.js on 127.0.0.1:3000 (background)
make web-status      # report state, URL, and log path
make web-stop        # stop the Next.js server started by make web-start
make web-typecheck   # tsc --noEmit
make web-build       # production build (into .next-build)
make web-e2e         # start Axum + Next.js, run Playwright, clean up the corpus
```

`make web-e2e` is the real-browser verification. It needs PostgreSQL and Ollama,
uses a unique corpus, and deletes only that corpus afterwards. Evidence and
screenshots are recorded under `docs/reports/rust-migration/web-e2e/` and
summarized in `docs/reference/rust-008a-browser-evidence.md`.

## UI state mapping

| Axum error / condition | UI state |
| --- | --- |
| loading request in flight | loading |
| no documents ingested | empty |
| `/api/health` unreachable (`dependency_unavailable`) | connection-failure |
| `incomplete_pdf`, `no_sections`, `extraction_failed` | extraction-error |
| `insufficient: true` in `/api/query` | insufficient-evidence |

## API contract

The browser-facing contract is in
[`docs/reports/rust-migration/web-ui/API-CONTRACTS.md`](../docs/reports/rust-migration/web-ui/API-CONTRACTS.md)
with the RUST-008A implementation differences recorded in
[`docs/reference/rust-008a-browser-evidence.md`](../docs/reference/rust-008a-browser-evidence.md).

`next.config.ts` keeps the same-origin `/api/*` rewrite. The browser never
receives the Axum URL, database DSN, or model configuration.
