# SOP Development Process — Rust migration

How Agentic SOP was applied to the Go → Rust migration in this repository, and
what it cost. Counts are reconstructed from SOP telemetry
(`.agent-sdlc/runs/**`) and the operator conversation. Nothing here is estimated:
values that cannot be established are marked UNKNOWN, and agent calls are never
equated with user turns.

- First recorded: 2026-10-17
- Last updated: 2026-10-18 (RUST-007D bilingual implementation and scripted parity PASS; live-model criterion UNMET; pending gate)
- SOP: `sop` on PATH, `sop version` = `sop dev` (source base `c1db733` plus the
  reviewed Cargo support and this session's agent-Cargo/gate changes;
  `/home/tomtran/go/bin/sop`, SHA-256
  `46bafeef0a0a04caa86a1a6f2755dc0e84d03b5cfb789e565415fbc027b9f470`)

## 1. Counts

Each count is defined so the categories stay separate.

### User turns

Operator messages that direct or unblock migration execution:

| # | Operator message | Role |
| - | ---------------- | ---- |
| U1 | "prep this repo for a GO to Rush conversion" | Directs the goal |
| U2 | "I installed the Rust MCP" | Unblock attempt (tooling) — no tool was actually available |
| U3 | "I just did the curl install" | Unblocks (installs the Rust toolchain) |
| U4 | attach `sop-end-to-end` skill + `docs/plans/PLAN-RUST-MIGRATION.md` | Directs SOP execution |
| U5 | "Proceed end to end with the validation and plan repair." | Directs repair; authorizes resume |
| U6 | "Also record how SOP supports the migration…" | Directs tracking |
| U7 | "Finish independent configuration… pause at the acceptance-check blocker" | Directs scope change and pause |

- **User turns total: 7.** All 7 either directed or unblocked migration work.

### Manual continuations

Operator instructions needed to *resume* execution after a SOP stop (a subset of
user turns, counted separately, not double-counted as new work):

| When | Stop being resumed | Instruction |
| ---- | ------------------ | ----------- |
| After U2 | Prerequisite gap (no Rust toolchain) | U3 (install toolchain) |
| After the plan-validation failure and the RUST-005 `CONTINUE` stop | Run had stopped | U5 (repair + resume) |
| After the acceptance-check discovery | Execution paused by design | U7 (pause and document) |

- **Manual continuations: 2 unambiguous resumes (U3, U5)**, plus U7 as a
  deliberate *course change / pause* rather than a resume. Recorded as
  **3 continuation-class instructions (1 is a pause)**.

### SOP invocations

Each actual `sop run` launch, including failed launches:

| # | Command | Outcome |
| - | ------- | ------- |
| I1 | `sop run docs/plans/PLAN-RUST-MIGRATION.md` | Plan validation failed (`summary is empty`); no work executed |
| I2 | `sop run docs/plans/PLAN-RUST-MIGRATION.md` | Compiled 7 tasks; ran RUST-001–004 (`LOCAL_DONE`); RUST-005 `CONTINUE`; process exited |

- **SOP invocations: 2** (1 failed launch, 1 execution).

### Agent calls

Two different populations, counted separately and never equated with user turns.

**SOP-internal agent calls** (reported by SOP telemetry, `sop report`):

- **11**, invocation I2 only (RUST-001: 2, RUST-002: 2, RUST-003: 2, RUST-004: 2,
  RUST-005: 2, plan-compile repair: 1).
- Invocation I1: **UNKNOWN** (not reported; it failed before execution).

**External development-agent calls** (model calls by the coding agents doing the
migration work outside `sop run`): **UNKNOWN** — not instrumented by SOP and no
countable telemetry exists for them.

### Task attempts

Initial attempts and retries, per task:

| Task | Attempts | Notes |
| ---- | -------- | ----- |
| RUST-001 | 1 | `LOCAL_DONE` |
| RUST-002 | 1 | `LOCAL_DONE`; SOP spent 1 internal *plan repair* inside synthesis |
| RUST-003 | 1 | `LOCAL_DONE` |
| RUST-004 | 1 | `LOCAL_DONE` |
| RUST-005 | 1 | `CONTINUE (INCOMPLETE_IMPLEMENTATION)` → requeued |
| RUST-006 | 0 | Never ran (blocked behind RUST-005) |
| RUST-007 | 0 | Never ran (blocked behind RUST-005) |

- **Task attempts: 5** (4 completed, 1 requeued, 2 never attempted).

### Approval requests

- **Approval requests: 0.** `sop approvals --json` was empty at every checkpoint.
  (The per-task line "human approval required before commit" is repository
  policy, not an open SOP gate.)

### Plan repairs

Changes needed to make the plan executable or verifiable:

| # | Repair | Purpose | Status |
| - | ------ | ------- | ------ |
| P1 | Canonicalize `PLAN-RUST-MIGRATION.md` to SOP's plan schema (Objective/Depends on/Acceptance fields) | Executable | Done (validated with SOP's compiler) |
| P2 | SOP internal plan repair during RUST-002 synthesis (capability status `UNKNOWN`) | Executable | Done by SOP (reported "Plan repairs: 1") |
| P3 | Add `acceptance_checks` to each stage so criteria are verifiable | Verifiable | **Blocked** (see §6) |
| P4 | Split RUST-005 into RUST-005A (storage) + RUST-005B (ingestion); RUST-006 now depends on RUST-005B | Bounded tasks | Done (validated; 8 stages, all 5 criteria preserved) |

- **Plan repairs: 3 done (P1, P2, P4); 1 blocked (P3).**

### Configuration repairs (tracked separately from plan repairs)

| # | Repair | Status |
| - | ------ | ------ |
| C1 | Add Rust build/test/Clippy/fmt to `.agent-sdlc/config.yaml` validation (Go checks preserved) | Done |
| C2 | Fix `rust/rust-toolchain.toml` pin (channel `1.83.0` + nonexistent `lldb-preview` component) | Done by agent (config repair) — flagged |
| C3 | Regenerate the corrupt `rust/Cargo.lock` (fabricated `thiserror` checksum) | Done — regenerated against crates.io; real checksums |
| C4 | Fix the Rust build/test/lint gaps (`E0277` `Debug` derives; unused `page_id`; missing `PartialEq` test comparisons; `take_while` unit-parsing bug in `parse_duration`; clippy `type_complexity`) | Done — `cargo build`/`test`/`clippy`/`fmt` all pass |

## 2. How SOP was applied

| Stage | What happened |
| ----- | ------------- |
| Discovery | Operator named the plan explicitly; SOP resolved `docs/plans/PLAN-RUST-MIGRATION.md` as the source and assigned Plan ID `plan-rust-migration`. |
| Preflight | `sop status` reported "project is not initialized"; `sop run` bootstrapped `.agent-sdlc/` (config + state.db). No pending approvals. |
| Plan compilation | SOP deterministically parsed the Markdown (`planner.PlanFromMarkdown`) and validated it (`Plan.Validate`). I1 failed here; after P1 it compiled 7 stages. |
| Task scheduling | `taskbuilder` persisted 7 tasks with the dependency chain RUST-001→…→RUST-007; executed in dependency order. |
| Model routing | Provider `ollama`, model `deepseek-v4.1-flash:cloud`, source `configuration`, harness `tool`. No model-routing layer configured (the `models:` block is commented out); class not recorded → UNKNOWN. |
| Implementation | IMPLEMENT ran per task (RUST-001–004); evidence in `.agent-sdlc/runs/RUST-00X/`. |
| Validation | Invocation I2 ran the configured validation (Go-only at the time): 4 validation runs; it never exercised the Rust deliverable. C1 (2026-10-17) added Rust build/test/Clippy/fmt; the full configured sequence now runs green locally (Go + Rust). No SOP invocation has re-run validation since C1. |
| Review | Engine `self`; 0 findings, 0 review runs. |
| Continuation | RUST-005 classified `CONTINUE (INCOMPLETE_IMPLEMENTATION, HIGH)`; autonomy `BALANCED` → `AUTO_CONTINUE`; requeued; then the invocation exited. |
| Checkpoint recovery | SOP state persisted in `.agent-sdlc/state.db`; plan state `ACTIVE`. Not resumed (paused at the acceptance-check gap). |

## 3. Per-task record

"Reported completion" is SOP's task state; "verified acceptance" is whether the
objective's required criteria were proven by an executed passing check.

| Task | Attempts | Validation | Disposition (SOP) | Verified acceptance | Human intervention | Evidence |
| ---- | -------- | ---------- | ----------------- | ------------------- | ------------------ | -------- |
| RUST-001 | 1 | Go validation PASS | `LOCAL_DONE` (outcome `PARTIAL`) | **No** — criteria unverifiable (no checks); and Rust not built | Plan canonicalization (P1) | `.agent-sdlc/runs/RUST-001/` (`report.md`, `outcome.json`, `validation.json`, `gate.json`, `diff.patch`) |
| RUST-002 | 1 | Go validation PASS | `LOCAL_DONE` (outcome `PARTIAL`) | **No** — criteria unverifiable | None during run | `.agent-sdlc/runs/RUST-002/` |
| RUST-003 | 1 | Go validation PASS | `LOCAL_DONE` (outcome `PARTIAL`) | **No** — criteria unverifiable | None during run | `.agent-sdlc/runs/RUST-003/` |
| RUST-004 | 1 | Go validation PASS | `LOCAL_DONE` (outcome `PARTIAL`) | **No** — criteria unverifiable | None during run | `.agent-sdlc/runs/RUST-004/` |
| RUST-005 | 1 | None | `CONTINUE`, requeued | **No** — no change made | None | `.agent-sdlc/runs/RUST-005/` (`classification.json`, `continuations.txt`, `report.md`) |
| RUST-006 | 0 | — | `PLANNED` | No | — | — |
| RUST-007 | 0 | — | `PLANNED` | No | — | — |

Note: every `LOCAL_DONE` was `PARTIAL`. Because `lifecycle.verdict_gate` is off,
a `PARTIAL` verdict still yields `LOCAL_DONE` — so reported completion is **not**
verified acceptance anywhere in this migration.

## 4. Observed benefits

- One command (`sop run <PLAN>`) bootstrapped state, compiled the plan, generated
  the task graph, scheduled it in dependency order, and executed four tasks with
  per-task evidence artifacts — with no per-task operator orchestration.
- Deterministic plan validation prevented execution on a malformed plan (I1).
- Per-task artifacts (`report.md`, `outcome.json`, `validation.json`, `gate.json`,
  `diff.patch`, `trace.json`) made the run auditable after the fact.
- SOP state (`state.db`) preserved progress across the stop without regeneration.
- Failure classification was specific and actionable (the RUST-005 `CONTINUE`
  reason named the exact validation-contract gap).

## 5. Observed friction

- **Initial schema failure.** I1 rejected the plan (`summary is empty`): the
  hand-written plan was recognized-but-invalid, and SOP (correctly) refuses to
  silently repair a recognized plan. Fixed by P1.
- **Go-only validation.** The configured validation ran only Go checks, so it
  never touched the Rust deliverable. Addressed by C1 (config) — but see §6.
- **Missing acceptance checks.** No stage declared `acceptance_checks`, so every
  required criterion was "unverifiable: no deterministic check supplied."
- **PARTIAL reaching LOCAL_DONE.** With the verdict gate off, four unverified
  tasks became `LOCAL_DONE`. This is the core risk: reported ≠ verified.
- **RUST-005 continuation stop.** The task exceeded its mutation window and was
  requeued; downstream tasks never ran.
- **Unbuildable Rust artifacts (resolved).** As produced by the run, the Rust code
  did not compile (`E0277`), shipped a fabricated `Cargo.lock`, and carried a
  broken toolchain pin. The Go-only validation let this reach `LOCAL_DONE`.
  Repaired 2026-10-17 (C2, C3, C4) and supplemented by Rust checks (C1). This is
  direct evidence that a `LOCAL_DONE` with no Rust check was not trustworthy.

## 6. Blocker: native Rust acceptance checks (tool capability gap)

**Classification: tool capability gap.** SOP cannot execute `cargo` as a
deterministic acceptance check, so no Rust criterion can be proven by a passing
check on the current build. Confirmed against the installed binary after the
update finished.

- **Version:** `sop version` → `sop dev`; binary `/home/tomtran/go/bin/sop`,
  mtime 2026-10-17 09:27; source HEAD `a46187b`.
- **Policy code paths:**
  - `internal/commandpolicy/commandpolicy.go` — `Classify` recognizes only `git`
    (read-only subcommands) and `go` (`test`, `build`, `vet`, `fmt`, `list`) as
    `SAFE`; the `default` branch returns `REQUIRES_APPROVAL`. A project `Policy`
    can only *tighten* (`Approve`/`Deny`), never widen.
  - `internal/cli/outcome.go` — `authorizeAcceptanceCommand` tokenizes with
    `toolharness.SplitCommand` (which rejects shell syntax: chaining, pipes,
    redirection, substitution, globs), rejects dangerous flag forms, then requires
    `commandpolicy.Classify == SAFE`. Non-SAFE ⇒ the check is recorded as a
    **boundary** and never executed.
  - `internal/outcome/outcome.go` — an unmet boundary criterion sets the outcome to
    `HOLD`, and `ValidateWaiver` states "a HOLD (an authority boundary crossed) is
    not waivable"; `ResolveLifecycle` maps `HOLD` → `NEEDS_HUMAN`.
- **Reproduction (deterministic):**
  ```
  sop version                                              # sop dev
  grep -n 'case "git"\|case "go"\|default:' internal/commandpolicy/commandpolicy.go
  # classify a candidate (source-level, no run required):
  #   commandpolicy.Classify(["cargo","test"]) -> REQUIRES_APPROVAL
  #   commandpolicy.Classify(["go","test","./..."]) -> SAFE
  #   toolharness.SplitCommand("cd rust && cargo test") -> error (shell metacharacters)
  ```
- **HOLD behavior:** a `cargo …` acceptance check authorizes to a boundary, never
  runs, forces `HOLD`, and `HOLD` is not waivable → `NEEDS_HUMAN`. It can never be
  a passing check.

Consequences recorded, not worked around:

- Cargo commands are **not** disguised as Go commands, not wrapped in a shell to
  bypass policy, and no Rust criterion is marked accepted from an unrelated Go
  check. Those would each be an integrity violation.

Task state and run evidence were preserved (`.agent-sdlc/state.db` and
`.agent-sdlc/runs/**` untouched). See
[`sop-enhancement-rust-acceptance-checks.md`](sop-enhancement-rust-acceptance-checks.md)
for the proposed SOP enhancement.

## 7. Pending / unverified

- RUST-001–004: reported `LOCAL_DONE`; **acceptance unverified** (no passing
  checks). The Rust artifacts they produced did not build; repaired afterwards
  (C3, C4).
- RUST-005A/B–007: not completed (RUST-005 split into two bounded stages, P4).
- Rust checks (build/test/Clippy/fmt): **passing** (C3, C4 done; C1 wired them
  into validation): 64 lib + 10 integration + 1 doc test pass; clippy `-D
  warnings` clean; `cargo fmt --check` clean.
- Required Go reference checks: run during validation and passing.
- Deterministic parity evidence: **none produced** (requires a passing Rust build).
- Acceptance-check capability: blocked (§6).

## 8. Cost note

No time-saving claim is made: there is no measured before/after comparison for
this migration. The counts in §1 are effort accounting, not a productivity delta.

## 9. Update log

### 2026-10-17 — validation repair pass (this update)

Trigger: the operator re-issued the U5+U6 directive (repair + tracking); the
earlier repair pass had stopped at the acceptance-check gap.

- **C1 re-applied.** `.agent-sdlc/config.yaml` validation now runs Rust build,
  test, Clippy, and formatting from `rust/` alongside the unchanged Go checks.
- **C3, C4 done.** Regenerated the fabricated `Cargo.lock`; fixed the compile,
  test, Clippy, and formatting gaps in `rust/`.
- **P4 done.** Split RUST-005; recompiled the plan with SOP's own
  `planner.PlanFromMarkdown`/`Plan.Validate` → 8 stages, chain
  RUST-004→005A→005B→006→007, all 5 criteria preserved.
- **C1–C4 verified locally:** the full configured validation sequence passes
  (`go build/test/vet`, `gofmt`, `cargo build/test/fmt/clippy`).
- **Counts unchanged:** SOP invocations still 2; agent calls still 11; task
  attempts still 5; approval requests 0. No `sop run` was launched in this pass
  because the acceptance-check blocker (§6) still prevents a verified completion,
  so a run could only re-produce `PARTIAL`/`HOLD`.
- **Still blocked:** P3 (acceptance checks). See
  [`sop-enhancement-rust-acceptance-checks.md`](sop-enhancement-rust-acceptance-checks.md).

## 10. Native cargo acceptance-check enhancement (implemented, pending installation)

### 10.1 Base and upstream

