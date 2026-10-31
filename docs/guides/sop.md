# Running SOP on tran

What it takes to get clean, reasonably fast SOP runs in this repository, and
what to check when a run stalls or crawls. Most of this was learned the hard
way on 2026-10-31.

## Configuration

`.agent-sdlc/config.yaml` is tracked. SOP rewrites `.agent-sdlc/.gitignore` to
`*` on every run, but Git keeps tracking a file once it's added, so edits to the
config show up in `git status` like any other file. Everything else in
`.agent-sdlc/` (state, runs, memory, archives) stays ignored.

The config holds policy only. Secrets live in `.env`. The one machine-specific
part is the pair of `advisory_retrieval` paths, which SOP requires to be
absolute. On any machine other than the Mac they were written for, set these in
that machine's `.env`.

```sh
SOP_ADVISORY_RETRIEVAL_INDEX=/absolute/path/to/tran/.agent-sdlc/memory/index.db
SOP_ADVISORY_RETRIEVAL_OUTCOMES=/absolute/path/to/tran/.agent-sdlc/memory/outcomes
```

Running `sop init --refresh-validation` replaces the whole `validation` block
with SOP's guess. Don't, or restore the block from Git afterward.

## Validation

SOP always runs the `build` list, then `test`, then `lint`, and stops at the
first failure. The fast checks (`cargo fmt --check`, `cargo build`, Clippy) sit
under `build` on purpose, so a format or lint error fails in the first minute
instead of after the database suite.

The test step goes through `./scripts/test-db.sh exec`, the same wrapper as
`make test`. It points the suite at the disposable `*_test` databases derived
from `.env` and refuses anything else. A plain `cargo test` would pick up the
real `tran_genealogy` URL from `.env`, and the integration tests would stop with
"refusing to write to non-disposable genealogy database".

Postgres has to be running before a run reaches the test step.

```sh
make db-up        # Postgres, on 5435 on the Mac
make test-db-up   # create and migrate the *_test databases
```

A "Connection refused" in the test output means Postgres is down, not that the
code is wrong. Stop the run before the fix cycle starts editing files to work
around it.

## Security scan

The scanner is on when `.env` has `SOP_SECURITY_ENABLED=true`, and
`SOP_SECURITY_MODE=report-only` keeps findings advisory. Results land in
`.agent-sdlc/runs/<task>/security/scan.json`.

The `security.exclude` list in the config is required. Before Semgrep starts,
SOP walks the whole workspace in-process to detect languages, and the walk
ignores `.gitignore` by design (scope changes have to be explicit and recorded).
Without the list it crawls `target/`, `.worktrees/` and `.claude/`, hundreds of
thousands of files, and the run looks hung after validation with `sop` busy on
the CPU and no scanner process. With the list the scan takes under a minute. If
you add another large ignored folder, add it to the list too.

Gitleaks reports 9 findings today, three in the local `.env` and six in test
fixtures. They're advisory in report-only mode. Review them before ever
switching to `required`.

## Slow tests on macOS

Any task that changes Rust code relinks about 68 test binaries, and macOS checks
each new binary the first time it runs. Without the fix that check cost about 9
minutes per run. The time doesn't show up in the tests' own timings, it sits
between test binaries.

| Full test run (`./scripts/test-db.sh exec -- cargo test --locked`) | Time    |
| ------------------------------------------------------------------ | ------- |
| Right after a rebuild, terminal not exempted                       | 11m–14m |
| Right after a rebuild, terminal exempted                           | 3m03s   |
| Nothing rebuilt                                                    | 1m30s   |

The fix is System Settings → Privacy & Security → Developer Tools. Turn on every
app you start SOP or cargo from (iTerm, Zed), then quit it with ⌘Q and reopen
it. The restart matters. A run from an iTerm session opened before the change
still took 13m51s. A `tmux` server started before the change keeps the old
behavior too, so restart it as well. If the Developer Tools section is missing,
run `sudo spctl developer-mode enable-terminal` once.

The background planner (`com.tomtran.sop-mac-tunnel`) is started by `launchd`,
not a terminal. `tunnel-client`, `sop-planner` and `sop` are on the list too,
but it isn't confirmed yet that macOS honors the exemption for programs started
that way. Check the test step's duration in a planner-started run's
`validation.json`. Around 3 minutes means it works, 11 or more means it doesn't,
and then the fallback is running `tunnel-client` inside `tmux` in iTerm instead
of as a LaunchAgent. A rebuilt `sop` or `sop-planner` may also count as a new
program and lose its exemption, so check the list after rebuilding them.

To check the terminal side, time one freshly built test binary, then a full run.

```sh
touch src/lib.rs
cargo test --locked --no-run 2>&1 | grep -m1 'tests/admin_members.rs'
time target/debug/deps/admin_members-<hash> --list > /dev/null   # milliseconds when exempted

touch src/lib.rs
time ./scripts/test-db.sh exec -- cargo test --locked 2>&1 | tail -3   # about 3 minutes
```

Linux doesn't have this check.

## Another machine

Pull the branch that tracks the config. If the machine already has its own
untracked `.agent-sdlc/config.yaml`, move it aside first or the pull will refuse
to overwrite it. Then set the two `advisory_retrieval` paths in its `.env`,
install `semgrep` and `gitleaks`, and run `make test-db-up`. On Linux,
`test-db.sh` tries port 5432 before 5434 and 5435, so a native Postgres wins.
`./scripts/test-db.sh status` shows which server it picked.

## When a run stalls

"a different plan is already active" means SOP keeps one plan at a time. Finish
or set aside the active one. Moving `state.db`, `plan.json` and `plan.meta.json`
from `.agent-sdlc/` into `.agent-sdlc/archive/<plan-id>/` shelves it, and moving
them back restores it.

`IMPLEMENT_NO_PROGRESS` right after a failed attempt usually means the earlier
attempt's edits are still in the tree. The agent sees the work done, changes
nothing, and SOP never reaches validation. Stash them
(`git stash push -u -- <paths>`) and run again.

A declined approval is final. `sop approval supersede` only works on pending
ones, and `sop retry` won't clear it. Moving `.agent-sdlc/runs/<task>/` into the
archive removes the approval record so the task can run again.

`retry budget exhausted (3/3)` after attempts that failed for environment
reasons is safe to reset with `sop retry <task> --force`.

When the agent can't fix a failure it fully understands (it keeps trying to run
`cargo`, which is denied, and stops on no progress), fix it by hand, commit, and
record it with `sop task complete <task> --external`. That command runs the full
validation itself and refuses if anything fails.
