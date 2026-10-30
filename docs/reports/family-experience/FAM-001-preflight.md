# FAM-001 preflight — Family archive reconciliation

**Status:** Complete (documentation only). FAM-002 through FAM-007 are not started.
**Stage:** FAM-001 (Reconcile) from [PLAN-TRAN-FAMILY-TREE.md](../../plans/PLAN-TRAN-FAMILY-TREE.md).
**Scope:** Reconcile current `main`, record the route/permission/data-owner matrix,
record the RBAC and genealogy dependency map, confirm the legacy-document inventory
approach, and record baseline verification evidence.

This report uses synthetic and public-safe content only. It contains no private
family names, source excerpts, content hashes, or living-person details.

## Baseline reconciliation

| Item | Value |
| --- | --- |
| Current `origin/main` | `d7624f3` (PR #10–#15 merged) |
| Baseline inspected inside the plan text | `cd9a5db` (PR #9–#12 merged) |
| Plan text | Written against `cd9a5db`; not rewritten by FAM-001 |
| Delta recorded | Current main is later than the plan baseline. PR #13 (legacy-document policy), PR #14 (nomination decisions and last-admin guard), and PR #15 (cleanup and legacy-route removal) landed after `cd9a5db`. |

FAM-001 records both revisions. It does not rewrite the historical plan text. Any
post-merge remediation is tracked separately and does not reopen historical SOP runs.

## What already exists (merged work)

- **RBAC/OIDC/genealogy/Head of Family (PR #10):** Rust Axum routes, issuer +
  subject bearer identity, database roles and corpus grants, OIDC
  login/session/CSRF, and the genealogy record, person, relationship, nomination,
  appointment, account, and administrative operations.
- **RBAC hardening (PR #14):** explicit nomination decisions and a transactional
  last-active-administrator guard covering both deactivation and role revocation.
- **Cleanup and legacy-route removal (PR #15):** the non-persisting
  `/api/admin/head-of-family/approvals` route is gone; nomination, approval, and
  revocation run through the genealogy endpoints.
- **Rust best-practices (PR #11):** Rust application-layer review work merged.
- **Family archive plan and mockups (PR #12):** the four-screen design and this plan.
- **Legacy-document policy (PR #13):** the preserved quarantine/backfill and
  review-before-promotion policy summarized below.
- **Next.js surfaces:** genealogy, Head of Family, session, sign-in, and query
  surfaces, plus the authenticated proxy and sign-in flow. Reuse them; do not add
  a second shell.

## Route, permission, and data-owner matrix

The four family-archive screens are **Family tree**, **Ask the archive**, **My
profile**, and **Documents**. The matrix records each relevant existing route with
one permission and one data owner. Routes are recorded from the merged baseline
(`d7624f3`); the permission names are the product policy in the plan's product
behavior table.

### Rust Axum routes (reused, not recreated)

| Route | Permission | Data owner |
| --- | --- | --- |
| `GET /api/genealogy/records` | Read permitted tree (viewer+) | Genealogy database (`GENEALOGY_DATABASE_URL`) |
| `GET /api/genealogy/head-of-family` | Read permitted tree (viewer+) | Genealogy database (appointment tables) |
| Person detail reads | Read permitted tree (viewer+) | Genealogy database |
| Relationship detail reads | Read permitted tree (viewer+) | Genealogy database |
| Nomination reads | Read permitted tree (viewer+) | Genealogy database (appointment tables) |
| Nomination writes | Existing RBAC policy only | Genealogy database (appointment tables) |
| Appointment reads | Read permitted tree (viewer+) | Genealogy database (appointment tables) |
| Appointment writes | Existing RBAC policy only | Genealogy database (appointment tables) |
| `GET /api/account` | Own account (viewer+) | RAG database (account/profile) |
| Admin role operations | Admin only | RAG database (RBAC tables) |
| Admin user operations | Admin only | RAG database (RBAC tables) |
| Admin corpus-grant operations | Admin only | RAG database (corpus grants) |

### Next.js surfaces (reused, not recreated)

| Surface | API calls | Permission | Data owner |
| --- | --- | --- | --- |
| Family tree (`/genealogy`) | Genealogy record, person, relationship, Head of Family reads | Read permitted tree | Genealogy database |
| Ask the archive (`/ask`) | Existing query, retrieval, generation, citation services | Read permitted RAG answers | RAG database (approved/ready versions) |
| My profile (`/profile`) | `GET /api/account`; profile asset and person-link endpoints | Own account; link request/confirm | RAG database (account/profile) |
| Documents (`/documents`) | Upload, review, process endpoints (FAM-003/FAM-004 not yet built) | Editor upload; admin review/process | RAG database (documents + additive metadata) |
| Session and sign-in | Authenticated proxy; CSRF on mutations | OIDC login/session/CSRF | RAG database (session) |

### Product behavior (viewer / editor / admin)

Reproduced from the plan product behavior table to confirm expectations.

| Capability | Viewer | Editor | Admin |
| --- | --- | --- | --- |
| Read permitted tree, sources, and RAG answers | Yes | Yes | Yes |
| Manage own photo and one personal document | Yes | Yes | Yes |
| Upload general documents | No | Yes | Yes |
| Review, approve, reject, process, or retry general documents | No | No | Yes |
| Link an account to a genealogy person | Request/confirm own link | Request/confirm own link | Review/approve link |
| Change roles or Head of Family appointments | Existing RBAC policy only | Existing RBAC policy only | Existing RBAC policy only |

## RBAC and genealogy dependency map

| Dependency | Current status | Owner |
| --- | --- | --- |
| Issuer + subject identity | **Partial** — tokens carry both, but persistence is unique on `subject` and provisioning rewrites `issuer`; see [Known open gaps](#known-open-gaps-on-this-baseline) | Rust authorization layer |
| Database roles and grants | Exists (PR #10) | Rust authorization layer |
| Corpus grants | Exists (PR #10) | Rust authorization layer |
| Authenticated Next.js proxy + CSRF on mutations | Exists | Next.js proxy layer |
| Separate guarded `GENEALOGY_DATABASE_URL` | Exists; never fall back to `DATABASE_URL` | Rust persistence layer |
| TRAN-002 provenance and review workflow | Exists | Genealogy persistence layer |
| Pure eligibility and appointment rules | Exists | Genealogy persistence layer |
| Head of Family active-appointment revalidation | **Present** — each active appointment is re-evaluated against current verified evidence when listed (gap 2, closed) | Genealogy persistence layer |
| Additive document version/review/job metadata | Not present — belongs to FAM-002 | RAG persistence layer |
| Legacy backfill/quarantine migration | Not present — belongs to FAM-002 | RAG persistence layer |
| Profile asset ownership and visibility storage | Not present — belongs to FAM-002 | RAG persistence layer |
| Upload and review endpoints | Not present — belongs to FAM-003 | Rust application layer |
| Durable processing/job runner | Not present — belongs to FAM-004 | Rust application layer |

Rules for these dependencies:

- An account must be identified by verified issuer + subject. It is separate from a
  genealogy person. Users may keep an unlinked profile. Persistence does not yet
  enforce this; see gap 1 above.
- Match by a stable person key through a confirmed, admin-reviewed link; never
  link by name similarity alone. Prevent conflicting active links and audit link changes.
- Keep `DATABASE_URL` and guarded `GENEALOGY_DATABASE_URL` separate. Never fall back.
- Display Head of Family only from an authoritative active appointment.
- Eligibility requires verified living status, male status, first-born male birth
  order, and membership in the third branch. Unknown/deceased status requires the
  separate succession rule. FAM-001 appoints no Head of Family and infers no successor.
- New access scopes are proposed product policy. Do not write a proposed FAM-002
  scope grant as if it already exists.

### Known open gaps on this baseline

FAM-001 is documentation-only and fixes none of these. They are recorded so later
FAM tasks do not build on a false assumption. Each needs its own authorized
remediation change.

| # | Gap | Evidence | Status |
| --- | --- | --- | --- |
| 1 | Cross-issuer account reassignment | `migrations/006_identity.sql` declares `subject TEXT NOT NULL UNIQUE`; `src/identity_store_pg.rs` provisions with `ON CONFLICT (subject) DO UPDATE SET issuer = EXCLUDED.issuer`. A second issuer presenting the same subject can take over the existing account and inherit its roles and corpus grants. | OPEN |
| 2 | Head of Family living-status revalidation | `src/genealogy/appointments.rs` `list_active_appointments` filters on `a.status = 'active'` only. It does not re-check living, male, first-born, branch, or verified-evidence status, so a candidate who later becomes deceased, disputed, or unverified is still presented as the active Head of Family. | CLOSED — `list_active_appointments` now re-runs the eligibility decision for each active appointment; an ineligible holder is returned as `needs_review` with its reason code and is never presented as the verified head. |
| 3 | Last active administrator | Guarded by PR #14: `ensure_not_last_admin` takes a `pg_advisory_xact_lock`, excludes deactivated users, and runs for both deactivation and admin revocation; `tests/identity_last_admin.rs` covers it. | CLOSED (PR #14) |
| 4 | Legacy approval endpoint | Removed by PR #15. | CLOSED (PR #15) |

Gaps 1 and 2 are security-relevant. Until they are fixed, do not treat
"verified issuer + subject" or "active appointment means currently eligible" as
implemented facts.

### Dependencies only required by later FAM tasks

- Additive metadata/version/review/job migrations — FAM-002.
- Legacy backfill/quarantine migration and dry-run inventory — FAM-002.
- Profile asset ownership and visibility storage — FAM-002.
- Typed upload and review endpoints — FAM-003.
- Durable processing/job runner — FAM-004.
- Tree interaction and responsive catalog — FAM-005.
- Ask the archive UI and cited navigation — FAM-006.
- Shared navigation, responsive layouts, screenshots, handoff — FAM-007.

## Legacy indexed documents — inventory approach and policy

Existing `documents` rows came from direct ingest. They predate document versions,
review decisions, and processing jobs. Presence in `documents` is **not** evidence of
approval, scope, or provenance for the new workflow. No database was read or written
for this report, and no row content is included.

### Preserved quarantine/backfill policy (from the plan)

1. **Preserve every existing row.** The migration is additive and never deletes,
   rewrites, re-embeds, or re-labels an existing chunk/source. Take a database
   backup before applying it.
2. **Quarantine by default.** Create one `legacy_import` version per existing
   `(corpus, source)` group with `review_status = pending_review`,
   `processing_status = legacy_indexed`, `origin = legacy_import`, and no invented
   owner or access scope. The approved retrieval path only reads versions that are
   approved, ready, scope-authorized, and not removed.
3. **Inventory, do not guess.** Produce a dry-run report grouped by corpus/source
   with counts and content fingerprints (no private names or excerpts). Resolve
   ambiguous ownership or scope through operator review only.
4. **Promote only by explicit review.** An admin reviews the legacy version,
   assigns scope and owner where required, records an approval decision, and queues
   processing. No implicit approval, bulk approval, or automatic backfill promotion.
5. **Direct ingest cannot bypass review.** Legacy CLI/direct-ingest output is
   recorded as pending review and excluded from the new approved path. A browser or
   direct-API upload follows the same pending-review contract.
6. **Rollback is data-preserving.** The new metadata is additive. Keep the backup
   and leave additive tables inert or drop only the new metadata tables after
   verification. Never roll back by deleting documents.
7. **Admin/audit access is separate.** Access to quarantined legacy rows is
   permission-checked and audited. It is not family retrieval and must not leak
   private names, excerpts, or source details.

### No-deletion rule and review-before-promotion boundary

- No legacy row is deleted. Rollback is data-preserving. No records are deleted.
- Legacy rows stay outside approved retrieval until an explicit admin review,
  scope assignment, and successful processing promote them. This is the
  review-before-promotion boundary.

### Required migration acceptance cases (listed, not run)

These cases belong to FAM-002. FAM-001 lists them and claims no result.

- **Clean install:** all migrations apply to an empty database; no legacy rows;
  approved/ready retrieval works.
- **Upgrade:** a synthetic database with existing `documents` rows upgrades without
  changing row counts, content hashes, corpus/source identities, or citations.
- **Quarantine regression:** pre-upgrade corpus content is not returned by the new
  approved retrieval path, search, ask, or citations while pending.
- **Promotion regression:** after admin review, scope assignment, and successful
  processing, the version is returned and citations resolve. Rejection keeps it outside.
- **Rollback:** after backup and backfill, reverting the application leaves the
  original `documents` rows and citations readable by the prior path.
- **No implicit approval:** an upgrade or backfill alone never marks a legacy
  version `approved` or `ready`.

**Owner:** FAM-002 owns the additive migration and backfill. FAM-001 does not
implement it.

## Baseline verification evidence

SOP's configured Rust and Next.js validation gates are the authoritative repository
checks. Two evidence sets are recorded: the SOP-verified FAM-001 run, and a manual
re-run after `main` advanced.

### SOP-verified run

`prompt-20261020-194211` — outcome `VERIFIED`, lifecycle `LOCAL_DONE`, recorded at
baseline `e52b306`. Its acceptance check, `git diff --check`, passed.

Observed results from that run: all of the following configured checks passed:

- `cargo build --locked` — passed.
- `make rust-check` — passed.
- `make web-check` — passed.
- `cargo test --locked` — passed.
- `make web-e2e` — passed.
- `cargo fmt --check` — passed.
- `cargo clippy --locked --all-targets -- -D warnings` — passed.

### Re-run on `d7624f3` (2026-10-20)

PR #14 and PR #15 changed Rust and web sources after the SOP run, so FAM-001
re-ran the same gates on the refreshed baseline. All passed:

| Check | Result | Notes |
| --- | --- | --- |
| `cargo fmt --check` | PASS | direct invocation |
| `make rust-check` | PASS | runs fmt, build, tests, clippy, transition evidence |
| `cargo test --locked` | PASS | invoked by `make rust-check` |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS | direct invocation |
| `make web-check` | PASS | typecheck and production build |
| `make web-e2e` | PASS | 37 Playwright tests |
| `git diff --check` | PASS | no whitespace errors |
| `git ls-files docs/tran/` | PASS | 0 tracked files |
| `git check-ignore -v docs/tran/` | PASS | ignored by `.gitignore:20` |

No live database inventory, browser flow, or acceptance case was run for this
refresh beyond the existing `make web-e2e` suite. The migration acceptance cases in
the section above remain owned by FAM-002 and have no result here.

Configured command set, exactly as documented in the plans:

```
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
```

### FAM-001 acceptance checks

The deliverable is this report file. Its authorized SOP acceptance check is:

```
git diff --check
```

A `test -f` file-existence probe is redundant with the deliverable criterion and is
not an authorized SOP acceptance check; it must not be added to the plan as one.

### Historical SOP evidence

Historical SOP run artifacts under `.agent-sdlc/runs/` are preserved. FAM-001 does
not modify `.agent-sdlc/config.yaml`, `.agent-sdlc/state.db`, or any
`.agent-sdlc/runs/` artifact. It cites them by path only and restates no unverified
verdict.

## Preservation statements

- The four-screen mockups at [docs/mockups/family-experience/](../../mockups/family-experience/README.md)
  are preserved and unchanged.
- The privacy/RBAC requirements in [PLAN-TRAN-RBAC.md](../../plans/PLAN-TRAN-RBAC.md)
  and [PLAN-TRAN-RBAC-INTEGRATION-FOLLOWUP.md](../../plans/PLAN-TRAN-RBAC-INTEGRATION-FOLLOWUP.md)
  remain authoritative.
- Historical SOP evidence under `.agent-sdlc/` is preserved and unchanged.
- No application code, schema, migration, database, mockup, private genealogy data,
  or SOP state is modified by this change.
- FAM-002 through FAM-007 are **not started**. FAM-001 stops here.
