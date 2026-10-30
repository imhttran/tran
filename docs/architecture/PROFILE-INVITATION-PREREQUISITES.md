# Profile Invitation Prerequisites — PIR-001

Evidence-based prerequisite trace and authority decision for the person-profile invitation plan. Every traced area names a repository file and a function or route. Items not inspected in this pass carry the word `unverified`.

## Repository baseline

The trace baseline is the working tree, not a recorded commit.

- Branch state: `git status` reports `main...origin/main [behind 2]`.
- Untracked, user-owned files present before this task: `docs/plans/PLAN-TRAN-PROFILE-INVITATION-RECOVERY.md`, `docs/plans/tran-person-profile-invitation-plan.md`, `docs/reports/phase-1-trace-and-define.md`.
- The superseded Phase 1 report `docs/reports/phase-1-trace-and-define.md` states the commit id was unavailable, so no commit id is claimed here.

Files inspected for this trace:

- `README.md`
- `docs/plans/tran-person-profile-invitation-plan.md`
- `docs/reports/phase-1-trace-and-define.md`
- `src/onboarding/invitation.rs`
- `src/onboarding/delivery.rs`
- `src/onboarding/link.rs`
- `src/onboarding/candidate.rs`
- `src/onboarding/username.rs`
- `src/genealogy/appointments.rs`
- `src/genealogy/eligibility.rs`
- `src/api.rs` (invitation, appointment, family-profile, and link handlers)
- `web/src/components/FamilyProfileView.tsx`
- `web/src/lib/api.ts` (profile and invitation types)

Not inspected in this pass: `src/identity.rs`, `.env.example`, `src/oidc.rs`, `web/app/**` route files, and the genealogy migration `genealogy/migrations/0006_head_of_family.sql`. Their statements below are marked `unverified` where the code cannot confirm them.

## Profile surface

- Route: `GET /api/family/persons/{person_key}/profile` in `src/api.rs`, registered in `AppState::router`.
- Handler: `family_profile` in `src/api.rs`. It calls `authorize` with `Operation::FamilyProfileRead`, then `onboarding::read_family_profile` and `onboarding::family_profile::read_family_archive_photographs`.
- Response that supplies the person: the `family_profile` JSON body carries `person.person_key` and `person.display_name`, plus `state`, `fields`, `dates`, `history`, `media`, and `archive_photographs`.
- Client route and component: route `/family/[personKey]` (documented in `README.md`); component `FamilyProfileView` in `web/src/components/FamilyProfileView.tsx`.
- Client type: `FamilyProfileResponse` and `getFamilyProfile` in `web/src/lib/api.ts` (profile types only).
- Mount point for an authorized invitation panel: inside `FamilyProfileView` in `web/src/components/FamilyProfileView.tsx`, in the rendered profile region next to the person header/details block, gated by the caller's admin status. The panel does not exist today; `web/src/lib/api.ts` has no per-profile invitation type.

## Invitation lifecycle

Service file: `src/onboarding/invitation.rs`. Route file: `src/api.rs`.

- Creation: `onboarding::create` in `src/onboarding/invitation.rs`; route `POST /api/admin/family-members/invitations` → `create_invitation` in `src/api.rs`. The handler resolves the person with `person_id_for_key`, clamps the TTL, and calls `create`.
- Pending / reused invitation: `create` is idempotent. When a pending, unexpired invitation exists it returns the same row with `token: None`; the same transaction runs the expiry sweep.
- Send: batch-scoped. `onboarding::delivery::send_batch` in `src/onboarding/delivery.rs`; route `POST /api/admin/family-members/bulk/send` → `bulk_send` in `src/api.rs`. A single-person send from a profile does not exist. Single-person send is a PIR-002..007 change, not an existing capability.
- Resend: route `POST /api/admin/family-members/invitations/{id}/resend` → `resend_invitation` in `src/api.rs`. It rejects an accepted row, calls `onboarding::revoke`, then calls `onboarding::create` for a replacement. `onboarding::delivery::issue_deliverable_invitation` in `src/onboarding/delivery.rs` performs the same revoke-then-reissue for a batch item.
- Revoke: `onboarding::revoke` in `src/onboarding/invitation.rs` (pending only); route `POST /api/admin/family-members/invitations/{id}/revoke` → `revoke_invitation` in `src/api.rs`.
- Accept: `onboarding::accept` in `src/onboarding/invitation.rs`. It is an atomic pending-and-unexpired update and classifies `InvalidToken`, `AlreadyAccepted`, `Expired`. Tokens are 32 random bytes, stored only as SHA-256 hex via `hash_token`. No accept route appears in the `src/api.rs` router; the accept entry point is `unverified` as a route.
- Expire: `onboarding::expire_due` in `src/onboarding/invitation.rs`, also run inside `create`.
- Delivery state: `onboarding::delivery::DeliveryStatus` in `src/onboarding/delivery.rs` (`not_sent`, `queued`, `sent`, `accepted`, `failed`, `expired`); `sync_invitation_status` copies invitation state onto the batch item; route `GET /api/admin/family-members/bulk/batches/{id}` → `bulk_batch_status` in `src/api.rs` reads it.
- Authorization today: every invitation route calls `authorize` with `Operation::OnboardingInvite`. The role mapping for that operation lives in `src/identity.rs` and is `unverified` here; the Phase 1 report records it as `Role::Admin` only.

