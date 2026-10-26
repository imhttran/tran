# Ask-the-archive repair — final acceptance

**Type:** Completion record (non-normative history). This is an ad-hoc repair tracked by
pull requests rather than an SOP plan, so **no SOP runtime state was modified**. The
separate `plan-tran-archive-photographs` plan and its `AP-101` approval gate were left
untouched.

Baseline: `main` @ `f1682a1`.

**Privacy:** public-safe summary. No family names, relationships, or passage text are
reproduced; only provenance (file name and PDF page numbers) and aggregate counts.

## Scope

Three merged pull requests repaired Ask the archive after the canonical family source
was approved for retrieval:

| PR | Change |
| -- | ------ |
| #95 | Derive suggested questions from approved archive content instead of a fixed list |
| #96 | Select the top `FINAL_K` sections before expanding them, so the matched chunk reaches the model |
| #98 | Anchor suggestions in substantive content rather than structural labels, and relax the full-text pass only after a strict miss |

Running commit: `f1682a1` (merge of #98; #95 and #96 are ancestors on `main`).

## Root causes repaired

1. **Unindexed family source.** The canonical Markdown was absent from
   `approved_documents`, so retrieval had no family evidence at all. Fixed through the
   document lifecycle (submit → approve for family scope → process), not by code.
2. **Expansion before truncation (#96).** Section expansion ran before the `FINAL_K`
   cut, so an earlier chunk of the same source displaced the matched chunk during
   context budgeting and the model never saw the evidence.
3. **Unmatchable suggestions (#98).** Suggested questions quoted structural section
   titles (`PDF page N`, `Contents`, editorial notes) that never appear in a passage,
   and `websearch_to_tsquery` ANDs the question template's scaffolding, so the keyword
   pass matched zero rows for every topic.

## Deterministic checks (recorded at merge; not re-run for this record)

| Check | Result |
| ----- | ------ |
| `cargo fmt --check` | PASS |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS |
| `cargo test --locked --lib` | PASS (495 tests) |
| `cargo test --locked --test rag_publication` (disposable database) | PASS (7 tests) |
| `npm run typecheck` | PASS |
| Focused browser suite (`e2e/rag.spec.ts --grep suggest`, disposable corpus) | PASS (3 tests: controlled fixture citation, empty suggestion state, failed suggestion state) |

A required check that did not execute is recorded as NOT-RUN, never as PASS.

## Post-merge verification (reused; no re-ingestion, no suite re-run)

| Check | Result |
| ----- | ------ |
| App health at `f1682a1` | `{"database":"ok","model":"ok","service":"rust-api","status":"ok"}` |
| `/ask` suggestion list | 6 content-derived topics; no page markers, `Contents`, or editorial metadata |
| Content-derived suggestion → answer | supported (`insufficient: false`); citations resolve to the section the topic was drawn from (`Tran-Family-English.md`, PDF page 3 and PDF page 2) |
| Canonical family source | 258 approved chunks in corpus `trusted` |
| Ingest state | unchanged; no document was re-submitted, re-approved, or re-processed |

## Preserved boundaries

- Corpus authorization, `approved_documents` visibility, publication revalidation,
  `MIN_SIMILARITY`, the answerability gate, and citation validation were not changed by
  this repair. The relaxed full-text pass adds recall only; evidence thresholds are
  unchanged.
- The family source remains private and untracked (`docs/tran/` ignore rule unchanged).
- Unrelated local working-tree items were preserved and not committed.

## Remaining limitations

- A suggested topic requires a substantive heading or a capitalised phrase from the
  passage. A prose-only document contributes none, and the page shows its explicit empty
  state while free-text questions stay available.
- The relaxed full-text pass runs only after a strict miss and ranks with `ts_rank`,
  which has no IDF term. It is bounded by fusion and the answerability gate.
- `normalize_fts_config` expects a BCP-47 tag (`en`), not a language name (`english`);
  a name selects `simple`. The production default (`""` → `english`) is unaffected.

## Status

**COMPLETE.** Ask the archive answers family questions with supported, correctly cited
evidence from the approved canonical source, and its suggestions are drawn from that
content.

## Evidence discipline

- Deterministic-check results are kept separate from live post-merge results.
- Aggregates and provenance only; no private names, relationships, or passage text.
- No result here is asserted PASS solely from manual inspection.
