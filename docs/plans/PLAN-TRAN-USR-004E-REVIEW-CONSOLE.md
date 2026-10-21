# PLAN-TRAN-USR-004E — Family Contribution Review Console

## Objective

Give authorized administrators a review queue and comparison console for
submitted profile text, personal stories, photographs, and personal documents.
Decisions reuse `onboarding_profile_revision` and
`onboarding_profile_review_event`. Approval updates only the existing
`onboarding_profile_value` pointer. No new review, media, or publication system
is introduced.

## Boundaries

- Administrator RBAC (`Operation::ProfileReview`) only. Head of Family and
  family-branch grants do not confer review authority in this slice.
- No branch-scoped review screen (`/branch-review`), no USR-005 grants.
- No automatic RAG indexing or genealogy publication. The approved-only
  publication contract is recorded for USR-006.
- No real invitations, notifications, deployments, or production data.

## Bounded tasks

### USR-004E-01 — Review contracts and authorization inventory

Inspect the existing revision, approved-value, review-event, media, RBAC, and UI
surfaces. Record that the decision transaction already enforces pending status,
self-review denial, owner-linked reviewer denial, notes for rejection/changes,
and stale version detection.

Acceptance: no duplicate lifecycle; the queue/detail/media design reuses the
existing tables and decision service.

### USR-004E-02 — Queue and detail read APIs

Add bounded, cursor-paginated `GET /api/admin/reviews` and
`GET /api/admin/reviews/{id}`. Include display name, field/media category,
submitted timestamp, status, current approved value or current media metadata,
proposed value/media metadata, requested visibility, revision/version id,
events, and earlier same-field revisions. Exclude drafts.

Acceptance: `Operation::ProfileReview` is required; filters cover pending,
approved, rejected, and needs-changes; pagination is stable; no storage key is
returned.

### USR-004E-03 — Restricted media review access

Add `GET /api/admin/reviews/media/{id}` for pending media only. Deny the
submitter and the linked owner. Reuse the existing storage reader and response
headers.

Acceptance: pending media is previewable for review; approved owner-private,
rejected, needs-changes, and post-decision media are not reachable through the
review route.

### USR-004E-04 — Admin review UI

Build `/admin/reviews` with filters, queue, current-versus-proposed comparison,
long-narrative formatting, authorized media preview/download, review notes,
confirmation, blocked-decision explanations, loading/error/empty states,
responsive layout, and keyboard operation.

Acceptance: only supported API actions render; the browser never supplies
reviewer identity or role.

### USR-004E-05 — Negative security and E2E acceptance

Add synthetic Rust tests for queue/detail, independent and concurrent decisions,
stale versions, owner-private preservation, review-media closure, and no drafts.
Add Playwright coverage for the queue, approval, rejection/changes notes,
self-approval denial, non-admin/deactivated denial, cross-account isolation,
photo/document review, concurrent decisions, mobile, and keyboard paths.

Acceptance: Rust checks, `make web-check`, disposable-database browser E2E,
`sop validate`, and `sop review` pass. No automatic merge or deployment.

## Verification

- Fresh disposable databases for browser E2E.
- `cargo fmt --check`, `cargo test --locked`, Clippy `-D warnings`.
- `make web-check`, `make web-e2e`.
- `sop validate`, `sop review`.
- GitHub CI on the focused PR.
