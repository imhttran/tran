# Rust best-practices review — `rag-foundation`

- **Date:** 2026-10-20
- **Branch:** `review/whole-crate-rust-best-practices` (from `main` at `97c1f6d`)
- **Scope:** the `rag-foundation` crate — `src/` (library, 65 files / 16,042 lines, plus
  nine binaries) and its inline tests. The `tests/` and `acceptance/` crates were
  consulted only to confirm call sites and reachability.
- **Rubric:** the `rust-best-practices` skill (Apollo GraphQL handbook): borrowing,
  `Option`/`Result`, error handling, performance, generics/dispatch, type state,
  comments/docs, testing, pointers.

## Verdict

The crate is **idiomatic at its core and unusually disciplined about effects**: no
`unsafe`, no hand-rolled SQL string interpolation, almost no bare `unwrap`/`expect` in
production code, and a clear "guard before any effect" ordering that is proved by
tests rather than asserted in prose. (The remaining panic paths are not bare `unwrap`s:
they come from unchecked duration arithmetic, `postgres::Row::get` type mismatches, and
a `to_lowercase` byte offset used to slice the original string — M7, M8, M9.) `cargo fmt --check`, `cargo clippy --all-targets
--all-features --locked -- -D warnings`, and the 278 library tests all pass on the
reviewed commit.

The weaknesses cluster in four places:

1. **Data isolation at the storage boundary** — the `(corpus, source)` identity is not
   carried through the replacement and unchanged-source paths. This is a correctness
   and data-loss class of bug, not a style issue.
2. **Error modelling** — many module boundaries flatten typed errors into `String`, and
   the API then re-derives HTTP status codes by substring-matching the message.
3. **Per-request and per-row work** — HTTP clients are rebuilt per request, the whole
   corpus (including embeddings) is read and parsed per query, and several loops issue
   one query per row.
4. **Documentation drift** — a few module docs describe a design that no longer exists,
   and 14 configured settings are parsed but never read.

Nothing in the review requires a rewrite. The highest-value work is a small number of
targeted changes at well-defined boundaries.

## Findings

### High — correctness and data isolation

> **Resolution:** H1 and H2 were fixed on `main` by `17c93ab` (`Corpus`
> newtype, corpus-scoped replacement, and corpus-scoped fingerprint reads).
> H3 is fixed on this branch: re-import captures the baseline before the
> database trigger can remove it, reconciles removed evidence, and reports
> trigger-driven reopens with one audit event. The regression test is in
> `tests/genealogy_repairs.rs`; the pure decision is unit-tested in
> `src/genealogy/reimport.rs`.

#### H1. `replace_source` deletes a source across every corpus

**Status:** fixed on `main` (`17c93ab`) — `SourceReplacer::replace_source`
takes a `Corpus`, the delete is `WHERE corpus = $1 AND source = $2`, and
`validate_chunk_corpora` rejects a chunk from another corpus before any effect.

`src/storage_pg.rs:554`

```rust
tx.execute("DELETE FROM documents WHERE source = $1", &[&source])
```

`corpus` is written on every row and is part of every read predicate, but this delete is
the only corpus-unaware write in the codebase. Two corpora that ingest a file with the
same basename — the normal case, since `source` is just the filename
(`src/bin/ingest.rs:63`) — will delete each other's rows. This directly contradicts
`docs/plans/PLAN-TRAN-RBAC.md:113`: "Two authorized corpora with the same document
filename retain independent data on replacement and reimport."

**Fix:** carry the corpus in the port and the predicate:

```rust
fn replace_source(&self, corpus: &str, source: &str, chunks: &[StoredChunk]) -> Result<(), ReplaceError>;
// ...
tx.execute(
    "DELETE FROM documents WHERE corpus = $1 AND source = $2",
    &[&corpus, &source],
)?;
```

Also key `RecordingSourceReplacer` by `(corpus, source)` and add a cross-corpus
replacement test. The existing `successful_replacement_leaves_other_sources_alone`
varies only `source`, so it cannot catch this.

#### H2. The unchanged-source decision is corpus-blind

