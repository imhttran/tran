# PLAN-TRAN-BROWSER-ACCEPTANCE-REMEDIATION

**Repository:** `https://github.com/imhttran/tran`
**Plan ID:** `plan-tran-browser-acceptance-remediation`
**Branch:** `fix/browser-acceptance-remediation`
**Baseline:** `origin/main` at `3b19d114af31fbeeee47f05cf827145fcb9c9d87` (2026-10-31)
**Evidence:** `.sop-evidence/acceptance-verify-3b19d11/REPORT.md`
**Owner:** Linux workstream

## Purpose

Resolve the five reproducible defects found by the final browser acceptance
verification, in dependency order, without weakening authentication, privacy,
grounding, or citation guarantees. The PDF/RAG acceptance scope already passed;
these fixes close the remaining family/admin browser failures and the brittle
live-citation assertion.

## Non-negotiable constraints

- Preserve the test OIDC/session flow and global session-expiry semantics.
- Do not weaken privacy assertions or accept `/api/profile` on restricted pages.
- Do not bypass authentication, CSRF, or authorization checks.
- Do not substitute generation/embedding providers.
- Do not add retries that mask missing or incorrect citations.
- Do not change unrelated functionality.
- Keep the acceptance-verification worktree and its evidence intact.

## Task 1 — INVITE-GUARD (do first)

**Objective:** A missing or malformed invitation payload must not crash the
family profile.

**Deliverables:**
- `web/src/components/ProfileInvitationPanel.tsx` treats a missing
  `state.invitation` as the existing `unavailable` panel state instead of
  dereferencing it.
- `web/e2e/family-dates.spec.ts` narrows its family-person route interception so
  it no longer captures `/invitation` and `/invitation/status`.
- Loading, hidden (401/403), unavailable, and ready states remain distinct.

**Acceptance criteria:**
- `family-dates.spec.ts` passes.
- A malformed invitation state renders the unavailable message, not an error
  page.
- No invitation regression in `profile-invitation.spec.ts` or the Rust
  `profile_invitation` suite.

## Task 2 — ADMIN-STUB-COVERAGE

**Objective:** Stub the protected link-requests call in stub-only admin tests so
an auxiliary panel cannot turn a scenario into a session-expiry failure.

**Deliverables:**
- `web/e2e/bulk-onboarding.spec.ts` stubs
  `GET /api/admin/family-members/link-requests` with an empty list.
- `web/e2e/admin-workspaces-visual.spec.ts` stubs the same endpoint.
- The application's 401 -> session-expired behavior is unchanged.

**Acceptance criteria:**
- The seven `bulk-onboarding.spec.ts` "stubbed preview filters" tests pass.
- The affected `admin-workspaces-visual.spec.ts` tests pass.
- A real 401 still expires the session (existing session-expiry tests pass).

## Task 3 — NAV-PRIVACY

**Objective:** Do not request the caller's `/api/profile` from restricted
family-facing and branch-review pages.

**Deliverables:**
- `web/src/components/ExperienceShell.tsx` gates the profile-review badge fetch
  off `/family/*` and `/branch-review*`.
- The badge still loads on member-facing pages.

**Acceptance criteria:**
- `branch-review-security.spec.ts` "never exposes private profile data" passes
  with zero `/api/profile` requests.
- `family-profile-ui.spec.ts` "never renders owner-private values" passes with
  zero `/api/profile` requests.
- The profile page still renders the review-decision badge.

## Task 4 — ADMIN-STATUS-SCOPE

**Objective:** Scope the ambiguous status assertion to the intended form result.

**Deliverables:**
- `web/e2e/admin-accounts.spec.ts` filters the status locator by the expected
  message (or the form-status element) instead of matching every
  `role="status"` node.

**Acceptance criteria:**
- The "admin links a reviewed account to a person and removes the link" test
  passes.
- The grant/revoke behavior assertions are unchanged.

## Task 5 — LIVE-CITATION-ASSERT

**Objective:** Replace brittle citation substring matching with provenance-based
validation.

**Deliverables:**
- `acceptance/live_rag/tests/live_rag.rs` parses the answer-body citations and
  verifies the expected source, section, and page against the stored rows, while
  accepting an optional `(shortkey)` suffix.
- Every cited provenance still resolves in PostgreSQL; a missing or wrong
  citation fails.
- No new retry logic is added; the existing bounded retry is preserved.

**Acceptance criteria:**
- `acceptance/live_rag` passes against live Ollama with the real test database.
- The policy answer must cite `live-policy.md` with a stored section.
- The PDF answer must cite `live-text-layer.pdf` page 2 with page provenance.

## Final verification

1. `cargo test --locked` against disposable test databases.
2. `cargo test --locked --manifest-path acceptance/api_contract/Cargo.toml`.
3. `cargo test --locked --manifest-path acceptance/live_rag/Cargo.toml`.
4. `make web-check`.
5. `make web-e2e`.

Record passed, failed, skipped, and blocked counts, the exact commit SHA, the
commands, and evidence paths. Then run the SOP review gate and open a pull
request. Do not merge without the required checks and approvals.
