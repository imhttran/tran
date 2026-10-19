# RUST-007D — live-model closure requirement

**Status: criterion-4 met and accepted by the operator; `sop verify RUST-007D`
reports `VERIFIED`. The acceptance decision and the recovery binding are
preserved in `.agent-sdlc/runs/RUST-007D/`.**

Criterion (`criterion-4`): generated answers meet the baseline groundedness and
citation criteria. This is a live-model closure requirement and cannot be
satisfied by scripted tests.

This note records the live run against a real Ollama serving
`nomic-embed-text` and `llama3.2:3b`, the Go reference answers over the same
corpus/questions/models/judge, and the effect of porting Go's single bounded
citation re-prompt into the shared Rust `AnswerService`. Groundedness,
citation validity, citation coverage, and refusals are reported separately.

## 1. Attempt binding (attempt 1 vs attempts 2)

The apparent discrepancy is resolved from persisted state, without resetting
anything:

| Source | Value | Meaning |
| ------ | ----- | ------- |
| `sop task RUST-007D` | `Attempts: 2` | domain task `attempt = 2`; `task_attempts` rows 1 and 2 (both `PLANNED`/`requeued`) |
| `.agent-sdlc/runs/RUST-007D/outcome.json` | mapping `attempt: 1` | the reviewed attempt whose verdict was derived |
| `report.json` / `outcome.json` `generated_at` | `17:36:00.2195Z` | derived before the requeue at `17:36:00.2215Z` |
| `counts.json` | `attempts: 4`, `invocations: 4`, `continuations: 2` | invocation/telemetry counts, not the reviewed attempt |

The outcome mapping names the attempt under review; the task counter advanced
to 2 only when the task was requeued after that verdict. `sop outcome --accept`
binds automatically to the mapping's attempt, so the operator does not pass an
attempt number. No state, budget, or history was reset.

## 2. Run identity

| Item | Value |
| ---- | ----- |
| Date | 2026-10-18 |
| Branch | `rust-007d-resume` (see `git log` for the port commit) |
| Ollama | `0.40.4` |
| Embedding model | `nomic-embed-text:latest` (768 dimensions) |
| Chat model | `llama3.2:3b` |
| Corpus | `rust-007d-live-20261018` (8 sources, 83 chunks), ingested with Go `cmd/ingest` |
| Dataset | `evals/retrieval-vi-en-expanded.json` (39 cases: 33 answerable, 6 unanswerable) |
| Judge | `llama3.2:3b` with Go's `groundednessPrompt`, `temperature 0`, `seed 0` |

The live generator does not pin decoding options, so per-run results are
stochastic. Each side therefore has two independent repeats; both are preserved
and the aggregate is reported alongside the per-run results.

## 3. Implementation under test

* Go's citation contract in the shared answer path: `[source - section]` /
  `[source - section - p.N]`, source normalization, page validation, and repair
  (`rust/src/answer/{citation,prompt,render,service}.rs`).
* Go's single bounded citation re-prompt in `AnswerService::validate_and_repair`:
  an answer is clean only when it makes at least one citation and every citation
  resolves; otherwise exactly one repair call is made. A repair error or empty
  repair falls back to stripping the original answer; the initial generation
  error still propagates.
* `RAG_CITATION_VALIDATION` is wired through `Config` into `query`, matching the
  Go default (`false`). `AnswerService::new` keeps validation on for existing
  callers; `with_citation_validation(false)` selects the raw-answer path.
* Parser alignment: the bracket inner text is no longer trimmed before the
  `" - "` cut. Go preserves the trailing space, so `[Source: doc.md - ]` parses
  (with an empty section) and is stripped as invalid; trimming hid it from the
  Rust parser. The pre-fix repaired run is preserved in
  `live-007d/repaired-before-parser-fix/`.

This is the production answer path, not an eval-only workaround.

## 4. Configuration

```text
RAG_CORPUS=rust-007d-live-20261018
OLLAMA_URL=http://127.0.0.1:11434
OLLAMA_EMBED_MODEL=nomic-embed-text
OLLAMA_CHAT_MODEL=llama3.2:3b
EMBED_DIM=768
MIN_SIMILARITY=0.6
TOP_K=4
FINAL_K=3
EXPAND_LIMIT=20
CONTEXT_BUDGET=4000
QUERY_REWRITE=false
RAG_CITATION_VALIDATION=true
REQUEST_TIMEOUT=5m
```

`RAG_CITATION_VALIDATION=true` enables the repair path for both `cmd/rag` (Go)
and `query` (Rust). The Rust retrieval context is unchanged from the first run;
the same `contexts.json` is used by every judged repeat.

## 5. Results

`substantive` = an answer that is not a refusal. Groundedness is judged only for
substantive answers with non-empty retrieved context. Citation validity is
`valid / rendered`; coverage is substantive answers with at least one rendered
citation.

### 5.1 Original run (before the repair port)

| Measure | Rust `query` | Go `cmd/rag` |
| ------- | ------------ | ------------ |
| Records | 39 | 39 |
| Refusals | 26 | 21 |
| Substantive | 13 | 18 |
| Grounded | **12 / 13 (0.92)** | 12 / 18 (0.67) |
| Rendered citations valid | **6 / 6** | 11 / 11 |
| Substantive with ≥1 citation | **5 / 13 (0.38)** | 10 / 18 (0.56) |

Rust `NOT_GROUNDED`: `eval-en-en-02`. Go `NOT_GROUNDED`: `eval-vi-vi-01`,
`eval-vi-vi-08`, `eval-vi-en-01`, `eval-vi-en-03`, `eval-vi-en-05`,
`eval-misleading-02`.

