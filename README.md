# Tran Family Archive

A private, evidence-backed family genealogy archive. It reads the family source
documents, stores the genealogy model — people, assertions, evidence, branches,
appointments — and presents **approved** family information to relatives through a
Rust API and a Next.js UI. A Retrieval-Augmented Generation (RAG) foundation runs
underneath for document search and cited answers.

The archive is built around one rule: **nothing reaches a family member until an
independent reviewer has approved it, and a member can always withdraw what they
shared.**

## Core principles

- **Approved-only presentation.** Drafts, pending and rejected submissions, owner-private
  fields, private media, and superseded versions never reach a family page or a retrieval
  answer. An owner restriction (decision A2) withdraws shared content immediately.
- **Evidence-backed facts.** A genealogy assertion is presented only when it is verified
  against original source evidence. Disputed, unverified, or unsupported lineage claims
  are withheld rather than guessed.
- **Authorization is technical, never genealogical.** A reviewed account-person link,
  a role, and an explicit grant decide access. A family relationship, branch membership,
  or the Head of Family title grants nothing by itself.
- **Private by default.** Contact details, addresses, private photographs, and personal
  documents stay private unless the owner explicitly shares them and a reviewer approves.
- **Local-first.** Embedding and generation run against a local Ollama server; storage is
  PostgreSQL with pgvector.

## Architecture

```mermaid
flowchart TD
    src[Family source documents] --> inv[Inventory and extraction]
    inv --> gen[(Genealogy PostgreSQL)]
    docs[Uploaded documents] --> ingest[Ingest and chunk]
    ingest --> rag[(RAG PostgreSQL + pgvector)]
    gen --> api[Rust Axum API]
    rag --> api
    web[Next.js UI] -->|same-origin /api/*| api
    gen --> review[Review console and branch review]
    review --> gen
```

Two PostgreSQL databases are involved:

- the **RAG database** (`DATABASE_URL`) holds documents, chunks, and embeddings;
- the **genealogy database** (`GENEALOGY_DATABASE_URL`) holds people, assertions,
  evidence, branches, profile revisions, review history, and grants.

The family surfaces require both. The retrieval commands need only the RAG database.
The API is the sole authorization authority; the browser never supplies a role,
identity, visibility, or approval status.

See the [architecture overview](docs/architecture/OVERVIEW.md) for the full picture,
the [family publication contract](docs/architecture/FAMILY-PUBLICATION-CONTRACT.md) for
what may be presented or indexed, and the
[genealogy schema contract](docs/architecture/GENEALOGY-SCHEMA-CONTRACT.md) for the data
model.

## Requirements

- Rust, using the channel pinned in [`rust-toolchain.toml`](rust-toolchain.toml). With
  `rustup` installed, the pinned toolchain and components install automatically when you
  run `cargo` in this repository.
- Ollama running locally, with an embedding model and a chat model:

  ```bash
  ollama pull nomic-embed-text
  ollama pull qwen3.8:27b-mlx   # or set OLLAMA_CHAT_MODEL to a model you have
  ```

- Docker (for the bundled PostgreSQL + pgvector, `pgvector/pgvector:pg17`).
- Node.js 20–24 for the Next.js UI (see `web/package.json` `engines`).

## Quick start: retrieval

```bash
make db-up                      # start PostgreSQL + pgvector and apply the RAG schema
make ingest                     # ingest examples/loan-policy.md
make ask Q="What happens if someone misses a payment?"
make eval                       # score retrieval against evals/retrieval.json
```

Settings come from the process environment. [`.env.example`](.env.example) lists every
variable:

```bash
cp .env.example .env
set -a; source .env; set +a
```

The [usage guide](docs/guides/usage.md) covers direct invocations and the full setup.

## Quick start: family archive

```bash
export DATABASE_URL=postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable
export GENEALOGY_DATABASE_URL=postgres://rag:rag@127.0.0.1:5434/genealogy?sslmode=disable

# create the genealogy database once, then apply its migrations
psql "$DATABASE_URL" -c "CREATE DATABASE genealogy"
for f in genealogy/migrations/*.sql; do
  psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 -f "$f"
done

cargo run --locked --bin api    # 127.0.0.1:8080
make web-start                  # 127.0.0.1:3000
```

Identity, roles, reviewer grants, and recovery are operator procedures: see
[operations/RBAC-OPERATIONS.md](docs/operations/RBAC-OPERATIONS.md).

## Commands

