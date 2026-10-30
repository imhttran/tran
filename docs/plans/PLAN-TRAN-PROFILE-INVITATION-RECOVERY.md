# Tran: Profile invitation recovery

## Project

Recover the superseded profile-invitation effort and implement the complete,
server-authorized individual invitation flow for the Tran family archive.

## Summary

The superseded plan `docs/plans/tran-person-profile-invitation-plan.md` was
archived as `SUPERSEDED` because its Phase 2 was falsely marked complete without
any repository change. Its plan document, reports, task attempts, and archive are
preserved. This plan replaces only the unfinished work with new task ids.

The implementation must:

- let a system administrator or an authorized family head invite one eligible
  living person from that person's profile;
- derive family-head authority only from the current approved
  `head_of_family_appointment` plus the reviewed account-person link and the
  appointment's explicit `branch_key`; a title, name, label, nomination, or
  client-supplied role is never authority;
- reuse the existing invitation, delivery, person-link, eligibility, username,
  and identity-provider services;
- store draft contact details with restricted access and mask them outside the
  owning administrator's view;
- save a "Needs contact" state without sending, and send only after an explicit
  review action;
- prevent duplicate invitations and accounts;
- add a minimal activation/welcome flow that preserves reviewed linking;
- add additive migrations only, with no changes to existing rows or defaults.

### Execution rules

- A task is complete only when it produces concrete code and tests, or when a
  test demonstrates that the required behavior already exists. A no-op,
  `changes_expected:false`, or a prose-only claim is not completion evidence.
- Do not add dependencies. Reuse the existing Rust, Axum, Next.js, PostgreSQL,
  and identity-provider stack.
- Read only the files a task names, at most once each. Do not attempt to compile,
  test, lint, migrate, or run the web app; the SOP validation stage and the
  operator run those commands.
- Tests use synthetic people, example.test addresses, a fake mail adapter, and a
  disposable genealogy database. Never send a real invitation or mutate a live
  person, account, appointment, or grant.
- Preserve `docs/plans/tran-person-profile-invitation-plan.md`,
  `docs/reports/phase-1-trace-and-define.md`, and every file under
  `.agent-sdlc/`.
- Do not use `docs/reports/` for required deliverables; SOP excludes that
  directory from mutation evidence. Use `docs/architecture/`.

## PIR-001 — Complete prerequisite trace and authority decision

Complete the unfinished trace from the superseded plan and record the exact
authority rule before any invitation code is enabled.

Files to read, at most once each: `README.md`,
`docs/plans/tran-person-profile-invitation-plan.md`,
`docs/reports/phase-1-trace-and-define.md`, `src/identity.rs`,
`src/onboarding/invitation.rs`, `src/onboarding/delivery.rs`,
`src/onboarding/link.rs`, `src/onboarding/candidate.rs`,
`src/onboarding/username.rs`, `src/genealogy/appointments.rs`,
`src/genealogy/eligibility.rs`, `src/api.rs` (invitation and appointment
handlers only), `web/src/components/FamilyProfileView.tsx`, and
`web/src/lib/api.ts` (profile and invitation types only).

Write `docs/architecture/PROFILE-INVITATION-PREREQUISITES.md` with these
sections:

- `## Repository baseline` — the commit or working-tree state traced and the
  files inspected.
- `## Profile surface` — the profile route and component, the API response that
  supplies the person, and where an authorized invitation panel can mount.
- `## Invitation lifecycle` — creation, pending/reused invitation, send,
  resend, revoke, accept, expire, and delivery-state paths, with file paths and
  function names.
- `## Delivery channels` — the verified adapter(s), the configuration keys (names
  only), and whether SMS exists. Do not assume SMS.
- `## Eligibility and review blockers` — the existing living/identity rules and
  the warning codes that must be translated to plain language.
- `## Person-link review` — the reviewed link path and why invitation authority
  must not approve or replace a link.
- `## Username policy` — the exact normalization function and whether it
  lowercases and strips Vietnamese accents while preserving display-name
  accents.
- `## Activation path` — the identity-provider entry, account setup, and the
  existing route or missing route for a welcome screen.
- `## Family-head authority` — the appointment fields, active-status rule,
  branch scope, linked-account requirement, and the exact server rule.
- `## Appointment scope decision` — whether the appointment model reliably
  identifies an approved head, a linked account, and the target scope. If it
  does not, state the exact blocker and the smallest additive schema or service
  change required. Do not invent authority.
