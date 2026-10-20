# PLAN-TRAN-RBAC — Private Genealogy Access and Head of Family

**Repository:** `https://github.com/imhttran/tran`  
**Status:** TRAN-003 POST-MERGE FOLLOW-UP REQUIRED — PR #7 merged on 2026-10-20; functional acceptance incomplete  
**Scope:** Rust Axum API, PostgreSQL/pgvector, Next.js UI, genealogy catalog integration  
**Execution:** This file records the merged publication and outstanding acceptance. The original `plan-tran-rbac` SOP run is historical; implement remaining work on a new isolated follow-up branch, with a distinct SOP run identity and new PR.

## Goal

Secure the Tran family genealogy project with user identities, role-based authorization, corpus isolation, and an audited Head of Family designation. Only a verified, living, first-born male of the **third family branch** can be designated Head of Family. Preserve evidence and never infer genealogy facts from names or model-generated text.

## Domain rules

1. A `user` is an authenticated application account. A `person` is a cataloged genealogy subject. They are not automatically the same entity.
2. Application roles are `admin`, `editor`, and `viewer`. Start with one private family corpus; extend to scoped branch/corpus grants only after the branch catalog has stable identifiers.
3. A `head_of_family` is a **genealogy designation**, not an administrator role. It grants no permission to manage users, documents, or security settings by itself.
4. Only an active `admin` may approve, create, revoke, or replace a Head of Family designation. Editors may draft a proposal with evidence; viewers may read approved, authorized information only.
5. Eligible candidate conditions, all required:
   - Cataloged person belongs to the **third family branch** (a verified branch identifier, not generation 3).
   - The person's documented sex/gender classification used by the family record is male. Do not infer this from a name.
   - The candidate is documented as the **first-born male** of the applicable third-branch lineage. Verify parentage, birth order, and any conflicting accounts using source citations.
   - The person has a **verified living status**, supported by recent, attributed attestation or authoritative evidence. `unknown`, `unverified`, `deceased`, and disputed status are not eligible.
   - The branch mapping and evidence have been reviewed. Do not convert uncertain translations or RAG answers into authoritative facts.
6. Enforce at most one active approved Head of Family for branch 3. Do not auto-appoint a successor when a person dies or eligibility becomes uncertain; suspend/revoke the designation and require an explicit succession decision.
7. If the first-born male is deceased, do **not** silently appoint the next male child. Succession beyond the first-born male is an unresolved family-policy decision, not a software default.
8. All genealogy evidence includes original source identity, page/section, translated passage when available, assertion type, provenance, reviewer, and review timestamp. The uploaded *Family Genealogy.pdf* remains the first reference, not proof that every mentioned person is related to the account holder.

## Permissions (MVP)

| Capability | Viewer | Editor | Admin |
| --- | --- | --- | --- |
| Read permitted family documents and search/Q&A | Yes | Yes | Yes |
| Read approved family tree and Head of Family | Yes | Yes | Yes |
| Upload/replace source documents | No | Yes | Yes |
| Edit catalog entries and propose genealogy corrections | No | Yes | Yes |
| Propose Head of Family with citations | No | Yes | Yes |
| Approve, assign, revoke, or replace Head of Family | No | No | Yes |
| Invite/deactivate users or assign roles | No | No | Yes |
| Audit log access | No | No | Yes |

All permissions require authentication. No role is assigned from client-provided headers or request-body fields. Absence of identity or permission denies access.


## Current baseline and source of truth (2026-10-20)

