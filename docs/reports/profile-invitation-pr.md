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

## Validation

Run with disposable `*_test` databases:

- `cargo build --locked` — PASS
- `cargo test --locked` — PASS
- `cargo fmt --check` — PASS
- `cargo clippy --locked --all-targets -- -D warnings` — PASS
- `sop validate` — PASS (BUILD, UNIT_TEST, LINT)
- `cargo test --locked --test profile_invitation` — PASS, 23 tests
- `cargo test --locked --test onboarding_activation` — PASS, 9 tests
- `cargo test --locked --test onboarding_provisioning` — PASS, 1 test
- `cargo test --locked --test onboarding_bulk` — PASS
- `make web-check` — PASS (TypeScript + production build)
- `PLAYWRIGHT_ARGS="e2e/activation.spec.ts e2e/profile-invitation.spec.ts"` — PASS, 16 tests (stubbed UI states)
- `PLAYWRIGHT_ARGS="e2e/invitation-activation-real.spec.ts"` — PASS, 5 tests (real API + browser)

Not run: the full `make web-e2e` suite, the OIDC authorization-code round trip in a
browser, and any delivery through a real mail provider (no provider credentials in
this environment).

## Known limitations

- Email is the only delivery channel, and no provider is configured here. A
  deployment needs `ONBOARDING_MAIL_ADAPTER=http`, `ONBOARDING_MAIL_URL=https://…`
  and an optional bearer. A 2xx is provider acceptance, not inbox delivery.
- Provisioning trusts the verified identity, not the invited address, so the
  administrator's link review is the human gate.
- The admin decision is an API plus a panel on `/admin/family-members`; there is no
  dedicated queue page.
- Expired invitations are not resent automatically.
- Browser tests stub the panel states; the real-API spec covers the integrated
  flow but not the OIDC redirect itself.

Full evidence: `docs/architecture/PROFILE-INVITATION-ACCEPTANCE.md`.
