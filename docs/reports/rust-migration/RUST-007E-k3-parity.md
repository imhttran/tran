# RUST-007E — original K=3 Go/Rust parity

Deterministic evidence that the production Rust `eval` command matches the
pinned Go reference at the original K=3 setting. Expected values come from the
Go reference; no fixture was regenerated from Rust.

## Contract

| Item | Value |
| --- | --- |
| Go reference | pinned commit `d72fb398e8959eea83ed24be03f6df8cdeca86a2` |
| Dataset | `evals/retrieval.json` |
| Embeddings | deterministic scripted server shared by both commands |
| `TOP_K` | `3` |
| `FINAL_K` | `3` |
| `EXPAND_LIMIT` | `20` |
| `MIN_SIMILARITY` | `0.6` |
| Rewrite / lexical rerank / LLM rerank / answerability gate / fact judge | off |
| Acceptance crate | `acceptance/k3_parity` |
| Ranking reference | `fixtures/parity/frozen/k3_rankdump_go_stdout.txt` (frozen from the pinned Go reference) |

The captured Go output used `TOP_K=3`, `FINAL_K=3`; Go's `cmd/eval` emits
`K=1,2,TOP_K`, so the K=3 line is direct. K=4 output is not used for this
evidence.

## Comparison

The crate ingests the dataset with the production Rust `ingest` binary and runs
the production Rust `eval` against the PostgreSQL corpus with deterministic
embeddings. It compares the Rust output with the frozen Go outputs
(`fixtures/parity/frozen/eval_k3_go_stdout.txt` and
`k3_rankdump_go_stdout.txt`). No Go toolchain is required. It compares:

- case count and question order (exact);
- per-case K=3 recall and precision (absolute tolerance `0.005`, one printed
  rounding unit);
- aggregate recall and precision;
- every case's fused source/section/chunk ranking order and tie ordering
  (exact);
- rejection labels for unanswerable cases.

## Result

`cargo test --locked --manifest-path acceptance/k3_parity/Cargo.toml` →
**PASS** (1 test). Ranking order, tie ordering, per-case metrics, and aggregate
metrics match the Go reference.

## Implementation note

The ranking comparison exposed a real divergence: the Rust evaluation path fused
candidates and printed every chunk, while the Go baseline applies
`DeduplicateSections` after fusion at `FINAL_K`. Rust `evaluation::evaluate_sql`
now applies the shared section deduplication (source + section, first occurrence)
before scoring and printing, matching the Go pipeline. The shared
`retrieval::dedup::dedup_sections` helper is covered by unit tests.