- `## Smallest change list` — the files and behaviors for PIR-002 through
  PIR-007.

The report must be evidence-based: every traced area names a file path and a
function/route. Mark anything not inspected as `unverified`; do not invent it.

### Dependencies

None

### Deliverables

- `docs/architecture/PROFILE-INVITATION-PREREQUISITES.md`.

### Acceptance Criteria

- Every required section is present and names concrete repository paths.
- The family-head authority rule uses the approved appointment's `branch_key`
  and the reviewed account-person link, and never a name, label, nomination, or
  client-supplied role.
- The appointment-scope decision is explicit: it either confirms the model can
  identify the approved head, linked account, and target scope, or records the
  exact blocker and the smallest corrective change.
- The delivery section states which channels exist and does not promise SMS.
- The username section records the actual normalization behavior.

### Discovery iterations

- 24

## PIR-002 — Family-admin scope resolver and authorization tests

Implement the server-side family-admin rule independently of HTTP handlers so
every invitation operation can reuse it.

Files to read, at most once each:
`docs/architecture/PROFILE-INVITATION-PREREQUISITES.md`,
`src/onboarding/mod.rs`, `src/onboarding/account_map.rs`,
`src/onboarding/link.rs`, `src/genealogy/appointments.rs`,
`src/genealogy/query.rs` (person/assertion reads only), and `src/identity.rs`.

Create `src/onboarding/family_admin.rs` and declare it in
`src/onboarding/mod.rs`. It must expose a single checked decision, for example
`authorize_person_invitation(client, account, target_person_key)`, that:

- returns allow for a system `admin` identity;
- otherwise resolves the caller's reviewed `onboarding_person_account_link`
  from the account id and identity fingerprint;
- resolves the caller's current `head_of_family_appointment` with
  `status = 'active'` and `current_eligibility = Eligible`;
- requires the appointment's `branch_key` to equal the target person's current
  verified branch claim, or the target person to be the caller's own linked
  person;
- refuses a missing link, no active appointment, an ineligible appointment, a
  different branch, an ambiguous branch, a revoked appointment, or a revoked
  link;
- never grants the system `admin` role or any other role;
- runs the link and appointment checks in the same read transaction as the
  caller's operation when the caller supplies a transaction-compatible client.

Add unit tests in the module for allowed admin, allowed family head in the same
branch, allowed self, wrong branch, unrelated person, revoked appointment,
revoked link, missing link, and ineligible appointment.

### Dependencies

PIR-001

### Deliverables

- `src/onboarding/family_admin.rs` and its `mod.rs` declaration.
- Unit tests in that module.

### Acceptance Criteria

- The rule is server-side and uses the exact appointment branch and reviewed
  link; no title/name/client-role path exists.
- System admin is allowed; a family head is allowed only for the appointment's
  branch or their own linked person.
- Every refusal is distinct enough for the API to return a safe plain-language
  reason.
- Revoked or ineligible authority cannot authorize a later operation.

### Discovery iterations

- 20

## PIR-003 — Contact draft and invitation preparation API

Add the restricted contact draft and the prepare/preview operation for one
person. Saving or previewing must not send a message or mark an invitation as
sent.

Files to read, at most once each: `src/onboarding/invitation.rs`,
`src/onboarding/delivery.rs`, `src/onboarding/family_admin.rs`,
`src/onboarding/candidate.rs`, `src/onboarding/username.rs`,
`src/api.rs` (the invitation handlers and adjacent request structs), and the
latest migration number under `genealogy/migrations/`.

Add one additive migration using the next free number (currently `0033`) that
creates a restricted draft-contact table keyed to a person, for example
`onboarding_invitation_contact_draft(person_id, email, created_by, updated_by,
created_at, updated_at)`. It must not alter existing tables or defaults. Add the
migration to the test migration lists that already enumerate migrations.

Create `src/onboarding/profile_invitation.rs` and declare it in
`src/onboarding/mod.rs`. It must provide:

- `read_invitation_state(client, account, person_key)` returning the person's
  eligibility decision, current invitation/delivery state, linked-account
  status, proposed username, and a safe plain-language blocker list;
- `save_contact_draft(client, account, person_key, email)` that validates and
  stores the address only in the restricted draft table and does not create an
  invitation or call any delivery adapter;
