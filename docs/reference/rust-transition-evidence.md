# Rust transition evidence (Rust-only)

Final evidence for the Go → Rust migration. RUST-007C moved the Cargo package to
the repository root and removed the Go implementation, `go.mod`/`go.sum`, and
Go-only tooling. The pinned Go reference and its frozen fixtures remain recorded
for audit.

The evidence crate `acceptance/evidence` reads this file from
`docs/reference/rust-transition-evidence.md` and asserts every section below.

## Capability matrix

| Capability | Final implementation | Status |
| --- | --- | --- |
| Build | `cargo build --locked` | Rust only |
| Unit/integration tests | `cargo test --locked` | Rust only |
| Lint | `cargo clippy --locked --all-targets -- -D warnings` | Rust only |
| Format check | `cargo fmt --check` | Rust only |
| Ingest | `cargo run --locked --bin ingest` (PDF, Markdown, plain text) | Rust only |
| Ask | `cargo run --locked --bin query` | Rust only |
| Eval | `cargo run --locked --bin eval` | Rust only |
| Bilingual export | `cargo run --locked --bin expbilingual` | Rust only |
| HTTP API | `cargo run --locked --bin api` (Axum) | Rust only |
| Browser UI | Next.js in `web/` over the Axum API | Rust + Next.js |
| Migration evidence | `acceptance/evidence`, `acceptance/final_layout` | Frozen |
| Clean checkout | `acceptance/clean_checkout` (Go absent from `PATH`) | Rust only |
| Go settings-matrix sweep | removed with the Go scripts; `eval` remains | Not ported |

The Makefile, `.githooks/`, and CI run the Rust-only commands. `make check` runs
`cargo test --locked`, the Rust checks, the final-layout/clean-checkout
acceptance, and the web typecheck/build.

## Remaining limitations

- The Go implementation was removed in RUST-007C. The pinned reference
  `d72fb398e8959eea83ed24be03f6df8cdeca86a2` and its frozen `fixtures/parity/`
  outputs are preserved; expected values are never regenerated from Rust.
- The Rust port has no replacement for the removed Go `scripts/sweep.sh` settings
  matrix. Use `cargo run --locked --bin eval` for evaluation.
- OCR for scanned PDFs is not offered; incomplete or fully scanned PDFs are
  rejected before embedding or database writes.
- RUST-008A delivered the real Axum/Next.js browser flow over the shared Rust
  services; browser checks are operator-run Playwright evidence, while the
  Rust-side contract is verified by `acceptance/api_contract`.
- The final Rust-only acceptance is RUST-008, which follows the verified RUST-007C
  layout/clean-checkout transition.

## Rollback reference

RUST-007C removed the Go source and tooling. If the Go reference must be restored
for audit:

1. Recover the Go tree from the pinned commit
   `d72fb398e8959eea83ed24be03f6df8cdeca86a2` (the RUST-001 reference freeze).
2. Restore `go.mod`/`go.sum`, `cmd/`, `internal/`, `tools/`, and the Go-era
   `Makefile`, hooks, and CI jobs from that commit.
3. Rebuild and test the Go reference with `go build ./...`, `go test ./...`, and
   `go vet ./...`.
4. The frozen Rust fixtures, `migrations/`, `evals/`, `examples/`, and
   `docker-compose.yml` are unchanged and need no recovery.

The Rust-only repository remains the forward implementation; the Go reference is
historical.

## Pinned Go commit SHA

`d72fb398e8959eea83ed24be03f6df8cdeca86a2`

## Source repository URL

Current repository: `https://github.com/imhttran/rag-template-rust`

Historical Go reference source URL: `https://github.com/tomtran/rag-template`

## Fixture provenance

Frozen fixtures and shared assets. RUST-007C does not modify them.

| Asset | Purpose |
| --- | --- |
| `migrations/*.sql` | PostgreSQL + pgvector schema; applied by `make db-schema`. Read-only inputs. |
| `evals/` | Frozen retrieval evaluation set (`evals/retrieval.json`). Read-only input. |
| `examples/` | Frozen sample documents (for example `examples/loan-policy.md`). Read-only input. |
| `docker-compose.yml` | PostgreSQL + pgvector service definition. Unchanged. |
| `fixtures/` | Frozen Rust fixture set (`en/`, `vi/`, `page/`, `parity/`) and the captured Go reference outputs under `fixtures/parity/frozen/`. Unchanged. |

Each asset keeps the provenance recorded in `RUST-001-go-baseline.md` (fixture
manifest) and `RUST-003-fixture-parity.md` (section/chunk acceptance matrix). The
frozen K=3/K=4 and bilingual outputs in `fixtures/parity/frozen/` record the pinned
Go commit and the deterministic capture setup.

## CI configuration

`.github/workflows/ci.yml` defines the Rust-only layout:

- `rust` installs the pinned toolchain from `/rust-toolchain.toml`, applies the
  migrations to a pgvector service, then runs `cargo fmt --check`,
  `cargo build --locked`, `cargo test --locked`, Clippy, and
  `make transition-evidence`.
- `api-contract` applies the migrations and runs
  `make rust-acceptance-api-contract`.
- `web` runs `npm ci`, `npm run typecheck`, and `npm run build` for the Next.js UI.

The local equivalents are the `.githooks/` hooks, which delegate to the same
Makefile targets.
