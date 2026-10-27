# Implementation Plan — Tran UI reskin, stage 5

**Status:** Ready for implementation.

## Project

Tran family archive web UI (`web/`).

## Summary

Apply the stage-3 shell and Linear-inspired light/dark system to the document
workspace and the two review consoles:

- `/documents`
- `/admin/reviews`
- `/branch-review`

The work is visual hierarchy and responsive presentation only. Every behavior,
API call, route, permission check, review rule, publication rule, and test ID is
preserved.

## Scope

- `web/src/app/documents/page.tsx`
- `web/src/components/ReviewConsole.tsx`
- `web/src/components/BranchReviewConsole.tsx`
- `web/src/app/globals.css`
- `web/e2e/workspace-visual.spec.ts` (new, fully mocked)
- Stage evidence and report under `docs/reports/ui-reskin/stage-5/`

Do not change Rust, API contracts, routes, authentication, authorization,
permissions, document lifecycle, review decisions, required notes, self-approval
restrictions, branch scope, privacy rules, or pagination behavior.

## Design

### Shared

- Reuse the stage-3 top header, neutral surfaces, jade accent, semantic status
  colors, focus ring, and 44px action targets.
- Put page content in a clear sequence: page header, notices, primary workspace,
  supporting queue/history.
- Use `status-chip` tokens for review and processing status instead of generic
  gray badges. Keep every status word visible; color is never the only signal.
- Put destructive actions apart from primary actions. Keep confirmation steps.
- Preserve readable line lengths, Vietnamese diacritics, and focus visibility.

### `/documents`

- Keep the existing upload form fields and behavior, but present them as one
  clear panel with a primary submit action.
- Keep the first-use guidance and its test IDs.
- Present processing jobs and submissions as responsive rows rather than wide
  tables on narrow screens. Preserve every existing `document-row-*` test ID.
- Show document title, version/category/filename, review status, processing
  status, visibility, and actions with clear grouping.
- Keep the rejection-reason field near the review actions.
- Keep Preview, Approve, Reject, scope, Process/Promote, Retry, and Remove.
- Keep `documents-error` and `documents-message` as explicit notices.
- No horizontal overflow at 390px.

### `/admin/reviews`

- Keep the queue/filter toolbar, status filter, pagination, message/error panels.
- Give the queue rows a clear selected state and readable metadata.
- Keep the detail panel hierarchy: submission header, current vs proposed
  comparison, media previews, reviewer note, decision actions, confirmation, and
  history.
- Keep required-note validation, self-submission/owner-linked decision blocks,
  and independent approval.
- Mobile stacks queue and detail without overflow.

### `/branch-review`

- Keep branch and status filters, queue pagination, error/message panels.
- Keep the no-authority and denied states explicit.
- Keep current vs proposed fact, cited sources, other person evidence, reviewer
  note, decisions, confirmation, and history.
- Keep branch scope, note requirements, decision blocks, and privacy behavior.
- Mobile stacks queue and detail without overflow.

## Deterministic acceptance checks

These checks are defined before execution and must be run against the isolated
production build:

1. `./scripts/web-control.sh typecheck` (from the repository root)
2. `./scripts/web-control.sh build` (from the repository root)
3. `cd web && npx playwright test e2e/workspace-visual.spec.ts`
4. `cd web && npx playwright test e2e/experience.spec.ts e2e/family-profile-visual.spec.ts e2e/my-profile-visual.spec.ts`
5. `git diff --check`

## Acceptance Criteria

1. `/documents`, `/admin/reviews`, and `/branch-review` use stage-3 surfaces,
   restrained jade, semantic status chips, and the shared header.
2. Document upload, processing, review, visibility, preview, retry, and remove
   behavior is preserved.
3. Review queues, filters, pagination, details, source previews, required notes,
   decisions, self-approval restrictions, and branch scope are preserved.
4. Desktop and 390px layouts have no unintended horizontal overflow.
5. Status, error, and review state labels remain explicit and readable in both
   themes.
6. Existing route, API, permission, and privacy behavior is unchanged.
7. The deterministic checks above pass.

## Acceptance Mapping

- criterion 1: checks 2 and 3
- criterion 2: checks 1 and 3
- criterion 3: checks 1 and 3
- criterion 4: check 3
- criterion 5: check 3
- criterion 6: checks 1 and 4
- criterion 7: checks 1, 2, 3, 4, and 5
