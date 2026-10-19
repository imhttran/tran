# Evidence: full `internal/cli` suite stalls (pre-existing)

Preserved evidence for the unresolved full-suite stall referenced by
[`sop-development-process.md`](sop-development-process.md#107-unresolved-full-internalcli-suite-stalls).
This is raw evidence; the cause is **not** established. It does not concern the
Cargo acceptance-check patch (see "Attribution" below). SQLite diagnosis is kept
separate from that patch.

## Commands and results

| # | Command | Result |
| - | ------- | ------ |
| 1 | `go test -count=1 -tags acceptance -timeout 8m ./internal/cli/` (worktree) | `panic: test timed out after 8m0s` → `FAIL … 480.253s` |
| 2 | `go test -count=1 -tags acceptance -timeout 8m ./internal/cli/` (clean `main`, base `a46187b`) | `panic: test timed out after 8m0s` → `FAIL … 480.269s` |
| 3 | same, `ulimit -n 65535` (already 1048576) | `FAIL … 360.448s` |
| 4 | same, `-parallel 1 -p 1` | `FAIL … 360.238s` |

The blocking test varies between runs (`TestRunTasksMalformedPlan`, then
`TestMemoryAddAndList`) — the invariant is the frame, not the test.

## Stack (identical in worktree and `main`)

```
panic: test timed out after 8m0s
…
modernc.org/sqlite.(*conn).ExecContext(...)
database/sql.(*DB).ExecContext.func1(...)
database/sql.(*DB).ExecContext(...)
github.com/imhttran/agentic-sop/internal/store.(*Store).createSchema(...)
	internal/store/sqlite.go:125
github.com/imhttran/agentic-sop/internal/store.(*Store).migrate(...)
	internal/store/sqlite.go:79
github.com/imhttran/agentic-sop/internal/store.Open(...)
	internal/store/sqlite.go:40
github.com/imhttran/agentic-sop/internal/cli.ensureProjectInitialized(...)
	internal/cli/init.go:52
…
goroutine [select]:
database/sql.(*DB).connectionOpener(...)
```

## Ruled out

- **Not file descriptors:** `ulimit -n` = 1048576; raising it changes nothing.
- **Not test parallelism:** `-parallel 1 -p 1` stalls the same way.
- **Not a specific test:** the test that blocks varies run to run; the blocking
  frame does not.
- **Not the acceptance tag / this patch:** it reproduces on the clean `main`
  worktree with no enhancement changes.

## Attribution

`internal/store` and `internal/store/sqlite.go` were last changed before the
enhancement base and are **unchanged** in this patch; upstream `origin/main` does
not touch `internal/store` either. Individual `cli` tests that call `store.Open`
pass in isolation (e.g. `TestRunTasksMalformedPlan` 1.4s here, 0.55s on `main`).
So the stall is triggered by the full suite's cumulative execution of many
`store.Open` calls in one process, not by a specific change. **Cause unresolved.**

## Reproduction guidance

Do not repeat command 1–4 to re-establish this; the four results above are the
evidence. Re-run only if an upstream `internal/store`/`modernc.org/sqlite` change
or a specific hypothesis justifies it.

## Integrated run (origin/main `a9f0495`)

- Command: `go test -count=1 -tags acceptance -timeout 8m ./internal/cli/` →
  `FAIL … 480.430s` (isolated worktree `agentic-sop-cargo-integration`).
- Deeper stack than the earlier runs: the goroutine is blocked in the **`fsync`
  syscall** — `modernc.org/libc.Xfsync` → `sqlite._full_fsync` → `_unixSync` →
  `_sqlite3OsSync` → `_syncJournal` → `_pagerCommitPhaseOne` → … →
  `store.(*Store).createSchema` → `store.Open`.
- Refined hypothesis: a **blocking `fsync`** (an I/O stall on this filesystem), not
  a lock/deadlock. `internal/store` is unchanged upstream (`a46187b..a9f0495`).
  Not re-run.
