# PLAN-TRAN-RBAC — Private Genealogy Access and Head of Family

**Repository:** `https://github.com/imhttran/tran`  
**Status:** PROPOSED — not implemented  
**Scope:** Rust Axum API, PostgreSQL/pgvector, Next.js UI, genealogy catalog integration  
**Execution:** SOP-managed, one end-to-end run, approval only at real security or publication gates

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

## Phase 0 — Preflight and baseline

1. Work from current `origin/main` in a new `feat/tran-rbac` branch or isolated worktree. Preserve existing changes. Inspect prior passing check evidence and reuse it where inputs are unchanged.
2. Map real Rust API endpoints (`/api/health`, `/api/query`, `/api/documents/ingest`, `/api/documents/{source}/status`), CLI entry points, Next.js rewrite, and database reads/writes.
3. Verify whether the people/branch genealogy catalog is present. If absent, add only the minimum stable `branches`, `people`, `relationships`, and cited evidence schema needed for the designation; do not invent or bulk-assign people.
4. Document the application identity approach. Use standards-based OIDC sign-in for multi-user browser access, verified on the Rust side; bind accounts by immutable provider subject, never email alone. Do not expose app endpoints publicly until real identity-provider settings and TLS are configured. For automated tests use isolated cryptographically valid test identities; no production auth-bypass switch.
5. Record baseline test results and any environment limitations.

**Gate:** No production secrets in repo or logs. No internet-facing deploy without functioning authentication and authorization.

## Phase 1 — Fix corpus isolation before RBAC

1. Fix PostgreSQL `replace_source` to replace rows by `(corpus, source)`, not filename alone. `src/storage_pg.rs` currently deletes using `DELETE FROM documents WHERE source = $1`.
2. Review `unchanged_source`, content-hash lookup, `source_status`, vector search, keyword search, expansion, direct row reads, and CLI reads for accidental cross-corpus access. Pass an explicit, validated corpus identity through every operation.
3. Deny requests for a corpus not in the authenticated user's grants. Treat the existing `trusted` corpus as private; do not expose legacy `NULL`-corpus rows automatically.
4. Cover two corpora with the same filename and different content. Reingestion, status, and Q&A must not change or disclose the other corpus.

**Gate:** Cross-corpus overwrite, search-result leakage, and citation leakage tests all pass.

## Phase 2 — Authentication and authorization

1. Add an identity adapter and a central permission policy in Rust. Validate OIDC token issuer, audience, signature (using cached JWKS), expiration, and subject. Reject tokens on verification errors; do not use unverified claims to grant roles.
2. Add database-backed `users`, `role_assignments`, `corpus_grants`, and `audit_events` with migrations. Keep role changes transactional and audited; use an explicit, secure initial-admin bootstrap.
3. Require `viewer` for `/api/query` and document status; `editor` for ingestion; `admin` for account, role, and Head of Family approval APIs. Keep `/api/health` public and limited to non-sensitive availability only.
4. Replace the unrestricted Next.js `/api/*` rewrite with a server-side authenticated API route/proxy that forwards only the verified identity credential to Axum. Prevent direct client-injected identity headers from elevating permissions. Apply CSRF protection for cookie-authenticated mutations, and secure/HttpOnly/SameSite session cookies.
5. Ensure unauthorized requests fail **before** document staging, database reads/writes, embeddings, or model generation. Return `401` for missing/invalid login and `403` for authenticated insufficient permission. Rate-limit sensitive operations.
6. Explicitly document the CLI trust boundary: CLI is a local operator interface, not a remote family-user API. Do not suggest browser RBAC protects unrestricted shell/database access.

**Gate:** All endpoints deny by default. Neither direct Axum calls nor Next.js proxy requests bypass RBAC. Identity-provider outages do not become anonymous access.

## Phase 3 — Genealogy Head of Family

1. Establish the canonical identifier for the **third branch** from reviewed genealogy catalog evidence. Do not conflate branch 3 with generation 3.
2. Add cited relationship, birth-order, branch membership, and living-status assertions. Each assertion can be `verified`, `unverified`, or `disputed`; living status also records a reviewer, evidence, and confirmation date.
3. Add `head_of_family_appointments`: candidate person ID, branch ID, approved-by user ID, evidence references, effective timestamp, revoked/suspended timestamp, and reason. Enforce one current approved appointment per branch in the database.
4. Centralize a server-side eligibility function. Check every domain rule within the same transaction that assigns the designation; lock relevant person/appointment rows or use equivalent serializable protection to prevent concurrent appointments and stale validation.
5. On death, dispute, or revocation of verified living status, prevent new approvals; suspend or revoke an active appointment by a documented policy. Never silently transfer the title.
6. Add an admin approval workflow. Editors can submit evidence-backed proposals. The approval API re-evaluates eligibility; no role or form field can override failed checks.
7. Keep the Head of Family title separate from user `admin` privileges. Any extra authority for this family title requires a later explicit permission policy.

