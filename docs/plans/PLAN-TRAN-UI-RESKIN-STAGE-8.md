# Implementation Plan — Tran UI reskin, stage 8 (final)

**Status:** Ready for implementation.

## Project

Tran family archive web UI (`web/`).

## Summary

Finalize the reskin:

- `/signin` — shared top navigation, auth panel, configured/unconfigured/error
  states, mobile layout, and keyboard focus.
- Shared denied/no-access presentation — one consistent state panel for every
  page that uses `DenialView`.

Every authentication route, redirect, cookie, session, error code, permission,
and existing test ID is preserved. No backend feature is added.

## Scope

- `web/src/app/signin/page.tsx`
- `web/src/app/signin/SignInShell.tsx` (new, thin client shell wrapper)
- `web/src/components/DenialView.tsx`
- `web/src/app/globals.css`
- `web/e2e/signin-visual.spec.ts` (new, fully mocked)
- `web/scripts/capture-ui-reskin-stage8.mjs` (new)
- `docs/reports/ui-reskin/stage-8/` and the reskin index report

Do not change Rust, API contracts, auth routes, redirects, cookies, sessions,
permissions, or unrelated screens. Do not expand into SOP policy or test-harness
repairs.

## Design

### `/signin`

- Wrap the page in the existing `ExperienceShell` so the top navigation and
  account menu are shared with the rest of the app.
- Configured state: a two-part auth panel with an eyebrow, clear heading, short
  explanation of administrator provisioning, one primary `Sign in` action that
  keeps `href="/api/auth/login"` and `data-testid="signin-button"`, and a small
  "what you need" side list.
- Unconfigured state: the same panel language, an explicit operator-action
  message, and no fabricated sign-in action.
- Error state: read the `error` query parameter and show a safe, mapped message
  for `unconfigured`, `invalid_callback`, `state_mismatch`, `missing_id_token`,
  `invalid_id_token`, `token_exchange_failed`, `provider_unavailable`,
  `not_provisioned`, `session_denied`, and `account_unavailable`; unknown codes
  get a generic message. Never echo the raw parameter.
- Keep `data-testid="signin-view"` and `data-testid="signin-button"` (configured
  only).
- Keyboard: the sign-in action is the first page action, keeps visible focus,
  and has a 44px target.
- 390px: panel and side list stack; no horizontal overflow.

### Denied and no-access states

- `DenialView` keeps `data-testid="denied-view"` / `"no-access-view"` and
  `role="alert"`, but uses the shared `state-panel state-panel-unavailable`
  surface instead of the unstyled `card denial`.
- No page-specific redesign; every existing denial becomes consistent.

### Themes and text

- Reuse `--surface`, `--card`, `--line`, `--accent`, `--focus-ring`, and semantic
  status tokens. Keep restrained jade; the primary sign-in action stays neutral.
- Vietnamese text and long error/provider names wrap without clipping.

## Deterministic acceptance checks

These checks are defined before execution. SOP's command policy permits only
read-only `git` and `go`, so Node, build, and browser execution are operator-run
against the isolated production build; SOP executes the whitespace check.

1. scripts/web-control.sh typecheck (repository root)
2. scripts/web-control.sh build (repository root)
3. from `web/`: npx playwright test e2e/signin-visual.spec.ts
4. from `web/`: npx playwright test e2e/experience.spec.ts e2e/admin-workspaces-visual.spec.ts e2e/workspace-visual.spec.ts e2e/head-of-family-visual.spec.ts e2e/family-profile-visual.spec.ts e2e/my-profile-visual.spec.ts
5. git diff --check (repository root)

## Acceptance Criteria

1. `/signin` uses the shared top navigation, surfaces, jade, and state panels.
2. Configured, unconfigured, and error states are explicit; auth routes,
   redirects, cookies, and session behavior are unchanged.
3. Denied and no-access states are visually consistent across the app.
4. Keyboard focus, Vietnamese text, and 390px layouts work without horizontal
   overflow.
5. Existing test IDs and behaviors are preserved.
6. The deterministic checks above pass.

## Acceptance Mapping

- criterion 1: checks 2 and 3
- criterion 2: checks 1 and 3
- criterion 3: check 3
- criterion 4: check 3
- criterion 5: checks 1 and 4
- criterion 6: checks 1, 2, 3, 4, and 5
