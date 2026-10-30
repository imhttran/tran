# Tran: Invite one family member from their profile

Status: Proposed implementation plan
Date: 2026-10-29
Repository: https://github.com/imhttran/tran

## Goal

Let a system administrator or authorized family head (family admin) invite one living person directly from that person's profile. A complete family email or phone list is not required.

Keep the first release small. Reuse the current identity provider, invitation services, person links, and authorization controls.

## User experience

1. Open a living person's profile.
2. Select **Invite to family archive**.
3. Add or confirm that person's invitation contact.
4. Review the person, contact, proposed username, and Member access.
5. Select **Send invitation**.
6. Show the actual invitation status and next action.

Use a small form or panel associated with the profile. Do not require the administrator to find the person again in a bulk table.

If the person already has a linked account, show **Account linked** instead of an invitation action. If an invitation exists, show its status and an authorized resend or revoke action. Prevent duplicate invitations and accounts.

## Profile action and eligibility

- Show the invitation entry point only to a system administrator or an authorized family admin for the target person.
- Offer the flow for living-person profiles. Reuse the existing eligibility rules on the server.
- If living status or identity needs review, explain the blocker and provide a review path. Do not allow the invitation to bypass the review.
- Do not infer living status from a missing death date.
- Do not require a birth date when the existing policy permits other verified living evidence.
- Translate internal warning codes into plain-language explanations.

## Family admin permission

This is a deliberate authorization extension. The current code reviewed for this discussion makes invitation administration system-admin-only. A genealogical Head of Family title currently grants no application permission.

For this release, an authorized family head may act as a **family admin** for individual member invitations within their appointed family or branch scope.

- Resolve the actor from the authenticated account and its reviewed person link.
- Resolve family-head authority from the current approved appointment. A name, profile label, nomination, or client-supplied role is not sufficient.
- Use the appointment's explicit scope. Do not silently widen branch authority to the whole archive.
- Enforce the same rule in the API for preparation, sending, resend, revoke, and status access.
- Recheck authority before each change. A revoked appointment or person link must remove this permission.
- Keep system administrators authorized across the application.
- Do not grant family admins the system `admin` role, general role management, bulk sending, or unrelated review privileges.
- Do not treat an existing branch reviewer grant as family-admin authority.
- Keep account-person link approval under the existing reviewed-link rules. Invitation authority alone must not approve or replace a link.

Implementation decision: inspect the existing appointment model and reuse its authoritative scope. If it cannot identify an approved head, linked account, and target scope reliably, report that exact blocker before enabling family-admin invitations. Do not substitute a title-string check.

## Contact and invitation handling

- Collect contact details for this person only.
- Support the existing verified delivery channel. Do not promise SMS delivery merely because a phone number can be stored.
- If email is the only supported channel, request an email address and explain why it is needed.
- Allow progress to be saved as **Needs contact** when no usable destination is available.
- Store draft contact details with restricted access. Mask them in summaries and exclude them from public profile fields and logs.
- Saving or previewing must not send a message or mark an invitation as sent.
- Show the destination and Member access before the explicit send action.
- Use the existing lowercase username policy without Vietnamese accents. Preserve accents in display names.
- Support safe retries. A delivery error must not create a second invitation or claim success.

## Access for the invited member

The simple flow assigns **Member (`viewer`)** access plus the required archive grants through the approved activation path. It does not offer elevated roles.

Authentication, archive access, and the reviewed account-person link remain separate checks. Accepting an invitation must not automatically bypass person-link review or grant family-admin access.

## New-user entry

Use a minimal invitation welcome screen:

1. Validate the invitation.
2. Continue through the existing identity provider for account setup or sign-in.
3. Confirm the intended person, with a **This is not me** option.
4. Show pending review if activation or linking is incomplete.
5. Continue to the family archive when access is ready.

Handle expired, revoked, and previously accepted invitations with clear next steps. Support existing accounts without creating duplicates. Resume safely after interruption. User confirmation does not replace administrator approval of the person link.

## Implementation phases

### Phase 1: Trace and define

Inspect repository instructions and current main. Trace the profile UI, invitation lifecycle, delivery provider, person-link review, and approved family-head appointments. Record the smallest changes needed and the exact authority rule.

### Phase 2: Implement the profile flow

Add the profile action, individual invitation form, restricted contact draft, plain-language blockers, and invitation status. Reuse existing services. Add the scoped family-admin authorization check on the server.

### Phase 3: Complete activation

Connect the invitation to the minimal welcome screen and existing account setup. Preserve reviewed linking and safe retries across the separate identity and genealogy databases. Display partial setup accurately.

### Phase 4: Verify and open a PR

Run focused checks for the changed paths and the repository's required gates. Reuse passing checks when inputs are unchanged. Open a PR with screenshots, the permission rule, and verification evidence. Continue until the implementation is complete or a concrete blocker prevents progress.

## Acceptance criteria

- A system admin can start an individual invitation from an eligible living person's profile.
- An approved family head can perform the same action only within their authorized scope.
- Ordinary members, unrelated family heads, and branch reviewers without family-admin authority cannot perform the action, including through direct API requests.
- Unknown or deceased status cannot bypass the existing eligibility policy.
- Missing contact can be saved and completed later without sending an invitation.
- A completed flow sends only after explicit review and reports the actual outcome.
- Linked accounts and existing invitations are handled without duplication.
- Revoked family-head authority or person links cannot authorize subsequent invitation changes.
- New members receive only the intended Member access and required archive grants.
- The welcome flow handles incorrect-person, expired-invitation, and interrupted-activation cases.
- Legitimate accounts and family records remain unchanged by test-data cleanup.

## Out of scope

- Bulk onboarding changes or importing a complete contact list.
- Public self-registration.
- New SMS delivery or authentication providers.
- General RBAC redesign or family-admin role-management screens.
- Automatic account merging or person linking by name.
- Deleting synthetic records or placeholder accounts based only on their labels.

## Delivery boundary

This file is a plan. Implementation must preserve unrelated local changes and follow repository instructions. Use an isolated branch and open a PR. Do not send real invitations, change live permissions, or delete live data as part of automated verification. Use isolated test identities and a test delivery sink.
