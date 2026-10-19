# RUST-001 — Go reference baseline (frozen)

Migration evidence for `PLAN-RUST-MIGRATION.md` task RUST-001. This record freezes the
Go reference implementation so later Rust tasks can compare against it.

Owner of this document: RUST-001 reference-freeze stage.

## 1. Reference identity (S1-reference-freeze)

| Field | Value |
| --- | --- |
| Reference SHA | `d72fb398e8959eea83ed24be03f6df8cdeca86a2` |
| Branch | `main` (tracking `origin/main`) |
| Worktree state at freeze | `main...origin/main` with unrelated user-owned modifications (`.gitignore`, `docs/README.md`) and untracked `docs/plans/PLAN-RUST-MIGRATION.md`, `docs/plans/PLAN-RUST-PORT.md` (default) |
| Repository | `rag-template` (Go module) |
| Go module directive | `go 1.26.0` (`go.mod`) |
| Toolchain used for checks | `GOTOOLCHAIN=go1.26.0` (default) — matches the module directive |
| Capture date | 2026-10-17 (default) |

Every later RUST-00x acceptance check must reference SHA
`d72fb398e8959eea83ed24be03f6df8cdeca86a2`. If the Go tree changes, the baseline is
invalid and must be re-frozen.

## 2. Environment record

| Component | Value | Notes |
| --- | --- | --- |
| Go toolchain | `go 1.26.0` (from `go.mod`) | default; pinned by the module directive |
| PostgreSQL + pgvector | docker-compose service, endpoint `postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable` | default DATABASE_URL from `Makefile` |
| Docker (DB runtime) | operator-supplied (`docker compose up -d --wait`) | default |
| Ollama embedding model | operator-supplied; README pins `nomic-embed-text` | model name not verifiable from repository contents — see Isolated gaps |
| Ollama chat model | operator-supplied; README example `qwen3.8:27b-mlx` via `OLLAMA_CHAT_MODEL` | model name/version not verifiable from repository contents — see Isolated gaps |

External service availability (Ollama, Docker/PostgreSQL) is outside the repository
and is not recorded as confirmed unless a capture in this record states so.

## 3. Pinned commands (S1, verbatim)

Working directory for every command: repository root.

| Purpose | Command | Required env |
| --- | --- | --- |
| Build | `go build ./...` | none |
| Unit tests (no database) | `make test` → `go test ./...` | none |
| Vet | `go vet ./...` | none |
| Format check | `gofmt -l .` (expect empty output) | none |
| Integration suite | `make integration` → `db-up` then `RAG_INTEGRATION=1 go test ./internal/retrieval/ ./internal/ingestion/ -run Integration -p 1 -v` | `RAG_INTEGRATION=1`, database up |
| DB bring-up | `make db-up` → `docker compose up -d --wait` then `make db-schema` | Docker |
| DB tear-down | `make db-down` → `docker compose down` | Docker |
| Ingest | `make ingest FILE=<file>` → `go run ./cmd/ingest <file>` | database, embedder |
| Ask | `make ask Q="<question>"` → `go run ./cmd/rag "<question>"` | database, embedder (+ generator for answers) |
| Retrieval eval | `make eval` → `go run ./cmd/eval` (corpus `evals/retrieval.json`) | database, embedder |
| Bilingual eval | `go run ./cmd/expbilingual` with `ARM` (A,B,C,D1,D2,E,all) and `EVAL_DATASET` | database, embedder, generator |
| Settings sweep | `make sweep AXIS=<axis>` → `./scripts/sweep.sh <axis>` | database, embedder |

## 4. Configuration baseline

Every setting is read from the environment; `.env.example` lists them all. Values
below are the baseline used for this freeze. Settings left at their repository default
are marked **(default)**.

| Setting | Baseline value | Source |
| --- | --- | --- |
| `DATABASE_URL` | `postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable` **(default)** | `Makefile` |
| `CHUNK_SIZE` / `CHUNK_OVERLAP` | repository defaults **(default)** | `.env.example` |
| `MIN_SIMILARITY` | repository default **(default)** | `.env.example` |
| Top-K / Final-K | repository defaults **(default)** | `.env.example` |
| `CONTEXT_BUDGET` | repository default; `0` disables **(default)** | `.env.example` |
| `QUERY_REWRITE` | repository default (off unless set) **(default)** | `.env.example` |
| Rerank settings / `RERANK_TIMEOUT` | repository default (rerank off) **(default)** | `.env.example` |
| `CORPUS_LANGUAGE` | unset → FTS `english`; unsupported → `simple` **(default)** | `internal/retrieval` `NormalizeFTSConfig` |
| `EMBED_PROVIDER` | `ollama` **(default)** | `internal/provider` |
| `EMBED_DIM` | repository default; validated against stored column before writes **(default)** | `.env.example`, `internal/ingestion` |
| `GEN_PROVIDER` | `ollama` **(default)** | `internal/provider` |
| `OLLAMA_CHAT_MODEL` | operator-supplied **(default: unset)** | README |
| `OBSERVABILITY_FORMAT` | `human` **(default)** | `internal/observability` |
| `RAG_CITATION_VALIDATION` | off **(default)** | `cmd/rag` |
| `ARM` / `EVAL_DATASET` (expbilingual) | repository defaults for corpus; `ARM=all` when running every arm **(default)** | `cmd/expbilingual` |

