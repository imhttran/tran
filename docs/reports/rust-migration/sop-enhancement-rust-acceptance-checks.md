# SOP enhancement — native Rust acceptance checks

A small, scoped proposal to let `sop run` prove Rust criteria with deterministic
`cargo` acceptance checks. This is a proposal document only; it changes nothing
in SOP. It complements the blocker record in
[`sop-development-process.md`](sop-development-process.md#6-blocker-native-rust-acceptance-checks-tool-capability-gap).

## Problem

On the installed build (`sop version` = `sop dev`, source HEAD `a46187b`,
binary mtime 2026-10-17 09:27), an acceptance check must pass
`commandpolicy.Classify(argv) == SAFE`. Only `go` (test/build/vet/fmt/list) and
read-only `git` are SAFE; every other program, including `cargo`, is
`REQUIRES_APPROVAL`. `authorizeAcceptanceCommand`
(`internal/cli/outcome.go`) then records the check as an unauthorized **boundary**,
never runs it, and the outcome becomes `HOLD` — which per
`internal/outcome/outcome.go` (`ValidateWaiver`, `ResolveLifecycle`) is **not
waivable** and maps to `NEEDS_HUMAN`. So a Rust project cannot get a *passing*
acceptance check, and its stages can only ever be `PARTIAL` (or `HOLD`).

## Goal

Allow a plan to declare, and SOP to execute, a **narrow, opt-in set of cargo
verification commands** as acceptance checks, without weakening the existing
approval boundaries or the project policy model.

## Design

### 1. Structured command + working directory

- Keep the existing structured model: `acceptance_checks` entries are tokenized
  by `toolharness.SplitCommand` into an argv vector and executed **without a
  shell** (no chaining, pipes, redirection, substitution, globs).
- Add an optional per-stage/check **working directory**, resolved relative to the
  repository root and confined to it (reject escapes, absolute paths outside the
  repo, and `..` traversals). The stage schema (`Stage.AcceptanceChecks`,
  `internal/domain`) gains a `working_directory` (or the check becomes a small
  structured `{dir, argv}` object). This removes the need for `cd`/`--manifest-path`
  workarounds, which the current shell-syntax rejection forbids.

### 2. Scoped cargo support

Classify **only** these shapes as SAFE, and only when the operator opts in:

- `cargo build …`
- `cargo test …`
- `cargo clippy …`
- `cargo fmt --check …` (and `cargo fmt … --check`; reject plain `cargo fmt`,
  which rewrites files — mirroring the existing `go fmt` rejection)

Everything else stays `REQUIRES_APPROVAL` or is rejected: `cargo run`,
`cargo install`, `cargo publish`, `cargo add/remove`, `cargo fix`, plain
`cargo fmt`, and any `--config`/`-Z`/tool-injection flag form (extend
`rejectDangerousForm` with the cargo analogues of the go flag set).

### 3. Preserve approval boundaries and project policy

- Permitting `cargo` is a **trust escalation** (see §5), so it is **opt-in and
  OFF by default**: a new config switch (e.g. `validation.acceptance_checks.cargo:
  true`) enables it. With the switch off, behavior is byte-identical to today.
- The project `Policy` (`Approve`/`Deny` prefixes) continues to apply and can only
  **tighten** — an operator can still force a given cargo command to
  `REQUIRES_APPROVAL` or `DENIED`.
- A cargo check that fails policy authorization keeps today's semantics: recorded
  as a boundary, `HOLD`, `NEEDS_HUMAN`, not waivable.

### 4. Tests

- **Allowed:** `cargo build`, `cargo test`, `cargo clippy --all-targets`,
  `cargo fmt --check`, each with an explicit working directory; assert SAFE and
  that argv+cwd are preserved into execution.
- **Rejected:** `cargo run/install/publish/add/fix`, plain `cargo fmt`, cargo flag
  injection forms, arbitrary binaries (`python`, `sh -c …`), and shell-syntax
  strings (`cd rust && cargo test`, pipes, `$(…)`) — assert boundary/HOLD.
- **Path handling:** working directory resolves under the repo root; `..` escapes,
  absolute out-of-repo paths, and symlink escapes are rejected; `--manifest-path`
  stays usable; a missing directory is a clear error, not a silent cwd.
- **Regression:** go/git classification and existing check-less plans stay
  unchanged (byte-identical plan.json / behavior).

### 5. Documentation

- Update `docs/specs/SECURITY.md` (command policy) and
  `docs/plans/ACCEPTANCE-CHECKS-AUTHORING.md` to state that **acceptance checks
  can execute project code**: `cargo build` runs build scripts and proc-macros,
  and `cargo test` runs the test harness. This is why cargo support is scoped and
  opt-in, and why it must never be treated as read-only the way `go vet`/`go test`
  arguably are.
- Document the working-directory field and the cargo allowlist with examples.

## Scope / non-goals

- No change to the go/git classification, the planner's "a model may not author
  acceptance checks" rule, or the no-shell execution model.
- Not a general "allow any command" escape hatch; explicitly not `cargo run`.
- Not adding network-tooling commands; cargo still needs its normal crates.io
  access at run time (operator concern, documented).

## Risks

| Risk | Mitigation |
| ---- | ---------- |
| Running project code at verification time | Opt-in, scoped subcommands, documented trust model |
| Policy bypass via flag injection | Extend `rejectDangerousForm` with cargo flag forms |
| Path escape via working directory | Confine resolution to the repo root; reject `..`/absolute escapes |
| Silent behavior change for existing plans | Default-off switch; keep go/git path byte-identical |

## Suggested validation for this enhancement

- Unit tests in `internal/commandpolicy` (classification) and `internal/cli`
  (authorization/HOLD) per §4.
- An end-to-end plan fixture with a cargo acceptance check in a temporary Rust
  crate, asserting a passing check yields `VERIFIED` and a failing one yields
  `PARTIAL`.
