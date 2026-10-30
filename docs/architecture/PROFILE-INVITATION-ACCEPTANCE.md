# Profile Invitation — Acceptance Evidence (PIR-007 / PR #136 follow-up in PR #137)

Evidence for the recovery plan `docs/plans/PLAN-TRAN-PROFILE-INVITATION-RECOVERY.md`
(tasks PIR-001 through PIR-007) and for the follow-up that makes the flow usable by
a real new family member. Every command listed under **Verification** was run in
this working tree; anything not run is marked `NOT RUN`. All data used in tests is
synthetic (`example.test` addresses, `synthetic-*` and `e2e-*` keys). No real name,
address, token, hash, or credential appears in this document.

## Follow-up changes (PR #136 follow-up, delivered in PR #137)

The first delivery was safe but not usable end to end. This follow-up closes five
gaps; each is verified below.

1. **Authorization.** A reviewed own-person link is no longer an invitation
   permission. Only a system admin or an active, eligible Head of Family acting
   inside the appointment's branch may invite, and that branch rule is the only
   way a family head may invite their own person.
2. **Email delivery.** A provider-backed `http` adapter now exists and is selected
   by the existing `ONBOARDING_MAIL_ADAPTER` configuration. It posts one JSON
   message to `ONBOARDING_MAIL_URL` and reports provider acceptance only.
3. **New-user activation.** Signing in from an invitation provisions the identity
   account with the viewer role and the archive corpus grant, audited and
   idempotent, so no manual CLI provisioning is needed.
4. **Person-link review.** Token possession records a link **request**; an
   administrator makes the explicit reviewed-link decision in the console (or
   through the admin API). An existing link is never replaced.
5. **Verification.** Focused Rust and browser coverage exercises unauthorized
   self-invitation, family-head scope, provisioning, reviewed linking, delivery
   failures, and the critical flow against the real API.

## Implemented surfaces

| Surface                                                                 | Path                                                                                                   |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Family-admin authority rule and denial cases                            | `src/onboarding/family_admin.rs`                                                                       |
| Contact draft, preparation, send, resend, revoke, status                | `src/onboarding/profile_invitation.rs`                                                                 |
| Invitation welcome, acceptance, and reviewed-link requests              | `src/onboarding/activation.rs`                                                                         |
| Member provisioning (viewer + corpus grant)                             | `src/onboarding/provisioning.rs`                                                                       |
| Delivery adapters (fake, http, unconfigured) and per-invitation receipt | `src/onboarding/delivery.rs`                                                                           |
| Draft contact table (additive)                                          | `genealogy/migrations/0033_onboarding_invitation_contact_draft.sql`                                    |
| Per-invitation delivery receipt columns (additive)                      | `genealogy/migrations/0034_onboarding_invitation_delivery.sql`                                         |
| Reviewed-link request table (additive)                                  | `genealogy/migrations/0035_onboarding_link_request.sql`                                                |
| Routes and safe error mapping                                           | `src/api.rs`                                                                                           |
| Profile invitation panel                                                | `web/src/components/ProfileInvitationPanel.tsx`                                                        |
| Activation and welcome screen                                           | `web/src/app/activate/[token]/page.tsx`                                                                |
| Post-sign-in provisioning and return path                               | `web/src/app/api/auth/login/route.ts`, `web/src/app/api/auth/callback/route.ts`, `web/src/lib/oidc.ts` |
| Admin pending-link review panel                                         | `web/src/components/PendingLinkReviews.tsx`, `web/src/app/admin/family-members/page.tsx`               |
| API client types and functions                                          | `web/src/lib/api.ts`                                                                                   |

### Routes

| Method       | Route                                                   | Authorization                                            |
| ------------ | ------------------------------------------------------- | -------------------------------------------------------- |
| `GET`        | `/api/family/persons/{person_key}/invitation`           | `Operation::OnboardingInvite` plus the family-admin rule |
| `PUT`/`POST` | `/api/family/persons/{person_key}/invitation`           | same; saves a draft contact only                         |
| `POST`       | `/api/family/persons/{person_key}/invitation/prepare`   | same; returns a review payload                           |
| `POST`       | `/api/family/persons/{person_key}/invitation/send`      | same; explicit send only                                 |
| `POST`       | `/api/family/persons/{person_key}/invitation/resend`    | same; revokes then reissues                              |
| `POST`       | `/api/family/persons/{person_key}/invitation/revoke`    | same; idempotent                                         |
| `GET`        | `/api/family/persons/{person_key}/invitation/status`    | same; read-only                                          |
| `GET`        | `/api/invitations/{token}`                              | public; the link is the credential, read-limited         |
| `POST`       | `/api/invitations/{token}/provision`                    | verified identity + a usable invitation                  |
| `POST`       | `/api/invitations/{token}/accept`                       | verified identity + a usable invitation                  |
| `GET`        | `/api/admin/family-members/link-requests`               | `Operation::OnboardingLink`                              |
| `POST`       | `/api/admin/family-members/link-requests/{id}/decision` | `Operation::OnboardingLink`                              |

