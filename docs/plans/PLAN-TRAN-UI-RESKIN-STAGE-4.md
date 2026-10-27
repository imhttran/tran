# Implementation Plan — Tran UI reskin, stage 4

**Status:** Implemented; awaiting review. Evidence is in
[`ui-reskin/stage-4`](../reports/ui-reskin/stage-4/README.md).

## Project

Tran family archive web UI (`web/`).

## Summary

Apply the stage-3 shell and Linear-inspired light/dark system to the owner
profile at `/profile`, including its revision, media, feedback, privacy, and
session states. This is the next unfinished reskin screen after stage 3. The
family tree, Ask page, and family profile already have their stage designs.

No API, permission, privacy, review, upload, or publication behavior changes.
Every existing interaction and test hook remains intact.

## Scope

- `web/src/components/MyProfile.tsx`
- `web/src/app/globals.css`
- A new mocked browser spec for visual/states coverage
- `web/e2e/my-profile.spec.ts` only if a retained selector needs adjustment
- Stage evidence and report under `docs/reports/ui-reskin/stage-4/`

Do not change `src/` (Rust), API contracts, routes, authentication, permissions,
review decisions, media limits, or privacy rules.

## Design

### Hero

- Keep the owner name, role/session explanation, completion progress, open
  submission count, and feedback count.
- Replace the text-glyph (`☺`) photo fallback with a real initials avatar using
  `initialLetter()` from `web/src/lib/person-display.ts`.
- Keep the approved photo preview when present.
- Use the stage-two family-profile avatar treatment: a 132px circle with
  `--accent-soft` border/background and `--accent` initials.
- Keep the hero as a neutral card with a subtle border and restrained shadow.

### Sections and field cards

- Use one clear section card per About, Story, and Memories group.
- Field cards use `--surface` rather than a second white card, so the hierarchy is
  section → field → value.
- Keep labels, helper text, approved value, visibility label, open revision,
  feedback, and every action.
- Keep status chips semantic; do not make all states green.
- Keep readable line lengths and Vietnamese text rendering.
- Keep action buttons at comfortable touch sizes and wrap them on narrow screens.

### Media and history

- Keep photo/document current, pending, rejected, private, and empty states.
- Keep upload constraints, visibility selection, withdraw, and make-private.
- Keep review history and reviewer notes.
- Use the same surface, border, spacing, and status language as the field cards.

### States

Replace the raw loading hint and plain error button with the shared
`state-panel` pattern used by the family profile:

- loading: `state-panel state-panel-loading`, `role="status"`, `aria-busy`;
- anonymous: `profile-signin` empty/sign-in panel;
- expired: `state-panel state-panel-expired` with sign-in action;
- denied: existing `DenialView`;
- unavailable/error: `state-panel state-panel-unavailable` with retry;
- unlinked: `state-panel state-panel-empty` with the existing safe explanation.

All existing test IDs must remain: `profile-loading`, `profile-signin`,
`profile-signin-button`, `profile-expired`, `profile-unlinked`, `profile-retry`,
`profile-header`, `profile-name`, `profile-pending-count`,
`profile-feedback-count`, `profile-photo-preview`, `profile-photo-current`,
`profile-document-current`, `profile-photo-empty`, `profile-document-empty`,
`profile-photo-input`, `profile-document-input`, `profile-history`, and the
field/open/feedback/action IDs.

## Tests

Add `web/e2e/my-profile-visual.spec.ts` with fully mocked session/profile
responses:

1. ready profile renders approved values, completion, open submission, feedback,
   media, and history without private leaks;
2. initials avatar renders when no photo exists; an approved photo uses the
   media URL;
3. loading, unavailable/retry, expired, sign-in, and unlinked states are
   distinct;
4. mobile 390px has no horizontal overflow and the editor is usable;
5. light and dark tokens apply.

Keep the existing real-API `my-profile.spec.ts` selectors working. It remains
operator/CI coverage because it requires the `scripts/web-e2e.sh` harness.

## Acceptance Criteria

- `/profile` reads as the same application as the stage-3 shell and the family
  profile: neutral surfaces, restrained jade, semantic status colors.
- All approved, draft, pending, rejected, needs-changes, private, restricted,
  media, history, and privacy states remain visible and actionable.
- Loading, unavailable, expired, sign-in, denied, and unlinked states are
  distinct and useful.
- The initials avatar replaces the text-glyph fallback.
- Desktop and 390px mobile layouts have no unintended horizontal overflow.
- Keyboard focus and 44px interactive targets remain usable.
- Existing routes, APIs, permissions, privacy rules, review behavior, upload
  limits, and test IDs are unchanged.
- Typecheck, build, focused mocked browser specs, and required repository gates
  pass.

## Verification

- `cd web && ./scripts/web-control.sh typecheck`
- `cd web && ./scripts/web-control.sh build`
- `npx playwright test e2e/my-profile-visual.spec.ts`
- Re-run `npx playwright test e2e/experience.spec.ts e2e/family-profile-visual.spec.ts`
- `git diff --check`
- Inspect desktop/mobile, light/dark, open editor, pending/feedback, and state
  screenshots with synthetic content.