- `prepare_invitation(client, account, person_key)` that rechecks the family
  admin rule, eligibility, reviewed link state, duplicate invitation, and draft
  contact, then returns a review payload with the person, masked contact,
  proposed username, and Member (`viewer`) access. It must not send.

Add API routes under `/api/family/persons/{person_key}/invitation`:

- `GET` for `read_invitation_state`;
- `PUT`/`POST` for the contact draft;
- `POST /prepare` for the review payload.

Use the existing authorization service; do not add a browser-supplied role. No
unmasked contact may appear in the response, logs, or errors.

Add integration tests in `tests/profile_invitation.rs` using
`tests/support/genealogy_isolation.rs`, a synthetic person, a synthetic family
head appointment and link, and `example.test` addresses. Cover missing contact,
saved draft not sending, duplicate pending invitation reuse, linked account,
wrong-branch family head, system admin, and masked summary.

### Dependencies

PIR-002

### Deliverables

- The additive migration and its test-list registration.
- `src/onboarding/profile_invitation.rs` and `mod.rs` declaration.
- The three API routes and safe error mapping.
- `tests/profile_invitation.rs`.

### Acceptance Criteria

- Missing contact can be saved as a draft and read back; no invitation row is
  created and no delivery adapter is invoked.
- Preparation returns the exact person, masked contact, proposed username, and
  Member access; it never returns an unmasked address.
- A linked account is reported as linked, not invitable.
- A duplicate pending invitation is reused, not duplicated.
- Wrong-branch, unlinked, revoked, and ineligible callers are denied.
- The migration is additive and does not renumber or modify an existing file.

### Discovery iterations

- 24

## PIR-004 — Send, resend, revoke, and invitation status API

Add the explicit send and lifecycle operations for one profile invitation,
reusing the existing invitation and mail-adapter services.

Files to read, at most once each: `src/onboarding/profile_invitation.rs`,
`src/onboarding/invitation.rs`, `src/onboarding/delivery.rs`,
`src/onboarding/family_admin.rs`, `src/api.rs` (invitation handlers only), and
`tests/profile_invitation.rs`.

Extend `src/onboarding/profile_invitation.rs` with:

- `send_invitation(client, adapter, account, person_key)` that rechecks family
  admin authority, eligibility, reviewed link, duplicate invitation, and draft
  contact inside one operation; creates at most one hashed invitation; hands the
  activation link only to the configured adapter; records the actual delivery
  receipt/state; and never reports `sent` unless the adapter confirms it;
- `resend_invitation` and `revoke_invitation` that preserve the same authority
  and duplicate rules;
- `read_invitation_status` returning pending/sent/queued/failed/expired/accepted
  and the safe provider error code, never a token, address, or credential.

Add individual routes under `/api/family/persons/{person_key}/invitation` for
send, resend, revoke, and status. Use `MailAdapter`/`FakeAdapter` from the
existing delivery module; do not create a new mail provider. A delivery error
must not create a second invitation or claim success.

Extend `tests/profile_invitation.rs` with a fake adapter and deterministic
delivery receipts: successful send, adapter failure, retry without duplicate,
resend, revoke, expired invitation, accepted invitation, wrong-branch denial,
and no token/address in status or errors.

### Dependencies

PIR-003

### Deliverables

- The send/resend/revoke/status service functions and routes.
- The synthetic integration tests.

### Acceptance Criteria

- A completed explicit send creates exactly one invitation and reports the
  adapter's actual outcome.
- A delivery failure leaves the invitation recoverable and does not create a
  second invitation or claim success.
- Resend and revoke are authorized by the same family-admin rule.
- Status distinguishes pending, sent/queued, failed, expired, accepted, and
  unconfigured delivery without exposing secrets.
- No token or full contact address appears in a response, error, or log.

### Discovery iterations

- 24

## PIR-005 — Profile invitation panel

Add the invitation action and status panel to the person profile, limited to
the server-authorized admin/family-admin caller.

Files to read, at most once each: `web/src/components/FamilyProfileView.tsx`,
`web/src/app/family/[personKey]/page.tsx`, `web/src/lib/api.ts` (profile and
invitation types/client only), `web/src/app/globals.css` (profile panel classes
only), and the PIR-003/PIR-004 API shapes.

Create `web/src/components/ProfileInvitationPanel.tsx` and render it from the
profile page or `FamilyProfileView` without exposing it to an unauthorized
viewer.

- Fetch the invitation state for the person; render nothing for a caller the API
  does not authorize.
