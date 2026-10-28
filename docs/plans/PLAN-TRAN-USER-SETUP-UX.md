# PLAN-TRAN-USER-SETUP-UX — A clearer member setup experience for admins and invitees

**Status:** PROPOSED. Design only; nothing implemented.
**Repository:** https://github.com/imhttran/tran
**Canonical requirements:** `docs/plans/PLAN-TRAN-USER-ONBOARDING.md` and `docs/plans/PLAN-TRAN-BULK-ONBOARDING.md`. This plan changes presentation and flow only; every security, privacy and review rule in those plans still applies.
**Builds on:** PR #127 (`docs/plans/PLAN-TRAN-BULK-ONBOARD-TABLE.md`), which added names, masked contact, a complete load, filters and a wider layout to the bulk preview table. Its approved decision lets admin-only onboarding views show a person's display name and a server-masked verified email; this plan extends that decision to the batch status table and the Accounts list.

## Problem

**Admin → Family Members** (`web/src/app/admin/family-members/page.tsx`) shows two separate lists:

1. **Bulk onboard**: one row per genealogy person. Since PR #127 the preview table shows the display name, a masked contact, status, proposed username and warnings, with status/contact/search filters. After **Prepare**, the batch status table still labels each row by `person_key` only.
2. **Accounts** (`web/src/components/AccountsAdmin.tsx`): one row per OIDC identity, labelled by `subject @ issuer`, with an Actions drawer where the admin types a raw person key to link.

What goes wrong for the admin:

- **Some views still show IDs instead of names.** The preview table is fixed, but the batch status table and `FamilyMember.linked_person` carry no display name, and no view shows a branch.
- **One person appears in two places.** A person who has been prepared, invited, signed in and is waiting for link review is split across both lists. Nothing answers "where is this person in setup?"
- **Linking means typing a person key by hand.** A typo can link the wrong person. The confirmation helps, but it repeats the same key back.
- **The next step is hard to find.** Preview, select, prepare, verify email and send are separate controls in separate sections, and the admin has to work out the order.
- **The proposed username cannot be edited.** The onboarding plan requires an admin confirmation for ambiguous Vietnamese name parsing and username collisions, but the UI only displays the proposal.

What goes wrong for the invitee:

- No invitation-landing or "awaiting review" page exists in `web/src/app`. Someone who signs in before their person link is reviewed probably sees a generic `DenialView`. **This needs a runtime check** with a synthetic unlinked user before step 5 is designed.

## Goals / explicit boundaries

- An admin can answer "who is set up, who is stuck, and what do I do next?" from one list, using names.
- An invitee always knows what state they are in after they sign in.
- **Out of scope:** self-signup, auto-linking by name/email/birthday match, a messaging centre, bulk password resets, and any new role or grant. A reviewed account-person link is still the only thing that grants person-level access.
- **Public repository.** Mockups, screenshots, fixtures and tests use synthetic people only (e.g. `Nguyen Van An`).

## Stages

### SETUP-001 — Show names in the remaining views

Done in PR #127: the preview table shows the display name with `person_key` as secondary text, and a masked contact column.

Remaining:

- Add `display_name` to the batch status items and to `FamilyMember.linked_person`. Read it server-side from the genealogy database under the existing admin check; the browser never supplies it. Reuse the display-name read PR #127 added to `src/onboarding/candidate.rs` rather than a second query.
- In the batch status table, render the Person cell the same way the preview table does (name in bold, key below), and use the name in the "Verified email for …" label.
- In the Accounts list, show the linked person's name instead of the key, and move `subject @ issuer` into the expanded details.
- Optional: a branch label in all three views, if one is cheap to read alongside the name. Drop it if it needs a new query.
- If the genealogy store is unavailable (`linkage: "unavailable"`), fall back to the key and keep the existing notice.

**Pass:** no admin row is labelled by `person_key` alone when the genealogy store is up; unit/E2E tests use synthetic names.

### SETUP-002 — Pick the person by searching for a name

- Replace the free-text "Person key" box in the link form with a search-as-you-type picker that returns name, branch, birth year (if approved for admin view) and key.
- Reuse the existing bounded search validation (`validate_person_search` in `src/api.rs`, already shared by the narrative and nomination person lists). Add an admin-only route and avoid a second search implementation.
- Exclude people who already have an active link, and show why when they are excluded.
- The confirmation dialog names the person ("Link *jane@example.test* to **Nguyen Van An** (Branch 2)?").

**Pass:** the link form cannot be submitted with an unknown key; already-linked people cannot be chosen; non-admin requests are denied.

### SETUP-003 — Let the admin edit the username before preparing

- In the preview table, make the proposed username editable for preparable rows, prefilled with the policy proposal.
- Send per-person username overrides with `bulkPrepare`. The server re-applies the existing username policy, normalization and case-insensitive uniqueness check; a rejected override returns the existing `username_policy` / `username_collision` reason for that row.
- Flag ambiguous name parses visually so the admin knows which rows to look at.

**Pass:** an override that breaks policy or collides is rejected per row without failing the batch; an accepted override is the reserved username.

### SETUP-004 — One list with a setup stage for each person

Mock this up in `docs/mockups/` before coding, starting from PR #127's preview table: its name and contact columns, status-card toggles, "No contact" toggle and accent-insensitive search are the base of the merged list, not something to replace.

- Merge the two lists into one roster keyed by genealogy person, plus a short "Unlinked accounts" section for identities that signed in without a matching invitation.
- Each row shows a stage chip:
  `Not invited → Prepared → Invited → Signed in, awaiting review → Active`, plus `Needs review`, `Not eligible`, and `Deactivated`.
- Each row has one primary button for its next step: **Prepare**, **Add verified email**, **Send invitation**, **Resend**, **Review link**. Role, password reset, unlink and deactivate stay in the secondary Actions drawer.
- Extend the existing status-card toggles to cover the new stages (e.g. "Awaiting review") and make them the default view, replacing the Accounts list's separate status/role filters. Bulk actions apply to the selected rows.
- The server computes the stage; the UI only renders it.

**Pass:** for the synthetic 6-person fixture used by BULK-005, each person appears exactly once with the correct stage, and the primary button matches the stage.

### SETUP-005 — Invitee pages

Depends on the runtime check above.

- **Welcome page** reached from the activation link after sign-in: "You were invited as **Nguyen Van An**. Is this you?" with *Yes, that's me* and *No, this isn't me*. Confirming records the claim for admin review; it grants nothing by itself.
- **Awaiting review page** for a signed-in, unlinked account: explains that an admin is reviewing the link and what the member can do in the meantime (edit their own private account profile, per the onboarding plan).
- Replace the generic denial for this state only; real permission denials stay unchanged.
- No existence leaks: an expired or wrong token shows a generic message.

**Pass:** an unlinked synthetic user sees the awaiting-review page instead of the denial; an expired token reveals nothing about the invited person.

## Suggested delivery

- **PR 1:** the rest of SETUP-001, plus SETUP-002 and SETUP-003. Small, independent of the redesign, and removes the riskiest manual step (typing keys).
- **PR 2:** SETUP-004 after the mockup is agreed.
- **PR 3:** SETUP-005 after the runtime check of today's invitee experience.

Each PR runs the existing Rust, migration and Playwright gates and uses synthetic data only.
