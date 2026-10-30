# Profile Invitation — Acceptance Evidence (PIR-007)

Evidence for the recovery plan `docs/plans/PLAN-TRAN-PROFILE-INVITATION-RECOVERY.md`
(tasks PIR-001 through PIR-006). Every command listed under **Verification** was
run in this working tree; anything not run is marked `NOT RUN`. All data used in
tests is synthetic (`example.test` addresses and `synthetic-*` person keys). No
real name, address, token, hash, or credential appears in this document.

## Implemented surfaces

| Surface | Path |
| --- | --- |
| Family-admin authority rule and denial cases | `src/onboarding/family_admin.rs` |
| Contact draft, preparation, send, resend, revoke, status | `src/onboarding/profile_invitation.rs` |
| Invitation welcome and activation | `src/onboarding/activation.rs` |
| Delivery adapter contract and per-invitation receipt | `src/onboarding/delivery.rs` |
| Draft contact table (additive) | `genealogy/migrations/0033_onboarding_invitation_contact_draft.sql` |
| Per-invitation delivery receipt columns (additive) | `genealogy/migrations/0034_onboarding_invitation_delivery.sql` |
| Routes and safe error mapping | `src/api.rs` |
| Self-service accept operation | `src/identity.rs` (`Operation::InvitationAccept`) |
| Profile invitation panel | `web/src/components/ProfileInvitationPanel.tsx` |
| Profile-page wiring | `web/src/components/FamilyProfileView.tsx` |
| API client types and functions | `web/src/lib/api.ts` |
| Activation and welcome screen | `web/src/app/activate/[token]/page.tsx` |
| Post-sign-in return path | `web/src/app/api/auth/login/route.ts`, `web/src/app/api/auth/callback/route.ts`, `web/src/lib/oidc.ts`, `web/src/lib/proxy-cookies.ts` |

### Routes

| Method | Route | Authorization |
| --- | --- | --- |
| `GET` | `/api/family/persons/{person_key}/invitation` | `Operation::OnboardingInvite` plus the family-admin rule |
| `PUT`/`POST` | `/api/family/persons/{person_key}/invitation` | same; saves a draft contact only |
| `POST` | `/api/family/persons/{person_key}/invitation/prepare` | same; returns a review payload |
| `POST` | `/api/family/persons/{person_key}/invitation/send` | same; explicit send only |
| `POST` | `/api/family/persons/{person_key}/invitation/resend` | same; revokes then reissues |
| `POST` | `/api/family/persons/{person_key}/invitation/revoke` | same; idempotent |
| `GET` | `/api/family/persons/{person_key}/invitation/status` | same; read-only |
| `GET` | `/api/invitations/{token}` | public; the link is the credential |
| `POST` | `/api/invitations/{token}/accept` | any authenticated account (`Operation::InvitationAccept`) |

## Family-admin authority rule

`authorize_person_invitation_in` in `src/onboarding/family_admin.rs` decides, in
this order:

1. The target person must exist (`UnknownTargetPerson`).
2. A system `admin` role is allowed (`system_admin`).
3. The caller must have a reviewed link (`MissingLink`, `RevokedLink`).
4. A Head of Family appointment must be active and eligible
   (`IneligibleAppointment`, `RevokedAppointment`, `IneligibleHolder`).
5. The target must be the caller's own linked person (`self_link`), inside the
   appointment's `branch_key` (`family_head_branch`), or refused as
   `WrongBranch`, `AmbiguousBranch`, or `UnrelatedPerson`.

The rule never grants a role and issues SELECT statements only. The same rule
authorizes read, draft, prepare, send, resend, revoke, and status: there is no
path where an unauthorized caller can read the draft contact or trigger a send.

## Contact and privacy boundary

- A proposed address is stored only in `onboarding_invitation_contact_draft`,
  written by `save_contact_draft`. Preparation and sending never write
  `onboarding_recipient_contact`, which remains the explicitly verified-recipient
  table used by the bulk flow.
- Only a masked address leaves the module (`mask_email`). The send and status
  responses carry `masked_contact`; they never carry the stored address.
- The plaintext invitation token exists only in the outbound message body. The
  database stores the SHA-256 hash (`token_hash`); no response, error, log line,
  or status view carries a token.
- A delivery failure is stored as a stable code (`not_configured`,
  `provider_failed`, `invalid_recipient`, `delivery_failed`, ...) in
  `last_delivery_error`. The API and the UI map it to prose; the raw code is not
  rendered.
- `send_invitation` and `resend_invitation` record `invited_email` on the
  invitation row for the delivery audit, matching the existing bulk send path.
  No read endpoint returns it.

## Delivery behavior

- `send_invitation` fails closed with `delivery_not_configured` before any write
  when the configured adapter is not configured.
- One explicit send creates at most one active invitation. A pending invitation
  whose delivery is already `sent` or `queued` is reported as it stands; it is
  never duplicated or silently re-sent.
- `resend_invitation` revokes the previous pending row first, so only one live
  link exists per person.
- The recorded state is the adapter's receipt: `sent` only when the adapter
  confirms (`accepted: true`), `queued` when it accepts the message for later
  delivery, `failed` with a safe code on a provider error. A failure leaves the
  row pending and recoverable; a retry revokes it and issues one replacement.
- `read_invitation_status` reports the lifecycle (`pending`, `accepted`,
  `revoked`, `expired`), the delivery state, the provider name, the safe error
  code, and the attempt count. A pending row past its expiry is reported as
  `expired` without a write.

## Activation and identity-provider behavior

- `GET /api/invitations/{token}` returns the intended person's display name and
  the invitation state. It is read-only: it never accepts, expires, or links.
