# PLAN — TRAN-002: Genealogy Data Model and Evidence-Backed Extraction

**Status:** PROPOSED — execute only after normal SOP authorization.
**Repository:** `imhttran/tran`
**Workspace:** `/Users/imhttran/agentic-workspace/projects/tran`
**Branch:** `sop/tran-002-genealogy-model` (create from current `origin/main`)
**Prerequisite:** TRAN-001 merged (PR #1, merge `f683c01579dc8df555df855e2a653c101936705c`).

## Project

Tran Family Tree RAG — Rust, PostgreSQL/pgvector, Ollama, and Next.js.

## Summary

Convert the existing private genealogy catalog into a structured, evidence-backed family graph. Keep every source assertion linked to its original page. Preserve uncertain identities and relationships as candidates until reviewed. Provide local ingestion, review, and query commands, with no new public API exposure.

The input catalog contains 263 _entries_, not necessarily 263 distinct verified people. TRAN-002 must measure and report that distinction. Reuse TRAN-001 findings: four local source documents, 56 PDF pages, Vietnamese and English transcription coverage, and nine PDF text-layer gaps. Do not redo completed OCR or translation without a verified gap.

## Boundaries and source authority

- **Source root (private):** `docs/tran/`, including `Family Genealogy.pdf`, `Tran-Family-Vietnamese.md`, `Tran-Family-English.md`, and `Tran-Family-People-Catalog.md`.
- **Private evidence:** `docs/tran/audit/`, including TRAN-001 manifest and reports. Do not commit this tree or private database exports.
- The PDF is the original reference. The Vietnamese transcription is a derivative. The English translation and people catalog are interpretation aids, **not independent proof** of relationships.
- For verified facts, require source evidence traceable to a PDF page or a separately identified primary record. An entry without traceable evidence stays `candidate` or `disputed`.
- Keep literal Vietnamese name spellings and aliases. Never infer birth order, gender, living status, branch membership, identity, or family relationships from surnames, missing dates, or model output.
- All new committed fixtures must be synthetic. No living-person names, actual family dates, private excerpts, private filenames beyond the already documented input names, or hashes in tracked fixtures and CI logs.
- Work in a new feature branch. Preserve existing history, Rust RAG behavior, frozen parity fixtures, and document data.
- **No RBAC or family-tree UI in this milestone.** Do not add unauthenticated Axum/Next.js routes that expose family records. Design an authorization seam for later work; the `Head of Family` genealogy designation must never confer technical administrator permissions.
- Use a separate, explicitly selected local genealogy database (`GENEALOGY_DATABASE_URL`) for real data; do not use or reset the existing RAG database. Block destructive or importing commands when the destination is unset, nonlocal, or equal to `DATABASE_URL`. Use only disposable databases for automated integration tests.

## Architecture contract

Prefer an additive Rust module (`src/genealogy/`), a thin local CLI (`src/bin/genealogy.rs`), and a dedicated migration directory (`genealogy/migrations/`) so the existing frozen `migrations/*.sql` files and RAG `documents` table remain unchanged. Reuse repository libraries and database patterns where appropriate. Do not introduce Neo4j, a new microservice, a new model provider, or a second RAG retrieval pipeline.

The model must distinguish _people_ from _claims about people_:

- `source_document`: private local source identity, SHA-256, language, and derivation lineage.
- `source_evidence`: source ID, 1-based PDF page (when known), locator/section, and optional private excerpt or offset.
- `person`: stable opaque person ID and review/identity state, separate from source ordinal.
- `person_name`: original form, normalized search form, language/script, and alias type.
- `family_branch`: stable branch identifier, explicit branch number/label, and evidence.
- `genealogy_assertion`: subject, predicate, optional object or literal value, evidence ID, assertion status (`candidate`, `verified`, `disputed`, `rejected`), and review metadata.
- `catalog_entry`: source-relative stable entry key, raw private text reference, parsing result, and linked candidate person/assertion IDs.

Relationships include parent-child, spouse/partner, and branch membership. Facts such as sex/gender, dates, and living status must be represented as evidence-backed assertions, not default guesses. Keep the schema small and normalized; document any implementation-specific table naming.

### Common acceptance and verification requirements

- Every stage lists deterministic test or inspection evidence; do not count a manual review as an automated pass.
- Keep tests independent of `docs/tran/`; a clean GitHub Actions checkout must pass using synthetic fixtures.
- Preserve the four source-file SHA-256 hashes before and after real-data processing.
- Capture private counts, unmatched entry IDs, ambiguous identities, and pending review in local audit files only.
- Run only affected checks during development; execute the complete Rust gate and relevant database integration tests at the final milestone.
- Do not mark an assertion `verified` merely because a parser, LLM, or translation produced it.

## TRAN-002A — Preflight, inventory hardening, and schema contract

Establish the preflight baseline, exclude generated audit output from the source inventory, and document the genealogy entity and evidence contract before any schema work.

### Deliverables

- Verify latest `origin/main` includes TRAN-001 and that the working branch is not `main`.
- Review the existing `src/inventory.rs`, `src/pdf_assessment.rs`, storage/migration conventions, TRAN-001 private reports, and source hashes.
- Fix recursive inventory so generated `docs/tran/audit/` outputs are excluded without hiding legitimate source files. Add a regression test for repeated inventory runs.
- Document entity identifiers, citation format, uncertainty states, identity-resolution rules, review transitions, and privacy boundaries under `docs/architecture/` or `docs/reference/`.
- Verify `.agent-sdlc/config.yaml` selects Cargo checks rather than Go checks; repair configuration through normal SOP procedure if needed.

### Acceptance Criteria

- `docs/tran/audit/` is never counted as source input on repeat runs; a synthetic nested file outside the audit folder is still counted.
- The source manifest continues to represent the four original input files; real source hashes are unchanged.
- The written schema contract covers all seven logical record categories and explicit assertion states.
- Branch and configured validation checks are verified before mutation.

### Verification

- Focused inventory unit/CLI tests pass using synthetic fixtures.
- `cargo fmt --check` and `git diff --check` pass.
- Record private preflight evidence in `docs/tran/audit/TRAN-002-preflight.md`.

## TRAN-002B — PostgreSQL genealogy schema

Add an additive PostgreSQL schema and persistence layer for the genealogy model without changing the existing RAG schema or data.

### Dependencies

- TRAN-002A

### Deliverables

- Add additive, versioned SQL in `genealogy/migrations/` for the documented model, constraints, indexes, and schema version tracking.
- Implement Rust persistence methods and a migration runner for the explicitly configured genealogy database.
- Enforce foreign keys, uniqueness for stable catalog keys, evidence references on verified assertions, and controlled review statuses.
- Prevent import or migration against the RAG database or an unapproved destination. Schema changes must not modify existing `migrations/*.sql` or `documents` data.

### Acceptance Criteria

- A clean disposable PostgreSQL database migrates successfully and a second migration is a no-op.
- Constraints reject dangling evidence, invalid statuses, and duplicate catalog keys.
- Failed imports roll back atomically; no partial family graph remains.
- Existing RAG migrations and retrieval behavior remain unchanged.

### Verification

- Add `genealogy_schema` and `genealogy_storage` tests with synthetic data and disposable database guards.
- Run the database integration tests against a dedicated test DB.

## TRAN-002C — Catalog parser and candidate extraction

Parse the private catalog into stable candidates and evidence-linked assertions without auto-merging identities or relationships.

### Dependencies

- TRAN-002A
- TRAN-002B

### Deliverables

- Parse the actual catalog structure observed in `Tran-Family-People-Catalog.md`; do not assume every entry has identical formatting.
- Produce stable catalog-entry keys and candidate person, name, branch, and relationship assertions.
- Resolve page references to available source evidence. Keep unresolved locators explicit; reject out-of-range page numbers (valid PDF pages are 1–56 for this input).
- Treat conflicting names, same-name people, and unclear parentage as distinct candidates until evidence supports a merge.
- Export a private dry-run extraction report with processed, parsed, ambiguous, rejected, and unmatched entry counts.

### Acceptance Criteria

- Every catalog entry is accounted for exactly once, whether parsed, ambiguous, or rejected with a reason.
- No automatic merge occurs for shared names; no unsourced relationship becomes verified.
- The parser preserves original Vietnamese spelling, aliases, and any known page references.
- A second parse of unchanged input yields the same stable catalog-entry keys and proposed assertions.

### Verification

- Add deterministic parser tests with synthetic Vietnamese and English examples, conflicts, missing page references, and duplicate names.
- Run a private catalog dry run; store its summary in `docs/tran/audit/TRAN-002-extraction.md`.

## TRAN-002D — Validation and human review workflow

Add explicit evidence-backed review transitions and validation rules that keep uncertain or contradictory claims unverified.

### Dependencies

- TRAN-002B
- TRAN-002C

### Deliverables

- Implement explicit assertion transitions (`candidate` → `verified` / `disputed` / `rejected`) with reviewer, timestamp, and evidence audit trail.
- Validate self-parenting, parent-child cycles, duplicate relations, incompatible branch claims, and contradictory vital-status assertions.
- Provide a local report of unresolved assertions and a safe review mechanism that requires a chosen assertion ID and supporting evidence; never mass-approve model proposals.
- Preserve prior review decisions across idempotent re-imports. Changed source evidence must reopen affected claims for review, not silently preserve `verified`.

### Acceptance Criteria

- Every verified relationship has valid source evidence and review metadata.
- Contradictory or cyclic relationships cannot be promoted silently to verified.
- Re-import does not erase reviewer history or auto-promote candidates.
- Unknown facts remain unknown and queries label disputed facts accordingly.

### Verification

- Add review-state, conflict, cycle, re-import, and stale-evidence tests using synthetic family graphs.

## TRAN-002E — Local ingestion and evidence-backed queries

Provide guarded local import and evidence-backed lookup commands that use only the genealogy database.

### Dependencies

- TRAN-002C
- TRAN-002D

### Deliverables

- Add local-only `genealogy` CLI operations for `import --dry-run`, guarded `import --apply`, `unresolved`, and person/relationship lookup. Keep command names stable and document usage.
- Use transactional upserts keyed by stable entry identity. The apply path requires explicit `GENEALOGY_DATABASE_URL` and cannot publish or modify raw documents.
- Return person and relationship details with assertion status and source/page citations. Do not present a candidate as a confirmed relative.
- Preserve the existing RAG API and citation behavior; document a future evidence-to-RAG integration seam, but do not expose a public genealogy query endpoint before RBAC is implemented.

### Acceptance Criteria

- Importing the same source twice creates no duplicate catalog entries, people, or assertions.
- Lookup of a verified synthetic relationship returns the correct evidence/page and review status.
- A query with insufficient evidence returns `unknown` / `unverified`, not a generated family connection.
- Existing `query`/`ingest` and Axum routes remain backward-compatible.

### Verification

- Add CLI and database integration tests with a disposable Postgres database and synthetic sources.
- Record local import/query coverage in `docs/tran/audit/TRAN-002-query-evidence.md`.

## TRAN-002F — Head of Family eligibility (genealogy domain only)

Implement a pure genealogy-domain Head of Family eligibility decision that cannot confer application permissions.

### Dependencies

- TRAN-002D
- TRAN-002E

### Deliverables

- Model the family-specific designation as a pure, testable domain decision separate from security roles.
- Eligibility requires verified third-branch membership, a verified male designation, verified male birth order identifying the first-born male of that branch, and independently verified living status.
- Never infer `living` from a missing death date. Do not assume the oldest recorded male is the first-born.
- If the first-born male is deceased, unknown, contested, or not demonstrably eligible, return **undetermined** and request an explicit succession-policy decision. Do not choose a successor automatically.
- Record the supporting assertion IDs and a reason code for every eligibility result; do not change RBAC permissions.

### Acceptance Criteria

- A synthetic fully verified eligible case returns `eligible` with supporting evidence IDs.
- Missing birth order, branch, gender, or living evidence returns `undetermined`.
- A deceased or contested first-born male does not cause automatic succession.
- The genealogy designation cannot grant or imply any application permission.

### Verification

- Add pure Rust unit tests for eligible, unknown, disputed, deceased, and potential-successor cases.

## TRAN-002G — Integration, privacy audit, and closure

Integrate the stages, run the full verification and privacy gates, and record final evidence and closure.

### Dependencies

- TRAN-002A
- TRAN-002B
- TRAN-002C
- TRAN-002D
- TRAN-002E
- TRAN-002F

### Deliverables

- Create `docs/tran/audit/TRAN-002-report.md` with real-data coverage, imported candidates, verified/disputed/unknown counts, citation gaps, source hash verification, unresolved items, and next steps.
- Create a public-safe technical acceptance summary under `docs/history/evidence/` containing only synthetic test evidence and nonidentifying metrics.
- Update the relevant architecture/command docs. Verify staged Git contents do not include anything under `docs/tran/`, private names/relationships, or local database exports.
- Preserve SOP task history and report actual deterministic checks separately from manual review.

### Acceptance Criteria

- All seven stages have recorded acceptance evidence; no result is asserted `PASS` solely from manual inspection.
- Pre/post-run SHA-256 hashes of the four source files match TRAN-001's private manifest.
- A fresh source inventory still lists exactly the four original inputs and never includes generated audit reports.
- All committed test fixtures are synthetic; Git privacy checks pass; no private data is pushed.
- Full Rust gates and relevant DB-backed integration tests pass; the preexisting RAG behavior remains intact.
- Final SOP gate is `PASS` with no unresolved required approval before committing/pushing a PR.

### Verification

Run from the repository root with required disposable database test services available:

```bash
cargo fmt --check
cargo build --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
make rust-check
git diff --check
git ls-files docs/tran/
git check-ignore -v docs/tran/
```

Also execute the new synthetic genealogy integration tests and the preexisting API-contract checks when their inputs are affected. Validate privacy against the **staged diff**, not only the working tree. Any database test that is not using an authorized disposable database must stop before a write.

## TRAN-002H — Birth and death dates (post-closure extension)

Extend the catalog parser and assertion model with explicitly stated birth and
death dates. This section records work done after TRAN-002G closed; it does not
reopen the earlier tasks.

### Deliverables

- Parse full dates, year-only dates, month-only dates, approximate dates, bounds,
  and explicit ranges from both catalog shapes.
- Store them as `birth_date` / `death_date` assertions with partial ISO values,
  the precision the source supports, and the source wording. Leave
  unclassifiable date text as `date_unresolved`, never as a birth or a death.
- Keep person columns (`birth_year`, `death_year`) untouched so eligibility and
  living status cannot change without a reviewer.
- Preserve conflicting claims on correction, and refuse to verify a date that
  contradicts an already-verified date of the same kind.
- Project verified dates through the existing authorized reads: the person
  endpoint, the tree lifespan, and the family profile page.

### Acceptance criteria

- Re-import is idempotent: no duplicate date assertions, no lost review history.
- A corrected or conflicting date never replaces a reviewed value.
- A missing or unresolved date stays unknown.
- No Head of Family or eligibility rule changes.

### Verification

See [guides/genealogy-dates.md](../guides/genealogy-dates.md).

## Dependency graph

```text
TRAN-002A -> TRAN-002B -> TRAN-002C -> TRAN-002D -> TRAN-002E -> TRAN-002F
        \___________________________________________________________/
                              TRAN-002G
```

## Stop conditions

Stop and request operator approval only for a policy decision or genuine blocker, including: an ambiguous succession rule, unverifiable source-to-page identity required for promotion, unsafe database selection, failure of a required gate, or a requested public-access change before authentication/RBAC exists. Continue independently on unaffected tasks when SOP permits. Never bypass an approval boundary.

## Completion / PR policy

After verified final SOP `PASS`, commit only code, schemas, synthetic tests, and public-safe documentation on the feature branch; push and open a PR. Do not auto-merge. Leave all real genealogy records, private reports, source documents, and database exports on the local Mac. Post the PR URL and validation evidence. Future milestones: TRAN-003 private family-tree UI + authenticated RBAC, and optional richer RAG-backed genealogy questions only after authorization boundaries are in place.
