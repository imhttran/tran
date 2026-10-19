# Documentation

This is the index for everything under `docs/`. The root
[`README.md`](../README.md) is the project landing page; start there for what the
project is and how to run it.

## How the docs are organized

| Directory | Owns | Nature |
| --- | --- | --- |
| [`requirements/`](requirements/PRD.md) | What we are building and why | Normative (scope) |
| [`architecture/`](architecture/OVERVIEW.md) | How the system is built (pipeline, packages, boundaries) | Descriptive (overview) |
| [`reference/`](reference/commands.md) | Commands, configuration, capabilities, structure, testing, experiments | Descriptive (facts) |
| [`guides/`](guides/usage.md) | Task-oriented how-tos | Descriptive (procedures) |
| [`operations/`](operations/embedding-dimension.md) | Operator-run procedures and contracts | Normative (procedures) |
| [`plans/`](plans/PLAN.md) | How work is sequenced, including recorded task plans and the deferred backlog | Plans (active + recorded) |
| [`history/`](history/lessons.md) | Completed experiment evidence, acceptance records, and retrospectives | Non-normative history |

[`history/lessons.md`](history/lessons.md) records the reasoning behind decisions
already made. It is non-normative; the requirements and reference documents remain
authoritative.

The authority chain for a change is:

```text
requirements/  ->  architecture/  ->  operations/ (contracts)  ->  plans/  ->  execution
```

A requirement constrains the architecture, which the operator contracts
implement, which a plan delivers. `history/` records what was measured or
completed; it never overrides the current requirements, contracts, or plan.

## Start here: setup, architecture, and smoke test

Four entry points cover the common tasks; follow them rather than duplicating
their content here.

- **Setup** — [guides/usage.md](guides/usage.md) walks through starting the
  database, loading `.env`, ingesting a document, and asking a question.
- **Architecture** — [architecture/OVERVIEW.md](architecture/OVERVIEW.md)
  describes the ingest and query paths, package responsibilities, boundaries, and
  the shipped-vs-proposed status. The command set is in
  [reference/commands.md](reference/commands.md).
- **Platform support** — [operations/platform-support.md](operations/platform-support.md)
  classifies macOS, Linux, and Windows (incl. WSL2) and lists prerequisites and gaps.
- **Smoke test** — [history/evidence/rag-025b-e2e-evidence.md](history/evidence/rag-025b-e2e-evidence.md)
  records an end-to-end run of the retrieval to answer loop and the evaluation
  harness; the procedures are in [guides/evaluation.md](guides/evaluation.md) and
  [reference/testing.md](reference/testing.md).

## Documents