**Gate:** Only a verified living first-born male of the documented third branch can hold an active designation. Every rejection explains which *validation category* failed, without leaking protected records.

## Phase 4 — Browser experience

1. Add sign-in/sign-out and a session-aware account menu.
2. Hide upload controls for viewers. Display read-only search and genealogy records only when authorized; retain Rust API enforcement regardless of UI state.
3. Add an evidence-linked Head of Family view. Add editor nomination and admin approval/revocation controls with explicit confirmation.
4. Show missing or conflicting evidence as `Unverified` or `Needs review`; never generate a family head from an LLM answer.
5. Keep sensitive account/identity fields, OIDC credentials, and database URLs off the browser and out of responses.

**Gate:** Playwright covers all three roles, sign-in expiry, permission denial, proposal, approval, and no-access states.

## Phase 5 — Verification, security review, and delivery

### Mandatory negative and concurrency cases

- Anonymous `POST /api/query`, upload, and source-status requests: `401`, no side effects.
- Viewer upload or family-head assignment: `403`, no side effects.
- Editor role assignment, account invite, or family-head approval: `403`.
- Admin actions: success with valid input, audit entry created.
- Forged or expired token, wrong issuer/audience, altered role claim: denied.
- Cross-corpus source collision, embedding retrieval, keyword retrieval, citation, status, and ingestion: no leak or overwrite.
- Third branch only; first-born male only; verified living only. Female, younger male, different branch, deceased, unknown or disputed living status, and incomplete parent/birth-order evidence: rejected.
- Two concurrent approvals for branch 3: at most one commits.
- Status changes after an approved designation: title does not continue as an unreviewed active designation.
- Permission changes take effect without trusting stale client role claims.
- Admin actions emit actor, action, target, timestamp, and outcome, but never secrets or unnecessary living-person details.
- Genealogy imports, embeddings, and RAG citations cannot bypass source-provenance checks.

### Commands and milestones

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
make rust-check
make web-check
make web-e2e
```

Run focused tests after local changes; rerun the affected web/Rust checks as needed. Run the full integration gates at completion. If migrations change, verify clean install and upgrade against a copy of existing data, including rollback/backup instructions. Never weaken an acceptance check to obtain a green result.

### Definition of done

1. New migrations and code are covered by deterministic tests; all supported test gates pass or record a concrete environmental blocker.
2. Permissions are enforced by Rust services; browser UI only reflects them.
3. Genealogy Head of Family eligibility is enforced server-side and auditable.
4. Existing indexed family documents remain intact; identical filenames in distinct corpora are isolated.
5. `docs/` describes setup, invite/admin operations, backup, role matrix, eligibility evidence, unresolved succession rule, and recovery.
6. Open a PR with summary, test evidence, migration safety, rollout considerations, and remaining risks. Do not merge until approval and required CI gates pass.

## Stop conditions / explicit decisions

- Stop before production/public deployment if an OIDC provider, TLS, secure session configuration, or administrative bootstrap is not available.
- Do not approve any real Head of Family if canonical third-branch mapping, first-born male evidence, or recent living-status verification is missing or disputed. Keep the candidate **pending**.
- Do not infer successor eligibility when the first-born male has died. Request a separate family-policy decision before changing succession behavior.
- Stop before potentially destructive migration or reassignment if data must be reclassified without verified provenance. Require a backup and operator approval.
- Continue automatically through nonblocked tasks; ask for approval only at genuine security, data, or merge gates.

## SOP execution instruction

Implement this plan end to end in `imhttran/tran`. Create a feature branch, fix the verified corpus isolation defect first, then add auth/RBAC, Head of Family rules, UI, tests, and documentation. Preserve unchanged passing evidence. Run focused verification after each phase and full integration verification before PR publication. Continue without repeated prompts until a real blocker or approval gate is reached. Do not assign a person as Head of Family unless all required source evidence is verified; do not invent an alternative succession rule.
