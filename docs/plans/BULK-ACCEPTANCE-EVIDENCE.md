# BULK-001…005 — Acceptance Evidence

**Plan ID:** `plan-tran-bulk-onboarding`
**Branch:** `sop/plan-tran-bulk-onboarding`
**Status:** implementation, Rust acceptance, web typecheck/build, and the full
Playwright E2E suite are complete. CI is the authoritative repeatable gate.

This note is public-repository safe. All rows and counts below are synthetic. It
contains no real names, addresses, birth dates, tokens, or private source hashes.

## Synthetic acceptance matrix

| Case | Expected | Evidence |
| --- | --- | --- |
| Verified living with cited evidence | `eligible_for_invite` | `tests/onboarding_bulk.rs`, candidate unit tests |
| Deceased / no living evidence | `not_eligible` | `tests/onboarding_bulk.rs` |
| Disputed living evidence | `needs_review` | `tests/onboarding_bulk.rs` |
| Unverified living evidence | `not_eligible` | `tests/onboarding_bulk.rs` |
| Ambiguous single-token name | `needs_review` | candidate unit tests, `tests/onboarding_bulk.rs` |
| Minor / guardian review | `needs_review` | candidate unit tests, `tests/onboarding_bulk.rs` |
| Already linked | `already_linked` | `tests/onboarding_bulk.rs` |
| Already invited | `invited` | `tests/onboarding_bulk.rs` |
| Already prepared | `already_prepared` | batch unit/integration tests |
| Missing verified recipient | eligible and preparable, blocked from send | dry-run unit tests, `tests/onboarding_bulk.rs` |
| Username collision | deterministic distinct reservations | `tests/onboarding_bulk.rs` |
| Duplicate batch submission | one batch, one prepared item per person | `tests/onboarding_bulk.rs` |
| Concurrent identical submissions | one batch, one reservation per person | `tests/onboarding_bulk.rs` |
| Partial failure | successful row preserved, failing row reported | `tests/onboarding_bulk.rs` |
| Revoked evidence after preparation | send/prepare revalidation blocks the person | candidate unit tests, `tests/onboarding_bulk.rs` |
| Unconfigured adapter | `not_configured`, no invitation created | delivery unit tests, `tests/onboarding_bulk.rs` |
| Fake adapter, explicit approval | exactly one send per prepared person | `tests/onboarding_bulk.rs` |
| Repeat send | no second invitation | `tests/onboarding_bulk.rs` |
| Token storage | only a 64-hex hash is stored | `tests/onboarding_bulk.rs`, existing invitation tests |
| Viewer/non-admin | 403 | `tests/onboarding_admin_api.rs`, `web/e2e/bulk-onboarding.spec.ts` |
| CSRF | mutation without token is rejected by the proxy | existing `web/e2e/proxy-rbac.spec.ts`, new bulk spec |
| Cross-issuer privilege transfer | unchanged; identity link remains issuer/subject scoped | existing `tests/identity_cross_issuer.rs` |

## Commands executed locally

```
cargo fmt --all
cargo clippy --locked --all-targets -- -D warnings     # PASS
cargo test --locked                                     # PASS (all test binaries)
cargo test --locked --test onboarding_bulk              # PASS
PATH=<temporary Node 22>/bin:$PATH make check           # PASS
PATH=<temporary Node 22>/bin:$PATH make web-e2e         # PASS (65/65)
```

The integration suite ran against a disposable PostgreSQL 16 cluster on
`127.0.0.1:5434` using synthetic databases only. No real database was written.
A temporary Node 22 toolchain was extracted under `/tmp` for the web gates; it is
not part of the repository.

## Browser gate

`make web-e2e` passed 65 tests, including:

- `e2e/bulk-onboarding.spec.ts` — admin preview, two-person selection, explicit
  preparation, idempotent retry, missing-recipient send block, verified-recipient
  recording, explicitly approved fake send, and no token in the DOM;
- `e2e/bulk-onboarding.spec.ts` — viewer denied the page and the API;
- `e2e/bulk-onboarding.spec.ts` — a bulk mutation without CSRF is rejected by the
  proxy;
- the existing auth, experience, family-tree, genealogy, Head of Family,
  proxy-RBAC, and RAG suites.

The harness uses `ONBOARDING_MAIL_ADAPTER=fake`, which performs no network I/O.
The Rust integration suite separately proves the default unconfigured adapter
fails closed before creating an invitation.

### Harness note

`scripts/seed-genealogy-e2e.sql` now checks the verified-assertion evidence rule
immediately (`SET CONSTRAINTS ALL IMMEDIATE`). The fixture deletes and recreates
its source document inside one transaction; a deferred check can otherwise see
the pre-cleanup evidence state on a reused disposable database.

## Security properties

- Preview and dry-run issue no write SQL.
- Preparation does not create an OIDC user, role, corpus grant, link, or email.
- Invitation creation is behind a separate explicit `approve: true` request.
- Server-side revalidation runs before reservation and again before sending.
- A revoked or disputed living assertion blocks preparation and sending.
- Recipient addresses must be explicitly verified by an admin; they are never
  inferred from genealogy documents.
- Invitation tokens are random, expiring, single-use, and stored only as hashes.
- Bulk responses never include a token or a recipient address.
- An unconfigured adapter returns `not_configured` before any invitation is
  created and never claims delivery.
- `FakeAdapter` performs no network I/O.

## Production configuration dependencies

1. **Delivery adapter.** Implement and approve a real provider adapter behind the
   `MailAdapter` trait. The current configuration accepts only `unconfigured`
   (default) and `fake`.
2. **Activation URL.** Set `ONBOARDING_ACTIVATION_URL` to the deployed activation
   route (default `/activate`).
3. **Send throttle.** Set `ONBOARDING_SEND_INTERVAL_MS` for production throttling
   (default `0` for tests).
4. **OIDC activation and review.** The identity provider and the existing reviewed
   person-link flow remain the only path to account activation. No bulk step
   provisions credentials.

## Remaining approval boundary

The branch is review-ready but unmerged. No real invitation was sent, no real
account was activated, and no private genealogy record was modified.
