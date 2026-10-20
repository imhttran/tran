# PLAN-TRAN-BULK-ONBOARDING — Reviewed batch preparation for living family members

**Status:** PROPOSED — design/implementation handoff only. No real invitations, account activation, or email delivery.
**Repository:** https://github.com/imhttran/tran
**Suggested SOP ID:** `plan-tran-bulk-onboarding`
**Canonical requirements:** `docs/plans/PLAN-TRAN-USER-ONBOARDING.md` and `docs/plans/PLAN-TRAN-USER-ONBOARDING-SOP.md`
**Related work:** PR #23 merged (verified living-person candidate classification, usernames, reservations, and hashed invitation tokens); PR #25 merged (reviewed identity linking); PR #26 is an **open draft** implementing USR-003D single-member admin invite/link endpoints. Re-read GitHub state before execution; never build a competing version of PR #26.

## Outcome / explicit boundaries

Provide an admin-only **Bulk Onboard** action in the existing **Admin → Family Members** UI. The operator can:

1. Preview who is independently **verified living**, with evidence/review flags.
2. Select all eligible candidates or choose individually, then correct/confirm any ambiguous name parts, username collisions, or intended recipient emails in the **private UI**.
3. Review an exact, immutable batch summary and **Prepare selected**. Preparation reserves usernames and stores a resumable, audited onboarding batch; it does **not** create active OIDC credentials, grant access, email anyone, or mark a person as verified.
4. Review prepared entries and **Approve sending invitations**, explicitly and separately, only after an OIDC provider / delivery adapter / recipients are configured and approved. A provider-managed invitation or secure one-time token is delivered without showing bulk plaintext tokens to the browser.
5. Track statuses, failures, skips, and retries for each member. Members activate through the approved identity provider and are linked to the stable genealogy person only after a verified, reviewed account-to-person association.

**Keep the MVP simple.** One private table with filters, checkboxes, a preview/confirmation dialog, a batch result screen, and a resend/retry action. No general-purpose workflow engine, no new genealogy person tables, no automatic branch-reviewer grant, no auto appointment to Head of Family, no custom password database, and no bulk password resets.

## Ownership and security

- **GENEALOGY_DATABASE_URL** owns private person IDs, reviewed living assertions, evidence, username reservations, invitation lifecycle, person-account links, and the minimum necessary batch metadata. Reuse TRAN-002 and existing `src/onboarding` modules; add a minimal additive batch table only if persistent batch/idempotency tracking cannot be achieved with existing tables.
- **DATABASE_URL** owns OIDC `(issuer,subject)` identity, `users`, roles, grants, and identity audit. A genealogy `person_key` or generated username is **not** an OIDC subject, password, or proof of the invitee's identity. No cross-database FK; resolve OIDC identity through the reviewed linking flow after activation.
- All preview, prepare, invite, retry, status and recipient operations require authenticated **application admin**, enforced by Rust before reading either database; Next.js proxy requires CSRF for mutations. No caller-provided role or branch claim is trusted.
- Preserve the `[lastInitial][MiddleInitial][FirstName]` policy and case-insensitive ASCII/Vietnamese normalization, deterministic collision suffixes, and admin confirmation on ambiguous name segmentation. No birthday-based passwords or default `admin` credentials.
- The repository is public. No real living-person names, emails, birth dates, candidate counts, source pages/excerpts, credentials, invitation tokens or private source hashes in commits, CI, screenshots, logs, or tracked acceptance evidence. Admin's private authenticated UI may show authorized names and contact details as necessary.
- `Head of Family` is a separate verified genealogical title. Bulk onboarding grants **no** system admin, branch-reviewer, or Head of Family authority. Reviewer grants require their separate explicit admin workflow.
- Protect minors/uncertain age with appropriate guardian/consent review; absence of exact DOB alone does not invalidate a verified living adult. Do not infer living status from a birth/death year, RAG output, family name, or lack of a death record.

## Functional rules

### 1. Eligible candidate preview

Reuse `GET /api/admin/family-members/candidates` and `onboarding::candidate::preview_candidates`. One candidate is selectable only if all are true:

