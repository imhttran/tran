# Implementation Plan — Tran UI reskin, stage 7

**Status:** Ready for implementation.

## Project

Tran family archive web UI (`web/`).

## Summary

Apply the stage-3 shell and Linear-inspired light/dark system to the four admin
consoles:

- `/admin/document-extraction`
- `/admin/family-members`
- `/admin/narrative-review`
- `/admin/source-of-truth`

The work is visual hierarchy, dense-record presentation, responsive layout,
state clarity, and keyboard access. Every API call, permission, privacy rule,
review requirement, filter, pagination behavior, extraction behavior,
member-management workflow, and test ID is preserved. No backend feature is
added.

## Scope

- `web/src/app/admin/document-extraction/page.tsx`
- `web/src/app/admin/family-members/page.tsx`
- `web/src/components/AccountsAdmin.tsx`
- `web/src/app/admin/narrative-review/page.tsx`
- `web/src/components/NarrativeReviewConsole.tsx`
- `web/src/app/admin/source-of-truth/page.tsx`
- `web/src/app/globals.css`
- `web/e2e/admin-workspaces-visual.spec.ts` (new, fully mocked)
- `web/scripts/capture-ui-reskin-stage7.mjs` (new, synthetic capture)
- Stage evidence and report under `docs/reports/ui-reskin/stage-7/`

Do not change Rust, API contracts, routes, authentication, authorization,
permissions, privacy rules, review decisions, extraction behavior, member
management, filters, pagination, or confirmation requirements.

## Design

### Shared

- Reuse the stage-3 top navigation, neutral surfaces, jade accent, semantic
  status colors, focus ring, and 44px action targets.
- Put page content in a clear sequence: page header, notices, primary workspace,
  supporting history/detail.
- Use `status-chip` tokens for review, extraction, delivery, member, and decision
  states. Keep every status word visible; color is never the only signal.
- Put destructive actions apart from primary actions. Keep every confirmation.
- Dense tables become labelled stacked cards at narrow widths; no horizontal page
  overflow at 390px.
- Preserve readable line lengths, Vietnamese diacritics, and visible focus.

### Destructive confirmations

For the member actions (`AccountsAdmin`) and the bulk prepare/send confirmations:

- move focus to **Cancel** when the confirmation opens;
- Escape cancels without issuing a request;
- focus returns to the control that opened the confirmation;
- Tab stays inside the confirmation while it is open;
- the confirm action remains available and explicit.

### `/admin/document-extraction`

- Present the intro, document selector, source metadata, and extract action as
  one clear control panel.
- Show proposals as cards with kind, subject, resolution, status chip, support
  text, candidate choice, value edit, and approve/reject actions.
- Keep apply-extraction, its disabled rules, and all messages.
- Loading, no-eligible-document, error, and denied states are explicit.

### `/admin/family-members`

- Present the preview summary as a readable stat grid; show review-needed,
  warnings, and the dry-run note.
- Show preview and batch tables as responsive rows/cards with status chips.
- Keep preview, select-eligible, prepare, verify-recipient, send, and their
  disabled rules.
- Style the accounts section: filters, member cards, role/status chips, linked
  person, actions disclosure, and confirmations.
- Keep empty, loading, linkage-unavailable, and error states explicit.

### `/admin/narrative-review`

- Keep the existing filter grid, active-filter summary, pagination, queue, and
  detail workflow.
- Add a queue heading and explicit placeholder/empty/loading panels.
- Use semantic chips for passage and link statuses while keeping the link filter
  help.
- Group link and passage decisions clearly; keep destructive rejection apart.
- Mobile stacks filters, queue, and detail without overflow.

### `/admin/source-of-truth`

- Present the intro, document/scope controls, preview action, effect panel,
  reasoning, confirmation checkboxes, apply, and history with a clear hierarchy.
- Use status chips for decision status and attention.
- Keep conflicts, omissions, preview hash, reason, acknowledge, and apply gates.
- Mobile stacks scope controls, preview items, and history without overflow.

## Deterministic acceptance checks

These checks are defined before execution. SOP's command policy permits only
read-only `git` and `go`, so Node, build, and browser execution are operator-run
against the isolated production build; SOP executes the whitespace check.

1. scripts/web-control.sh typecheck (repository root)
2. scripts/web-control.sh build (repository root)
3. from `web/`: npx playwright test e2e/admin-workspaces-visual.spec.ts
4. from `web/`: npx playwright test e2e/experience.spec.ts e2e/workspace-visual.spec.ts e2e/head-of-family-visual.spec.ts e2e/family-profile-visual.spec.ts e2e/my-profile-visual.spec.ts
5. git diff --check (repository root)

## Acceptance Criteria

1. All four screens use the stage-3 header, neutral surfaces, restrained jade,
   semantic status chips, and shared state panels.
2. Headings, filters, dense records, detail panels, status labels, and action
   placement have a clear hierarchy on desktop.
3. Existing extraction, member-management, narrative-review, and
   source-of-truth workflows, filters, pagination, permissions, and confirmations
   are preserved.
4. Destructive confirmations focus Cancel, cancel on Escape, restore focus, and
   keep the confirm action explicit.
5. Loading, empty, denied, expired, and error states are consistent and explicit.
6. Desktop and 390px layouts have no unintended horizontal overflow.
7. The deterministic checks above pass.

## Acceptance Mapping

- criterion 1: checks 2 and 3
- criterion 2: check 3
- criterion 3: checks 1 and 3
- criterion 4: check 3
- criterion 5: check 3
- criterion 6: check 3
- criterion 7: checks 1, 2, 3, 4, and 5
