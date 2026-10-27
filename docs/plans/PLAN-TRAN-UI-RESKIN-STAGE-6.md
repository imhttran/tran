# Implementation Plan — Tran UI reskin, stage 6

**Status:** Ready for implementation.

## Project

Tran family archive web UI (`web/`).

## Summary

Apply the stage-3 shell and Linear-inspired light/dark system to the Head of
Family page:

- `/head-of-family`

The work is visual hierarchy, state presentation, responsive layout, and keyboard
accessibility only. Every designation rule, permission check, API contract,
mutation workflow, confirmation step, and test ID is preserved. A genealogy
designation remains distinct from a system administrator role, and the page
never infers a successor.

## Scope

- `web/src/app/head-of-family/page.tsx`
- `web/src/components/HeadOfFamilyView.tsx`
- `web/src/components/HeadOfFamilyActions.tsx`
- `web/src/app/globals.css`
- `web/e2e/head-of-family-visual.spec.ts` (new, fully mocked)
- Stage evidence and report under `docs/reports/ui-reskin/stage-6/`

Do not change Rust, API contracts, routes, authentication, authorization,
permissions, designation rules, branch scope, nomination/approval/revocation
behavior, or the evidence model.

## Design

### Hero and identity

- Keep the shared `PageHeader` and top navigation.
- Add a designation hero: eyebrow, "Head of Family", the cited head name when one
  exists, a short scope note that the designation is not a system administrator
  role, and a semantic evidence/status chip.
- Show the appointment count. Do not derive or suggest a successor.

### Person details and evidence

- Show record and appointment identifiers as quiet metadata.
- Keep the evidence status badge and every citation button; the metadata-only
  source dialog stays the only drill-down.
- Group cited evidence in its own surface inside the conclusion card.

### Actions

- Keep the per-conclusion nominate form and the existing approve/revoke controls.
- Keep the separate nomination/approval panel (`HeadOfFamilyActions`).
- Group the forms clearly; place revocation and rejection apart from approval.
- Keep every confirmation step. Confirm dialogs become keyboard accessible:
  focus moves to the confirm action, Escape cancels, and buttons keep 44px
  targets.
- Preserve role gates: viewers see no controls, editors nominate, administrators
  approve and revoke.

### States

- Loading: shared loading panel with `role="status"` and `aria-busy`.
- Empty (no active appointment): explicit empty panel; no person is named and no
  successor is implied.
- Denied and no access: explicit, non-leaking panels.
- Expired: explicit sign-in panel with a sign-in action.
- Catalog unavailable: explicit unavailable panel with no inferred head.

### Responsive and themes

- Desktop: readable measure, stacked hero, evidence, and actions.
- 390px: hero, metadata, evidence, forms, and actions stack; buttons wrap or fill
  the width; no horizontal page overflow.
- Reuse `--surface`, `--card`, `--line`, `--accent`, `--focus-ring`, and semantic
  status tokens in both themes. Vietnamese diacritics wrap without clipping.

## Deterministic acceptance checks

These checks are defined before execution. SOP's command policy permits only
read-only `git` and `go`, so Node, build, and browser execution are operator-run
against the isolated production build; SOP executes the whitespace check.

1. scripts/web-control.sh typecheck (repository root)
2. scripts/web-control.sh build (repository root)
3. from `web/`: npx playwright test e2e/head-of-family-visual.spec.ts
4. from `web/`: npx playwright test e2e/experience.spec.ts e2e/workspace-visual.spec.ts e2e/family-profile-visual.spec.ts e2e/my-profile-visual.spec.ts
5. git diff --check (repository root)

## Acceptance Criteria

1. `/head-of-family` uses the stage-3 header, neutral surfaces, restrained jade,
   semantic status chips, and shared state panels.
2. The head, record/appointment metadata, evidence status, and citations are
   presented with a clear hierarchy.
3. Nomination, approval, and revocation workflows keep their confirmation steps
   and role gates.
4. Loading, empty, denied, no-access, expired, and catalog-unavailable states are
   distinct and consistent.
5. Desktop and 390px layouts have no unintended horizontal overflow, and the
   confirm dialog is keyboard operable.
6. The designation stays distinct from system administrator; no successor is
   inferred; backend contracts are unchanged.
7. The deterministic checks above pass.

## Acceptance Mapping

- criterion 1: checks 2 and 3
- criterion 2: checks 1 and 3
- criterion 3: checks 1 and 3
- criterion 4: check 3
- criterion 5: check 3
- criterion 6: checks 1 and 4
- criterion 7: checks 1, 2, 3, 4, and 5
