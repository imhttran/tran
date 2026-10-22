# Document worker reliability

This guide records the claim fencing and lease renewal added for FAM-RAG-02.
It complements [`document-publication-integrity.md`](document-publication-integrity.md)
and the document eligibility predicate in
[`FAMILY-PUBLICATION-CONTRACT.md`](../architecture/FAMILY-PUBLICATION-CONTRACT.md).

## Problem

The worker lease was set once at claim time and never renewed. Embedding can run
longer than the lease, so a second worker reclaimed a healthy job and duplicated
the work. Ownership was checked only by `claimed_by`, which is a worker *name*
(`cli-worker` for every CLI process), so after a reclaim the earlier attempt
still matched and could publish or fail the row.

## Claim fence

A claim is identified by `(job_id, attempts)`. `claim_next_job` increments
`attempts` on every claim, so each attempt has a distinct number. Every write
that acts as the owner requires all of:

```
job_id = $1 AND status = 'processing' AND claimed_by = $2 AND attempts = $3
```

Enforced in:

| Operation | Fence |
| --- | --- |
| `renew_lease` | yes |
| `mark_job_ready` (publication) | yes |
| `fail_job` | yes |
| legacy `legacy_import` publication | via `mark_job_ready` |

A zero-row update returns `ClaimLost`. The fenced update is in the same
transaction as the version update, so a lost claim rolls the whole publication
back.

Two processes that configure the same `cli-worker` name can no longer
impersonate one another: only the attempt number that currently owns the row can
act.

## Lease renewal

`LeaseConfig` validates the configured lease and derives a heartbeat interval:

- `lease_seconds < 3` is rejected with `ProcessingError::InvalidLease`.
- heartbeat interval = `lease_seconds / 3` (at least one second), strictly
  shorter than the lease.

While a non-legacy job is ingesting or embedding, `LeaseHeartbeat` runs a thread
with an **independent** PostgreSQL connection (the worker's main connection is
blocked inside ingestion). The thread waits on a channel and, on each timeout,
calls the fenced `renew_lease`. Failure or a lost fence sets a shared `lost`
flag and exits.

Lifecycle:

- Started after the approval revalidation and before storage/ingest work.
- Stopped and joined before any terminal transaction, on success, error,
  rejection, staging failure, and claim loss.
- `Drop` always joins the thread, so no heartbeat outlives the job.
- If renewal is lost, `process_job` returns `stale` and does not publish or fail
  the newer attempt.
- If the independent connection cannot be opened, the attempt fails closed
  (`lease heartbeat unavailable`) rather than processing without renewal.

Crash recovery is unchanged: a worker that dies stops renewing, its lease
expires, and `claim_next_job` reclaims the job with a higher attempt number.

## Legacy-import safety

The `legacy_import` fast path previously marked the version and job ready with no
ownership or approval check. It now uses the same shared
`revalidate_approval` + fenced `mark_job_ready` as normal publication:

- a removed, rejected, restricted (`admin_only`), superseded, or
  fingerprint-mismatched legacy version is never marked ready;
- a stale attempt cannot mark a reclaimed legacy job ready;
- linking is a no-op because the migration already linked the preserved chunks.

## Configuration

The lease is passed by the caller:

- API-triggered runs: `300` seconds.
- CLI (`process-documents`): `300` seconds.

Both are validated. No migration is required; `attempts` and
`lease_expires_at` already exist.

## Failure behavior

| Situation | Result |
| --- | --- |
| Heartbeat renews | Normal processing continues |
| Renewal returns zero rows (reclaimed) | `stale`, no publication, no failure write |
| Renewal connection fails | `stale`, no publication |
| Approval/scope/removal changes mid-job | `StaleApproval` at publication; job failed if still owned |
| Worker crashes | Lease expires; job reclaimable with a new attempt |
| Heartbeat start fails | Job failed with `lease heartbeat unavailable` |

## Tests

`tests/worker_reliability.rs` (deterministic, disposable database, no live
model):

| Case | Test |
| --- | --- |
| Healthy long run renews; second worker cannot reclaim | `healthy_long_run_renews_and_blocks_reclaim` |
| Abandoned lease is reclaimable | `abandoned_lease_is_reclaimable` |
| Heartbeat stops after failure; no lease outlives the job | `heartbeat_stops_after_failure` |
| Legacy path respects approval and the attempt fence | `legacy_fast_path_respects_approval_and_fence` |

`tests/document_processing.rs::attempt_fence_blocks_stale_and_unapproved_publication`
covers same-name reclaim, stale publish, stale failure, renewal, and
cancelled/rejected/`admin_only`/superseded refusal.

```sh
cargo test --locked --test worker_reliability
cargo test --locked --test document_processing
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

The healthy-run test uses a gated embedder: ingestion blocks inside the first
embedding call until the test releases it, so the timing is coordinated rather
than guessed. The lease-expiry cases set the lease directly instead of sleeping.

## Known limitations

- **Clock-based lease.** Renewal and expiry use the database clock. A large clock
  skew between workers and the database could expire a lease early; the fenced
  attempt number still prevents double publication.
- **No lease CLI flag.** The 300-second lease is fixed by the callers. A future
  change can expose it as configuration once worker sizing is known.
- **Retry accounting.** A heartbeat-start failure consumes an attempt. With the
  default `max_attempts = 3`, a persistent connection problem leaves the job
  failed for an explicit admin retry rather than looping.
- **Abandoned partial chunks.** A crashed attempt may leave chunks under its
  version-specific source; they are unapproved and never retrievable. A retry
  replaces them.