**Status:** fixed on `main` (`17c93ab`) —
`FingerprintReader::rows_for_hash` and `unchanged_source` take a `Corpus`, and
the read uses `WHERE corpus = $1 AND content_hash = $2 AND source = $3`.

`src/storage_pg.rs:257` (`find_by_content_hash`), reached from
`src/fingerprint.rs:143` and `src/pipeline.rs:227`

`unchanged_source(reader, source, content_hash)` never receives the corpus, so a
`README.md` already present in corpus A makes the same bytes in corpus B report
`Unchanged { rows: N }`; corpus B is left with zero rows or stale rows. `rows_for_hash`
selects on `content_hash` and `source` only (`src/storage_pg.rs:254-266`).

**Fix:** add `corpus` to `FingerprintReader::rows_for_hash` and to the `WHERE` clause,
and pass `self.config.corpus` from `after_guard`.

#### H3. Re-import reports "unchanged" for assertions the trigger just reopened, and never removes stale evidence

**Status:** fixed — the baseline is captured before the evidence sync, changed
evidence and removed evidence are both reconciled, and the reopen decision uses
the trigger's post-state as well as the fingerprint.

`src/genealogy/reimport.rs:137-146`, `:176-199`, `:237-250`; trigger at
`genealogy/migrations/0004_review_workflow.sql:75-84`

`sync_evidence` inserts new `assertion_evidence` rows **before**
`reopened_by_evidence_change` runs. The `assertion_evidence_change_reopens` trigger
fires on that insert, flips the assertion to `needs_review`, and deletes
`assertion_review_baseline`. `reopened_by_evidence_change` then finds no baseline and
returns `Ok(false)`, so the report counts `unchanged` and no `reopened` audit event is
written — even though the status really changed.

The second half is independent: `sync_evidence` only ever `INSERT ... ON CONFLICT DO
UPDATE`. Evidence rows removed from the source are never deleted, so the evidence set
can only grow and a removed citation leaves the assertion permanently verified against
provenance that no longer exists.

**Fix:** make syncing reconciliatory (delete rows not in the new set, letting the
trigger reopen the assertion), and decide the outcome by re-reading `review_status`
after the sync rather than by baseline presence. The trigger is already the authority;
`reopened_by_evidence_change` duplicates it and should either be removed or become
advisory.

### Medium

> **Resolution (`fix/rust-best-practices-mediums`):** M1–M20 are addressed. The
> API classifies dependency failures by type, builds the query service and health
> client once, reads a corpus projection without embeddings, moves staging and
> hashing into `spawn_blocking`, and enforces both question and input limits.
> Storage decodes every column fallibly, uses one batch insert, and validates the
> embedding model and dimension before reporting a source unchanged. Genealogy
> uses one joined citation query and one validation transaction, matches
> relationship labels instead of substrings, and no longer claims an unreachable
> duplicate check. Errors are typed across storage, pipeline, genealogy, PDF,
> evaluation, query service, and API boundaries. Inert config settings and the
> unreachable rerank/filter/render helpers were removed; `QUERY_REWRITE` and
> `RAG_ANSWERABILITY_GATE` are now wired.

#### M1. HTTP status codes are chosen by substring-matching error text

`src/api.rs:200-207`

```rust
let text = error.to_string().to_ascii_lowercase();
text.contains("connection") || text.contains("pool") || text.contains("timeout") ...
```

A loader error that happens to contain "connection" becomes `503`, and any rewording of
a `PgStoreError` silently turns an outage into a `500`. The compiler cannot help here,
and the `PipelineError` variants are discarded at the boundary.

**Fix:** classify on the type/source chain (`downcast_ref::<PgStoreError>()`,
`downcast_ref::<OllamaError>()`), or add an explicit
`PipelineError::Dependency { .. }` at the origin.

#### M2. A fresh `QueryService` and two HTTP clients are built per request

`src/api.rs:500-503`, `src/query_service.rs:108-123`

```rust
let service = QueryService::from_config(&config, store, corpus)?;
```