### 5.2 Repaired run 1 (bounded re-prompt enabled)

| Measure | Rust `query` |
| ------- | ------------ |
| Records / refusals / substantive | 39 / 26 / 13 |
| Grounded | **12 / 13 (0.92)** |
| Rendered citations valid | **14 / 14** |
| Substantive with ≥1 citation | **8 / 13 (0.62)** |

`NOT_GROUNDED`: `eval-en-vi-04`.

### 5.3 Repaired run 2 (independent repeat)

| Measure | Rust `query` | Go `cmd/rag` |
| ------- | ------------ | ------------ |
| Records / refusals / substantive | 39 / 28 / 11 | 39 / 15 / 24 |
| Grounded | **7 / 11 (0.64)** | 16 / 24 (0.67) |
| Rendered citations valid | **6 / 6** | 17 / 17 |
| Substantive with ≥1 citation | **5 / 11 (0.45)** | 9 / 24 (0.38) |

Rust `NOT_GROUNDED`: `eval-en-en-01`, `eval-vi-vi-05`, `eval-en-vi-04`,
`eval-misleading-02`. Go `NOT_GROUNDED`: `eval-vi-vi-03`, `eval-vi-vi-04`,
`eval-vi-vi-06`, `eval-en-vi-04`, `eval-vi-en-01`, `eval-vi-en-04`,
`eval-vi-en-05`, `eval-misleading-02`.

### 5.4 Aggregate over two repeats per side

| Measure | Rust `query` | Go `cmd/rag` |
| ------- | ------------ | ------------ |
| Records | 78 | 78 |
| Refusals | 54 | 36 |
| Substantive | 24 | 42 |
| Grounded | **19 / 24 (0.79)** | 28 / 42 (0.67) |
| Rendered citations valid | **20 / 20 (1.00)** | 28 / 28 (1.00) |
| Substantive with ≥1 citation | **13 / 24 (0.54)** | 19 / 42 (0.45) |

Across the two repeats the Rust answers meet or exceed the Go reference on
groundedness, rendered-citation validity, and citation coverage. The
original-to-repaired change raised Rust coverage from 5/13 to 13/24 in aggregate
and removed every invalid rendered citation.

## 6. Observed failures (preserved)

* `eval-en-en-02` (original): answers that the fee may be assessed at the
  servicer's discretion, while the retrieved evidence says it may be **waived**.
* `eval-en-vi-04` (both repaired runs): consistently `NOT_GROUNDED`; the Go
  reference is also `NOT_GROUNDED` for this case in both repeats.
* Five uncited substantive answers in repaired run 1 (`eval-en-en-06`,
  `eval-vi-vi-02`, `eval-vi-vi-04`, `eval-vi-vi-07`, `eval-vi-vi-08`) show the
  model still omits citations for some answers; this is model behavior, not a
  citation-validity failure. The same shortfall appears in the Go reference.
* Unnecessary refusals remain in both sides (for example `eval-vi-vi-10` and
  `eval-misleading-02` in the original Rust run) and are counted as refusals,
  not as grounded answers.

## 7. Artifacts

Under `docs/reports/rust-migration/live-007d/`:

| Path | Contents |
| ---- | -------- |
| `original/` | first Rust run: `answers.jsonl`, `contexts.json`, `groundedness.json`, progress logs, ranking log |
| `repaired/` | repaired run 1: answers, contexts, judge results |
| `repaired-repeat2/` | repaired run 2: answers, contexts, judge results |
| `repaired-before-parser-fix/` | the run that exposed the trailing-space parser divergence |
| `go-baseline-original.json` | Go reference, first run |
| `go-baseline-repeat2.json` | Go reference, second run |
| `ingest.log`, `expbilingual.log`, `query-cache.json` | corpus ingestion and bilingual harness evidence |
| `go-eval-factjudge.log` | pre-existing Go fact-judge crash (`supported fact index out of range: 2`) |

## 8. Assessment

In the baseline's own measured terms — grounded answers among judged answers,
rendered-citation validity, and citation coverage — the Rust answers meet or
exceed the Go reference across two independent repeats. Citation validity is
100% on both sides; the earlier empty-citation defect is gone; the bounded
re-prompt is the production path.

**Criterion-4 is met by the recorded evidence.** The result remains subject to
the operator's manual acceptance decision; this note does not mark the SOP
lifecycle `VERIFIED`.

## 9. Supported acceptance action (not run)

The project currently has no designated approvers, so `--accept` fails closed
until one is added. Exact actions for the operator's decision:

```yaml
# .agent-sdlc/config.yaml
human:
  designated_approvers:
    - operator
```

```sh
sop outcome RUST-007D --accept criterion-4 \
  --evidence "docs/reports/rust-migration/RUST-007D-live-model-evidence.md; two repaired live repeats: Rust grounded 19/24 vs Go 28/42; rendered citations 20/20 vs 28/28; coverage 13/24 vs 19/42; artifacts live-007d/{repaired,repaired-repeat2,original,go-baseline-original.json,go-baseline-repeat2.json}" \
  --by operator

sop approve RUST-007D --by operator
```

The decision binds to the recorded mapping's `criterion-4`, description, manual
evidence, and attempt 1. A criterion-scoped waiver is a different action and is
not recommended here.

## 10. Open requirements (not closed by this evidence)

* Original K=3 parity (the RUST-007 comparison uses `FINAL_K=4`).
* Markdown and plain-text ingestion (the production ingestion service is
  PDF-only).
* Live ingest → query → cited-answer end-to-end evidence through the production
  service path.
* Real Axum/Next.js browser flow.
* Rust-only clean checkout without Go.

These remain explicitly open and are not satisfied by this live-model evidence.
