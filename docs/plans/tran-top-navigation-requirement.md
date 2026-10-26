# Tran: top navigation requirement

Status: Ready for implementation  
Date: 2026-10-26  
Repository: https://github.com/imhttran/tran

## Goal

Replace the left sidebar with a slim top header. Keep the family brand on the left. Align navigation and account actions to the right. Give the family tree and page content the width previously occupied by the sidebar.

Use [Linear](https://linear.app/) as the primary design reference and [Vercel](https://vercel.com/) as a secondary reference. Use their compact navigation approach. Apply the light and dark color plans below. Keep Tran's typography and Vietnamese text support. Retain green as a restrained highlight.

## Desktop layout

- Show the family brand at the left of the header.
- Show Family tree, Ask the archive, and Documents as direct links on the right.
- Place secondary destinations in a More dropdown.
- Keep account status and sign-in/sign-out actions at the far right.
- Mark the current destination clearly.
- Use a subtle bottom border and restrained spacing.
- Remove the sidebar column and its reserved width.
- Preserve readable content widths. Let the tree and review screens use their existing wider layouts.

## Navigation mapping

Preserve all existing routes and visibility rules. Confirm the current route inventory before implementation.

| Destination | Existing route | Desktop location |
| --- | --- | --- |
| Family tree | `/genealogy` | Direct link |
| Ask the archive | `/ask` | Direct link |
| Documents | `/documents` | Direct link |
| My profile | `/profile` | More |
| Head of Family | `/head-of-family` | More |
| Branch review | `/branch-review` | More |
| Add documents | `/` | More; preserve upload eligibility |
| Personal history review | `/admin/narrative-review` | More; admin only |
| Reviews | `/admin/reviews` | More; admin only |
| Extractions | `/admin/document-extraction` | More; admin only |
| Source of truth | `/admin/source-of-truth` | More; admin only |
| Bulk onboard | `/admin/family-members` | More; admin only |

Keep backend authorization unchanged. Menu visibility must not replace server-side permission checks.

## Mobile and narrow screens

- Replace the desktop links and More control with one top-right Menu button.
- Put all permitted destinations in the mobile menu.
- Keep the family brand and account actions accessible.
- Select the breakpoint before header content overlaps or wraps awkwardly.
- Keep the menu inside the viewport. Allow vertical scrolling for long admin menus.
- Close the menu after navigation.
- Prevent horizontal page overflow.

## Interaction and accessibility

- Use semantic header and navigation elements with clear accessible names.
- Retain the skip-to-content link and main-content target.
- Support keyboard activation and normal Tab navigation.
- Expose the menu's expanded state and controlled element.
- Close the dropdown on Escape, outside click, or focus leaving it.
- Return focus to the menu button when Escape closes the dropdown.
- Keep visible focus indicators and touch targets of at least 44 pixels.
- Preserve anonymous, signed-in, expired-session, and sign-out states.
- Keep current-page indicators accessible.

## Scope

Update the shared shell and its navigation styles. Adjust affected browser tests.

Start with:
- `web/src/components/ExperienceShell.tsx`
- `web/src/components/AccountMenu.tsx`
- `web/src/app/globals.css`
- Existing navigation and account browser tests under `web/e2e/`

Do not change genealogy data, photo approval workflows, API contracts, authentication, or authorization.

## Acceptance criteria

1. No page using the shared shell shows the old left sidebar or an empty sidebar gutter.
2. Desktop navigation appears in the top-right header area.
3. The three primary destinations are directly accessible on desktop.
4. More contains every remaining permitted destination.
5. Mobile Menu exposes all permitted destinations without overflow.
6. Existing routes, role restrictions, and account states still work.
7. Keyboard navigation, Escape dismissal, outside-click dismissal, and focus indicators work.
8. The tree, Ask page, and an admin review page remain usable at desktop and mobile widths.
9. Light and dark themes remain readable.

## Implementation and handoff

1. Fetch the latest repository state. Inspect local changes before updating the checkout.
2. Implement the change in the shared shell. Preserve unrelated work.
3. Run the frontend typecheck and focused navigation/account checks.
4. Inspect desktop and mobile layouts, including an admin menu.
5. Reuse passing verification when relevant inputs are unchanged.
6. Open a PR with the change summary, screenshots, checks run, and any blocked checks.

Continue through implementation and PR creation. Stop only for a real blocker, such as missing access, a conflicting change that cannot be safely resolved, or a required check that cannot run. Report the blocker precisely. Do not claim that blocked checks passed.



## Plan 2: Linear-inspired light theme

### Goal

Use white and cool neutral surfaces, dark text, subtle separators, and minimal shadows. Retain jade green for selected navigation, links, focus rings, and selected tree nodes.

These are proposed Tran implementation values, not verified copies of Linear's CSS tokens. Match the reference's neutral visual direction while preserving readable application controls.

### Light palette

| Existing or new token | Value | Purpose |
| --- | --- | --- |
| `--bg` | `#FAFAFB` | Page background |
| `--card` | `#FFFFFF` | Header, cards, menus, dialogs |
| `--surface` | `#F3F4F6` | Hover and secondary surfaces |
| `--surface-strong` | `#E9EAED` | Pressed and disabled surfaces |
| `--fg` | `#18191B` | Main text |
| `--muted` | `#60646C` | Secondary text |
| `--line` | `#E2E4E8` | Decorative separators |
| `--line-strong` | `#80858E` | Required control boundaries |
| `--accent` | `#0F6B4A` | Green links and selected states |
| `--accent-strong` | `#0B5639` | Green hover state |
| `--accent-soft` | `#E8F3ED` | Selected background |
| `--accent-soft-strong` | `#D5EADD` | Pressed selected background |
| `--accent-contrast` | `#FFFFFF` | Text on solid green |
| `--focus-ring` | `#0F6B4A` | Keyboard focus |
| `--action-bg` | `#202124` | Neutral primary button |
| `--action-hover` | `#34363B` | Neutral primary button hover |
| `--action-fg` | `#FFFFFF` | Primary button text |

### Implementation

1. Set the light theme tokens in `web/src/app/globals.css`.
2. Apply white or neutral surfaces to the header, menus, forms, cards, and dialogs.
3. Use neutral primary buttons. Reserve solid green for a deliberate emphasis where needed.
4. Use soft green backgrounds for selected navigation and selected tree nodes.
5. Remove large green-tinted surfaces and green decorative shadows.
6. Keep borders subtle. Use `--line-strong` where a visible boundary identifies a control.

### Acceptance

- The page reads as white and neutral gray with small green highlights.
- Body text, secondary text, links, and controls remain readable.
- Hover, active, selected, disabled, and focus states are distinct.
- Photos retain their original colors. Do not apply theme filters to media.

## Plan 3: Linear-inspired dark theme

### Goal

Use near-black backgrounds, charcoal surfaces, soft white text, and subtle neutral borders. Use green sparingly for the same purposes as the light theme. Do not tint the whole interface green.

The values below are proposed companion colors for Tran. They are not a claim that Linear publishes this exact palette.

### Dark palette

| Existing or new token | Value | Purpose |
| --- | --- | --- |
| `--bg` | `#08090A` | Page background |
| `--card` | `#111214` | Header, cards, menus, dialogs |
| `--surface` | `#191B1F` | Hover and secondary surfaces |
| `--surface-strong` | `#24272D` | Pressed and disabled surfaces |
| `--fg` | `#F1F2F4` | Main text |
| `--muted` | `#A2A7B0` | Secondary text |
| `--line` | `#2A2D33` | Decorative separators |
| `--line-strong` | `#707782` | Required control boundaries |
| `--accent` | `#4CCF95` | Green links and selected states |
| `--accent-strong` | `#6FE0AC` | Green hover state |
| `--accent-soft` | `#122A20` | Selected background |
| `--accent-soft-strong` | `#1A3A2B` | Pressed selected background |
| `--accent-contrast` | `#082018` | Text on solid green |
| `--focus-ring` | `#6FE0AC` | Keyboard focus |
| `--action-bg` | `#E6E8EB` | Neutral primary button |
| `--action-hover` | `#FFFFFF` | Neutral primary button hover |
| `--action-fg` | `#111214` | Primary button text |

### Implementation

1. Apply the dark palette through the existing theme mechanism.
2. Set native controls to the matching `color-scheme`.
3. Use surface differences and borders to separate layers. Avoid heavy shadows and glow.
4. Keep tree labels, connectors, citations, form fields, and modal backdrops legible.
5. Use the same green highlight rules as the light theme.
6. Keep warning, error, success, and evidence-state colors semantic. Do not replace them all with green.

### Acceptance

- The page reads as near-black and charcoal with small green highlights.
- Cards, dropdowns, dialogs, and inputs remain distinguishable from the page.
- Green text is readable on normal and selected surfaces.
- No light-theme backgrounds or unreadable SVG labels remain in dark mode.

## Shared theme integration and verification

These color plans supersede the original instruction to preserve the existing palette. Keep the top-navigation requirements unchanged.

1. Use shared semantic tokens. Replace hard-coded component colors only where they conflict with these themes.
2. Keep system-preference switching. Preserve an existing manual selector if present; a new selector is not required for this change.
3. Apply theme changes consistently to the top header, account actions, family tree, profiles, Ask page, documents, and review screens.
4. Keep status labels and icons. Color alone must not convey approval, review, or error state.
5. Check text contrast at 4.5:1 for normal text and 3:1 for large text. Check required control boundaries and focus indicators at 3:1 against adjacent colors. Decorative dividers may be subtler.
6. Inspect the tree, Ask page, and an admin review screen at desktop and mobile widths in both themes. Include an open dropdown and a dialog.
7. Run the frontend typecheck and affected navigation/theme checks. Reuse unchanged passing evidence.
8. Open a PR with light and dark screenshots, a short change summary, and verification results. Record any blocked check explicitly.

Complete both themes as one coherent change. Do not add a separate styling system or change data, permissions, routes, or approval behavior. Stop only for a real blocker. If an exact reference color cannot be verified, use the proposed palette and identify it as an adaptation.