`from_config` constructs two `reqwest::blocking::Client`s, each with its own internal
runtime thread and connection pool. Every query therefore pays client construction and
TCP/TLS setup and throws both pools away; keep-alive never happens. `model_reachable`
(`src/api.rs:147`) builds a third client per health poll.

**Fix:** build the service (or at least one shared `HttpTransport`) once at startup on a
dedicated blocking thread and store `Arc<QueryService>` in `AppState`. `answer` already
takes `&self`, and `HttpTransport` is `Sync`.

#### M3. Each query reads and parses the entire corpus, embeddings included

`src/query_service.rs:41-49`, `src/storage_pg.rs:277-290`, `:467-470`

`rows_for_corpus` selects `embedding::text` for every document and `row_to_chunk` parses
768 floats per row. `stored_chunk` immediately discards the embedding: only content and
provenance are needed for expansion. A query therefore pays an O(corpus) transfer and
float parse it never uses, in addition to the vector and FTS queries.

**Fix:** add a `chunks_for_corpus` projection without `embedding`, or move expansion into
SQL. Keep the full read for dedup/interop paths.

#### M4. N+1 citations in the genealogy read path

`src/genealogy/query.rs:64-79`, `:94-104`, `:216-227`

`assertion_rows` issues one `load_citations` query per assertion, and
`unresolved_assertions` does the same. A person report is 1+N round trips; the
unresolved report is 1+ (the entire unresolved set).

**Fix:** one `LEFT JOIN assertion_evidence` query grouped by `assertion_id`, and factor
the duplicated row → `AssertionRow` mapping into one helper.

#### M5. The unresolved report opens a transaction per assertion

`src/genealogy/report.rs:92-123`

For each row, `run_validation` starts a transaction, runs `validate_candidate`
(one to four queries), and commits a read-only transaction. A report over N assertions
costs N transactions and up to 4N queries — the slowest path in the module, and the one
a reviewer runs most often.

**Fix:** one transaction around the loop (or none — `validate_candidate` can take
`&mut Client`), or a set-based validation query for all assertion IDs at once.

#### M6. The duplicate parent-child rule can never fire

`src/genealogy/validation.rs:182-210`

`check_duplicate_parent_child` reads `parent_child_link`, which has
`UNIQUE (parent_id, child_id)` (`genealogy/migrations/0003_assertions_evidence_and_links.sql:73`),
so `seen.contains(&pair)` is unreachable. Documented rule (c) is silently unenforced
while costing a query and an O(n²) scan; the comment at `:194-196` concedes the
constraint prevents it.

**Fix:** detect duplicate assertions over `assertion` instead of links, or delete the
rule and the documentation that promises it. Do not leave a check that cannot fail.

#### M7. Row decoding both panics and silently zeroes provenance

`src/storage_pg.rs:466-489`

`row.get(...)` panics on a missing column or type change, while neighbouring columns use
`try_get(...).unwrap_or_default()`. The failure modes are opposite and both bad: a
migration widening `chunk_index` panics the API task, while a decode error on
`content_hash`/`embed_model`/`chunker_config`/`ingested_at` silently yields `""`/`0` —
which then feeds the unchanged-source comparison (H2).

**Fix:** decode every column with `try_get` + `map_err`, and make the legacy-NULL intent
explicit (`try_get::<_, Option<String>>(...).unwrap_or_default()` with a comment).

#### M8. Offsets from `to_lowercase` are used to slice the original bytes

`src/genealogy/parser.rs:280-285`, `:515-518`

```rust
let lower = text.to_lowercase();
let abs = search + pos;
let rest = &text[abs + marker.len()..];
```

`to_lowercase` can change byte length (for example `İ` becomes `i` + U+0307), so `abs` is
not a valid boundary in `text`; the slice can panic. `extract_name_variants` repeats the
pattern at `:516-517`.

**Fix:** use `to_ascii_lowercase()` for the ASCII page markers (length-preserving), and
keep `to_lowercase` only in the branch that genuinely matches a Unicode keyword.

#### M9. A malformed duration setting panics the process

`src/config.rs:413`

```rust
total += unit_duration.mul_f64(value);
```