TRAN-002 (PR #6) is merged into main. The merged TRAN-002 plan, genealogy schema contract, migrations 0001–0005, source-provenance checks, catalog parser, local CLI, review workflow, and pure Head of Family eligibility function are the **canonical genealogy implementation**. TRAN-003 must consume these interfaces rather than recreate a second people/relationships/assertions model.

The source inventory consists of four private originals/derivatives under docs/tran/. The catalog dry run reported 263 entries (259 parsed, 4 ambiguous), not 263 verified distinct relatives. The recorded disposable import produced 356 unverified assertions and **zero verified** assertions. Do not automatically convert candidate records into an approved family tree or Head of Family.

**PR #7 is merged** (merge commit `c5cfd96`, feature head `ae0fcdd`, contained in `origin/main`). It delivered corpus-scoped documents, roles and grants, an identity migration, API authorization, a Next.js server-side proxy, UI components, and synthetic browser tests. Six CI checks passed and SOP publication evidence was preserved. **Publication is not complete functional acceptance**: the gaps recorded below still require a focused follow-up PR. Do not reopen the historical `plan-tran-rbac` run.

| Existing surface | Disposition for TRAN-003 |
| --- | --- |
| TRAN-002 genealogy database, source evidence, review, eligibility | Reuse unchanged; extend additively only if a required API or appointment capability is absent |
| RAG database and documents table | Preserve contents, indexes, and existing migration history |
| Merged PR #7 corpus-scoped reads and writes | Retain and regression-test in follow-up |
| Merged PR #7 identity migration, role policy, and proxy | Retain; complete token verification, login, session, and negative tests |
| Merged PR #7 genealogy pages | Wire to authorized Rust endpoints; existing UI tests mock API responses |
| Merged PR #7 Head of Family approval handler | Replace `pending_catalog` placeholder with an evidence-backed transactional workflow |
| PR #4 merged planning baseline | This file remains the canonical plan; no competing plan or schema |

## Post-merge disposition and outstanding acceptance (2026-10-20)

**Published and historical:** PR #4 (planning) merged at `2b61f47`; PR #7 (RBAC foundation) merged at `c5cfd96`, feature head `ae0fcdd`. The reported SOP validation and six PR CI checks passed. The `plan-tran-rbac` SOP lifecycle was historicalized and the evidence archived locally under ignored `.sop-evidence/plan-tran-rbac/`. The PR #7 execution worktree and local branch were cleaned; remote branches remain. These are publication facts, **not** proof of end-to-end functionality.

| Area | Merged `main` observed state | Required follow-up acceptance |
| --- | --- | --- |
| Genealogy read API | No registered `GET /api/genealogy/records` or `GET /api/genealogy/head-of-family` in `src/api.rs` | Implement authorized reads using the separate TRAN-002 genealogy database; verify denials and record privacy |
| Head of Family writes | Admin approval handler returns `status: "pending_catalog"` without a persistent appointment | Reuse TRAN-002 pure eligibility, cited evidence, atomic appointment/audit writes, uniqueness and concurrency protection |
| Account and role access | `Operation::Account` requires `admin` | Permit viewer/editor/admin to access *their own* account; keep role management admin-only |
| Sign-in | `web/src/app/signin/page.tsx` is an unconfigured placeholder | Implement and test provider-backed OIDC authorization-code + PKCE and secure session lifecycle |
| Token verification | `src/oidc.rs` is custom and HS256-only | Use a maintained verifier for chosen provider's approved asymmetric algorithms and public JWKS |
| Admin recovery / enrollment | No final-admin revocation guard in `change_role`; bootstrap exists but normal account provisioning is absent | Add last-active-admin safeguards and auditable issuer+subject-based account provisioning |
| Browser mutations | Head of Family UI posts to unregistered endpoints, omits CSRF headers, and does not check response success; UI tests mock these routes | Wire real endpoints with stable IDs, CSRF, response/error handling, and persisted-state integration tests |

**Acceptance ledger:** The published PR #7 foundation (including corpus-scoped writes) is retained. TRAN-003A's original publication/reconciliation history is complete, subject to new-branch preflight. TRAN-003B needs follow-up regression evidence. TRAN-003C, D, E, F and G remain **not accepted** until their specific criteria below pass with real API, database, and browser integration. Preserve TRAN-002's genealogy schema and pure eligibility engine; do not create duplicate family tables. No real Head of Family has been designated.

**Follow-up execution:** Keep this file as the canonical requirement baseline. Use a new isolated worktree, a new PR, and a distinct SOP identity (suggested `plan-tran-rbac-integration-followup`), without rewriting `plan-tran-rbac` history. Use synthetic data only, respect the two-database trust boundary, and keep public deployment blocked pending full authentication, authorization, and explicit approval.

### Persistence and trust boundaries

- **DATABASE_URL:** existing RAG PostgreSQL database; RAG document/corpus operations and application identity tables may be hosted here. Authorization must never depend on request-supplied roles or corpus names.
- **GENEALOGY_DATABASE_URL:** independent, explicitly configured local genealogy database, protected by the TRAN-002 guard. Only its existing tables/migrations own persons, branches, evidence, assertions, review history, and Head of Family appointments. Never silently fall back to DATABASE_URL.
- Application account (immutable issuer + subject) is not automatically a genealogy person. Keep actor identifiers auditable without cross-database foreign keys. Appointment eligibility and its authoritative appointment audit must commit atomically in the genealogy database. Identity/role changes remain independently transactional and audited.
- Rust Axum enforces identity, permission, record/corpus grants, and privacy before any document fetch, DB write, embedding, or model call. Next.js only presents allowed results and securely mediates the browser session.
- Original genealogy evidence must be a valid original source and locator as governed by TRAN-002. Catalogs, translations, transcriptions, RAG answers, and name similarity do not independently prove any relationship.

## Common SOP delivery contract

- Treat stages **TRAN-003A–TRAN-003G** as the *follow-up acceptance contract*, not instructions to reopen the historical SOP lifecycle. Reconcile prior evidence first; execute and verify only outstanding implementation under a distinct plan/run ID.
- First inspect merged PR #7 and latest main, including changed files, historical SOP evidence, private/untracked artifacts, migration histories, and passing tests. Preserve unrelated work and never blindly reset, discard, force-push, or rebase a dirty tree.
- Use verify-first for work genuinely implemented in merged PR #7. Re-run affected checks when code, migrations, or prerequisites changed; do not treat historical green checks as acceptance of missing functionality.
- Tests and tracked documentation use **synthetic genealogy data only**. Never commit docs/tran/, its audit subdirectory, private source excerpts or hashes, local database exports, real family names, or living-person details. Check both staged and committed diffs.
- Real family records remain candidate, unverified, needs_review, or disputed until the required source evidence and authorized review are complete. No model-generated answer grants permissions or approves a family designation.
- Use disposable local PostgreSQL databases for migration and DB integration testing. Do not reset the RAG database or local private genealogy database as a test convenience.

## TRAN-003A — Preflight, branch reconciliation, and contract mapping

**Dependencies:** TRAN-002 merged; PR #4 planning baseline merged; PR #7 merged and historicalized.

### Deliverables

- Confirm `origin/main` contains TRAN-002 migrations 0001–0005, provenance/review/eligibility code, and the merged PR #7. Compare implementation against the functional acceptance criteria instead of treating publication as completion.
- Continue remaining implementation on a **new isolated follow-up branch and PR**, not on the published `sop/plan-tran-rbac` run. Preserve original RAG and TRAN-002 genealogy migrations.
- Inventory current Axum route registrations, UI-requested endpoints, OIDC support, Next.js session/login handlers, identity grants, genealogy DB access, and appointment persistence. Record a grounded route/permission/data-owner matrix.
- Record baseline checks and identify previously verified code that needs no changes, confirmed defects that need repair, and environment-only deployment prerequisites.
- Preserve the original SOP archive and evidence as history. Use a distinct follow-up SOP identity; do not restart completed tasks or bypass approvals.

### Acceptance criteria

- The follow-up branch starts from merged `origin/main` containing TRAN-002 and PR #7, with no duplicate/conflicting genealogy schema, an attributable diff, and a safe worktree state.
- Every browser genealogy operation maps to an existing or explicitly planned server route with a named permission, data source, and expected denial behavior.
- A public-safe preflight document records exact SHA, changed baseline, outstanding tasks, and verification limitations.

### Verification

- Git ancestry/diff and migration inventory checks; existing Rust/build/CI smoke checks when reconciliation changes code.
- Confirm git ls-files docs/tran/ is empty and the source inventory still contains only the four original files.

## TRAN-003B — Corpus isolation and default-deny review

**Dependencies:** TRAN-003A.

### Deliverables

- Reuse PR #7's scoped (corpus, source) replacement and review unchanged-source checks, status, keyword/vector retrieval, expansion, citation resolution, ingestion, and local CLI boundaries for isolation.
- Validate the trusted corpus is private; legacy NULL-corpus rows and ungranted corpora are not exposed by default. Never infer corpus access from a filename or request header.
- Require authenticated grants on Rust handlers; deny before DB mutation, staging, embeddings, or model generation. Limit public health to nonsensitive status.

### Acceptance criteria

- Two authorized corpora with the same document filename retain independent data on replacement and reimport.
- An ungranted user cannot read, replace, search, expand, or retrieve citations or source-status details from another corpus.
- Anonymous/expired/forged credentials fail closed with no side effects; viewers cannot upload and client-provided role/corpus headers cannot elevate permissions.

### Verification

- Synthetic PostgreSQL same-filename/cross-corpus tests; Rust API direct-call 401/403 tests; no-leak retrieval and citation cases. Reuse existing PR #7 tests only after confirming the merged baseline.

## TRAN-003C — OIDC authentication, sessions, and roles

**Dependencies:** TRAN-003A; TRAN-003B's trust-boundary contract.

### Deliverables

- Retain database-backed users, role assignments, corpus grants, audit events, and secure bootstrap when reconciled, but verify all required migrations and effective-permission behavior.
- Replace PR #7's custom HS256-only JWT/JWKS path with a maintained standards-compliant token verifier supporting only the chosen provider's securely approved algorithms. Correctly parse JWK public keys, require verified issuer/audience/expiry/subject, and fail closed on unavailable or invalid keys. Do not introduce a shared secret in an exposed JWKS or silently accept unsigned tokens.
- Implement a real standards-based OIDC authorization-code + PKCE browser sign-in/out flow (including state and nonce checks where applicable). Provider setup is an explicit deployment prerequisite; choose no provider without operator direction.
- Secure HttpOnly, Secure, appropriately SameSite session cookies, session expiry/rotation, CSRF protection for browser cookie-authenticated mutations, and logging/redaction. Keep tokens and provider secrets off client-visible payloads.
- Load roles and corpus grants from the server-side database by immutable issuer + subject. Changes and revocations take effect without trusting token role claims; administer role changes transactionally and audit them.
- Preserve the local-operator CLI trust boundary; browser RBAC does not protect local shell or database credentials.

### Acceptance criteria

- Sign-in and sign-out actually work with an approved provider or a standards-compliant isolated synthetic provider in acceptance tests; the page does not claim a configured sign-in when it is only a placeholder.
- Wrong issuer/audience/algorithm, malformed JWK, forged/expired token, replayed state, invalid session, provider outage, client-supplied roles, and CSRF failure deny securely.
- Admin/editor/viewer role changes are enforced by Rust even with stale browser UI; direct Axum access does not bypass policy.
- No public deployment or real-family multi-user enrollment without operator-configured OIDC, TLS, secure cookies, and initial-admin bootstrap.

### Verification

- Rust OIDC/security tests using generated synthetic signing keys; login/session/CSRF browser integration tests; role revocation/permission audit integration and negative tests.

## TRAN-003D — Authorized genealogy read APIs on TRAN-002

**Dependencies:** TRAN-003A; TRAN-003C.

### Deliverables

- Connect Rust handlers to the **existing** separate genealogy database through the guarded GENEALOGY_DATABASE_URL; do not copy or rebuild the person/assertion/branch/source evidence tables in the identity or RAG database.
- Implement the browser-required read endpoints, including GET /api/genealogy/records and GET /api/genealogy/head-of-family, plus needed person, relationship, and citation-detail reads. Specify response contracts, pagination, statuses, and grant scopes.
- Read existing TRAN-002 verified/unverified assertions and source-page links. Show candidate identities, conflicts, and uncertain relations as such; do not render candidate links as verified relatives.
- Authorize before consulting genealogy DB or emitting existence-sensitive responses; ensure 401/403/no-access and error bodies cannot disclose protected names, pages, or branch associations.

### Acceptance criteria

- Authorized synthetic users can fetch a cited verified relationship and distinguish it from unknown, disputed, or missing evidence.
- Anonymous and ungranted requests disclose neither records nor source existence; direct Rust and proxied Next.js behavior agrees.
- Genealogy read operations use only the genealogy DB; RAG data and its migrations remain unchanged. Missing/unusable genealogy DB fails closed without falling back.

### Verification

- Disposable-DB Axum integration and response-schema tests; private-record non-disclosure tests; browser API-contract tests.

## TRAN-003E — Head of Family nomination, appointment, and review

**Dependencies:** TRAN-003C; TRAN-003D.

### Deliverables

- Reuse TRAN-002's pure eligibility decision and reviewed assertion/evidence rules. Branch 3 means the **verified third branch**, not generation 3; the first-born male's status and birth order must be evidenced, not inferred.
- Extend genealogy/migrations/ additively for nomination, one-active-appointment-per-branch constraints, appointment evidence snapshots, reviewer/approver identity, status, revocation/suspension, and append-only audit events. Never duplicate TRAN-002 people/branches/assertions tables.
- Implement editor nomination, admin approval/rejection, and admin revocation/suspension routes matching the Next.js contract. Authenticate and authorize each write; identify the person by stable key, not a free-text name.
- In a genealogy-DB transaction, lock or serialize the competing appointment/eligible subject, re-evaluate verified evidence and living status, prevent stale approvals, enforce unique active appointment, and atomically record the appointment plus audit.
- If living status becomes deceased, unknown, or disputed, suspend/revoke any affected designation according to an explicit reviewed policy. Never auto-appoint a successor or grant any administrator permission through the family title.
- No actual family head is appointed by implementation or test runs. Real appointments require authorized review of current evidence, then explicit operator/family approval under the unresolved succession policy.

### Acceptance criteria

- Fully synthetic, independently verified first-born living male in the documented third branch can be approved only by an admin; verified evidence IDs and reviewer are recorded.
- All missing/contested/deceased/younger/wrong-branch or gender/birth-order conditions are denied or reported undetermined, without automatic succession.
- Two simultaneous approvals yield at most one active appointment. Evidence changes or revoked living-status verification cannot leave an unreviewed active title.
- Viewer/editor cannot approve/revoke; Head of Family status does not alter RBAC privileges. Every real mutation is audited and cannot be mistaken for a placeholder success.

### Verification

- Synthetic domain tests plus transactional PostgreSQL concurrency, rollback, revocation, and eligibility/role separation tests; negative direct API and browser tests.

## TRAN-003F — Browser and user-flow integration

**Dependencies:** TRAN-003C; TRAN-003D; TRAN-003E.

### Deliverables

- Wire sign-in, sign-out, session-aware account menu, record browsing, citation drill-down, and the Head of Family view to real API results.
- Keep upload/nomination/approval/revocation controls role-aware but always enforce the same permissions in Rust. Use real stable person/nomination/appointment IDs, not labels as identifiers.
- Ensure POST/PUT/DELETE actions send the required CSRF header/token, await responses, surface success **only after** server confirmation, and show meaningful 401/403/409 and missing-evidence states.
- Avoid leaked token/cookie/database/identity fields in HTML, client JavaScript, browser logs, and JSON response payloads.

### Acceptance criteria

- Playwright tests cover authenticated viewer, editor, admin, signed-out and expired cases, real upload denial, reviewed record queries, nomination, approval and revocation, and unresolved-evidence display.
- Browser button clicks change persisted authorized state on the server in integration tests, rather than only dismiss confirmation dialogs.
- No Head of Family designation appears without a verified active appointment. No LLM answer can cause an appointment.

### Verification

- make web-check and make web-e2e with a synthetic OIDC test authority and disposable databases; direct and proxied authorization parity tests.

## TRAN-003G — End-to-end security, privacy, and closure

**Dependencies:** TRAN-003A, TRAN-003B, TRAN-003C, TRAN-003D, TRAN-003E, TRAN-003F.

### Deliverables

- Validate clean installs and upgrades for both **distinct** database migration families, preserving existing documents and verified genealogy review history.
- Record post-TRAN-002 integration evidence, all authorization/cryptography negatives, corpus-leakage and appointment-race results, browser outcomes, deployment prerequisites, and known limitations.
- Update docs for OIDC provider setup, admin bootstrap, family-user invite/revocation, corpus grants, privacy, backup/restore, appointment evidence, and succession-policy ambiguity.
- Record only public-safe synthetic validation and nonidentifying metrics in tracked evidence docs. Keep private real-data audit outputs under ignored docs/tran/audit/ and preserve the four original source file hashes.
- Open a **new follow-up PR** with focused post-merge verification evidence. Do not rewrite published PR #7, automatically merge, deploy, historicalize an unfinished follow-up run, or delete historical evidence.

### Acceptance criteria

- Deterministic required gates pass on the reconciled feature branch and GitHub Actions CI passes on that same commit.
- Rust enforces all permissions, the separate genealogy-DB guard works, and all six stages have verifiable acceptance evidence.
- No real genealogy record is promoted or appointed by a synthetic test; all source privacy checks pass; no real-person data appears in tracked diffs or CI.
- The new follow-up SOP run reaches PASS with no unresolved required approval and its new PR is merge-ready against latest main, subject to explicit operator merge authorization.

### Verification

Run focused tests during development. At closure, run:

~~~bash
cargo fmt --check
cargo build --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
make rust-check
make web-check
make web-e2e
git diff --check
git ls-files docs/tran/
git check-ignore -v docs/tran/
~~~

Run synthetic PostgreSQL migration clean-install and upgrade checks, genealogy provenance/rollback tests, OIDC-negative tests, two-corpus isolation tests, concurrent appointment tests, and applicable direct Rust API and real-browser acceptance checks. Verify the **staged diff** for private data; a green CI checkout by itself does not prove local privacy.

## Dependencies

~~~text
TRAN-003A -> TRAN-003B -> TRAN-003C -> TRAN-003D -> TRAN-003E -> TRAN-003F
       \___________________________________________________________/
                             TRAN-003G
~~~

Stages B and C may be developed independently after the preflight contracts are recorded, but the end-to-end security and privacy gates remain mandatory for G.

## Stop conditions and explicit operator decisions

- Stop before external/public deployment if a real OIDC provider, correct cryptographic verification, TLS, secure sessions, or initial-admin setup is unavailable. Local synthetic verification may continue.
- Stop before any real Head of Family designation while third-branch mapping, first-born male parentage/birth order, recent living status, or original-source provenance is missing, disputed, or out of date. Never infer a successor; ask for a separate succession decision.
- Stop before destructive migrations, production data moves, or ambiguous source reassignment. Require backups and operator approval. Do not reset actual family or RAG databases.
- Stop on failing required deterministic gates and investigate without relaxing checks or fabricating PASS. Continue unaffected tasks only where SOP permits.
- No plan execution, successful CI, or green local test authorizes an automatic merge, public rollout, private-data publication, or deletion of untracked SOP artifacts.

## Completion and PR policy

**PR #7 has already merged and been published.** Complete the remaining TRAN-003 acceptance work in a new isolated follow-up branch and separate SOP run/PR. Preserve evidence, require explicit merge approval, and only after the follow-up PR merges use `/sop-publish` to archive that new run. Green CI from PR #7 alone is not closure of missing capabilities.

The application remains private. A live family-tree experience and a real Head of Family appointment are distinct operational decisions, not side effects of merging code.
