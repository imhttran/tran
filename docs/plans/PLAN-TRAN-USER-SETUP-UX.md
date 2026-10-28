# Implementation Plan

## Project

Tran genealogy — Member setup experience: names everywhere, a person picker for
linking, and editable usernames

## Summary

**Admin → Family Members** (`web/src/app/admin/family-members/page.tsx`) is
where an admin sets up family members. PR #127
(`docs/plans/PLAN-TRAN-BULK-ONBOARD-TABLE.md`) gave the bulk preview table
names, a masked contact, a complete load and filters. Three problems remain:

1. **Two views still show IDs instead of names.** After **Prepare**, the batch
   status table labels each row by `person_key` only. The Accounts list
   (`web/src/components/AccountsAdmin.tsx`) shows a linked person as
   `person_key` and leads with `subject @ issuer`.
2. **Linking means typing a person key by hand.** The Accounts list's link form
   is a free-text "Person key" box. A typo links the wrong person, and the
   confirmation repeats the same key back.
3. **The proposed username cannot be edited.** `PLAN-TRAN-USER-ONBOARDING.md`
   requires an admin confirmation for ambiguous Vietnamese name parsing and
   username collisions, but the preview only displays the proposal.

This plan fixes all three. It changes presentation and flow only; every
security, privacy and review rule in `PLAN-TRAN-USER-ONBOARDING.md` and
`PLAN-TRAN-BULK-ONBOARDING.md` still applies.

Approved decision (extends PR #127's): admin-only onboarding and account views
may show a person's display name and verified branch labels. No contact detail
beyond PR #127's masked email is added anywhere.

### Later work (not in this plan)

These need design input before they can be planned as tasks:

- **One list with a setup stage per person.** Merge the bulk table and the
  Accounts list into one roster keyed by person, with a stage chip
  (`Not invited → Prepared → Invited → Signed in, awaiting review → Active`) and
  one primary button for the next step. Needs a mockup in `docs/mockups/` built
  on PR #127's table and filters.
- **Invitee pages.** A "You were invited as *Name*. Is this you?" page after the
  activation link, and an "An admin is reviewing your link" page for a
  signed-in, unlinked account. Needs a runtime check first of what an unlinked
  synthetic user sees today; no such page exists in `web/src/app`.

### How execution works in this repository

SOP's command policy classifies `cargo` and `psql` as `REQUIRES_APPROVAL`, and
that class cannot be loosened by project configuration. The agent therefore
cannot run a build, a test, or a migration, and must not spend its window trying
to. The SOP validation stage runs the configured commands after the edit,
supplies the runtime evidence, and drives the FIX cycle when they fail.

Consequences for every task below:

- Read only the files the task names. Do not re-read a file already read in the
  same task.
- Write the code and the report; do not attempt to compile, test, lint, migrate,
  or query a database.
- State in the closing message what was changed and what the validation stage
  must prove. Do not claim a command was run.
- Acceptance criteria describe outcomes the validation stage verifies.

### Boundaries that apply to every task

- **No migration.** `person.display_name`, branch claims, links and username
  reservations already exist. Do not add or change a migration.
- **Server decides.** Names, branches, link state and username validity come
  from the Rust API under the existing admin checks. The browser never supplies
  a name, a branch, or an approval.
- **Reuse, don't duplicate.** Person search reuses `validate_person_search`
  (`src/api.rs`) and `appointments::search_nomination_people`
  (`src/genealogy/appointments.rs`). Username validation reuses
  `src/onboarding/username.rs`. The web picker reuses
  `web/src/components/NominationPersonSelector.tsx`.
- **No auto-linking.** Search only helps the admin choose; the reviewed,
  confirmed link action stays the only thing that grants person-level access.
- **Synthetic fixtures only.** Tests use made-up names such as `Nguyen Van An`
  and `example.test` addresses. No real family name or address enters a
  committed file.
- **Isolated databases only.** Integration tests use
  `tests/support/genealogy_isolation.rs`, never the operator database.
- **Bounded context.** Change only the files each task names and keep each
  task's diff small. `.agents/` stays out of every diff.

## USX-101 — Names in the batch status and the linked person

Return the person's display name wherever the admin API currently returns only
a `person_key`.

Files you may change: `src/api.rs` (the `bulk_batch_status` query and JSON
mapping only), `src/admin_members.rs` (the `LinkedPerson` struct and the
`linked_people` query only).

- In `bulk_batch_status`, the item query already joins `person`; also select
  `person.display_name` and add `display_name` to each item's JSON.
- In `admin_members.rs`, add `display_name: String` to `LinkedPerson` and select
  `p.display_name` in `linked_people`, which already joins `person`.
- Keep `person_key` in both responses; the UI still shows it as secondary text.

### Dependencies

None

### Deliverables

- The query and struct changes above.
- Updated unit-test fixtures in `src/admin_members.rs` so they compile with the
  new field.

### Acceptance Criteria

- Each batch status item has `display_name`.
- Each `linked_person` in the family-members list has `display_name`.
- No other field of either response changes.
- The crate compiles and the unit tests pass under the SOP validation stage.

## USX-102 — Admin API coverage for the names

Prove the new fields against a disposable database.

Files you may change: `tests/onboarding_admin_api.rs`, `tests/admin_members.rs`.

### Dependencies

- USX-101

### Deliverables

- A test that prepares a batch for one synthetic person and asserts the batch
  status item carries that person's `display_name`.
- A test that links a synthetic account to a synthetic person and asserts the
  family-members list returns the person's `display_name` in `linked_person`.

### Acceptance Criteria

- Both tests fail if the field is removed.
- The suites use only their own disposable databases.

## USX-103 — Admin person search for linking

Give the link form a server-side search that returns people the admin can
choose, with link state.

Files you may change: `src/api.rs` (a new route and handler next to
`create_link`), `src/admin_members.rs` (one new read-only function),
`tests/admin_members.rs`.

- Add `GET /api/admin/family-members/people?q=&limit=`, authorized with
  `Operation::OnboardingLink` (the same operation as `create_link`).
- Validate with `validate_person_search` and search with
  `appointments::search_nomination_people`, as `nomination_people` does. Use
  error codes `invalid_member_person_search` and
  `invalid_member_person_limit`.
- Add `linked_person_keys(genealogy, &[String]) -> Result<HashSet<String>, String>`
  to `admin_members.rs`, using the same active-link rule as `linked_people`.
- Return the same shape as `nomination_people` (`record_id`, `person_label`,
  `birth`, `branches`) plus `linked: bool`.

### Dependencies

None

### Deliverables

- The route, handler, and read function above.
- Tests: an admin search finds a synthetic person by an unaccented name;
  an already-linked person comes back with `linked: true`; a non-admin is
  denied; an empty query returns no people; an over-long query is rejected.

### Acceptance Criteria

- The handler performs no write.
- A caller without `OnboardingLink` receives the existing denial.
- The search shares validation and query code with `nomination_people`; no
  second search implementation exists.

## USX-104 — Show names in the batch table and the Accounts list

Render the names returned by USX-101.

Files you may change: `web/src/lib/api.ts`,
`web/src/app/admin/family-members/page.tsx` (the batch status table only),
`web/src/components/AccountsAdmin.tsx`.

- Types: add `display_name: string` to the batch status item type and to
  `FamilyMember.linked_person`.
- Batch status table: rename the "Person key" column to "Person" and render it
  like the preview table's Person cell (name in bold, key below in smaller
  muted text). Use the name in the "Verified email for …" label. Keep every
  `data-label` matching its header.
