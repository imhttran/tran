# Documentation

This is the index for everything under `docs/`. The root [`README.md`](../README.md)
is the project landing page; start there for what the project is and how to run it.

## How the docs are organized

| Directory                                         | Owns                                                                             | Nature                            |
| ------------------------------------------------- | -------------------------------------------------------------------------------- | --------------------------------- |
| [`architecture/`](architecture/OVERVIEW.md)       | How the system is built and what its contracts are                               | Descriptive + normative contracts |
| [`operations/`](operations/platform-support.md)   | Operator-run procedures and operational contracts                                | Normative (procedures)            |
| [`reference/`](reference/commands.md)             | Commands, configuration, capabilities, crate, testing, experiments               | Descriptive (facts)               |
| [`guides/`](guides/usage.md)                      | Task-oriented how-tos for users, reviewers, and maintainers                      | Descriptive (procedures)          |
| [`plans/`](plans/PLAN-TRAN-USER-ONBOARDING.md)    | How work is sequenced, including SOP task plans and the deferred backlog         | Plans (recorded + active)         |
| [`history/`](history/lessons.md)                  | Completed experiment evidence, acceptance records, retrospectives, archived PRDs | Non-normative history             |
| [`reports/`](reports/rust-migration/README.md)    | Engineering evidence, reviews, preflight notes, and investigation reports        | Non-normative evidence            |
| [`mockups/`](mockups/family-experience/README.md) | Static UI mockups                                                                | Non-normative design input        |
| [`tran/`](tran/Tran-Family-English.md)            | The family source corpus and its audit findings                                  | Source data (operational)         |

The authority chain for a change is:

```text
architecture/ (contracts)  ->  operations/ (operator contracts)  ->  plans/  ->  execution
```

A contract constrains what an operator procedure may do, which a plan delivers, which
SOP executes. `history/`, `reports/`, and `mockups/` record what was measured,
decided, or drawn at a point in time; they never override the contracts, the
operator procedures, or the active plan.

The original RAG product requirements were archived with the rest of the historical
material: [`history/PRD.md`](history/PRD.md) and
[`history/PRD-Phase-23b.md`](history/PRD-Phase-23b.md). They are non-normative now.
The current family and genealogy scope is defined by the architecture contracts and
the `PLAN-TRAN-*` plans.

## Start here

- **Run the project** — [guides/usage.md](guides/usage.md): start the database, load
  `.env`, ingest a document, ask a question.
- **Understand the system** — [architecture/OVERVIEW.md](architecture/OVERVIEW.md)
  describes the ingest and query paths, package responsibilities, and boundaries.
  Command details are in [reference/commands.md](reference/commands.md).
- **Work on the family surfaces** — start with
  [guides/my-profile.md](guides/my-profile.md), [guides/review-console.md](guides/review-console.md),
  and [guides/branch-review.md](guides/branch-review.md).
- **Set up identity and operations** — [operations/RBAC-OPERATIONS.md](operations/RBAC-OPERATIONS.md)
  and [operations/platform-support.md](operations/platform-support.md).
- **Test anything** — [reference/testing.md](reference/testing.md) and
  [guides/testing.md](guides/testing.md).

## Architecture

| Document                                                                                   | Read it when                                                                                      |
| ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------- |
| [architecture/OVERVIEW.md](architecture/OVERVIEW.md)                                       | You want the pipeline, package responsibilities, and boundaries.                                  |
| [architecture/FAMILY-PUBLICATION-CONTRACT.md](architecture/FAMILY-PUBLICATION-CONTRACT.md) | You are deciding what family content may be presented or indexed, and how revocation must behave. |
| [architecture/GENEALOGY-SCHEMA-CONTRACT.md](architecture/GENEALOGY-SCHEMA-CONTRACT.md)     | You are reading or extending the genealogy entity and evidence schema.                            |
| [architecture/ONBOARDING-IDENTITY.md](architecture/ONBOARDING-IDENTITY.md)                 | You are tracing how an OIDC identity becomes a linked person.                                     |
| [architecture/cli-trust-boundary.md](architecture/cli-trust-boundary.md)                   | You need the local CLI trust boundary.                                                            |