## Family-admin authority rule

`authorize_person_invitation_in` in `src/onboarding/family_admin.rs` decides, in
this order:

1. The target person must exist (`UnknownTargetPerson`).
2. A system `admin` role is allowed (`system_admin`).
3. The caller must have a reviewed link (`MissingLink`, `RevokedLink`).
4. A Head of Family appointment must be active and eligible
   (`IneligibleAppointment`, `RevokedAppointment`, `IneligibleHolder`).
5. The target's single verified branch claim must equal the appointment branch
   (`WrongBranch`, `AmbiguousBranch`, `UnrelatedPerson`).

There is no separate self permission: a linked caller with no appointment cannot
invite even their own person, and a family head whose own claim leaves the
appointment branch loses the authority. The rule grants no role and issues SELECT
statements only.

## Contact and privacy boundary

- A proposed address is stored only in `onboarding_invitation_contact_draft`,
  written by `save_contact_draft`; `onboarding_recipient_contact` stays the
  explicitly verified-recipient table used by the bulk flow.
- Only a masked address leaves the module. Send and status responses carry
  `masked_contact`; they never carry the stored address.
- The plaintext token exists only in the outbound message body. The database
  stores the SHA-256 hash; no response, error, log, or status view carries it.
- A delivery failure is stored as a stable code (`not_configured`,
  `provider_failed`, `provider_unreachable`, `provider_status_<n>`, ...) and
  mapped to prose. No error carries the provider URL, the bearer, the address, or
  the message body.

## Delivery behavior

- `send_invitation` fails closed before any write when the configured adapter is
  not configured.
- One explicit send creates at most one active invitation. A pending invitation
  already `sent` or `queued` is reported as it stands; it is never duplicated or
  silently re-sent.
- `resend_invitation` revokes the previous pending row first.
- The recorded state is the adapter's receipt: `sent` only when the adapter
  confirms, `queued` when the provider accepts it for later delivery, `failed`
  with a safe code on an error. A failure leaves the row recoverable.
- `http` adapter: posts `{to, subject, body}` to `ONBOARDING_MAIL_URL` with an
  optional bearer, refuses redirects, and treats a 2xx answer as **provider
  acceptance**. It does not claim inbox delivery.

## Activation, provisioning, and reviewed linking

- `GET /api/invitations/{token}` returns the intended person, the invitation
  state, and the link-request state. Read-only.
- The OIDC login route stores a validated same-origin `returnTo`; the callback
  provisions the member when the return path is an invitation, then establishes
  the session. A failed provision returns to the welcome screen without a session.
- `POST /api/invitations/{token}/provision` requires a verified identity and a
  usable invitation. It creates or reactivates the identity row and grants exactly
  the viewer role and the archive corpus grant (audited, idempotent). It never
  grants editor or admin.
- `POST /api/invitations/{token}/accept` consumes the invitation and records one
  pending link request. It refuses expired, revoked, already-accepted (by another
  account), person-already-linked, and account-linked-elsewhere cases.
- `POST /api/admin/family-members/link-requests/{id}/decision` applies the
  reviewed-link policy: approve calls `link_reviewed` and writes its audit row;
  reject records the decision with no link. A repeat decision with the same
  outcome is reported as already decided.

## Verification

Commands run from the repository root with disposable test databases
(`DATABASE_URL` and `GENEALOGY_DATABASE_URL` pointed at `*_test` copies, and
`REQUIRE_TEST_DATABASE=1` where applicable).

| Command                                                                             | Result                                                                                                                  |
| ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `cargo build --locked`                                                              | PASS                                                                                                                    |
| `cargo test --locked`                                                               | PASS                                                                                                                    |
| `cargo fmt --check`                                                                 | PASS                                                                                                                    |
| `cargo clippy --locked --all-targets -- -D warnings`                                | PASS                                                                                                                    |
| `sop validate`                                                                      | PASS — BUILD, UNIT_TEST, LINT                                                                                           |
| `cargo test --locked --test profile_invitation`                                     | PASS — 23 tests                                                                                                         |
| `cargo test --locked --test onboarding_activation`                                  | PASS — 9 tests                                                                                                          |
| `cargo test --locked --test onboarding_provisioning`                                | PASS — 1 test                                                                                                           |
| `cargo test --locked --test onboarding_bulk`                                        | PASS — 1 test                                                                                                           |
| `make web-check`                                                                    | PASS — TypeScript and production build                                                                                  |
| `PLAYWRIGHT_ARGS="e2e/activation.spec.ts e2e/profile-invitation.spec.ts"` (stubbed) | PASS — 16 tests                                                                                                         |
| `PLAYWRIGHT_ARGS="e2e/invitation-activation-real.spec.ts"` (real API + browser)     | PASS — 5 tests                                                                                                          |
| Full `make web-e2e` (entire suite)                                                  | NOT RUN — only the focused specs were run                                                                               |
| A real mail provider delivery                                                       | NOT RUN — no provider credentials in this environment                                                                   |
| The OIDC authorization-code round trip in a browser                                 | NOT RUN — the harness sets a session cookie; the callback's provision call is covered by review, not by that round trip |

