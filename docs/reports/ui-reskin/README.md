# Tran UI reskin — stage index and closeout

The reskin moved the Tran family archive web UI onto one shared visual system
without changing backend behavior. Each stage shipped as its own reviewed pull
request, merged to `main` only after review.

## Completed stages

| Stage | Scope                                                                                                      | PR               | Merge commit |
| ----- | ---------------------------------------------------------------------------------------------------------- | ---------------- | ------------ |
| 1     | Shared shell, navigation baseline, Ask the archive                                                         | #101             | `bcdfae9`    |
| 2     | Family tree and person details                                                                             | #102             | `a4c4af0`    |
| 3     | Top navigation and Linear-inspired light/dark tokens                                                       | #109             | `51f6546`    |
| 4     | Owner profile (`/profile`) and its states                                                                  | #110             | `95c8617`    |
| 5     | `/documents`, `/admin/reviews`, `/branch-review`                                                           | #111             | `cf5cd5c`    |
| 6     | `/head-of-family`                                                                                          | #112             | `d50ff1d`    |
| 7     | `/admin/document-extraction`, `/admin/family-members`, `/admin/narrative-review`, `/admin/source-of-truth` | #113             | `61ce4c4`    |
| 8     | `/signin` and shared denied/no-access states                                                               | see stage report | —            |

Stage reports live beside this file (`stage-1/` … `stage-8/`), each with
synthetic before/after screenshots and its own verification table.

### Shared system now in place

- **Top navigation** — one header, primary nav, "More" disclosure, account menu,
  and a mobile menu, shared by every route through `ExperienceShell`.
- **Light/dark tokens** — the stage-3 token set (`--bg`, `--surface`, `--card`,
  `--line`, `--fg`, `--muted`, `--accent`, `--focus-ring`) follows the system
  preference. No stage changed a token value after stage 3.
- **State panels** — `state-panel` with `-loading`, `-empty`, `-expired`,
  `-unavailable`, and `-not-found` variants, used by every page state, including
  denied and no-access views from `DenialView`.
- **Restrained jade** — accent is limited to eyebrows, focus rings, and selected
  states. Primary actions stay neutral.

## Outstanding verification

These are **not** passing results and are kept separate from the verified gates.

- **Stage-one live-RAG timeout** — still outstanding from stage 1; not rerun in
  any later stage.
- **Real-API browser suites** — `auth`, `admin-accounts`, `bulk-onboarding`,
  `narrative-review`, `session-expiry`, `session-renewal`,
  `head-of-family-real`, `branch-review-security`, and the rest of the
  `scripts/web-e2e.sh` cases require the synthetic OIDC issuer and seeded
  disposable databases. They were skipped throughout and are never claimed as
  passing.
- **SOP `PARTIAL` outcomes** — every reskin stage ran as a governed ad-hoc task
  whose capabilities were `LOCAL_DONE` with a **`PARTIAL`** outcome. SOP could
  execute only `git diff --check`; Node, build, and browser checks are
  `REQUIRES_APPROVAL` and were run by the operator. `PARTIAL` here means "some
  criteria had no deterministic check inside SOP's command policy", not a
  failure and not a recorded acceptance decision.
- **Manual theme selector** — theme follows the system preference only; no
  in-app switcher was added.

## Verification approach used by every stage

Each stage defined its acceptance checks before execution, ran them against an
isolated production build on ports `3001`/`3002` while the developer server on
`3000` and API on `8080` kept running, and captured synthetic desktop/mobile
screenshots in both themes. Repository gates were `typecheck`, `build`, the
mocked Playwright visual suites, and `git diff --check`. Passing evidence was
reused across a stage only where the relevant inputs were unchanged, and each
stage report says explicitly which suites were rerun.

## Remaining reskin work

Visual reskin scope is complete once stage 8 merges. Anything still listed under
**Outstanding verification** is a test-harness or environment gap, not an
unfinished screen.
