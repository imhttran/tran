# UI reskin — stage 7

**Status:** Implemented; awaiting review. **Scope:** the four admin consoles at
`/admin/document-extraction`, `/admin/family-members`,
`/admin/narrative-review`, and `/admin/source-of-truth`. The stage-3 shell, top
navigation, light/dark tokens, shared state panels, and restrained jade are
reused unchanged.

Previous stage: PR #112 merged as `d50ff1d` and delivered stage 6
(`/head-of-family`). This stage does not replay that work.

## Direction

All four consoles now share one admin surface language: page header, notices,
a control/filter panel, dense record cards or labelled tables, and separated
primary, secondary, and destructive actions. Statuses use semantic chips with
visible words. Dense tables become labelled stacked cards at narrow widths.
Destructive member and invitation confirmations focus **Cancel** on open, cancel
on Escape, restore focus to the opener, and keep the confirm action explicit.

No Rust, API contract, route, permission, filter, pagination, extraction,
member-management, or review rule changed.

## Before

| Extraction                                                   | Family members / accounts                                              |
| ------------------------------------------------------------ | ---------------------------------------------------------------------- |
| ![Before: extraction](./before/desktop-light-extraction.png) | ![Before: family members](./before/desktop-light-family.png)           |
| **Narrative review**                                         | **Source of truth**                                                    |
| ![Before: narrative](./before/desktop-light-narrative.png)   | ![Before: source of truth](./before/desktop-light-source-of-truth.png) |

The baseline had an unstyled accounts list, a non-wrapping summary row that
overflowed by 180 px at 390 px, no focus handling in confirmations, and raw
badge labels.

## After

### Desktop

| Extraction (light)                                         | Family members (light)                                               |
| ---------------------------------------------------------- | -------------------------------------------------------------------- |
| ![After: extraction](./after/desktop-light-extraction.png) | ![After: family members](./after/desktop-light-family.png)           |
| **Narrative review (light)**                               | **Source of truth (light)**                                          |
| ![After: narrative](./after/desktop-light-narrative.png)   | ![After: source of truth](./after/desktop-light-source-of-truth.png) |

| Extraction (dark)                                              | Family members (dark)                                                    |
| -------------------------------------------------------------- | ------------------------------------------------------------------------ |
| ![After: extraction dark](./after/desktop-dark-extraction.png) | ![After: family members dark](./after/desktop-dark-family.png)           |
| **Narrative review (dark)**                                    | **Source of truth (dark)**                                               |
| ![After: narrative dark](./after/desktop-dark-narrative.png)   | ![After: source of truth dark](./after/desktop-dark-source-of-truth.png) |

### Confirmations

| Prepare confirmation (light)                                        | Prepare confirmation (dark)                                             |
| ------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| ![After: prepare confirm](./after/desktop-light-family-confirm.png) | ![After: prepare confirm dark](./after/desktop-dark-family-confirm.png) |

Cancel is first, receives focus, and Escape cancels without issuing a request.

### Mobile (390 px)

| Extraction                                                       | Family members                                                    |
| ---------------------------------------------------------------- | ----------------------------------------------------------------- |
| ![After: mobile extraction](./after/mobile-light-extraction.png) | ![After: mobile family](./after/mobile-light-family.png)          |
| **Narrative review**                                             | **Source of truth**                                               |
| ![After: mobile narrative](./after/mobile-light-narrative.png)   | ![After: mobile source](./after/mobile-light-source-of-truth.png) |

Filters, summaries, tables, queues, details, notes, and actions stack; there is
no horizontal page overflow.

## What changed

- `web/src/app/admin/family-members/page.tsx` and
  `web/src/components/AccountsAdmin.tsx`: stat grid, labelled responsive tables,
  member cards with status/role chips, action disclosure, and keyboard-operable
  prepare/send/member confirmations.
- `web/src/app/admin/document-extraction/page.tsx`: shared loading/empty/error
  panels and semantic proposal chips; all proposal, candidate, value, and apply
  behavior is unchanged.
- `web/src/app/admin/source-of-truth/page.tsx`: shared empty/error panels,
  semantic decision chips, full-width reason and reversal fields; all preview,
  acknowledgement, apply, and history behavior is unchanged.
- `web/src/components/NarrativeReviewConsole.tsx`: filter heading, queue heading
  and count, shared loading/empty/placeholder panels, status chips, and clearer
  link/passage action grouping; filters, pagination, search, and decisions are
  unchanged.
