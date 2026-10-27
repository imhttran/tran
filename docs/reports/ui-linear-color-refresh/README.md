# UI-901 — Linear-inspired color refresh

**Status:** Implemented on `feat/ui-901-linear-color-refresh`. **Scope:** shared
theme tokens in `web/src/app/globals.css`. Layout, structure, copy, routes, auth,
permissions, API calls and test IDs are unchanged.

Binding contract: `docs/plans/PLAN-TRAN-UI-LINEAR-COLOR-REFRESH.md` (not edited).
Token values and their derivation: [`token-contract.md`](./token-contract.md).

## Direction

A restrained graphite palette with soft white text, subtle borders and a muted
indigo accent. The light theme is soft neutral, not stark white everywhere. The
dark theme uses layered graphite surfaces. Primary actions stay neutral.
Semantic green, amber and red stay reserved for success, warning and error.

The theme still follows the system preference only. `color-scheme: light dark`
and the `@media (prefers-color-scheme: dark)` structure are unchanged.

## Token changes

| Token | Light before → after | Dark before → after |
| --- | --- | --- |
| `--bg` | `#fafafb` → `#f6f6f7` | `#08090a` → `#121214` |
| `--card` | `#ffffff` → `#ffffff` | `#111214` → `#1b1b1e` |
| `--surface` | `#f3f4f6` → `#f0f0f2` | `#191b1f` → `#29292d` |
| `--surface-strong` | `#e9eaed` → `#e6e6e9` | `#24272d` → `#35353a` |
| `--fg` | `#18191b` → `#222326` | `#f1f2f4` → `#f4f5f8` |
| `--muted` | `#60646c` → `#5c5f66` | `#a2a7b0` → `#a6a8b0` |
| `--line` | `#e2e4e8` → `#e2e2e6` | `#2a2d33` → `#2e2e33` |
| `--line-strong` | `#80858e` → `#75787f` | `#707782` → `#76787f` |
| `--accent` | `#0f6b4a` → `#4f5db8` | `#4ccf95` → `#a0a9f8` |
| `--accent-strong` | `#0b5639` → `#3d4a96` | `#6fe0ac` → `#bcc3fb` |
| `--accent-soft` | `#e8f3ed` → `#ecedf8` | `#122a20` → `#23263a` |
| `--accent-soft-strong` | `#d5eadd` → `#dcdff3` | `#1a3a2b` → `#2e3250` |
| `--accent-contrast` | `#ffffff` → `#ffffff` | `#082018` → `#121214` |
| `--action-bg` | `#202124` → `#222326` | `#e6e8eb` → `#f4f5f8` |
| `--action-hover` | `#34363b` → `#36373c` | `#ffffff` → `#ffffff` |
| `--action-fg` | `#ffffff` → `#ffffff` | `#111214` → `#121214` |
| `--warn-line` | `#ecd9a8` → `#e6cf9b` | `#4a3c1c` → `#4a3c1c` |
| `--info-soft` | `#eef1f0` → `#eceef2` | `#1b211f` → `#1e1f24` |
| `--focus-ring` | `#0f6b4a` → `#4f5db8` | `#6fe0ac` → `#a0a9f8` |
| `--backdrop` | new `rgba(18,18,20,0.6)` | new `rgba(0,0,0,0.7)` |
| shadows | `rgba(16,24,40,…)` → `rgba(18,18,20,…)` | unchanged `rgba(0,0,0,…)` |
| `--ok` / `--warn` / `--err` | unchanged | unchanged |
| `--ok-soft` / `--warn-soft` / `--err-soft` | unchanged | unchanged |

## What changed

- `web/src/app/globals.css`: the light `:root` and dark
  `@media (prefers-color-scheme: dark)` token values now use the plan palette
  and its derived tokens. `color-scheme: light dark` and the media query
  structure are unchanged.
- `web/src/app/globals.css`: page-local literals moved onto the neutral
  graphite family. `.source-dialog::backdrop` and
  `.personal-history-dialog::backdrop` moved from `rgba(8, 20, 16, 0.55)`
  (jade-tinted) to the new `--backdrop` token (`rgba(18, 18, 20, 0.6)` in light,
  `rgba(0, 0, 0, 0.7)` in dark). The `.family-node rect` drop-shadow moved from
  `rgba(0, 0, 0, 0.12)` to `rgba(18, 18, 20, 0.14)`.
- `web/src/app/globals.css`: `.button-danger` text moved from the literal `#fff`
  to `var(--accent-contrast)`.
- `web/e2e/admin-workspaces-visual.spec.ts`, `experience.spec.ts`,
  `head-of-family-visual.spec.ts`, `my-profile-visual.spec.ts`,
  `signin-visual.spec.ts`, and `workspace-visual.spec.ts`: updated the shared
  light/dark token assertions from the retired stage-3 `#fafafb`/`#08090a`
  values to `#f6f6f7`/`#121214`. No assertion was removed or weakened.
- `docs/reports/ui-linear-color-refresh/token-contract.md` and this report.

The dark-mode danger overrides (`color: var(--bg)` for
`.review-actions .button-danger`, `.hof-form-danger button`,
`.hof-pending-actions .button-danger`, `.fm-actions-body .fm-deactivate`) are
kept. The dark red status token stays light and `--bg` is now `#121214`, so the
text still reads on the light red fill.

## Semantic separation

`--ok`, `--warn` and `--err` keep their green, amber and red hue families in
both themes. No selector repurposes them as decoration. The indigo accent
replaces the jade accent only in accent roles: links, selected navigation, the
current-page dot, focus rings, citations, `source-quote`, eyebrows, role chips,
`.hof-chip`, `.signin-eyebrow`, selected tree nodes and `family-edge` hover.

