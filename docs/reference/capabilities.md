# Implemented capabilities

These are shipped and covered by unit tests (and, where noted, the PostgreSQL
integration suite). They are recorded here so the code and this document agree:

- **Pluggable document loader (RAG-006)** — `src/loader/` exposes a
  `Loader` interface plus a `Registry`/`Dispatch`; Markdown and plain-text
  loaders ship today. `src/bin/ingest.rs` dispatches by loader, so a new format is added
  by registering a loader, not by editing the ingest path. Unknown formats return
  a clear `no loader for <path>` error.
- **Ingestion provenance and idempotent re-index (RAG-003)** — every chunk stores
  a content hash, embed model, dimension, chunker config, ingest time, and
  language; re-ingesting an unchanged file is a no-op (migrations 002/003).
- **Model/provider independence (RAG-004/RAG-005)** — `src/provider.rs` exposes a
  registry (`provider::embedder` / `provider::generator`) keyed by
  `EMBED_PROVIDER` / `GEN_PROVIDER`, and an unregistered name fails fast at
  `Config::load` with the variable and value. Only `ollama` is registered today,
  and the command binaries still construct the Ollama client directly, so the
  selector is validated but does not yet switch the active client. `EMBED_DIM`
  is validated against the stored column before any write.
- **Multilingual retrieval (RAG-009)** — `CORPUS_LANGUAGE` selects the FTS
  configuration (`normalize_fts_config` in `src/storage_pg.rs`; unset → `english`,
  unsupported → `simple`) and is stored per document; the FTS config is a bound SQL
  parameter.
- **Metadata filtering (RAG-011)** — an optional parameterized `Filter` (source
  prefix with LIKE-escaping, language, ingested-date range) narrows vector search,
  keyword search, and section expansion; a zero filter is byte-identical to the
  unfiltered query.
- **Deterministic context budget (RAG-010)** — `src/answer/context_budget.rs` selects a
  subset of ranked documents whose byte-estimated size fits `CONTEXT_BUDGET`
  (higher-rank priority, round-robin fairness, oversize chunks excluded); `0`
  disables it and keeps the chunk-count behaviour.
- **Reranking hardening (RAG-012) — not present in the Rust tree.** The Go
  baseline degraded a malformed, slow, or failed LLM reranker to the fused order
  (bounded by `RERANK_TIMEOUT`); the Rust retrieval path has no reranker.
- **Structured citation validation (RAG-013)** — `src/answer/citation.rs` parses,
  validates, and repairs `[source - section]` citations; `RAG_CITATION_VALIDATION`
  enables it in the answer path (off by default).
- **Structured observability (RAG-014) — not present in the Rust tree.** The Go
  baseline's `OBSERVABILITY_FORMAT=json` records are not implemented by the Rust
  binaries.
- **Security hardening (RAG-015)** — `MAX_QUESTION_BYTES`/`MAX_INPUT_BYTES` fail
  fast with actionable errors, and the answer prompt separates trusted instructions
  from untrusted retrieved text (see
  [`docs/operations/prompt-injection.md`](../operations/prompt-injection.md)).

## Phase 23b capabilities

- **PDF text-layer ingestion (RAG-007)** — `src/loader/` registers a PDF
  loader that extracts one section per page from the PDF text layer; a PDF with no
  text layer (a scan) is rejected explicitly until OCR is authorized.
- **Page provenance and page-aware citations (RAG-017)** — a 1-based page number
  flows from the loader through chunking, storage (`documents.page`), retrieval,
  and citation rendering; the contract is
  [`docs/operations/page-provenance.md`](../operations/page-provenance.md).

The work still **not** implemented — OCR for scanned PDFs (RAG-008) and genealogy
entity extraction (RAG-016) — is described in the archived
[`docs/history/PRD-Phase-23b.md`](../history/PRD-Phase-23b.md). Its execution plan
was archived with the other RAG task plans and remains in git history.
