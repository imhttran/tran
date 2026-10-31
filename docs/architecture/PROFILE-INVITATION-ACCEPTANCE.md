# Profile Invitation — Acceptance Evidence (PIR-007 / follow-ups in PRs #136, #137, and #138)

Evidence for the recovery plan `docs/plans/PLAN-TRAN-PROFILE-INVITATION-RECOVERY.md`
(tasks PIR-001 through PIR-007) and for the follow-up that makes the flow usable by
a real new family member. Every command listed under **Verification** was run in
this working tree; anything not run is marked `NOT RUN`. All data used in tests is
synthetic (`example.test` addresses, `synthetic-*` and `e2e-*` keys). No real name,
address, token, hash, or credential appears in this document.

## Follow-up changes (PRs #136–#138)

The first delivery was safe but not usable end to end. This follow-up closes five
gaps; each is verified below.

1. **Authorization.** A reviewed own-person link is no longer an invitation
   permission. Only a system admin or an active, eligible Head of Family acting
   inside the appointment's branch may invite, and that branch rule is the only
   way a family head may invite their own person.
2. **Email delivery.** A provider-backed `http` adapter now exists and is selected
   by the existing `ONBOARDING_MAIL_ADAPTER` configuration. It posts one JSON
   message to `ONBOARDING_MAIL_URL` and reports provider acceptance only. Its
   contract and the compatibility finding are in
   [`PROFILE-INVITATION-MAIL-ADAPTER.md`](PROFILE-INVITATION-MAIL-ADAPTER.md).
3. **New-user activation.** Signing in from an invitation creates the identity
   account, so no manual CLI provisioning is needed. The account starts with **no**
   role and **no** archive grant.
4. **Person-link review.** Token possession records a link **request**; an
   administrator makes the explicit reviewed-link decision in the console (or
   through the admin API). An existing link is never replaced.
5. **Verification.** Focused Rust and browser coverage exercises unauthorized
   self-invitation, family-head scope, provisioning, reviewed linking, delivery
   failures, the real OIDC round trip, and the real-API browser flow.

### Access timing (latest revision)

Member access follows the **approved link**, not the invitation token:

- Signing in or accepting creates only the identity row, one accepted invitation,
  and one pending link request.
- Until an administrator approves, the account has no role and no corpus grant.
  `/api/genealogy/tree`, `/api/query`, and the other archive reads are refused with
  `403`; the family profile is closed because the person is not linked.
- Approval writes the reviewed link and then grants the viewer role plus the
  archive corpus grant. Both steps are audited and idempotent, and the token is
  never consulted for access.

The real-API suites assert the `403` before approval and the `200` after it.

## Implemented surfaces

| Surface                                                                 | Path                                                                                                   |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Family-admin authority rule and denial cases                            | `src/onboarding/family_admin.rs`                                                                       |
| Contact draft, preparation, send, resend, revoke, status                | `src/onboarding/profile_invitation.rs`                                                                 |
| Invitation welcome, acceptance, and reviewed-link requests              | `src/onboarding/activation.rs`                                                                         |
| Member provisioning (identity row) and approved member access           | `src/onboarding/provisioning.rs`                                                                       |
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
  provisions the identity row when the return path is an invitation, then
  establishes the session. A failed provision returns to the welcome screen
  without a session. No application account needs to exist beforehand; the
  identity provider still authenticates the person.
- `POST /api/invitations/{token}/provision` requires a verified identity and a
  usable invitation. It creates or reactivates the identity row only.
- `POST /api/invitations/{token}/accept` consumes the invitation and records one
  pending link request. It refuses expired, revoked, already-accepted (by another
  account), person-already-linked, and account-linked-elsewhere cases.
- `POST /api/admin/family-members/link-requests/{id}/decision` applies the
  reviewed-link policy: approve calls `link_reviewed`, writes its audit row, and
  then grants the viewer role and the archive corpus grant by opaque account id;
  reject records the decision with no link and no grant. A repeat decision with
  the same outcome is reported as already decided, and a repeated approval
  re-grants idempotently.

## Verification

Commands run from the repository root with disposable test databases
(`DATABASE_URL` and `GENEALOGY_DATABASE_URL` pointed at `*_test` copies, and
`REQUIRE_TEST_DATABASE=1` where applicable).

