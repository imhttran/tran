## Summary

Lets a system administrator or an authorized family head invite one eligible
living person from that person's own profile, with a restricted contact draft, an
explicit send, resend/revoke/status, and a minimal welcome/activation flow. This
is the recovery implementation for
`docs/plans/PLAN-TRAN-PROFILE-INVITATION-RECOVERY.md` (PIR-001 … PIR-007).

## What changed

### Authority

- `src/onboarding/family_admin.rs` — one rule for every invitation operation:
  system admin, active and eligible Head of Family for the target branch, or the
  caller's own linked person. Every denial case is distinct and safe. The rule
  grants no role.

### Contact draft, preparation, and delivery

- `src/onboarding/profile_invitation.rs` — `read_invitation_state`,
  `save_contact_draft`, `prepare_invitation`, `send_invitation`,
  `resend_invitation`, `revoke_invitation`, `read_invitation_status`.
- A draft address is stored only in `onboarding_invitation_contact_draft`; only a
  masked address appears in a response.
- One explicit send creates at most one active invitation and records the mail
  adapter's actual receipt (`sent`, `queued`, or `failed` with a safe code). An
  unconfigured adapter fails closed before any write. A resend revokes the
  previous pending link first, so only one link is ever live.
- Migrations `0033_onboarding_invitation_contact_draft.sql` and
  `0034_onboarding_invitation_delivery.sql` are additive only.

### API

- Per-person routes for state, draft, prepare, send, resend, revoke, and status
  under `/api/family/persons/{person_key}/invitation`.
- Public `GET /api/invitations/{token}` previews the intended person and state.
- Authenticated `POST /api/invitations/{token}/accept` links only the caller's
  account to the invited person, only when no link exists, then consumes the
  one-time invitation. It never replaces a link and never grants a role.

### Web

- `web/src/components/ProfileInvitationPanel.tsx` renders nothing until the API
  authorizes the caller; shows **Account linked** for a linked person; saves a
  draft as **Needs contact** without sending; requires a review step before the
  explicit **Send invitation**; shows the real delivery status and prose
  blockers; offers resend/revoke.
- `web/src/app/activate/[token]/page.tsx` is the welcome screen: intended person,
  **This is not me**, continue through the existing OIDC provider, explicit
  accept, and clear expired/revoked/already-used/wrong-person/pending-review
  states.
- The OIDC login route honors a validated same-origin `returnTo`, so the invitee
  returns to the welcome screen after sign-in.

## Follow-up in this PR

Makes the flow usable by a real new family member:

- **Authorization.** Removed the independent "own linked person" permission. Only
  a system admin or an active, eligible Head of Family inside the appointment's
  branch may invite; that branch rule is also the only way a family head may
  invite their own person.
- **Real email adapter.** `ONBOARDING_MAIL_ADAPTER=http` posts one JSON message to
  `ONBOARDING_MAIL_URL` (optional `ONBOARDING_MAIL_BEARER`), refuses redirects, and
  reports provider acceptance only. `fake` stays for tests; both fail closed when
  unconfigured.
- **Self-service activation.** Signing in from an invitation provisions the
  identity account with the viewer role and the archive corpus grant (audited and
  idempotent). No CLI provisioning is required, and no elevated role is granted.
- **Reviewed linking.** Acceptance records a link request; an administrator
  approves or rejects it in the family-members console (or the admin API). Token
  possession never approves a link, and an existing link is never replaced.
- **Real-flow coverage.** A new browser/API spec drives provisioning, acceptance,
  the admin approval in the console, and the resulting family-profile access
  against the running API and PostgreSQL.

## Latest revision

- **Access follows the approved link, not the token.** Signing in and accepting
  create only the identity row, the accepted invitation, and a pending link
  request. Viewer and the archive corpus grant are written at approval. Until
  then `/api/genealogy/tree` and the other archive reads return `403`.
- **OIDC round trip verified.** A new browser spec signs in a subject the
  application has never seen through the real authorization-code redirect chain;
  the callback provisions the account, the invitation survives sign-in, and
  acceptance reaches Pending review.
- **Mail adapter contract documented.** `docs/architecture/PROFILE-INVITATION-MAIL-ADAPTER.md`
  defines the request/response contract and records that the only existing
  outbound provider (`PASSWORD_RESET_URL`) is not compatible: it receives
  `{issuer, subject}` and resolves the address itself.
- **Plan updated.** The recovery plan's PIR-002 text no longer describes a
  self-invitation permission. SOP reports the source change against the executed
  task; accepting it is an operator decision.

## Validation

Run with disposable `*_test` databases:

- `cargo build --locked` — PASS
- `cargo test --locked` — PASS
- `cargo fmt --check` — PASS
- `cargo clippy --locked --all-targets -- -D warnings` — PASS
- `sop validate` — PASS (BUILD, UNIT_TEST, LINT)
- `cargo test --locked --test profile_invitation` — PASS, 23 tests
- `cargo test --locked --test onboarding_activation` — PASS, 9 tests
- `cargo test --locked --test onboarding_provisioning` — PASS, 1 test (no access on sign-in; viewer + corpus only after approval)
- `cargo test --locked --test onboarding_bulk` — PASS
- `make web-check` — PASS (TypeScript + production build)
- `PLAYWRIGHT_ARGS="e2e/activation.spec.ts e2e/profile-invitation.spec.ts"` — PASS, 16 tests (stubbed UI states)
- `PLAYWRIGHT_ARGS="e2e/invitation-activation-real.spec.ts e2e/invitation-oidc-round-trip.spec.ts"` — PASS, 6 tests (real API + real OIDC round trip)

Not run: the full `make web-e2e` suite and delivery through a real mail provider
(no provider endpoint or credential in this environment).

## Known limitations

- No mail provider is configured here. Required: `ONBOARDING_MAIL_ADAPTER=http`,
  `ONBOARDING_MAIL_URL=https://…` (loopback `http://` for local testing), optional
  `ONBOARDING_MAIL_BEARER`, and `ONBOARDING_ACTIVATION_URL` for the public link.
  A 2xx is provider acceptance only. The existing `PASSWORD_RESET_URL` contract is
  not compatible.
- The family's identity provider must authenticate the person. The application
  needs no pre-existing account (verified by the round-trip test), but the IdP
  still has to issue a token for them.
- Provisioning trusts the verified identity, not the invited address, so the
  administrator's link review is the human gate.
- The admin decision is an API plus a panel on `/admin/family-members`; no
  dedicated queue page.
- Expired invitations are not resent automatically.
- Browser suites stub some panel states; the real-API and round-trip specs cover
  the integrated flow, but the full `make web-e2e` suite was not run.

Full evidence: `docs/architecture/PROFILE-INVITATION-ACCEPTANCE.md` and
`docs/architecture/PROFILE-INVITATION-MAIL-ADAPTER.md`.
