# PLAN-TRAN-USER-ONBOARDING — Living Members, Profiles, and Branch Review

**Status:** PROPOSED; design/implementation plan only; no real accounts, invitations, passwords, or family facts are created by this document.
**Repository:** https://github.com/imhttran/tran
**Suggested SOP identity:** `plan-tran-user-onboarding`
**Dependencies:** TRAN-002 genealogy provenance/eligibility; TRAN-003 identity/OIDC; `docs/plans/PLAN-TRAN-FAMILY-TREE.md` profile and asset contract; latest merged authorization fixes.
**Privacy:** The repository is public. Never commit names of actual living relatives, dates of birth, account usernames, personal documents, photos, source excerpts, passwords, hashes of private source files, or real-family inventory counts.

## Goal

Identify **verified living people** in the existing private genealogy catalog; allow a system administrator to review and prepare their family-member invitations; link each activated OIDC account to exactly the correct genealogy person; support self-service profile/photo/document updates and evidence-backed review; authorize a designated branch reviewer to maintain and verify shared data **only within their assigned branch**; retain an independent system-admin role.

The product must distinguish a *genealogy person*, an *invitation*, a *verified OIDC account* and a *permission grant*. A person entry must not automatically become an authenticated account on the basis of a name.

## Existing architecture and precedence

- `GENEALOGY_DATABASE_URL` owns `person`, `family_branch`, `assertion`, `source_evidence`, `assertion_evidence`, review history, and Head of Family appointments. Do not duplicate these objects in the RAG database.
- `DATABASE_URL` owns existing OIDC users/roles, corpus grants, and audit events. OIDC `(issuer, subject)` is the immutable authentication identity. A username is a login/display alias, **not** the security principal.
- Existing `/api/account`, admin provision/deactivate routes, genealogy read and review/appointment services, and the Next.js authenticated proxy should be extended, not replaced.
- The family-tree plan already specifies `/profile` with one photo and one personal document, owner-private defaults, controlled account/person linking, review-gated indexing, and general document-review requirements. Reuse its storage/review workflows rather than implement a second asset engine.
- **Precondition:** inspect the *current* identity database migration and `provision_user`. The historical implementation has `users.subject UNIQUE` and `ON CONFLICT (subject) DO UPDATE SET issuer = ...`. If these remain, fix or depend on a verified remediation before bulk provisioning. Never transfer privileges between issuers.
- The existing genealogical `Head of Family` designation is limited by the evidence-backed *verified, living, first-born male of the third branch* rule. It is **not** an application role. This plan does not appoint a new family head or authorize succession.

## Roles and permissions

| Capability | Family member (viewer + self service) | Branch reviewer, scoped to granted branches | System admin |
| --- | --- | --- | --- |
| View authorized family tree | Yes | Yes, within granted scope | Yes, within explicit authorized scope |
| Edit own private profile/photo/personal document | Yes | Yes | Yes |
| Submit changes to shared genealogy facts | Own-person changes, pending review | Propose/correct within assigned branch | Propose/correct system-wide |
| Verify another member's submitted branch facts | No | Yes, **only** within explicit branch grant | Yes |
| Verify own submissions | No | No | No; another authorized reviewer required |
| Change other members' private contact details or owner-private assets | No | No | Only through explicitly authorized, audited support/review access |
| Approve/reject publication of personal documents | No | Only with separately assigned document-review permission | Yes with explicit documented review purpose |
| Create invitations, manage username conflicts and links, revoke grants | No | No | Yes |
| Assign/revoke application admin and branch review rights | No | No | Yes |
| Appoint/revoke genealogical Head of Family | No | No, unless also independently granted application admin | Yes, subject to existing eligibility policy |

**Branch reviewer** is an application authorization grant such as `branch_reviewer` + `branch_key` (or a branch-scoped capability), not the same as the historical title `head_of_family`. When the verified Head of Family has an account, the administrator may **explicitly grant** reviewer authority for their own branch; the genealogical title must never automatically grant privileged access. Support delegated branch reviewers without inventing new genealogy titles.

"Update everything under their branch" means maintain **shared, reviewable genealogy content within the branch**. It does **not** authorize changes to other adults' private profile assets, account credentials, personal contact fields, global system settings, audit history, original evidence, or other branches. Handle cross-branch edges by requiring both applicable permissions or a system-admin review.

## Living-person candidate inventory

1. Read from the **private genealogy database** only; never use inferred age, absence of death year, RAG answers, or unverified source mentions as proof of being alive.
2. Require a reviewed `assertion_kind='vital_status'` with verified **living** evidence and attributed citations. Reuse the current TRAN-002 convention (`assertion_key='living'`) and validation code; do not invent alternative assertions.
3. Exclude or hold candidates with contradictory death evidence, disputed review, missing original citations, ambiguous person keys/branch membership, or unresolved duplicate identities.
4. Mark persons with incomplete or conflicting evidence `needs_review`, **not eligible for automatic invitation**. Do not publish names or inventory counts to GitHub/CI.
5. Treat minors/unknown age as review-required; require appropriate guardian involvement and privacy defaults before an invitation, if local policy allows accounts for minors.
6. Offer `dry-run` / preview with outcomes `eligible_for_invite`, `needs_review`, `already_linked`, `invited`, and `not_eligible`. Review privately before the admin confirms a batch. Do not create active logins or send invites from read-only preview.
7. Be idempotent and restartable. Re-runs must not duplicate account links, invitations, usernames, audit entries, or create new authorizations without approval. Revalidate status immediately before activation.
8. If **zero** verified living people exist, show an empty eligible list and a review backlog; do not infer family members or create fake real accounts.