| Command                                                                                           | Result                                                     |
| ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| `cargo build --locked`                                                                            | PASS                                                       |
| `cargo test --locked`                                                                             | PASS                                                       |
| `cargo fmt --check`                                                                               | PASS                                                       |
| `cargo clippy --locked --all-targets -- -D warnings`                                              | PASS                                                       |
| `sop validate`                                                                                    | PASS — BUILD, UNIT_TEST, LINT                              |
| `cargo test --locked --test profile_invitation`                                                   | PASS — 23 tests                                            |
| `cargo test --locked --test onboarding_activation`                                                | PASS — 9 tests                                             |
| `cargo test --locked --test onboarding_provisioning`                                              | PASS — 1 test                                              |
| `cargo test --locked --test onboarding_bulk`                                                      | PASS — 1 test                                              |
| `make web-check`                                                                                  | PASS — TypeScript and production build                     |
| `PLAYWRIGHT_ARGS="e2e/activation.spec.ts e2e/profile-invitation.spec.ts"` (stubbed)               | PASS — 16 tests                                            |
| `PLAYWRIGHT_ARGS="e2e/invitation-activation-real.spec.ts e2e/invitation-oidc-round-trip.spec.ts"` | PASS — 6 tests (5 real-API flow + 1 real OIDC round trip)  |
| Full `make web-e2e` (entire suite)                                                                | NOT RUN — only the focused specs were run                  |
| A real mail provider delivery                                                                     | NOT RUN — no provider endpoint or credential is configured |

The real-API suites prove, against a running Axum API and PostgreSQL: a public
preview; an unauthenticated provision is refused; signing in creates an account
with no role and no archive access (`/api/genealogy/tree` is `403`); acceptance
records a request and leaves the family profile closed; the admin console
approval grants viewer plus the corpus grant (`/api/genealogy/tree` becomes
`200`); and a second invitee completes the flow from the welcome page.

The OIDC round-trip suite additionally proves the case application-level
provisioning alone cannot: the test issuer signs in a **subject the application
has never seen**, the browser completes the authorization-code redirect chain,
the callback provisions the account, the invitation survives the round trip, and
acceptance reaches pending review.

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
| A new invitee is provisioned without operator action                                | `onboarding_provisioning.rs`; `invitation-activation-real.spec.ts` (`signing in creates an account without archive access`); `invitation-oidc-round-trip.spec.ts`                                                                |
| No archive access before the link is approved                                       | `onboarding_provisioning.rs` (no role/grant after provisioning); both real specs assert `/api/genealogy/tree` is `403` before approval and `200` after                                                                           |
| Member access only; no elevated role                                                | `onboarding_provisioning.rs` asserts exactly `[viewer]` and no `admin`/`editor` after approval; the real specs assert the same through `/api/account`                                                                            |
| Token possession never approves a person link                                       | `onboarding_activation.rs`; `invitation-activation-real.spec.ts` (`accepting records a link request instead of approving it`)                                                                                                    |
| An administrator decides the reviewed link                                          | `activation.rs` (`decide_link_request`); `onboarding_activation.rs`; the admin console path in the real spec                                                                                                                     |
| An existing link is never replaced                                                  | `onboarding_activation.rs` (`a_person_linked_to_another_account_is_never_reclaimed`, `an_account_linked_to_another_person_is_refused`)                                                                                           |
| Expired, revoked, accepted, and wrong-person render clear next steps                | `activation.spec.ts`; `onboarding_activation.rs`                                                                                                                                                                                 |

## Known limitations

- **No mail provider is configured in this environment.** The `http` adapter is
  implemented and unit-tested against a local one-shot HTTP server, but no real
  delivery was attempted. The endpoint contract is documented in
  [`PROFILE-INVITATION-MAIL-ADAPTER.md`](PROFILE-INVITATION-MAIL-ADAPTER.md). A
  deployment needs `ONBOARDING_MAIL_ADAPTER=http`,
  `ONBOARDING_MAIL_URL=https://...` (loopback `http://` is allowed for local
  testing), and optionally `ONBOARDING_MAIL_BEARER`. A 2xx means provider
  acceptance only, never inbox delivery. Email remains the only channel.
