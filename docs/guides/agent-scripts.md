# Git hooks and agent scripts

## Git hooks

`.githooks/pre-commit` loads `.env` when present, validates every tracked JSON
file with `jq`, then runs the Rust and web checks through the Makefile:

- `make rust-fmt` (`cargo fmt --check`)
- `make rust-acceptance-evidence`
- `make rust-acceptance-final-layout`
- `make web-check-env`
- `make web-typecheck` (`tsc --noEmit`)

`.githooks/pre-push` runs the full gates:

- `make rust-build rust-test rust-clippy`
- `make transition-evidence` (evidence, final layout, and the clean checkout
  with Go absent from `PATH`)
- `make web-check-env`
- `make web-build` (the Next.js production build)

The Makefile targets are the single source of truth; the hooks do not duplicate
the commands.
`.agents/scripts/review-agent.sh` reviews the diff against `origin/main` and is
advisory — a finding never fails anything. It is not wired into the hook either,
so run it by hand when you want a second pair of eyes: it is read-only
(`--tools ""`), budget-capped (`REVIEW_BUDGET`, default `0.25` USD), and skips
itself unless at least `REVIEW_MIN_LINES` lines of what `REVIEW_PATHS` matches
changed.

`.agents/scripts/audit-agent.sh` hunts over-engineering — dependencies the
standard library already ships, single-implementation interfaces, dead flags — and
writes its report to `AUDIT.md` at the root of the repository (gitignored). It is
not wired into the hook, so it never delays a push: run it by hand. It gets
read-only tools (`AUDIT_TOOLS`, default `Read,Grep,Glob`), so it can walk the tree
but cannot change it, and it is budget-capped (`AUDIT_BUDGET`, default `0.50` USD).
It skips itself unless at least `AUDIT_MIN_LINES` lines changed against
`AUDIT_BASE`.

The audit's rulebook is a skill file, `AUDIT_SKILL`, so one file drives the
script and an editor session that invokes the skill. It defaults to the global
ponytail-audit skill at `~/.agents/skills/ponytail-audit/SKILL.md`; point it
somewhere else to audit by different rules. Run it whenever you want a report:

```sh
AUDIT_MIN_LINES=0 sh .agents/scripts/audit-agent.sh
```

The scripts read their settings from environment variables, so another repository
can reuse them by copying the folder somewhere shared and pointing a hook (or a
manual run) at them:

```sh
AUDIT_AGENT=/path/to/shared/audit-agent.sh
REVIEW_AGENT=/path/to/shared/review-agent.sh
```

Set `REVIEW_MIN_LINES=0` to review any change, or `REVIEW_CMD=false` to switch the
review off; `AUDIT_MIN_LINES=0` and `AUDIT_CMD=false` do the same for the audit.
Each script documents the rest.

Git does not pick up `.githooks/` on its own. Enable it once per clone:

```bash
git config core.hooksPath .githooks
```

Bypass the commit hook with `git commit --no-verify`, or the push hook with
`git push --no-verify`.

## Installing the hooks

The hooks are version-controlled under `.githooks/`. Enable them once per clone:

```sh
git config core.hooksPath .githooks
```

Bypass a single operation with `git commit --no-verify` / `git push --no-verify`.
`core.hooksPath` is repository-local configuration, so each clone must run the
command above. Verify with `git config --get core.hooksPath` (expect `.githooks`).

## Rust and web checks

Both hooks delegate to the Makefile so the commands never drift:

- `pre-commit` runs `make rust-fmt`, `make rust-acceptance-evidence`,
  `make rust-acceptance-final-layout`, `make web-check-env`, and
  `make web-typecheck`.
- `pre-push` runs `make rust-build rust-test rust-clippy`,
  `make transition-evidence` (including the clean checkout without Go), and
  `make web-build`.

The Cargo package is at the repository root; no separate `rust/` path is used.