## Username policy

Requested format: `[lastInitial][MiddleInitial][FirstName]`.

- Use verified structured family/middle/given names; for Vietnamese names, family name is usually first and given name last, but do not guess ambiguous parsing.
- Example with **synthetic** name: `Nguyen Van An` -> display username `NVAn`, canonical case-insensitive key `nvan`.
- Normalize Vietnamese diacritics for the account alias, preserve the original display name, and specify deterministic treatment of `Đ/đ`. Never mutate `person_key`.
- For multiple/missing middle components or compound given names, use a documented parse rule with an admin confirmation opportunity. Do not silently invent initials.
- Enforce case-insensitive uniqueness in the account/invitation namespace. Use a documented, non-sensitive collision suffix such as `nvan2` with admin confirmation; never use a birth date in the suffix.
- Reserve an alias during approved invitation; keep it stable after profile edits. Name changes are separately reviewed and do not silently rebind login identity.

## Temporary-password policy and identity provider

User-proposed initial password: `[FirstName][MMDDYYYY birthday][tr@n]`.

**Do not implement this deterministic password in production.** It can be guessed from genealogy names/birthdays, some records store only a birth year, and the application uses OIDC rather than an internal password database.

- Prefer a provider-managed, expiring, single-use invitation or activation link requiring the member to establish their own password/passkey and MFA where supported.
- If the provider requires an initial credential, use a cryptographically random high-entropy one-time value, delivered out-of-band, expiring promptly, and forced to change on first use. Never store/log/email plaintext credentials in GitHub, RAG, audit trails, analytics, or fixtures.
- Do not invent a real OIDC provider or its management API. Provider-specific provisioning and delivery remain deployment gates. Fully synthetic invitations/accounts are permitted in tests.
- Date of birth must not be required to generate account credentials. Retain only minimum private verification data where explicitly approved.

## Identity and link lifecycle

`candidate -> admin-reviewed invitation draft -> invited -> provider-activated -> identity-claimed -> linked/active`.

- Invitations reference a stable `person_key` only within the private service/database boundary and reserve a username; they are not active access grants.
- Use one-time invite verification and owner confirmation, then bind the authenticated OIDC `(issuer, subject)` to that person **after admin/approved identity proof review**. Never claim by display name, guessed username, surname, birthday, or account email alone.
- Enforce one active account-to-person link per person, with explicit exception/recovery for verified account replacement. Do not silently merge two records.
- Keep cross-database relations as stable opaque references with server-side audit and compensating/idempotent operations; do not create cross-database foreign keys.
- An unlinked account can edit its own private account profile but cannot edit genealogy claims for a person until its link is verified.
- Provide resend, expire, cancel, revoke, deactivate, and audited recovery workflows; enforce last-usable-admin protection and prevent deactivated users from accessing any protected records.
- Bootstrap the **system administrator** using the existing explicit authenticated operator process. No default `admin/admin`, hard-coded password, or automatic bootstrap from genealogy data.

## Profile, media, and submission review

Support member-managed fields with per-field grants and visibility, e.g. display name, preferred/alternate name, biography, profile picture, personal document, and explicitly approved contact fields. Genealogical facts such as birth/date, relationships, branch affiliation, living/deceased status, and source citations are separately versioned **proposals** rather than immediately verified truth.

- Member edits create versioned `draft/pending_review` submissions with actor, timestamp, original value, proposed value, and evidence/attachments. The member may preview a draft privately; authorized viewers continue seeing the last approved value.
- Photos are private by default; validate size/type and decode/re-encode to strip metadata, scan unsafe content, use private object storage, and authorize previews/downloads. Owner may choose broader visibility after review.
- One personal document follows the existing family-tree plan; add extra documents or assigned fields only by a separate explicit configuration, not hardcoded ad hoc permissions.
- Review workflow: `draft -> submitted -> approved | rejected | needs_changes`; every decision records reviewer, branch grant, reason, evidence ID(s), timestamp, and immutable history. Require a distinct reviewer from the submitter; admin is not exempt.
- A reviewer can approve a *proposed shared fact* only with the required original-source evidence. A user-entered birth date or self-attested `living` is not automatically accepted as verified genealogy evidence.
- On approval, update the appropriate authoritative record and revision atomically (with conflict checks). On disagreement, retain historical approved value and mark conflict. Never overwrite original source evidence or source provenance.
- Only approved family-visible content may enter RAG/indexing. Owner-private documents remain outside retrieval unless explicitly submitted, authorized, approved, and indexed per `PLAN-TRAN-FAMILY-TREE.md`.
- Changing approved genealogy evidence must trigger review/re-evaluation of any affected Head of Family eligibility without auto-appointing a successor.

