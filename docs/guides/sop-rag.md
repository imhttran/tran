# SOP RAG guide

This guide shows how to run and check the SOP RAG knowledge layer for tran.
The layer gives every SOP task a few cited passages from tran's own docs.
The scripts live in [`scripts/sop-rag/`](../../scripts/sop-rag/).
The plan that builds them is
[`plans/PLAN-TRAN-SOP-RAG-RETRIEVAL.md`](../plans/PLAN-TRAN-SOP-RAG-RETRIEVAL.md).

## What SOP retrieves and when

When the layer is enabled, SOP runs one executable for a task and sends a
request on stdin. SOP gets the answer on stdout. See the `rag:` block in
[`scripts/sop-rag/rag.config.yaml`](../../scripts/sop-rag/rag.config.yaml).

SOP adds up to `top_k` (default 5) cited items within `context_budget_bytes`
(default 4096) to a task's context. This happens when the task has no
`required_evidence`. A retrieval failure is ignored, and the task runs without
the items.

The executable is
[`scripts/sop-rag/retrieval.sh`](../../scripts/sop-rag/retrieval.sh).
It sources [`env.sh`](../../scripts/sop-rag/env.sh) and execs the `retrieval`
binary. `env.sh` assigns `DATABASE_URL`, `OLLAMA_*`, `EMBED_*`,
`RAG_CORPUS` and `RAG_RETRIEVAL_ALLOWED_CORPORA` from their
`SOP_RAG_`-prefixed overrides or defaults. An inherited `DATABASE_URL` never
wins. The retrieval database defaults to
`postgres://rag:rag@127.0.0.1:5436/rag?sslmode=disable`.

The corpus is `tran-docs`. It holds `README.md` and the Markdown files in
`docs/guides/`, `docs/architecture/` and `docs/reference/`. Plans, reports and
history stay out. The list is
[`scripts/sop-rag/corpus.txt`](../../scripts/sop-rag/corpus.txt).

## The database layout and why it is separate

Three Postgres instances can run on the same machine. Each uses its own port:

| Port | Database         | Owner                     |
| ---- | ---------------- | ------------------------- |
| 5434 | rag-template dev | `rag-template-postgres-1` |
| 5435 | tran             | `tran-postgres-1`         |
| 5436 | SOP retrieval    | Compose project `sop-rag` |

Ports 5434 and 5435 are already taken. The SOP retrieval database runs as its
own Compose project on 5436. This keeps the SOP index apart from rag-template's
scratch data and from tran's application data. A reset of the SOP index cannot
harm the other two.

The port is enforced by
[`scripts/sop-rag/compose.port.yml`](../../scripts/sop-rag/compose.port.yml),
which overrides only the `postgres` service to `5436:5432`.

## Operator steps

These steps are run by a human operator, not by SOP. SOP cannot build outside
tran, start Docker, pull models, or edit its untracked config.

```sh
# 1. Build the producer and the ingester from rag-template-rust main.
cd ~/agentic-workspace/projects/rag-template-rust
git switch main && git pull
cargo build --locked --release --bin retrieval --bin ingest
mkdir -p bin && cp target/release/retrieval target/release/ingest bin/

# 2. Start the retrieval database on 5436 as its own Compose project.
docker compose -p sop-rag -f docker-compose.yml \
  -f ~/agentic-workspace/projects/tran/scripts/sop-rag/compose.port.yml up -d --wait

# 3. Make sure Ollama has the embedding model.
ollama pull nomic-embed-text

# 4. Ingest the corpus.
~/agentic-workspace/projects/tran/scripts/sop-rag/ingest.sh

# 5. Turn the layer on: append the rag: block to the primary checkout's
#    SOP config (.agent-sdlc/ is gitignored).
cd ~/agentic-workspace/projects/tran
cat scripts/sop-rag/rag.config.yaml >> .agent-sdlc/config.yaml

# 6. Prove it live.
./scripts/sop-rag/check-live.sh
```

## Refreshing the corpus after a docs change

[`scripts/sop-rag/ingest.sh`](../../scripts/sop-rag/ingest.sh) expands
`corpus.txt` and runs `ingest FILE` once per file. Run it again after a docs
change to `README.md` or a file under `docs/guides/`, `docs/architecture/` or
`docs/reference/`:

```sh
./scripts/sop-rag/ingest.sh
```

You can preview the file list and run nothing with `--dry-run`:

```sh
./scripts/sop-rag/ingest.sh --dry-run
```

Re-running ingest on the same file is safe. It adds or updates each document.
SOP reads the new content on its next task. No restart of SOP is needed for a
content change.

## Resetting the index, and the deleted-document limit

**Ingest adds and updates documents, but it never removes one.** A deleted or
renamed doc stays retrievable until you reset the database. This is the
deleted-document limit. A moved file leaves a stale copy behind, and a deleted
file keeps answering.

The remedy is a full reset:

```sh
# From the rag-template-rust directory, stop the SOP project and drop the volume.
docker compose -p sop-rag down -v
```

Then start the database and ingest the corpus again. Rerun operator steps 2 and
4:

```sh
# Start the database on 5436 again (operator step 2).
docker compose -p sop-rag -f docker-compose.yml \
  -f ~/agentic-workspace/projects/tran/scripts/sop-rag/compose.port.yml up -d --wait

# Ingest the corpus again (operator step 4).
~/agentic-workspace/projects/tran/scripts/sop-rag/ingest.sh
```

The `-v` flag drops the database volume, so the next start is empty. Only the
`sop-rag` project is affected. tran on 5435 and rag-template dev on 5434 are
left alone.

## Checking the setup

### Live check

Run the live check after the operator steps:

```sh
./scripts/sop-rag/check-live.sh
```

[`scripts/sop-rag/check-live.sh`](../../scripts/sop-rag/check-live.sh) sends
two requests through `retrieval.sh`. The first is an allowed query for corpus
`tran-docs`; it must return status `OK` with at least one cited item. The
second uses corpus `not-allowed`; it must return `UNAUTHORIZED_SCOPE`. The
script exits zero only when both checks pass. On failure it prints the status
and the producer's `diagnostics.error_code`. It prints the operator steps when
the status is `UNAVAILABLE`.

### A run's rag.json

SOP writes a `rag.json` file in each run's directory. It records what that run
retrieved. Read it to see whether the task got cited passages and which ones.
The schema and location are owned by SOP, not by tran. This repository does not
define its fields, so read the file from the run directory directly rather than
relying on a fixed field list here.

## Turning it off

There are two off switches. Use the one that matches where you work.

- **Per process.** Set `SOP_RAG_ENABLED=false` in the environment of the SOP
  process. Use this for a one-off run or a shell session.
- **Per config.** Set `enabled: false` in the `rag:` block of
  `.agent-sdlc/config.yaml`. Use this to turn the layer off for a checkout.

The template block is
[`scripts/sop-rag/rag.config.yaml`](../../scripts/sop-rag/rag.config.yaml). It
ships with `enabled: true`.
