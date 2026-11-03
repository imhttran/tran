# Tran: recorded branch and account details for branch review

## Project

Tran family archive: show recorded family-branch identity beside the person and
account facts an administrator needs to grant branch-review authority, without
inferring membership or widening authorization.

## Summary

The archive already records branch membership as genealogy assertions and grants
branch-review authority explicitly per branch. Two gaps remain:

1. A family-tree person detail, a person profile, and the administrator branch
   controls do not show the recorded branch label or distinguish missing,
   unverified, and conflicting membership.
2. The administrator branch-review controls cannot show the exact person,
   account, and branch, nor grant review authority for the branch the person's
   own verified claim records.

This plan closes both gaps. It reuses the authorized person search and the
verified branch projection shipped by PR #120 (`GET
/api/genealogy/nomination-persons`, `appointments::search_nomination_people`,
`narrative::search_review_people`). PR #121 is closed and was not merged; its
selector work is not a dependency.

Membership is never inferred from a surname, a relationship, an appointment, a
role, or a missing field. An absent claim means _not recorded_. An unverified
claim means _recorded but not verified_. More than one distinct recorded branch
means _conflicting_. Only one current verified claim that resolves to one
`family_branch` row may authorize a branch-scoped grant.

### How execution works in this repository

- SOP's command policy classifies `cargo`, `psql`, and browser tooling as
  `REQUIRES_APPROVAL`. The implementing agent must write the code and the report;
  the SOP validation stage runs the configured build, test, lint, and integration
  commands after the edit.
- Do not attempt to compile, test, lint, migrate, query a database, or start a
  server from a task. Do not claim a command ran.
- Read only the files a task names, at most once each. Name the exact functions
  and sections to change instead of re-reading a large file.
- `src/api.rs` is large. Locate the named handler and helper with a search; do not
  read the whole file.
- The repository is public. Never commit a real family name, real contact detail,
  real genealogy identifier, source excerpt, hash of a private source, or private
  media. Tests and fixtures are synthetic.
- Never create a person, branch claim, account link, invitation, role, or live
  grant as part of implementation or verification. A grant may be created only by
  a test or by an administrator through the implemented UI/API in a disposable
  test database.

### Motivating case (read-only finding, private identity deliberately omitted)

A read-only inspection of the local development genealogy database found a real
cataloged person with no `onboarding_person_account_link`, no username
reservation, no `family_branch` membership, and no current verified
`branch_claim`. The person has one current, unverified legacy `branch` assertion
whose value is descriptive text, not a branch key. The person is therefore
ineligible for a recorded-branch grant: there is no linked account and no
verified branch. The implementation must report these states honestly and must
not infer membership, create a link, or grant access. Do not commit the person's
name or identifiers.

### Boundaries that apply to every task

- Authorization stays server-side. An administrator role or a Head of Family
  designation never substitutes for an explicit branch grant, and a grant for one
  branch never authorizes another branch.
- Contact details are administrator-only. Never add them to `GET
/api/genealogy/person/{key}`, `GET /api/family/persons/{person_key}/profile`,
  branch-review member responses, logs, or error bodies.
- Person identity and account identity stay distinct: `person_id`/`person_key`
  identify the genealogy person; `user_id` (or `account_id`) identifies the
  identity account. Do not print one as the other.
- A missing optional email or phone never blocks a valid grant.
- Reuse the existing `onboarding_branch_reviewer_grant` row and its append-only
  grant event. Do not add a second grant table or a second audit path.
- Do not modify the active archive-photographs plan or its worktree.

## BD-001 — Recorded branch projection

Create one deterministic projection of a person's recorded branch membership and
reuse it from the existing nomination search.

Files to read, at most once each: `src/genealogy/current.rs`,
`src/genealogy/appointments.rs` (only `search_nomination_people`),
`src/genealogy/mod.rs`, `docs/guides/branch-review.md`, and
`scripts/seed-branch-review-e2e.sql`.

Create `src/genealogy/branch_projection.rs` and declare it in
`src/genealogy/mod.rs`. The module must:

- Read only current assertions: `assertion_kind` in (`branch_claim`, `branch`)
  with a non-blank `value_text` and the shared `current::CURRENT_ASSERTION`
  predicate. `branch_claim` is the canonical kind; `branch` is the legacy
  imported kind and must stay visible as a recorded claim.
- Resolve a claim to a `family_branch` only by an exact, case-insensitive,
  trimmed match of the claim value to `branch_key` or `branch_label`. Never
  partial-match, never use a surname, a relationship, an appointment, or a role.
- Classify with a pure function so it can be unit-tested without a database:
  - `missing`: no current claims;
  - `conflicting`: more than one distinct resolved `branch_key`, or no resolved
    key and more than one distinct recorded text, or more than one distinct
    verified text that does not resolve to one key;
  - `verified`: exactly one distinct resolved `branch_key` among verified claims,
    no conflicting state, and every verified claim resolves;
  - `unverified`: claims exist but the verified rule does not hold.
- Expose the state, the single verified `branch_key` when verified, and every
  claim with `assertion_kind`, `value_text`, `review_status`, `verified`,
  `branch_key`, and `branch_label`.
- Provide `project(client, person_key)`, `project_for_person_id(client,
person_id)`, `project_many(client, person_keys)`, and a
  `verified_branch_labels(client, person_key)` helper. Use
  `postgres::GenericClient` so a transaction can call the projection inside an
  authorization-sensitive write.
- Refactor `appointments::search_nomination_people` to use
  `verified_branch_labels` instead of its own copy of the verified-claim SQL. Its
  response shape and behavior must not change.

Write unit tests in the module for: no claim, one unverified claim, one verified
claim, two verified claims naming the same branch by key and label, two verified
claims naming different branches, an unverified claim beside a verified claim,
and an unrecognized descriptive value that must not resolve to a branch.

### Dependencies

None

### Deliverables

- New `src/genealogy/branch_projection.rs` and its declaration in
  `src/genealogy/mod.rs`.
- The reusable helper in `appointments::search_nomination_people`.
- Unit tests in the new module; no database and no file writes.

### Acceptance Criteria

- The four states are distinguishable and the classifier never invents a branch
  from an unrecognized value.
- A legacy, unverified `branch` claim remains visible as an unverified claim and
  does not become a `family_branch` label unless its text exactly matches a
  recorded key or label.
- Two verified claims that name the same branch by key and by label are
  `verified`, not `conflicting`.
- `search_nomination_people` keeps returning the same verified labels for the
  same data through the shared helper.
- The module compiles under the SOP validation stage and its unit tests pass.

## BD-002 — Person detail and family profile branch labels

Expose the projection on the two member-facing person reads and render it in the
family tree person details and the family profile page.

Files to read, at most once each: `src/api.rs` (only `genealogy_person`,
`family_profile`, and the adjacent JSON helper area), `src/genealogy/query.rs`
(only `PersonDetail`), `web/src/lib/api.ts` (only the person/profile types),
`web/src/components/FamilyTree.tsx` (only the person-details section),
`web/src/components/FamilyProfileView.tsx` (only the dates section and its
styles), and `web/src/app/globals.css` (only the family profile and tree
section classes).

Add an additive `branches` object to both responses:

```json
{
  "state": "missing | unverified | verified | conflicting",
  "verified_branch_key": null,
  "claims": [
    {
      "assertion_kind": "branch_claim",
      "value_text": "branch-1",
      "review_status": "verified",
      "verified": true,
      "branch_key": "branch-1",
      "branch_label": "First Branch"
    }
  ]
}
```

- `GET /api/genealogy/person/{key}` keeps its existing `GenealogyRead`
  authorization. An unknown person keeps `404`.
- `GET /api/family/persons/{person_key}/profile` keeps its existing membership
  and publication authorization. Unknown or unauthorized persons keep the
  existing response. The branch block contains shared genealogy membership only,
  never owner-private profile fields or contact details.
- In the family-tree person details, add a "Recorded branch" section. Show the
  state in words, each recorded claim with its review status, and the resolved
  label only when one exists. Missing says "No branch recorded"; unverified says
  "Recorded but not verified"; conflicting says the recorded branches disagree.
  Do not render a branch for an unresolved claim.
- In the family profile, add the same section using the existing dates/empty
  section pattern. Keep it visible for a published profile and omit it cleanly
  when the payload has no branch block (older server).
- Make the TypeScript contract additive and tolerant of an absent `branches`
  field. Use semantic HTML and text, not color alone.

### Dependencies

BD-001

### Deliverables

- The additive JSON block in the two API handlers and their helper.
- The additive TypeScript types.
- The "Recorded branch" rendering in `FamilyTree.tsx` and
  `FamilyProfileView.tsx`, with minimal CSS additions.
- Focused component or browser coverage for missing, unverified, verified, and
  conflicting states.

### Acceptance Criteria

- A person with no claim shows "No branch recorded" and no invented label.
- A person with one unverified claim shows the recorded text and "not verified",
  with no resolved label unless the text exactly matches a recorded branch.
- A person with one verified claim shows the recorded branch label and key.
- A person with two conflicting claims shows the conflict and no single
  authorized branch.
- Neither endpoint returns account ids, usernames, emails, phone numbers, or
  owner-private profile values.
- The web build and the focused browser checks pass.

## BD-003 — Administrator account, person, and contact details

Enrich the admin directory and grant list with the exact person and account facts
the requirement names, without exposing them outside admin-only operations.

Files to read, at most once each: `src/admin_members.rs`,
`src/api.rs` (only `family_members`, `FamilyMemberBody`, `grant_json`, and
`list_branch_reviewers`), `web/src/lib/api.ts` (only the `FamilyMember` type and
admin member client), `web/src/components/AccountsAdmin.tsx` (only `MemberRow`
and its metadata rendering), and `src/genealogy/branch_projection.rs`.

Requirements:

- Extend `admin_members::LinkedPerson` with `person_id` and `display_name` and
  return them from `linked_people`. Keep `link_id`, `person_key`, and `username`.
- Add an admin-only `contact` object to each directory row and to each grant
  entry:
  - `email`: the identity account's `users.email` value, source
    `identity_account`, verification `not_recorded`;
  - `phone`: `null`, source `not_recorded`, verification `not_recorded`.
    The archive records no phone field. Never infer a phone from `whatsapp`,
    `contact_preferences`, or any other owner-private value. Never label an email
    verified: the identity schema has no verification flag.
- Add the person's recorded-branch projection to each linked directory row. Batch
  the projection for the page; do not run one query per row.
- Add the account's active branch grants to each directory row, batched by
  account id, so the UI can show "already granted" without a duplicate request.
- Enrich the `GET /api/admin/branch-reviewers` response with `person` and
  `account` blocks. Keep every existing grant field. An account or link that no
  longer exists is reported as `null`, never guessed.
- Keep all of this behind `Operation::MemberDirectory` or
  `Operation::BranchReviewerAdmin`. Non-admin and anonymous calls must remain
  `401`/`403` before any database read.
- In `AccountsAdmin`, show the person display name and `person_id` beside the
  existing person key and username, the account `user_id`, the email with its
  source and recorded verification label, and "phone: not recorded". Show the
  branch state and label when present. Do not change role, link, reset, or
  deactivate behavior.

### Dependencies

BD-001

### Deliverables

- Extended `LinkedPerson` and `linked_people`.
- Admin-only contact, branch, and active-grant fields in the directory and grant
  list JSON, with batching.
- The corresponding TypeScript types and `AccountsAdmin` rendering.
- Focused tests proving admin-only access and the missing-contact state.

### Acceptance Criteria

- An administrator sees the person's name, `person_id`, `person_key`, username,
  account `user_id`, email, and available phone detail, with the email and phone
  sources and verification status labelled.
- `person_id` and `user_id` are visibly distinct and named separately.
- A missing email or phone is shown as not recorded and does not block any admin
  action.
- A viewer or anonymous caller cannot read the directory or the contact fields.
- Branches and grants are batched; the directory does not issue a query per row.
- The existing admin account tests and the web build pass.

## BD-004 — Recorded-branch grant service and API

Add a server-side grant path that resolves the reviewed link and the recorded
branch, then writes through the existing grant and audit machinery.

Files to read, at most once each: `src/onboarding/branch_review.rs`
(`grant_branches`, `grant_one`, `lock_reviewed_link`, `branch_by_key`, error
types), `src/genealogy/branch_projection.rs`, `src/api.rs` (only the branch grant
handlers and `branch_grant_error`), `src/onboarding/account_map.rs`, and
`tests/onboarding_branch_grants.rs`.

Add `grant_recorded_branch(client, account, person_key, actor, reason)` to
`src/onboarding/branch_review.rs`. It must:

- clean the actor and reason with the existing validators;
- open one transaction, call `lock_reviewed_link`, and re-read the linked
  person's key;
- reject a supplied `person_key` that does not match the reviewed link;
- call the branch projection inside the same transaction;
- reject `missing`, `unverified`, and `conflicting` membership with distinct
  errors;
- resolve the single verified branch key with `branch_by_key`, call the existing
  `grant_one`, and commit. This preserves the existing unique-active-grant and
  append-only event behavior; a second grant for the same branch is
  `AlreadyActive`, never a duplicate row.
- never create a link, branch, person, or role.

Add a new admin-only route `POST /api/admin/branch-reviewers/grants` with body
`{issuer, subject, person_key, reason}`. Resolve the account through the identity
store, then open the genealogy store and call the service. Map the new errors to
stable codes:

- `branch_membership_missing` (409);
- `branch_membership_unverified` (409);
- `branch_membership_conflicting` (409);
- `person_link_mismatch` (409);
- the existing `account_not_linked`, `branch_grant_exists`, and validation
  responses stay unchanged.

Add a read-only admin route `GET /api/admin/branch-reviewers/persons?q=` that
reuses the authorized person search (`narrative::search_review_people`, the same
search behind PR #120's nomination selector), limited to a small page. For each
result return the exact person, the recorded branch projection, the resolved
reviewed account (or `null`), the admin contact block, the active grant for the
verified branch (or `null`), and one eligibility value:

- `eligible`;
- `missing_link`;
- `missing_branch`;
- `unverified_branch`;
- `conflicting_branch`;
- `already_granted`.

The search must never infer an account from a name, email, or username. A
missing email or phone must not change `eligible`.

### Dependencies

BD-001

### Deliverables

- `grant_recorded_branch` and its error variants, reusing `grant_one` and the
  existing event ledger.
- The two admin routes and their JSON/error mapping.
- Service-level tests and API/browser tests for the valid, missing, unverified,
  conflicting, unlinked, mismatched, and duplicate cases.

### Acceptance Criteria

- A linked person with exactly one verified recorded branch receives one active
  grant for exactly that branch, and the append-only event names the actor and
  reason.
- A missing link, missing branch, unverified branch, conflicting branch, or
  mismatched person is rejected and writes nothing.
- A second grant for the same account and branch returns
  `branch_grant_exists`; no duplicate grant row or duplicate event is written.
- A grant for one branch does not authorize another branch.
- Missing email or phone does not block an otherwise valid grant.
- Admin, viewer, and anonymous access are distinguished before any sensitive read.
- The Rust tests and focused browser tests pass.

## BD-005 — Administrator branch-review grant console

Add the administrator-facing search, confirmation, grant, listing, and revocation
surface for recorded-branch grants.

Files to read, at most once each: `web/src/components/AccountsAdmin.tsx` (for the
confirmation-dialog and status patterns), `web/src/app/admin/family-members/page.tsx`,
`web/src/lib/api.ts` (the admin member client and types), `web/src/app/globals.css`
(the admin console section classes), and the new admin routes from BD-004.

Create `web/src/components/BranchGrantAdmin.tsx` and render it on
`/admin/family-members` for administrators only, below the account directory.

- Search by person name with a short debounce, stale-response guard, and
  loading/empty/error states.
- Each result shows the exact person (name, `person_key`, `person_id`), the
  resolved account (username, `user_id`, email with source/verification, phone
  with source/verification), and the recorded branch (state, label, key).
- The "Grant branch review" control is enabled only for `eligible`. For every
  other eligibility value, render the reason and disable the control: not linked,
  no branch recorded, branch not verified, conflicting branch, or already
  granted.
- Confirmation shows the exact person, account `user_id`, and branch label/key,
  plus a required reason field. Cancel takes initial focus; Escape and outside
  click cancel; success is shown only after the API confirms and the list
  reloads. A 409 is displayed as a stable, human message, not as success.
- The existing grants table shows person, account, branch, actor, reason, status,
  and timestamps, and offers a revoke action with a required reason and
  confirmation. Revocation keeps existing behavior and audit history.
- Reuse the existing admin denial view. No authorization decision is made in the
  browser.
- Keep the layout accessible and responsive: labelled controls, visible focus,
  text state (not color alone), and no horizontal overflow.

### Dependencies

- BD-003
- BD-004

### Deliverables

- New `BranchGrantAdmin.tsx` and its wiring into the admin family-members page.
- Additive API client functions and types.
- Minimal CSS consistent with the existing admin console.
- Focused browser coverage for eligibility, confirmation, grant, duplicate, and
  revocation.

### Acceptance Criteria

- An administrator can search a person, see the exact person/account/branch, enter
  a reason, confirm, and observe the new active grant.
- A missing link, missing branch, unverified branch, or conflicting branch is
  blocked in the UI and rejected by the API.
- A duplicate grant is shown as already granted; no second grant appears.
- A missing email or phone does not disable an otherwise eligible grant.
- A viewer sees the existing denial and no grant console.
- The web build, typecheck, and focused browser tests pass.

## BD-006 — Acceptance evidence and privacy check

Add the cross-cutting acceptance coverage and a privacy-safe report.

Files to read, at most once each: `web/e2e/branch-grants.spec.ts`,
`web/e2e/branch-review-security.spec.ts`, `web/e2e/admin-accounts.spec.ts`,
`web/e2e/family-profile.spec.ts`, `web/e2e/family-tree.spec.ts`,
`docs/guides/branch-review.md`, and the changed files from BD-001 through BD-005.

- Extend the browser/API acceptance suite with synthetic data only:
  - a non-admin and an anonymous caller cannot read admin contact fields;
  - an administrator sees the contact source and verification labels, including
    a missing phone;
  - missing, unverified, verified, and conflicting branch states render without
    inference;
  - a valid recorded-branch grant succeeds when the account has no email;
  - a duplicate grant returns the stable conflict and creates no second grant;
  - a reviewer is denied outside the granted branch and inside the granted branch
    (reuse the existing security spec where it already proves this).
- Add `docs/reports/branch-account-details-acceptance.md` with: the scope, the
  synthetic fixtures, the four membership states, the grant and denial results,
  the admin-only field checks, and the exact validation commands the SOP stage
  runs. Do not include a real person, identifier, email, phone, source excerpt,
  or private path.
- Update `docs/guides/branch-review.md` only where the new administrator grant
  path and contact display need documenting. Keep the existing authority rules
  unchanged.
- Scan the tracked diff for private data before finishing: no real family name,
  no real identifier, no contact value, no source excerpt, no private media.

### Dependencies

- BD-002
- BD-003
- BD-004
- BD-005

### Deliverables

- Extended synthetic browser/API acceptance coverage.
- `docs/reports/branch-account-details-acceptance.md`.
- The focused documentation update.
- A privacy review of the tracked diff.

### Acceptance Criteria

- The acceptance suite proves admin-only field access, the four membership
  states, a valid recorded-branch grant with a missing email, duplicate
  prevention, and denial outside the granted branch.
- The report and the tracked diff contain no real family data.
- The repository validation gate passes: `cargo build --locked`, `cargo test
--locked`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D
warnings`, `make web-check`, and `make web-e2e`.
- No live grant, account link, person, branch, role, or invitation is created in
  the operator's application database.