| Command                                                          | What it does                                           |
| ---------------------------------------------------------------- | ------------------------------------------------------ |
| `cargo run --locked --bin ingest <file>`                         | Chunk a file and store its embeddings (safe to re-run) |
| `cargo run --locked --bin query "question"`                      | Answer a question from stored chunks with citations    |
| `cargo run --locked --bin eval`                                  | Score retrieval against `evals/retrieval.json`         |
| `cargo run --locked --bin expbilingual`                          | Run the bilingual evaluation arms                      |
| `cargo run --locked --bin api`                                   | Serve the Axum API for the Next.js UI                  |
| `cargo run --locked --bin genealogy`                             | Genealogy ingest and eligibility adapters              |
| `cargo run --locked --bin process_documents`                     | Claim and process queued document jobs                 |
| `cargo run --locked --bin admin`                                 | Administrative maintenance commands                    |
| `cargo run --locked --bin inventory [SOURCE_ROOT] <manifest>`    | Inventory the family source corpus (read-only)         |
| `cargo run --locked --bin pdf_assessment [SOURCE_ROOT] <report>` | Assess PDF text layers (read-only)                     |

The full list, flags, and `make` shortcuts are in
[reference/commands.md](docs/reference/commands.md).

## Web UI

The browser calls only same-origin Next.js `/api/*` endpoints; the Next.js server
proxies them to the Axum API, which applies RBAC and the publication rules.

| Route                   | Purpose                                                    |
| ----------------------- | ---------------------------------------------------------- |
| `/genealogy`            | Family tree and person selection                           |
| `/family/[personKey]`   | Approved family profile, biography, photographs, documents |
| `/profile`              | A member's own profile, submissions, and **Make private**  |
| `/admin/reviews`        | Contribution review queue and decisions                    |
| `/branch-review`        | Branch-scoped reviewer queue and proposals                 |
| `/head-of-family`       | Head of Family evidence and appointments                   |
| `/documents`            | Document lifecycle and publication                         |
| `/ask`                  | Retrieval question and cited answer                        |
| `/admin/family-members` | Member administration                                      |

```bash
make web-check     # typecheck + production build
make web-e2e       # real-browser Playwright suite against disposable databases
```

See [web/README.md](web/README.md) and the
[browser evidence](docs/reference/rust-008a-browser-evidence.md).

## Testing

```bash
cargo build --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

Database-backed tests need `DATABASE_URL` and `GENEALOGY_DATABASE_URL`; each genealogy
suite creates and drops its own disposable database. See
[reference/testing.md](docs/reference/testing.md) and
[guides/testing.md](docs/guides/testing.md).

Changes are governed by Agentic SOP in this repository:

```bash
sop validate    # configured build, test, integration, and lint gates
sop review      # review the current change
```

## Documentation

The full index, with a one-line "when to read it" for every document, lives in
[docs/README.md](docs/README.md). Visual identity, design tokens, typography,
iconography, and motion are defined in
[docs/reference/brand.md](docs/reference/brand.md).

| Area                                              | What is here                                                                   |
| ------------------------------------------------- | ------------------------------------------------------------------------------ |
| [Architecture](docs/architecture/OVERVIEW.md)     | Pipeline, package responsibilities, and the publication and schema contracts   |
| [Operations](docs/operations/RBAC-OPERATIONS.md)  | Identity, RBAC, Head of Family, embedding, and platform procedures             |
| [Reference](docs/reference/commands.md)           | Commands, configuration, capabilities, crate, testing, experiments             |
| [Guides](docs/guides/usage.md)                    | Task-oriented how-tos for usage, profiles, review, branch review, and testing  |
| [Plans](docs/plans/PLAN-TRAN-USER-ONBOARDING.md)  | Recorded and active implementation plans                                       |
| [History](docs/history/lessons.md)                | Archived requirements, experiment evidence, and retrospectives (non-normative) |
| [Reports](docs/reports/rust-migration/README.md)  | Engineering evidence, reviews, and investigation reports (non-normative)       |
| [Source corpus](docs/tran/Tran-Family-English.md) | The family source text edition and its audit findings                          |

## Status

Shipped and covered by tests:

- the retrieval loop (ingest → chunk → embed → retrieve → answer with citations),
  hybrid search, answerability gating, and the evaluation harness;
- private genealogy access with reviewed identity links, RBAC, the administrator
  review console, and Head of Family eligibility;
- branch-scoped reviewer grants and proposals;
- approved-only family publication: the family profile endpoint and page, approved
  photographs and documents, owner-initiated immediate privacy restriction (A2), and
  revocation-safe document retrieval;
- reliable document processing workers with fenced leases;
- isolated, repeatable genealogy integration tests (one disposable database per suite).

Open policy gates, deliberately unimplemented: decision **D** (which content may be
indexed), **BR-004** (cross-branch relationship authority), and Head of Family
eligibility revalidation. The acceptance matrices record their status explicitly:
[family publication](docs/guides/family-publication-acceptance.md) and
[branch review](docs/guides/branch-review-acceptance.md).

OCR for scanned PDFs and genealogy entity extraction are **not** implemented; the
archived phase-23b requirements describe that proposal.
