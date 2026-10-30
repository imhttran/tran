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

## Validation

Run with disposable `*_test` databases:

- `cargo build --locked` — PASS
- `cargo test --locked` — PASS
- `cargo fmt --check` — PASS
- `cargo clippy --locked --all-targets -- -D warnings` — PASS
- `cargo test --locked --test profile_invitation` — PASS, 21 tests
- `cargo test --locked --test onboarding_activation` — PASS, 7 tests
- `cargo test --locked --test onboarding_bulk` — PASS
- `sop validate` — PASS (BUILD, UNIT_TEST, LINT)
- `make web-check` — PASS (TypeScript + production build)
- `API_PORT=8081 PLAYWRIGHT_ARGS="e2e/activation.spec.ts e2e/profile-invitation.spec.ts" ./scripts/test-db.sh exec -- ./scripts/web-e2e.sh` — PASS, 14 tests

Not run: the full `make web-e2e` suite, and any delivery through a real mail
provider.

## Known limitations

- Email is the only delivery channel; only the synthetic adapter is implemented,
  so a real deployment must supply a provider adapter before invitations can be
  delivered.
- The invitee's identity account must already be provisioned; activation never
  creates an account, and an unprovisioned identity sees **Pending review**.
- Acceptance grants no role; Member access is the family-read capability of a
  linked account.
- Expired invitations are not resent automatically.
- Browser tests stub the API; the Rust integration suites cover the real
  database paths.

Full evidence: `docs/architecture/PROFILE-INVITATION-ACCEPTANCE.md`.