- The welcome screen (`/activate/[token]`) shows the person, the Member access
  level, and two paths: continue through the existing OIDC provider, or
  "This is not me".
- The OIDC login route stores a validated same-origin `returnTo` path and the
  callback redirects there, so the invitee returns to the welcome screen.
- `POST /api/invitations/{token}/accept` writes exactly one reviewed link for the
  invited person and the caller's own account, then consumes the one-time
  invitation. It refuses:
  - `invalid_invitation` — unknown or blank token (no person probe);
  - `invitation_expired` / `invitation_revoked` — terminal states;
  - `invitation_accepted` — already consumed by another account;
  - `person_already_linked` — the person belongs to another account;
  - `wrong_person` — this account is already linked to a different person.
- If the invitation cannot be consumed after the link was written (a concurrent
  use of the same link), exactly that new link is revoked.
- Acceptance never creates an identity account and never writes a role. The
  linked account gains the existing family-read capability (Member access) and
  nothing else. An identity that has no provisioned account sees the
  "Pending review" state.

## Verification

Commands run from the repository root with disposable test databases
(`DATABASE_URL` and `GENEALOGY_DATABASE_URL` pointed at `*_test` copies, and
`REQUIRE_TEST_DATABASE=1`).

| Command | Result |
| --- | --- |
| `cargo build --locked` | PASS |
| `cargo test --locked` | PASS |
| `cargo fmt --check` | PASS |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS |
| `cargo test --locked --test profile_invitation` | PASS — 21 tests |
| `cargo test --locked --test onboarding_activation` | PASS — 7 tests |
| `cargo test --locked --test onboarding_bulk` | PASS — 1 test |
| `sop validate` | PASS — BUILD, UNIT_TEST, LINT (fmt, clippy) |
| `make web-check` | PASS — TypeScript and production build |
| `API_PORT=8081 PLAYWRIGHT_ARGS="e2e/activation.spec.ts e2e/profile-invitation.spec.ts" ./scripts/test-db.sh exec -- ./scripts/web-e2e.sh` | PASS — 14 tests |
| Full `make web-e2e` (entire suite) | NOT RUN — only the two focused specs were run |
| A real mail provider delivery | NOT RUN — only the synthetic adapter is implemented |

## Acceptance criteria coverage

| Criterion | Evidence |
| --- | --- |
| Authority rule allows system admin, branch family head, and self; refuses every other case | `src/onboarding/family_admin.rs` unit tests; `tests/profile_invitation.rs` (`wrong_branch_family_head_is_denied`, `unlinked_callers_are_denied`, `revoked_callers_are_denied`) |
| A draft contact is saved without sending | `tests/profile_invitation.rs` (`missing_contact_saves_as_draft_without_sending`, `saved_draft_does_not_send`) |
| One explicit send creates exactly one invitation and reports the real receipt | `tests/profile_invitation.rs` (`explicit_send_reports_delivery_and_never_duplicates`) |
| A delivery failure does not duplicate or claim success | `tests/profile_invitation.rs` (`adapter_failure_stays_recoverable_then_retry_succeeds`, `queued_delivery_is_not_reported_as_delivered`) |
| Resend and revoke use the same authority | `tests/profile_invitation.rs` (`resend_revokes_previous_and_issues_one_link`, `revoke_reports_state_and_is_idempotent`, `wrong_branch_family_head_cannot_send_or_revoke`) |
| Status distinguishes pending, sent, queued, failed, expired, accepted, unconfigured | `src/onboarding/profile_invitation.rs` (`ProfileDeliveryState`), `tests/profile_invitation.rs`, `web/e2e/profile-invitation.spec.ts` |
| No token or full address in a response, error, log, or rendered page | `tests/profile_invitation.rs` and `tests/onboarding_activation.rs` assertions; `web/e2e/profile-invitation.spec.ts` and `web/e2e/activation.spec.ts` token/address assertions |
| A valid invitation leads to the identity provider and a confirmation step | `web/e2e/activation.spec.ts` (`an anonymous visitor continues through the identity provider`, `a signed-in invitee accepts and is linked to Member access`) |
| Expired, revoked, accepted, and wrong-person render clear next steps | `web/e2e/activation.spec.ts` |
| Activation never auto-links or grants elevated access | `src/onboarding/activation.rs`; `tests/onboarding_activation.rs` (`an_account_linked_to_another_person_is_refused`, `a_person_linked_to_another_account_is_never_reclaimed`) |
| An interrupted activation can be resumed without a duplicate account | `tests/onboarding_activation.rs` (`a_valid_invitation_previews_and_activates_once` — repeat accept is an idempotent success with one link); account creation is never performed by activation |

## Known limitations

- **Email is the only delivery channel.** `Config::mail_adapter`
  (`src/config.rs`) implements only the synthetic `fake` adapter; any other value
  yields `UnconfiguredAdapter`, which fails closed. A production deployment needs
  a real provider adapter and its configuration before invitations can be
  delivered.
- **The invited account must already be provisioned.** Activation links an
  existing identity account; it never creates one. An identity without an active
  account sees "Pending review" until an administrator enables it.
- **Acceptance grants no role.** Member access is the family-read capability a
  linked account already has. Nothing in this flow grants `admin` or a
  Head-of-Family appointment.
- **Expired invitations are not resent automatically.** A family administrator
  must resend or issue a new invitation.
- **The public preview exposes the person's display name** to the holder of the
  link. That is required for the "This is not me" confirmation; no other person
  data is exposed.
- **The browser suites are stubbed.** `web/e2e/profile-invitation.spec.ts` and
  `web/e2e/activation.spec.ts` stub the API, so they prove the UI contract, not a
  live end-to-end delivery. The focused Rust integration suites exercise the real
  database paths for both modules.
