# PLAN-TRAN-FAM-RAG-02 — Reliable document processing workers

**Repository:** `https://github.com/imhttran/tran`
**Plan ID:** `plan-tran-fam-rag-02`
**Branch:** `fix/fam-rag-02-worker-reliability`
**Baseline:** `origin/main` at `1713c8c` (PRs #51, #53, #54 merged)
**Owner:** Linux workstream

## Purpose

Prevent healthy document-processing workers from losing their claims during long
embedding runs, and prevent stale attempts from completing or failing a newer
attempt. No change to document approval, family-profile, or RAG publication
policy.

## Root cause

1. `claimed_by` is a worker *name*. The CLI uses a fixed `cli-worker`, so two
   processes share it. After a reclaim the row still says `cli-worker`, and the
   earlier attempt passes the ownership checks in `publish_ready`/`fail_job`.
2. The lease is set once at claim time and never renewed. Embedding can exceed
   the lease, so another worker reclaims a healthy job and duplicates the work.
3. The `legacy_import` fast path marks the version and job `ready` without any
   claim-ownership or approval revalidation.

## Design

- **Fence.** Use the existing incrementing `attempts` value as the per-attempt
  fence. Every ownership check requires `job_id`, `status = 'processing'`,
  `claimed_by`, and `attempts`. A reclaim increments `attempts`, so an older
  attempt can never match.
- **Renewal.** Add `renew_lease`, which extends `lease_expires_at` only when the
  fence still matches. Add a `LeaseHeartbeat` that owns an **independent**
  database connection and renews every `lease_seconds / 3`, waking immediately
  on stop.
- **Lifecycle.** The heartbeat stops on success, error, cancellation, and claim
  loss, and is joined before the publication/failure transaction. A failed
  renewal marks the attempt lost and publication fails closed.
- **Validation.** Reject `lease_seconds < 3`.
- **Legacy path.** Reuse the shared approval revalidation and fenced
  `mark_job_ready`, without linking or superseding chunks.
- **No migration.** `attempts` and `lease_expires_at` already exist.

## Non-negotiable constraints

- Keep FAM-RAG-01 approved-only retrieval, citation, and revocation behavior
  intact; do not edit `src/answer`, retrieval reads in `src/storage_pg.rs`, or
  the publication view.
- Do not modify Mac-owned profile, branch-review, or Head of Family code.
- Use synthetic documents and disposable `*_test` databases only.
- Do not merge, deploy, or process real family documents.

## Task 1 — `rag-201-attempt-fence`

**Objective:** Make the claim fence unique per attempt so two attempts with the
same worker name cannot both own the same job.

**Deliverables:**
- `renew_lease`, `publish_ready`, `fail_job`, and the `legacy_import` path
  require `attempts = job.attempts` in addition to `job_id`, `status`, and
  `claimed_by`.
- A shared `mark_job_ready` helper with the fence.
- A shared approval revalidation helper used by both publication paths.
- Integration coverage that two `cli-worker` claims cannot impersonate one
  another.

**Acceptance criteria:**
- After a reclaim, the earlier attempt returns `stale` from publish and failure
  and changes no row.
- A healthy current attempt publishes exactly once.
- Cancelled, removed, rejected, `admin_only`, and superseded versions cannot be
  marked ready.

## Task 2 — `rag-202-lease-heartbeat`

**Objective:** Renew the lease while a healthy attempt is still ingesting or
embedding.

**Deliverables:**
- `LeaseConfig` validating `lease_seconds >= 3` and deriving a heartbeat
  interval shorter than the lease.
- `LeaseHeartbeat` with an independent connection, a stop channel, and `Drop`
  that joins the thread.
- `process_job` starts the heartbeat before ingestion, checks for ownership loss
  before publication, and stops the heartbeat on every return path.
- `ProcessingError::InvalidLease` for invalid configuration.
- `run_due_jobs`/`claim_next_job` validate the lease.

**Acceptance criteria:**
- A run longer than the original lease keeps its claim; a second worker gets no
  job.
- If renewal fails or ownership is lost, the attempt does not publish.
- No heartbeat thread outlives the job.
- An abandoned worker's lease still expires and becomes reclaimable.

## Task 3 — `rag-203-legacy-safety`

**Objective:** Make the legacy-import fast path respect current approval and
claim ownership.

**Deliverables:**
- The legacy branch uses shared approval revalidation and fenced
  `mark_job_ready`.
- No chunk linking or supersession is performed for legacy rows.

**Acceptance criteria:**
- A removed, restricted, rejected, or stale-fingerprint legacy version is never
  marked ready.
- A stale attempt on a legacy job returns `stale` and changes nothing.

## Task 4 — `rag-204-regression-matrix`

**Objective:** Add deterministic concurrency and crash-recovery tests.

**Deliverables:** `tests/worker_reliability.rs` and the existing
`tests/document_processing.rs` covering:
1. A healthy long-running worker renews and a second worker cannot reclaim
   (`worker_reliability`).
2. A crashed worker stops renewing and its job becomes reclaimable
   (`worker_reliability`).
3. An old attempt cannot publish after a reclaim (`worker_reliability`,
   `document_processing`).
4. An old attempt cannot fail a newer attempt (`document_processing`).
5. Two processes with the same worker name cannot impersonate one another
   (`worker_reliability`, `document_processing`).
6. `admin_only`, removed, rejected, and superseded versions cannot be published
   (`document_processing`).
7. The legacy fast path respects approval and ownership
   (`worker_reliability`).
8. Heartbeat shutdown on success and on error (`worker_reliability`).
9. A failed and restarted attempt preserves chunks, history, citations, and
   audit evidence (`document_processing`).

**Acceptance criteria:**
- Every case passes against a disposable database.
- No test relies on a live model or a real family record.
- FAM-RAG-01 tests still pass unchanged.

## Task 5 — `rag-205-verification-delivery`

**Objective:** Record the worker contract and deliver one reviewable PR.

**Deliverables:**
- `docs/guides/document-worker-reliability.md` describing the fence, heartbeat,
  legacy safety, configuration, failure behavior, and tests.
- Full gate run.
- One PR titled
  `fix(documents): renew worker leases and fence stale processing attempts`.

**Acceptance criteria:**
- All gates pass or the exact failure is disclosed.
- No migration and no Mac-owned file changed.
- The PR is open, not merged.