## Delivery channels

- Adapter trait: `MailAdapter` (`name`, `is_configured`, `deliver`) in `src/onboarding/delivery.rs`.
- Fail-closed default: `UnconfiguredAdapter` returns `NotConfigured` and creates no invitation.
- Test adapter: `FakeAdapter` is in-memory and for tests only.
- A configured live network mail provider adapter was not inspected and is `unverified`.
- Configuration key names: `unverified`. `.env.example` was not read in this pass, so no key names are asserted.
- SMS: MISSING. `src/onboarding/delivery.rs` has one mail-only adapter trait, no SMS adapter, no phone field, and no SMS configuration key. This report does not promise SMS.
- Contact capture: `delivery::verify_recipient` validates an email shape and upserts `onboarding_recipient_contact`; `contact_for` reads only verified rows; `candidate::mask_email` masks the local part. Email is the only captured contact channel.

## Eligibility and review blockers

Source: `src/onboarding/candidate.rs` (`Classification`, `Reason`, `classify`) and `src/genealogy/eligibility.rs` (`EligibilityInput`, `decide`, `ReasonCode`).

- Classifications: `EligibleForInvite`, `NeedsReview`, `AlreadyLinked`, `Invited`, `NotEligible`.
- Living rule: `is_living_value` never treats a missing death date as living; only explicit living evidence qualifies. `src/genealogy/eligibility.rs` likewise decides living status from recorded input only.
- Warning codes (non-blocking; still return `EligibleForInvite`): `missing_dob`, `username_collision`.
- Blocker / review codes: `contradictory_vital`, `disputed`, `missing_citation`, `ambiguous`, `unresolved_duplicate`, `ambiguous_segmentation`, `minor_guardian_review`, `no_living_evidence`, `blocked_missing_recipient` (blocks sending only), and `missing_verified_contact` / `missing_verified_recipient` (send-only block).
- Precedence is implemented in `classify`; `reclassify_person` recomputes it and `preview_candidates` builds the preview.
- Plain-language requirement: `candidate.rs` exposes stable snake_case reason strings only. There is no message catalogue or translation function in the inspected files, so every code above must be mapped to plain-language UI text in a later stage. That mapping is MISSING today.

## Person-link review

Source: `src/onboarding/link.rs`; routes in `src/api.rs`.

- Reviewed link path: `link::link_reviewed` in `src/onboarding/link.rs`. It records a review `LinkDecision` (`Approve`/`Reject`) against a `(person_id, account)` pair with the reviewer actor.
- Lookups: `link::link_for_person` (person → account) and `link::link_for_account` (account → person). `candidate.rs` uses the `onboarding_person_account_link` table to detect `AlreadyLinked`.
- Revoke: `link::revoke_link`; route `POST /api/admin/family-members/links/{id}/revoke` → `revoke_link` in `src/api.rs`.
- Create/review route: `POST /api/admin/family-members/links` → `create_link` in `src/api.rs`. It resolves the account with `onboarding::resolve_account`, resolves the person with `person_id_for_key`, then calls `link::link_reviewed`.
- Why invitation authority must not approve or replace a link: the reviewed link is a separate review decision with its own reviewer and audit record, made under `Operation::OnboardingLink`. Inviting a person is authorization to contact that person, not authorization to assert which account is that person. Merging the two would let an invitation flow create or overwrite an identity binding without the link review that the link table records.

## Username policy

Source: `src/onboarding/username.rs`.

- Normalization function: `key`. It lowercases the candidate username and strips Vietnamese diacritics (for example `đ` maps to `d`).
- Display-name function: `normalize_display_name`. It preserves accents in the display name.
- Supporting functions: `candidate_username` (builds a base from `NameParts`), `validate_override` (admin override check), `unique_username` (uniqueness allocation).
- Result: username keys are lowercase and accent-free; display names keep Vietnamese accents intact.

## Activation path

- Identity-provider entry: tokens are verified by `validate_token` from `src/oidc.rs`, called through `authenticate` and `authorize` in `src/api.rs`; JWKS is fetched by `fetch_jwks`. The concrete provider, issuer, and audience come from configuration and are `unverified` here because config files were not read.
- Account setup: an invited person receives a token. `delivery::activation_link` in `src/onboarding/delivery.rs` appends `token=<token>` to a caller-supplied base URL, so the base route is set by the caller and is `unverified`. On first verified sign-in the account is resolved with `onboarding::resolve_account`; an unregistered identity returns `identity_not_registered` (see `account_map_error` in `src/api.rs`). Provisioning runs through `provision_user` → `IdentityStore::client().provision_user` in `src/api.rs`.
- Accept step: `onboarding::accept` in `src/onboarding/invitation.rs` consumes the token.
- Welcome screen: MISSING. No welcome route was observed in the inspected files, and the `web/app` directory listing was not read (`unverified`). PIR-006 covers adding the welcome screen when the route is confirmed absent.