The real-API spec proves, against a running Axum API and PostgreSQL: a public
preview; an unauthenticated provision is refused; provisioning grants viewer plus
the corpus grant; acceptance records a request without a link and the family
profile stays closed; the admin console approves the link; the family profile then
opens; and a second invitee completes the flow from the welcome page itself.

## Acceptance criteria coverage

| Criterion                                                                           | Evidence                                                                                                                                                                                                                         |
| ----------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Only admin or in-scope family head may invite                                       | `family_admin.rs` unit tests; `profile_invitation.rs` (`wrong_branch_family_head_is_denied`, `a_linked_caller_without_an_appointment_cannot_invite_themselves`, `a_family_head_may_invite_themselves_only_through_their_branch`) |
| A draft contact is saved without sending                                            | `profile_invitation.rs` (`missing_contact_saves_as_draft_without_sending`, `saved_draft_does_not_send`)                                                                                                                          |
| One explicit send creates one invitation and reports the real receipt               | `profile_invitation.rs` (`explicit_send_reports_delivery_and_never_duplicates`)                                                                                                                                                  |
| A delivery failure does not duplicate or claim success                              | `profile_invitation.rs` (`adapter_failure_stays_recoverable_then_retry_succeeds`, `queued_delivery_is_not_reported_as_delivered`); `delivery.rs` adapter tests                                                                   |
| Resend and revoke use the same authority                                            | `profile_invitation.rs` (`resend_revokes_previous_and_issues_one_link`, `revoke_reports_state_and_is_idempotent`, `wrong_branch_family_head_cannot_send_or_revoke`)                                                              |
| Status distinguishes pending, sent, queued, failed, expired, accepted, unconfigured | `ProfileDeliveryState`; `profile_invitation.rs`; `profile-invitation.spec.ts`                                                                                                                                                    |
| No token or full address in a response, error, log, or rendered page                | Rust test assertions; both stubbed and real browser specs                                                                                                                                                                        |
| A new invitee is provisioned without operator action                                | `onboarding_provisioning.rs`; `invitation-activation-real.spec.ts` (`signing in provisions Member access without an operator`)                                                                                                   |
| Member access only; no elevated role                                                | `onboarding_provisioning.rs` asserts exactly `[viewer]` and no `admin`/`editor`; the real spec asserts the same through `/api/account`                                                                                           |
| Token possession never approves a person link                                       | `onboarding_activation.rs`; `invitation-activation-real.spec.ts` (`accepting records a link request instead of approving it`)                                                                                                    |
| An administrator decides the reviewed link                                          | `activation.rs` (`decide_link_request`); `onboarding_activation.rs`; the admin console path in the real spec                                                                                                                     |
| An existing link is never replaced                                                  | `onboarding_activation.rs` (`a_person_linked_to_another_account_is_never_reclaimed`, `an_account_linked_to_another_person_is_refused`)                                                                                           |
| Expired, revoked, accepted, and wrong-person render clear next steps                | `activation.spec.ts`; `onboarding_activation.rs`                                                                                                                                                                                 |

## Known limitations

- **No mail provider is configured in this environment.** The `http` adapter is
  implemented and unit-tested against a local one-shot HTTP server, but no real
  delivery was attempted. A deployment needs `ONBOARDING_MAIL_ADAPTER=http`,
  `ONBOARDING_MAIL_URL=https://...` (loopback `http://` is allowed for local
  testing), and optionally `ONBOARDING_MAIL_BEARER`. A 2xx means provider
  acceptance only, never inbox delivery. Email remains the only channel.
- **Provisioning trusts the verified identity, not the invited address.** The
  invitation token proves control of the delivered link; the identity provider
  proves control of the signed-in identity. The IdP address is not compared with
  the invited address, so the administrator's link review is the human gate. An
  account created this way holds only viewer plus the archive corpus grant.
- **The admin decision is an API plus a panel on `/admin/family-members`.** There
  is no dedicated review queue page.
- **Expired invitations are not resent automatically.** An administrator must
  resend or issue a new invitation.
- **The OIDC round trip is not exercised by the browser suite.** The harness sets
  the session cookie directly; the callback's provisioning call is reviewed but
  not covered end to end by a browser test.
- **The full `make web-e2e` suite was not run**; only the focused invitation specs
  were executed.
- **The original plan text for PIR-002 mentions a self-invitation permission.**
  The operator instruction for this follow-up removed it; the code and this report
  describe the stricter rule that is now implemented.