- Worktree `~/agentic-workspace/agentic-sop-rust-acceptance`, branch
  `sop-rust-acceptance-checks`, base `a46187b`
  (`a46187bbdc0179964e48e04fcca5c27982a5d145`). Uncommitted work preserved — the
  worktree was **not** rebased.
- `git fetch origin` advanced `origin/main` to `286fa14`. The only commits missing
  from the base are the deceleval-determinism fix (PR #13):
  `internal/deceleval/*` and `evals/decision-model/README.md`.
  **No upstream change touches `commandpolicy`, `config`, or `cli`**, so no
  relevant fix is missing.

### 10.2 Change (branch diff, 8 files + 2 new tests)

| File | Change |
| --- | --- |
| `internal/commandpolicy/commandpolicy.go` | `Policy.AllowCargo` + `isAllowedCargo` (scoped `cargo build\|test\|clippy`, and `fmt` only with `--check`) |
| `internal/config/config.go` | `validation.acceptance_checks.cargo` (default false) + `Config.CargoAcceptanceChecksEnabled()` |
| `internal/cli/outcome.go` | `deriveOutcome`/`authorizeAcceptanceCommand` take the policy (from the already-threaded `cfg`); cargo `--config`/`-Z`/`--target-dir` rejected; `acceptanceCheckPolicy` |
| `internal/commandpolicy/commandpolicy_test.go` | +3 tests (allow-list, tighten-wins, default never widens) |
| `internal/cli/cargo_acceptance_check_test.go` | authorization seam |
| `internal/cli/cargo_acceptance_flow_test.go` | real-cargo outcome flow (`-tags acceptance`) |
| `docs/specs/SECURITY.md`, `docs/plans/ACCEPTANCE-CHECKS-AUTHORING.md` | documentation |

### 10.3 Verification

- `go build ./...`, `gofmt -l` (clean), `go vet ./...`: pass.
- Affected packages in isolation: `commandpolicy`, `config`, `outcome`,
  `taskbuilder`, `toolharness`, `planner`: **ok**.
- End-to-end outcome flow (real cargo, `-tags acceptance`): **PASS** — passing
  check → `VERIFIED` (with argv/exit evidence); failing check → `PARTIAL` (check
  unmet); no check → `PARTIAL` (`objective-acceptance` unmet); rejected commands
  (`cargo run\|install\|fmt`, shell syntax) → `HOLD` boundary; un-opted-in cargo
  check → `HOLD`; a `go` check → `VERIFIED` (unchanged).
- `internal/cli` whole-package run is very slow in this environment and did not
  complete in the observation window. It stalls once at
  `TestRunTasksMalformedPlan` in SQLite `createSchema` (`ExecContext`); that test
  passes in isolation in this worktree (1.4s) and on `main` (0.55s), so the stall
  is environmental, not from this change. The affected-package and flow tests
  above are the evidence used.

### 10.4 Intervention counts (this phase)

| Count | This phase | Total |
| --- | --- | --- |
| User turns | 1 | 8 |
| Manual continuations | 1 | 4 (1 is a pause) |
| SOP invocations | 0 | 2 |
| Agent calls (SOP-reported) | 0 | 11 |
| Task attempts | 0 | 5 |
| Approval requests | 0 | 0 |
| Plan repairs | 0 | 3 done, 1 blocked |

No `sop run` was launched: this phase is SOP **source** work, not a run.

### 10.5 Residual

- The enhancement spec's §1 working-directory field is **not implemented**:
  `--manifest-path` covers a crate in a subdirectory argv-safely, so no
  shell/`cd` workaround is needed. Path *containment* for that argument **is**
  implemented (see 10.6).
- Not committed, pushed, merged, or installed.

### 10.6 Verification added (path containment, nested crates, never-execute)

- **Path containment.** `rejectUnsafeCargoPaths` + `containedRepoPath`
  (`internal/cli/outcome.go`) confine `--manifest-path` to the repo root: it
  refuses an empty value, an absolute path, any `..` component, and a path whose
  **nearest existing ancestor** symlink-resolves outside the root. `--manifest-path`
  is the only path-selecting cargo argument that can reach execution
  (`--target-dir`, `--config`, `-Z` are rejected as dangerous forms). Both
  `--manifest-path VALUE` and `--manifest-path=VALUE` are handled.
  Test: `TestAuthorizeCargoManifestPathContainment` — allowed (in-repo, both forms)
  and rejected (absolute, `..`, symlink escape, empty, missing value): PASS.
- **Nested crate, all four commands.** `TestCargoNestedCrateCommandsVerify` runs a
  real cargo against a crate at `rust/` and asserts `VERIFIED` for
  `cargo build|test|clippy --manifest-path rust/Cargo.toml` and
  `cargo fmt --manifest-path rust/Cargo.toml --check` (plus the `=` forms): PASS.
- **Never execute.** `TestRejectedCargoCommandNeverExecutes` puts a marker-writing
  fake `cargo` on PATH: the allowed control runs (marker appears); `cargo run`,
  `cargo build --config x=y`, and a `../` manifest path do not: PASS.
- **Default-off and Deny-wins.** Default-off is asserted at the authorization seam
  and at the outcome level; Deny-wins over `AllowCargo` is asserted in
  `TestPolicyAllowCargoOnlyWidensScopedShapes`.

### 10.7 Unresolved: full `internal/cli` suite stalls

- **Command:** `go test -count=1 -tags acceptance -timeout 8m ./internal/cli/`
  (in the worktree) → `FAIL ... 480.253s`.
- **Stack:** `store.Open → Store.migrate → createSchema → database/sql.(*DB).ExecContext`
  (`internal/store/sqlite.go:40/79/125`), driver `modernc.org/sqlite`. The
  blocking test varies between runs (`TestRunTasksMalformedPlan`, then
  `TestMemoryAddAndList`).
- **Cause: UNRESOLVED.** Established evidence: it **reproduces on the clean
  `main` worktree** (`a46187b`, no enhancement changes) with the same stack, so it
  is pre-existing and environmental, not from this change; individual tests pass
  in isolation (`TestRunTasksMalformedPlan` 1.4s here, 0.55s on `main`); it is not
  file-descriptor related (`ulimit -n` = 1048576) and not fixed by `-parallel 1 -p 1`;
  the `./...` suite and the full `cli` package both exceed the window. No
  established root cause.
- **Impact:** the full `internal/cli` package suite cannot be run to completion in
  this environment. Affected-package coverage is provided by the isolated,
  green runs (`commandpolicy`, `config`, `outcome`, `taskbuilder`, `toolharness`,
  `planner`) and the targeted `cli` tests (cargo authorization, path containment,
  never-execute, and the acceptance-tagged cargo flow), which all pass.

### 10.8 Default-deny cargo argument policy (review findings F1/F2/F3)

- **F1/F2 resolved.** `internal/commandpolicy` now validates the whole argv against
  a **default-deny allow-list** (`cargoPolicies` / `cargoFlagsAllowed`):
  `build`/`test`/`clippy` accept only `--locked`, `--all-targets`, and
  `--manifest-path` (each at most once; `clippy` may end in the exact suffix
  `-- -D warnings`); `fmt` requires `--check` and accepts `--all` and
  `--manifest-path`. Everything else is refused: `--target` (which also names a
  custom target-spec path), `--target-dir`, `--artifact-dir`, `--config`, `-Z`,
  unknown flags, a `+toolchain` override, positional arguments, a repeated flag,
  and any argument after `--`. The redundant cargo flag block was removed from
  `rejectDangerousForm`; the allow-list is the single source of truth, and under it
  `--manifest-path` is genuinely the only path selector that can reach execution.
  Verified by table-driven `TestPolicyAllowCargo` (42 cases) and the extended
  never-execute test.
- **F3 documented, not wired.** No configuration supplies a project
  `Policy.Approve`/`Deny` for acceptance checks, so `docs/specs/SECURITY.md` now
  states it MUST be honored *when present* and MUST NOT be described as enforced
  today; Deny-wins remains tested at the policy level only. No new policy
  configuration system was added.
- The SQLite full-suite stall remains the unresolved baseline failure (§10.7 /
  [`cli-fullsuite-timeout-evidence.md`](cli-fullsuite-timeout-evidence.md)); it was
  not re-run.

## 11. RUST-003 fixture parity (this phase)

Deterministic parity evidence for the document-processing port; see
[`RUST-003-fixture-parity.md`](RUST-003-fixture-parity.md) for the full matrix.

- **Expected outputs come from the pinned Go reference.** `tools/genfixtures`
  (a read-only consumer of `internal/document`, `internal/loader`,
  `internal/chunking`) writes `rust/fixtures/parity/section_chunk_parity.json`,
  recording provenance (`go1.27.1`, commit `d72fb39`). Expected values are never
  edited to match Rust.
- **Repairs (implementation, not fixtures):** R1 section parser drops the
  pre-heading block; R2 chunker is word-based (was byte-based) and returns no
  chunks for `size == 0` or `overlap >= size`; R3 plain-text loader yields one
  section titled by file name; R4 PDF sections titled `Page N`.
- **Result: 18/18 cases match** the reference on section title/text, chunk
  content/boundaries, ordering, document-global index, page metadata, and loader
  status/incomplete pages (`cargo test --test section_chunk_parity`).
  `source`/`language` propagation is checked separately.
- **PDF extraction status now matches (R5).** `load_pdf` returns
  `PdfExtraction { sections, incomplete_pages }`, mirroring Go's
  `([]Section, error)` shape without losing readable pages: a partial extraction
  keeps its sections **and** names the OCR pages (`pdf-mixed` → sections
  `[Page 1]`, `incomplete_pages [2]`), a fully scanned PDF is `Err(Scanned)`, and a
  hard failure is `Err(Invalid)`. Asserted per case by
  `pdf_extraction_status_matches_reference`; the difference is not documented away.
- **Pre-effect guard + caller check (RUST-005B).** Added `rust/src/ingest.rs`
  (`gate_pdf` / `gate_sections`): fatal loader errors, fully scanned PDFs, partial
  PDFs (reporting the affected pages), and empty sources are rejected **before** any
  embedding call or database write. `rust/tests/ingest_caller_rejects_incomplete_pdf.rs`
  proves a mixed text/scanned PDF causes zero embedding calls and zero database
  writes (control: a complete PDF embeds each chunk and writes once). The plan gained
  an explicit RUST-005B acceptance criterion and a cross-task dependency
  (`Depends on: RUST-005A, RUST-003`), re-validated with SOP's compiler (8 stages,
  VALID). Loader parity (complete) is kept separate from caller parity (the
  production ingest binary is still a stub; wiring is RUST-005B).
- **Checks:** `cargo fmt/test/clippy` and Go `build/gofmt/vet/test` all pass.
- **Counts unchanged:** SOP invocations 2; SOP-internal agent calls 11 (external
  development-agent calls UNKNOWN); task attempts 5; approval requests 0; plan
  repairs 3 done + 1 blocked.
- **SOP state (read with `sop status` / `sop verify`, never edited):** RUST-001–004
  are `LOCAL_DONE`; RUST-003's recorded outcome is `PARTIAL` (its deterministic-
  checks gate passed, but all five criteria are "unverifiable: no deterministic
  check supplied"), so SOP does not count its acceptance as verified. This
  fixture-parity evidence is **external** to SOP's acceptance path; nothing is
  marked `VERIFIED`. (An earlier draft of this section mis-stated the lifecycle as
  `PLANNED`; that value is only the rendered plan's evidence table, not the
  persisted state.)

## 12. SOP Cargo-support integration onto current origin/main

- **Source SHAs.** Reviewed patch base `a46187bbdc0179964e48e04fcca5c27982a5d145`;
  integration base (current `origin/main`) `a9f0495d4d47be4db708101b430a848c03a03cbf`
  (32 commits ahead of the patch base).
- **Integration.** A clean, isolated worktree (`agentic-sop-cargo-integration`,
  branch `sop-cargo-integration`, based on `origin/main`) received the reviewed
  patch. `git apply` was **clean — no conflicts**, and the build required **no
  changes beyond the original patch** (upstream did not touch
  `deriveOutcome`/`authorizeAcceptanceCommand`/`commandpolicy`, so there is no
  signature drift). The original worktree `agentic-sop-rust-acceptance` is intact.
- **Integrated patch.** `docs/reports/rust-migration/sop-cargo-support-integrated.patch`
  — base `a9f0495`, 12 files, 737(+)/16(−), SHA-256
  `48530f34c6998c13bfab3c387c182f6697df87b4cea53beb36122b677e964ac6`. The original
  patch (`sop-cargo-support.patch`, SHA-256 `79dd707e…b893`) remains valid against
  its base `a46187b`.
- **Checks (integrated).** `go build ./...` OK; `go vet` OK; `gofmt -l` clean for
  the touched files; `commandpolicy` + `config` ok; focused `cli`
  (`TestAuthorizeCargo*`, `TestContainedRepoPathRejectsWhitespace`,
  `TestCargoManifestPathSymlinkEscapeRejected`, `TestRejectedCargoCommandNeverExecutes`)
  PASS; acceptance-tagged `TestCargoNestedCrateCommandsVerify` +
  `TestCargoAcceptanceCheckOutcomeFlow` + `TestOperatorAcceptanceACC*` ok. Windows
  **compilation** (`GOOS=windows go build`/`go vet`) OK; Windows **runtime** is not
  exercised here.
- **Full `internal/cli` suite (once).**
  `go test -count=1 -tags acceptance -timeout 8m ./internal/cli/` →
  `FAIL … 480.430s`. Stack: blocked in the **`fsync` syscall**
  (`modernc.org/libc.Xfsync` → `sqlite._full_fsync` → `_syncJournal` →
  `_pagerCommitPhaseOne`) beneath `store.createSchema`. `internal/store` is
  unchanged upstream. Refined hypothesis: a blocking `fsync` (I/O), not a
  deadlock. Not re-run; see
  [`cli-fullsuite-timeout-evidence.md`](cli-fullsuite-timeout-evidence.md).
- **Counts.** SOP invocations 2; SOP-internal agent calls 11; external
  development-agent calls UNKNOWN; task attempts 5; approvals 0; plan repairs 3
  done + 1 blocked.
- **Remaining gaps.** The full `cli` suite still stalls (fsync); Windows runtime is
  unverified; RUST-008 and the rest of the Rust migration remain unimplemented and
  paused. Migration changes stay separate from this SOP enhancement.

### 12.1 Candidate binary smoke test (disposable project)

- **Equivalence.** The integrated patch's 12 resulting files are byte-identical to
  the reviewed patch's, except `internal/config/config.go`, where the only
  difference is upstream's unrelated one-line doc-comment edit (`test -z "$(gofmt
  -l .)"` → `gofmt -l .`) at ~line 1082; the patch's own hunk is identical in both
  (7 occurrences). So the changes are equivalent, with only base/context differences.
- **Candidate binary.** Built with `go build -o /tmp/sop-candidate ./cmd/sop` from
  the integrated worktree (branch `sop-cargo-integration`, base `a9f0495` +
  integrated patch). SHA-256
  `1178a54c168883c801a30092bebf9fec3ca8903d6fb14248cee0541df18596af`. The installed
  binary (`~/go/bin/sop`, SHA-256 `ed6dc1519c221ab1f0e8df1748bd4adbdf08c6eb94dbd9e09013f8ffd33353ef`)
  was **not** replaced.
- **Disposable project.** `/tmp/sop-cargo-trial` (throwaway git repo, no commit).
  Config opts in with `validation.acceptance_checks.cargo: true`; the single stage
  uses `Execution: verify-first`, so no model is invoked.
