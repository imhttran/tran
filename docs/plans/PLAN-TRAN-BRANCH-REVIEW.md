# PLAN-TRAN-BRANCH-REVIEW — Head of Family and branch-scoped verification

**Status:** PROPOSED — implementation handoff only; no permissions, appointments, user accounts, or family facts created.
**Repository:** https://github.com/imhttran/tran
**Suggested SOP identity:** `plan-tran-branch-review`
**Canonical requirements:** `docs/plans/PLAN-TRAN-USER-ONBOARDING.md` (USR-005 and USR-006), `docs/plans/PLAN-TRAN-USER-ONBOARDING-SOP.md`, `docs/plans/PLAN-TRAN-RBAC.md`, `docs/plans/PLAN-TRAN-FAMILY-TREE.md`.
**Dependency:** existing reviewed OIDC person-account linking (PR #25), admin invitation/linking endpoints (PR #26), existing TRAN-002 assertion review, existing Head of Family appointment/eligibility rules, and USR-004 shared profile submission / USR-006 review workflows as they become available. Reconfirm latest `origin/main` and current worktrees before execution. PR #27 contains the separate bulk-onboarding plan and is not part of this feature.

## Goal and minimal UI

Allow a person with an active, reviewed OIDC account to be granted **Branch Reviewer** for one or more specifically approved branches by a system administrator. The member with the independently verified **Head of Family** genealogy designation may receive an explicit reviewer grant for **their own branch**. They can propose corrections to shared family-tree facts and approve/reject/request changes for **another person's** submissions within granted branches, with source/evidence review and immutable audit.

Build one **Branch Review** screen at `/branch-review`, in the existing Next.js shell, with a simple queue, person/branch scope, proposed-vs-current diff, evidence links, and three decisions: **Approve**, **Request changes**, **Reject**. Include a proposed-correction action for authorized shared branch facts.

Do **not** equate genealogical `Head of Family` with application `admin`, and do **not** automatically grant reviewer privileges because someone appears in a family tree or is displayed as Head of Family.

## Baseline and explicit boundaries

- `genealogy/migrations/0007_onboarding.sql` already defines `onboarding_branch_reviewer_grant(account_id, family_branch_id, granted_by, created_at, revoked_at)`. Reuse it; avoid duplicate reviewer tables. Add constraints/indexes/audit via an **additive** migration only when required. Verify actual migration numbering; FAM-002 and other onboarding work may concurrently add migrations.
- `genealogy/migrations/0003_assertions_evidence_and_links.sql` owns `person`, `family_branch`, reviewed assertions, source evidence, spouse and parent/child links. `genealogy/migrations/0004_review_workflow.sql` and `src/genealogy/review.rs` already implement source-backed assertion review. Reuse their validators, audit, and provenance rules; do not create a second source-truth store.
- The OIDC identity lives in the RAG/identity DB, keyed by immutable `(issuer, subject)`. Reviewed `onboarding_person_account_link` associates it with a genealogy person via stable opaque account ID / identity fingerprint; never authorize by name or user-provided account ID. No cross-database FK.
- `src/genealogy/eligibility.rs` keeps the genealogical Head of Family rule: verified **living first-born male of the third branch**, with evidence; unknown/deceased/disputed never auto-promote a successor. `src/genealogy/appointments.rs` currently selects `status='active'` without validating current eligibility on read, so this is a **known safety dependency**. A title being out of date must not create reviewer privileges.
- System admin manages provider accounts, role assignments, security settings, reviewer grants, and override/escalation decisions. Branch reviewer does **not** get global account management, admin APIs, password reset, invitation privileges, Head of Family nomination/approval, or modification of immutable audit/source records.
- Member-private photos, contact details, personal documents, OIDC credentials, and unsubmitted private drafts are **not** opened by a branch grant. A submission explicitly shared for review can be inspected only for the named fields/evidence/version with an audited purpose. General document approval remains with explicitly authorized admin/document reviewers.
- This is scoped shared-genealogy editing and verification, **not** arbitrary modification of other adults' private profiles. Any broader delegation requires separate user consent and a new policy review.
- The repository is public. Never commit real names, dates, recipient emails, genealogy excerpts, hashes of private sources, private media, or actual eligible counts.

## Permission matrix

| Action | Family member | Branch Reviewer (granted branch) | System admin |
| --- | --- | --- | --- |
| Edit own private profile and submit shared-fact correction | Yes, within linked-person scope | Yes | Yes |
| Read in-scope submitted shared genealogy diff and supporting evidence | Own submission only | Yes, for assigned branch and explicitly shared version | Yes, under audited review permission |
| Create/propose shared branch correction | Own person / authorized proposal | Yes, within assigned branch, pending independent review | Yes |
| Approve / reject / request changes on *another person's* proposal | No | Yes, within authorized branch | Yes |
| Approve one's own proposal | No | **No** | **No** |
| Edit another member's contact details, password, private photo/document | No | No | Only explicit audited support access, not this reviewer grant |
| Change identity roles, grants, OIDC, admin settings | No | No | Yes |
| Approve/revoke genealogical Head of Family appointment | No | No | Admin-only under existing rules |
| Read or modify facts/relationships involving another branch | No by reviewer authority alone | Escalate for additional scoped approval | Resolve through verified, logged authorization |

**Multiple branches / cross-branch edges:** For a relationship, branch membership change, marriage or parent-child link impacting multiple branches, require a policy-approved authorization for every affected branch or escalate to system admin. Do not grant access to the whole graph by following one cross-branch edge. People with ambiguous/disputed branch membership are not reviewable by scope until branch evidence is resolved through an independently authorized workflow.

## Reviewer authority lifecycle

1. **Resolve verified identity.** Only an active OIDC account linked through the reviewed account-to-person process can receive a reviewer grant. No grant to an unlinked person key, pending invitation, deactivated identity, or unresolved cross-issuer identity.
2. **Select branch.** Admin selects a stable `family_branch_id` from the existing genealogy catalog. Source-backed branch mapping is required; never infer from surname, UI ancestry position, or RAG answer.
3. **Optional Head of Family validation.** If the admin is granting reviewer authority *because* the member is the Head of Family, verify the authoritative active appointment and re-evaluate its current original-source evidence. Permit only that appointment's branch. Do **not** expand to other branches. If the verification is unavailable, do not claim Head-of-Family-based authorization.
4. **Explicit grant.** Admin confirms account, branch, scope, and reason; write grant and audit transactionally. Unique active grants, idempotent repeat grants, revocation, and concurrent admin operations must be tested.
5. **Effective permissions.** API verifies account activity, identity fingerprint/link, active grant, and up-to-date branch scope on *every* read and write. Admin role does not become attached to title. A title-specific grant must cease being effective when the title loses verified active status; delegated non-title branch reviewers remain governed by their own explicit grants.
6. **Revocation.** Admin revokes grant; requests thereafter are denied including stale browser sessions. Audit the grant, revoke, reason and actor without exposing private identity claims. Account deactivation and person-link revocation also immediately void effective reviewer access.

Retain the **Head of Family title** and **branch reviewer permission** as separate records. A revoked reviewer grant does not change family succession history; a suspended or invalid Head of Family appointment is not an instruction to appoint another head.

## Submission, evidence and decision rules

- Every edit to shared family facts (name variant, relationship, birth/death dates, branch membership, biography excerpt chosen for family sharing, etc.) creates a versioned proposal, not an immediate verified mutation. Store stable `person_key`, affected `branch_key`/IDs, submitter's resolved account ID, existing record revision, proposed value, supporting evidence IDs and optional note.
- Reuse USR-004 / FAM profile-submission infrastructure when merged. If it is missing, implement only the **minimal, typed shared-genealogy proposal/decision** contract and do not duplicate their general-purpose photo/document asset pipelines. Isolate pending work in a distinct SOP slice if dependencies are still active.
- `pending -> approved | needs_changes | rejected`. Only a distinct reviewer from the submitter (including when both have admin role) may decide. An admin may not self-approve.
- Reviewer selection and **database mutation** must both enforce scope. Filter the queue and record reads at the query layer, and recheck scope, submission revision, branch membership and account activity inside the decision transaction to prevent time-of-check/time-of-use authorization bypass.
- A verified genealogy assertion requires original-source, locator-backed citations and must pass the existing TRAN-002 validation. The fact that someone submitted or approved a claim is *not* independent evidence of its truth. Missing/disputed evidence stays pending or rejected; no silent assumption that a missing death record means alive.
- Decisions append immutable reviewer, action, reason, evidence references, timestamp and old/new revision. Handle two concurrent reviews as one effective decision (second is `409 conflict`, no overwrite); a rejected proposal leaves the prior verified value unchanged.
- Recompute affected Head of Family eligibility on verified vital/branch/birth-order changes. An invalid active appointment must no longer be represented as currently verified; fail closed and surface review/suspension instead of auto selecting a successor. Do not write automatic successor appointments or confer app grants.
- Approved family-visible data may be included in RAG only after review/processing acceptance. Pending, rejected, disputed, legacy/unapproved, or owner-private items must not enter retrieval, answers or citations.

## API contracts

Reuse current OIDC session, CSRF-protected Next.js proxy, Rust authorization and privacy-safe errors. Final route naming should be consistent with current code, but the minimum operations are:

- `GET /api/branch-review/grants/me` — own effective grants, no other user's identities.
- `GET /api/branch-review/submissions?branch=<id>&status=pending` — strictly filter by authorized grant, with pagination and minimal needed data.
- `POST /api/branch-review/submissions` — propose shared corrections within caller's authorized scope.
- `POST /api/branch-review/submissions/{id}/decision` — approve, reject or needs_changes with required reason/evidence. Client-supplied branch/reviewer values are not trusted.
- `GET /api/admin/branch-reviewers` and `POST /api/admin/branch-reviewers/grants` + `POST .../{id}/revoke` — admin-only list/grant/revoke of scoped permissions.

Response rules: `401` unauthenticated; `403` no grant; `404` unknown or intentionally concealed off-scope record; `409` stale status/decision/identity, `422` invalid evidence; `503` unavailable genealogy/identity DB. Authorize before sensitive DB reads and never leak off-branch person names or existence through errors, counts, pagination or citations. Normalize account identity server-side from verified token, not JSON request body.

## Proposed UI /branch-review

- Header shows verified **My Branches** and authorized scope; no links if the user lacks a reviewer grant.
- Queue: pending submissions for branches they may review; filter by branch, status and person (authorized results only).
- Detail: affected person, before/after diff, submitted evidence and review status, submitter label where allowed, and warnings for missing/disputed sources and cross-branch impact.
- Decisions: Approve, Request changes, Reject; confirmation and reason input where required; display success only after server state reload and committed transaction.
- Proposed correction: a small form for shared family facts/relationships with evidence IDs, held for independent review; no direct authoritative overwrite.
- Admin screen adds **Assign branch reviewer** and **Revoke** to member actions, using existing reviewed person-account links.
- Responsive, accessible, clear empty/loading/denied states; no general messaging, document upload, or complex role editor in this feature.

## SOP tasks

### BR-001 — Baseline, dependencies and threat model

Preflight `origin/main`, current PR/worktrees and SOP run state. Reconcile USR-003/004/005/006 and FAM-002 work, migration numbering and existing authz. Verify historical plans/evidence. Record route-permission-data-owner matrix without exposing real family facts. **Do not create a duplicate SOP run for already-active USR-005.**

**Pass:** dependencies and the changed-code ownership boundaries are clear, no conflicting implementation.

### BR-002 — Effective branch grants

Implement audited admin grant/revoke service over `onboarding_branch_reviewer_grant`; resolve active reviewed identity; enforce stable branch scope, active status, optional title validity, audit and concurrency. Add indexes/migrations only if needed.

**Pass:** title only = denied; explicit grant in-scope = allowed; revoked/unlinked/deactivated/cross-issuer = denied; other branch = denied.

### BR-003 — Scoped submission/review API

Implement minimal shared-fact proposal and decision service using existing TRAN-002 review/evidence validation. Queue filtering, CSRF, no self-approval, cross-branch escalation, stale revision checks and immutable audit.

**Pass:** pending facts never replace verified facts; valid evidence + independent reviewer + in-scope transaction is required; no off-branch reads/writes.

### BR-004 — Head of Family validity and permissions

Bind any title-dependent reviewer grant to a currently valid, independently verified appointment **for the same branch**, using TRAN-002 eligibility. Address existing active-appointment read gap: `list_active_appointments` checks only status, so ensure a no-longer-verified appointment is not represented as valid or used as a reviewer authority. Implement explicit audited suspension/review as needed, never a successor.

**Pass:** revoked/disputed living, branch or first-born evidence blocks title-based authority and shows review-needed, without side-effecting reads.

### BR-005 — Small UI and integration acceptance

Deliver `/branch-review` queue and proposal/decision controls, plus minimal admin grant/revoke. Test real Rust API + disposable identity/genealogy DB + synthetic OIDC and synthetic people; no real family names. Run `cargo fmt --check`, `cargo build --locked`, `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`, `make rust-check`, `make web-check`, `make web-e2e`, `git diff --check`; inspect tracked files for private data.

**Pass:** 403 and nondisclosure across scopes, reviewer self-approval denied, genuine original evidence required, race conditions handled, accurate title validation, no role escalation, green CI. Publish a new focused implementation PR with an acceptance matrix and synthetic screenshots.

## Out of scope / approval gates

- No production branch-reviewer assignment, real Head of Family appointment, re-appointment, email/invitation, credential changes, account activation, source deletion, private document exposure, bulk genealogy verification, auto merges or deployment.
- Changes to genealogical succession rules require explicit family-policy approval. Do not infer branch head for other branches from the third-branch-only title.
- Reuse existing single-family `tran` database boundaries. Do not introduce multi-family/tenant abstractions unless separately authorized.
- Keep human approval at genuine security/data/merge gates and continue automatically through safe development tasks.
