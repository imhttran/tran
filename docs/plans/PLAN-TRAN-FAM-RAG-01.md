# PLAN-TRAN-FAM-RAG-01 — Approved retrieval, citation provenance, revocation integrity

**Repository:** `https://github.com/imhttran/tran`
**Plan ID:** `plan-tran-fam-rag-01`
**Branch:** `fix/fam-rag-publication-integrity`
**Baseline:** `origin/main` at `d3efab5` (PR #51 merged)
**Owner:** Linux workstream

## Purpose

Make the shared query path retrieve and cite only the exact approved, current,
authorized document version, and make revocation and supersession effective on
the next authorized read or answer.

This is a correctness and security slice over the existing architecture. It adds
no retrieval store, no second pipeline, no profile/story indexing, and no
database migration.

## Findings

| ID | Finding | Severity |
| --- | --- | --- |
| F1 | `ContextPassage` keeps only `source/section/page/content`; chunk id, version id, corpus, and fingerprint are dropped before citation matching. | High |
| F2 | `citation::matches` normalizes lifecycle suffixes and takes the first match, so two same-named documents can be confused. | High |
| F3 | No final publication revalidation: retrieval snapshots `approved_documents`, then generation runs; a revocation committing mid-generation still yields revoked text and sources. | High |
| F4 | The answer and API sources expose no corpus/version/fingerprint provenance. | Medium |
| F5 | Ambiguous model citations are attributed to whichever passage appears first instead of failing closed. | High |

## Design

1. A new read, `PgStore::retrieval_snapshot(corpus)`, selects the provenance
   fields from the existing `approved_documents` view. It is the only new read;
   the predicate is unchanged and no second store is added.
2. `Candidate` carries an optional `ChunkProvenance`; `retrieval_pg::fuse`
   fills it from the SQL row, the query service re-attaches it after section
   expansion, and `context_budget::budget` copies it into
   `ContextPassage.identity`.
3. `citation::resolve` matches the exact stored source first. It falls back to
   normalized filename matching only when exactly one passage matches. Zero or
   multiple matches are invalid and are stripped, never substituted.
4. `AnswerService` takes an optional `PublicationGuard`. After generation and
   repair it revalidates every budgeted passage in one statement against
   `approved_documents`; if any is no longer published the answer fails closed to
   the insufficient-evidence refusal. No transaction or lock spans the model
   call.
5. `Answer.citations` and the API `sources` add `corpus`,
   `document_version_id`, `chunk_id`, and `content_hash`; display fields are
   unchanged.

## Non-negotiable constraints

- Do not add a migration; existing tables already carry the needed provenance.
- Do not weaken or duplicate `approved_documents`, and do not create a second
  retrieval store or citation database.
- Keep operator and administrative inventory reads separate from family
  retrieval; raw operator access is not a publication path.
- Do not modify Mac-owned profile, branch-review, Head of Family, or genealogy
  presentation code, and avoid `src/api.rs` changes beyond the additive query
  response fields.
- Use synthetic documents and disposable `*_test` databases only. Never reset or
  migrate a database containing real family information.
- Do not merge, deploy, or process real family documents.

## Task 1 — `rag-101-retrieval-snapshot`

**Objective:** Prove the approved-only retrieval boundary and add a provenance
snapshot that every user-facing retrieval path can use.

**Deliverables:**
- `PgStore::retrieval_snapshot(corpus)` returning `PublicationRow` values
  (`chunk_id`, `document_version_id`, `corpus`, `source`, `section`, `page`,
  `chunk_index`, `content_hash`, `content`) from `approved_documents`.
- A written audit of every documented read path (`rows_for_corpus`,
  `chunks_for_corpus`, `vector_search`, `keyword_search`, `section_chunks`,
  `find_by_id`, `chunks_for_source`, `chunks_by_hash`) classifying it as
  family retrieval, ingestion, evaluation, or operator inventory.
- Tests proving the snapshot includes an approved ready family chunk and excludes
  pending, rejected, unprocessed, `admin_only`, removed, and superseded chunks.

**Acceptance criteria:**
- Every family retrieval read selects from `approved_documents` and filters on
  corpus.
- No read used by family retrieval selects from `documents` directly.
- The snapshot returns the same rows as the existing approved reads for the same
  corpus.

## Task 2 — `rag-102-context-provenance`

**Objective:** Carry the authoritative chunk and version identity from retrieval
through budgeting into the context passage.

**Deliverables:**
- `ChunkProvenance` on `Candidate`, populated from `StoredChunkRow` in
  `retrieval_pg::fuse`, preserved through dedup, and re-attached to expanded
  candidates by the query service.
- `PassageIdentity` on `ContextPassage` with `chunk_id`,
  `document_version_id`, `corpus`, `source`, and `content_hash`.
- Updated constructors and unit tests in the answer modules.

**Acceptance criteria:**
- A budgeted passage always carries the chunk id and version id observed at
  retrieval time when the retrieval came from PostgreSQL.
- Section-expanded passages carry the same identity as the corpus row, not the
  originating candidate.
- Existing answer unit tests and acceptance tests still pass.

## Task 3 — `rag-103-citation-identity`

**Objective:** Make citation resolution exact and unambiguous so two same-named
documents are never confused.

**Deliverables:**
- `citation::resolve` returning the unique matching passage index or `None`.
- Exact-source matching first, plus section and page; normalized filename matching
  only as a fallback when exactly one passage matches.
- `matches`, `is_valid`, `validity`, `repair`, `AnswerService::references`, and
  `AnswerService::cited_passages` all use `resolve`.
- Deduplication of cited passages by provenance identity rather than
  `(source, section, page)`.
- Unit tests: same filename with different keys, ambiguous citation rejected,
  exact keyed citation resolves, unkeyed human citation accepted only when
  unique, removed/unknown citation invalid.

**Acceptance criteria:**
- A citation never resolves to a passage with a different exact source when an
  exact match exists.
- A citation whose normalized form matches two or more distinct passages is
  invalid and is stripped.
- Repair never invents a source or substitutes a different document.

## Task 4 — `rag-104-revalidation`

**Objective:** Add a final, fail-closed publication check between generation and
the response.

**Deliverables:**
- `PublicationGuard` trait in the answer service.
- `PgStore::revalidate_publication(corpus, identities)` performing one statement
  against `approved_documents` and reporting which identities are still
  published.
- `AnswerService` refuses with the insufficient-evidence answer when any budgeted
  passage is no longer published.
- Documentation of the validation boundary, including that a response already
  transmitted cannot be recalled.

**Acceptance criteria:**
- A document removed, restricted, or superseded after retrieval but before the
  response produces a refusal with no answer text and no sources.
- The check does not hold a transaction or lock across the model call.
- When the guard confirms every identity, the answer is unchanged.

## Task 5 — `rag-105-answer-provenance`

**Objective:** Expose authoritative provenance in the answer and the `/api/query`
response without breaking the existing shape.

**Deliverables:**
- `Answer.citations` passages include identity fields.
- `/api/query` `sources` include `corpus`, `document_version_id`, `chunk_id`, and
  `content_hash` alongside `source`, `section`, `page`, and `text`.
- A contract test asserting the additive fields.

**Acceptance criteria:**
- Existing response fields are unchanged.
- A cited source carries a version id and fingerprint that match the approved
  database row.
- No private original content or storage key is exposed.

## Task 6 — `rag-106-regression-matrix`

**Objective:** Add the deterministic regression matrix required by FAM-RAG-01.

**Deliverables:**
- `tests/rag_publication.rs` covering RAG-01 through RAG-18 with synthetic
  documents and disposable databases.
- Cases assert the positive control before each negative assertion so exclusions
  are not vacuous.
- Vector, keyword, and section-expansion paths share the same publication
  restrictions.
- Cross-corpus isolation with identical filenames.
- Citation ambiguity, removed-version citation, current-version citation.
- Concurrent revocation between retrieval and response.
- Stale or reclaimed processing job cannot republish revoked content.
- A second query in the same process after restriction returns nothing.

**Acceptance criteria:**
- Every matrix row has an executable test and the suite passes.
- Tests read the real view and the real Rust retrieval path, not a mocked
  predicate.
- A missing configured database fails the test rather than skipping silently.

## Task 7 — `rag-107-documentation`

**Objective:** Record the implemented boundary and its evidence.

**Deliverables:**
- `docs/guides/document-publication-integrity.md` describing the predicate,
  citation identity and rendering, revocation and supersession, the race
  boundary, the test matrix with commands, and known limitations.
- Update matrix rows 13 and 16 in `docs/guides/family-publication-acceptance.md`
  with the new evidence, leaving Mac-owned profile rows untouched.
- Cross-reference `docs/architecture/FAMILY-PUBLICATION-CONTRACT.md`.

**Acceptance criteria:**
- The guide states what is guaranteed, what is not, and the exact commands.
- No PASS claim is made for an unexecuted test.

## Task 8 — `rag-108-delivery`

**Objective:** Verify the complete change and prepare one reviewable PR.

**Deliverables:**
- Full verification: `cargo fmt --check`, `cargo build --locked`,
  `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`,
  `make web-check`, `make web-e2e`, `sop validate`, `sop review`, and
  `git diff --check`.
- A diff review confirming no Mac-owned files and no migration.
- One PR against `main` titled
  `fix(rag): enforce exact citation provenance and revocation-safe retrieval`.

**Acceptance criteria:**
- All required gates pass or the exact failure is disclosed.
- The committed tree contains no generated screenshots or `tsconfig` churn.
- The PR is open, not merged, and ready for human review.