- Show **Account linked** instead of the invite action for a linked account.
- Show **Invite to family archive**, a contact form, plain-language eligibility
  blockers, the proposed username, and Member access.
- Save a draft as **Needs contact** without sending.
- Show a review step with the person, destination, and Member access, then an
  explicit **Send invitation** action; send only after that action.
- Show the real status for pending, sent/queued, failed, expired, and accepted,
  with authorized resend/revoke actions.
- Never display an unmasked contact address, token, credential, or raw warning
  code.

Add the API client types/functions to `web/src/lib/api.ts`. Add a focused
Playwright spec `web/e2e/profile-invitation.spec.ts` using stubbed responses for
admin, family-head, ordinary-member, linked-account, missing-contact, send, and
error states. Use synthetic names and example.test addresses.

### Dependencies

PIR-004

### Deliverables

- `web/src/components/ProfileInvitationPanel.tsx`.
- Profile-page wiring and API client additions.
- Focused Playwright coverage.

### Acceptance Criteria

- The panel is visible only when the API authorizes the caller.
- A linked account shows **Account linked**, not an invite action.
- Missing contact can be saved without sending; only the explicit send action
  triggers delivery.
- The UI shows the actual invitation/delivery status and plain-language blockers.
- No contact address, token, or credential is rendered.

### Discovery iterations

- 24

## PIR-006 — Activation and welcome flow

Add the minimal invitation welcome and activation flow, preserving reviewed
person linking and the existing identity provider.

Files to read, at most once each: `src/onboarding/invitation.rs`,
`src/onboarding/link.rs`, `src/api.rs` (invitation and session handlers only),
`web/src/app/signin/page.tsx`, `web/src/lib/api.ts`, and the existing identity
provider configuration in `src/oidc.rs`.

Add:

- a public invitation validation/accept endpoint that returns the intended
  person, invitation state, and whether a reviewed link already exists; it must
  not automatically approve or replace a person link;
- a minimal `web/src/app/activate/[token]/page.tsx` welcome screen that validates
  the invitation, continues through the existing identity provider, shows the
  intended person with a **This is not me** path, and shows pending review when
  activation or linking is incomplete;
- clear expired, revoked, already-accepted, wrong-person, and
  interrupted-activation states;
- no duplicate account creation; reuse the existing account setup path.

Accepting an invitation must assign only Member (`viewer`) access plus the
existing required archive grants through the approved activation path; it must
not grant admin or family-admin authority.

Add focused Rust tests for token validation/accept/expired/revoked/accepted and
focused Playwright coverage for the welcome states using synthetic tokens.

### Dependencies

PIR-004

### Deliverables

- The activation API and welcome page.
- Focused Rust and browser tests.

### Acceptance Criteria

- A valid invitation leads to the existing identity-provider setup and a
  person-confirmation step.
- Expired, revoked, accepted, and wrong-person invitations render clear next
  steps.
- The flow never auto-links a person or grants elevated access.
- An interrupted activation can be resumed without creating a duplicate account.

### Discovery iterations

- 24

## PIR-007 — Acceptance evidence and pull request report

Record the implementation evidence and prepare the pull request report.

Files to read, at most once each: the reports and changed files from PIR-001
through PIR-006, `README.md`, and `docs/architecture/PROFILE-INVITATION-PREREQUISITES.md`.

Write `docs/architecture/PROFILE-INVITATION-ACCEPTANCE.md` containing:

- the implemented surfaces and exact file paths;
- the family-admin authority rule and its denial cases;
- the contact/privacy boundary;
- the activation and identity-provider behavior;
- the commands actually run and their results, using `NOT RUN` for anything not
  run;
- known limitations, including whether email is the only delivery channel;
- no real name, address, token, or credential.

Write the PR body to `docs/reports/profile-invitation-pr.md`. The operator opens
the PR after the SOP tasks; this task does not push or merge.

### Dependencies

- PIR-005
- PIR-006

### Deliverables

- `docs/architecture/PROFILE-INVITATION-ACCEPTANCE.md`.
- `docs/reports/profile-invitation-pr.md`.

### Acceptance Criteria

- Every acceptance criterion from PIR-001 through PIR-006 has evidence or
  `NOT RUN`.
- The report states the actual validation results; it does not claim delivery
  without observed adapter evidence.
- No secret, token, full address, or real family data is present.

### Discovery iterations

- 12