- Accounts list: show "Linked to *Name*" with the key and username as secondary
  text. Move `subject @ issuer` out of the always-visible meta line into the
  Actions drawer.
- When `linkage` is `"unavailable"`, keep the existing notice and fallback.

### Dependencies

- USX-101

### Deliverables

- The type and render changes above.

### Acceptance Criteria

- No row in the batch status table or the Accounts list is labelled by
  `person_key` alone when the genealogy store is up.
- `subject @ issuer` is still reachable in the Actions drawer.
- The TypeScript build passes.

## USX-105 — Person picker in the link form

Replace the free-text person key box with a search picker.

Files you may change: `web/src/components/NominationPersonSelector.tsx`,
`web/src/components/AccountsAdmin.tsx`.

- Add an optional `endpoint` prop to `NominationPersonSelector`, defaulting to
  the current `/api/genealogy/nomination-persons` URL so existing callers are
  unchanged. Add an optional `isDisabled(person)` prop so the caller can mark
  results that cannot be chosen.
- In `AccountsAdmin`, use the selector with
  `/api/admin/family-members/people`. Results with `linked: true` are shown but
  disabled, with the text "Already linked".
- The pending link action carries the chosen person's label and branches. The
  confirmation reads "Confirm you reviewed the identity proof and link *who* to
  **Name** (branches)?".
- "Link after review" stays disabled until a person is chosen from the results;
  typed text alone can never be submitted.

### Dependencies

- USX-103
- USX-104

### Deliverables