## Family-head authority

Source: `src/genealogy/appointments.rs`; routes in `src/api.rs`.

- Appointment fields: `appointment_id`, `person_key`, `branch_key`, `approved_by`, `status` (`active` / `suspended` / `revoked`), `evidence_assertion_ids`, `current_eligibility`, and the display name on read rows.
- Active-status rule: only an appointment with `status = active` is a current head. A partial unique index for one active head per branch is documented in the migration `genealogy/migrations/0006_head_of_family.sql` (stated in the Phase 1 report; the migration itself is `unverified` here).
- Branch scope: the appointment carries `branch_key`. `list_active_appointments` returns active rows; `citations_for_assertions` resolves their evidence.
- Nomination and approval: `appointments::nominate`, `appointments::approve`, `appointments::reject`, `appointments::end_appointment`. Routes: `POST /api/genealogy/nominations` → `nominate_head_of_family`; `POST /api/genealogy/nominations/{id}` → `decide_nomination` (uses `Operation::HeadOfFamilyApproval`, calls `appointments::approve` with the configured third branch key); `POST /api/genealogy/appointments/{id}` → `end_head_of_family`. The read route is `GET /api/genealogy/head-of-family` → `genealogy_head_of_family`.
- Linked-account requirement: the reviewed account-person link is the only source that names which account is a person. `link::link_for_person` and `link::link_for_account` in `src/onboarding/link.rs` return that binding.
- Exact server rule: an account may act as a family head for branch `B` only when
  1. `appointments::list_active_appointments` returns an appointment whose `status = active`, and
  2. that appointment's `branch_key = B`, and
  3. `link::link_for_account(account)` returns a reviewed link whose `person_key` equals that appointment's `person_key`.

  The rule never reads a name, label, nomination text, request-body branch, or client-supplied role. Nomination and label text are display data; the authority decision uses `branch_key` and the reviewed link only.

- Note: `genealogy_head_of_family` also refuses to present an appointment as verified when `current_eligibility != ReasonCode::Eligible`. That is a display-side eligibility check in `src/api.rs`, not a replacement for the rule above.

## Appointment scope decision

Decision: HOLD for family-admin invitations. The appointment model today reliably identifies the approved head and the target scope, but not the linked account on its own.

- Approved head: identified. `list_active_appointments` gives `person_key` with `status = active`.
- Target scope: identified. The appointment carries `branch_key`.
- Linked account: NOT stored on the appointment. The appointment row has no account id column; only the reviewed link table (`onboarding_person_account_link` via `link::link_for_person` / `link::link_for_account`) maps a person to an account.

Exact blocker: no single server function combines an active appointment, its `branch_key`, and the reviewed account-person link to authorize a caller as that branch's head. Until such a function exists, family-admin invitation authority cannot be granted without inventing authority.

Smallest corrective change (additive, read-only): add one service function in `src/genealogy/appointments.rs` or `src/onboarding/link.rs`, for example `head_authority_for_account(client, account) -> Option<HeadScope>`, that returns the account's active appointment `branch_key` and `person_key` only when `list_active_appointments` and `link_for_account` agree. Add no new table and no new column. Wire it to `authorize` after that function exists. Family-admin invitations stay disabled until this function is reviewed and the decision is changed to PASS.

## Smallest change list

- PIR-002 — Single-person send from the profile. Files: `src/onboarding/delivery.rs` (one-person send that reuses `issue_deliverable_invitation`), `src/api.rs` (a profile-scoped send route next to `create_invitation`), `web/src/lib/api.ts` (a profile invitation type), `web/src/components/FamilyProfileView.tsx` (the send action in the invitation panel). Behavior: prepare, send, and report delivery state for one person without a batch.
- PIR-003 — Plain-language mapping. Files: `web/src/lib/api.ts` and the invitation panel in `web/src/components/FamilyProfileView.tsx`. Behavior: one label per `Reason` and `ReasonCode` from `src/onboarding/candidate.rs` and `src/genealogy/eligibility.rs`; no code shown raw.
- PIR-004 — Head-authority service. Files: `src/genealogy/appointments.rs` or `src/onboarding/link.rs` (new `head_authority_for_account`), `src/api.rs` (call it inside `authorize` for the family-admin invitation operation). Behavior: the exact server rule in the Family-head authority section; the decision moves to PASS.
- PIR-005 — Authorized invitation panel. Files: `web/src/components/FamilyProfileView.tsx` (panel mount), `web/src/lib/api.ts` (types). Behavior: the panel renders only when the server reports the caller as that branch's head; the client sends no role.
- PIR-006 — Welcome and accept screen. Files: a new route under the web app (route confirmed missing is `unverified`), `src/api.rs` (an accept route if none exists). Behavior: token accept via `onboarding::accept`, `identity_not_registered` handled with a clear setup step.
- PIR-007 — Activation base and channel policy. Files: `src/onboarding/delivery.rs` (`activation_link` base), configuration files (key names recorded after inspection). Behavior: email only; no SMS. SMS stays out of scope.