Family Tree state stays distinct: verified edges use `--ok`, review edges use a
dashed `--warn`, unverified edges use a dotted `--muted`, selected nodes use a
`--accent` fill and stroke, and keyboard focus uses a dashed `--focus-ring`.

## Verification

Run in this worktree on `feat/ui-901-linear-color-refresh`.

| Check | Command | Result |
| --- | --- | --- |
| Full repository validation | `API_PORT=8081 API_URL=http://127.0.0.1:8081 ./scripts/validate.sh` | **PASS** |
| Rust build, tests, fmt, clippy | included in `scripts/validate.sh` | **PASS** |
| Web typecheck and production build | included (`make web-check`) | **PASS** |
| Real-browser Playwright suite | included (`make web-e2e`) | **PASS — 283/283** |
| Transition evidence | included | **PASS** |
| Contrast pairs | `token-contract.md` | **PASS** |
| 390 px overflow | Playwright visual suites | **PASS** |
| `git diff --check` | `git diff --check` | **PASS** |
| Screenshots (light, dark, mobile) | 53 synthetic captures | **PASS** |

Notes:

- The harness was run with `API_PORT=8081` and
  `API_URL=http://127.0.0.1:8081` because an unrelated `tunnel-client` occupies
  port 8080. This is an environment routing choice; no application behavior
  changed.
- The disposable PostgreSQL test databases were recreated before the run.
  Branch-grant events are append-only, so a reused `*_test` database accumulates
  state. Resetting the disposable databases is test-environment hygiene, not a
  product change.
- Six shared visual specs were updated to assert the new palette
  (`#f6f6f7` light, `#121214` dark) instead of the retired stage-3 values. No
  assertion was removed or weakened, and the test IDs and behavior checks are
  unchanged.
- The real-API suites ran under the synthetic OIDC issuer and disposable
  databases. They are not skipped in this result.

## Visual evidence

All screenshots are synthetic and were captured with mocked sessions and API
responses. They cover desktop light, desktop dark, and 390 px mobile where the
capture script defines a mobile state. They were produced with the existing
synthetic capture scripts:

```bash
cd web
BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-linear-color-refresh/after/navigation-ask-admin node scripts/capture-ui-reskin-stage3.mjs
BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-linear-color-refresh/after/family-tree-profile node scripts/capture-ui-reskin-stage2.mjs
BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-linear-color-refresh/after/documents-branch-review node scripts/capture-ui-reskin-stage5.mjs
BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-linear-color-refresh/after/head-of-family node scripts/capture-ui-reskin-stage6.mjs
BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-linear-color-refresh/after/signin-denied node scripts/capture-ui-reskin-stage8.mjs
```

### Shared navigation and Ask the Archive

| Light | Dark |
| --- | --- |
| ![Navigation and Ask, light](./after/navigation-ask-admin/desktop-light-ask.png) | ![Navigation and Ask, dark](./after/navigation-ask-admin/desktop-dark-ask.png) |
| ![More menu, light](./after/navigation-ask-admin/desktop-light-more-open.png) | ![More menu, dark](./after/navigation-ask-admin/desktop-dark-more-open.png) |

Mobile: [menu open, light](./after/navigation-ask-admin/mobile-light-menu-open.png)

### Family Tree and person profile

| Light | Dark |
| --- | --- |
| ![Family tree, light](./after/family-tree-profile/desktop-light-tree.png) | ![Family tree, dark](./after/family-tree-profile/desktop-dark-tree.png) |
| ![Person profile, light](./after/family-tree-profile/desktop-light-profile.png) | ![Person profile, dark](./after/family-tree-profile/desktop-dark-profile.png) |

Mobile: [tree](./after/family-tree-profile/mobile-light-tree.png),
[profile](./after/family-tree-profile/mobile-light-profile.png)

### Documents and Branch Review

| Light | Dark |
| --- | --- |
| ![Documents, light](./after/documents-branch-review/desktop-light-documents.png) | ![Documents, dark](./after/documents-branch-review/desktop-dark-documents.png) |
| ![Branch review, light](./after/documents-branch-review/desktop-light-branch-detail.png) | ![Branch review, dark](./after/documents-branch-review/desktop-dark-branch-detail.png) |

Mobile: [documents](./after/documents-branch-review/mobile-light-documents.png)

### Head of Family

| Light | Dark |
| --- | --- |
| ![Head of Family, light](./after/head-of-family/desktop-light-hof.png) | ![Head of Family, dark](./after/head-of-family/desktop-dark-hof.png) |

Mobile: [Head of Family](./after/head-of-family/mobile-light-hof.png)

### Sign In and denied state

| Light | Dark |
| --- | --- |
| ![Sign in, light](./after/signin-denied/desktop-light-signin.png) | ![Sign in, dark](./after/signin-denied/desktop-dark-signin.png) |
| ![Denied, light](./after/signin-denied/desktop-light-denied.png) | ![Denied, dark](./after/signin-denied/desktop-dark-denied.png) |

Mobile: [sign in](./after/signin-denied/mobile-light-signin.png)

## Environment and test-harness notes

- The theme still follows the system preference only; no manual selector.
- The real-API suites require the `scripts/web-e2e.sh` harness with the synthetic
  OIDC issuer and seeded disposable databases. They passed in this run and are
  not claimed from a different environment.
- Console-error assertions were not added by this task. The 283-test browser
  suite passed, and the visual suites exercise theme switches without a failure.

## Known limitations

- The theme still follows the system preference only; no manual selector.
- The light `--ok` on `--ok-soft` chip pair measures 4.6:1, close to the 4.5:1
  floor. It stays unchanged because the status hue must not change.
- `--line` is decorative and does not reach 3:1. It never marks a control or a
  state; every essential boundary uses `--line-strong` or an accent token.
