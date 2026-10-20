# PLAN-TRAN-USER-ONBOARDING — SOP execution decomposition

## Project

Tran family genealogy application: living-member onboarding, profiles, and branch-scoped review.

## Summary

Deterministic SOP decomposition of `docs/plans/PLAN-TRAN-USER-ONBOARDING.md` (the canonical
requirements). This document only defines executable stages; the canonical plan remains the
authority for product requirements, privacy rules, and acceptance. No real family member is
invited or activated, no production administrator is bootstrapped, no real Head of Family is
assigned, and nothing is deployed or merged by these stages. All fixtures are synthetic.

## USR-001 — Preflight and identity security prerequisite

Verify the execution baseline and fix the identity model before any onboarding work. The
current schema makes `users.subject` globally unique and `provision_user` rewrites `issuer`
on conflict, which would let the same subject string from another issuer inherit an existing
account. Replace that with an immutable `(issuer, subject)` identity and prove that a
different issuer cannot take over, reactivate, or transfer privileges for an existing
account. Preserve the last-active-admin guard and the existing audit behavior. Record in the
stage evidence that open PR #16 (read-only family tree) and PR #17 (this plan) exist and that
no profile/asset engine has been merged, so this work must not duplicate them.


### Execution

- done

### Deliverables

- An additive migration that replaces global `subject` uniqueness with uniqueness on
  `(issuer, subject)`, preserving existing rows and indexes.
- `provision_user` keyed on `(issuer, subject)` that never rewrites an existing issuer and
  only reactivates the matching identity.
- Lookups that change global state (role grants, corpus grants, deactivation) resolve an
  identity by `(issuer, subject)`, or are explicitly documented as subject-only with a safe
  disambiguation rule.
- Synthetic integration tests for cross-issuer takeover, reactivation, and deactivation.
- A short preflight note recording the baseline commits, open work, and the prerequisite fix.

### Acceptance Criteria

- A subject already registered under issuer A is never modified or reactivated by a
  provision call for issuer B.
- The same subject string may exist as two distinct accounts under different issuers without
  sharing roles, grants, audit history, or deactivation state.
- Deactivating the last active administrator still fails with the existing guard.
- Migrations are additive, upgrade-safe from the current schema, and tested only against a
  disposable database.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

## USR-002 — Onboarding schema and verified living candidates

Add the additive onboarding schema (invitations, username reservations, account links,
branch reviewer grants, profiles, profile revisions, and media metadata) and implement the
read-only living-person candidate inventory. Candidate eligibility is derived only from the
private genealogy database: a reviewed `assertion_kind='vital_status'` with verified living
evidence and attributed citations, reusing the TRAN-002 conventions. Exclude or hold
contradictory, disputed, uncited, ambiguous, or unresolved-duplicate people as `needs_review`.
Provide an admin-only preview with classification reasons and a dry-run that makes zero
account or invitation changes. Never publish names or counts, and never infer living status
from a missing death year or a model answer.


### Execution

- done

### Deliverables

- Additive `genealogy/migrations/0007_onboarding.sql` (or the next free version) for
  invitations, username reservations, person-account links, branch reviewer grants, profile
  values, profile revisions, and media metadata; no destructive changes to existing tables.
- A Rust `onboarding` module exposing a deterministic candidate query and classification:
  `eligible_for_invite`, `needs_review`, `already_linked`, `invited`, `not_eligible`.
- An admin-only `GET` preview endpoint and a dry-run operation that performs no writes.
- Synthetic fixtures and tests covering verified living, deceased, disputed, contradictory,
  missing-citation, ambiguous, and already-linked cases.
- Privacy-safe logging: no names, dates, source excerpts, or private hashes in operational
  output.

### Acceptance Criteria

- Only a person with verified living evidence and citations is `eligible_for_invite`.
- Contradictory death evidence, disputed review, missing citations, and unresolved
  duplicates are `needs_review`, never automatically invitable.
- The preview is admin-only and a non-admin receives `403` before any database read.
- A dry-run creates no invitation, link, reservation, or audit row.
- No real person data appears in committed fixtures, tests, logs, or error bodies.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

## USR-002B — Admin living-candidate preview endpoint

