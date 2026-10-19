# RUST-007E — production Markdown and plain-text ingestion

Evidence that the production Rust `ingest` binary and service accept `.md`,
`.markdown`, and `.txt` sources through the same chunk, embed, fingerprint, and
transactional replacement path as PDFs.

## Behavior

- Extension dispatch mirrors the Go loader registry:
  - `.pdf` → PDF text-layer loader and `gate_pdf` (incomplete/scanned pages are
    still rejected).
  - `.md` / `.markdown` → the shared `## ` section parser and `gate_sections`.
  - `.txt` → the plain-text loader (one page-less section, titled by file name)
    and `gate_sections`.
  - any other extension → a loader rejection naming the path.
- Markdown and plain-text rows store `page = NULL`; source identity is the
  requested `RAG_SOURCE` (or the file name), language is the configured
  `CORPUS_LANGUAGE`, chunk indexes preserve document order, and the content hash
  and chunker configuration come from the one shared fingerprint.
- The pre-effect gate runs before any embedding call or database write. Empty
  sources are rejected with the baseline "no sections found" message.
- The unchanged-source fingerprint check and the transactional source
  replacement are unchanged, so a duplicate ingest is a no-op and a failed
  replacement preserves the previous rows.

## Acceptance evidence

`cargo test --locked --manifest-path acceptance/ingest_md_text/Cargo.toml` →
**PASS** (1 test). It drives the real `ingest` binary against a dedicated
PostgreSQL/pgvector database and a deterministic embedding server, and proves:

- Markdown with two sections stores two ordered, page-less rows with the right
  source, language, corpus, and hash;
- plain text stores one page-less section and keeps the note fact;
- a duplicate ingest reports `unchanged` and writes no additional rows;
- empty Markdown, a missing file, and an unknown extension are rejected with no
  rows written;
- an unreachable embedding endpoint writes no rows;
- a changed source that fails on an embedding-dimension mismatch leaves the
  previously stored content and fingerprint intact.

The unit tests in `src/pipeline.rs` additionally cover Markdown/plain-text
dispatch, page-less provenance, empty rejection, and unknown formats with
recording ports (zero embedding calls and zero writes).
