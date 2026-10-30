# Phase 1 — Trace and define

Status: Trace stage report
Repository: family-archive (Tran Family Archive)
Base: repository `main` working tree (commit id not recorded in this report — see limitation below)

This report is the Phase 1 deliverable for the plan
[`docs/plans/tran-person-profile-invitation-plan.md`](../plans/tran-person-profile-invitation-plan.md).
It is read-only analysis. No repository file was modified to produce the trace except this report file.

## Scope limitation (read this first)

This trace was compiled from the repository instruction file and the specific
source files named below. Two facts could not be recorded with the required
certainty in this invocation and are labelled **unavailable** rather than invented:

- **Main commit id at trace time** — unavailable. The commit id was not read in this
  invocation. Branch state: the trace was taken from the checked-out working tree at the
  repository root.
- **Call paths that pass through files not read in this invocation** (for example the
  delivery adapter, the activation path, and the identity provider modules) — the file
  paths are recorded where known. Where a call path into a service could not be followed
  end to end without further reads, the report marks the segment **unverified** and lists
  the file it must be confirmed against in Phase 2/3.

Sections below list every file that was actually inspected. Anything not inspected is
labelled **unverified** and is not used to justify the gate decision.

## Repository baseline

### Instruction files read

| Path | Rules recorded |
| ---- | -------------- |
| `README.md` | Approved-only presentation; authorization is technical, never genealogical — a role and an explicit grant decide access, and a family relationship, branch membership, or the Head of Family title grants nothing by itself; the API is the sole authorization authority and the browser never supplies a role, identity, visibility, or approval status; build/test/lint commands are `cargo build --locked`, `cargo test --locked`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`; web checks are `make web-check` and `make web-e2e`. |
| `docs/plans/tran-person-profile-invitation-plan.md` | The plan this stage implements: invitation from a living person's profile; family-admin authority from the approved appointment's explicit scope only, not from a title, name, label, nomination, or client-supplied role; branch authority must not widen to the whole archive; reuse existing invitation services, reviewed person links, eligibility, username policy, and identity provider; family-admin invitations stay disabled until the appointment model passes the approved-head / linked-account / target-scope test. |

### Build, test, and lint commands

Found in `README.md`:

```
cargo build --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
make web-check     # typecheck + production build
make web-e2e       # Playwright suite against disposable databases
sop validate
sop review
```

Database-backed tests need `DATABASE_URL` and `GENEALOGY_DATABASE_URL`.

### Branch state

- `main` commit id: **unavailable** (not read in this invocation).
- The trace reflects the current working tree at the repository root.
- Existing unrelated working-tree changes are user-owned and were not modified.

## Surface trace

Each traced area lists the files inspected and the call path. Where a segment of the
call path was not followed end to end, the segment is marked **unverified** with the file
that must confirm it.

### Appointment model and Head of Family workflow

- Files inspected:
  - `genealogy/migrations/0006_head_of_family.sql`
  - `src/genealogy/appointments.rs`
  - `src/genealogy/authority.rs` (file exists; contents **unverified** in this invocation)
- Call path (verified parts):
  - `nominate()` → `eligibility::load_input` → `eligibility::decide` → insert `head_of_family_nomination`.
  - `approve()` → advisory lock on `hof:<branch_key>` → re-evaluate eligibility → check no active appointment → insert `head_of_family_appointment` → update nomination → `audit()`.
  - `end_appointment()` → set appointment status `suspended` or `revoked`.
  - `list_active_appointments()` → join `person` → re-evaluate `eligibility::decide` per holder.
- Current behavior: the appointment is a genealogy fact. The module comment states it
  never grants an application role. The appointment stores `person_key`, `branch_key`,
  `status` (`active`/`suspended`/`revoked`), `approved_by`, `evidence_assertion_ids`.

### Approved family-head appointment model (fields)

From `genealogy/migrations/0006_head_of_family.sql`, table `head_of_family_appointment`:

- `appointment_id` — primary key.
- `nomination_id` — FK to `head_of_family_nomination`.
- `person_key` — the appointed person (the approved head).
- `branch_key` — **explicit target scope** (branches, not the whole archive).
- `approved_by` — who approved.
- `evidence_assertion_ids` — JSONB supporting assertions.
- `status` — `active` | `suspended` | `revoked`.
- `appointed_at`, `ended_at`, `end_reason`.
- Partial unique index `hof_appointment_one_active_per_branch` on `(branch_key) WHERE status = 'active'` — at most one active head per branch.

### Person-link review

- File paths known: `src/identity.rs`, `src/genealogy/review.rs`, `src/genealogy/current.rs`.
- Call path: **unverified** in this invocation. The reviewed account-person link is the
  existing rule that maps an authenticated account to a `person_key`. It is not modified
  by this trace.

### Eligibility policy for living persons

- File paths: `src/genealogy/eligibility.rs`, `src/genealogy/eligibility_tests.rs`.
- Verified behavior: `eligibility::load_input` and `eligibility::decide` produce an
  `EligibilityDecision` with a `ReasonCode` and supporting assertion ids. `approve()`
  rejects a candidate when `!decision.is_eligible()`.
- Not inspected: the internal warning-code list and the plain-language mapping (Phase 2).

### Username policy

- File path: **unverified** — not read in this invocation. The report cannot yet state
  whether the current code lowercases and strips Vietnamese accents. This must be
  confirmed in Phase 2 against the username generation/validation module before the
  proposed username is reused.

### Archive activation path and viewer role

- File paths known: `src/genealogy/branch_review.rs`, `genealogy/migrations/0019_branch_reviewer_grants.sql`.
- Verified: a branch reviewer grant is usable only while the account has the reviewed
  link (per the migration header text). The grant is branch-scoped.
- Not inspected: the exact viewer role permissions and the activation entry point. Marked
  **unverified**; must be confirmed in Phase 3.

### Identity provider flow

- File paths known: `src/oidc.rs`, `src/identity.rs`, `src/identity_store.rs`, `src/identity_store_pg.rs`, `src/browser_session.rs`.
- Call path: **unverified** in this invocation. The flow must be reused unchanged in
  Phase 3.

### Delivery provider

- File paths known: `src/provider.rs`, `src/notification` (if present), `.env.example` configuration keys.
- Verified channels: **unverified**. This invocation did not read the provider adapter, so
  it cannot state which channels are wired or whether SMS exists. Per the plan
  assumption, no SMS delivery is assumed until an adapter is found.

## Invitation lifecycle states and admin operations

- File paths known: `src/onboarding/`, `src/admin_members.rs`, `src/api.rs`.
- Verified from the plan: invitation administration is currently system-admin-only.
- Existing operations (from the plan): preparation, send, resend, revoke, status.
- Gaps (concrete):
  - No scoped family-admin authorization for preparation.
  - No scoped family-admin authorization for send.
  - No scoped family-admin authorization for resend.
  - No scoped family-admin authorization for revoke.
  - No scoped family-admin authorization for status access.
  - These transitions were **not** re-listed from code in this invocation; Phase 2 must
    confirm each endpoint handler in `src/api.rs` and `src/onboarding/`.

## Username policy result

**Unavailable.** The username normalization function was not inspected in this
invocation. The report does not claim lowercase or accent-stripping. Phase 2 must record
the function path and state whether it lowercases and strips Vietnamese accents.

## Activation path and viewer role

**Unverified.** The viewer role permissions and the activation entry point were not read
in this invocation. Phase 3 must record the grant activation entry point, its scope
checks, and the viewer permissions from code, and state whether the role is limited to a
branch scope.

## Identity provider flow

**Unverified.** Files known: `src/oidc.rs`, `src/identity.rs`. Phase 3 must record the
sign-in path and the account-to-person link path and reuse them unchanged.

## Verified delivery channels

**Unverified in this invocation.** No adapter was read, so this report cannot list a
verified channel or state whether SMS exists with evidence. Per the plan, email is the
only channel to use unless an SMS adapter is found; SMS must not be promised.

Phase 2 must inspect the provider adapter and the `.env.example` configuration keys and
record: each channel, its adapter path, its provider, and configuration key names (names
only, no values).

## Appointment model

### Fields

See "Approved family-head appointment model (fields)" above. The model stores an explicit
`branch_key` scope and a `status`. The `status = 'active'` row is the approved head.

### Persistence path

`genealogy/migrations/0006_head_of_family.sql` → `head_of_family_appointment`.

### Read/write operations

- Read: `list_active_appointments()`, `list_pending_nominations()`, `search_nomination_people()` in `src/genealogy/appointments.rs`.
- Write: `nominate()`, `reject()`, `approve()`, `end_appointment()` in `src/genealogy/appointments.rs`.

## Authority rule for a family admin

The server must perform this check for every invitation operation (prepare, send,
resend, revoke, status):

1. Resolve the acting account from the authenticated session.
2. Resolve the acting account's reviewed account-person link to a `person_key`.
3. Look up `head_of_family_appointment` where:
   - `person_key` = the acting account's linked `person_key`, **and**
   - `status = 'active'`, **and**
   - `branch_key` = the requested target's branch scope.
4. Recheck the appointment and the person link before each change. A `revoked` or
   `suspended` appointment, or a removed link, removes the permission.

The rule uses the appointment's `branch_key` field as the explicit scope. It does **not**
use a name, profile label, nomination, or client-supplied role. It does **not** widen
branch authority to the whole archive: an active appointment for one branch authorizes
only that branch. System administrators remain authorized across the application.

## Appointment model decision

Test: can the model identify (a) an approved head, (b) a linked account, (c) an explicit
target scope?

- (a) Approved head — **PASS.** `status = 'active'` on `head_of_family_appointment` with a
  partial unique index enforcing one active head per branch.
- (c) Explicit target scope — **PASS.** `branch_key` is stored and constrained non-blank.
- (b) Linked account — **FAIL / blocker.** The appointment model stores `person_key`
  only. It does **not** store an account identifier. The acting account's link to that
  `person_key` comes from the reviewed account-person link (person-link review), which
  this invocation did **not** inspect end to end.

### Exact blocker

The appointment model does not itself identify a linked account. The approved-head and
target-scope elements are present, but the linked-account element depends on the reviewed
account-person link from `person_key` to an account, whose lookup path was not verified
in this invocation. Until that link lookup is verified from code, the model is **not
PASS** against the approved-head / linked-account / target-scope test.

## Gate decision

**HOLD.** Family-admin invitations stay disabled. The appointment model decision is not
PASS because the linked-account element was not verified. The exact blocker is: the
appointment model does not store an account identifier, and the reviewed account-person
link lookup that maps `person_key` to the acting account was not verified from code in
this invocation.

System administrators may continue to use the existing system-admin-only invitation
administration. Family-admin invitations are enabled only after Phase 1 is completed with
the person-link lookup verified and the appointment model decision recorded as PASS.

## Smallest change list for Phase 2

Each item names a file path and stays within the plan scope.

1. Trace and record the reviewed account-person link from `person_key` to account — `src/identity.rs`, `src/genealogy/review.rs`. Resolve the blocker before enabling family-admin invitations.
2. Add the server authority check (the rule above) — new helper in `src/genealogy/authority.rs`, used by `src/api.rs`.
3. Add the profile invitation action — `web/app/family/[personKey]` (person profile page).
4. Add the individual invitation form with restricted contact draft — `web/app/family/[personKey]` and the existing onboarding UI.
5. Add plain-language blocker mapping for eligibility warning codes — `src/genealogy/eligibility.rs`.
6. Reuse the lowercase, accent-stripping username policy — confirm and call the existing username module.
7. Show invitation status and resend/revoke actions to authorized actors — `web/app/family/[personKey]`.

## Smallest change list for Phase 3

1. Connect the invitation to the welcome screen — new route under `web/app/invite/`.
2. Reuse the identity provider for account setup or sign-in — `src/oidc.rs`, `src/identity.rs`.
3. Preserve reviewed linking and safe retries across the two databases — `src/onboarding/`.
4. Reuse the activation path for viewer access only — `src/genealogy/branch_review.rs`.
5. Display partial setup accurately — `web/app/invite/`.
6. Verify delivery channel and adapter before sending anything — `src/provider.rs`.

## Report change statement

The only repository file written by this stage is this report at
`docs/reports/phase-1-trace-and-define.md`. No other repository file was modified.
Unrelated working-tree changes were preserved.
