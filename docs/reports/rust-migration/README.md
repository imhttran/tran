# Rust migration evidence (RUST-001 baseline)

Evidence for `docs/plans/PLAN-RUST-MIGRATION.md`. Reference SHA frozen by RUST-001:
`d72fb398e8959eea83ed24be03f6df8cdeca86a2` (branch `main`).

| File | Purpose |
| --- | --- |
| `RUST-001-go-baseline.md` | RUST-001 reference freeze: SHA, toolchain, environment, pinned commands, configuration, dependencies, capability→Rust-task map, evaluation settings/thresholds/tolerances, fixture manifest, isolated gaps |
| `fixtures/en/` | Frozen English retrieval fixture (input, question, labelled expected fields) |
| `fixtures/vi/` | Frozen Vietnamese bilingual fixture (input, question, labelled expected fields) |
| `fixtures/page/` | Page-provenance fixture (text-layer input, expected 1-based pages) and the scanned/no-text-layer rejection fixture |
| `sop-development-process.md` | How SOP was applied to this migration, effort counts, per-task record, benefits/friction, and the acceptance-check capability gap |
| `sop-enhancement-rust-acceptance-checks.md` | Proposed SOP enhancement enabling scoped, opt-in `cargo` acceptance checks |
| `dependency-decisions.md` | Retained / adopted / selected Rust crates, the pinned-toolchain constraint, the Markdown/PDF adoption gate, and the verification behind each |
| `cli-fullsuite-timeout-evidence.md` | Preserved evidence for the pre-existing full `internal/cli` suite stall (commands, stacks, attribution, reproduction guidance) |
| `RUST-003-fixture-parity.md` | RUST-003 section/chunk acceptance matrix vs the pinned Go reference (provenance, repairs, per-case results, the one unresolved PDF-signaling difference) |
| `RUST-007D-bilingual-contract.md` | RUST-007D frozen `cmd/expbilingual` contract, capability inventory, divergences, and the Go→Rust invocation mapping for all four commands |
| `RUST-007D-live-model-evidence.md` | RUST-007D live-model groundedness/citation closure evidence (accepted; `VERIFIED`) |
| `RUST-007E-live-evidence.md` | RUST-007E live Ollama + PostgreSQL ingest → query → cited-answer observations, citations, and missing-context refusal |
| `RUST-007E-k3-parity.md` | RUST-007E original K=3 parity: frozen Go reference vs Rust `eval`, ranking order, and metrics |
| `RUST-007E-md-text-ingestion.md` | RUST-007E production Markdown/plain-text ingestion, provenance, and transactional safety |
| `web-e2e/` | RUST-008A Playwright screenshots for the real browser flow and UI states |
| `web-e2e/rust-008-acceptance/` | RUST-008 acceptance-run screenshots preserved with the then-required mock-only label |
| `RUST-008-final-acceptance.md` | RUST-008 operator evidence, six passing gates, post-banner-removal verification, and the SOP `VERIFIED / LOCAL_DONE` verdict |
| `sop-cargo-support.patch` | Reviewed SOP Cargo-support patch, base `a46187b` (SHA-256 `79dd707e…b893`) |
| `sop-cargo-support-integrated.patch` | The same patch integrated onto current `origin/main` `a9f0495` (SHA-256 `48530f34…4ac6`) |

Every captured expected output is labelled by comparison type (`exact`,
`float_tol_0.001`, `order`) consistent with the comparison rules in
`RUST-001-go-baseline.md` section 8. Outputs that could not be captured because the
pinned Go commands require Docker/PostgreSQL and Ollama are recorded as isolated
gaps, never as captured.