`Duration::mul_f64` panics on overflow and `Add` can panic on overflow. A value such as
`REQUEST_TIMEOUT=1e300h` aborts startup with a panic instead of the promised
`ConfigError` naming the variable, taking the API server down before it can report
anything.

**Fix:** `checked_mul_f64` + `checked_add` (or `Duration::try_from_secs_f64`) and return
`ConfigError::Invalid`.

#### M10. A failed client build silently disables the configured timeout and redirect policy

`src/ollama.rs:124-128`

```rust
.build()
.unwrap_or_else(|_| reqwest::blocking::Client::new());
```

The fallback loses the timeout (reqwest's default is none) and the
`redirect(Policy::none())` guarantee documented on the type. The substituted client is
weaker than the one the caller asked for. `Client::new()` can itself panic if TLS cannot
initialize.

**Fix:** return `Result<Self, _>` and fail at startup. Never substitute a differently
configured client.

#### M11. The fingerprint read error is discarded

`src/pipeline.rs:227-229`

```rust
.map_err(|_| PipelineError::Fingerprint(format!("unchanged-source read failed for {source}")))
```

The underlying `PgStoreError` (pool exhaustion, auth failure, syntax error) is thrown
away and replaced with a message that names only the source. The variant cannot carry a
`#[source]`.

**Fix:** constrain `R::Error: std::error::Error + Send + Sync + 'static` and store it as
the source, or make `PipelineError::Fingerprint` hold a typed store error.

#### M12. RRF fusion clones every candidate, including its 768-float embedding

`src/retrieval_pg.rs:31-35`, `:64-72`

`entries.entry(..).or_insert_with(|| Entry { hit: hit.clone(), .. })` deep-clones a
`SqlHit` → `StoredChunkRow` → `StoredChunk` → `Vec<f64>` for every candidate in both
lists. With `candidate_k = 50` that is hundreds of kilobytes of vector copies per query,
and the embedding is never read by fusion.

**Fix:** store `&StoredChunkRow` (or only the fields fusion needs) in the map, and build
the final `Chunk` by consuming the owned rows.

#### M13. One INSERT round trip per chunk, with a duplicated column list

`src/storage_pg.rs:557-581`

`replace_source` executes one INSERT per chunk inside the open transaction. A 500-chunk
document is 500 round trips while the transaction holds locks and WAL on `documents`;
the 13-column statement and its parameter list are duplicated verbatim from
`write_chunk` (`:508-528`), so a column addition must be made twice.

**Fix:** one multi-row `INSERT ... SELECT * FROM unnest($1::text[], ...)`, and share the
SQL through a `const` used by both functions.

#### M14. The fingerprint ignores the embedding model and dimension

`src/fingerprint.rs:46-53`

The hash covers bytes length, bytes, and chunker config only. Migration 002 states that
a model or dimension change forces a re-embed, but nothing reads the stored `embed_model`
back. Switching to a different 768-wide model makes `ingest` report `Unchanged` and
leaves the old model's vectors in place — a silent retrieval-quality regression.

**Fix:** fold `(embed_model, embedding_dim)` into the fingerprint material, or compare
them against the stored row before returning `Unchanged`.

#### M15. Blocking filesystem and hashing work runs on the async executor

`src/api.rs:301-325`, `:344`

`create_dir_all`, the up-to-10 MiB `fs::write`, `fingerprint`, and the recursive
`remove_dir_all` all execute directly in the async handler, despite the module doc
claiming every handler moves its work onto `spawn_blocking`. Under concurrent uploads
this stalls unrelated async tasks.

**Fix:** move staging, hashing, and cleanup into the existing `spawn_blocking` closure
(or a second one).

#### M16. Fourteen configured settings are parsed but never read, and the modules they configure are unreachable

`src/config.rs:83-98` and the `Config` fields `embed_provider`, `gen_provider`,
`max_input_bytes`, `embed_workers`, `embed_retries`, `lexical_rerank`, `llm_rerank`,
`rag_llm_rerank`, `rerank_timeout`, `answerability_gate`, `fact_judge`, `rewrite_only`,
`rag_answerability_gate`, `observability_format` are referenced nowhere outside
`config.rs`. Correspondingly, `retrieval::rerank`, `retrieval::fallback`,
`retrieval::filter::apply`, `answer::query_rewrite::rewrite`, and
`answer::render::render_pages` have no production callers.

An operator setting `RAG_ANSWERABILITY_GATE=false` sees no behaviour change, and
`QUERY_REWRITE=true` does not rewrite. This is the most misleading surface in the
crate: it advertises features that do not run.

**Fix:** wire the settings into `QueryService`/`AnswerService`, or delete both the
setting and the dead module so the public API matches what the server does. If the work
is genuinely deferred, reference a tracked issue in the config documentation.

#### M17. Stringly-typed errors at most module boundaries

`src/storage_pg.rs:56-73`, `src/transaction.rs:29-30`, `src/genealogy/mod.rs:37-42`,
`src/pdf_assessment.rs:258`, `src/evaluation.rs:143-157`, `src/api.rs:68`, `:95`,
`src/query_service.rs:44`, `:54`, `:59`, `:66`

The crate depends on `thiserror` and uses it in places (`BilingualError`,
`InventoryError`, `AnswerError`, `GenealogyError::Config/Store`), but the majority of
boundaries flatten to `String` or hand-roll `Display`. `Error::source()` is therefore
always empty at those boundaries, and callers cannot distinguish a pool error from an
I/O error without parsing text — which is exactly what M1 then does.

**Fix:** `#[error(transparent)] Store(#[from] PgStoreError)` and friends, keeping the
chain. Do this together with M1 and M11; they are the same change.

#### M18. The API does not enforce `MAX_QUESTION_BYTES`

`src/api.rs:476-484`; the CLI does enforce it at `src/bin/query.rs:66-72`

A client can post several megabytes of question text (bounded only by the global body
limit) and make the server embed and forward it to the model. `MAX_INPUT_BYTES` is also
parsed but never read.

**Fix:** apply the same check in the handler and return `400 invalid_question`.

#### M19. Relationship detection matches substrings like `con` and `cha`

`src/genealogy/parser.rs:559-607`

`lower.contains("con")` matches "second" and `lower.contains("cha")` matches "Chandler",
so a note like `Notes: second marriage of An` becomes a `ParentChild` assertion. When a
line matches both sets, `is_spouse` silently wins.

**Fix:** match field labels (the text before `:`/`=`) against a small set, and emit both
kinds or an explicit ambiguity when a line matches both.

#### M20. Duplicated materialization and repeated lowercasing in the eval hot paths

`src/evaluation.rs:241-248`, `src/bilingual.rs:889-894`

`load_corpus` calls `stored_chunk(row)` twice per row (once for `chunks`, once for each
`IndexedVector`), cloning every content string twice. `evidence_recall` calls
`content.to_lowercase()` once per `(evidence, chunk)` pair inside the innermost loop,
allocating a new `String` each time; with 20 expanded chunks and several evidence
strings this is real Unicode work repeated for the same chunk.

**Fix:** zip the already-built chunks into the index; pre-lowercase chunks once per case
and group by `(source, section)`.

### Low

- **L1. `expect`/`unwrap` in library functions.** `src/retrieval/fusion.rs:33`
  (`.expect("id present in by_id")`), `src/answer/citation.rs:146-147`
  (`chars().last().unwrap()` after an empty check), `src/inventory.rs:309`, `:368`.
  All are provably safe today, but the skill reserves `unwrap` for tests, impossible
  failures, or cases the alternatives cannot express. Prefer `let Some(..) else`,
  `unwrap_or_default`, or a `filter_map` that skips instead of panicking.
- **L2. Nested `Result` forces double validation.** `src/storage.rs:142-152` returns
  `Result<Result<(), E>, DimensionMismatchError>`, so callers write `unwrap().unwrap()`
  (`src/storage.rs:229-231`), and `PgStore::write_chunk` validates a second time
  (`src/storage_pg.rs:503`). Give the trait `type Error: From<DimensionMismatchError>`.
- **L3. Stale documentation.** `src/storage.rs:5-8` and `:123-124` still say the future
  database client will implement `DocumentStore`; `PgStore` implements it today
  (`src/storage_pg.rs:492`). `src/pipeline.rs:9` lists "compute the source fingerprint"
  as a pipeline step, but `ingest_source` takes the hash from the caller. The doc block
  at `src/pdf_assessment.rs:104-106` says any `## Trang PDF` prefix that does not parse
  is ambiguous, but the code only matches the prefix with a trailing space (`:116`), so
  `## Trang PDF: 3` is silently treated as absent.
- **L4. Suppressed or dead statements.** `src/genealogy/parser.rs:36`
  `#[allow(dead_code)]` (the skill prefers `#[expect]` with a reason, and the two
  variants it covers are never constructed); `src/genealogy/reimport.rs:171`
  `let _ = assertion_id;`; `src/genealogy/migrate.rs:90` `let _ = conn;`.
- **L5. Hand-rolled SHA-256.** `src/inventory.rs:373-481` implements FIPS 180-4 to avoid
  a dependency. It is tested against the empty and `"abc"` vectors, but crypto code is
  the canonical "use the well-known crate" case; `sha2` would remove ~100 lines and the
  maintenance risk. If the no-new-dependency rule is deliberate, keep the comment that
  says so.
- **L6. Documentation coverage.** 170 public items (fields, constants, modules) have no
  `///`, and the crate does not enable `#![deny(missing_docs)]`; `rustdoc` also reports
  missing `# Errors` sections on most fallible public functions. The skill asks for
  docs on public functions, structs, traits, enums, and constants.
- **L7. Test quality gaps.** `src/pdf_assessment.rs:505`
  `assert_eq!(render_markdown(&report), render_markdown(&report))` cannot fail;
  `src/genealogy/eligibility_tests.rs:89-99` mutates the same input as the preceding
  test and asserts the same reason, so it cannot fail independently. Several tests
  assert multiple behaviours (for example `src/retrieval/vector.rs:93-97`). None of the
  parser's error paths (missing input, `Trang:` with no digits) is covered.
- **L8. A full upload copy.** `src/api.rs:282` calls `bytes.to_vec()` on an owned
  `Bytes` that is only read afterwards; store `Bytes` and pass `&bytes`.
- **L9. O(citations × passages) with two allocations per comparison.**
  `src/answer/service.rs:176-209` re-normalizes passage sources for every comparison
  (`src/answer/citation.rs:34-36`). Small at current budgets; normalize once.
- **L10. Duplicate I/O and a TOCTOU window in ingest.** `src/bin/ingest.rs:58-62` reads
  the file to hash it, then `ingest_source` re-reads the path through the loader. Hashing
  the bytes the loader actually consumed would remove both.

## What is working well (preserve these)

- **No `unsafe`, no SQL interpolation.** Every query is parameterized, and dynamic
  identifiers that reach SQL (`normalize_fts_config`) are mapped from a closed list to
  `&'static str`.
- **Effect ordering is enforced and tested.** The dimension guard, the ingest gate, and
  the fail-before-write paths assert zero effects, not just an error
  (`src/storage.rs:205-234`, `src/transaction.rs:259-269`, `src/pipeline.rs:425-481`).
- **Transaction scoping is correct.** `replace_source` borrows a pooled connection,
  opens one transaction, propagates with `?`, and releases the connection promptly; no
  connection is held across embedding or HTTP work.
- **Configuration is injectable.** `Settings::from_lookup` / `env_or` / `env_int` take a
  lookup closure, which is why 20+ config tests need no `unsafe` env mutation.
- **Ports over concretions.** `Embedder`, `Generator`, `Transport`, `RetrievalSource`,
  `FingerprintReader`, `SourceReplacer` are generic at the service layer; `dyn` appears
  only at the composition/registry boundary, which is the right place.
- **The prompt-injection boundary is explicit and tested.** Trusted instructions are
  delimited from untrusted context, and a test asserts injected marker text stays inside
  the context block (`src/answer/prompt.rs:163-168`).
- **Deterministic ordering everywhere.** Fusion, vector, FTS, and fallback sort by score
  descending then id ascending, and the tie-break is tested.
- **API errors do not leak internals.** Handlers log detail server-side and return a
  stable `{ok, error, message}` shape; upload defences are layered (`DefaultBodyLimit`,
  size check, extension allowlist, `file_name()` sanitization, empty-file rejection).
- **Tests are descriptive and hermetic.** 278 library tests pass in ~0.01 s with no
  database; names state behaviour (`d2_pins_the_baseline_top1_and_fills_to_four`,
  `similarity_floor_applies_to_vector_hits_only`), and DB-free ports keep them fast.

## Chapter scorecard

| Chapter                     | Rating       | Note                                                                                                                                                                                             |
| --------------------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1. Idioms & borrowing       | Good         | Borrowed parameters throughout; clones are usually deliberate. A few deep clones remain in fusion/rerank/vector.                                                                                 |
| 2. Clippy & lints           | Good         | Clean with `-D warnings`; no `#[allow]` except one `dead_code`. Missing `#[expect]`/`#![deny(missing_docs)]`.                                                                                    |
| 3. Performance              | Needs work   | Per-request clients, whole-corpus parse per query, per-row queries, per-chunk INSERT, per-element vector formatting. Nothing is measured, so these are structural rather than profiled findings. |
| 4. Error handling           | Weakest axis | `thiserror` is present but bypassed at many boundaries; string matching drives HTTP status.                                                                                                      |
| 5. Testing                  | Good         | 278 fast tests, descriptive names, effect-level assertions. Some vacuous/duplicate tests and thin error-path coverage.                                                                           |
| 6. Generics & dispatch      | Good         | Static dispatch in services, `dyn` only at the boundary.                                                                                                                                         |
| 7. Type state               | Mixed        | `ReviewStatus` and `ParseOutcome` are enums, but `EligibilityDecision.decision` and `ReviewOutcome` carry raw status strings, so invalid combinations are representable.                         |
| 8. Comments & docs          | Mixed        | Excellent "why" comments in places; several stale docs and 170 undocumented public items.                                                                                                        |
| 9. Pointers & thread safety | Good         | No raw pointers; `Arc` used for shared configuration/store; blocking work is mostly moved off the runtime.                                                                                       |

## Suggested order of work

1. **H1–H3 first.** ✅ H1/H2 were fixed on `main` (`17c93ab`); H3 is fixed on
   this branch with the reimport regression test.
2. **One typed-error pass (M1, M11, M17).** Introduce `#[from]`/`#[source]` errors at the
   storage, pipeline, genealogy, PDF, and evaluation boundaries, then classify HTTP
   status from types. This removes a whole class of string parsing.
3. **API lifecycle (M2, M3, M15, M18).** Build clients once, add a corpus projection
   without embeddings, move filesystem work into `spawn_blocking`, enforce
   `MAX_QUESTION_BYTES`.
4. **Database round trips (M4, M5, M13).** One join for citations, one validation pass,
   one multi-row insert.
5. **Cleanup (M16, M19, M14, L1–L10).** Decide which configured features are real; wire
   or delete them. Add `#![deny(missing_docs)]` and fix the stale docs last.

## Validation performed

```text
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
```

- `cargo fmt --check`: clean.
- `cargo clippy ... -D warnings`: clean (`rag-foundation` and all targets).
- `cargo test --locked` with the local PostgreSQL container: **all targets pass**,
  including the extended `tests/genealogy_repairs.rs` (the pre-existing
  hash-change reopen test still passes). Library tests: **305 passed, 0 failed**.
- Acceptance crate compilation: `acceptance/ingest_cli` builds.
- Additional targeted lints (`redundant_clone`, `needless_collect`, `map_unwrap_or`,
  `large_enum_variant`, `missing_errors_doc`) surfaced only eight library warnings, one
  redundant clone in a test helper, and two in binaries/tests — all listed above or
  immaterial.

The database-backed integration and acceptance suites were run against the local
`docker compose` PostgreSQL instance. Findings that depend on schema behaviour were
checked against the migration files directly (`genealogy/migrations/0003`, `0004`,
`migrations/005`).