- `web/src/app/globals.css`: scoped `fm-*` rules plus extraction,
  source-of-truth, and narrative status/form refinements. No token values changed.
- `web/e2e/admin-workspaces-visual.spec.ts` (new, fully mocked).
- `web/scripts/capture-ui-reskin-stage7.mjs` (new, synthetic capture).

## Defects found and fixed

1. The generic `.status-panel dl` flex row out-specified `.fm-stat-grid`, which
   inflated the family-members page to 415 px of overflow at 390 px. The stat
   grid is now scoped with higher specificity.
2. A `.proposal-value input` rule stretched radio buttons to full width; it is
   now limited to text inputs and textareas.
3. Confirmation dialogs had no focus management; Cancel now takes focus, Escape
   cancels, focus returns to the opener, and Tab stays inside.
4. Proposal and decision statuses lacked semantic classes; proposed, draft,
   active, superseded, and reversed states now have visible chips.
5. The source-of-truth reason and reversal fields were narrow; they now span the
   panel.
6. Narrative filter fields aligned to the bottom of a tall help cell; they now
   align to the top.

## Verification

Run against an isolated production build on `127.0.0.1:3001`; the existing app on
`3000` and API on `8080` were left running. All screenshots use synthetic data.

| Check                                                                                                       | Result                   |
| ----------------------------------------------------------------------------------------------------------- | ------------------------ |
| `scripts/web-control.sh typecheck`                                                                          | **PASS**                 |
| `scripts/web-control.sh build`                                                                              | **PASS**                 |
| `npx playwright test e2e/admin-workspaces-visual.spec.ts`                                                   | **PASS — 13/13**         |
| `experience` + `workspace-visual` + `head-of-family-visual` + `family-profile-visual` + `my-profile-visual` | **PASS — 66/66**         |
| Real-API `document-extraction` + `source-of-truth` (and token-gated admin suites)                           | **9 passed, 14 skipped** |
| 390px overflow (all four screens)                                                                           | **PASS — 0 px**          |
| Confirmation focus / Escape / focus restore                                                                 | **PASS**                 |
| Light/dark `--bg` tokens unchanged                                                                          | **PASS**                 |
| `git diff --check`                                                                                          | **PASS**                 |

Baseline before the reskin: the mocked admin suite failed 3 of 13 (member
confirmation focus ×2, 180 px mobile overflow) and passed 10.

The 66 regression tests were run after the console changes; the final
`.proposal-value` scoping fix touches only extraction/source-of-truth selectors,
which those suites do not use.

### SOP disposition

The initial stage task `UI-701` failed twice with `IMPLEMENT_NO_PROGRESS`
(no repository mutations) and SOP's own classification said a repeated unchanged
stop must not be retried. The stage was therefore split into three narrower
governed tasks on the same branch:

- **UI-701A** (family members + accounts): stage `PASSED`, gate `PASS`,
  0/3 fix cycles, lifecycle `LOCAL_DONE`, outcome `PARTIAL`.
- **UI-701B** (document extraction + source of truth): stage `PASSED`, gate
  `PASS`, 0/3 fix cycles, lifecycle `LOCAL_DONE`, outcome `PARTIAL`.
- **UI-701C** (narrative review): stage `PASSED`, gate `PASS`, 0/3 fix cycles,
  lifecycle `LOCAL_DONE`, outcome `PARTIAL`.

All three ran LARGE / `ollama` / `deepseek-v4.1-flash:cloud`. Each executed its
`git diff --check` acceptance check; the criteria remain informational because
Node/build/browser execution is outside SOP's command policy. The operator ran
the full deterministic gate and recorded it here. No acceptance decision or
waiver was recorded on the operator's behalf.

### Not run

- The token-gated browser cases in `admin-accounts.spec.ts`,
  `bulk-onboarding.spec.ts`, and `narrative-review.spec.ts` require the
  `scripts/web-e2e.sh` harness with the synthetic OIDC issuer and seeded
  disposable databases. They were skipped and are **not** claimed as passing.
- The stage-one live-RAG timeout remains **outstanding** and was not rerun.

## Known limitations

- Theme follows the system preference only; no manual selector was added.
- Confirmation panels remain inline within the page (existing behavior) rather
  than modal overlays; they are keyboard operable and focus-restoring.
- The narrative filter help text stays long; it is readable but makes the filter
  panel tall on desktop.
- Real-API authorization, privacy, and mutation behavior remains operator/CI
  coverage; this stage changed presentation, state clarity, and keyboard access
  only.
