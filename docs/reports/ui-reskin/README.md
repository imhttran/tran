# Tran UI reskin — stage index and closeout

**Status: complete.** All eight visual stages are implemented and merged to
`main`. The reskin moved the Tran family archive web UI onto one shared visual
system without changing backend behavior. Each stage shipped as its own reviewed
pull request and was merged only after review.

Latest merge: PR #114 at `0c81bdb` (stage 8).

## Completed stages

| Stage | Scope                                                                                                      | PR   | Merge commit |
| ----- | ---------------------------------------------------------------------------------------------------------- | ---- | ------------ |
| 1     | Shared shell, navigation baseline, Ask the archive                                                         | #101 | `bcdfae9`    |
| 2     | Family tree and person details                                                                             | #102 | `a4c4af0`    |
| 3     | Top navigation and Linear-inspired light/dark tokens                                                       | #109 | `51f6546`    |
| 4     | Owner profile (`/profile`) and its states                                                                  | #110 | `95c8617`    |
| 5     | `/documents`, `/admin/reviews`, `/branch-review`                                                           | #111 | `cf5cd5c`    |
| 6     | `/head-of-family`                                                                                          | #112 | `d50ff1d`    |
| 7     | `/admin/document-extraction`, `/admin/family-members`, `/admin/narrative-review`, `/admin/source-of-truth` | #113 | `61ce4c4`    |
| 8     | `/signin` and shared denied/no-access states                                                               | #114 | `0c81bdb`    |

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

## SOP disposition

The reskin used two governance shapes, neither of which belonged to the active
`plan-tran-archive-photographs` plan. That plan was not read, edited, or
transitioned during the reskin.

- **Stages 1–2** ran under their own SOP plans (`plan-tran-ui-reskin-stage-1`,
  `plan-tran-ui-reskin-stage-2`) with tasks `UI-101`…`UI-204`.
- **Stages 3–8** ran as governed ad-hoc tasks on their own branch
  (`sop run --task .agent-sdlc/tasks/<ID>.md`), with tasks `UI-301`…`UI-801`.

Reconciled read-only at closeout (`sop status`, `sop report <id>`,
`sop question list`, `sop approvals`). The recorded outcomes are **preserved as
recorded**, not reinterpreted:

| Run       | Stage | Outcome                  | Lifecycle    |
| --------- | ----- | ------------------------ | ------------ |
| `UI-101`  | 1     | PARTIAL                  | `LOCAL_DONE` |
| `UI-104`  | 1     | PARTIAL                  | `LOCAL_DONE` |
| `UI-203`  | 2     | none recorded (see note) | —            |
| `UI-301`  | 3     | PARTIAL                  | `LOCAL_DONE` |
| `UI-401`  | 4     | PARTIAL                  | `LOCAL_DONE` |
| `UI-501`  | 5     | HOLD                     | `LOCAL_DONE` |
| `UI-601`  | 6     | PARTIAL                  | `LOCAL_DONE` |
| `UI-701A` | 7     | PARTIAL                  | `LOCAL_DONE` |
| `UI-801`  | 8     | PARTIAL                  | `LOCAL_DONE` |

- `PARTIAL` means some acceptance criteria had no deterministic check inside
  SOP's command policy. SOP can execute only read-only `git`/`go`, so it ran
  `git diff --check` and marked the rest "unverifiable: no deterministic check
  supplied". Node, build, and browser checks are `REQUIRES_APPROVAL` and were run
  by the operator.
- `UI-501` recorded `HOLD` because all five acceptance commands were rejected by
  the command policy (the check strings contained backticks and shell
  metacharacters). That is a policy rejection, not a failed implementation: the
  stage-5 work merged in PR #111.
- `UI-203` has no recorded outcome. Per the stage-2 report, a tool overwrote
  `web/src/app/globals.css`; the stylesheet was restored from git, the rules were
  re-appended, and `UI-203`/`UI-204` were completed externally by the operator.

No waiver, approval, or acceptance decision was recorded on the operator's
behalf, and none was attempted. No pending approvals or pending planner
questions exist. Because these runs are ad-hoc rather than persisted plan tasks
(`sop task UI-801` reports "not found"), SOP offers no supported lifecycle
transition to apply, so the `PARTIAL` and `HOLD` outcomes stay exactly as
recorded.

## Remaining verification

Nothing below is a passing result. These gaps are tracked separately from the
verified gates in each stage report.

### Skipped real-API suites

The token-gated browser suites need the `scripts/web-e2e.sh` harness with the
synthetic OIDC issuer and seeded disposable databases. They were skipped in
every stage and are **never** claimed as passing:

- **Authentication** — `auth.spec.ts`, `session-expiry.spec.ts`,
  `session-renewal.spec.ts`.
- **Administration** — `admin-accounts.spec.ts`, `bulk-onboarding.spec.ts`,
  `proxy-rbac.spec.ts`, `branch-grants.spec.ts`,
  `branch-review-security.spec.ts`.
- **Review** — `narrative-review.spec.ts`, `branch-review.spec.ts`,
  `review-console.spec.ts`, `document-extraction.spec.ts`,
  `source-of-truth.spec.ts`.
- **Head of Family** — `head-of-family-real.spec.ts`.

The reskin changed presentation, state clarity, and keyboard access only. No
Rust, API contract, route, redirect, cookie, session call, permission, privacy
rule, filter, pagination, or workflow changed, so this coverage remains
operator/CI responsibility rather than reskin evidence.

### Outstanding stage-one follow-up

- **Live-RAG timeout** — recorded as outstanding in stage 1 and deliberately not
  rerun in any later stage. It is still open and is not reskin scope.

### SOP criteria not verified by its command policy

- All visual acceptance criteria in stages 1–8 remain **unverified by SOP**
  (outcome `PARTIAL`). Their evidence is the operator-run gate recorded in each
  stage report: `typecheck`, `build`, the mocked Playwright visual suites,
  390 px overflow assertions, light/dark token assertions, and `git diff --check`.
- The stage-8 governed change record covers only `signin/page.tsx` and
  `SignInShell.tsx`. The `DenialView` and `globals.css` changes, plus two final
  contract-alignment edits, fall outside that diff and rest on the operator-run
  gate alone. This is recorded in the stage-8 report.

### Known product limitations

- Theme follows the system preference only; no in-app theme selector was added.
- Field-level validation messages keep their existing inline pattern
  (`form-status error`, `.hint`, `.insufficient`); they are not page states.

## Verification approach used by every stage

Each stage defined its acceptance checks before execution, ran them against an
isolated production build on ports `3001`/`3002` while the developer server on
`3000` and API on `8080` kept running, and captured synthetic desktop/mobile
screenshots in both themes. Repository gates were `typecheck`, `build`, the
mocked Playwright visual suites, and `git diff --check`. Passing evidence was
reused across a stage only where the relevant inputs were unchanged, and each
stage report says explicitly which suites were rerun.

## Remaining reskin work

None. Visual reskin scope is complete as of `0c81bdb`. Everything under
**Remaining verification** is a test-harness, environment, or out-of-scope gap,
not an unfinished screen.
