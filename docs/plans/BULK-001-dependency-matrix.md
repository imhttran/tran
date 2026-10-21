# BULK-001 — Dependency Matrix and Privacy-Safe Baseline

**Plan ID:** `plan-tran-bulk-onboarding` (distinct from the historical onboarding SOP execution)
**Stage:** preflight and reconciliation.
**Canonical requirements:** `docs/plans/PLAN-TRAN-USER-ONBOARDING.md`, `docs/plans/PLAN-TRAN-USER-ONBOARDING-SOP.md`, `docs/plans/PLAN-TRAN-BULK-ONBOARDING.md`

This note is public-repository safe. It records only role, contract and qualitative
presence statements. It contains no real names, emails, birth dates, candidate
counts, tokens or private source hashes.

## Preflight state

- The feature branch is based on the latest `origin/main` after fetching PRs #28–#31.
- Pre-existing changes in other worktrees are user-owned and were preserved; this
  stage neither reverted nor cleaned them.
- The historical onboarding SOP execution and
  `docs/plans/PLAN-TRAN-USER-ONBOARDING-SOP.md` remain preserved and unmodified.
- Migration numbers below come from the observed `genealogy/migrations/` inventory,
  not from assumption.

## USR-003D / PR #26 status

**Status: MERGED.** PR #26 was merged through the onboarding-closure line
(PR #28). No competing USR-003D branch remains open, so the bulk work reuses the
reviewed invite/link API instead of rebuilding it.

## Contract matrix

| Contract | Finding | Notes |
| --- | --- | --- |
| Preview | EXISTS | `POST /api/admin/family-members/bulk/preview` reuses `onboarding::candidate::preview_candidates` and `onboarding::dry_run::preview_dry_run`. It is read-only and returns opaque keys, classifications, reasons, proposed usernames, and send blockers. |
| Preparation | EXISTS | `POST /api/admin/family-members/bulk/prepare` revalidates eligibility, reserves usernames transactionally, and records durable batch items. It never activates an account or sends a message. |
| Recipient verification | EXISTS | `POST /api/admin/family-members/bulk/recipients` records an explicitly confirmed address. The server never derives an address from genealogy documents. |
| Send | EXISTS | `POST /api/admin/family-members/bulk/send` requires explicit `approve: true`, a configured adapter, and a verified recipient. It returns no token. |
| Status | EXISTS | `GET /api/admin/family-members/bulk/batches/{id}` reports preparation and delivery state, including invitation acceptance/expiry, without tokens or addresses. |
| Permission | EXISTS | Rust enforces authenticated application admin before any database read through `OnboardingDryRun`, `OnboardingBulkPrepare`, and `OnboardingBulkSend`. The Next.js proxy requires CSRF for mutations. |
| Data owner | EXISTS | `GENEALOGY_DATABASE_URL` owns person IDs, reviewed living assertions, evidence, username reservations, invitation lifecycle, person-account links, verified recipient contacts, and batch metadata. `DATABASE_URL` owns OIDC `(issuer,subject)` identity, users, roles, grants and identity audit. No cross-database FK; identity is resolved only through the reviewed linking flow. |
| Delivery / email configuration | PARTIAL | The repository implements a fail-closed `UnconfiguredAdapter` and a synthetic `FakeAdapter`. A production operator must supply and approve a real provider adapter. Real sends remain disabled by default. |

## Migration inventory (observed)

Identity/RAG migrations are unchanged. The genealogy inventory now reaches:

- `0007_onboarding.sql`
- `0008_onboarding_identity.sql`
- `0009_onboarding_link_audit.sql`
- `0010_onboarding_foreign_keys.sql`
- `0011_onboarding_foreign_key_reconciliation.sql`
- `0012_profile_revisions.sql`
- `0013_profile_revision_actor_scope.sql`
- `0014_profile_revision_draft_withdrawal.sql`
- `0018_onboarding_batch.sql`

`0018` is additive: it creates the batch, batch-item, batch-audit, and verified
recipient-contact tables, and adds a partial unique index for one prepared item
per person. It reuses the existing invitation, reservation, and link tables.

## Idempotency and concurrency contract

- `batch_idempotency_key` is derived from the actor, the confirmed count, and the
  sorted opaque person ids.
- `person_idempotency_key` is derived from the batch key and the person id.
- `onboarding_batch_item` has unique keys on the batch/person key and on
  `(batch_id, person_id)`; the item insert uses `ON CONFLICT DO NOTHING` so both
  constraints are safe under a concurrent duplicate submission.
- A partial unique index permits one `prepared` item per person across batches.
- `reserve` locks the person row before checking for an existing reservation, so
  concurrent reservations serialize and reuse the first row.

## Privacy-safe baseline

**Verified living private data exists: UNKNOWN.** Presence of verified living
candidates in private genealogy data was not confirmed or excluded during this
stage. No names and no real counts are recorded. All implementation and
acceptance use synthetic data only. No write was issued against any real
database; any inspection used read-only queries only.

## Outstanding approval gate

The operator approval gate for merge, deploy, and real sends remains OUTSTANDING.
No real accounts, emails or private records may change without explicit operator
approval. The `FakeAdapter` never performs network I/O.