- **No existing provider is compatible.** `PASSWORD_RESET_URL`, the only outbound
  mail provider contract in the repository, receives `{issuer, subject}` and
  resolves the address itself; it cannot deliver this message. A generic endpoint
  is not automatically compatible with a mail provider: point the adapter at a
  small gateway, or add a provider-specific adapter.
- **The identity provider must authenticate the person.** The application needs no
  pre-existing account, and the OIDC round trip is verified, but the family's IdP
  still has to issue a token for that person (self-signup or an operator-created
  IdP account). That is an external prerequisite.
- **Provisioning trusts the verified identity, not the invited address.** The
  invitation token proves control of the delivered link; the identity provider
  proves control of the signed-in identity. The IdP address is not compared with
  the invited address, so the administrator's link review is the human gate. The
  account holds no access until that review.
- **The admin decision is an API plus a panel on `/admin/family-members`.** There
  is no dedicated review queue page.
- **Expired invitations are not resent automatically.** An administrator must
  resend or issue a new invitation.
- **The full `make web-e2e` suite was not run**; only the focused invitation specs
  were executed.
- **The plan document was updated** to remove the obsolete self-invitation
  permission. The operator authorized the change and it was accepted through
  `sop reconcile "docs/plans/PLAN-TRAN-PROFILE-INVITATION-RECOVERY.md"
--accept-changed PIR-002`; SOP reports `RECONCILE_CLEAN` with all seven tasks
  satisfied and the execution history preserved.

## Deployment prerequisites

These are external to the repository. None was changed or exercised here.

1. **Identity-provider registration.** The deployed provider is Keycloak
   (`http://127.0.0.1:8180/realms/tran`). A read-only probe of the realm's login
   page found no registration link, which is Keycloak's standard signal that
   `User registration` is off. An invited person **without** a Keycloak account
   therefore cannot create one today. To enable self-service, an operator opens
   the Keycloak admin console for realm `tran`, enables _Realm settings → Login →
   User registration_ (optionally _Email as username_ and _Verify email_, which
   needs SMTP), and confirms the `tran-web` client accepts the flow. Otherwise the
   operator creates each user under _Users → Add user_ and sets a credential. The
   discovery document's `registration_endpoint` is OAuth **client** registration,
   not user sign-up, and does not help here. A third option keeps registration
   off: set `ONBOARDING_KEYCLOAK_LOGINS` and the API creates the login when the
   invitation is sent ([`KEYCLOAK-INVITATION-LOGINS.md`](../operations/KEYCLOAK-INVITATION-LOGINS.md)).
2. **Compatible mail delivery.** Set `ONBOARDING_MAIL_ADAPTER=http` and
   `ONBOARDING_MAIL_URL` to an endpoint that accepts the contract in
   [`PROFILE-INVITATION-MAIL-ADAPTER.md`](PROFILE-INVITATION-MAIL-ADAPTER.md).
   No provider is configured, and `PASSWORD_RESET_URL` is not compatible. A 2xx is
   provider acceptance only.
3. **Publicly reachable activation URL.** `ONBOARDING_ACTIVATION_URL` must be an
   absolute public URL. It is unset here, so the default `/activate` is used.

### Activation-link shape (fixed in #140)

The delivered link was built as `{ONBOARDING_ACTIVATION_URL}?token=<token>`
(`activation_link` in `src/onboarding/delivery.rs`), but the welcome screen is the
path route `/activate/[token]`. A queried `/activate?token=...` did not match that
route, so an emailed link would have returned 404 even with a correct public host.

PR #140 fixes it: `activation_link` now emits `{base}/{token}`, trims trailing
slashes from the configured base, and percent-encodes the token as one path
segment. Send, resend, and the bulk send path share the builder; unit, database,
and browser regressions cover the generated URL. Prerequisite 3 below is therefore
only the public host configuration.

### Distinguishing the two accounts

- **Identity-provider account (Keycloak):** authenticates the person. It is
  self-registered (disabled here), created by an operator, or created by the API
  at send time when `ONBOARDING_KEYCLOAK_LOGINS` is on.
- **Tran application account (identity database `users` row):** created
  automatically by `/api/invitations/{token}/provision` on the first successful
  sign-in and audited. No operator action is needed. It carries no role and no
  archive grant until an administrator approves the reviewed link.

The real OIDC round-trip test proves the second point with a subject the
application had never seen.