Wire the existing `onboarding::candidate` classifier to an admin-only, read-only HTTP
endpoint. The endpoint returns opaque person keys, classifications, and reasons only. It never
returns display names, dates, source excerpts, hashes, or counts of real people. Authorization
runs before any genealogy database read, and the endpoint performs no writes.

### Dependencies

- USR-002

### Discovery Iterations

- 6


### Execution

- done

### Deliverables

- `GET /api/admin/family-members/candidates` returning `{rows, summary}` with opaque
  `person_key`, `classification`, and `reasons`.
- Admin authorization before opening the genealogy database; a non-admin receives `403`
  without a read.
- Integration test `tests/onboarding_preview_api.rs` using synthetic people only.
- Privacy-safe logging and error bodies.

### Acceptance Criteria

- An admin receives classified rows; a non-admin receives `403` before any database read.
- The response contains no display names, birth dates, source excerpts, or private hashes.
- A zero-candidate database returns an empty list and a review backlog, not an error.
- The endpoint performs no write.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1
- criterion 4: check 1

## USR-003 — Invitation and linking closure

Integration closure for the four USR-003 slices. Verify that the slices compose into one
idempotent flow: eligible preview, transactional username reservation, hashed single-use
invitation, reviewed identity link, and audit history. This stage adds no new product behavior;
it proves the slices fit together and that no slice weakened a privacy or authorization rule.

### Dependencies

- USR-002B
- USR-003A
- USR-003B
- USR-003C
- USR-003D

### Deliverables

- `tests/onboarding_flow.rs` covering the full synthetic flow and a repeated run with no
  duplicate reservation, invitation, link, or audit row.
- A short integration note in `docs/architecture/ONBOARDING-IDENTITY.md` linking the slices.

### Acceptance Criteria

- The full synthetic flow succeeds with only synthetic identities and fixtures.
- Re-running the flow is idempotent and creates no duplicate rows.
- No slice bypasses admin authorization, cross-issuer checks, or hash-only token storage.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1

## USR-003A — Username normalization and transactional reservation

Implement transactional reservation of the candidate username for one person. The display
username follows `[lastInitial][MiddleInitial][FirstName]` from verified structured parts and
keeps the original Unicode display form; the canonical key is ASCII, case-insensitive, and
folds Vietnamese diacritics including `Đ/đ`. Reservation enforces a case-insensitive unique
key in the genealogy database, resolves collisions with the smallest numeric suffix (`nvan2`),
is idempotent for the same person, and never uses a date of birth.

### Dependencies

- USR-002

### Discovery Iterations

- 6


### Execution

- done

### Deliverables

- `src/onboarding/reservation.rs` with reserve, claim, and release operations in one
  transaction.
- Additive `genealogy/migrations/0008_onboarding_identity.sql` adding a canonical
  `username_key` with a unique index to `onboarding_username_reservation`.
- `tests/onboarding_reservation.rs` using synthetic person rows only.
- Wire the module into `src/onboarding/mod.rs`.

### Acceptance Criteria

- `Nguyen Van An` reserves display `NVAn` with key `nvan`.
- A taken `nvan` reserves `nvan2`; an existing `nvan` and `nvan2` reserve `nvan3`;
  comparison is case-insensitive.
- Repeating the same reservation for the same person returns the existing row and writes no
  duplicate.
- Releasing a reservation allows the key to be reused; no birthday is stored or used.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1
- criterion 4: check 1

## USR-003B — Random, expiring, single-use invitation tokens

Implement the invitation token lifecycle with provider-independent credentials. Generate a
cryptographically random 32-byte token, return the plaintext exactly once to the caller, and
persist only its SHA-256 hash. Tokens expire promptly, are single-use, and have explicit
pending/accepted/revoked/expired transitions. No plaintext token, password, or birth date is
stored, logged, or placed in an audit row.

### Dependencies

- USR-003A

### Discovery Iterations

- 6


### Execution

- done

### Deliverables

- `src/onboarding/invitation.rs` with token creation, hash verification, expiry, consumption,
  and revoke/cancel transitions.
- Additive `genealogy/migrations/0008_onboarding_identity.sql` adding `token_hash` (unique),
  `expires_at`, `used_at`, and making the legacy plaintext column nullable and unused.
- `tests/onboarding_invitation.rs` covering expiry, reuse, wrong token, and idempotent create.

