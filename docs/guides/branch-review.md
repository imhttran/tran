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
| `GET /api/admin/branch-reviewers?account_id=`        | List current/historical grants and events, optionally for one account.     |
| `POST /api/admin/branch-reviewers`                   | Grant one or more branches to a verified, active, reviewed-linked account. |
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

Revoke request:

```json
{ "reason": "Reviewer term ended" }
```

The response never returns a raw OIDC subject, token, email, or credential. It
returns the opaque account id, identity fingerprint, branch key/label, actor
fingerprint, timestamps, reason, `active`, and `usable`.

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
and append-only event enforcement.

The browser harness seeds `branch-1`–`branch-3` and resets synthetic grants in
the disposable E2E database only.

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
