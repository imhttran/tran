# PLAN-TRAN-FAM-DOCUMENT-SECURITY-HARDENING — Post-merge hardening

**Branch:** `fix/fam-document-security-hardening`
**Integrated workstream:** PR #47 (FAM-002 through FAM-004)
**Scope:** Linux-owned document lifecycle security and reliability. No branch-review or Mac-owned work.

## Remediation checklist

### Authorization and privacy

- [x] Require a database corpus grant for `DocumentSubmit`, `DocumentRead`,
  `DocumentReview`, and `DocumentProcess`.
- [x] Scope list, detail, original-content, review, scope, removal, processing,
  legacy-inventory, and job operations to the configured corpus.
- [x] Bind editor ownership to `(issuer, subject)`, not the subject alone.
- [x] Reject approval that silently changes an `admin_only` document to `family`;
  the browser sends the row's existing scope.
- [x] Keep pending, rejected, removed, restricted (`admin_only`), and superseded
  versions outside `approved_documents` and all retrieval paths.
- [x] Add cross-corpus and cross-issuer negative tests.

### Data integrity

- [x] Use the full document key in the chunk source and enforce a unique
  `(corpus, source)` index.
- [x] Compose every chunk source with one shared helper for both the browser/API
  and trusted operator paths, so two source schemes can never collide.
- [x] Normalize a client-supplied document key to the generated slug alphabet,
  so a crafted value cannot forge a version suffix or another source shape.
- [x] Probe `(corpus, source)` while allocating a version, so a legacy or
  older-format row cannot turn the insert into a raw unique violation.
- [x] Serialize version-number allocation with a per-document advisory lock.
- [x] Register trusted CLI imports before ingestion and finish/fail them
  explicitly; a failed publication leaves the prior approved version intact.
- [x] Make the migration-009 legacy linkage a one-time `INSERT ... RETURNING`
  operation guarded by `NOT EXISTS`, so a rerun cannot attach post-migration
  chunks to an approved legacy version even without the unique index.
- [x] Preserve original documents, chunks, history, and citation provenance.

### Worker reliability

- [x] Claim processing jobs with a lease and reclaim abandoned `processing`
  rows after lease expiry.
- [x] Require the current leaseholder to publish; an old worker receives
  `claim_lost` and cannot mark the version ready once another worker reclaimed
  the job. A pre-lease row (`lease_expires_at IS NULL`) may still publish while
  its `claimed_by` holder is unchanged.
- [x] Invalidate and requeue the job when a scope change or rejection removes
  readiness; reapproval can queue the same job again.
- [x] Test concurrent workers, expired leases, retry, approval invalidation, and
  cancellation.

## Migration

- `migrations/010_document_security_hardening.sql` adds `owner_issuer`,
  `document_processing_job.lease_expires_at`, a unique `(corpus, source)` index,
  and a lease index. It is additive and preserves all rows.
- `migrations/009_document_lifecycle.sql` legacy linkage is now one-time-only.

## Verification

Run against disposable `*_test` databases rebuilt the way CI builds them
(all `migrations/*.sql` and `genealogy/migrations/*.sql` applied once).

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | PASS |
| `cargo build --locked` | PASS |
| `cargo test --locked` | PASS |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS |
| `make web-check` | PASS |
| `make web-e2e` | PASS — 106 passed |
| `sop review` | 5 findings, 0 blocking |

The browser suite runs against a freshly created disposable database. Two
earlier local-only failures were test-environment artifacts, not code:
`admin-accounts.spec.ts` was fixed on main by PR #49 before this branch was
rebased, and a stale `e2e-auth.py` process plus accumulated fixtures from
reused databases caused the later bulk-onboarding and private-marker failures.
A fresh database and freed ports produce a clean run.

### Recorded residual findings

- `finish_operator_import`, `publish_ready`, and the CLI ingest flow are not one
  atomic operation. A process that exits between begin and finish leaves an
  approved-but-not-ready version; the next import of the same content reuses
  that row instead of stranding it.
- The processing lease is not renewed mid-job, so a document that takes longer
  than the lease to embed can be reclaimed and reprocessed. Publication is
  authorized by `claimed_by`, so the original worker cannot publish after a
  reclaim; the work is redone rather than corrupted.
- `normalize_source` is a citation-matching heuristic. It removes lifecycle
  suffixes and applies identically to both sides of a comparison; two real
  documents whose base names collide would also collide without the suffix.
- The migration-009 quarantine keeps the raw legacy source instead of
  `compose_source` output, deliberately: it must preserve the pre-existing
  `documents.source` identity and provenance.