## Operations

| Document                                                               | Read it when                                                          |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------- |
| [operations/RBAC-OPERATIONS.md](operations/RBAC-OPERATIONS.md)         | You are setting up identity, roles, migrations, backup, or recovery.  |
| [operations/HEAD-OF-FAMILY.md](operations/HEAD-OF-FAMILY.md)           | You need the Head of Family eligibility evidence and succession rule. |
| [operations/TRAN-003-ACCEPTANCE.md](operations/TRAN-003-ACCEPTANCE.md) | You are verifying the RBAC acceptance matrix.                         |
| [operations/embedding-dimension.md](operations/embedding-dimension.md) | You are changing `EMBED_DIM` (operator-run procedure).                |
| [operations/embedding-models.md](operations/embedding-models.md)       | You are comparing embedding models on the evaluation harness.         |
| [operations/page-provenance.md](operations/page-provenance.md)         | You need the page-provenance contract.                                |
| [operations/prompt-injection.md](operations/prompt-injection.md)       | You need the prompt-injection risk model and mitigations.             |
| [operations/defect-recovery.md](operations/defect-recovery.md)         | You are recovering from a suspected SOP defect during a RAG task.     |
| [operations/platform-support.md](operations/platform-support.md)       | You need supported platforms, prerequisites, and known gaps.          |

## Reference

| Document                                                                           | Read it when                                                      |
| ---------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| [reference/commands.md](reference/commands.md)                                     | You want the command and `make`-target list.                      |
| [reference/configuration.md](reference/configuration.md)                           | You need to know what an environment setting does or its default. |
| [reference/capabilities.md](reference/capabilities.md)                             | You want the shipped capabilities and where they live.            |
| [reference/rust-crate.md](reference/rust-crate.md)                                 | You want the crate's module and API reference.                    |
| [reference/project-structure.md](reference/project-structure.md)                   | You want the package and directory layout.                        |
| [reference/testing.md](reference/testing.md)                                       | You want to run or extend the unit and integration tests.         |
| [reference/experiments.md](reference/experiments.md)                               | You want the decisions driven by the recorded experiment results. |
| [reference/rust-transition-evidence.md](reference/rust-transition-evidence.md)     | You need the frozen Go→Rust transition record.                    |
| [reference/rust-008a-browser-evidence.md](reference/rust-008a-browser-evidence.md) | You need the browser integration evidence.                        |

## Guides

| Document                                                                             | Read it when                                                                          |
| ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------- |
| [guides/usage.md](guides/usage.md)                                                   | You want to set up, ingest a document, or ask a question.                             |
| [guides/testing.md](guides/testing.md)                                               | You need the test-database isolation model and how the suites are run.                |
| [guides/evaluation.md](guides/evaluation.md)                                         | You want to score retrieval or compare settings/models.                               |
| [guides/my-profile.md](guides/my-profile.md)                                         | You are building or reviewing the member My Profile surface and its privacy boundary. |
| [guides/review-console.md](guides/review-console.md)                                 | You are building or reviewing the administrator review queue and media review.        |
| [guides/branch-review.md](guides/branch-review.md)                                   | You are granting or auditing branch reviewer authority.                               |
| [guides/branch-review-acceptance.md](guides/branch-review-acceptance.md)             | You need the branch-review acceptance matrix.                                         |
| [guides/family-profile-presentation.md](guides/family-profile-presentation.md)       | You are calling or reviewing the read-only family profile endpoint and page.          |
| [guides/family-publication-acceptance.md](guides/family-publication-acceptance.md)   | You need the negative-by-default publication matrix and the open policy gates.        |
| [guides/document-publication-integrity.md](guides/document-publication-integrity.md) | You are reviewing the approved-document retrieval boundary.                           |
| [guides/document-worker-reliability.md](guides/document-worker-reliability.md)       | You are reviewing processing-worker leases and fencing.                               |
| [guides/agent-scripts.md](guides/agent-scripts.md)                                   | You are working with the git hooks or the agent helper scripts.                       |

## Plans

Active and recorded plans. SOP matches a plan by the path recorded in
`.agent-sdlc/plan.meta.json`; see the exceptions section below.