### Acceptance Criteria

- A token is usable at most once and cannot be reused after acceptance or expiry.
- Only the hash is persisted; the plaintext is never written to the database or logs.
- An expired or revoked token fails verification with a safe error.
- Creating an invitation for a person with a pending invitation is idempotent.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1
- criterion 4: check 1

## USR-003C — Reviewed OIDC account-to-person linking

Implement reviewed linking between an authenticated OIDC identity and a genealogy person. The
genealogy database stores only an opaque account id plus an identity fingerprint; the identity
database remains the authority for `(issuer, subject)`. A link requires an explicit admin
review decision, is limited to one active link per person and per account, is audited, and
rejects unknown, deactivated, or cross-issuer-ambiguous identities. No client-supplied account
id can select the identity.

### Dependencies

- USR-003B

### Discovery Iterations

- 6

### Deliverables

- `src/onboarding/link.rs` implementing review, link, revoke, and replacement with audit rows.
- `src/onboarding/account_map.rs` resolving `(issuer, subject)` to the identity account id and
  rejecting cross-issuer ambiguity, unknown users, and deactivated users.
- `docs/architecture/ONBOARDING-IDENTITY.md` defining the cross-database mapping and failure
  rules.
- Additive `genealogy/migrations/0008_onboarding_identity.sql` adding an identity fingerprint
  column and uniqueness indexes.
- `tests/onboarding_link.rs` with synthetic identities and people.

### Acceptance Criteria

- A link is created only after an explicit review decision and an existing active identity.
- A same-subject identity from another issuer is rejected as ambiguous and never linked.
- One person has at most one active link; replacement is audited and does not merge records.
- Deleting or deactivating an identity does not silently transfer or preserve access.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1
- criterion 4: check 1

## USR-003D — Admin-only invitation and linking API

Expose the invitation and linking slices through admin-only HTTP endpoints. Authorization runs
before any database read, response bodies carry stable ids and safe errors (`401/403/404/409`),
and no names, tokens, hashes, or identity claims appear in logs. The Next.js proxy remains the
CSRF trust boundary; the browser never supplies a role, account id, or approval status.

### Dependencies

- USR-003A
- USR-003B
- USR-003C

### Discovery Iterations

- 6

### Deliverables

- Admin routes for invitation create/resend/cancel/revoke and link review/revoke.
- Handlers in `src/api.rs` delegating to the slice modules; no duplicated business logic.
- `tests/onboarding_admin_api.rs` proving admin-only access, safe errors, and no PII leakage.
- Reuse the existing idempotency and audit behavior.

### Acceptance Criteria

- A non-admin receives `403` before any database read on every onboarding route.
- Unknown ids return `404`; duplicate or conflicting transitions return `409`.
- Responses and logs contain no plaintext token, hash, display name, or identity claim.
- Invitation and link operations are idempotent under retry.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1
- criterion 4: check 1

## USR-004 — Self-service profile, photo, and personal document

Implement member-managed profile submissions using the additive tables from USR-002. A member
may edit only their own approved personal fields, photo, and one personal document. Every
edit creates a versioned `draft`/`pending_review` revision with actor, timestamp, original
value, proposed value, and attachments; authorized viewers continue to see the last approved
value until review completes. Photos are owner-private by default, validated for type and
size, decoded and re-encoded to strip metadata, and served only through authorized endpoints.
An unlinked account cannot edit genealogy claims. Do not build a second asset engine: extend
the profile/revision/media tables and reuse the existing review and authorization patterns.

### Deliverables

- Profile read/write endpoints for the authenticated member: current approved values, own
  drafts, and review decisions.
- Versioned profile revision storage with draft/submitted/approved/rejected/needs_changes
  states and immutable history.
- Private photo and personal-document upload with validation, metadata stripping, and
  authorized download/preview.
- Per-field visibility and owner-private defaults.
- Negative tests: another member, an unlinked member, and a non-member cannot read or edit
  private profile fields or media.

### Acceptance Criteria

- A draft is visible only to its owner and authorized reviewers; other viewers see the last
  approved value.
- A member cannot edit another member's profile, media, or genealogy claims.
- Unapproved or owner-private content never enters retrieval or public responses.
- Uploaded media is re-encoded, size/type validated, and never stored or served with
  original metadata.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

