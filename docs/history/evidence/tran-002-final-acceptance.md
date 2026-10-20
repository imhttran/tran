# TRAN-002 Final Acceptance (Public-Safe Summary)

Technical acceptance summary for the TRAN-002 integration and closure task. This file contains only synthetic test evidence and nonidentifying aggregate metrics. It contains no private names, relationships, source content, or local database export names.

## Scope

- Four source inputs remain in scope. Their digests live in a private, git-ignored manifest and are **not** reproduced here.
- All public evidence in this file is derived from deterministic checks or from synthetic fixtures only.

## Deterministic checks

| Check                         | Command (by name)                                                           | Result                                                                                         |
| ----------------------------- | --------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Rust format                   | `cargo fmt --check`                                                         | PASS                                                                                           |
| Rust build                    | `cargo build --locked`                                                      | PASS                                                                                           |
| Rust tests                    | `cargo test --locked`                                                       | PASS (disposable PostgreSQL available)                                                         |
| Rust clippy                   | `cargo clippy --locked --all-targets -- -D warnings`                        | PASS                                                                                           |
| DB schema apply + schema test | `psql … -f … && cargo test --locked --test genealogy_schema -- --nocapture` | PASS                                                                                           |
| DB CLI integration            | `cargo test --locked --test genealogy_cli`                                  | PASS                                                                                           |
| Evidence provenance           | `cargo test --locked --test genealogy_provenance`                           | PASS (null links and derivative catalogs cannot verify; original source with page/locator can) |
| Rust acceptance suite         | `make rust-check`                                                           | PASS                                                                                           |
| Source integrity (SHA-256)    | pre/post hash compare vs private manifest                                   | PASS (all four digests unchanged; values not reproduced)                                       |
| Fresh source inventory        | inventory CLI, exactly four inputs                                          | PASS (four source records; generated audit output excluded)                                    |
| Diff hygiene                  | `git diff --check`                                                          | PASS                                                                                           |
| Git privacy                   | `git ls-files docs/tran/` / `git check-ignore -v docs/tran/`                | PASS (no tracked source; ignore rule present)                                                  |
| Staged-content privacy scan   | `git diff --cached` scan for `docs/tran/` and private identifiers           | PASS                                                                                           |

A required check that did not execute is recorded as NOT-RUN, never as PASS.

## Manual review and unresolved genealogy questions

- Source-to-page identity and every family relationship remain candidate, disputed, or unverified until a reviewer supplies evidence.
- The unresolved-assertion report remains the review queue; no relationship is auto-promoted.
- The Head of Family eligibility decision returns `undetermined` whenever branch, male designation, first-born order, or living evidence is missing, contested, or deceased. No successor is chosen automatically.
- The genealogy designation is a domain value only and confers no RBAC permission.

## Privacy gate summary

- `/docs/tran/` ignore rule is present in `.gitignore`.
- Hooks (`.githooks/`) and CI workflows do not read or publish under `docs/tran/`.
- No private data is staged for commit. The staged-content scan runs before any push.
- Local audit reports, source documents, and database exports remain untracked.

## Evidence discipline

- Aggregate, nonidentifying metrics only.
- Deterministic-check results are kept separate from manual-review results.
- No result here is asserted PASS solely from manual inspection.