- Cataloged stable person identifier, a `vital_status` living assertion with `review_status = 'verified'` and attributable original-source citations;
- No contradicting deceased/disputed/needs-review status, unresolved duplicate, or ambiguous identity;
- No active reviewed person-account link and no still-valid pending invitation;
- No unresolved name parsing issue for requested username, and no recipient/identity verification blocker at **send time**.

Preview must distinguish `eligible_for_invite`, `needs_review`, `already_linked`, `invited`, `not_eligible`. Missing or unverified recipient email can remain **preparable** but must be **blocked from sending** until independently confirmed; never guess addresses from genealogy documents. With zero verified living candidates, return a valid empty table and review-needed explanation, not invented identities. Implement server-side pagination/filtering if needed; do not trust client-side filters to decide eligibility.

### 2. Dry run

Admin selects candidate IDs or `all_currently_eligible` explicitly. A dry-run reports:

- Ready to prepare, already prepared/invited/linked, needs review, missing recipient, ambiguous username, not eligible, and conflicts.
- Proposed username, scope and status in the authenticated admin UI only. Collision resolution uses existing transactional reservation logic on actual preparation; preview proposals are not promises of a reserved name.
- No SQL mutation, OIDC provisioning, email dispatch, token issuance, or role grant, even if repeated. Return a short-lived preview identifier/snapshot and expiration, or recheck directly on submit.

Make batch membership explicit: selecting all means all currently eligible candidates as evaluated on the server, not an uncontrolled future stream of newly imported relatives.

### 3. Batch preparation

Admin must press **Prepare selected** and confirm exact count (plus any exceptions). The backend re-evaluates each selected candidate under appropriate transaction/locking; never promote stale or conflicting candidates.

- Reserve username transactionally and create/update a durable, per-person batch item with `prepared`, `skipped_*`, `needs_review` or `failed` status, reason code and administrative audit.
- Duplicate submissions and restarts are idempotent: same person cannot get duplicate active reservations, prepared invites or links. Use DB constraints and stable idempotency keys, not in-memory dedupe.
- Process in bounded chunks (e.g. 50 or fewer per chunk) with per-person commits and a resumable summary. If one row fails, preserve successful rows and report exact safe errors. Do not hold a transaction across an external email/provider call.
- Account **preparation is not account activation**. A generated alias is not the OIDC identity. No user role/corpus grant is given until the verified activation/linking policy completes.
- Re-run eligibility before token generation/dispatch and again before activation, especially if living evidence is revoked/disputed.

### 4. Invitation sending — separate explicit gate

After reviewing the prepared list, the admin may select a subset and explicitly approve **Send invitations**. This requires verified contact addresses and an explicitly configured/approved OIDC provider and delivery adapter. An unconfigured adapter returns `not_configured` without claiming delivery. No automatic sending from migration, initial batch prepare, a PR merge, SOP, or CI.

- Prefer provider-managed invitation/activation. Where supported by the existing application, use one-use, time-limited cryptographically random invitation tokens; persist only hashes, deliver securely to verified recipients, never dump bulk secrets to UI/logs/audit.
- Ensure a safe handoff from token creation to sending. Do not persist raw tokens in a new batch table; handle failed delivery with explicit retry/reissue and cancellation of superseded tokens.
- Track `approved_for_send`, `queued`, `provider_accepted`/`sent`, `failed`, `expired`, `accepted`, `linked`. Never claim an email was delivered merely because a job was queued or a provider accepted an API request.
- Throttle sends, require idempotency per person/invitation, audit actor, time, state and non-sensitive provider outcome. Double clicks and retries cannot create duplicate credentials or send multiple active invites.
- Activation requires real OIDC authentication and admin-reviewed linkage to the correct catalog person; only then provision/enable permitted viewer access if policy approves. The application never derives a password from birth date or family name.

### 5. Admin UI

Use `/admin/family-members` and the existing application shell and Next.js authenticated proxy. Keep the UI simple:

- Search/filter candidate table, multi-select, **Select eligible**, **Preview batch**, **Prepare selected**.
- A confirmation modal with exact selected/preparable/skipped counts, username exception review, warning **No accounts activated or emails sent**.
- Results screen showing prepared, needs review, already linked/invited, failed and retryable entries.
- Separate **Send invitations** action, disabled if provider/verified contacts are unavailable; display queue/delivery states without false success.
- Per-row status and audit/history; link to existing individual member review. No member profile editor or branch-review workflow is required for this focused feature.
- Responsive and accessible interaction; keyboard operation, sensible empty/error/loading states. Member/reviewer/non-admin requests get 403 and UI denies admin page access.

