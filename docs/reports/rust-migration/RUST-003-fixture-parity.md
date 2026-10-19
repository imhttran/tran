# RUST-003 — section/chunk fixture parity (acceptance matrix)

Deterministic parity evidence for the document-processing port (RUST-003). The
Rust section parser, chunker, and loaders are compared against the **pinned Go
reference**; the expected values are generated from Go and are never edited to
match Rust.

## Provenance

| Field | Value |
| --- | --- |
| Reference commit | `d72fb398e8959eea83ed24be03f6df8cdeca86a2` (`d72fb39`) |
| Go toolchain | `go1.27.1` |
| Generator | `tools/genfixtures` (a read-only consumer of `internal/document`, `internal/loader`, `internal/chunking`) |
| Command | `go run ./tools/genfixtures` (from the repository root) |
| Fixture | `rust/fixtures/parity/section_chunk_parity.json` |
| Inputs | `rust/fixtures/parity/inputs/` (`plain.txt`, `empty.txt`, `text-layer.pdf`, `mixed-text-scan.pdf`, `no-text-layer.pdf`) |
| Rust test | `rust/tests/section_chunk_parity.rs` (`cargo test --test section_chunk_parity`) |

## Repairs made to reach parity (implementation, not fixtures)

| # | Divergence (Rust → Go) | Repair |
| - | ---------------------- | ------ |
| R1 | Section parser kept a pre-heading block as a `""`-titled section | `parse_sections` now ignores text before the first `## ` heading |
| R2 | Chunker sliced **bytes** | `chunk_sections` now works on **words**, joins with single spaces, and returns no chunks when `size == 0` or `overlap >= size` |
| R3 | Plain-text loader split on `## ` | `load_text(path)` yields one section titled with the file's base name |
| R4 | PDF section title empty | PDF sections titled `Page N` with 1-based `page` |
| R5 | Partial PDF extraction dropped its status | `load_pdf` now returns `PdfExtraction { sections, incomplete_pages }` (see below) |

## Acceptance matrix

18 cases; **all section, chunk, and extraction-status outputs match the Go
reference** (section title/text, chunk content/boundaries, ordering,
document-global index, page metadata, and loader status/incomplete pages).

| Case | Kind | Status | Sections | Chunks |
| ---- | ---- | ------ | -------- | ------ |
| md-english-sections | markdown | Complete | match | match |
| md-vietnamese-sections | markdown | Complete | match | match |
| md-empty | markdown | Complete | match (none) | match (none) |
| md-whitespace-only | markdown | Complete | match (none) | match (none) |
| md-leading-block | markdown | Complete | match | match |
| md-boundary-exact-size | markdown | Complete | match | match |
| md-boundary-one-over | markdown | Complete | match | match |
| md-multi-chunk-overlap | markdown | Complete | match | match |
| md-repeated-title | markdown | Complete | match | match |
| md-heading-edge | markdown | Complete | match | match |
| md-unicode-words | markdown | Complete | match | match |
| md-zero-size | markdown | Complete | match | match (none) |
| md-overlap-equals-size | markdown | Complete | match | match (none) |
| txt-single-section | text | Complete | match | match |
| txt-empty | text | Complete | match (none) | match (none) |
| pdf-text-layer | pdf | Complete | match | match |
| pdf-mixed | pdf | **Incomplete** | match | match |
| pdf-no-text-layer | pdf | **Hard error** | match (none) | match (none) |

## PDF extraction status parity

`load_pdf` returns `Result<PdfExtraction, PdfError>`, mirroring Go's
`([]Section, error)` shape without losing the readable pages:

- **Complete** — `PdfExtraction { sections, incomplete_pages: [] }`.
- **Incomplete** — `PdfExtraction` with the readable `sections` **and** the
  non-empty `incomplete_pages` (the baseline's `*IncompleteExtractionError`).
  For `mixed-text-scan.pdf` this is sections `[Page 1]` with `incomplete_pages
  [2]`, exactly as Go.
- **Hard error** — `Err(PdfError::Scanned)` when every page lacks text (the
  baseline's "has no extractable text layer: OCR is required"); `Err(PdfError::Invalid)`
  for an unreadable/malformed PDF.

`pdf_extraction_status_matches_reference` asserts the reference's status category
and its exact incomplete-page list per case. The difference is **not** documented
away.

`source` and `language` are propagated onto every chunk; the Go reference attaches
them at ingestion, so this is checked as propagation
(`chunk_source_and_language_are_propagated`).

## Commands run (all pass)

| Check | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo test` | 69 lib + 10 integration + 5 parity + 1 doc test pass |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `go build ./...`, `go vet ./...` | clean |
| `go test ./...` | 16 packages `ok`, no failures |
| `gofmt -l .` | flags `cmd/retrieval/main.go` — **pre-existing, unmodified vs HEAD** (reference formatting drift; unrelated to this work) |

## Lifecycle (actual SOP state)

SOP state read with `sop status` / `sop verify RUST-003` (**read, never edited**):
**RUST-003 is `LOCAL_DONE`** with a recorded **`PARTIAL`** outcome — the
"deterministic-checks" gate passed, but all five acceptance criteria are
"unverifiable: no deterministic check supplied". SOP therefore does **not** count
RUST-003's acceptance as verified. (An earlier draft of this report mis-stated the
lifecycle as `PLANNED`; `PLANNED` is only the rendered plan's evidence table, not
persisted state.)

This fixture-parity evidence is **external** to SOP's acceptance path: it shows
Rust matches the Go reference, but it is not an executed SOP acceptance check, so
no task is marked `VERIFIED`.

## Caller parity (kept separate from loader parity)

**Loader parity — complete.** All 18 fixture cases match the reference (see the
matrix above).

**Caller parity — in progress (RUST-005B).** The library now exposes the
pre-effect ingest guard `ingest::gate_pdf` / `ingest::gate_sections`, which mirrors
Go's caller: a fatal loader error, a fully scanned PDF, and a partial PDF
(reporting the affected pages) are rejected **before** any embedding call or
database write, as is a source that yields no chunks.
`rust/tests/ingest_caller_rejects_incomplete_pdf.rs` proves this at the caller
level: for `mixed-text-scan.pdf` (and `no-text-layer.pdf`) a recording embedder and
store observe **zero** calls and **zero** writes, while a complete PDF observes one
embedding per chunk and one write (the control case).

**Not yet established:** the production ingest binary (`src/bin/ingest.rs`) is still
a stub, so no *shipped* caller invokes the guard or touches a database; the real
embedding/database wiring is RUST-005B. The guard and its contract test are in
place, but caller parity is not complete until that ingest exists. Go's contract
the guard encodes: `cmd/ingest.loadChunks` fails on **any** `loader.Dispatch` error
(including `*IncompleteExtractionError`) and never ingests a partial extraction.

## Cross-task dependency

RUST-005B's integration check depends on RUST-003's loader status contract
(`PdfExtraction::incomplete_pages`); the revised plan records `Depends on:
RUST-005A, RUST-003` for RUST-005B and an explicit acceptance criterion for this
rejection behavior.