## 5. Dependency snapshot (S1)

| Dependency | Reference | Recorded state |
| --- | --- | --- |
| Go module | `go.mod` | module directive `go 1.26.0`; direct/indirect requirements as committed at the frozen SHA |
| Go checksums | `go.sum` | as committed at the frozen SHA |
| Database service | `docker-compose.yml` | bundled PostgreSQL + pgvector service (default endpoint in §2) |
| Schema | `migrations/` | migrations applied via `make db-schema`; migrations 002/003 add provenance columns |
| Ollama embedder | operator | `nomic-embed-text` (README) — exact version not verifiable from repo |
| Ollama generator | operator | operator-supplied chat model — exact version not verifiable from repo |

## 6. Baseline checks (S2-baseline-checks)

Commands run against the frozen SHA `d72fb398e8959eea83ed24be03f6df8cdeca86a2`,
working directory = repository root.

| Check | Command | Result |
| --- | --- | --- |
| Build | `go build ./...` | PASS |
| Unit suite | `go test ./...` (`make test`) | PASS — all packages `ok` (or `[no test files]` for `cmd/expbilingual`, `cmd/retrieval`, `internal/embedding`, `internal/generation`, `internal/ollama`) |
| Vet | `go vet ./...` | PASS |
| Format | `gofmt -l .` | PASS — expected empty output |
| Integration suite | `make integration` | NOT RUN in this environment — see Isolated gaps |

Per-package unit result at freeze: `cmd/eval`, `cmd/ingest`, `cmd/rag`,
`internal/answerability`, `internal/chunking`, `internal/citations`,
`internal/config`, `internal/contextbudget`, `internal/document`,
`internal/ingestion`, `internal/loader`, `internal/observability`,
`internal/provider`, `internal/rag`, `internal/reranking`, `internal/retrieval`
all PASS.

No source code or test was modified to make a check pass. Every failure mode below
is recorded as an isolated gap, not fixed (fixing Go code is outside RUST-001 scope).

### Database state

Integration tests (`internal/retrieval`, `internal/ingestion`, `-run Integration`)
require the docker-compose PostgreSQL+pgvector service, apply the schema via
`make db-schema` from `migrations/`, and truncate/delete the `documents` table.
They must be run against a disposable database, never shared data.

## 7. Capability → Rust task map (S3-capability-map)

Every capability in `docs/reference/capabilities.md` plus the README command table.
Comparison modes: **exact** (byte/value equality), **float-tol** (float within tolerance
from §9), **order** (deterministic ordering must match), **set** (unordered set equality).

