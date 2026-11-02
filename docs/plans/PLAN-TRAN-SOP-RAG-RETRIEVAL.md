# Implementation Plan

## Project

Turn on SOP's RAG knowledge layer for tran, so every SOP task gets a few cited
passages from tran's own documentation in its context. The retrieval service is
`cmd/retrieval` from `projects/rag-template-rust`, which speaks SOP's
`rag-retrieval/v1` contract. This plan adds the tran-side glue (a wrapper, an
ingest script, a database port override, a live check, the `rag:` config block)
and a short guide. It does not change SOP, rag-template-rust, or tran's
application code.

## Summary

### What is already true (checked 2026-10-30)

- **SOP side is built.** `internal/ragclient` runs one configured executable
  with no arguments, sends the request JSON on stdin and reads the response on
  stdout. The child inherits SOP's environment. When `rag.enabled` is true,
  every task without `required_evidence` gets up to `top_k` items within
  `context_budget_bytes`; a retrieval failure is ignored and the task runs
  without them. SOP rejects the config unless `executable` is an absolute path,
  `corpus` is set, and `corpus` is in `allowed_corpora`.
- **Producer side is built.** `rag-template-rust/cmd/retrieval` reads one
  request (`schema_version`, `query`, `top_k`, `filters.corpus`,
  `deadline_ms`), searches Postgres with pgvector, and returns `status` `OK` or
  `EMPTY` with ranked, cited `evidence`. It allows only the corpora in
  `RAG_RETRIEVAL_ALLOWED_CORPORA` and answers `UNAUTHORIZED_SCOPE` otherwise. It
  reads `DATABASE_URL`, `OLLAMA_URL`, `OLLAMA_EMBED_MODEL`, `EMBED_DIM` and
  `EMBED_PROVIDER` from the environment (defaults: port 5434,
  `nomic-embed-text`, 768).
- **Ingest exists.** `rag-template-rust/cmd/ingest FILE` chunks and embeds one
  file, tags it with the corpus in `RAG_CORPUS`, and records its path relative
  to the repository root. Re-running it on the same file is safe.

### Two conflicts this plan resolves

