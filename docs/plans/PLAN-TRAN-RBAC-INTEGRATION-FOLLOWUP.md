# PLAN-TRAN-RBAC-INTEGRATION-FOLLOWUP — TRAN-003 post-merge completion

**Repository:** `https://github.com/imhttran/tran`
**Canonical requirements:** `docs/plans/PLAN-TRAN-RBAC.md`
**Plan ID:** `plan-tran-rbac-integration-followup`
**Base:** `origin/main` at `2034d34` (PR #7 and PR #8 merged)
**Status:** FOLLOW-UP REQUIRED — complete the outstanding TRAN-003 acceptance work

## Purpose

Implement the remaining TRAN-003 acceptance requirements without reopening the
historical `plan-tran-rbac` SOP run, recreating the merged TRAN-002 genealogy
model, or rewriting published history. This file is the SOP-executable delivery
plan; the canonical requirements remain in `docs/plans/PLAN-TRAN-RBAC.md`.

## Non-negotiable constraints

- Keep the RAG database and the separate genealogy database distinct. Never fall
  back from `GENEALOGY_DATABASE_URL` to `DATABASE_URL`.
- Reuse the merged TRAN-002 schema, provenance rules, review workflow, and pure
  eligibility function. Extend them additively only where a required API or
  appointment capability is absent.
- Use synthetic genealogy data only in tests, tracked documentation, and CI.
  Never commit private source material, real family names, hashes, or living-person
  details.
- Do not appoint a real Head of Family, merge the PR, deploy publicly, or infer a
  successor. Stop for explicit operator authorization at those boundaries.
- Do not weaken authentication, authorization, validation, or privacy checks to
  obtain a green result.

## Task 1 — `phase-0-followup-preflight`

**Objective:** Confirm the merged baseline and record the route, permission, and
data-owner matrix for the remaining work.

**Deliverables:**
- Public-safe preflight note recording `origin/main` SHA, the merged TRAN-002 and
  PR #7 surfaces, outstanding gaps, and environment limitations.
- Matrix mapping every browser genealogy/account operation to a Rust route,
  permission, database owner, and expected denial behavior.
- Confirmation that `git ls-files docs/tran/` is empty and the private inventory
  remains untracked.
- Confirmation that SOP validation uses Rust and Next.js commands, not Go defaults.

**Acceptance criteria:**
- The follow-up branch is based on merged `origin/main` and contains no duplicate
  genealogy schema.
- Every UI-requested endpoint is either implemented or listed as a planned task.
- Baseline checks and environment prerequisites are recorded.

## Task 2 — `phase-1-rbac-repair`

**Objective:** Verify and repair the merged corpus-isolation and default-deny
foundation.

**Deliverables:**
- Focused regression tests for same-filename replacement in two corpora.
- Tests proving that ungranted, anonymous, expired, and forged credentials fail
  before document staging, database writes, embeddings, or generation.
- Tests proving viewer/editor/admin permissions and client-injected headers cannot
  elevate access.
- Documentation of the local CLI trust boundary.

**Acceptance criteria:**
- Two corpora with the same filename remain independent on replace, reimport,
  status, search, expansion, and citation reads.
- An ungranted user cannot observe another corpus or its source status.
- Health remains public only for non-sensitive availability data.

## Task 3 — `phase-2-oidc-sessions`

**Objective:** Complete standards-based authentication, sessions, and role
administration.

**Deliverables:**
- Replace the custom HS256-only verifier with a maintained standards-compliant
  verifier supporting the approved provider's asymmetric JWKS algorithms. Parse
  JWK public keys, verify issuer, audience, expiry, signature, and subject, and fail
  closed on invalid or unavailable keys.
- Implement OIDC authorization-code + PKCE sign-in with state and nonce checks,
  secure HttpOnly/Secure/SameSite session cookies, session expiry/rotation, CSRF
  protection, and sign-out.
- Use a standards-compliant synthetic OIDC provider for automated tests. Leave the
  real provider as an explicit deployment prerequisite.
- Allow every authenticated user to read their own account; keep role changes and
  security administration admin-only.
- Bind accounts to immutable issuer + subject.
- Add audited user provisioning, deactivation, role changes, and corpus-grant
  management.
- Prevent removal of the last active administrator, including concurrent attempts.

**Acceptance criteria:**
- Wrong issuer/audience/algorithm, malformed JWK, forged/expired token, replayed
  state, invalid session, provider outage, client-supplied roles, and CSRF failures
  deny securely with no side effects.
- Role changes take effect without trusting stale token claims.
- No public deployment or real-family enrollment occurs without operator-configured
  OIDC, TLS, secure sessions, and initial-admin bootstrap.
- No authentication bypass exists for tests.

## Task 4 — `phase-3-genealogy-read-api`

**Objective:** Add authorized genealogy read APIs over the existing TRAN-002
genealogy database.

**Deliverables:**
- `GET /api/genealogy/records` and `GET /api/genealogy/head-of-family`, plus
  person, relationship, and citation-detail reads needed by the existing UI.
- Response contracts with stable person/record IDs, statuses, pagination, and
  evidence citations.
- Record-level authorization before any genealogy query or existence-sensitive
  response.
- Tests on a disposable genealogy database for verified, unverified, disputed,
  and missing evidence.

**Acceptance criteria:**
- Authorized synthetic users can distinguish verified relationships from unknown,
  disputed, or missing evidence.
- Anonymous and ungranted requests disclose neither records nor source existence,
  through direct Rust or the Next.js proxy.
- A missing or unusable genealogy database fails closed without falling back to
  the RAG database.
- No RAG tables or migrations are changed for genealogy reads.

## Task 5 — `phase-4-head-of-family`

**Objective:** Implement the audited, transactional Head of Family workflow using
the merged eligibility engine.

**Deliverables:**
- Additive genealogy migrations for nominations, one active appointment per
  branch, evidence snapshots, reviewer/approver identity, status, suspension,
  revocation, and append-only audit.
- Editor nomination, admin approval/rejection, and admin suspension/revocation
  routes matching the UI contract.
- Transactional approval that locks/serializes competing appointments, rechecks
  verified eligibility and living status, and records evidence plus audit.
- Explicit suspension/revocation behavior when living status becomes deceased,
  unknown, or disputed. Never auto-appoint a successor.

**Acceptance criteria:**
- A synthetic verified living first-born male in the documented third branch can be
  approved only by an admin, with evidence IDs and reviewer recorded.
- Missing, contested, deceased, younger, wrong-branch, wrong-gender, or
  incomplete-birth-order candidates are denied or reported undetermined.
- Two concurrent approvals yield at most one active appointment.
- Evidence changes or revoked living status cannot leave an unreviewed active
  title.
- Viewer/editor cannot approve or revoke; Head of Family never grants admin rights.
- Every mutation is audited and cannot be mistaken for a placeholder success.
- No real person is appointed.

## Task 6 — `phase-5-browser-integration`

**Objective:** Connect the Next.js experience to the real authenticated APIs.

**Deliverables:**
- Wire sign-in, sign-out, account menu, genealogy records, citation drill-down,
  nominations, appointments, and revocations to real API results.
- Use stable IDs, send CSRF headers on mutations, await server responses, and show
  success only after confirmation.
- Surface 401/403/409 and missing-evidence states without leaking tokens, cookies,
  database URLs, identity fields, private names, or source details.
- Browser integration tests against real Rust handlers and disposable databases.

**Acceptance criteria:**
- Playwright covers viewer, editor, admin, signed-out, expired, upload denial,
  reviewed record queries, nomination, approval, revocation, and unresolved
  evidence.
- Browser actions change persisted authorized server state.
- No Head of Family appears without a verified active appointment.

## Task 7 — `phase-6-acceptance-delivery`

**Objective:** Prove the follow-up meets TRAN-003A–G and open one new PR.

**Deliverables:**
- Acceptance matrix for TRAN-003A through TRAN-003G with PASS, FAIL, or BLOCKED
  and concrete evidence.
- Migration clean-install and upgrade evidence for both database families.
- Security, privacy, corpus-isolation, OIDC-negative, rollback, and concurrent
  appointment results.
- Updated operations documentation for OIDC setup, admin bootstrap, user
  lifecycle, corpus grants, backup/restore, appointment evidence, and succession
  ambiguity.
- One new PR against `main` with the acceptance matrix and known limitations.

**Acceptance criteria:**
- The required deterministic gates pass on the feature branch:
  `cargo fmt --check`, `cargo build --locked`, `cargo test --locked`,
  `cargo clippy --locked --all-targets -- -D warnings`, `make rust-check`,
  `make web-check`, `make web-e2e`, `git diff --check`,
  `git ls-files docs/tran/`, and `git check-ignore -v docs/tran/`.
- Rust enforces all permissions and the separate genealogy database guard.
- No real genealogy record is promoted or appointed by tests; no private data
  appears in tracked or staged diffs.
- The PR is merge-ready but is not merged without explicit operator authorization.

## Stop conditions

- Stop before public deployment if a real OIDC provider, correct cryptographic
  verification, TLS, secure sessions, or initial-admin setup is unavailable.
- Stop before any real Head of Family designation when third-branch mapping,
  first-born parentage/birth order, recent living status, or original-source
  provenance is missing, disputed, or stale.
- Stop before destructive migration or reassignment of real data. Require backups
  and operator approval.
- Stop on failing required gates; investigate without relaxing checks.
