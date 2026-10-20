# TRAN-003 Acceptance Matrix

**Canonical plan:** `docs/plans/PLAN-TRAN-RBAC.md`
**Follow-up plan:** `docs/plans/PLAN-TRAN-RBAC-INTEGRATION-FOLLOWUP.md`
**Base:** `origin/main` at `2034d34`
**Data:** synthetic only; no real Head of Family appointment exists.

## A — Preflight, branch reconciliation, and contract mapping

**Result: PASS**

- Follow-up work is on `feat/tran-rbac-integration`, based on merged `origin/main`
  containing TRAN-002 (PR #6) and the RBAC foundation (PR #7).
- The historical `plan-tran-rbac` run was not reopened; the new SOP plan ID is
  `plan-tran-rbac-integration-followup`.
- No duplicate genealogy schema was added. TRAN-002 tables remain the only
  person/relationship/assertion/evidence model.
- `git ls-files docs/tran/` is empty; private inventory remains untracked.

### Route, permission, and data-owner matrix

| Browser operation | Rust route | Permission | Data owner | Expected denial |
| --- | --- | --- | --- | --- |
| Health probe | `GET /api/health` | Public, non-sensitive | None | N/A |
| Upload/search/Q&A | `POST /api/query`, `POST /api/documents/ingest`, `GET /api/documents/{source}/status` | Viewer for query/status; editor for ingest; corpus grant required | RAG DB | 401 anonymous, 403 ungranted/low role |
| Own account | `GET /api/account` | Any authenticated account | RAG DB identity tables | 401 missing/invalid session |
| Role change | `POST /api/admin/roles` | Admin | RAG DB | 403 non-admin, 409 last-admin |
| Provision/deactivate | `POST /api/admin/users`, `POST /api/admin/users/{subject}/deactivate` | Admin | RAG DB | 403 non-admin |
| Record list | `GET /api/genealogy/records` | Viewer + corpus grant | Genealogy DB | 401/403 before query |
| Person/relationship/citations | `GET /api/genealogy/person/{key}`, `GET /api/genealogy/relationship/{from}/{to}` | Viewer + corpus grant | Genealogy DB | 401/403/404 |
| Head of Family view | `GET /api/genealogy/head-of-family` | Viewer + corpus grant | Genealogy DB | 401/403 before query |
| Pending nominations | `GET /api/genealogy/nominations` | Viewer + corpus grant | Genealogy DB | 401/403 |
| Nominate | `POST /api/genealogy/nominations` | Editor/admin + corpus grant | Genealogy DB | 403 viewer |
| Approve/reject | `POST /api/genealogy/nominations/{id}` | Admin | Genealogy DB | 403 viewer/editor |
| Suspend/revoke | `POST /api/genealogy/appointments/{id}` | Admin | Genealogy DB | 403 viewer/editor |

## B — Corpus isolation and default-deny

**Result: PASS**

- `replace_source` deletes by `(corpus, source)`; regression test
  `tests/storage_interop.rs::replace_source_is_isolated_by_corpus` passes.
- Corpus-scoped reads use validated corpus identities; legacy NULL rows are not
  matched by a bound corpus.
- API/proxy tests deny anonymous, viewer upload, forged/expired claims, and
  client-injected identity headers before side effects.
- Health exposes only availability, not document content.

## C — OIDC authentication, sessions, and roles

**Result: PASS (local synthetic provider); production provider setup remains an
explicit deployment prerequisite.**

- Custom HS256 verification replaced by `ring`-based asymmetric JWKS
  verification (`RS256/RS384/RS512/PS256/PS384/PS512/ES256`). Symmetric JWKs and
  unknown algorithms are rejected.
- OIDC authorization-code + PKCE login, state/nonce checks, secure HttpOnly
  session cookie, session expiry, sign-out, and CSRF double-submit are
  implemented.
- Accounts bind to immutable issuer + subject; roles and corpus grants are loaded
  from the database, never from token claims.
- All authenticated users may read their own account; role/security
  administration remains admin-only.
- Provisioning, deactivation, role changes, and corpus grants are audited.
- The last active administrator cannot be revoked; concurrent revocations leave
  one active admin.
- Tests: OIDC verifier unit tests, `tests/identity_last_admin.rs`, real OIDC
  browser login, proxy RBAC/CSRF tests.

## D — Authorized genealogy read APIs

**Result: PASS**

- `GET /api/genealogy/records`, `/head-of-family`, `/person/{key}`, and
  `/relationship/{from}/{to}` read the separate genealogy database through the
  guarded `GENEALOGY_DATABASE_URL`; there is no RAG fallback.
- Responses use stable person keys and evidence IDs/citations and distinguish
  verified, unverified, and needs-review evidence.
- Authorization and corpus-grant checks run before opening the genealogy DB.
- Tests: real API integration in `web/e2e/genealogy-api.spec.ts`; synthetic
  record/relationship reads; anonymous and viewer-denial assertions.

## E — Head of Family workflow

**Result: PASS (synthetic only; no real appointment).**

- `genealogy/migrations/0006_head_of_family.sql` adds nominations, appointments,
  one-active-per-branch partial unique index, evidence JSONB snapshots, and an
  append-only audit table.
- Approval reuses TRAN-002's pure eligibility engine inside a transaction,
  serializes on the branch with an advisory lock, and records evidence/audit
  atomically.
- Tests: `tests/genealogy_appointments.rs` covers eligible approval, ineligible
  rejection (missing living evidence), revocation, and concurrent approvals
  yielding at most one active appointment.
- Viewer/editor cannot approve; the family title never grants application
  permissions. Succession remains an unresolved family-policy decision.

## F — Browser and user-flow integration

**Result: PASS (synthetic).**

- Real OIDC sign-in/sign-out is exercised in `web/e2e/auth.spec.ts`.
- The Head of Family page has real nomination, approval/rejection, and revocation
  controls using stable IDs, CSRF headers, awaited responses, and server-confirmed
  reloads.
- Real UI flow in `web/e2e/head-of-family-real.spec.ts`: admin signs in, nominates
  the synthetic person, approves, sees the appointment, revokes, and sees the
  designation disappear.
- Presentation-only tests remain in `web/e2e/experience.spec.ts`; direct API and
  proxy parity tests remain in `web/e2e/proxy-rbac.spec.ts`.

## G — End-to-end security, privacy, and closure

**Result: PASS locally; GitHub Actions verification is performed on the PR.**

- `cargo fmt --check`, `cargo build --locked`, `cargo test --locked`,
  `cargo clippy --locked --all-targets -- -D warnings`, `make rust-check`,
  `make web-check`, and `make web-e2e` pass.
- Both migration families apply cleanly to disposable local databases; no RAG or
  private genealogy database is reset by tests.
- `git diff --check` passes. `git ls-files docs/tran/` is empty and
  `git check-ignore -v docs/tran/` confirms the private path is ignored.
- No real genealogy record is promoted or appointed; all fixture names are
  synthetic.

## Deployment prerequisites (not implementation failures)

- A real OIDC provider, TLS termination, secure production cookies, and the
  initial-admin bootstrap must be configured before any public deployment.
- A real Head of Family appointment requires verified third-branch mapping,
  first-born male parentage/birth order, recent living-status evidence, original
  source provenance, and explicit family approval.
- Succession after a deceased, unknown, or disputed first-born male is a separate
  family-policy decision and is intentionally not implemented as a default.
