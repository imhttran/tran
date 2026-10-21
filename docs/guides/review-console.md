# Review console

The administrator review console at `/admin/reviews` lets an authorized
administrator inspect submitted profile changes, personal stories, photographs,
and personal documents, then decide independently. It reuses the existing
profile revision lifecycle and review-event audit; there is no second review or
content store.

## Queue and detail

- `GET /api/admin/reviews?status=pending_review|approved|rejected|needs_changes|all`
  returns submitted revisions only. Drafts are never returned.
- `GET /api/admin/reviews/{revision_id}` returns the proposal, the current
  approved value or media metadata, the append-only review events, and earlier
  same-field revisions.
- `GET /api/admin/reviews/media/{media_id}` serves the **pending** photo or
  document attached to a review.
- `POST /api/profile/revisions/{revision_id}/review` remains the decision
  endpoint. It rechecks reviewer independence, revision status, reviewer notes,
  stale versions, and the approved-value pointer inside one transaction.

Queue pagination is bounded to 100 rows (default 20 in the UI) and ordered by
descending revision id. The last id is the cursor, so equal timestamps cannot
drop or duplicate rows.

## Independence and authorization

Every review route requires `Operation::ProfileReview` (administrator) and a
verified OIDC identity. The browser never sends a reviewer identity, role,
account id, or genealogy person id.

A decision is blocked when:

- the reviewer is the submitter (`self_submitted`);
- the reviewer owns the linked genealogy person (`reviewer_owns_person`);
- the revision is no longer pending (`not_pending`).

Rejection and requests for changes require a reviewer note. Approval updates the
last-approved value atomically; rejection and needs-changes leave it unchanged.
The queue exposes `can_decide` only as a presentation hint. Rust remains the
authority.

## Media boundary

- Pending review media is readable only through the review-media route by an
  administrator who neither submitted it nor owns the linked person.
- Approved `owner_private` media remains private.
- Approved `family` media is served only through the existing authorized
  profile-media route when it is the current approved asset.
- Rejected and needs-changes media is not served through the review route.
- Review media responses use `private, no-store`, `nosniff`, a sandbox CSP, and
  a safe content disposition. Storage keys, hashes, and raw bytes never appear
  in JSON or logs.

## Family presentation and USR-006

Approval updates `onboarding_profile_value` and the review-event history through
the existing service logic. It does not automatically publish anything.

- Approved + `owner_private`: owner-only.
- Approved + `family`: eligible for authorized family presentation.
- Pending, rejected, and needs-changes: never family-visible and never eligible
  for retrieval.

This PR deliberately adds no automatic RAG indexing. The USR-006 contract is:

1. Read only the current `onboarding_profile_value` row whose referenced
   revision is approved.
2. Include text/media only when the approved value has `family` visibility.
3. Never index a revision, draft, pending, rejected, or needs-changes row.
4. Re-check the approved revision/version at publication time and remove the
   previously published item when a new revision supersedes it.

## Tests

- Rust: `tests/onboarding_profile_revisions.rs` covers queue filtering,
  pagination, detail history, self/owner-linked decision blocking, concurrent
  decisions, stale versions, and the family-audience boundary.
- Rust media: `tests/onboarding_profile_media.rs` covers pending-only review
  access, submitter/owner denial, and post-decision closure.
- Browser: `web/e2e/review-console.spec.ts` covers the queue, comparison,
  approval, needs-changes notes, rejection history, self-review blocking,
  non-admin and deactivated denial, private draft exclusion, media preview,
  concurrent decisions, mobile layout, and keyboard activation.
