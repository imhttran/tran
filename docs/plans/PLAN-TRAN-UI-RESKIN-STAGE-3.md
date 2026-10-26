# Implementation Plan — Tran UI reskin, stage 3

## Project

Tran family archive web UI (`web/`).

## Summary

Replace the shared left sidebar with a slim top header and apply the requested
Linear-inspired light and dark color plans to the shared application shell. This
stage continues the reskin after merged stages 1 (`#101`) and 2 (`#102`). It is
navigation and shared-theme work only; it does not redesign individual content
screens.

The source requirement is
[`tran-top-navigation-requirement.md`](./tran-top-navigation-requirement.md).
Where that document describes "Plan 2" and "Plan 3", this stage implements both
the light and dark palettes as one coherent change.

## What must not change

- Existing routes, page content, API calls, authentication, authorization,
  permissions, retrieval rules, and document approval behavior.
- Existing session states: anonymous, signed-in, expired, denied, and sign-out.
- Existing role gating: admin-only destinations remain admin-only; upload
  destinations remain upload-eligible only.
- Existing typography and Vietnamese diacritic support.
- Existing page-specific width overrides for the tree, profiles, and review
  consoles.
- Existing status semantics: warning, error, success, and evidence colors stay
  semantic and keep their text labels. Media keeps its original colors.

## Design specification

### Desktop header

- A semantic `<header>` spans the page with a subtle bottom border and a
  restrained surface.
- The family brand sits at the left and links to `/genealogy`.
- The primary navigation sits on the right and exposes three direct links:
  Family tree (`/genealogy`), Ask the archive (`/ask`), Documents (`/documents`).
- A **More** menu contains every remaining permitted destination:
  My profile, Head of Family, Branch review, Add documents (only when
  `canUpload(role)`), and the admin destinations
  (Personal history review, Reviews, Extractions, Source of truth, Bulk onboard).
- Account status and sign-in/sign-out actions remain at the far right.
- The current destination is marked with `aria-current="page"` on its link.
  When the current destination is inside **More**, the More control also shows a
  visible active state and an accessible "current page" indication.
- Remove the sidebar column and its reserved width. The header must not leave an
  empty gutter.

### Mobile and narrow screens

- At the selected breakpoint, hide the desktop primary links and More control.
- Show one top-right **Menu** button and keep the brand and account actions
  accessible.
- The mobile menu contains every permitted destination, in the same order as
  the desktop navigation (primary first, then secondary).
- The menu stays inside the viewport, scrolls vertically when long, and closes
  after navigation.
- No horizontal page overflow at 390 px.

### Interaction and accessibility

- Use semantic `<header>` and `<nav>` elements with accessible names.
- Keep the skip-to-content link and the `#main-content` target.
- The More and Menu controls expose `aria-expanded` and `aria-controls`.
- Close More on Escape, outside click, or focus leaving it; return focus to the
  More button when Escape closes it.
- Close the mobile menu on Escape and return focus to the Menu button.
- Support normal Tab navigation and keyboard activation.
- Keep visible focus indicators; interactive header targets are at least 44 px.
- Do not rely on color alone for current page or account state.

### Light and dark palettes

Use the values in the source requirement as shared semantic tokens. The light
theme becomes white/cool-neutral with restrained jade; the dark theme becomes
near-black/charcoal with restrained green. Add neutral action tokens so primary
buttons are neutral by default; reserve solid green for deliberate emphasis such
as links, selected navigation, focus, and selected tree nodes.

Both themes keep system-preference switching. Existing manual selectors, if any,
remain unchanged.

## UI-301 — Top header, desktop More menu, and mobile menu

### Deliverables

- Updated `web/src/components/ExperienceShell.tsx`.
- Updated `web/src/components/AccountMenu.tsx` only if needed for header layout.
- Updated shared shell and navigation styles in `web/src/app/globals.css`.

### Acceptance Criteria

- No shared-shell page renders the old sidebar or an empty sidebar gutter.
- Desktop navigation is in the top-right header with the three direct primary
  links and a More menu for all remaining permitted destinations.
- Mobile shows one Menu button and a scrollable in-viewport menu with all
  permitted destinations.
- Existing routes, role restrictions, account states, and sign-out behavior
  remain intact.
- More and Menu close on Escape; More also closes on outside click and focus
  leave; focus returns to the button on Escape.
- Keyboard navigation and visible focus work; interactive targets are at least
  44 px.
- No horizontal overflow at 390 px.

## UI-302 — Linear-inspired light and dark shared theme

### Deliverables

- Updated semantic color and action tokens in `web/src/app/globals.css`.
- Neutral primary button styling using the action tokens.
- Neutral header, menus, cards, inputs, and focus styling in both themes.

### Acceptance Criteria

- Light mode reads as white and cool neutral gray with small jade highlights.
- Dark mode reads as near-black and charcoal with small green highlights.
- Header, dropdown, mobile menu, inputs, cards, and dialogs remain distinguishable
  from their backgrounds in both themes.
- Body text, secondary text, links, and controls remain readable; warning, error,
  success, and evidence states remain semantic.
- Focus indicators and required control boundaries remain visible.
- Photos and other media are not tinted or filtered.

## UI-303 — Focused browser coverage and evidence

### Deliverables

- Updated navigation coverage in `web/e2e/experience.spec.ts`.
- Updated shared-shell selectors in affected review specs.
- New `web/scripts/capture-ui-reskin-stage3.mjs` for synthetic screenshots.
- `docs/reports/ui-reskin/stage-3/README.md` with before/after evidence and known
  limitations.

### Acceptance Criteria

- The navigation suite covers desktop primary links, More contents and current
  state, mobile Menu contents, Escape/outside-click/focus-return behavior, and
  light/dark token application.
- Existing affected review suites still pass.
- Screenshots use synthetic sessions and data only.

## Verification

- `cd web && ./scripts/web-control.sh typecheck`
- `cd web && ./scripts/web-control.sh build`
- `npx playwright test e2e/experience.spec.ts e2e/branch-review-ui.spec.ts e2e/branch-review-security.spec.ts e2e/my-profile.spec.ts`
- `git diff --check`
- Inspect desktop and mobile layouts in light and dark themes, including an open
  More menu and the mobile menu.
