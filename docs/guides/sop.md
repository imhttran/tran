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
to overwrite it. Then set the two `advisory_retrieval` paths in its `.env` and
install `semgrep` and `gitleaks`.

`test-db.sh` uses the database URLs from `.env`. Only when they're unset does it
probe for a server (5432 first on Linux, 5434 first on macOS).
`./scripts/test-db.sh status` shows which server it picked.

### Linux box

Set up on 2026-10-31. Two things made SOP slow or broken there.

The first was the database. The box runs its own Postgres on 5432, but `.env`
pointed at 5434, the Docker port used on the Mac, so every test step failed with
"Connection refused" and SOP burned fix cycles on code that wasn't broken. Both
URLs now point at 5432 and carry the password. The Rust tests use the `postgres`
crate, which doesn't read `~/.pgpass`, so the password has to be in the URL,
URL-encoded.

```sh
DATABASE_URL=postgres://rag:<encoded-password>@127.0.0.1:5432/tran_rag_test?sslmode=disable
GENEALOGY_DATABASE_URL=postgres://rag:<encoded-password>@127.0.0.1:5432/tran_genealogy_test?sslmode=disable
```

The Docker Postgres makes `rag` a superuser, so the Mac never needed grants. A
shared server does. Run these once as `postgres`. If `make test-db-up` stops on
`CREATE EXTENSION vector`, create the extension and run it again.

```sh
sudo -u postgres createuser --login --createdb --pwprompt rag   # or: ALTER ROLE rag CREATEDB
sudo -u postgres psql -d tran_rag_test -c 'CREATE EXTENSION IF NOT EXISTS vector'
sudo -u postgres psql -c 'GRANT SET ON PARAMETER session_replication_role TO rag'   # Postgres 15+
make test-db-up
```

The last grant is for two tests in `onboarding_profile_revisions` that switch
triggers off for their own session. Don't make `rag` a superuser instead.

The second was the disk. `/home` sits on a 2.5" spinning drive, and building the
test binaries there kept 12 cores at 86% CPU, mostly waiting. Moving Cargo's
output to an SSD fixed it.

| Build of the test binaries after a change | Time  |
| ----------------------------------------- | ----- |
| Linux, hard drive                         | 6m29s |
| Linux, SSD                                | 46s   |
| Mac                                       | 53s   |

The SSD is an external Samsung on a 10 Gbps USB port (`uas` driver), split into
`Builds` (ext4, mounted at `/srv/builds`) and `SharedData` (exFAT, shared with
the Mac). Keep Cargo output off exFAT, it has no Linux permissions and is slow
with many small files. Set the target folder in the shell and in `tran/.env`,
since SOP loads `.env` into every command.

```sh
export CARGO_TARGET_DIR=/srv/builds/cargo-target/tran   # in ~/.zshrc
CARGO_TARGET_DIR=/srv/builds/cargo-target/tran          # in tran/.env
```

Because the disk is external, `fstab` mounts it with `nofail` so the box still
boots without it, and the empty mount point is locked with
`sudo chattr +i /srv/builds` (run while unmounted). If the disk is missing,
Cargo fails right away instead of writing 40 GB to the main disk.

`scripts/local-app.sh` and `scripts/web-e2e.sh` follow `CARGO_TARGET_DIR`.
Anything else that runs `target/debug/...` directly would pick up a stale
binary, so check for that when adding scripts.

The test run itself still waits a lot (25% CPU). The likely cause is Postgres
keeping its data on the hard drive, which hasn't been checked yet.

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