- **Smoke results.**
  - cargo enabled + valid check → **VERIFIED**; `check-1` argv
    `["cargo","test","--manifest-path","rust/Cargo.toml"]`, exit 0; **agent calls
    0** ("verified first").
  - cargo **default-off** → **HOLD**: boundary `check-1` "not authorized
    (REQUIRES_APPROVAL)".
  - rejected commands (enabled) → **HOLD**: `cargo run` (command rejection) and
    `cargo test --manifest-path ../Cargo.toml` (path containment: "traverses to a
    parent directory").
  - outcome recording confirmed via `.agent-sdlc/runs/TRIAL-001/outcome.json`
    (per-criterion `Command`/`Boundary`/`Met`).
- **Gaps retained.** Full `cli` suite fsync timeout; Windows runtime unverified.
  The candidate is not installed; no commit/push/merge.

### 12.2 Local installation (trial)

- **Installed** the exact verified candidate over `~/go/bin/sop`.
- **Backup / rollback point:** `~/go/bin/sop.pre-cargo-support-20261017T183800Z`,
  SHA-256 `ed6dc1519c221ab1f0e8df1748bd4adbdf08c6eb94dbd9e09013f8ffd33353ef` (the
  pre-install binary; created with `cp -p`, no overwrite).
- **Install method:** copied the candidate beside the target, set `0755`, then
  atomically renamed over `~/go/bin/sop`. Installed SHA-256
  `1178a54c168883c801a30092bebf9fec3ca8903d6fb14248cee0541df18596af`; `sop version`
  prints `sop dev`.
- **Post-install smoke (installed binary):** enabled + valid check → **VERIFIED**
  (`check-1` argv `["cargo","test","--manifest-path","rust/Cargo.toml"]`);
  default-off → **HOLD** ("not authorized (REQUIRES_APPROVAL)"); rejections →
  **HOLD** (`cargo run`; `--manifest-path ../Cargo.toml` "traverses to a parent
  directory").
- **Rollback:** `cp ~/go/bin/sop.pre-cargo-support-20261017T183800Z ~/go/bin/sop`.
- **Gaps retained.** Full `cli` suite fsync timeout; Windows runtime unverified.

## 13. Migration resume with the installed candidate (this run)

- **Binary.** Installed `~/go/bin/sop`, SHA-256
  `1178a54c168883c801a30092bebf9fec3ca8903d6fb14248cee0541df18596af` (verified).
- **Branch.** Migration work moved to branch `rust-migration` (not `main`); all
  uncommitted work preserved.
- **Config.** `validation.acceptance_checks.cargo: true` enabled for this project.
- **Plan.** Executable acceptance checks added to all 9 stages and mapped to their
  criteria (`cargo test --manifest-path rust/Cargo.toml [--test <name>]`; RUST-001
  uses the Go reference checks). Re-validated: 9 stages, **VALID**. Build success is
  not treated as behaviour parity — RUST-003/004/005B map to the parity and caller
  tests, not to `cargo build`.
- **Policy limitation.** RUST-008's Next.js UI has **no authorizable acceptance
  check**: the command policy allows only `go`/read-only `git`/scoped `cargo`, so
  Node.js/`next` checks cannot be declared; only the Rust-side API contract can be
  checked (`--test api_contract`). Reported, not bypassed.
- **Reconcile (operator-directed).** `sop reconcile … --accept-changed RUST-001..004`:
  added `RUST-005A`, `RUST-005B`, `RUST-008`; removed `RUST-005`; updated
  `RUST-006`, `RUST-007`; accepted the changed definitions of the executed
  `RUST-001`–`RUST-004`.
- **Run.** `sop run docs/plans/PLAN-RUST-MIGRATION.md` executed `RUST-005A` →
  **`FAIL`/`BLOCKED (IMPLEMENT_NO_PROGRESS, HIGH)`**: provider
  `ollama`/`deepseek-v4.1-flash:cloud`, 11 tool calls, **0 repository mutations, 0
  changed files**, `termination=no_progress`; validation **NOT RUN**. SOP did not
  auto-continue (it would repeat under unchanged conditions) and did not retry.
  Artifact: `.agent-sdlc/runs/RUST-005A/report.md`.
- **Approvals.** None pending (`sop approvals --json` empty); the block is operator
  intervention, not an approval gate.
- **Counts (this run).** SOP invocations +1 (total **3**); SOP-internal agent calls
  +2 (total **13**); task attempts: `RUST-005A` ×1 (blocked); approvals 0.
- **Blockers.** (1) `IMPLEMENT_NO_PROGRESS` — the model produced no changes for
  RUST-005A; a retry may succeed. (2) **Environment** — `docker` is missing and
  Postgres `:5434` is unreachable, so the pgvector storage/ingestion acceptance
  checks cannot pass in this environment.
- **Gaps.** `RUST-001`–`RUST-004` remain `LOCAL_DONE`/`PARTIAL` (not re-verified);
  `RUST-005B`–`RUST-008` unstarted; full `cli` fsync timeout; Windows runtime
  unverified; the UI is not built, so no browser URL is reported. No commit, push,
  or merge.

## 14. Blocker-resolution attempt (this run)

- **PostgreSQL inspection.** A native PostgreSQL 16 cluster is online at
  `127.0.0.1:5432` (owner `postgres`). The configured `:5434` is the Docker
  `pgvector/pgvector:pg17` image, and **Docker is not installed** — so `:5434` is
  not the correct endpoint here; `:5432` is the suitable instance.
- **Credentials blocker.** No `~/.pgpass`, no `PG*` env; peer role `tomtran` does
  not exist; `sudo` requires a password; `/etc/postgresql/16/main/pg_hba.conf` is
  unreadable; the configured `myuser:mypassword` fails. I therefore **cannot
  authenticate**, so a dedicated migration database cannot be created. Stopped per
  the privileges/credentials stop condition. Existing databases/data were not
  touched; Docker was not installed to satisfy the original assumption.
- **Acceptance checks fixed (step 3).** `--test <name>` is outside the allowed Cargo
  flag set (policy allows only `--locked`, `--all-targets`, `--manifest-path`), so
  those checks would have been unauthorized boundaries. Replaced with the allowed
  whole-suite `cargo test --manifest-path rust/Cargo.toml`. Re-validated with SOP's
  compiler and the command policy: plan **VALID**; all 10 acceptance commands
  **SAFE**. Criteria were not weakened (the whole suite is at least as strong as a
  single test target).
- **Commands execute (step 4).** `go build ./...`, `go test ./...` (16 ok),
  `cargo build --manifest-path rust/Cargo.toml`, `cargo test --manifest-path
  rust/Cargo.toml` (73 lib + 3 caller + 10 integration + 5 parity + 1 doc) — all
  exit 0.
- **Revalidation unsupported (step 5).** No supported SOP operation re-runs a
  `LOCAL_DONE` task: `retry` requeues only `BLOCKED`; `verify` is read-only;
  `reconcile --accept-changed` records the change without re-executing ("SOP will
  not silently replace the definition of an executed task"). RUST-001–004 keep
  their `PARTIAL` verdicts; their new checks are recorded but unexecuted.
  `state.db` was not edited; no verdict invented; manual Zed checks stay pending.
- **RUST-005A no-progress (step 6).** 11 tool calls, 0 repository mutations,
  `termination=no_progress` — the model performed only discovery and cargo
  commands. Proposed bound/split (to apply once a database is available):
  **RUST-005A1** schema + migrations + dimension guard (unit-only);
  **RUST-005A2** DB client/pool + Go↔Rust interop (integration). Requirements and
  dependencies preserved. Not restructured now — the database dependency still
  blocks acceptance.
- **UI (step 8).** RUST-008's Node.js UI cannot be an SOP acceptance check; recorded
  as operator-run evidence (plan note added). No browser URL: the UI is not built
  and the end-to-end flow is unverified.
- **Counts (this run).** SOP invocations +0 (reconcile is not a run); SOP-internal
  agent calls +0; task attempts +0; approvals 0; plan repairs +1 (acceptance checks
  re-authored to the allowed flag set).
- **Stop condition.** Missing database credentials/privileges (a listed stop
  condition). Next action is operator-supplied: create a Postgres role/database (or
  provide credentials) so the migration database and its acceptance checks can run.

## 15. UI plan (PLAN-RAG-WEB-UI) — canonicalized, blocked on the active plan

- **UI plan canonicalized.** `docs/plans/PLAN-RAG-WEB-UI.md` did not compile as a SOP
  plan (`project is empty`, `criteria=0`, `deps=[]`). It was converted to the
  canonical schema (`## Project`/`## Summary` + `## Tasks` stages with inline
  `- **Objective:**` / `- **Depends on:**` / `- **Acceptance:**`), preserving every
  criterion verbatim; the only authorizable check added was
  `cargo test --manifest-path rust/Cargo.toml` (on UI-003). Re-validated: 5 stages,
  deps UI-001→UI-002/UI-003, UI-002+UI-003→UI-004, UI-004→UI-005, **VALID**.
- **SOP invocation** `sop run docs/plans/PLAN-RAG-WEB-UI.md` → **`NEEDS_HUMAN: a
  different plan is already active`**. Active `plan-rust-migration` still has
  unresolved work; SOP will not mix two plans and will not switch away from
  unfinished work. SOP's own resolution: complete or reconcile the active plan, then
  rerun. This is a genuine human boundary — **not crossed**. UI-001/UI-002 were not
  executed, and no manual implementation was substituted for the delegated work.
- **Policy limitation (UI).** UI-001/UI-002 carry **no SOP-authorizable acceptance
  check** (Node.js is outside the command policy). Their verification is operator-run
  evidence and is not counted as SOP-verified.
- **Verification-only follow-up tasks (supported).** SOP supports verification-only
  stages via `Execution: verify-first` + deterministic `acceptance_checks`: such a
  stage runs the checks and derives the outcome without invoking a model. Proposal
  (**not applied**): add `RUST-001V..004V` verify-first stages carrying the recorded
  checks, so RUST-001–004 criteria can be re-verified without rerunning
  implementation. Applying it needs a reconcile of the active plan (operator
  decision); `state.db` was not edited.
- **Database setup instructions prepared** at
  `docs/reports/rust-migration/database-setup.md` (dedicated `rag_migration` role +
  `rag_migration_test` database on `127.0.0.1:5432`, pgvector required). The
  privileged steps were not executed; no passwords requested in chat.
- **Counts (this run).** SOP invocations +1 (total **4**; the UI run stopped at the
  active-plan boundary); SOP-internal agent calls +0; task attempts +0; approvals 0;
  plan repairs +1 (UI plan canonicalized).
- **Preserved.** RUST-001–004 remain `LOCAL_DONE`/`PARTIAL`; the migration plan stays
  active with RUST-005A `BLOCKED`. No commit/push/merge. No browser URL — the UI was
  not run.

## 16. UI-001/UI-002 — external development work (not SOP-executed)

The operator explicitly authorized implementing UI-001 and UI-002 **outside SOP**
while the migration plan stays active. The UI plan was **not** launched through SOP
(SOP refused a second active plan — §15).

- **UI-001 — API contracts** (deliverable):
  `docs/reports/rust-migration/web-ui/API-CONTRACTS.md`. Schemas for
  `GET /api/health`, `POST /api/documents/ingest`, `POST /api/query`; states;
  formats (`.md`/`.markdown`/`.txt`/`.pdf`, no OCR); a 10 MiB upload limit;
  proxy/request timeouts; citation fields (`source`/`section`/`page` nullable/`text`);
  intentional differences. Documentation — not SOP-verified.
- **UI-002 — Next.js prototype** (deliverable): `web/` (Next.js 15 App Router,
  React 19, TypeScript). Reuses inventory conventions (`AppShell`/`PageHeader`/
  `PageTitle`/`PageFooter`/`FormStatus`/`SystemStatus`, `@/*` paths, same-origin
  `/api` rewrite with server-side `API_URL`). **Mock mode is visibly labeled** on
  every screen; responses are simulated and do not come from the Rust pipeline.
- **Mock scenarios**: success, empty, insufficient-evidence, connection-failure,
  invalid-file, incomplete-pdf, loading.
- **Checks (operator-run)**: `npm install` ok; `npm run typecheck` (`tsc --noEmit`)
  clean; `npm run build` ok (Next.js 15.5.28). No SOP-authorizable check exists for
  Node.js, so these are operator-run evidence, not SOP-verified.
- **Running**: `npm run dev` → `http://127.0.0.1:3000` (listening on `127.0.0.1`
  only), HTTP 200, title "RAG Web UI (prototype)", "Mock mode" present.
- **Pending / not implemented**: UI-003 (Axum API), UI-004 (real wiring), UI-005
  (verification). The prototype does **not** use the real Rust pipeline; no Cargo
  check is claimed as evidence for the UI/API criteria. Database integration stays
  pending.
- **Counts (this work)**: SOP invocations +0; SOP-internal agent calls +0; task
  attempts +0; approvals 0. Recorded as **external development** work, distinct from
  SOP-managed execution.

## 17. Prototype operations (Makefile) — external development work

Development outside SOP; SOP state and the active migration plan are unchanged.

- **Changed files**: `Makefile` (added `web-start`/`web-stop`/`web-status`; all
  existing targets preserved), `scripts/web-control.sh` (new control script),
  `web/.gitignore` (`/.run`), `web/README.md` (documented the targets).
- **Behaviour**: starts the Next.js prototype in the background on `127.0.0.1:3000`;
  tracks the PID and log under git-ignored `web/.run/`; resolves Node/npm from PATH,
  the Zed-bundled installation, or `NODE_BIN`/`NPM_BIN`; refuses a duplicate start and
  refuses when port 3000 is held by another service (never stops it); stops only this
  project's process group (npm + next-server) and handles a stale PID file safely;
  reports state, URL, and log path.
- **Verification**: `make web-start` → started (pid), URL, log; `GET / → 200`; a
  second `make web-start` → "already running … not starting a duplicate";
  `make web-status` → running + URL + log; `make web-stop` → stopped, port free; a
  second `make web-stop` → "not running (no pid file)"; with a dummy service on port
  3000, `make web-start` → "port 3000 is already in use; refusing to start (another
  service is left untouched)" and the dummy was left running; `make web-start` again
  → restarted. Left running on `http://127.0.0.1:3000`.
- **Counts (this work)**: SOP invocations +0; SOP-internal agent calls +0; task
  attempts +0; approvals 0. External development work.

## 18. Git hooks for Rust + Next.js — external development work

- **Inspected**: `.githooks/pre-commit` (Go: `go mod tidy`/`fmt`/`vet`/`staticcheck`/
  `govulncheck`/`test`/`build` + a `jq` JSON check) and `.githooks/pre-push` (Go
  integration tests). `core.hooksPath` was **unset** (hooks inactive). The Makefile's
  `.PHONY` named `rust-*` targets that did not exist (dangling).
- **Changed files**: `Makefile` (defined the missing
  `rust-fmt|rust-build|rust-test|rust-clippy|rust-check`, added
  `web-typecheck|web-build|web-check` and `check`), `scripts/web-control.sh` (added
  `typecheck`/`build` subcommands using the same Node/npm resolution),
  `.githooks/pre-commit` (added `make rust-fmt` + `make web-typecheck`),
  `.githooks/pre-push` (added `make rust-build rust-test rust-clippy` + `make
  web-build`), `docs/guides/agent-scripts.md` (installation + the new checks).
- **Existing checks preserved**: every Go check remains in both hooks; none removed.
- **Installed / active**: `git config core.hooksPath .githooks` → **`.githooks`**.
- **Verification**:
  - `make rust-fmt` and `make web-typecheck` clean.
  - `pre-push` in the clean state → **exit 0** (Rust build/test/clippy + Next.js
    production build).
  - **Blocking**: with a deliberately bad Rust line, `make rust-fmt` → exit 2 and
    `.githooks/pre-push` → exit 2 (clippy `-D warnings`). A `git commit --allow-empty`
    was **aborted with no commit created** (HEAD unchanged at `d72fb39`).
  - A **pre-existing** Go `staticcheck` failure (`internal/rag/reliability_test.go`
    ST1012) currently blocks pre-commit before the Rust step; noted, not fixed
    (out of scope).
  - Side effect observed and reverted: the hook's `go fmt ./...` reformatted
    `cmd/retrieval/main.go`; restored to HEAD.
- **Counts (this work)**: SOP invocations +0; SOP-internal agent calls +0; task
  attempts +0; approvals 0. External development work.

## 19. Shared Node/npm toolchain check — external development work

- **Inspected first**: `Makefile`, `web/package.json`, the lockfile
  (`web/package-lock.json`), and `scripts/web-control.sh`.
- **Declared** in `web/package.json`: `engines` (`node >=20.0.0 <25.0.0`,
  `npm >=10.0.0 <12.0.0`), `packageManager: npm@11.19.0`, and `testedWith`
  (`node 24.21.0`, `npm 11.19.0`).
- **Single discovery point**: `scripts/web-control.sh` resolves Node/npm as
  explicit overrides -> PATH -> Zed's bundled Node (glob over any `node-*/bin`,
  Linux and macOS locations; no user name or single Zed version hard-coded), then
  validates both majors against `engines` (a pure-shell parse of the `engines`
  block, so an old node is still judged correctly) and exports the resolved Node
  dir on `PATH` so npm and its children share the toolchain. Missing or incompatible
  tools stop with setup instructions; nothing is installed or upgraded.
- **Reuse**: `check-env` gates `install`, `typecheck`, `build`, and `start`;
  `make web-check-env` / `make web-install` added; both Git hooks call
  `make web-check-env` (pre-commit before typecheck, pre-push before the build).
- **Verification**: default -> **Zed fallback** (`node-v24.21.0`); **PATH discovery**
  (`/tmp/pathbin/node`); **explicit overrides** honoured; **missing tools** -> setup
  hint + non-zero; **incompatible** (node v18 / npm 9) -> "outside the supported
  range" + non-zero; `make web-typecheck` and `make web-build` both exit 0 with the
  resolved toolchain.
- **Counts (this work)**: SOP invocations +0; SOP-internal agent calls +0; task
  attempts +0; approvals 0. External development work.

## 20. Hook + toolchain finish — external development work

- **ST1012 fixed (no behaviour change)**: renamed the sentinel in
  `internal/rag/reliability_test.go`, `failingGeneratorErr` ->
  `errFailingGenerator` (staticcheck ST1012 wants `errFoo`).
  `go test -count=1 ./internal/rag/` passes; `staticcheck ./internal/rag/` clean.
- **Read-only formatting checks**: `pre-commit` no longer runs `go mod tidy` /
  `go fmt ./...` (which rewrote files). It now runs `go mod tidy -diff` and a
  `gofmt -l .` check that reports unformatted files and exits non-zero. Hooks no
  longer modify source files. The pre-existing gofmt drift in
  `cmd/retrieval/main.go` was fixed (behaviour-neutral) so the read-only check
  passes.
- **Toolchain / vulnerabilities**: the flagged stdlib vulns (GO-2026-6617,
  GO-2026-6613) are fixed only in go1.27.2. A global `toolchain go1.27.2` pin broke
  the prebuilt `staticcheck` (older export-data reader), so the pin is scoped to the
  govulncheck step (`GOTOOLCHAIN=go1.27.2 govulncheck ./...`); the rest of the hook
  keeps the default toolchain. `govulncheck ./...` -> "No vulnerabilities found". No
  tools were installed or upgraded.
- **Build robustness**: `web-build` now clears its isolated dist dir first, so a
  stale `.next-build` cannot fail the build; `make web-build` leaves the running UI
  healthy (`GET / -> 200`).
- **Verification**:
  - Both hooks pass on clean source: `pre-commit` exit 0, `pre-push` exit 0.
  - Controlled failures block each hook: with a bad Rust line, `pre-commit` exit 2
    (rust-fmt) and `pre-push` exit 2 (clippy `-D warnings`).
  - Disposable checkout (`rsync` copy) with a misformatted Go file: `git commit
    --allow-empty` was **blocked** by the read-only gofmt check and **no commit was
    created** (HEAD unchanged); the file was left unmodified; the checkout was
    removed and the real repo untouched.
- **Counts (this work)**: SOP invocations +0; SOP-internal agent calls +0; task
  attempts +0; approvals 0. External development work.

## 21. SOP agent-Cargo authorization, verdict-gate enforcement, and storage re-verification

Date: 2026-10-17. This section records the repair work done in the isolated SOP
worktree and the first migration run after the repaired binary was installed. It
does not rewrite any historical outcome.

### 21.1 Source identity and worktree

- Worktree: `~/agentic-workspace/agentic-sop-agent-cargo` (branch `sop-agent-cargo`).
- Base: current `origin/main` at `c1db733e61a40516bda69dfc1f1d977cb0fcb481`
  plus the previously reviewed Cargo acceptance-check patch.
- Uncommitted by instruction; no commit, push, or merge.
- Source changes:
  - `internal/config/config.go` — `agent.tool_cargo` (default `false`).
  - `internal/ollamaagent/{config,bridge,tools,harness}.go` — pass the option
    explicitly into the tool harness; `NativeAgentFromEnvWithOptions` added and
    `NativeAgentFromEnv` preserved as the default-off caller.
  - `internal/cli/cli.go` — composition root reads the project setting and passes
    it without mutating process-wide environment variables
    (`nativeAgentForProject` / `agentToolCargo`).
  - `internal/toolharness/harness.go` — `Config.AllowCargo` is classified through
    the existing scoped `commandpolicy.Policy`.
  - `internal/cli/run.go`, `internal/cli/drive.go` — the operator-enabled verdict
    gate is now authoritative over local completion (see 21.3).
  - Documentation: `docs/reference/LIFECYCLE-VERDICT-GATE.md`,
    `docs/reference/CONFIGURATION.md`.
  - New tests: `internal/ollamaagent/bridge_options_test.go`,
    `internal/cli/agent_tool_cargo_test.go`,
    `internal/toolharness/cargo_agent_tool_test.go`,
    `internal/cli/verdict_gate_scheduling_test.go`.

### 21.2 Agent-tool Cargo authorization

- The setting is **separate** from `validation.acceptance_checks.cargo`: either can
  be enabled without the other.
- Default-off tests reject `cargo build/test/clippy/fmt --check`; opt-in tests allow
  only the scoped shapes (`--locked`, `--all-targets`, `--manifest-path`, and
  clippy's exact `-- -D warnings` suffix; `fmt` requires `--check`).
- Rejected forms include `cargo run/install/publish/add/fix`, `+toolchain`, unknown
  flags, `--target`, duplicate flags, missing/empty `--manifest-path`, post-`--`
  arguments, and plain `cargo fmt`.
- Containment is proven for both `--manifest-path VALUE` and
  `--manifest-path=VALUE`, for absolute, parent-traversal, whitespace-prefixed, and
  symlink-resolved escapes; rejected commands are proven never to execute (marker
  file), and a real nested-crate `cargo test` executes through the harness.
- Project propagation: `agent.tool_cargo: true` reaches the constructed native
  agent; absent/false/unreadable configuration fails closed; two directories do not
  inherit each other's permission.
- Verified commands: `go build ./...`, `go vet ./...`, `gofmt -l` clean;
  `go test ./internal/{toolharness,commandpolicy,ollamaagent,config}` PASS; focused
  `internal/cli` policy/flow tests PASS; installed-binary smoke PASS (see 21.4).

### 21.3 Verdict gate: failed acceptance now blocks advancement

The two-task disposable proof exposed a real gap: the gate recorded
`lifecycle: NEEDS_HUMAN` / `waiver_denied`, then still completed the task as
`LOCAL_DONE` and ran its dependent. The completion path was repaired:

- `emitRunSummary` returns a non-pass outcome when the gate resolves `NEEDS_HUMAN`.
- `runScheduledTask` parks the run at the existing `WAITING_FOR_HUMAN` stage,
  records the existing human-approval request, and requeues through the existing
  bounded retry path instead of calling `completeTask`.
- The scheduler therefore returns without entering the dependent task.
- Flag-off behavior is unchanged (the branch is unreachable).

Proof (`verdict_gate_scheduling_test.go`, persisted state, not console labels):

- VG-001 with a failing required check → outcome `PARTIAL`, lifecycle
  `NEEDS_HUMAN`, VG-001 **not** `LOCAL_DONE`, VG-002 `PLANNED`, and no VG-002 run
  directory.
- VG-001 with a passing required check → both tasks `LOCAL_DONE` with run evidence.

### 21.4 Local installation of the verified candidate

- Candidate: `/tmp/sop-candidate-20261017T1929Z`, SHA-256
  `46bafeef0a0a04caa86a1a6f2755dc0e84d03b5cfb789e565415fbc027b9f470`.
- Installed atomically to `~/go/bin/sop`; installed checksum matches the candidate.
- Backup (the binary that was installed before this step):
  `~/go/bin/sop.pre-agent-cargo-20261017T1929Z`, SHA-256
  `6bdf8e9a44b481e2543617ef6293d8d36a2144aad1a0894c3622b10ad969a772`.
- Rollback: `cp ~/go/bin/sop.pre-agent-cargo-20261017T1929Z ~/go/bin/sop`.
- Installed-binary smoke against a scripted local Ollama `/api/chat`:
  - `agent.tool_cargo: true` → the requested `cargo test --manifest-path
    rust/Cargo.toml` executed (marker written).
  - Same project without the setting → the command was denied before execution
    (no marker; the denial appears in the tool transcript).
  - Separate project directories did not inherit the permission.
- The fake server was stopped; no test processes were left running.

### 21.5 Plan repair and storage verification

- Plan repair: added **RUST-005D — Verify storage client and Go/Rust
  interoperability** (verify-first, depends on RUST-005A + RUST-005C, same two-way
  interop criterion, whole-suite acceptance check). RUST-005B now depends on
  RUST-005D as well as RUST-005C/RUST-003, so downstream work cannot advance before
  storage verification.
- `sop reconcile PLAN --list-changed --json` showed unchanged RUST-001–004,
  RUST-005A/C, RUST-006–008; **updated RUST-005B; added RUST-005D; no changed
  executed task**. `sop reconcile PLAN` applied it. Historical outcomes were not
  rewritten.
- First run after reconcile stopped at the explicit plan-change gate (no work
  executed); this is a supported boundary, not a failure. After reconcile:
  - `sop run PLAN --max-tasks 1` → **RUST-005D `VERIFIED` / `LOCAL_DONE`**,
    `agent calls 0`, `validation runs 1`, 42.6 s. Verify-first avoided model calls.
  - The acceptance check ran the whole Rust suite: `storage_interop` 3/3 passed
    (Rust round-trip, Rust reads Go-written row, Go reads Rust-written row), plus
    85 lib tests, 3 caller tests, 10 integration, 5 parity, 1 doc test. Direct
    reproduction with the root `.env` loaded: exit 0.
- Database evidence is recorded separately from the historical RUST-005C
  `LOCAL_DONE`/`PARTIAL` verdict, which is preserved.

### 21.6 Counts (artifact-backed where available)

| Count | Value | Evidence / limitation |
| ----- | ----- | --------------------- |
| User turns | **UNKNOWN total**; 7 previously enumerated + this session's continuation instruction | The compacted interval's turns are not individually recoverable from artifacts. |
| Manual continuations | **4 recorded** (3 previous + this session's checkpoint continuation) | Instruction-level count; compacted interval UNKNOWN. |
| SOP invocations | **6 recorded** (4 previous + 2 this session) | This session: one plan-change stop, one RUST-005D run. Launches during the compacted interval are **UNKNOWN** (run artifacts show the attempts but not the launch count). |
| SOP-internal agent calls | **17 across the latest run artifacts** (RUST-001:2, RUST-002:3, RUST-003:2, RUST-004:2, RUST-005:2, RUST-005A:2, RUST-005B:2, RUST-005C:2, RUST-005D:0) | Earlier attempts overwritten by later run metrics are **UNKNOWN**. Not equated with user turns. |
| External development-agent calls | **UNKNOWN** | Not instrumented by SOP. |
| Task attempts | RUST-001–004: 1 each; RUST-005: 1; RUST-005A: 1; RUST-005B: 1 (requeued); RUST-005C: 4 continuations (attempt count beyond the budget **UNKNOWN**); RUST-005D: 1 | From run artifacts/`sop status --detailed`. |
| Approval requests | **0** | `sop approvals --json` empty. |
| Plan repairs | **6 recorded** (P1–P4 previous, P5 RUST-005D + RUST-005B dependency, P6 verdict-gate completion-path enforcement) | P3 is now resolved by the Cargo acceptance support; P6 is a code repair required for the gate to be effective. |

### 21.7 Remaining gaps and blockers

- **Full `internal/cli` suite timeout (baseline, unresolved):** one 12-minute run
  timed out with a goroutine stack blocked in
  `modernc.org/sqlite` `_unixSync` during `store.Open` → `migrateHandoffs` from
  `initProject` in `TestGraphRunPreExecutionRunsAfterTriageBeforeImplementation`.
  Log: `/tmp/sop-cli-full.log`. Not reproduced further; not caused by this patch.
- **Windows runtime unverified** (symlink and POSIX-shell tests are explicitly
  skipped, not silently passed).
- **Node.js acceptance checks remain outside SOP policy**; UI verification stays
  operator-run evidence.
- RUST-001–004 remain historical `LOCAL_DONE`/`PARTIAL`; no verify-first follow-up
  has been created for them yet.
- No commit, push, or merge.

## 22. Migration continuation after storage verification, and the RUST-007 gate

Date: 2026-10-17. One `sop run` invocation resumed the active plan after
RUST-005D. It advanced RUST-005B and RUST-006, then stopped at RUST-007's
human-approval boundary. No state was reset and no approval was granted.

### 22.1 Run results

| Task | SOP verdict | Lifecycle | Agent calls | Validation | Wall time |
| ---- | ----------- | --------- | ----------- | ---------- | --------- |
| RUST-005B | `VERIFIED` | `LOCAL_DONE` | 2 | 1 run, PASS | 5m 47s |
| RUST-006 | `VERIFIED` | `LOCAL_DONE` | 2 | 1 run, PASS | 3m 28s |
| RUST-007 | — | stopped at `WAITING_FOR_HUMAN` | 2 | not completed | 4m 10s |

- Direct full Rust suite after the run: exit 0 — 104 lib, 4 pipeline, 3 caller,
  10 Ollama contract, 5 parity, 3 storage interop, 1 doc test.
- RUST-005B added `fingerprint.rs`, `transaction.rs`, `pipeline.rs`, the
  `PgStore` source replacer, and `tests/ingest_pipeline.rs`; RUST-006 added the
  `retrieval/` and `answer/` modules with unit tests.

### 22.2 Verification gap that must not be hidden by `VERIFIED`

Both outcome artifacts mark every substantive free-text criterion `Met: false`
(informational) and only the whole-suite `check-1` required. The gate therefore
reports `VERIFIED` from the suite alone. Inspection shows the criteria are not
all met:

- **RUST-005B:** `rust/src/bin/ingest.rs` is still a configuration-only stub; it
  does not construct `IngestPipeline` or call `gate_pdf`. The "caller-level" test
  (`tests/ingest_caller_rejects_incomplete_pdf.rs`) defines a local helper that
  mimics a caller rather than driving the production binary. The guard is proven
  at the library/fake level, not in the production ingest path.
- **RUST-006:** the task's own implementation summary says wiring into `lib.rs`,
  the `ask` binary, the service composition module, and the ingest→answer test is
  "not yet complete and remain to be finished by the next invocation." The
  free-text criteria (live end-to-end path, shared retrieval for answering and
  evaluation, citation behavior) are marked unmet/informational.

This is the same failure mode the operator warned about: a passing general Cargo
suite is not evidence for missing database, API, UI, or production-caller
behavior. The historical SOP verdicts are preserved, but these tasks must not be
treated as accepted completion until the production wiring and the missing
end-to-end evidence exist.

### 22.3 RUST-007 approval boundary (not crossed)

`sop approvals --json` records one `PENDING` `NEEDS_HUMAN` request for RUST-007:

- The required validation list still contains Go commands (`go build ./...`,
  `go test ./...`, `go vet ./...`, `gofmt`), which cannot pass after the final
  Rust-only criteria remove Go source and `go.mod`.
- RUST-007 is too large for one invocation: it combines porting the remaining
  commands/evaluation, freezing parity evidence, rewriting docs/Makefile/hooks/CI,
  moving Rust to the root, removing Go, and verifying a clean checkout.
- SOP requeued RUST-007 to `PLANNED` (attempts: 1) and parked the run at
  `WAITING_FOR_HUMAN`. It was not approved or declined.

A supported repair (not applied, because the approval is a human decision) is:

1. Split RUST-007 into bounded stages: (A) port the remaining Rust
   commands/evaluation and wire the real ingest/answer caller; (B) freeze parity
   evidence and update docs/Makefile/hooks/CI while Go is present; (C) move Rust
   to the root, remove Go, replace the Go validation commands, and verify a clean
   checkout without Go. RUST-008 depends on the last stage.
2. Add bounded follow-ups for the RUST-005B production-caller wiring and the
   RUST-006 service/bin/end-to-end wiring, with explicit evidence; preserve the
   historical task records.
3. Resolve the pending RUST-007 approval with `sop approve` or `sop decline`
   (operator decision), then reconcile the revised plan and rerun.

### 22.4 Counts (this run)

| Count | Value |
| ----- | ----- |
| SOP invocations | **+1** (total **7 recorded**; compacted interval remains UNKNOWN) |
| SOP-internal agent calls | **+6** (total **23 across latest artifacts**) |
| Task attempts | RUST-005B +1 (after its earlier requeue), RUST-006 +1, RUST-007 +1 (blocked) |
| Approval requests | **1 pending** (RUST-007 `NEEDS_HUMAN`) |
| Plan repairs | **0** in this run |

### 22.5 State and remaining blockers

- SOP plan `plan-rust-migration` is **ACTIVE**; RUST-001–006 are `LOCAL_DONE`
  (RUST-001–005C historical `PARTIAL`; RUST-005D `VERIFIED`; RUST-005B/006
  `VERIFIED` with the caveats above). RUST-007 is `PLANNED` at a pending human
  gate; RUST-008 is `PLANNED`.
- No SOP process is running. No commit, push, or merge.
- Remaining: RUST-007 approval and split, RUST-005B/006 production wiring and
  criterion evidence, Rust-only layout, Axum/Next.js integration, browser
  end-to-end verification, Node.js operator-run checks, the full `internal/cli`
  SQLite timeout, and Windows runtime verification.
- The mock Next.js prototype still answers `http://127.0.0.1:3000` with HTTP 200;
  it is **mock-only** and is not evidence of the real Rust pipeline or RUST-008.

## 23. Concrete migration repair prepared for RUST-007 approval

Date: 2026-10-17. No SOP run was active. The pending RUST-007 gate was read, not
decided. The revised plan was compiled and reconciled only in `--list-changed`
mode; the plan update was **not applied** to SOP state.

### 23.1 Pending gate (unchanged)

- `sop approval RUST-007`: `PENDING`, kind `NEEDS_HUMAN`, stage
  `WAITING_FOR_HUMAN`, task status `PLANNED`.
- SOP's supported resolution commands: `sop approve RUST-007`,
  `sop decline RUST-007`, then `sop run`.
- The reason is the Go-validation/Rust-only conflict plus the oversized scope.
  Neither command was run.

### 23.2 Plan repair

Added bounded follow-ups (historical outcomes preserved):

| Task | Purpose | Depends on |
| ---- | ------- | ---------- |
| RUST-005E | Production ingestion CLI/service wiring through `IngestPipeline`/`gate_pdf` | RUST-005B, RUST-005D |
| RUST-006B | Retrieval and cited-answer CLI/service wiring | RUST-006, RUST-005E |
| RUST-006C | Real ingest → retrieval → cited-answer verification | RUST-005E, RUST-006B |

RUST-007 keeps its ID and historical execution record but is now the bounded
evaluation/command-parity stage. Its former transition scope moved to:

| Task | Purpose | Depends on |
| ---- | ------- | ---------- |
| RUST-007B | Freeze parity/capability/rollback/provenance evidence and pre-removal docs/CI/Makefile/hooks; clean checkout with Go present | RUST-007 |
| RUST-007C | Rust-only layout: root crate, remove Go/tooling, Rust-only validation/docs/CI, clean checkout with Go absent | RUST-007B |

RUST-008 now depends on RUST-007C. It remains mock-only until the real API and
pipeline pass; Node/browser checks remain operator-run evidence.

### 23.3 Criterion-to-evidence mapping and the informational-criteria limitation

- SOP treats each `Acceptance Checks` command as **required** and each free-text
  `Acceptance` line as **informational** whenever checks exist. Informational
  criteria do not affect `VERIFIED`; this is why the whole-suite checks allowed
  RUST-005B/RUST-006 to reach `VERIFIED` with production wiring missing.
- The plan now maps the new required criteria to dedicated acceptance crates:
  `rust/acceptance/ingest_cli`, `retrieval_cli`, `e2e`, `eval_parity`, `evidence`,
  `clean_checkout_go`; and after the move, `acceptance/final_layout`,
  `acceptance/clean_checkout`, `acceptance/api_contract`.
- The whole-suite `cargo test` remains supplementary. It is never the sole
  evidence for a production path, database behavior, API contract, or UI state.
- Clean-checkout criteria use dedicated crates that copy the working tree and run
  the configured checks with a sanitized PATH (Go present for RUST-007B; Go absent
  for RUST-007C), rather than asserting from a developer shell.
- Node.js/browser criteria remain outside the command policy; they are recorded
  separately and cannot produce a SOP `VERIFIED` verdict.

### 23.4 Transition vs final checks

- While Go exists (RUST-001–007B), the Go reference checks remain required.
- After Go removal (RUST-007C), only root Rust checks are required.
- `.agent-sdlc/config.yaml` now uses transition-aware validation: Go checks run
  only when `go.mod` exists; Rust checks follow `rust/Cargo.toml` during the
  transition and `Cargo.toml` at the root afterwards.
- `sop validate` with the revised config: **PASS** (both Go and Rust phases).
- `cargo fmt` was applied to the agent's uncommitted Rust files to remove
  formatting drift introduced by RUST-005B/006; no semantic change.

### 23.5 Validation and reconciliation inspection (not applied)

`sop reconcile docs/plans/PLAN-RUST-MIGRATION.md --list-changed --json`:

- unchanged: RUST-001–004, RUST-005A–D, RUST-005B, RUST-006
- updated: RUST-008 (dependency now RUST-007C)
- added: RUST-005E, RUST-006B, RUST-006C, RUST-007B, RUST-007C
- changed executed: RUST-007 (definition changed; history retained)
- removed/removed executed: none

The plan compiles. Applying the update requires the supported operator-authorized
command:

`sop reconcile docs/plans/PLAN-RUST-MIGRATION.md --accept-changed RUST-007`

This records the changed executed definition without deleting history, resets no
budget, and does not edit `state.db` directly.

### 23.6 Exact proposed gate action and effects

After reviewing and applying the repair, the proposed gate action is:

`sop approve RUST-007`

- Records the operator's approval on the pending `NEEDS_HUMAN` gate.
- RUST-007 is `PLANNED` (runnable), so the approval records authorization as
  provenance and does not spend a retry or requeue a blocked task.
- It does not implement anything by itself; the next `sop run` executes the
  revised bounded task graph.

Alternative, if the repaired plan should replace the gate without authorizing the
old boundary: `sop decline RUST-007` records a negative decision; it does not
authorize execution. The task remains without an approved boundary and will not
be treated as complete.

### 23.7 Installed-binary provenance

- Previously recorded installed binary: `1178a54c168883c801a30092bebf9fec3ca8903d6fb14248cee0541df18596af`
  (the reviewed Cargo acceptance candidate). Its pre-install backup
  `~/go/bin/sop.pre-cargo-support-20261017T183800Z` has SHA-256
  `ed6dc1519c221ab1f0e8df1748bd4adbdf08c6eb94dbd9e09013f8ffd33353ef`.
- At the start of this repair session, the installed binary on disk was **not**
  `1178a54c`; it was
  `6bdf8e9a44b481e2543617ef6293d8d36a2144aad1a0894c3622b10ad969a772`
  (mtime 19:05). It did not contain the `agent.tool_cargo` setting
  (`strings … | grep -c tool_cargo` = 0). Its exact build source/commit is
  **UNKNOWN**: it is an intermediate SOP worktree build (base + Cargo acceptance
  patch, before the agent-Cargo plumbing) or a rebuilt candidate from the
  compacted interval. No artifact identifies it.
- We preserved that actual pre-install binary as
  `~/go/bin/sop.pre-agent-cargo-20261017T1929Z`
  (`6bdf8e9a…`), rather than overwriting it. This is why the new backup checksum
  differs from the previously recorded installed checksum: the backup captures the
  binary that was actually installed at install time, not the older `1178a54c`
  artifact. The older backup (`ed6dc151…`) remains on disk.
- The currently installed binary is
  `46bafeef0a0a04caa86a1a6f2755dc0e84d03b5cfb789e565415fbc027b9f470`,
  built from worktree base `c1db733e61a40516bda69dfc1f1d977cb0fcb481` plus the
  reviewed Cargo patch and the agent-Cargo/gate changes.
- Rollback: `cp ~/go/bin/sop.pre-agent-cargo-20261017T1929Z ~/go/bin/sop`.

### 23.8 Counts (this repair)

| Count | Value |
| ----- | ----- |
| SOP invocations | 0 (no `sop run`) |
| SOP-internal agent calls | 0 |
| Task attempts | 0 |
| Approval requests | 1 pending (RUST-007), unchanged |
| Plan repairs | +1 (bounded follow-ups + RUST-007 split + criterion mapping) |
| User turns / manual continuations | 1 this repair |

No commit, push, or merge. The UI remains mock-only at
`http://127.0.0.1:3000` (HTTP 200) and is not RUST-008 evidence.

## 24. Authorized reconcile, gate approval, and revised-plan resume

Date: 2026-10-17. The operator authorized the prepared repair and the RUST-007
gate. Both were applied. The revised plan then ran RUST-005E twice; both attempts
made zero repository changes, so the run stopped as a real no-progress blocker.
No third unchanged attempt was made.

### 24.1 Applied operations

- `sop reconcile docs/plans/PLAN-RUST-MIGRATION.md --accept-changed RUST-007`
  - accepted the changed executed definition of RUST-007; added RUST-005E,
    RUST-006B, RUST-006C, RUST-007B, RUST-007C; updated RUST-008; no removals.
  - Historical task records and budgets were preserved.
- `sop approve RUST-007 --by operator`
  - recorded `APPROVED`; decision action `ALREADY_RUNNABLE` (RUST-007 was
    `PLANNED`), so no retry was spent and no blocked task was requeued.
  - `sop approvals --json` is now empty.

### 24.2 RUST-005E runs

| Attempt | Result | Evidence |
| ------- | ------ | -------- |
| 1 | `FAIL` → `BLOCKED (NO_PROGRESS)` | 6 implementation turns; 6 tool calls, 5 allowed / 1 denied; 0 mutations; 0 changed files; 1 plan repair; validation NOT RUN. |
| 2 (after `sop retry`) | `FAIL` → `BLOCKED (NO_PROGRESS)` | 17 iterations; 12 discovery inspections; 13 allowed / 2 denied; 0 mutations; 0 changed files; validation NOT RUN. |

Both traces show read-only discovery (list/read/narrate) and no write, create,
delete, or mutating command. The second attempt read `ingest.rs`, `pipeline.rs`,
`ingest.rs`, `embedding.rs`, `config.rs`, `fingerprint.rs`, `transaction.rs`,
`storage_pg.rs`, `lib.rs`, `ollama.rs`, `Cargo.toml`, `tests/ingest_pipeline.rs`,
and `storage.rs`, then hit `termination=no_progress`.

`rust/acceptance` was never created, so the required acceptance checks
(`cargo test --locked --manifest-path rust/acceptance/ingest_cli/Cargo.toml` and
the whole suite) were **NOT RUN**. The task has no outcome verdict; it is
`BLOCKED`, attempts 1.

### 24.3 Why the run stopped

- This is a genuine implementation-progress blocker, not an approval boundary.
  The model produced no repository mutation in two bounded attempts. SOP's
  classification is `TERMINAL (NO_PROGRESS, HIGH)` under the balanced autonomy
  policy.
- Repeating the same task unchanged is explicitly not justified. The smallest
  next step is to make the task materially narrower or provide a concrete
  implementation seam, for example:
  - split RUST-005E into (a) add a callable production `ingest_service` function
    and the `rust/acceptance/ingest_cli` crate, then (b) wire `bin/ingest.rs`,
    config, and the real adapters; or
  - give the agent an explicit file-level change contract for the first stage.
- No downstream task can run: RUST-006B/006C/007/007B/007C/008 all depend on
  RUST-005E directly or transitively.

### 24.4 Counts (this resume)

| Count | Value |
| ----- | ----- |
| SOP invocations | **+2** (total **9 recorded**; compacted interval UNKNOWN) |
| SOP-internal agent calls | **+5** (3 first attempt + 2 retry; total **28 across latest artifacts**) |
| Task attempts | RUST-005E: 2 actual invocations, recorded `attempts: 1` after the supported retry |
| Approval requests | **1 resolved** (RUST-007 approved); 0 pending |
| Plan repairs | 1 in the first attempt (planner correction); 0 in the retry |

### 24.5 Current state

- SOP plan `plan-rust-migration` is ACTIVE.
- RUST-005E is `BLOCKED (NO_PROGRESS)`; RUST-006B/006C/007/007B/007C/008 remain
  `PLANNED`; RUST-001–006 retain their historical states.
- No SOP process is running. No commit, push, or merge.
- The UI remains the mock-only prototype at `http://127.0.0.1:3000` (HTTP 200);
  it is not real RUST-008 evidence.

## 25. RUST-005E split, successful service seam, and RUST-005F blocker

Date: 2026-10-17. The operator authorized the proposed split. RUST-005E was
narrowed to the production service seam + acceptance crate, RUST-005F was added
for binary/config/adapter wiring, and the dependent tasks were re-pointed.

### 25.1 Plan repair applied

- `sop reconcile PLAN --accept-changed RUST-005E --accept-changed RUST-006`
  - narrowed RUST-005E; added RUST-005F; updated RUST-006B/006C dependencies and
    the historical RUST-006 definition (heading correction).
  - RUST-005E retains its ID and prior zero-mutation attempt history.
- A duplicated RUST-006 heading introduced during the split was caught in
  `sop status`, corrected in the plan, and reconciled again before any run.

### 25.2 RUST-005E — VERIFIED

- `sop run` → **RUST-005E `VERIFIED` / `LOCAL_DONE`**, 2 agent calls, 1 validation
  run, 6m 14s.
- Production seam: `rust/src/service.rs` (`IngestService`) delegates to
  `IngestPipeline`, which calls `gate_pdf` before any effect.
- Acceptance crate: `rust/acceptance/ingest_cli/tests/ingest_acceptance.rs` drives
  `rag_foundation::service::IngestService` (production code), not a local helper;
  it asserts mixed/scanned rejection, affected page `[2]`, zero embedding calls,
  zero replacements/rows written, and untouched stored data.
- This is meaningful evidence for the service seam; it is not evidence for the
  binary/config/adapter wiring, which is RUST-005F.

### 25.3 RUST-005F — blocked after multiple supported attempts

RUST-005F (`Production ingest binary/config/adapter wiring`) did not produce any
repository change:

| Attempt | Outcome | Evidence |
| ------- | ------- | -------- |
| Split run | `CONTINUE` 1/4 | discovery budget closed before reading adapter signatures |
| Continuation 2 | `RETRY` (transient) | `ollama returned an empty response` |
| Continuation 3 | `CONTINUE` 2/4 | agent returned `needs_human` for missing constructor signatures |
| Continuation 4 | `BLOCKED (NO_PROGRESS)` | 17 turns, 12 discovery inspections, 13 allowed / 2 denied, **0 mutations, 0 changed files**; it read `bin/ingest.rs`, `config.rs`, `service.rs`, `lib.rs`, `pipeline.rs`, `embedding.rs`, `storage_pg.rs`, `fingerprint.rs`, the acceptance crate, and `Cargo.toml`, then terminated `no_progress` |

The agent repeatedly read the relevant modules but never wrote `bin/ingest.rs` or
extended the acceptance crate. `rust/src/bin/ingest.rs` is still the
configuration-only stub. Validation and the acceptance checks were **NOT RUN** for
RUST-005F. This is a terminal no-progress blocker; no unchanged retry was made.

### 25.4 State and counts (this phase)

| Count | Value |
| ----- | ----- |
| SOP invocations | +4 (total **13 recorded**; compacted interval UNKNOWN) |
| SOP-internal agent calls | +10 (RUST-005E 2 + RUST-005F 8; total **38 across latest artifacts**) |
| Task attempts | RUST-005E: 1 successful (recorded attempts 2 including the earlier blocked stage); RUST-005F: recorded attempts 1, continuations 2/4 |
| Approval requests | 0 pending |
| Plan repairs | +1 (RUST-005E split / RUST-005F addition) |

- SOP state: RUST-005E `LOCAL_DONE`/`VERIFIED`; RUST-005F `BLOCKED`; RUST-006B/006C/
  007/007B/007C/008 `PLANNED`; no SOP process running.
- No commit, push, or merge.
- UI remains the mock-only prototype at `http://127.0.0.1:3000` (HTTP 200), not
  RUST-008 evidence.

### 25.5 Smallest next step

RUST-005F needs a materially narrower binary-level change contract (for example:
"replace only `rust/src/bin/ingest.rs` with a call to `IngestService` using the
already-read constructors; do not touch the acceptance crate yet"), followed by a
separate acceptance-crate extension. Repeating the current RUST-005F unchanged is
not justified.

## 26. Updated SOP (fb49b2f), production ingest wiring, and RUST-006B gate

Date: 2026-10-18. Resumed the migration with the updated SOP binary
`fb49b2fdbd65676d31996900ccb8bcbbd865b7a9` (SHA-256
`2746a6c8d23f1427dff059264cb76672b109744f671827a677c99670ed432d49`). No SOP
installation or enhancement was performed.

### 26.1 Compatibility and prerequisites

- Config migrated from the legacy keys to the new independent form:
  `cargo.agent_tool: true` and `cargo.acceptance_check: true`; legacy
  `agent.tool_cargo` and `validation.acceptance_checks.cargo` removed.
- `sop validate` with the transition-aware validation: **PASS** (Go reference
  while `go.mod` exists; Rust build/test/fmt/clippy).
- PostgreSQL `rag_migration_test` on `127.0.0.1:5432`: reachable; pgvector
  **0.6.0**. Ollama: **healthy**; `deepseek-v4.1-flash:cloud` and
  `nomic-embed-text:latest` present.
- The verdict gate is active. It caught the RUST-006B missing required check and
  stopped at `NEEDS_HUMAN` instead of advancing.

### 26.2 Production ingest wiring

- RUST-005F (`LOCAL_DONE`, historical **PARTIAL**) rewrote
  `rust/src/bin/ingest.rs` to build real adapters from `Config` and call the
  single `IngestService`/`IngestPipeline`; the acceptance checks passed, but the
  updated SOP reported PARTIAL because the plan lacked a parsed acceptance
  mapping.
- Added `rust/tests/ingest_binary.rs`: it spawns the real `ingest` binary via
  `CARGO_BIN_EXE_ingest`, seeds a row in the dedicated database, and verifies
  mixed/scanned/missing inputs exit non-zero, report affected pages, and leave
  the row untouched. It passes against PostgreSQL.
- Fixed a real clippy `question_mark` finding in `bin/ingest.rs`; `cargo fmt`,
  clippy, and the whole suite pass.

### 26.3 New SOP acceptance mapping and verification tasks

- The plan was converted from the legacy `###` task shape to the new SOP's
  rendered `## <id> — <title>` shape so `### Acceptance Mapping` is actually
  parsed. Reconciliation showed **no changed-executed tasks**; only
  RUST-008/mapping metadata and the new tasks changed.
- RUST-005G (legacy-shape verification) remained `LOCAL_DONE`/`PARTIAL`;
  RUST-005H (mapped verification) ran verify-first with **0 agent calls** and
  recorded **VERIFIED / LOCAL_DONE**.
- RUST-006B/006C now depend on RUST-005H.

### 26.4 RUST-006B gate (not crossed)

`RUST-006B` implemented `rust/src/answer/service.rs` (a single `AnswerService`
composing the shared retrieval module, context budgeting, answerability,
prompting, generation, citation repair, and page rendering) and validation
passed, but the required mapped check failed:

`cargo test --locked --manifest-path rust/acceptance/retrieval_cli/Cargo.toml`
→ **manifest path does not exist**.

SOP recorded `PARTIAL` / `NEEDS_HUMAN`, parked at `WAITING_FOR_HUMAN`, and did
not advance. `sop approvals --json` shows one pending gate: **RUST-006B**. It was
not approved or declined. The missing acceptance crate is the concrete blocker;
the smallest fix is to add `rust/acceptance/retrieval_cli` with tests that drive
the production `AnswerService`, then re-verify through a mapped verification
task (preserving RUST-006B's historical outcome).

### 26.5 Counts (this phase)

| Count | Value |
| ----- | ----- |
| SOP invocations | +8 (total **21 recorded**; compacted interval UNKNOWN) |
| SOP-internal agent calls | +12 (total **50 across latest artifacts**) |
| Task attempts | RUST-005F 2; RUST-005G 5 invocations (continuation 4/4); RUST-005H 1; RUST-006B 1 (pending gate) |
| Approval requests | **1 pending** (RUST-006B); RUST-007 previously resolved |
| Plan repairs | +3 (rendered-plan conversion/mappings; RUST-005G; RUST-005H) |

### 26.6 State

- RUST-005E/F/G `LOCAL_DONE` (F/G historical PARTIAL), RUST-005H `VERIFIED`,
  RUST-006B `PLANNED` at the pending gate, RUST-006C/007/007B/007C/008 `PLANNED`.
- No SOP process is running. No commit, push, or merge.
- The UI is still the mock-only prototype at `http://127.0.0.1:3000` (HTTP 200);
  no real Axum/Next.js flow exists yet.

## 27. RUST-006B repair — production answering and retrieval acceptance

Date: 2026-10-18. Direct development work, outside SOP execution. The pending
RUST-006B approval was not touched.

### 27.1 What was missing

RUST-006B's recorded implementation did not persist `answer/service.rs` or a
production query binary, and the mapped acceptance crate
`rust/acceptance/retrieval_cli` did not exist. The gate correctly failed the
required check and parked at `NEEDS_HUMAN`.

### 27.2 Implemented

- `rust/src/answer/service.rs` — `AnswerService` composes the shared retrieval
  `Candidate` type with context budgeting, answerability, prompt construction,
  generation, citation repair, and page rendering. `AnswerError` distinguishes
  retrieval from generation dependency failures.
- `rust/src/answer/mod.rs`, `rust/src/lib.rs` — export the answer and retrieval
  modules (they were previously not compiled as part of the library).
- `rust/src/storage_pg.rs` — `rows_for_corpus` production read path.
- `rust/src/bin/query.rs` + `[[bin]] query` — real production query entry point
  using PostgreSQL rows, the shared vector/FTS/fusion/dedup/expand modules, the
  Ollama embedder/generator, and `AnswerService`.
- Fixed pre-existing compile errors hidden by the unexported modules
  (`expand.rs` reference clone; test-only Clone/Sync issues in query-rewrite and
  rerank stubs).
- `rust/acceptance/retrieval_cli` — new acceptance crate with **5**
  `AnswerService` tests and **4** real query-binary tests:
  - cited source and page metadata;
  - invalid citations dropped;
  - insufficient evidence refuses without generation;
  - retrieval and generation dependency failures reported;
  - real binary answer with `source: <source>, page 2`;
  - real binary refusal on an empty corpus;
  - real binary database failure;
  - real binary generation failure.
  Test-owned rows are deleted in a `Drop` cleanup.

### 27.3 Verification

- `cargo test --locked --manifest-path rust/acceptance/retrieval_cli/Cargo.toml`
  — **PASS**: 5 + 4 tests (with the real database and scripted Ollama server).
- `cargo test --locked --manifest-path rust/Cargo.toml` — **PASS**: 154 lib +
  1 binary + 3 caller + 4 pipeline + 10 contract + 5 parity + 3 interop + 1 doc.
- `cargo fmt --check` and `cargo clippy … -D warnings` for both the main crate
  and the acceptance crate — **PASS**.
- `sop validate` — **PASS** (Go reference + Rust build/test/fmt/clippy).

### 27.4 Gate state and counts

- Pending approval: **RUST-006B `NEEDS_HUMAN` / `WAITING_FOR_HUMAN`**, untouched.
- SOP invocations: **0**; SOP-internal agent calls: **0**; task attempts: **0**.
- Plan repairs: **0**. Direct development only.
- No commit, push, or merge.
- Mock UI remains at `http://127.0.0.1:3000` (HTTP 200); no real browser flow
  has been verified yet.

## 28. RUST-006B verify-first completion and RUST-006C e2e repair

Date: 2026-10-18. Direct repair work plus SOP execution. The RUST-006C gate was
not approved or declined.

### 28.1 RUST-006B completion

The RUST-006B gate repeated because the agent could not re-run the configured
validation forms under the agent-tool policy (shell conditionals, clippy `--`,
and `cargo fmt --manifest-path`). The implementation and mapped checks already
passed. RUST-006B was changed to `verify-first` (same acceptance criteria and
mapping), reconciled with `--accept-changed`, the pending gate was approved, and
the lifecycle ran the checks with **0 agent calls**:

- `RUST-006B` → `VERIFIED` / `LOCAL_DONE`.
- Evidence: `rust/acceptance/retrieval_cli` (5 AnswerService + 4 real query
  binary tests) and the whole Rust suite.

### 28.2 RUST-006C e2e repair

The agent created `rust/acceptance/e2e` with a driver and a frozen baseline but
no `Cargo.lock` and no tests; the fix loop exhausted on the missing lockfile.
Direct repair:

- Added `rust/acceptance/e2e/tests/e2e.rs`:
  - real production ingest of `rust/fixtures/parity/inputs/text-layer.pdf`
    through `IngestService` into PostgreSQL/pgvector;
  - production retrieval + `AnswerService` with a **scripted** fixture embedder
    and mocked generator;
  - asserts the frozen local baseline: 2 chunks, top candidate
    `rust/fixtures/parity/inputs/text-layer.pdf::Page 1::0`, cited answer,
    page-1 reference;
  - covers invalid, incomplete, empty, and oversized inputs with rejection and
    stored-row preservation;
  - asserts the report keeps `[deterministic]` and `[live-model]` sections
    separate and that a scripted run is not presented as live-model evidence.
- Generated `rust/acceptance/e2e/Cargo.lock` (required by `--locked`).
- Moved `serde`/`serde_json` to the crate's runtime dependencies (the driver
  uses them) and fixed the e2e unit-embedder clippy finding.
- Switched the local e2e corpus to the existing frozen text-layer PDF because
  the production ingest path is PDF-based. The local e2e baseline is a
  deterministic regression record, not the frozen Go parity fixture; the Go
  provenance fixtures are untouched.

### 28.3 Verification

- `cargo test --locked --manifest-path rust/acceptance/e2e/Cargo.toml` — **PASS**
  (2 tests: real deterministic path + live-section separation).
- `cargo test --locked --manifest-path rust/Cargo.toml` — **PASS** (154 lib +
  binary/integration/doc tests).
- `cargo fmt --check` and `cargo clippy -D warnings` for the e2e crate — **PASS**.
- `sop validate` — **PASS**.
- Live Ollama: **NOT RUN** for the e2e baseline. The deterministic section is
  scripted-provider evidence; live observations remain opt-in via
  `RAG_E2E_LIVE_OBSERVATIONS` and are reported separately.

### 28.4 Gate and counts

- Pending approval: **RUST-006C `NEEDS_HUMAN` / `WAITING_FOR_HUMAN`**, untouched.
- RUST-006B: `LOCAL_DONE` / `VERIFIED`, 0 agent calls on the verify-first pass.
- RUST-006C: `PLANNED`, attempts 1, fix cycles 3/3.
- SOP invocations: +5 (26 recorded). SOP-internal agent calls: +14 (64 across
  latest artifacts). Plan repairs: +1 (RUST-006B verify-first).
- No commit, push, merge, `state.db` edit, or budget reset.
- UI remains mock-only at `http://127.0.0.1:3000` (HTTP 200); no browser flow.

## 29. RUST-006C verified, RUST-007 blocked, and explicit unmet requirements

Date: 2026-10-18. RUST-006C was converted to verify-first and approved under the
operator's authorization. RUST-007 then made zero repository changes and blocked
at a new approval boundary. That new gate was not approved.

### 29.1 RUST-006C

- Converted to `verify-first` with all criteria and mappings preserved.
- Reconciled with `--accept-changed RUST-006C`; the pending gate was approved
  under the explicit authorization for this task.
- **RUST-006C `LOCAL_DONE` / `VERIFIED`**, **0 agent calls**, validation PASS.
- Evidence: `rust/acceptance/e2e` (2 tests) over the real text-layer PDF and
  PostgreSQL/pgvector, with a fixture embedder and mocked generator.

### 29.2 Evidence separation

The plan now has a **Known unmet requirements** section and RUST-006C states its
evidence scope explicitly:

- **Scripted service-level e2e (satisfied):** production ingest/retrieval/answer
  services over the real database with a fixture embedder and mocked generator.
- **Live-model e2e (UNMET):** no real Ollama answer was run; live observations
  remain opt-in and separate.
- **Markdown/plain-text ingestion (UNMET):** the production ingestion path is
  PDF-only; the e2e corpus is a PDF and does not satisfy text/Markdown loaders.
- **CLI/browser e2e (UNMET):** RUST-006C does not drive the browser UI; the real
  query binary is covered separately by RUST-006B acceptance tests.
- **Go parity (separate):** the frozen Go fixtures/provenance remain the
  reference; the local e2e baseline is a regression record, not new parity.

None of these were removed, remapped, or weakened.

### 29.3 RUST-007 result

The bounded evaluation/command-parity task ran and inspected the Go eval and
bilingual commands, `evals/`, the retrieval modules, and the Rust tree, then
terminated `no_progress`:

- 17 iterations, 12 discovery inspections, 13 allowed / 1 denied tool call,
  **0 mutations, 0 changed files**, validation NOT RUN.
- SOP classified it `NEEDS_HUMAN (APPROVAL_REQUIRED, HIGH)` and blocked it.
- **RUST-007 is `BLOCKED`; a new gate is pending.** This gate was not approved
  and the RUST-006C approval was not reused.

### 29.4 Counts and state

| Count | Value |
| ----- | ----- |
| SOP invocations | +1 (27 recorded) |
| SOP-internal agent calls | +2 (66 across latest artifacts) |
| Task attempts | RUST-006C final verify pass; RUST-007 attempt 1 (blocked) |
| Approval requests | **1 pending — RUST-007** |
| Plan repairs | +1 (RUST-006C verify-first + explicit unmet requirements) |

- RUST-006B/C `LOCAL_DONE`/`VERIFIED`; RUST-007 `BLOCKED`; RUST-007B/C/008
  `PLANNED`.
- No commit, push, merge, `state.db` edit, or budget reset.
- UI remains mock-only at `http://127.0.0.1:3000` (HTTP 200); no browser flow.

## 30. RUST-007 repair proposal — eval narrowed, bilingual follow-up

Date: 2026-10-18. Plan-only repair. The pending RUST-007 gate was not approved,
no run was started, and no code or state was changed.

### 30.1 RUST-007 failure analysis

- Implementation provider/model: **ollama / deepseek-v4.1-flash:cloud**,
  harness `tool`, no fallback.
- Persisted trace: 14 tool calls (13 allowed, 1 denied), 12 discovery
  inspections, **no `write_file`/`create_file`/mutating command**, 0 source
  mutations; termination `no_progress` after narration.
- The sanitized trace does **not** persist the denied call's name/arguments; it
  lists only the allowed calls. The exact denied call is **UNKNOWN**.
- Because the model never attempted a write in the visible trace and spent the
  budget reading/listing/narrating, the primary cause is **implementation
  behavior** (discovery/narration loop), not a demonstrated authorization gap.
  The one denied call cannot be classified from the artifacts; it did not block a
  visible write attempt.
- `changed_files.json` lists `rust/target/debug/ingest` and `smoke` build
  artifacts, not source changes; the run made no source mutation.

### 30.2 Proposed narrowing (not applied to SOP state)

- **RUST-007** (ID/history retained) now covers exactly one production command:
  `eval`.
  - Files: `rust/src/bin/eval.rs`, `rust/src/evaluation.rs`, `[[bin]] eval` in
    `rust/Cargo.toml`, `rust/acceptance/eval_parity/{Cargo.toml,src/lib.rs,tests/eval_parity.rs}`,
    and an invocation-mapping report.
  - Inputs: `evals/retrieval.json` (default; `EVAL_DATASET` override), the
    ingested PostgreSQL corpus, and the fixed configuration
    `QUERY_REWRITE=false`, `EVAL_LEXICAL_RERANK=false`, `EVAL_LLM_RERANK=false`,
    `EVAL_ANSWERABILITY_GATE=false`, `EVAL_FACT_JUDGE=false`, `TOP_K=4`,
    `FINAL_K=3`, `EXPAND_LIMIT=20`.
  - Outputs: per-case top-K source/section ordering and aggregate vector/hybrid
    recall/precision and evidence recall, compared with the pinned Go `cmd/eval`.
  - The acceptance crate is created **with** the production command; a stub or
    fixture-only command must fail it. No passing crate is pre-created.
- **RUST-007D** is the single bilingual follow-up: `expbilingual` arms
  A/B/C/D1/D2/E, `evals/retrieval-vi-en-expanded.json`, `ARM`, `REPEATS`, `DIAG`,
  `QUERY_CACHE`, `GEN_TIMEOUT`, pinned generation (temperature 0, seed 0,
  num_predict 64), cache semantics, and the four-command mapping closure.
  - Its live-model groundedness/citation criterion is explicitly **manual/UNMET**
    until a real Ollama run is recorded.
- **RUST-007B** now depends on both RUST-007 and RUST-007D.

### 30.3 Reconcile delta (validated, not applied)

`sop reconcile docs/plans/PLAN-RUST-MIGRATION.md --list-changed --json`:

- added: **RUST-007D**
- updated: **RUST-007B**
- changed executed: **RUST-007**
- removed / removed executed: none

The plan compiles. No code, dependency, or acceptance-crate file was created.

### 30.4 Closure requirements

The plan's **Known unmet requirements** section now states that these must be
resolved before final project completion/cleanup:

- Markdown/plain-text ingestion through the production path (still PDF-only).
- Live-model end-to-end verification (RUST-007D criterion 4 is manual/UNMET).
- Combined CLI/browser end-to-end (RUST-008 must replace mock-only).
- Go parity remains the frozen reference; the local e2e baseline is not parity.

### 30.5 Checks and counts

- Only the plan compiler was run (`sop reconcile --list-changed`); no full suite
  was rerun. Unchanged RUST-005H/006B/006C evidence is reused.
- SOP invocations: **0**; SOP-internal agent calls: **0**; task attempts: **0**.
- Pending approval: **1 — RUST-007** (untouched).
- Plan repairs: +1. No commit, push, merge, `state.db` edit, or budget reset.

## 31. RUST-007 narrowed eval run — real parity mismatch

Date: 2026-10-18. The prepared reconciliation and gate approval were applied, and
the narrowed eval task ran once with `--max-tasks 1`. It implemented the
production command but did not create the parity crate, so the gate re-opened.
I prepared the parity crate; it exposes a real Go/Rust mismatch. No further run
or gate approval was made.

### 31.1 Run evidence

- Provider/model: **ollama / deepseek-v4.1-flash:cloud**, harness `tool`.
- The agent added `rust/src/bin/eval.rs` and `rust/src/evaluation.rs`; validation
  passed. It did not add `rust/acceptance/eval_parity`, so the mapped check
  failed with "manifest path does not exist"; SOP re-opened `NEEDS_HUMAN`.
- RUST-007 is `PLANNED`, attempts 2, with a pending gate.

### 31.2 Parity crate prepared

- Added `rust/acceptance/eval_parity` with a real behavior test:
  - builds/runs Go `cmd/ingest` and `cmd/eval`;
  - builds/runs the production Rust `eval`;
  - both use the same PostgreSQL corpus, `evals/retrieval.json`, fixed config
    (`QUERY_REWRITE=false`, rerank/answerability/fact-judge off, `TOP_K=4`,
    `FINAL_K=4`, `EXPAND_LIMIT=20`, `MIN_SIMILARITY=0.6`), and the same
    deterministic `/api/embed` vectors derived from the input text;
  - compares per-case hybrid recall/precision at the common emitted K=4 and the
    aggregate metrics, with an absolute tolerance of **0.005** (two-decimal
    output rounding); case count, question order, and rejection labels are exact;
  - cleans only the test-owned corpus rows.
- The temporary leftover `interop://rust/...` test row was removed so the
  unfiltered Go eval and corpus-filtered Rust eval see the same rows.

### 31.3 Exact failure

`cargo test --locked --manifest-path rust/acceptance/eval_parity/Cargo.toml`:

```text
case 0 (How long is the grace period?) recall:
Go=0.0000 Rust=0.5000 differs by more than the documented tolerance 0.005
```

Go's case-0 hybrid top-4 were from `examples/commercial-servicing-manual.md`
while the expected evidence is in `examples/loan-policy.md` /
`examples/large-loan-policy.md`; the Rust eval returned recall 0.5. The likely
cause is the retrieval-path difference: the Rust eval composes the in-memory
vector/FTS/fusion modules over rows, while Go `cmd/eval` uses its SQL vector and
full-text retrieval. This is a real parity gap, not a test fixture issue; the
test was not weakened.

### 31.4 State and counts

- Pending approval: **RUST-007 `NEEDS_HUMAN` / `WAITING_FOR_HUMAN`**, untouched.
- SOP invocations: +1 (28 recorded); SOP agent calls: +2 (68 across latest
  artifacts); RUST-007 attempts: 2.
- Plan repair applied this turn: the previously prepared RUST-007 narrowing.
- No commit, push, merge, `state.db` edit, or budget reset.
- Markdown/text ingestion, live-model E2E, and combined CLI/browser E2E remain
  explicit unmet closure requirements.
- UI remains mock-only at `http://127.0.0.1:3000` (HTTP 200).

### 31.5 Smallest next step

Fix the production Rust eval retrieval to match Go's SQL vector/FTS/fusion path
(or establish a documented, evidence-backed reason for a permitted numerical
difference), then rerun the parity crate. Do not approve the RUST-007 gate or
claim VERIFIED while the parity test fails.

## 32. RUST-007 eval parity repair — SQL retrieval path

Date: 2026-10-18. Direct repair under the existing migration scope. The RUST-007
gate remains pending and was not approved.

### 32.1 Stage-by-stage case-0 comparison

- **Corpus rows/filters:** both commands saw the same test-owned rows. Go
  `cmd/eval` does not filter by corpus; the Rust eval filters by `RAG_CORPUS`.
  The parity test now requires an isolated database and fails closed if any
  foreign corpus exists; it does not delete foreign rows.
- **Query embeddings:** both used the same deterministic `/api/embed` server and
  identical question text, so the query vectors were identical.
- **Vector scores/threshold:** Go uses `1 - (embedding <=> query)` in SQL;
  Rust used in-memory cosine. The agreed `MIN_SIMILARITY=0.6` applied to both.
- **Full-text tokenization/ranking:** this was the first substantive divergence.
  Go uses PostgreSQL `to_tsvector` + `websearch_to_tsquery` + `ts_rank` with the
  language configuration. Rust used its simple in-memory tokenizer/term count.
- **Fusion/tie-breaking:** Go uses RRF k=60, ordered by fused score, best rank,
  then database row id. Rust used the in-memory fusion (string id tie-break),
  then deduplication and expansion before truncation.
- **Evidence matching/metrics:** Go `cmd/eval` scores the raw fused ranking at
  each K. The Rust eval applied dedup/expansion before truncating, which changed
  the case-0 ranking even when the underlying hits were close.

### 32.2 Correction (shared production path)

- `rust/src/storage_pg.rs`: added SQL `vector_search` (pgvector cosine) and
  `keyword_search` (PostgreSQL FTS with the Go language-config mapping), binding
  literals as `text` and casting to `vector`/`regconfig` for the Rust driver.
- `rust/src/retrieval_pg.rs`: added shared RRF fusion with Go's exact ordering
  (score, best rank, database row id).
- `rust/src/evaluation.rs`: added `evaluate_sql`, using the SQL vector/FTS path
  and raw fused ranking at `FINAL_K`, matching Go `cmd/eval`.
- `rust/src/bin/eval.rs`: uses `evaluate_sql`.
- `rust/src/bin/query.rs`: the production query path now uses the same SQL
  vector/FTS retrieval and shared fusion, then the existing dedup/expansion.
- No eval-only workaround and no expected-result change: the Go reference was
  not modified.

### 32.3 Results

- `cargo test --locked --manifest-path rust/acceptance/eval_parity/Cargo.toml`
  — **PASS** (43-case Go/Rust comparison, tolerance 0.005).
- `cargo test --locked --manifest-path rust/Cargo.toml` — **PASS** (169 lib
  tests plus integration/doc tests).
- `cargo test --locked --manifest-path rust/acceptance/retrieval_cli/Cargo.toml`
  — **PASS** (5 AnswerService + 4 real query-binary tests) after the query-path
  change.
- `cargo fmt --check` / `cargo clippy -D warnings` for main and eval-parity
  crates — **PASS**.
- `sop validate` — **PASS**.

### 32.4 Configuration and remaining gaps

- `FINAL_K=4` is the documented common-metric comparison because Go emits
  `K=1,2,TOP_K` and does not emit K=3. It **does not** prove the original
  `FINAL_K=3` behavior.
- The Go eval has no corpus filter. The parity test now requires an isolated
  database and fails closed if any foreign corpus is present; it does not delete
  rows by source prefix. (One pre-existing `interop://rust/...` test row was
  removed during diagnosis before this guard existed; the test cleanup itself
  uses only its unique run corpus.)
- Markdown/text ingestion, live-model E2E, and combined CLI/browser E2E remain
  explicit unmet closure requirements.
- Pending approval: **RUST-007 `NEEDS_HUMAN` / `WAITING_FOR_HUMAN`**.
- No commit, push, merge, `state.db` edit, or budget reset.

## 33. RUST-007D Claude recovery and bilingual implementation

Date: 2026-10-18. One bounded RUST-007D recovery was run with the Claude command adapter after the installed SOP snapshot fix, using the existing task budget. No DeepSeek retry was made.

### 33.1 Backend and context

- Installed SOP: `fff8d742...`, source revision `7a1d1759...`, includes the Git-aware `SnapshotFiles` fix. No SOP rebuild.
- Effective implementation backend: `SOP_AGENT_PROVIDER=command` with the Claude Code adapter, authenticated through the **Claude.ai Pro subscription** (`authMethod: claude.ai`), `jq` 1.7. No paid-API key and no invented spending cap.
- The SOP report header prints `Provider: ollama` because the configured provider remains ollama; the startup/effective provider was `command`. This is a reporting-label discrepancy, not the executed backend.
- `SOP_TASK_INPUTS` supplied a 4.2 KB recovery context with the Go `cmd/expbilingual` contract and the verified Rust SQL/retrieval/API signatures.
- Tool restrictions: `SOP_CLAUDE_ISOLATE=1`; explicit cargo rules for build/test/fmt/clippy/generate-lockfile; no git commit/push/merge/reset/clean. The command backend does not enforce the native toolharness policy; the wrapper allow-list was the restriction.

### 33.2 Outcome

- Validation: all build/test/lint commands **PASS**.
- Objective criteria 1–3 and 5: **met** (invocation mappings, arms/dataset/env, pinned generation/cache, no required check unverified).
- Criterion 4 (live-model groundedness/citations): **UNMET**, as documented in `RUST-007D-live-model-evidence.md`. Scripted parity does not close it.
- Outcome: `PARTIAL`; lifecycle `NEEDS_HUMAN`; RUST-007D is `PLANNED` with a pending gate. The run made meaningful repository changes; this is a required live-evidence boundary, not a no-progress failure.
- Implementation: `rust/src/bilingual.rs`, `rust/src/bin/expbilingual.rs`, `PgStore::section_chunks` in `rust/src/storage_pg.rs`, `rust/tests/expbilingual_binary.rs`, `rust/tests/section_chunks_pg.rs`, and `rust/acceptance/bilingual_parity` with Go/Rust parity tests. The harness is evaluation-only and adds no writes.
- Budget: task attempts **2/3**; continuations **2/4** (2 remaining). Aggregate run counts: 4 attempts, 2 continuations, 4 invocations, 9 model calls. Current run metrics: agent calls 2, validation runs 1, review runs 1, plan repairs 0, wall 30m 26s. No budget reset.

### 33.3 Remaining open requirements

- Live Ollama ingest → query → cited answer and groundedness/citation evidence.
- Original K=3 parity (current eval proof uses FINAL_K=4).
- Markdown/plain-text ingestion.
- Real Axum/Next.js browser flow (UI remains mock-only at `http://127.0.0.1:3000`, HTTP 200).
- Rust-only clean checkout after required parity passes.
- Go removal remains deferred.

No merge or auto-merge was performed. RAG CI remains absent; SOP repository CI is separate.

## 34. RUST-007D verified recovery, acceptance audit, and recovery binding

Date: 2026-10-19. This section records the completed RUST-007D verdict. Nothing
in `.agent-sdlc/` was edited, and no budget, attempt counter, criterion, or gate
was reset.

### 34.1 No active SOP process

- No `sop run`, SOP agent, or implementation subprocess was active when this
  record was written (`ps` showed only the long-running Next.js prototype and
  unrelated system processes; `sop approvals --json` returned an empty list).
- The plan is idle at `ACTIVE`; the only open work is `RUST-007B` (see §34.5).

### 34.2 Verified outcome

- `sop verify RUST-007D` → `status: VERIFIED`, `validation: passed`,
  `lifecycle: LOCAL_DONE`.
- `state.json` → `stage: PASSED`; `gate.json` → `Decision: PASS`.
- All five required criteria are `Met`: criteria 1–3 and 5 through their mapped
  deterministic checks; criterion 4 (live-model groundedness/citations) through
  the operator acceptance bound to it.
- The eight configured validation commands (Go build/test/vet/gofmt and Rust
  build/test/fmt/clippy) all exited `0`.
- Execution was verify-first: `report.json` records `execution_mode:
  verify-first`, `verified_first: true`; the zero-mutation verification reused
  the delivered implementation (`agent_calls: 0`, `agent_calls_avoided: 1`,
  `validation_runs: 1`, `review_runs: 0`, `fix_cycles: 0`). No model call was
  needed to close the task.

### 34.3 Preserved acceptance audit

- `.agent-sdlc/runs/RUST-007D/acceptance_decisions.jsonl` preserves the operator
  decision: `criterion-4`, `decision: accept`, `approver: operator`,
  `recorded_at: 2026-10-18T23:45:16Z`, bound to `attempt: 1`. The evidence names
  `docs/reports/rust-migration/RUST-007D-live-model-evidence.md` and the two
  repaired live repeats plus the Go baseline artifacts under
  `docs/reports/rust-migration/live-007d/`.
- `.agent-sdlc/runs/RUST-007D/acceptance_recoveries.jsonl` preserves the
  recovery binding: `from_attempt: 1` → `to_attempt: 3`, `source: approve`,
  `by: operator`, `recorded_at: 2026-10-19T03:34:18Z`. This is the binding that
  moved the accepted criterion-4 evidence onto the reviewed attempt without
  re-running the live model.
- `.agent-sdlc/runs/RUST-007D/recovery.jsonl` records the earlier attempt-2
  `authorization_failure` → `human` stop. It is preserved, not overwritten.
- `approval-history.json` preserves both `NEEDS_HUMAN` requests and their
  approvals (2026-10-18T23:45:18Z and 2026-10-19T03:34:18Z).
- `.agent-sdlc/` remains git-ignored (`*`) and untracked; the audit is preserved
  on disk and was not modified by this record.

### 34.4 Installed SOP revision

- Installed binary: `/home/tomtran/go/bin/sop`; `sop version` → `sop dev`.
- `go version -m` build information: module
  `github.com/imhttran/agentic-sop`, pseudo-version
  `v0.0.0-20261019032842-205701add58c`, `vcs.revision`
  `205701add58cfebee9b83d3e7afb86720f31cfff`, `vcs.time`
  `2026-10-19T03:28:42Z`, `vcs.modified: false`, `go1.27.1`.
- SHA-256: `0d1284ae195b8cbea01cd64f94623e130bc1bbcfd2ebc1db54c8af8f2f469ea3`.
- This supersedes the `fff8d742` / `7a1d1759` snapshot named in §33.1; that
  value described the binary used for the earlier recovery, not the binary that
  produced the VERIFIED verdict recorded here.

### 34.5 State and remaining requirements

- `sop status`: RUST-007D is `LOCAL_DONE`; `RUST-007B` and `RUST-007C` remain
  `PLANNED`; `RUST-008` remains `PLANNED`.
- Still explicitly open (not implied closed by RUST-007D): original K=3 parity
  (current eval proof uses `FINAL_K=4`), Markdown/plain-text ingestion, live
  production ingest → query → cited answer, and the real Axum/Next.js browser
  flow. The Rust-only clean checkout and Go removal remain deferred to
  `RUST-007B`/`RUST-007C`.
- Next unmet requirement: `RUST-007B` (freeze transition evidence and update
  pre-removal tooling while Go remains present).

## 35. RUST-007B verified — transition evidence frozen with Go present

Date: 2026-10-19. Two `--max-tasks 1` runs, no state edit, no budget reset, no Go
removal.

### 35.1 Run 1 — FAILED, root cause isolated

- `sop run docs/plans/PLAN-RUST-MIGRATION.md --max-tasks 1` executed only
  RUST-007B. Validation passed, then two review cycles failed on HIGH findings,
  and the run ended `FAILED` with `CONTINUE` (`INCOMPLETE_IMPLEMENTATION`).
- Root cause: the frozen evidence record was written under `docs/reports/`, and
  SOP deliberately excludes `docs/reports/` (and `.agent-sdlc/`) from the
  reviewed implementation diff. The review therefore could not see the document
  and reported it missing even though it existed.
- Secondary defects in the same attempt: `rust/acceptance/clean_checkout_go` had
  no `Cargo.toml`/`Cargo.lock`; the documented section was `###` while the
  harness matched `##`; and the harness did not forward `DATABASE_URL`, so the
  documented `make rust-test` step failed inside the staged copy.
- `changed-files.json` and `diff.patch` from this attempt are preserved under
  `.agent-sdlc/runs/RUST-007B/`.

### 35.2 Operator corrections between runs (no SOP state written)

- Moved the evidence record to `docs/reference/rust-transition-evidence.md`, a
  path SOP includes in the reviewed diff, and updated every reference.
- Added `rust/acceptance/clean_checkout_go/Cargo.toml` and generated both
  acceptance `Cargo.lock` files so the `--locked` checks can run.
- Made `Clean checkout with Go present` a top-level section in
  `docs/guides/usage.md`, and made the clean-checkout harness read the source
  checkout's `.env` and forward its values to the documented steps (the file is
  never copied into the staged tree). Added unit tests for that reader.
- Completed the pre-removal tooling: `Makefile` targets
  `rust-acceptance-evidence`, `rust-acceptance-clean-checkout`, and
  `transition-evidence` wired into `rust-check`; `.githooks/pre-commit` runs the
  evidence crate and `.githooks/pre-push` runs `make transition-evidence`; and
  `.github/workflows/ci.yml` runs the Go reference and the Rust/transition jobs
  with both toolchains installed. The command reference documents the targets.

### 35.3 Run 2 — VERIFIED

- `sop run docs/plans/PLAN-RUST-MIGRATION.md --max-tasks 1` → RUST-007B
  `PASSED`; `sop verify RUST-007B` → `VERIFIED`, `validation: passed`,
  `lifecycle: LOCAL_DONE`.
- All four checks and all four criteria are `Met`:
  `go build ./...`, `go test ./...`, `go vet ./...`,
  `cargo test --locked --manifest-path rust/Cargo.toml`,
  `cargo test --locked --manifest-path rust/acceptance/evidence/Cargo.toml` (5
  tests), and
  `cargo test --locked --manifest-path rust/acceptance/clean_checkout_go/Cargo.toml`
  (3 tests, including the staged-copy run of `go build`, `go test`, `go vet`, and
  `make rust-test`).
- The eight configured validation commands all passed. The run used 0 fix
  cycles, 2 agent calls, one validation run, and one review run; wall time was
  7m 13s.
- The installed SOP binary is the revision recorded in §34.4
  (`205701a…`, SHA-256 `0d1284ae…`).

### 35.4 Frozen evidence and preserved state

- `docs/reference/rust-transition-evidence.md` records the capability matrix,
  remaining limitations, rollback reference, pinned Go commit SHA
  `d72fb398e8959eea83ed24be03f6df8cdeca86a2`, the source repository URL, and the
  fixture provenance for `migrations/*.sql`, `evals/`, `examples/`,
  `docker-compose.yml`, and `rust/fixtures/`.
- `rust/acceptance/clean_checkout_go` stages a working-tree copy and runs the
  documented setup with Go present.
- Go sources, `go.mod`, `go.sum`, and all frozen assets remain untouched. No
  `state.db` edit, budget reset, merge, or auto-merge occurred.

### 35.5 Counts and remaining requirements

- RUST-007B: 2 attempts, 2 invocations, 15 model calls, 0 approvals,
  1 continuation (the run-1 `AUTO_CONTINUE` classification).
- `sop status`: RUST-007B is `LOCAL_DONE`; RUST-007C and RUST-008 remain
  `PLANNED`. RUST-007C was **not** started and Go was not removed.
- Still explicitly open: original K=3 parity (the eval proof uses `FINAL_K=4`),
  Markdown/plain-text ingestion, live production ingest → query → cited answer,
  and the real Axum/Next.js browser end-to-end flow.
- Next unmet requirement: `RUST-007C` (Rust-only transition and final
  verification), which must not start until the open parity and live-flow
  requirements are resolved.

## 36. RUST-007E functional closure — verified

Date: 2026-10-19. This section supersedes §35.5's open list: the original K=3
parity, production Markdown/plain-text ingestion, and the live production RAG
path are now verified. Go remains present.

### 36.1 Preflight and plan update

- Installed SOP: `7540edc9bc2b3bb5a9c6cfe4f3799021b807e085` (`sop dev`), matching
  the requested revision. No SOP process was active.
- Isolated branch: `rust-007e-functional-closure`, based on the verified
  RUST-007B commit `2e17ee1`. RUST-007B and RUST-007D remained
  `LOCAL_DONE`/`VERIFIED`.
- Added `RUST-007E` (dependencies RUST-007, RUST-007D, RUST-007B) and
  `RUST-008A` (dependency RUST-007E); changed RUST-007C to depend on RUST-007B,
  RUST-007E, and RUST-008A. RUST-008 still depends on RUST-007C.
- Reconcile preview: 19 unchanged, `RUST-007C` updated (never executed),
  `RUST-007E`/`RUST-008A` added, no executed-task changes. Applied with
  `sop reconcile`.
- RUST-007E was later switched to `Execution: verify-first` after its first
  implementation attempt failed. SOP required the executed-definition change to
  be accepted explicitly; `sop reconcile … --accept-changed RUST-007E` recorded
  it. No criterion was weakened. The BLOCKED task was requeued with
  `sop retry RUST-007E`.

### 36.2 First run — no-progress block, no repeated attempt

- `sop run docs/plans/PLAN-RUST-MIGRATION.md --max-tasks 1` with the configured
  Ollama provider ran RUST-007E for 3 minutes: 17 iterations, 11 discovery
  inspections, **0 repository mutations**, `IMPLEMENT_NO_PROGRESS`. The last
  action was a rejected `write_file`.
- SOP classified it `NO_PROGRESS` / `BLOCK` / `TERMINAL` and did not
  auto-continue, because a retry under unchanged conditions would repeat.
  `.agent-sdlc/runs/RUST-007E/` preserves that attempt.
- The implementation was completed by the operator within the authorized scope;
  SOP still owned the verdict. The run was not claimed as SOP-implemented.

### 36.3 Implemented

- Production ingest dispatch (`rust/src/pipeline.rs`, `rust/src/service.rs`,
  `rust/src/bin/ingest.rs`): `.pdf` keeps the PDF loader and incomplete-page
  gate; `.md`/`.markdown` use the shared section parser; `.txt` uses the
  plain-text loader; unknown extensions are rejected before any effect. Both
  page-less paths run `gate_sections` and converge on the existing fingerprint,
  embedding, and transactional replacement path.
- K=3 parity: added `rust/acceptance/k3_parity` and the Go `tools/rankdump`
  reference helper. The ranking comparison exposed a real divergence — Rust
  evaluation printed every chunk, while Go deduplicates sections after fusion.
  Added `retrieval::dedup::dedup_sections` and used it in
  `evaluation::evaluate_sql`, matching the Go pipeline.
- Markdown/plain-text acceptance: `rust/acceptance/ingest_md_text` proves
  provenance, ordering, unchanged duplicates, empty/missing/unknown rejection,
  embedding failure isolation, and dimension-failure rollback.
- Live RAG: `rust/acceptance/live_rag` drives the production `ingest`/`query`
  binaries against real Ollama (`nomic-embed-text`, `llama3.2:3b`) and
  PostgreSQL/pgvector, validates every citation against stored rows, and proves
  the empty-corpus refusal. Evidence is recorded in
  `docs/reports/rust-migration/RUST-007E-live-evidence.md`.
- Added Makefile targets and command-reference entries for the three acceptance
  crates; the live target is explicit and not part of the ordinary edit loop.

### 36.4 Verify-first run — VERIFIED

- `sop run docs/plans/PLAN-RUST-MIGRATION.md --max-tasks 1` (verify-first) →
  RUST-007E `PASSED`; `sop verify RUST-007E` → `VERIFIED`,
  `validation: passed`, `lifecycle: LOCAL_DONE`.
- All five criteria are `Met`:
  K=3 parity (`k3_parity`), Markdown/plain-text ingestion (`ingest_md_text`),
  transactional safety and provenance (`ingest_md_text`), live Ollama/PostgreSQL
  RAG (`live_rag`), and the existing Go/reference plus transition checks.
- All nine acceptance checks passed, including `go build/test/vet`, the Rust
  suite, `eval_evidence`, and the clean-checkout-with-Go crate. The eight
  configured validation commands also passed.
- Run metrics: 0 agent calls (verified first), 1 validation run, 0 fix cycles,
  3m 52s.

### 36.5 Results

- **K=3 parity:** per-case ranking order, tie ordering, K=3 recall/precision,
  rejection labels, and aggregate metrics match the pinned Go reference. K=4
  output was not substituted.
- **Markdown/plain text:** both formats ingest through the production binary;
  rows keep source, language, chunk order, `page = NULL`, corpus, and hash;
  duplicates are no-ops; rejected/failed cases leave prior rows intact.
- **Live RAG:** Markdown, PDF (page 2), and plain-text citations validated
  against stored rows; the empty corpus returned the baseline refusal with no
  citations.
- **Existing checks:** Go reference and RUST-007B transition/clean-checkout
  checks remain green. Go is still present and operational.

### 36.6 State and next milestone

- `sop status`: RUST-007E `LOCAL_DONE`; RUST-008A `PLANNED`; RUST-007C
  `PLANNED`; RUST-008 `PLANNED`.
- No `state.db` edit, budget reset, approval reset, merge, or auto-merge.
  RUST-008A and RUST-007C were not started.
- Next executable milestone: `RUST-008A` (browser integration), then RUST-007C
  (Rust-only transition).

## 37. RUST-008A browser integration — verified

Date: 2026-10-19. This section supersedes §36.6: RUST-008A is now verified and
RUST-007C is the next milestone. Go remains present.

### 37.1 Preflight and scope

- Installed SOP: `7540edc9bc2b3bb5a9c6cfe4f3799021b807e085`; no SOP process was
  active. RUST-007E was `LOCAL_DONE`/`VERIFIED` and RUST-008A was `PLANNED` with
  its dependency satisfied.
- Branch: `rust-008a-browser-integration`, based on the verified RUST-007E
  commit `60fbad2`. No uncommitted work was discarded.
- The implementation stayed within RUST-008A: no Go removal, no crate move, no
  RUST-007C or RUST-008 execution.

### 37.2 Shared services and API

- Extracted `rust/src/query_service.rs` so `bin/query` and the Axum API use the
  same retrieval, generation, and citation-validation path. `Answer` now carries
  the cited `ContextPassage` list for structured API sources; the CLI output is
  unchanged.
- Added `rust/src/api.rs` and `rust/src/bin/api.rs`: `GET /api/health`,
  `POST /api/documents/ingest`, `GET /api/documents/{source}/status`, and
  `POST /api/query`. Ingestion calls `IngestService`; query calls `QueryService`.
  Synchronous database/Ollama work runs on `spawn_blocking`.
- Upload validation covers extension, 10 MiB size, empty files, missing names,
  unexpected fields, and one file per request. Incomplete PDFs return `422` with
  `incomplete_pages` and write nothing.
- Added `PgStore::source_status` for ordered chunk/section/page provenance.
- The API binds `127.0.0.1:8080` by default; `API_URL`, the database DSN, and
  model credentials stay server-side.

### 37.3 Next.js and browser E2E

- Replaced the mock-only `web/` page with real same-origin `/api/*` calls through
  the existing Next.js server-side rewrite. Removed the mock module and banner;
  the UI now shows real health, ingestion status, cited answers, page references,
  and the loading, empty, connection-failure, extraction-error, and
  insufficient-evidence states.
- Added `web/e2e/rag.spec.ts`, `web/playwright.config.ts`, the Playwright dev
  dependency, and `scripts/web-e2e.sh`. The runner starts Axum + Next.js with a
  unique corpus and deletes only that corpus.
- `make web-e2e` → **5 passed** (1.2m, then 18s on a warm model). The real browser
  verified Markdown upload → cited answer, PDF upload → `Page 2 - p.2`, loading,
  connection failure, incomplete-PDF extraction, and insufficient evidence. Every
  `/api/*` request was same-origin `127.0.0.1:3000`; no request reached the Axum
  port. Screenshots are under
  `docs/reports/rust-migration/web-e2e/`; the summary is
  `docs/reference/rust-008a-browser-evidence.md`.

### 37.4 Deterministic verification

- `cargo test --locked --manifest-path rust/acceptance/api_contract/Cargo.toml`
  → **PASS** (1 test): health, validation, incomplete-PDF no-write, Markdown/PDF/
  plain-text status, duplicate no-op, cited query whose sources resolve, empty
  question, and empty-corpus refusal. The test starts the real API binary with a
  deterministic Ollama server and the dedicated database.
- `sop validate --cache` → all eight Go/Rust build/test/lint commands **PASS**.
- Re-ran the affected acceptance crates: `k3_parity`, `ingest_md_text`,
  `live_rag`, `eval_parity`, `evidence`, and `clean_checkout_go` → all pass.
- `make web-typecheck` and `make web-build` → pass. Go build/test/vet remain
  green.

### 37.5 SOP verdict and operator acceptance

- Verify-first `sop run … --max-tasks 1` → `PASSED`; outcome `PARTIAL` with
  `NEEDS_HUMAN` because criteria 2 and 3 are manual browser evidence. The run did
  not fabricate acceptance.
- The operator recorded the evidence-bound acceptance decisions with
  `sop outcome RUST-008A --accept criterion-2 … --by operator` and the same for
  `criterion-3`, then approved the human gate with `sop approve RUST-008A`.
- A resumed verify-first run → `PASS`; `sop verify RUST-008A` → **`VERIFIED`**,
  validation passed, **`LOCAL_DONE`**. All five criteria are met. The run used
  0 agent calls and 1 validation run.

### 37.6 Intentional contract differences

- Incomplete PDFs return `422` with `incomplete_pages` and write nothing (the
  UI-001 draft proposed `200` with readable chunks); the verified pipeline
  rejects them pre-effect.
- Added `GET /api/documents/{source}/status`.
- `top_k` is accepted and validated but the server's `TOP_K`/`FINAL_K` govern
  retrieval, matching the CLI.

### 37.7 State and next milestone

- `sop status`: RUST-008A `LOCAL_DONE`; RUST-007C `PLANNED`; RUST-008 `PLANNED`.
- No `state.db` edit, budget reset, approval-history edit, merge, or auto-merge.
- Next executable milestone: **RUST-007C** (Rust-only transition and final
  verification). It was not started; Go remains present.

## 38. RUST-007C Rust-only transition — verified

Date: 2026-10-19. This section supersedes §37.7: RUST-007C is verified, the Go
implementation is gone, and RUST-008 is the only remaining plan task.

### 38.1 Preflight

- Installed SOP: `7540edc9bc2b3bb5a9c6cfe4f3799021b807e085`; no SOP process was
  active. RUST-007B, RUST-007E, and RUST-008A were `LOCAL_DONE`/`VERIFIED`;
  RUST-007C was `PLANNED` with all dependencies satisfied.
- Branch: `rust-007c-rust-only`, based on the verified RUST-008A commit
  `7c3c3e1623622f546f0a886ac5078ecbbeb600b8`.

### 38.2 Final layout and Go removal

- Moved the Cargo package to the repository root: `Cargo.toml`, `Cargo.lock`,
  `rust-toolchain.toml`, `src/`, `tests/`, `fixtures/`, and `acceptance/`.
  Updated acceptance `repo_root()` calculations, binary paths, fixture paths,
  and test working directories.
- Removed `cmd/`, `internal/`, `go.mod`, `go.sum`, `tools/` (including the Go
  fixture generator, interop helper, and K=3 rank dump), the Go sweep scripts,
  the Go interop test directions, and `acceptance/clean_checkout_go`.
- The Rust package provides every command (`ingest`, `query`, `eval`,
  `expbilingual`, `api`), the shared services, the Axum API, and the Next.js UI.
- No `.go` file remains. `migrations/`, `evals/`, `examples/`,
  `docker-compose.yml`, `fixtures/`, and the historical evidence under
  `docs/reports/rust-migration/` are preserved.

### 38.3 Frozen Go reference outputs

- Before removing Go, captured the pinned reference outputs into
  `fixtures/parity/frozen/`: K=3 and K=4 `cmd/eval` stdout, the K=3 ranking dump,
  the bilingual `ARM=all` and `ARM=B,C,E` stdout, and both query caches.
- Rewrote `acceptance/eval_parity`, `acceptance/k3_parity`, and
  `acceptance/bilingual_parity` to ingest with the production Rust `ingest` and
  compare the Rust commands against those frozen outputs. No expected value is
  generated from Rust. The bilingual footer normalizes the dynamic `wall=` and
  `cache=` tokens.
- Added `acceptance/final_layout` (root layout, no Go files/tooling, preserved
  provenance/assets, Rust-only Makefile/hooks/CI) and
  `acceptance/clean_checkout` (copy the tree, remove Go from `PATH`, run
  `cargo build --locked`, `cargo test --lib`, and the section/chunk parity test).

### 38.4 Tooling and documentation

- Makefile is Rust-only: `cargo build/test/fmt/clippy`, the acceptance targets,
  `web-*`, and the explicit `web-e2e` target. The Go `integration`/`sweep`
  targets and scripts are gone.
- Hooks run Rust fmt, evidence, final-layout, web typecheck, build/test/clippy,
  clean checkout, and the web build; no Go commands.
- CI has `rust` (pgvector service, fmt/build/test/clippy, final layout/clean
  checkout), `api-contract`, and `web` jobs; the Go job is gone.
- Updated README, command reference, usage, architecture, project structure,
  capability/configuration/testing references, platform support, editor
  verification, and the transition evidence for the root Rust-only layout.

### 38.5 SOP verification and command-policy repair

- Two verify-first attempts stopped at setup blockers because the installed
  command policy rejected `cargo clippy … -- -D warnings` and
  `cargo fmt --manifest-path Cargo.toml --check` on the acceptance path. The
  acceptance checks were changed to the authorizable root forms
  `cargo clippy --locked --all-targets` and `cargo fmt --check`. The configured
  validation still runs Clippy with `-D warnings` and the fmt check, so no
  criterion was weakened.
- The verify-first run then passed: `sop verify RUST-007C` → **`VERIFIED`**,
  validation passed, **`LOCAL_DONE`**; all six criteria met; 0 agent calls, one
  validation run, 0 fix cycles.

### 38.6 Verification results

- Root: `cargo fmt --check`, `cargo build --locked`, `cargo test --locked`
  (216 library tests plus integration/doc tests), and
  `cargo clippy --locked --all-targets -- -D warnings` all pass.
- Acceptance: `final_layout` (5), `clean_checkout` (1), `evidence` (5),
  `api_contract` (1), `k3_parity` (1), `eval_parity` (1), `ingest_md_text` (1),
  `bilingual_parity` (1), `e2e` (2), `live_rag` (2), `ingest_cli` (2), and
  `retrieval_cli` (9) pass. The live checks use real Ollama and PostgreSQL.
- Web: `make web-typecheck`, `make web-build`, and `make web-e2e` pass (5
  browser tests). The Axum API and CLI binaries build from the root package.

### 38.7 State and next milestone

- `sop status`: RUST-007C `LOCAL_DONE`; RUST-008 `PLANNED`.
- No `state.db` edit, budget reset, approval-history edit, merge, or auto-merge.
- Next executable milestone: **RUST-008** (final Rust-only browser acceptance).
  RUST-007C is complete; Go no longer exists in the repository.