| Capability | Go evidence | Rust task | Verification method (mode; Go artifact) |
| --- | --- | --- | --- |
| Pluggable loader registry/dispatch (RAG-006) | `internal/loader`; `docs/reference/capabilities.md` | RUST-003 | exact; loader dispatch output and `no loader for <path>` error text on shared fixtures |
| Ingestion provenance + idempotent re-index (RAG-003) | `internal/ingestion`; migrations 002/003 | RUST-005 | exact; stored provenance fields and unchanged-source no-op on re-ingest (fixture §11) |
| Provider independence / `EMBED_DIM` validation (RAG-004/RAG-005/RAG-025A) | `internal/provider`; `config.Config.Embedder()`/`Generator()` | RUST-002 (config) + RUST-004 (provider) | exact; config-load failure for unregistered provider names; dimension guard before writes |
| Multilingual FTS config (RAG-009) | `internal/retrieval` `NormalizeFTSConfig` | RUST-006 | exact; FTS config selected for unset/english/supported/unsupported languages |
| Metadata filtering (RAG-011) | `internal/retrieval` `Filter` | RUST-006 | set; filtered result set equals baseline; zero filter byte-identical to unfiltered |
| Context budget (RAG-010) | `internal/contextbudget` | RUST-006 | order; selected subset and priority/round-robin order for a budget |
| Reranking hardening (RAG-012) | `internal/reranking` | RUST-006 | order; malformed/slow/failed reranker falls back to fused order within `RERANK_TIMEOUT` |
| Citation validation (RAG-013) | `internal/citations`; `cmd/rag` | RUST-006 | exact; parsed/validated/repaired `[source - section]` strings on shared cases |
| Structured observability (RAG-014) | `internal/observability` | RUST-006 | exact; JSON record fields (run ID, stage timings, counts, context usage) for a fixed run |
| Security limits / prompt-injection separation (RAG-015) | `internal/config`; `docs/operations/prompt-injection.md` | RUST-002 + RUST-006 | exact; `MAX_QUESTION_BYTES`/`MAX_INPUT_BYTES` errors; prompt separates trusted instructions from untrusted text |
| PDF text-layer ingestion (RAG-007) | `internal/loader` PDF loader | RUST-003 | exact; one section per page from a text-layer PDF; explicit rejection for no-text-layer PDF |
| Page provenance + page-aware citations (RAG-017) | loader→chunk→store→retrieve→citation; `docs/operations/page-provenance.md` | RUST-003 (load/chunk) + RUST-005 (store) + RUST-006 (cite) | exact; 1-based page number on chunk and citation (fixture §12) |
| `cmd/ingest` | `cmd/ingest`; README command table | RUST-005 | exact; stored rows/provenance match fixture §11 |
| `cmd/rag` | `cmd/rag`; README command table | RUST-006 | order (retrieval) + exact (citations); fixture §11/§12 retrieval order and citation strings |
| `cmd/eval` | `cmd/eval`; `evals/retrieval.json` | RUST-007 | float-tol; recall/precision per §9 tolerances on the fixed corpus |
| `cmd/expbilingual` | `cmd/expbilingual` | RUST-007 | float-tol (metrics) + order (arms); fixture §11 Vietnamese retrieval |
| `make sweep` | `scripts/sweep.sh`; `docs/experiments-eval-sweep.md` | RUST-007 | float-tol; metric matrix per axis within §9 tolerances |

Rows whose evidence is documentation only (no test) are marked here: `make sweep`
(documented in `docs/experiments-eval-sweep.md`, no dedicated Go test package),
`cmd/expbilingual` (`[no test files]`), and the OCR-adjacent rejection path (documented
in `docs/reference/capabilities.md`). All other rows have Go unit-test evidence.

### Out of scope

| Capability | Status | Deferring reference |
| --- | --- | --- |
| OCR for scanned PDFs (RAG-008) | Not implemented — out of scope for the map | `docs/requirements/PRD-Phase-23b.md`, `docs/plans/PLAN-RAG-Phase-23b.md` |
| Genealogy entity extraction (RAG-016) | Not implemented — out of scope for the map | `docs/requirements/PRD-Phase-23b.md`, `docs/plans/PLAN-RAG-Phase-23b.md` |

## 8. Evaluation settings (S4-eval-baseline)

Baseline evaluation configuration for `cmd/eval` over `evals/retrieval.json`. All
values are repository defaults unless noted.

| Setting | Baseline value |
| --- | --- |
| `DATABASE_URL` | `postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable` **(default)** |
| `CHUNK_SIZE` / `CHUNK_OVERLAP` | repository defaults **(default)** |
| `MIN_SIMILARITY` | repository default **(default)** |
| Top-K / Final-K | repository defaults **(default)** |
| `CONTEXT_BUDGET` | repository default **(default)** |
| `QUERY_REWRITE` | repository default **(default)** |
| Rerank settings | repository default (off) **(default)** |
| `CORPUS_LANGUAGE` / FTS config | unset → `english` **(default)** |
| `EMBED_PROVIDER` / `EMBED_DIM` | `ollama` / repository default **(default)** |
| `GEN_PROVIDER` / chat model | `ollama` / operator-supplied **(default)** |
| `OBSERVABILITY_FORMAT` | `human` **(default)** |
| `RAG_CITATION_VALIDATION` | off **(default)** |
| `ARM` / `EVAL_DATASET` | repository corpus defaults; `ARM=all` for full sweep **(default)** |
| Corpus | `evals/retrieval.json` (fixed) |

### Pre-declared comparison rules (set before any Rust metric)

These are fixed now, before Rust produces any metric. A Rust metric is compared
against the baseline record using exactly these rules.

| Metric / field | Comparison | Rule |
| --- | --- | --- |
| Case id set | set | Must equal the `evals/retrieval.json` id set exactly |
| Retrieved chunk id order (per case) | order | Deterministic tie ordering: sort by score descending, then by chunk id ascending (stable), matching Go's fused order |
| Recall | float-tol | tolerance ±0.001 absolute |
| Precision | float-tol | tolerance ±0.001 absolute |
| MRR | float-tol | tolerance ±0.001 absolute |
| Hit/miss classification (per case) | exact | Boolean must match |
| Citation strings | exact | Byte equality after Go's documented repair step |
| Page numbers on chunks/citations | exact | 1-based integer equality |
| Retrieved result count | exact | Integer equality per case |
| Section/chunk content hash | exact | Hex digest equality |
| Thresholds (pass/fail) | — | Recall ≥ baseline − 0.001, Precision ≥ baseline − 0.001, MRR ≥ baseline − 0.001; no per-case hit regression allowed |