## API and screen contract

**Admin /family-members**: private eligible-living inventory (preview only); evidence flags; explicit batch invite confirmation; collision resolution; pending/active status; linked genealogy person; role/grant assignment; expiring/resend/revoke invitations; last-admin guard. No real names/PII in operational logs.

**Member /profile**: edit own approved personal fields; upload/replace one photo and one personal document; manage private visibility; view current verified versus draft versions and review decisions; request/confirm person link; no branch/global edits.

**Branch /branch-review**: list only assigned branch's queued submissions, with citations/diff; approve/reject/request changes for another person's submitted revisions; propose shared branch corrections; report ambiguous cross-branch claims for admin review.

**Rust API**: endpoints for eligible preview (admin only); invite lifecycle (admin only); self-account/person link request; own-profile reads/writes; media upload; submission review; branch-grant management; decision/audit history. Explicitly define contracts with stable IDs, cursor pagination, authz before DB reads, and `401/403/404/409` safe error behavior. Never trust client-provided role, branch, person key, reviewer, or approval status as authorization.

The Next.js proxy and OIDC session remain the trust boundaries. Use CSRF validation on cookie-authenticated mutations and avoid sensitive PII in analytics, screenshots, CI, URLs, and error bodies.

## SOP delivery sequence

1. **USR-001 — Preflight/security prerequisite:** inspect latest `origin/main`, existing plans, open SOP executions and merged fixes; verify issuer+subject immutability and admin-deactivation safety; preserve dirty trees and historical runs.
2. **USR-002 — Living candidates and dry-run:** implement evidence-based candidate query, exclusion/review reasons, privacy-safe preview, idempotent reservation, synthetic tests. **No real account side effects.**
3. **USR-003 — Invitations/accounts and linking:** additive migrations, username policy, provider-independent invitations, synthetic activation, immutable OIDC link with audit, expiry, collision handling, admin seed procedure.
4. **USR-004 — Self-service profile and assets:** reuse `PLAN-TRAN-FAMILY-TREE.md` versioned asset and ownership model; implement draft read/write, photo and personal document control, private access tests.
5. **USR-005 — Scoped branch review:** explicit branch reviewer grants, query/mutation scoping, no self-approval, cross-branch conflicts, audit and tests; retain genealogical title separation.
6. **USR-006 — Review and publication:** field-level changes, cited original evidence, approval/rejection/needs-changes, verification/version snapshots, notifications/status, Head of Family evidence re-evaluation; ensure approved-only indexing.
7. **USR-007 — UI and end-to-end acceptance:** admin invitation list, member profile, branch-review queue, browser and Rust integration with synthetic people and a synthetic OIDC authority, zero-leak negative tests, docs, and PR.

Decompose these tasks according to SOP's supported plan format; this document is the canonical feature requirements, **not** a re-open of any historical SOP run. Execute in an isolated worktree and new feature PR. Review the current status of `PLAN-TRAN-FAMILY-TREE.md` to avoid two teams implementing the same profile/asset system.

## Mandatory acceptance

- Living inventory correctly excludes no-evidence, deceased, disputed, conflicting, missing-DOB, minor-without-guardian-consent, already-linked, or ambiguous people from automatic activation; uncertain age/DOB should block only when actually needed for consent, not falsely classify adults as deceased.
- Dry-run makes zero account changes. Admin-approved invitation is repeatable without duplication. No invitations or real account activations run until the operator approves both identity provider and recipient list.
- Username alias algorithm passes Vietnamese/ASCII normalization, missing/compound name, collision and idempotent retry tests. No password is generated from birthday or surname; authentication is provider-managed.
- Identity linking prevents cross-issuer privilege transfer and person-link hijacking. Non-members and impersonators cannot edit another person's profile.
- Branch reviewers can verify another person's submissions **inside assigned scope only**; they cannot self-approve, cross branches, change system roles, access private photos/contact fields without explicit delegation, or create/revoke a Head of Family designation.
- Admin manages users, invitations, reviewer assignments and audit, but actual family facts still require cited evidence and a distinct reviewer.
- Verified family facts and RAG results never incorporate unapproved submissions, owner-private materials, or fabricated genealogy.
- Separate RAG and genealogy database guards remain enforced; migrations are additive, clean-install/upgrade-safe, and test against disposable databases only.
- Test `cargo fmt --check`, `cargo build --locked`, `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`, `make rust-check`, `make web-check`, `make web-e2e`, `git diff --check` plus focused migration/security/permission/browser tests.
- Before pushing, verify `git ls-files docs/tran/` is empty and staged diffs contain no real people, dates, passwords, private media, source excerpts or hashes. Use synthetic fixtures only. All security-critical negative tests must pass.
- Report implemented code and acceptance separately from environment gates: production OIDC provider/admin bootstrap/TLS, genuine real-person eligibility/identity verification, guardian consent where needed, and explicit live invitation approval.

**Do not bulk create real login accounts, assign branch reviewer grants, or bootstrap another production admin as side effects of tests or merging a feature PR. Do not automatically deploy or merge.**