- The prop additions and the link form change above.

### Acceptance Criteria

- The link form cannot submit a key that did not come from a search result.
- An already-linked person cannot be chosen.
- The nomination page's use of the selector behaves exactly as before.
- The TypeScript build passes.

## USX-106 — Admin username override at preparation

Let the admin replace a proposed username when preparing, with the server
enforcing the existing policy.

Files you may change: `src/onboarding/username.rs`,
`src/onboarding/reservation.rs`, `src/onboarding/batch.rs`, `src/api.rs` (the
`BulkPrepareRequest` struct and the `bulk_prepare` mapping only),
`tests/onboarding_bulk.rs`.

- `username.rs`: add `validate_override(&str) -> Result<String, UsernameError>`
  that applies the same normalization and character rules as
  `candidate_username` and returns the display username.
- `reservation.rs`: let `reserve` take an `Option<&str>` override. When present,
  validate it and try it instead of `candidate_username(parts)`, with the same
  case-insensitive uniqueness check and numeric suffix loop. An active
  reservation is still returned unchanged; an override never replaces one.
- `batch.rs`: pass each person's override through to `reserve`. A rejected
  override records the existing `username_policy` or `username_collision`
  reason for that row only.
- `api.rs`: add an optional `usernames: HashMap<String, String>` (person key to
  username) to `BulkPrepareRequest`. Reject the request with
  `invalid_username_override` if it names a key that is not in `person_keys`.

### Dependencies

None

### Deliverables

- The changes above.
- Unit tests for `validate_override`: accepted plain name, diacritics
  normalized, empty input rejected, disallowed characters rejected.
- Integration tests: an accepted override is the reserved username; a
  colliding override gets a suffix or `username_collision` per the existing
  rule; a policy-breaking override fails that row and the rest of the batch
  still prepares; an override for a person with an active reservation leaves
  the reservation unchanged.

### Acceptance Criteria

- Without `usernames`, preparation behaves exactly as before.
- An override can never bypass the policy or the uniqueness check.
- The crate compiles and the tests pass under the SOP validation stage.

## USX-107 — Editable username in the preview

Expose the override from USX-106 in the bulk preview.

Files you may change: `web/src/lib/api.ts` (the `bulkPrepare` request type),
`web/src/app/admin/family-members/page.tsx` (the preview row and `prepare`
only).

- For preparable rows, render the Proposed username cell as a text input
  prefilled with `proposed_username`, labelled "Username for *Name*". Other rows
  keep the read-only text.
- Track edits per person key. On prepare, send `usernames` with only the
  selected rows whose value differs from the proposal.
- Highlight rows whose reasons include an ambiguous-name code so the admin
  knows which usernames to check.
- The prepare confirmation lists how many usernames were changed.

### Dependencies

- USX-106

### Deliverables

- The input, request, and confirmation changes above.

### Acceptance Criteria

- An unedited prepare sends no `usernames` field.
- An edited, selected row sends its new username; an edited, unselected row
  sends nothing.
- A per-row rejection from the server appears on that row in the batch status
  table.
- The TypeScript build passes.

## USX-108 — End-to-end coverage

Update the Playwright specs for names, the picker, and username editing.

Files you may change: `web/e2e/bulk-onboarding.spec.ts`,
`web/e2e/admin-accounts.spec.ts`.

### Dependencies

- USX-105
- USX-107

### Deliverables

- Batch status table shows names with the key as secondary text.
- Accounts list shows the linked person's name; `subject @ issuer` is in the
  Actions drawer.
- Link picker: searching an unaccented name finds the accented synthetic
  person; an already-linked result is disabled; the confirmation names the
  person; typed text alone cannot submit.
- Username edit: an edited username is sent on prepare and shown in the batch
  status; an unedited prepare sends no override.
- The mobile viewport still renders labelled cards.

### Acceptance Criteria

- The specs pass against stubbed responses.
- No real family name or address appears in the specs.

## USX-109 — Prepare the pull request

Write `docs/reports/user-setup-ux-pr.md` from the validation evidence on disk:
what changed, the extended privacy decision (names and branches in admin
views), the new admin person-search route and its authorization, the username
override rules, the verification results, and the later work listed in the
Summary. Use `NOT RUN` for anything the evidence does not contain.

### Dependencies

- USX-102
- USX-108

### Deliverables

- `docs/reports/user-setup-ux-pr.md`.

### Acceptance Criteria

- Every acceptance criterion above is listed with its evidence or `NOT RUN`.
- No real name, email, or credential appears.
- Nothing is merged.