**Deterministic retrieval metrics must not regress against this baseline.** Any
regression is a failed parity check.

### Baseline metrics

Baseline metric values from `cmd/eval` over `evals/retrieval.json` at the frozen SHA
are **not captured in this environment** — see Isolated gaps. The comparison rules
and thresholds above are recorded now, so Rust comparison can proceed once the metric
capture is performed against the frozen corpus with the baseline configuration.

## 9. Fixture manifest (S5-fixture-freeze)

Frozen fixtures live under `docs/reports/rust-migration/fixtures/`. Every fixture
records the reference SHA, the capture command, the settings used, and its capture
outcome. Expected-output fields are labelled: `exact`, `float-tol`, or `order`.

| Fixture id | Type | Input(s) | Capture command | SHA | Settings | Outcome |
| --- | --- | --- | --- | --- | --- | --- |
| `en-retrieval` | English retrieval | `docs/reports/rust-migration/fixtures/en/input.md`, `.../en/question.txt` | `make ingest FILE=...` then `make ask Q=...` / `make eval` | `d72fb398e8959eea83ed24be03f6df8cdeca86a2` | §8 defaults | Isolated gap — see §10 |
| `vi-retrieval` | Vietnamese retrieval (bilingual) | `.../vi/input.md`, `.../vi/question.txt` | `ARM=A EVAL_DATASET=... go run ./cmd/expbilingual` | `d72fb398e8959eea83ed24be03f6df8cdeca86a2` | §8 defaults + `ARM=A` | Isolated gap — see §10 |
| `page-provenance` | Page provenance (text-layer PDF) | `.../page/text-layer.md` (PDF source), expected pages in `.../page/expected.json` | `make ingest FILE=...` then `make ask Q=...` | `d72fb398e8959eea83ed24be03f6df8cdeca86a2` | §8 defaults | Spec freeze — see §10 |
| `scanned-rejection` | Rejection (no text layer) | `.../page/scanned.md` (documented behaviour) | `make ingest FILE=...` | `d72fb398e8959eea83ed24be03f6df8cdeca86a2` | §8 defaults | Spec freeze — asserts explicit rejection, not extraction |

Fixture input/expectation files are committed alongside this manifest. Their
expected-output fields are labelled by comparison type in the per-fixture JSON, and
the labels are consistent with §8.

## 10. Isolated gaps

Each gap records the exact command attempted, the environment condition, and why
capture did not occur. None is counted as passed.

| Gap | Command attempted | Condition | Status |
| --- | --- | --- | --- |
| Integration suite | `make integration` | docker-compose PostgreSQL+pgvector service not confirmed available in this environment; the task's integration check is operator-environment dependent | BLOCKED — not counted as passed |
| Baseline `cmd/eval` metrics | `make eval` (`go run ./cmd/eval`) | requires the database and a reachable Ollama embedder; service availability not confirmed in this environment | BLOCKED — not counted as passed |
| `en-retrieval` fixture outputs | `make ingest` + `make ask` | requires database + Ollama embedder/generator | BLOCKED — expected outputs not captured |
| `vi-retrieval` fixture outputs | `ARM=A ... go run ./cmd/expbilingual` | requires database + Ollama embedder/generator and the bilingual dataset | BLOCKED — expected outputs not captured |
| Ollama model versions | — | model names/versions are operator-supplied and not verifiable from repository contents | UNVERIFIED — record actual versions when services are captured |

Deterministic text-processing and page-provenance expectations that need no external
service are frozen as specification fixtures (input + labelled expected fields) so
RUST-003 can compare against them immediately; service-dependent outputs are recorded
as gaps, never as captured.

## 11. Reproduction checklist

To re-run the baseline in an environment with Docker and Ollama available, from the
repository root at SHA `d72fb398e8959eea83ed24be03f6df8cdeca86a2`:

1. `make db-up` (starts docker-compose PostgreSQL+pgvector and applies `make db-schema`)
2. `make test` (expect PASS)
3. `make integration` (expect PASS; truncates the `documents` table)
4. `make eval` and record metrics using the §8 comparison rules
5. `make ingest FILE=docs/reports/rust-migration/fixtures/en/input.md` then capture
   `cmd/rag` output for the `en-retrieval` fixture
6. Run the bilingual arm for the `vi-retrieval` fixture
7. Record the actual Ollama model names/versions in §2
8. `make db-down`