1. **5434 and 5435 are taken.** rag-template's own dev database
   (`rag-template-postgres-1`) publishes `5434`, and tran's running database
   (`tran-postgres-1`, which tran's `.env` uses) publishes `5435`. The SOP
   retrieval database runs as a separate Compose project on **5436**.
2. **tran's environment would leak in.** tran is itself a RAG app and uses the
   same variable names (`DATABASE_URL`, `OLLAMA_*`, `EMBED_*`). Because the
   retrieval binary inherits SOP's environment, the wrapper must set every one
   of them explicitly, never fall back to what is already set.

### Decisions

- Corpus name: `tran-docs`. It holds `README.md` and the Markdown files in
  `docs/architecture/`, `docs/guides/` and `docs/reference/`. Plans, reports,
  history and images stay out: they are long, point-in-time, or not text.
- Settings: `top_k: 5`, `context_budget_bytes: 4096`, `timeout_ms: 5000` (the
  first request after Ollama loads the embedding model can be slow).
- Known limit: ingest adds and updates documents but never removes one. A
  deleted or renamed doc stays retrievable until the database is reset. The
  guide documents the reset.

### Operator steps (run by Tom, not by SOP)

SOP cannot build outside tran, start Docker, pull models, or edit its own
untracked `.agent-sdlc/config.yaml` in a way that reaches `main`. After RAG-101
to RAG-104 are merged to `main`:

```sh
# 1. Build the producer and the ingester from rag-template-rust main.
cd ~/agentic-workspace/projects/rag-template-rust
git switch main && git pull
go build -o bin/retrieval ./cmd/retrieval
go build -o bin/ingest ./cmd/ingest

# 2. Start the retrieval database on 5436 as its own Compose project.
docker compose -p sop-rag -f docker-compose.yml \
  -f ~/agentic-workspace/projects/tran/scripts/sop-rag/compose.port.yml up -d --wait

# 3. Make sure Ollama has the embedding model.
ollama pull nomic-embed-text

# 4. Ingest the corpus.
~/agentic-workspace/projects/tran/scripts/sop-rag/ingest.sh

# 5. Turn the layer on: append the RAG-103 block to the primary checkout's
#    SOP config (.agent-sdlc/ is gitignored, so no task can commit this).
cd ~/agentic-workspace/projects/tran
cat scripts/sop-rag/rag.config.yaml >> .agent-sdlc/config.yaml

# 6. Prove it live, then record RAG-104's live check as done.
./scripts/sop-rag/check-live.sh
```

SOP's acceptance policy runs only `git`, `go` and opted-in `cargo` commands, so
the shell-script tests run through one cargo integration test,
`tests/sop_rag_scripts.rs`, and `check-live.sh` is run by the operator.

### How execution works in this repository

SOP's command policy does not let the agent build, test, run Docker, or query a
database. The agent writes the files; the validation stage and the acceptance
checks run the commands.

- Read only the files a task names.
- Write the files and the tests; do not try to run them.
- In the closing message, state what changed and what the checks must prove.

### Boundaries that apply to every task

- **tran only.** Change files inside this repository only. Do not edit
  agentic-sop or rag-template-rust.
- **No application changes.** Nothing under `src/`, `web/` or `migrations/`
  changes, the only file under `tests/` is the new `tests/sop_rag_scripts.rs`, and tran's own database and `docker-compose.yml` are left
  alone.
- **Clean stdout.** Anything SOP executes as the producer writes only the
  producer's JSON to stdout. Diagnostics go to stderr.
- **Explicit environment.** Every script sets the retrieval variables itself
  from `scripts/sop-rag/env.sh`. None of them reads tran's `.env`.
- **POSIX shell.** Scripts use `#!/bin/sh`, `set -eu`, and work on macOS and
  Linux, like the existing `scripts/*.sh`.
- **No secrets.** The database credentials are rag-template's local defaults
  (`rag`/`rag`); nothing else sensitive enters a committed file.
- **Small diffs.** `.agents/` stays out of every diff.

## RAG-101 — Retrieval wrapper and shared settings

Add the executable SOP will run, and the settings file every SOP RAG script
sources.

Files you may add: `scripts/sop-rag/env.sh`, `scripts/sop-rag/retrieval.sh`,
`scripts/sop-rag/test-sop-rag.sh`, `tests/sop_rag_scripts.rs`.

- `env.sh` (sourced, not executed) exports, each overridable by an
  `SOP_RAG_`-prefixed variable:
  - `SOP_RAG_BIN_DIR`, default `$HOME/agentic-workspace/projects/rag-template-rust/bin`
  - `DATABASE_URL` from `SOP_RAG_DATABASE_URL`, default
    `postgres://rag:rag@127.0.0.1:5436/rag?sslmode=disable`
  - `OLLAMA_URL` (default `http://localhost:11434`), `OLLAMA_EMBED_MODEL`
    (`nomic-embed-text`), `EMBED_DIM` (`768`), `EMBED_PROVIDER` (`ollama`)
  - `RAG_CORPUS` and `RAG_RETRIEVAL_ALLOWED_CORPORA`, both `tran-docs`
  Each value is assigned unconditionally from its `SOP_RAG_` variable or the
  default, so an inherited `DATABASE_URL` or `OLLAMA_*` never wins.
- `retrieval.sh` sources `env.sh` and `exec`s `$SOP_RAG_BIN_DIR/retrieval`,
  passing stdin and stdout straight through. If the binary is missing, it writes
  one line to stderr and exits non-zero without writing to stdout.
- `test-sop-rag.sh` tests both scripts with a stub binary in a temporary
  directory (set through `SOP_RAG_BIN_DIR`) that echoes stdin and prints its
  environment to a file. No database or Ollama is needed.
- Known defect to fix in `scripts/sop-rag/test-sop-rag.sh` (scenario C fails
  today; the other files are correct and need no rewrite): the stdin
  round-trip check captures output with `actual=$(...)`, and command
  substitution strips trailing newlines, so it can never equal `$payload`,
  which ends in a newline. Preserve the trailing newline with a sentinel:
  `actual=$(printf '%s' "$payload" | SOP_RAG_BIN_DIR="$bin_dir" sh "$wrapper"; printf x)`
  then `actual=${actual%x}`. Done when `sh scripts/sop-rag/test-sop-rag.sh`
  prints PASS for every scenario.
- `tests/sop_rag_scripts.rs` holds one `#[test]` that runs
  `sh scripts/sop-rag/test-sop-rag.sh` from `env!("CARGO_MANIFEST_DIR")` with
  `std::process::Command` and asserts it exits zero, printing the script's
  stdout and stderr on failure. It uses only the standard library, so it passes
  `cargo fmt --check` and `cargo clippy` with no new dependency.

### Dependencies

None

### Deliverables

- The three scripts above, executable, and `tests/sop_rag_scripts.rs`.

### Acceptance Criteria

- With `DATABASE_URL`, `OLLAMA_URL` and `EMBED_DIM` set to other values in the
  caller's environment, the stub sees the `env.sh` values.
- Setting `SOP_RAG_DATABASE_URL` changes the `DATABASE_URL` the stub sees.
- The stub's stdin reaches it unchanged, and the wrapper adds nothing to stdout.
- With no binary present, the wrapper exits non-zero and stdout is empty.

### Acceptance Checks

- `cargo test --locked`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1
- criterion 4: check 1

## RAG-102 — Database port override and ingest script

Add what the operator steps need to run the retrieval database beside tran's and
to load the corpus.

Files you may add or change: `scripts/sop-rag/compose.port.yml`,
`scripts/sop-rag/corpus.txt`, `scripts/sop-rag/ingest.sh`,
`scripts/sop-rag/test-sop-rag.sh`.

- `compose.port.yml` overrides only the `postgres` service's published port to
  `5436:5432`, using the Compose `!override` tag so 5434 is replaced, not added.
- `corpus.txt` lists the corpus as paths relative to the repository root, one
  per line, with `#` comments: `README.md` and the three docs folders from the
  Summary.
- `ingest.sh` sources `env.sh`, expands `corpus.txt` to the Markdown files that
  exist, and runs `$SOP_RAG_BIN_DIR/ingest FILE` for each one from the
  repository root. `--dry-run` prints the file list and runs nothing. It stops
  at the first failure and prints which file failed.
- Extend `test-sop-rag.sh` to cover `--dry-run` and to run `ingest.sh` against a
  stub `ingest` that records its arguments and `RAG_CORPUS`.

### Dependencies

RAG-101

### Deliverables

- The override, the corpus list, the ingest script and the extended test.

### Acceptance Criteria

- `--dry-run` lists `README.md` and every Markdown file under the three docs
  folders, and nothing from `docs/plans/`, `docs/reports/` or `docs/history/`.
- With the stub, `ingest` runs once per listed file with `RAG_CORPUS=tran-docs`.
- A failing stub stops the script with a non-zero exit naming the file.
- `compose.port.yml` publishes `5436` and does not mention `5434`.

### Acceptance Checks

- `cargo test --locked`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1
- criterion 4: check 1

## RAG-103 — Enable the RAG block

Ship the `rag:` block that turns the layer on. `.agent-sdlc/` is gitignored,
so the block lives in a tracked file and the operator appends it to the primary
checkout's config (operator step 5). Do not edit `.agent-sdlc/config.yaml`.

Files you may add or change: `scripts/sop-rag/rag.config.yaml`,
`scripts/sop-rag/test-sop-rag.sh`.

- `rag.config.yaml` contains exactly this block, with a leading `#` comment
  naming operator step 5:

  ```yaml
  rag:
    enabled: true
    executable: /Users/imhttran/agentic-workspace/projects/tran/scripts/sop-rag/retrieval.sh
    corpus: tran-docs
    allowed_corpora:
      - tran-docs
    timeout_ms: 5000
    top_k: 5
    context_budget_bytes: 4096
  ```

- Extend `test-sop-rag.sh` to check that `rag.config.yaml` has those values and
  that `executable` ends in `scripts/sop-rag/retrieval.sh`, the wrapper from
  RAG-101.

### Dependencies

RAG-101

### Deliverables

- `scripts/sop-rag/rag.config.yaml` and the extended test.

### Acceptance Criteria

- `rag.config.yaml` holds the block above and nothing else but its comment.
- The `executable` path is absolute and names `scripts/sop-rag/retrieval.sh`.
- `corpus` is listed in `allowed_corpora`.

### Acceptance Checks

- `cargo test --locked`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1

## RAG-104 — Live retrieval check

Write the script that proves the real service answers through the wrapper. It
needs Docker, Ollama and the built binaries, which SOP cannot run, so the
operator runs it (operator step 6) after merging and records the result with
`sop task complete RAG-104 --external`. SOP validates this task with the
configured validation only.

Files you may add or change: `scripts/sop-rag/check-live.sh`.

- `check-live.sh` sends two requests through `retrieval.sh`:
  1. `{"schema_version":"rag-retrieval/v1","query":"how does branch review work","top_k":3,"filters":{"corpus":"tran-docs"},"deadline_ms":5000}`
  2. the same request with `"corpus":"not-allowed"`.
- It passes when request 1 returns `status` `OK` with at least one evidence item
  whose `corpus` is `tran-docs` and whose `source` starts with `docs/` or is
  `README.md`, and request 2 returns `status` `UNAUTHORIZED_SCOPE`. It parses
  JSON with `python3`, which the existing scripts already use.
- On failure it prints the status and the producer's `diagnostics.error_code`,
  and reminds the reader of the operator steps when the status is `UNAVAILABLE`.

### Dependencies

- RAG-102
- RAG-103

### Deliverables

- `check-live.sh`, executable.

### Acceptance Criteria

- An allowed query returns cited evidence from `tran-docs`.
- A query for another corpus is refused with `UNAUTHORIZED_SCOPE`.

## RAG-105 — Guide

Document how the SOP RAG setup works and how to run it.

Files you may add or change: `docs/guides/sop-rag.md`, `docs/README.md` (one
index line).

- `sop-rag.md` covers: what SOP retrieves and when; the two-database layout
  (rag-template dev on 5434, tran on 5435, SOP retrieval on 5436) and why; the operator steps; refreshing
  the corpus after docs change; resetting the index (`docker compose -p sop-rag
  down -v`, then start and ingest again) and why deleted docs need it;
  checking it with `check-live.sh` and a run's `rag.json`; turning it off with
  `SOP_RAG_ENABLED=false` or `enabled: false`.
- Link it from `docs/README.md` under guides.

### Dependencies

RAG-104

### Deliverables

- `docs/guides/sop-rag.md` and the index line.

### Acceptance Criteria

- Every command in the guide matches a script or flag added in RAG-101 to
  RAG-104.
- The guide states the deleted-document limit and the reset.
- `docs/README.md` links the guide.

## RAG-106 — Corpus folder correction

RAG-102 shipped the wrong corpus: `scripts/sop-rag/corpus.txt` lists
`docs/operations` where the Summary's decision names `docs/architecture`, and
RAG-105's guide repeats it. Replace `docs/operations` with `docs/architecture`
everywhere the corpus is stated, so the corpus is exactly `README.md` and the
Markdown files under `docs/architecture/`, `docs/guides/` and `docs/reference/`.

Files you may change: `scripts/sop-rag/corpus.txt` (line 7),
`scripts/sop-rag/test-sop-rag.sh` (the `for dir in ...` loop at line 95),
`docs/guides/sop-rag.md` (lines 30 and 88).

- Change only the folder name in those four places; nothing else in the files
  changes.
- The test must check the expected folder set from the Summary, not just echo
  whatever `corpus.txt` lists: it fails if `docs/architecture/` files are absent
  from `--dry-run` or any `docs/operations/` file is present.
- Add one guide check to `test-sop-rag.sh`: it fails unless
  `docs/guides/sop-rag.md` contains `docs/architecture/` and fails if it
  contains `docs/operations/`. The guide itself is already correct.

### Dependencies

- RAG-105

### Deliverables

- The corrected corpus list, test and guide.

### Acceptance Criteria

- `ingest.sh --dry-run` lists every Markdown file under `docs/architecture/`
  and nothing under `docs/operations/`, `docs/plans/`, `docs/reports/` or
  `docs/history/`.
- `docs/guides/sop-rag.md` names `docs/architecture/`, not `docs/operations/`.

### Acceptance Checks

- `cargo test --locked`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