| Document | Read it when |
| --- | --- |
| [requirements/PRD.md](requirements/PRD.md) | You need the canonical product requirements, goals, and non-goals. |
| [requirements/PRD-Phase-23b.md](requirements/PRD-Phase-23b.md) | You need the requirements for the proposed PDF/OCR/multilingual phase. |
| [architecture/OVERVIEW.md](architecture/OVERVIEW.md) | You want the pipeline, package responsibilities, and boundaries. |
| [reference/commands.md](reference/commands.md) | You want the command and `make`-target list. |
| [reference/configuration.md](reference/configuration.md) | You need to know what an environment setting does or its default. |
| [reference/capabilities.md](reference/capabilities.md) | You want the shipped gap-closure capabilities and where they live. |
| [reference/project-structure.md](reference/project-structure.md) | You want the package and directory layout. |
| [reference/testing.md](reference/testing.md) | You want to run or extend the unit and integration tests. |
| [reference/experiments.md](reference/experiments.md) | You want the decisions driven by the recorded experiment results. |
| [guides/usage.md](guides/usage.md) | You want to set up, ingest a document, or ask a question. |
| [guides/evaluation.md](guides/evaluation.md) | You want to score retrieval or compare settings/models. |
| [guides/agent-scripts.md](guides/agent-scripts.md) | You are working with the git hooks or the `.agents/scripts/` helpers. |
| [operations/embedding-dimension.md](operations/embedding-dimension.md) | You are changing `EMBED_DIM` (the operator-run procedure). |
| [operations/embedding-models.md](operations/embedding-models.md) | You are comparing embedding models on the evaluation harness. |
| [operations/page-provenance.md](operations/page-provenance.md) | You need the page-provenance contract. |
| [operations/prompt-injection.md](operations/prompt-injection.md) | You need the prompt-injection risk model and mitigations. |
| [operations/defect-recovery.md](operations/defect-recovery.md) | You are recovering from a suspected `agentic-sop` defect during a RAG task. |
| [operations/platform-support.md](operations/platform-support.md) | You need the supported platforms, prerequisites, and known gaps. |
| [plans/PLAN.md](plans/PLAN.md) | You want the engineering and learning plan. |
| [plans/PLAN-RAG-Gap-Closure.md](plans/PLAN-RAG-Gap-Closure.md) | You want the gap-closure governance and audit record. |
| [plans/PLAN-RAG-Phase-23b.md](plans/PLAN-RAG-Phase-23b.md) | You want the phase-23b execution plan. |
| [plans/PLAN-RAG-025-closure.md](plans/PLAN-RAG-025-closure.md) | You want the RAG-025 final acceptance and closure record. |
| [plans/BACKLOG-RAG-007.md](plans/BACKLOG-RAG-007.md) | You want the deferred production bilingual-retrieval backlog. |
| [history/lessons.md](history/lessons.md) | You want the lessons-learned retrospective (non-normative). |
| [history/experiments/rq-006-decision-record.md](history/experiments/rq-006-decision-record.md) | You want the RQ-006 bilingual-retrieval decision record. |
| [history/evidence/rq-008-final-acceptance.md](history/evidence/rq-008-final-acceptance.md) | You want the RAG-008 final acceptance record. |
| [history/evidence/rq-024-operator-recovery-evidence.md](history/evidence/rq-024-operator-recovery-evidence.md) | You want the RAG-024 operator-recovery evidence record. |
| [history/evidence/rag-025a-recovery-evidence.md](history/evidence/rag-025a-recovery-evidence.md) | You want the RAG-025A recovery evidence record. |
| [history/evidence/rag-025b-e2e-evidence.md](history/evidence/rag-025b-e2e-evidence.md) | You want the RAG-025B end-to-end (smoke-test) evidence record. |
| [history/evidence/rag-025e-recovery-evidence.md](history/evidence/rag-025e-recovery-evidence.md) | You want the RAG-025E recovery evidence record. |
| [experiments-eval-sweep.md](experiments-eval-sweep.md) | You want the recorded embedding-model sweep (`scripts/eval-model-sweep.sh` output). |

Raw RQ-001 through RQ-006 experiment captures, matrices, and diagnostic outputs
live under [`history/experiments/`](history/experiments/). They are preserved as
measurements, not current guidance; read
[`reference/experiments.md`](reference/experiments.md) for the decisions.

## Where a new document belongs

- A new requirement or product decision -> `requirements/`.
- A new description of how the system is built -> `architecture/`.
- A new command, setting, capability, or interface fact -> `reference/`.
- A new how-to for a user task -> `guides/`.
- A new operator-run procedure or contract -> `operations/`.
- A new implementation plan or deferred backlog -> `plans/`.
- A completed evidence record or experiment capture -> `history/evidence/` or
  `history/experiments/`.
- A retrospective of completed work -> `history/lessons.md`.

Do not add task evidence at the root of `docs/`; the root is reserved for the
index, the operational sweep record, and the bilingual query cache.

## Paths kept in place (operational exceptions)

These paths are deliberately **not** reorganized, because code or scripts depend
on the exact location:

- [`operations/embedding-dimension.md`](operations/embedding-dimension.md) — the
  path is hardcoded in `internal/ingestion/schema.go` (`migrationFile`) and named
  by `ingestion.CheckEmbeddingDim`'s error message, so moving it would change
  behavior.
- [`experiments-eval-sweep.md`](experiments-eval-sweep.md) — it is the default
  `SWEEP_RESULTS` output path in `scripts/eval-model-sweep.sh`.
- [`operations/prompt-injection.md`](operations/prompt-injection.md) — named by a
  code comment in `internal/rag/answer.go` (comment only; not behavior).
- `rq-006-query-cache.json` — the default `QUERY_CACHE` path in
  `cmd/expbilingual`, so it stays at the root of `docs/`.

`plans/` retains the recorded SOP-compatible `PLAN-RAG-*.md` task plans. They are
kept in place because task plans may be invoked by path; completed evidence and
retrospectives moved to `history/`.
