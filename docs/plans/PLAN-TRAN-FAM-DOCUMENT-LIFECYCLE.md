# PLAN-TRAN-FAM-DOCUMENT-LIFECYCLE — Family document versions, review, and processing

## Project

`plan-tran-fam-document-lifecycle`

## Summary

Implement the Family Document Lifecycle workstream from
`docs/plans/PLAN-TRAN-FAMILY-TREE.md` (FAM-002–FAM-004) in three separately
reviewable stages. All document metadata lives in the RAG database
(`DATABASE_URL`); genealogy data remains in `GENEALOGY_DATABASE_URL`. Additive
migrations only: existing `documents` rows are preserved, quarantined by default,
and never deleted or implicitly approved.

The approved retrieval path (`/api/query`, `/api/ask`, citations, and section
expansion) must read only versions that are explicitly approved, processed,
scope-authorized, and not removed. Browser and API uploads enter pending review;
the trusted local CLI operator workflow remains a separate, explicit publication
boundary. Do not modify branch-reviewer grants or `/branch-review` functionality.

## FAM-002 — Document contracts, versions, and legacy quarantine

Define the version, provenance, privacy, review, and processing contracts and
persist them additively. Existing `documents` chunks are never deleted or
rewritten. A migration quarantines one `legacy_import` version per existing
`(corpus, source)` group as `pending_review` / `legacy_indexed`, with no invented
owner or scope. New submissions are stored privately by content fingerprint and
never enter the approved retrieval path before explicit review and processing.

### Deliverables

- `migrations/009_document_lifecycle.sql` with additive `document_version`,
  `document_processing_job`, `document_audit_event`, and
  `documents.document_version_id`; append-only audit enforcement; indexes.
- `src/document_lifecycle.rs` with typed origin, privacy scope, review status,
  processing status, version records, private content-addressed storage,
  submission creation, review, removal, and legacy inventory.
- Approved-only retrieval filtering in `src/storage_pg.rs`.
- `DOCUMENT_ROOT` configuration and `.env.example` documentation.
- Migration tests: clean install, upgrade from synthetic legacy rows, no row
  deletion or content changes, no implicit approval, quarantine regression.

### Acceptance Criteria

- Existing `documents` row counts, content, hashes, corpus/source identities, and
  citations are unchanged by the migration.
- Every legacy group gets exactly one `legacy_import` version with
  `pending_review`, `legacy_indexed`, no approved scope, and no owner guess.
- A migration or backfill alone never marks a version `approved` or `ready`.
- Pending, rejected, removed, and unprocessed versions are excluded from the
  approved retrieval path.
- Private originals are stored outside any static-served tree and are never
  exposed by metadata responses.
- Branch reviewer grants and `/branch-review` routes are untouched.

### Execution

Write the additive migration, library, retrieval filter, and synthetic migration
tests. Verify with `cargo fmt --check`, `cargo clippy --locked --all-targets --
-D warnings`, and the document lifecycle integration tests against disposable
PostgreSQL databases.

## FAM-003 — General submissions and administrator review

Add editor/admin family-document submissions and an administrator review
workflow. A browser or API upload creates a pending version and does not write
chunks or embeddings. Editors see their own submissions; admins see the review
queue. Approval binds the document, content fingerprint, version, and access
scope. Rejection and removal keep the version outside retrieval.

### Dependencies

- FAM-002

### Deliverables

- Typed admin/editor routes for submit, list, read, preview content, review
  approve/reject, scope change, remove, and status.
- `POST /api/documents/ingest` no longer publishes directly; it creates a pending
  submission through the same lifecycle service.
- Trusted local CLI ingest creates an explicit `operator_import` approved version
  and links its chunks, preserving the separate operator boundary.
- A minimal `/documents` UI for submission and admin review/processing, reusing
  the existing application shell and authenticated proxy.
- Tests for viewer/editor/admin authorization, CSRF-protected mutations, pending
  exclusion, approval binding, rejection, removal, and direct-ingest bypass.

### Acceptance Criteria

- A viewer cannot upload or review; an editor cannot approve, reject, process, or
  choose scope; an admin can.
- A browser/API submission writes no `documents` chunks until approval and
  successful processing.
- Rejected, pending, removed, and superseded versions never appear in
  `/api/query`, `/api/ask`, citations, or section expansion.
- Approval is invalidated by a content or scope change and cannot be reused for
  a different version.
- No route returns a private original to an unauthorized caller.
- The trusted local CLI workflow still publishes explicitly and is documented as
  separate from browser/API approval.

### Execution

Add the lifecycle routes and UI, then run focused Rust API tests and the
Playwright document flow against disposable PostgreSQL and synthetic documents.

## FAM-004 — Durable processing and approved-only publication

Implement a durable database job runner over the existing ingestion services.
Approval alone is not readiness: a queued version becomes retrievable only after
processing commits its chunks and the version is marked ready. Jobs are
idempotent, retryable, and fail closed on stale approval, version, fingerprint,
scope, or removal checks.

### Dependencies

- FAM-002
- FAM-003

### Deliverables

- `src/document_processing.rs` with enqueue, claim (`FOR UPDATE SKIP LOCKED`),
  process, retry, failure recording, and stale-work cancellation.
- Processing uses the existing `IngestService`/pipeline and private original
  storage; legacy promotion adopts preserved chunks without re-embedding.
- A durable worker entry point plus an admin-triggered due-job run for tests.
- Audit records for queued, started, ready, failed, retried, and removed.
- Tests for idempotency, concurrent claims, retry, processing failure, stale
  approval/removal, legacy dry-run/apply, and approved-only retrieval.

### Acceptance Criteria

- Processing the same version twice yields one logical ready version and one set
  of chunks.
- Concurrent workers cannot publish the same job twice.
- A failure records a safe error and leaves the version outside retrieval; an
  admin retry can succeed.
- A stale approval, changed fingerprint, changed scope, or removed version
  cancels publication.
- A ready version is returned by the approved path with resolvable source/page
  citations; failed, pending, rejected, and removed versions are not.
- Legacy inventory is read-only; legacy promotion is explicit, audited, and
  never deletes or re-embeds existing chunks.

### Execution

Implement the durable job runner, wire it to the review/process API, update the
documents UI and browser acceptance flow, and run the repository Rust, migration,
and Next.js gates with synthetic documents only.