| Document                                                                                                 | Read it when                                                                   |
| -------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| [plans/PLAN-TRAN-TEST-DB-ISOLATION.md](plans/PLAN-TRAN-TEST-DB-ISOLATION.md)                             | You are running the **active** SOP plan for genealogy test-database isolation. |
| [plans/PLAN-TRAN-USER-ONBOARDING.md](plans/PLAN-TRAN-USER-ONBOARDING.md)                                 | You want the living-members, profiles, and branch-review plan.                 |
| [plans/PLAN-TRAN-USER-ONBOARDING-SOP.md](plans/PLAN-TRAN-USER-ONBOARDING-SOP.md)                         | You want the SOP execution decomposition of that plan.                         |
| [plans/PLAN-TRAN-BRANCH-REVIEW.md](plans/PLAN-TRAN-BRANCH-REVIEW.md)                                     | You want the branch-scoped verification plan.                                  |
| [plans/PLAN-TRAN-BULK-ONBOARDING.md](plans/PLAN-TRAN-BULK-ONBOARDING.md)                                 | You want the reviewed batch-preparation plan.                                  |
| [plans/BULK-001-dependency-matrix.md](plans/BULK-001-dependency-matrix.md)                               | You need the bulk-onboarding dependency and privacy baseline.                  |
| [plans/BULK-ACCEPTANCE-EVIDENCE.md](plans/BULK-ACCEPTANCE-EVIDENCE.md)                                   | You need the BULK-001…005 acceptance evidence.                                 |
| [plans/PLAN-TRAN-RBAC.md](plans/PLAN-TRAN-RBAC.md)                                                       | You want the private genealogy access and Head of Family plan.                 |
| [plans/PLAN-TRAN-RBAC-INTEGRATION-FOLLOWUP.md](plans/PLAN-TRAN-RBAC-INTEGRATION-FOLLOWUP.md)             | You want the TRAN-003 post-merge completion record.                            |
| [plans/PLAN-TRAN-USR-004E-REVIEW-CONSOLE.md](plans/PLAN-TRAN-USR-004E-REVIEW-CONSOLE.md)                 | You want the contribution review-console plan.                                 |
| [plans/PLAN-TRAN-FAMILY-TREE.md](plans/PLAN-TRAN-FAMILY-TREE.md)                                         | You want the family-archive experience plan.                                   |
| [plans/PLAN-TRAN-FAM-DOCUMENT-LIFECYCLE.md](plans/PLAN-TRAN-FAM-DOCUMENT-LIFECYCLE.md)                   | You want the family document versioning and review plan.                       |
| [plans/PLAN-TRAN-FAM-DOCUMENT-SECURITY-HARDENING.md](plans/PLAN-TRAN-FAM-DOCUMENT-SECURITY-HARDENING.md) | You want the post-merge document security hardening record.                    |
| [plans/PLAN-TRAN-FAM-RAG-01.md](plans/PLAN-TRAN-FAM-RAG-01.md)                                           | You want the approved-retrieval, citation-provenance, and revocation plan.     |
| [plans/PLAN-TRAN-FAM-RAG-02.md](plans/PLAN-TRAN-FAM-RAG-02.md)                                           | You want the reliable document-processing-worker plan.                         |
| [plans/TRAN-001-document-inventory.md](plans/TRAN-001-document-inventory.md)                             | You want the genealogy document-inventory plan.                                |
| [plans/TRAN-002-genealogy-data-model.md](plans/TRAN-002-genealogy-data-model.md)                         | You want the genealogy data-model and evidence-extraction plan.                |
| [plans/USR-001-preflight.md](plans/USR-001-preflight.md)                                                 | You need the identity-security prerequisite note.                              |

## History (non-normative)

