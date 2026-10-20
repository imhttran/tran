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

## USR-003 — Usernames, invitations, and account linking

Implement the username policy and the provider-independent invitation/link lifecycle.
Usernames follow `[lastInitial][MiddleInitial][FirstName]`, normalize Vietnamese diacritics
(with a documented `Đ/đ` rule), preserve the original display name, and never mutate
`person_key`. Missing or compound name components use a documented parse rule and an admin
confirmation path; ambiguous parses are not silently accepted. Enforce case-insensitive
uniqueness and deterministic numeric collision suffixes (`nvan2`) with no birthday-derived
content. Invitations store only a hash of a cryptographically random one-time activation
token, expire promptly, are single-use, and are never logged. Linking binds the authenticated
OIDC `(issuer, subject)` to a person only after explicit review, one active link per person,
with audited revoke/replace and no cross-issuer privilege transfer. Date of birth is never
required to generate credentials, and no deterministic birthday-derived password is
implemented.

### Deliverables

- A username module with normalization, initials extraction, validation, and collision
  resolution, covered by unit tests for Vietnamese, ASCII, missing and compound names.
- Invitation create/resend/expire/cancel/revoke operations with hashed single-use tokens and
  explicit status transitions.
- A person-account link workflow with a unique active link per person, audited replacement,
  and rejection of unverified claims.
- Admin-only API endpoints for the invitation lifecycle and link review, with authorization
  before database reads.
- Synthetic tests for collision retries, token expiry/reuse, cross-issuer attempts, and
  duplicate-link rejection.

### Acceptance Criteria

- `Nguyen Van An` produces display `NVAn` and case-insensitive key `nvan`; diacritics and
  `Đ/đ` are handled deterministically.
- Collisions resolve deterministically with an approved numeric suffix and remain stable
  after profile edits.
- A token is usable at most once, expires, and is stored only as a hash.
- No password is generated from a name, surname, or birthday.
- A link cannot be claimed by display name, email, username, or birthday alone.
- One person has at most one active account link; replacement is audited.

### Acceptance Checks

- `cargo test --locked`
- `cargo fmt --check`

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