## SOP implementation stages and acceptance

### BULK-001 — Preflight and reconcile dependencies

- Fetch latest `origin/main`; inspect running SOP executions, dirty/untracked worktrees, migration inventory and PR #26. **Do not start bulk integration on top of an unmerged competing USR-003D branch.** If PR #26 is still open, implement the batch design/tests with a clearly recorded dependency and avoid conflicting endpoints; finish integration after the reviewed API is merged.
- Record route/permission/data-owner contracts, present invitation delivery capability, email configuration and whether there is any verified living private data. Never emit actual names or counts in tracked notes.
- Confirm exact versioned migrations; do not assume the next migration number (FAM-002 also has planned migrations).
- Create a new isolated worktree and a **distinct SOP plan ID**; preserve historical onboarding SOP execution.

**Pass:** dependency matrix and privacy-safe baseline with no writes to real databases.

### BULK-002 — Read-only selection and dry-run

- Reuse current candidate and username modules. Provide admin-only explicit selection / dry-run contract with a stale-preview guard and reason classifications.
- Add synthetic tests for verified living, no evidence, conflicting death status, already-linked, invited, ambiguous name, no verified contact email, missing DOB, minor/guardian review, zero candidates and stale selections.

**Pass:** preview/dry-run have zero mutations; no unauthorized existence leaks or eligibility bypass.

### BULK-003 — Resumable, idempotent preparation

- Add minimal additive batch persistence only where needed, transactionally reserve names, record selected person IDs and outcomes, revalidate evidence before mutation; retain audit history.
- Handle retries, concurrent batch requests, partial failures and unique username collisions without creating active OIDC accounts or emailing people.
- Use synthetic disposable PostgreSQL genealogy and identity databases. Protect the distinct DB ownership boundaries.

**Pass:** preparing the same batch twice yields one logical prepared entry/reservation per person; failures are recoverable and never elevate access.

### BULK-004 — Explicit sending adapter

- Reuse the approved USR-003D invitation/link endpoints and existing one-time tokens. Add a small provider/mail adapter interface with fake test implementation and a real adapter **only if the operator has actually approved provider details**.
- Confirm verified recipient and admin approval separately from preparation; dispatch with throttling, status, audit, idempotency and safe failure handling.
- Do not implement SMTP passwords, an arbitrary provider choice, plaintext credential email, or birth-date passwords.

**Pass:** fake adapter proves one approved send, retries and no sends during preview/prepare. Real sends remain disabled until operator config/approval.

### BULK-005 — Admin UI and end-to-end validation

- Implement the private candidate table, selection, preview, prepare, result summary, optional separate send-confirmation and status.
- E2E: admin previews 6 synthetic people, 2 eligible and 4 blocked; selects eligible; dry-run makes no writes; prepare creates only 2 reservations/staged records; retry creates none extra; provider unconfigured blocks sending; configured fake adapter sends exactly 2 on explicit approval; synthetic user identity claims are independently reviewed; non-admin is denied.
- Run applicable Rust, migration, security and Playwright gates, inspect staged and committed diffs for personal data, and report exact evidence.

**Pass:** all mandatory tests and CI pass; PR remains unmerged until approved; no real accounts or email changed.

## Delivery expectations

- One implementation PR for BULK-001–005 **after** the required onboarding invitation API is ready; if still blocked, publish only independent pieces, explicitly label unaccepted integration criteria and do not claim bulk onboarding complete.
- Include synthetic acceptance matrix, safe migration clean-install/upgrade evidence, batch-level state diagram, admin screenshots with synthetic records only, and provider deployment checklist.
- Use supported SOP task execution, bounded retries, and approval gates; do not mark a task PASS when only mocks exist.
- Do not merge, deploy, send real emails, invite real relatives, bootstrap an admin, grant family-review permissions, modify private genealogy records, or appoint a real Head of Family without explicit operator approval.