| Document                                                                   | Read it when                                                                                           |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| [history/lessons.md](history/lessons.md)                                   | You want the lessons-learned retrospective.                                                            |
| [history/PRD.md](history/PRD.md)                                           | You need the archived original product requirements.                                                   |
| [history/PRD-Phase-23b.md](history/PRD-Phase-23b.md)                       | You need the archived PDF/OCR/multilingual phase requirements.                                         |
| [history/rust-editor-verification.md](history/rust-editor-verification.md) | You need the RUST-002 editor verification record.                                                      |
| [history/evidence/](history/evidence/)                                     | You need a completed acceptance or recovery evidence record (RAG-025A/B/E, RQ-008, RAG-024, TRAN-002). |
| [history/experiments/](history/experiments/)                               | You need a raw RQ-001…RQ-006 experiment capture, matrix, or decision record.                           |

Raw experiment captures are preserved as measurements, not current guidance; read
[reference/experiments.md](reference/experiments.md) for the decisions they drove.

## Reports and mockups (non-normative)

| Document                                                                                         | Read it when                                                                                                                                                                              |
| ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [reports/genealogy-test-db-isolation.md](reports/genealogy-test-db-isolation.md)                 | You need the root-cause investigation for the genealogy test-database interference.                                                                                                       |
| [reports/rbac/phase-0-baseline.md](reports/rbac/phase-0-baseline.md)                             | You need the RBAC phase-0 preflight and baseline.                                                                                                                                         |
| [reports/rust-best-practices-review.md](reports/rust-best-practices-review.md)                   | You want the crate's best-practices review.                                                                                                                                               |
| [reports/family-experience/FAM-001-preflight.md](reports/family-experience/FAM-001-preflight.md) | You need the family-archive reconciliation preflight.                                                                                                                                     |
| [reports/rust-migration/](reports/rust-migration/)                                               | You need the Go→Rust migration evidence set (baseline, parity, bilingual contract, live evidence, final acceptance, database setup, dependency decisions, SOP process, UI API contracts). |
| [mockups/family-experience/README.md](mockups/family-experience/README.md)                       | You want the family-archive UI mockup.                                                                                                                                                    |

## Source corpus (operational)

| Path           | Read it when                                                                                                              |
| -------------- | ------------------------------------------------------------------------------------------------------------------------- |
| [tran/](tran/) | You need the family source text edition (English and Vietnamese), the people catalog, or the TRAN-001/002 audit findings. |

`docs/tran` is the default `SOURCE_ROOT` for the `inventory` and `pdf_assessment`
binaries. Do not move or rename it.

## Where a new document belongs

- How the system is built, or a rule other documents must obey → `architecture/`.
- An operator-run procedure or contract → `operations/`.
- A command, setting, capability, or interface fact → `reference/`.
- A how-to for a user task → `guides/`.
- An implementation plan or deferred backlog → `plans/`.
- A completed evidence record, experiment capture, or retrospective → `history/`.
- An engineering review, preflight note, or investigation report → `reports/`.
- A static UI mockup → `mockups/`.

Do not add evidence at the root of `docs/`.

## Paths kept in place (operational exceptions)

These paths are deliberately **not** reorganized, because something other than a
reader depends on the exact location:

- [`tran/`](tran/) — the default `SOURCE_ROOT` in `src/bin/inventory.rs` and
  `src/bin/pdf_assessment.rs`; `docs/tran/audit/` is also a generated-output prefix in
  `src/inventory.rs`.
- [`rq-006-query-cache.json`](rq-006-query-cache.json) — the default query-cache path in
  `src/bilingual.rs`.
- [`plans/PLAN-TRAN-TEST-DB-ISOLATION.md`](plans/PLAN-TRAN-TEST-DB-ISOLATION.md) — the
  plan path recorded in `.agent-sdlc/plan.meta.json`. Moving it would break
  `sop run`/`resume`/`reconcile`; SOP matches a plan by its recorded source path.
- [`experiments-eval-sweep.md`](experiments-eval-sweep.md) — the documented
  `SWEEP_RESULTS` output path referenced by
  [operations/embedding-models.md](operations/embedding-models.md) and
  [guides/evaluation.md](guides/evaluation.md). Kept at the same path so recorded
  outputs and their references stay valid.

`plans/` keeps the recorded SOP task plans in place because a task plan may be invoked
by path. Completed evidence and retrospectives live under `history/`.

## Not authoritative

Checkouts under `.claude/worktrees/` and `.worktrees/` are working copies with their own
`docs/` trees. They are not documentation sources; `main` is.
