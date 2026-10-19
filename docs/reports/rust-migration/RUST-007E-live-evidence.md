# RUST-007E live production RAG evidence

Live Ollama + PostgreSQL/pgvector evidence for the Rust production `ingest` and
`query` binaries. This record is separate from deterministic mock tests: no
generation was mocked, and every citation below was resolved against a stored
row in the run-owned corpus.

## Run identity

| Item | Value |
| --- | --- |
| Date | 2026-10-19 (UTC) |
| Branch | `rust-007e-functional-closure` |
| Source commit under test | `8b1ab39` plus the RUST-007E working-tree changes |
| Ollama | `0.40.4` at `http://127.0.0.1:11434` |
| Embedding model | `nomic-embed-text` (768 dimensions) |
| Chat model | `llama3.2:3b` |
| Database | dedicated PostgreSQL 16 `rag_migration_test` on `127.0.0.1:5432` |
| pgvector | `0.6.0` |
| Corpus | `rust-007e-live-46114-1792431976910637237` (unique, deleted after the run) |
| Binaries | `target/debug/ingest`, `target/debug/query` (production binaries) |

## Configuration

| Setting | Value |
| --- | --- |
| `DATABASE_URL` | the dedicated test database from the repository `.env` |
| `OLLAMA_URL` | `http://127.0.0.1:11434` |
| `OLLAMA_EMBED_MODEL` | `nomic-embed-text` |
| `OLLAMA_CHAT_MODEL` | `llama3.2:3b` |
| `EMBED_DIM` | `768` |
| `CORPUS_LANGUAGE` | `english` |
| `CHUNK_SIZE` / `CHUNK_OVERLAP` | `50` / `10` |
| `MIN_SIMILARITY` | `0.3` |
| `TOP_K` / `FINAL_K` / `EXPAND_LIMIT` | `4` / `3` / `20` |
| `REQUEST_TIMEOUT` | `180s` |
| `RAG_CORPUS` | the unique run corpus above |
| Sources | `live-policy.md`, `live-text-layer.pdf`, `live-note.txt` |

## Commands

The acceptance test drove the production binaries directly:

```bash
cargo build --locked --bin ingest
cargo build --locked --bin query

# Markdown
target/debug/ingest examples/loan-policy.md        # RAG_SOURCE=live-policy.md
# PDF (two-page text layer)
target/debug/ingest fixtures/parity/inputs/text-layer.pdf  # RAG_SOURCE=live-text-layer.pdf
# Plain text
target/debug/ingest <tmp>/rust-007e-note.txt       # RAG_SOURCE=live-note.txt

target/debug/query "What happens if someone misses a payment?"
target/debug/query "What text appears on page two of the PDF document?"
target/debug/query "What is the RUST-007E support code?"
target/debug/query "What is the RUST-007E support code?"   # empty corpus
```

All commands ran with the configuration in the table above. The acceptance
harness retried a live query up to three times when generation produced no valid
citation; the successful attempts are recorded below.

## Results

| Query | Result |
| --- | --- |
| Markdown: missed payment | Grounded answer from `live-policy.md` describing the 10-day grace period |
| PDF: page two text | Grounded answer from `live-text-layer.pdf`, page 2 |
| Plain text: support code | Answer contained `ZX-431` from the plain-text note |
| Empty corpus | Baseline refusal, no citations |

The stored corpus contained the Markdown sections, both PDF pages, and the
single plain-text section. Every citation below resolved to a row with the same
corpus, source, section, and (where present) page.

## Citations

```text
source: live-policy.md - Missed Payments
source: live-text-layer.pdf - Page 2 - p.2
source: live-note.txt - rust-007e-note.txt
```

These exercise the three provenance shapes the migration must preserve:
page-less Markdown, paged PDF, and page-less plain text. The PDF citation carries
the 1-based page number from the PDF loader.

## Refusal

With `RAG_CORPUS` set to an empty corpus, the production `query` binary returned
exit status 0, no citations, and exactly:

```text
I cannot answer this question from the provided documents.
```

This is the baseline refusal (`answerability::REFUSAL`), reached without a
generation call when retrieval has no passages.

## Observations

- Ingestion and querying used the real Rust production binaries; no scripted
  generation or fixture substitution was involved.
- Live generation is stochastic. The answer text varies between runs, so the
  acceptance check asserts citation validity and the grounding facts rather than
  exact wording. The recorded answers above are one successful run.
- The run used a unique corpus and deleted only that corpus afterwards. No
  shared or production data was reset or modified.
- `MIN_SIMILARITY` was lowered to `0.3` for the small live corpus so the
  production retrieval path had passages to answer from; this is a run
  configuration, not a change to the default.
- The deterministic K=3 parity and Markdown/plain-text acceptance crates provide
  the reproducible evidence; this document records the live observation.
