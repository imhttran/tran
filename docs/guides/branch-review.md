# Branch reviewer grants

USR-005A adds explicit, audited branch reviewer authority. A grant is a
technical permission issued by a system administrator; it is never inferred from
a genealogy title, Head of Family appointment, birth order, branch membership, or
profile role.

## Grant model

- At most one **active** grant row per `(account_id, family_branch_id)`; revoked rows remain as immutable history.
- The row is bound to the exact reviewed `identity_fingerprint` of the
  account-person link.
- A grant is usable only while all of these remain true:
  - the row is not revoked;
  - the account still has the reviewed link with the same account id and
    fingerprint;
  - the branch still exists.
- Replacing the account or revoking the link makes the old grant unusable. A new
  account never inherits it.
- Legacy active grant rows without a fingerprint are fail-closed. An administrator
  can re-grant the branch to bind that active row to the reviewed identity.
- Revocation keeps the row as history. Re-granting the same branch creates a new
  active row and appends a new grant event.
- Grant and revocation events are append-only and survive account, link, branch,
  or grant-row cleanup.

A Head of Family may receive a grant, but only through this same explicit
administrator process. The designation itself grants no application permission.

## Admin APIs

All routes require `Operation::BranchReviewerAdmin` (system administrator).

| Route                                                | Purpose                                                                    |
| ---------------------------------------------------- | -------------------------------------------------------------------------- |
| `GET /api/admin/branch-reviewers/branches`           | List grantable branch keys and labels.                                     |
| `GET /api/admin/branch-reviewers?account_id=`        | List current/historical grants and events with person and account context. |
| `POST /api/admin/branch-reviewers`                   | Grant one or more branches to a verified, active, reviewed-linked account. |
| `POST /api/admin/branch-reviewers/grants`            | Grant the one verified branch recorded on the reviewed person.             |
| `GET /api/admin/branch-reviewers/persons?q=`         | Search people with account, contact, branch, and eligibility details.      |
| `POST /api/admin/branch-reviewers/{grant_id}/revoke` | Revoke one active grant with a required reason.                            |

Grant request:

```json
{
  "issuer": "https://issuer.example",
  "subject": "opaque-subject",
  "branch_keys": ["branch-1", "branch-2"],
  "reason": "Approved by family council"
}
```

Recorded-branch grant request. The administrator names the reviewed person; the
server resolves the single verified recorded branch and never accepts a branch
choice:

```json
{
  "issuer": "https://issuer.example",
  "subject": "opaque-subject",
  "person_key": "opaque-person-key",
  "reason": "Approved by family council"
}
```

Revoke request:

```json
{ "reason": "Reviewer term ended" }
```

The grant response never returns a token or credential. The admin grant list and
person search are the only responses that return the account's recorded email:
it sits in an admin-only `contact` block labelled source `identity_account` and
verification `not_recorded`, and the archive records no phone, so `phone` is
`null` with source `not_recorded`. No member-facing endpoint, log, or error body
carries those contact values.

## Authorization rules

- The target account must resolve in the identity store and be active.
- The target account must have a reviewed `onboarding_person_account_link` row
  with the same account id and identity fingerprint.
- A grant or revocation requires a non-empty reason; the actor is the verified
  administrator fingerprint.
- Duplicate active grants are rejected (`409 branch_grant_exists`).
- Grant and event listings use descending id cursors and a hard limit of 100;
  grants and events in one response come from the same repeatable-read snapshot.
- Granting multiple branches is atomic: every requested branch must be valid and
  non-blank, or none is written.
- A branch reviewer does not gain any administrator capability. Admin APIs still
  require the administrator role.

### Recorded-branch grants

- The supplied `person_key` must name the person the reviewed link points at;
  otherwise the request is `409 person_link_mismatch`.
- The branch is taken from the person's own current `branch_claim`/`branch`
  assertions. A name, relationship, appointment, role, or missing field never
  supplies membership.
- Only one current verified claim that resolves to one `family_branch` may
  authorize the grant. Rejections are `409 branch_membership_missing`,
  `409 branch_membership_unverified`, and `409 branch_membership_conflicting`,
  and they write nothing.
- The projection, link lock, grant row, and append-only event are written in one
  transaction; a second grant for the same branch is `409 branch_grant_exists`.
- A missing email or phone never blocks a valid grant.
- `GET /api/admin/branch-reviewers/persons?q=` reports one eligibility value:
  `eligible`, `missing_link`, `missing_branch`, `unverified_branch`,
  `conflicting_branch`, or `already_granted`.

## Database

`0019_branch_reviewer_grants.sql` is additive:

- adds `identity_fingerprint`, grant/revoke reason and actor columns, and
  `updated_at` to `onboarding_branch_reviewer_grant`;
- adds the append-only `onboarding_branch_reviewer_grant_event` ledger;
- adds active-account and event indexes.

No grant is created by the migration, and no existing grant is deleted.

## Verification

