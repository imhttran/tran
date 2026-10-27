# Implementation Plan — Tran UI Linear-Inspired Color Refresh

**Task:** UI-901  
**Project:** [imhttran/tran](https://github.com/imhttran/tran)  
**Target:** `web/`  
**Status:** Proposed  
**Reference:** [Linear design refresh](https://linear.app/now/behind-the-latest-design-refresh)

## Goal

Refine the Tran Family Archive color scheme using Linear's restrained graphite surfaces, subtle borders, soft white text, and muted blue accents. Preserve Tran's identity and current page layouts.

This is a **color refinement**, not a new UI reskin. The previous eight-stage reskin is complete, including shared navigation, light/dark theme tokens, and state panels.

## Design Direction

- Use dark graphite backgrounds with layered neutral surfaces.
- Keep the light theme soft and neutral rather than stark white everywhere.
- Use muted indigo-blue for interactive emphasis and selected states.
- Keep primary actions neutral; reserve semantic green, amber, and red for success, warning, and error.
- Reduce visual noise from borders and shadows.
- Retain the existing typography, spacing, page layouts, and shared components unless a small adjustment is required for legibility.
- Use Linear as a visual reference. Do not copy its brand assets or claim exact color parity.

### Proposed Theme Palette

These are **Tran proposals**, not verified copies of Linear's production tokens.

| Role | Dark theme | Light theme |
| --- | --- | --- |
| Background (`--bg`) | `#121214` | `#F6F6F7` |
| Card/panel (`--card`) | `#1B1B1E` | `#FFFFFF` |
| Secondary surface (`--surface`) | `#29292D` | `#F0F0F2` |
| Primary text (`--fg`) | `#F4F5F8` | `#222326` |
| Accent (`--accent`) | `#A0A9F8` | `#4F5DB8` |

Derive the remaining tokens from these values: `--surface-strong`, `--muted`, `--line`, `--line-strong`, `--accent-strong`, `--accent-soft`, `--accent-soft-strong`, `--accent-contrast`, `--action-bg`, `--action-hover`, `--action-fg`, `--focus-ring`, and shadows. Check contrast for actual foreground/background combinations; do not assume the proposed hex values pass every use case.

Do not repurpose semantic status tokens (`--ok`, `--warn`, `--err`) as decorative accents.

## Scope

### In scope

- Shared theme token changes in `web/src/app/globals.css`.
- Targeted shared-component styling for navigation, panels, buttons, tabs, inputs, links, selected states, and focus states.
- Cleanup of page-specific hardcoded colors that conflict with shared tokens.
- Visual review of the Family Tree, person profiles, Ask the Archive, Documents, Branch Review, Head of Family, administrative workspaces, and Sign In.
- Updated focused visual evidence in light, dark, and mobile layouts.

### Out of scope

- Rust/API or database changes.
- Authentication, authorization, routing, redirects, cookies, or sessions.
- Rebuilding navigation or page layouts.
- New design libraries or styling frameworks.
- Manual Light / Dark / System theme selector (optional separate follow-up).
- Changes to family-history data, documents, photos, or review workflows.

## Implementation Phases

### Phase 1 — Theme tokens

1. Create a feature branch from the latest `origin/main`.
2. Review the current theme declarations and existing stage-3 styling in `web/src/app/globals.css`.
3. Update the shared light/dark tokens using the proposed palette.
4. Derive supporting surface, border, hover, focus, and action tokens with adequate contrast.
5. Preserve `prefers-color-scheme` behavior and existing semantic status colors.

**Done when:** Shared colors render correctly in both themes without changing application behavior.

### Phase 2 — Shared components

1. Review `ExperienceShell`, top navigation, and other shared UI components.
2. Apply the revised surfaces, borders, accents, links, and focus styles through tokens.
3. Remove only conflicting hardcoded component colors.
4. Keep neutral primary actions and preserve accessible focus visibility.

**Done when:** Shared components have consistent contrast and selected/hover/focus behavior across themes.

### Phase 3 — Workspace review

1. Check Family Tree and person details.
2. Check Ask the Archive and Documents.
3. Check Branch Review, Head of Family, and administration pages.
4. Check Sign In, denied/no-access states, loading, empty, and error panels.
5. Correct theme-specific color regressions without changing layout or function.

**Done when:** No reviewed page uses inconsistent decorative colors or loses semantic distinctions.

### Phase 4 — Verification and PR

1. Reuse passing checks when their relevant inputs have not changed.
2. Run the affected TypeScript, build, and focused Playwright checks after implementation.
3. Capture desktop light/dark and 390px mobile screenshots for representative shared and role-specific pages.
4. Run `git diff --check`.
5. Record the screenshots, check results, known limitations, and any unverified criteria in the PR.
6. Commit the changes and open a pull request. Do not merge it automatically.

**Done when:** The focused gates pass and the PR contains enough evidence for visual review.

## Acceptance Criteria

1. Both light and dark themes use the shared proposed palette consistently.
2. Normal text meets WCAG AA contrast of at least 4.5:1, except where WCAG explicitly allows a lower threshold. Essential UI boundaries and focus indicators meet the applicable non-text contrast requirement of 3:1.
3. Family Tree relationships, document/photo content, administrative approval states, and semantic status messages remain visually distinguishable.
4. Navigation, authentication, permissions, API calls, routes, and existing test IDs are unchanged.
5. A 390px viewport has no horizontal overflow; Vietnamese text wraps and remains legible.
6. Theme changes do not introduce console errors, broken interactive states, or obvious visual regressions.
7. Focused checks and screenshots are recorded; the implementation is submitted as a PR.

## Stop Conditions

- Stop and report if a requested visual change requires backend, authorization, or workflow changes.
- Stop and report if an accessibility contrast issue cannot be resolved within the existing shared component system.
- Do not weaken assertions, remove tests, or bypass required verification to obtain a passing result.
- Do not expand this task into a second full UI reskin.

## Delivery

Open one PR titled **`style(web): refine Tran colors with Linear-inspired palette`**. Include before/after screenshots, a summary of token changes, focused verification results, and remaining limitations.
