# Recorded branch and account details — acceptance

This report records the implementation and verification of
`docs/plans/tran-branch-account-details-requirement.md` (BD-001 through BD-006).
Every fixture, name, key, address, actor, and reason below is synthetic. No real
family name, genealogy identifier, contact value, source excerpt, or private
media appears in this report or in the tracked diff.

## Scope

- BD-001 — deterministic recorded-branch projection, reused by the nomination
  person search.
- BD-002 — additive `branches` block on `GET /api/genealogy/person/{key}` and
  `GET /api/family/persons/{person_key}/profile`, rendered in the family-tree
  person details and the family profile.
- BD-003 — admin-only person, account, contact, branch, and active-grant context
  in the account directory and the grant list.
- BD-004 — `grant_recorded_branch` service and the two new admin routes.
- BD-005 — `BranchGrantAdmin` search, confirmation, grant, list, and revoke
  console on `/admin/family-members`.
- BD-006 — browser/API acceptance coverage, this report, and the focused guide
  update.

Authorization is server-side throughout. An administrator role, Head of Family
designation, surname, relationship, or appointment never substitutes for a
recorded and verified branch claim or for an explicit grant.

## Synthetic fixtures

The harness seeds only disposable test databases. Fixture people:

| Person key                     | Recorded claims                                   | Projected state |
| ------------------------------ | ------------------------------------------------- | --------------- |
| `e2e-branch-subject`           | one verified `branch-1` claim                      | `verified`      |
| `e2e-branch-grantee`           | one verified `branch-2` claim                      | `verified`      |
| `e2e-branch-unverified`        | one unverified `branch-1` claim                    | `unverified`    |
| `e2e-branch-conflicting`       | `branch-1` and `branch-2` claims, unverified       | `conflicting`   |
| `e2e-branch-missing`           | none                                               | `missing`       |

Accounts `e2e-branch-grantee`, `e2e-branch-unverified`,
`e2e-branch-conflicting`, and `e2e-branch-missing` have **no email** in the
identity store and are linked to the matching person. `e2e-member` has a
reserved `.test` address. `e2e-admin` and `e2e-approver` are administrators;
`e2e-viewer` is not linked to a person.

## Recorded branch states

The API person read (`/api/genealogy/person/{key}`) returned:

- `e2e-branch-subject` — `state: verified`, `verified_branch_key: branch-1`.
- `e2e-branch-unverified` — `state: unverified`,
  `verified_branch_key: null`, claim text `branch-1`, `verified: false`.
- `e2e-branch-conflicting` — `state: conflicting`,
  `verified_branch_key: null`, more than one resolved branch key.
- `e2e-branch-missing` — `state: missing`, `verified_branch_key: null`, no
  claims.

The family-tree and family-profile presentations show the state in words, list
each recorded claim with its review status, and show a resolved label only for a
verified state. A payload without a `branches` block renders no section.

## Grant and denial results

- `POST /api/admin/branch-reviewers/grants` for
  `e2e-branch-grantee` / `e2e-branch-grantee` returned `200` with
  `branch_key: branch-2`, `active: true`, and one `granted` event naming the
  administrator fingerprint and the submitted reason. The account has no email,
  so a missing email does not block the grant.
- A second identical request returned `409 branch_grant_exists`; no second
  active grant row was created.
- `e2e-branch-unverified` returned `409 branch_membership_unverified`.
- `e2e-branch-conflicting` returned `409 branch_membership_conflicting`.
- `e2e-branch-missing` returned `409 branch_membership_missing`.
- An unlinked viewer account returned `409 account_not_linked`.
- A person key that is not the reviewed link returned `409
  person_link_mismatch`.
- Every rejection above wrote no grant and no event.
- In the browser console, an administrator searched the grantee, saw the person,
  `person_id`, account `user_id`, branch label/key, and `Eligible`; confirmed
  with a required reason; observed the active grant and audit fields; then
  revoked it and observed the revoked state. Escape and Cancel cancelled the
  confirmation, and the confirm control stayed disabled until a reason was
  entered. A viewer saw the denial view and no console.
- Reviewer denial outside the granted branch remains covered by
  `web/e2e/branch-review-security.spec.ts`; this change does not add a second
  authorization path.

## Admin-only field checks

- The directory row reported `contact.email.value` (source
  `identity_account`, verification `not_recorded`) and `contact.phone` as
  `{ value: null, source: not_recorded, verification: not_recorded }`.
- `linked_person` carried both `person_id` and `person_key`; the account
  `user_id` was shown separately in the UI.
- The grant list returned `person` and `account` blocks plus the contact block
  for each grant. A person or account that no longer exists is `null` and is
  never guessed.
- A viewer received `403` and an anonymous caller `401` for the directory,
  person search, and grant route before any database read.
- The member-facing person read contained no `contacts` block, no email, and no
  other admin contact value.
- Directory grants and branch projections are batch-loaded: one claims query,
  one branch catalog read, and one active-grant query per page rather than a
  query per row.

## Validation commands

The repository gate is:

```sh
cargo build --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
make web-check
make web-e2e
```

Results from this worktree:

- `cargo build --locked` — pass.
- `cargo test --locked` — pass, including
  `tests/onboarding_branch_grants.rs` (11 tests) with a disposable
  `GENEALOGY_DATABASE_URL`.
- `cargo fmt --check` — pass.
- `cargo clippy --locked --all-targets -- -D warnings` — pass.
- `make web-check` — pass (typecheck and production build).
- Browser acceptance — `make web-e2e` requires `API_PORT` (8080), `WEB_PORT`
  (3000), and `E2E_AUTH_PORT` (9099) to be free. In this workstation those
  ports were occupied, so the same harness was run on `API_PORT=8480`,
  `WEB_PORT=3180`, `E2E_AUTH_PORT=9299` against isolated disposable databases
  (`bad_rag_test`, `bad_gen_test`) with a focused spec list:
  `branch-details.spec.ts`, `branch-account-details.spec.ts`,
  `admin-accounts.spec.ts` (port-adjusted copy), `branch-grants.spec.ts`,
  `family-tree.spec.ts`, `family-profile.spec.ts`, `family-profile-ui.spec.ts`,
  and `branch-review-security.spec.ts`. **101 tests passed.** The SOP stage
  should still run the canonical `make web-e2e` on a host with the default
  ports free.

## Privacy review

- The tracked diff contains only synthetic `.test` addresses, synthetic person
  keys, synthetic branch keys (`branch-1`…`branch-3`), and synthetic reasons.
- No real family name, identifier, contact value, source excerpt, hash of a
  private source, or private media was added.
- No live grant, account link, person, branch, role, or invitation was created
  in an application database; every grant and link in this report exists only
  in a disposable test database.