## USR-005 — Branch-scoped reviewer grants

Add explicit application grants for branch review, independent of the genealogical Head of
Family title. A branch reviewer may verify another person's submissions only inside granted
branch keys; they cannot self-approve, cross branches, change system roles, read private
profile media or contact fields without separate delegation, or create/revoke a Head of
Family designation. The verified Head of Family may be granted review authority for their own
branch only by an explicit admin action and gains no automatic admin privileges. Cross-branch
claims require both applicable grants or an admin decision.

### Deliverables

- Additive branch reviewer grant storage with branch key, granted-by, timestamps, and
  revocation.
- Authorization helpers that resolve reviewer scope from the database, never from client
  input or the genealogical title.
- Scoped queue and decision endpoints that filter by granted branch before returning data.
- Tests for in-scope approval, cross-branch denial, self-approval denial, role-change denial,
  and Head-of-Family-without-grant denial.

### Acceptance Criteria

- A reviewer sees and decides only submissions whose person belongs to a granted branch.
- A submitter can never approve their own submission, including when they are an admin.
- Holding the Head of Family title grants no application permission by itself.
- Cross-branch or ambiguous claims require an admin decision and are audited.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

## USR-006 — Review decisions, publication, and audit

Implement field-level shared-fact proposals and the review workflow. A proposal records the
actor, original and proposed values, evidence ids, and branch scope. Decisions are
`approved`, `rejected`, or `needs_changes`, require a reviewer distinct from the submitter
(admin included), and are recorded immutably with reason, evidence, branch grant, and
timestamp. Approval updates the authoritative record and revision atomically with a conflict
check; disagreement retains the previous approved value and marks a conflict. Original source
evidence and provenance are never overwritten. Only approved, family-visible content may be
indexed for retrieval. Changing approved genealogy evidence re-evaluates affected Head of
Family eligibility without appointing a successor.

### Deliverables

- Field-level proposal storage and endpoints for shared genealogy facts.
- A decision service enforcing distinct-reviewer, evidence, branch-scope, and conflict rules.
- Atomic approval that updates the authoritative value and revision, with immutable decision
  history.
- Retrieval/indexing guard that admits only approved, non-private content.
- Eligibility re-evaluation hook that never auto-appoints.
- Tests for approved/rejected/needs_changes, self-review denial, missing-evidence denial,
  conflict retention, and approved-only indexing.

### Acceptance Criteria

- Every decision records reviewer, reason, evidence, branch grant, and timestamp.
- A proposal without the required original-source evidence cannot be approved.
- A conflicting approval preserves the last approved value and records the conflict.
- Unapproved, rejected, or private content is never returned by retrieval.
- Approved evidence changes trigger eligibility re-evaluation without succession.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

## USR-007 — Admin/member/branch UI and end-to-end acceptance

Deliver the three browser surfaces and the end-to-end acceptance evidence. The admin
family-members screen previews eligible living people and manages invitations, links, username
collisions, branch reviewer grants, and audit history. The member profile screen edits only
the member's own approved fields, photo, and personal document, and shows draft versus
verified state. The branch-review screen lists only granted-branch submissions with citations
and diffs. All screens rely on the existing OIDC session and CSRF-protected proxy; no
authorization decision moves to the browser. Use synthetic people and a synthetic OIDC
authority for every test.

### Deliverables

- Admin family-members page with preview, invite lifecycle, collision resolution, link review,
  and grant management.
- Member profile page with draft editing, private media, and review status.
- Branch-review queue and decision page scoped to the reviewer's grants.
- Rust API and Playwright tests using synthetic data, including zero-leak negative tests for
  names, tokens, hashes, and private media.
- Documentation updates for operators and the privacy boundary.
- A final privacy/security pass: `git ls-files docs/tran/` empty, no real PII or secrets in
  the diff, and all negative tests passing.

### Acceptance Criteria

- Each surface renders only the data its caller is authorized to see.
- Browser tests prove admin-only preview, member self-service isolation, and branch-scope
  denial with synthetic fixtures.
- No real person, date, password, token, source excerpt, or private hash appears in the
  repository or test output.
- Rust gates, web typecheck/build, and focused end-to-end tests pass.

### Acceptance Checks

- `cargo test --locked`
- `git diff --check`