`tests/onboarding_branch_grants.rs` covers multi-branch grants, branch scope,
duplicate rejection, revoke/regrant history, unlinked and fingerprint-mismatch
accounts, account replacement, legacy fingerprint-less rows, concurrent grants,
append-only event enforcement, and the recorded-branch grant: missing,
unverified, and conflicting membership are rejected without a write, one
verified branch is granted and scoped, and a mismatched person is rejected.

The browser harness seeds `branch-1`–`branch-3` and resets synthetic grants in
the disposable E2E database only. It also seeds linked synthetic people for the
missing, unverified, verified, and conflicting states, an account without an
email, and one recorded-branch grantee. `web/e2e/branch-account-details.spec.ts`
covers the admin-only contact fields, the four states, a valid grant without an
email, duplicate rejection, the rejection codes, and the admin console's grant
and revocation flows. `web/e2e/branch-details.spec.ts` covers the four states in
the family tree and the family profile.

## Branch-scoped shared-fact review (USR-005B)

Branch reviewers use narrowly scoped operations, not the administrator review
operation. Every request resolves the caller from the verified OIDC identity and
requires the exact reviewed account-person link.

| Route                                                               | Purpose                                                                      |
| ------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `GET /api/branch-review/grants/me`                                  | The caller's active, link-bound branch grants.                               |
| `GET /api/branch-review/submissions?branch=&status=&cursor=&limit=` | Pending or historical proposals within granted branches.                     |
| `GET /api/branch-review/submissions/{id}`                           | One in-scope proposal, its current authoritative fact, evidence, and events. |
| `POST /api/branch-review/submissions`                               | Submit a source-backed shared-fact proposal.                                 |
| `POST /api/branch-review/submissions/{id}/decision`                 | Approve, reject, or request changes.                                         |

- A proposal stores the proposed value, the prior verified snapshot, branch key,
  submitter fingerprint, evidence references, and an append-only event ledger
  (`genealogy_fact_proposal`, migration `0020`).
- A usable grant is an administrator appointment: it must be active, bound to
  the caller's exact verified identity, still backed by the reviewed
  account-person link, and recorded by the append-only `granted` audit event.
  A grant row created out of band confers no authority, and a genealogy title
  never substitutes for a grant.
- Creation requires exactly one verified `branch_claim` for the person and the
  requested branch. Ambiguous or unverified membership is rejected.
- Every evidence reference must resolve to the cited original, non-derivative
  source document at the stated page; a mismatched `document_ref` is rejected so
  an approval cannot record a citation that does not exist.
- A linked owner may propose for their own person; a branch reviewer may propose
  only within a granted branch.
- Decisions recheck the active grant, current link, proposal status, and prior
  verified snapshot inside one transaction. Submitters and linked owners cannot
  decide. Rejection and needs-changes require a note.
- Approval applies the proposal through the existing assertion evidence,
  provenance, validation, review-event, and baseline workflow. Pending proposals
  never replace a verified fact.
- Approval updates the single authoritative assertion in place, exactly as the
  existing assertion review does. The superseded value is never lost: it stays
  immutable on the approved proposal row (`prior_value_text`,
  `prior_review_status`), and the proposal event ledger remains the change
  history for that fact.
- Queue and detail filters never expose an off-branch person or proposal. No
  member-private profile fields, drafts, contact details, personal documents, or
  owner-private media are read by this module.
- Cross-branch relationships and title-derived authority are intentionally not
  inferred. Ambiguous membership and multi-branch relationships must be escalated
  through an explicit policy decision.

## The `/branch-review` interface (USR-005C)

`/branch-review` is available to any signed-in member, but it renders only what
the Rust API authorizes. It reuses the review-console components and layout.

- The branch picker is built from `GET /api/branch-review/grants/me`, so a
  branch that was never granted cannot be selected.
- The queue defaults to pending submissions and offers approved, rejected,
  needs-changes, and all-decision filters, with bounded cursor pagination.
- A row shows the person, the branch, the affected fact, the proposal id, the
  submission time, and how many sources it cites.
- The detail pane compares the currently verified value with the proposed one,
  lists the proposal's cited sources and the other sources recorded for that
  person, and shows the review history.
- Approve, reject, and request-changes use a confirmation step. Rejection and
  change requests require a note. A conflict (409) reloads the current state
  instead of pretending the decision succeeded.
- A proposal the caller submitted, or one about a person the caller is linked
  to, renders an explanation instead of decision controls.

Failure states are explicit and reveal nothing: an expired session, a denial,
and the "no branch review authority" explanation are separate views. A member
without a grant sees an invitation to ask an administrator, never an empty
queue that implies data exists elsewhere.

Browser coverage lives in `web/e2e/branch-review-ui.spec.ts` and runs through
`scripts/web-e2e.sh` against synthetic identities, branches, and sources. The
sixteen-scenario security and end-to-end matrix, with the evidence for each
scenario and the remaining policy gaps, is in
[branch-review-acceptance.md](branch-review-acceptance.md).
