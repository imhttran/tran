# RUST-007D — `cmd/expbilingual` contract, capability inventory, and invocation mapping

Frozen reference: `cmd/expbilingual/main.go` at repository commit `acd1fb9`.
Rust port: `rust/src/bilingual.rs` (module) and `rust/src/bin/expbilingual.rs`
(binary). Differential proof: `rust/acceptance/bilingual_parity`.

**The harness is evaluation-only.** It must not modify production retrieval or
ingestion. The port adds a read operation to `rust/src/storage_pg.rs` and a new
module and binary; it does not change `rust/src/retrieval/`,
`rust/src/retrieval_pg.rs`, `rust/src/pipeline.rs` or `rust/src/ingest.rs`, and
it never writes to the `documents` table.

## 1. Frozen Go contract

### Environment

| Variable | Default | Parsing (Go) |
| -------- | ------- | ------------ |
| `ARM` | `all` | `envOr`, then `strings.ToUpper(strings.TrimSpace(...))`. `ALL` runs `A,D1,D2`; any other value is `strings.Split(arm, ",")` with **no** per-element trim. |
| `REPEATS` | `1` | `envInt`: blank, any non-digit (including a sign or `.`), or a value below 1 falls back. |
| `DIAG` | off | `envOr("DIAG","") == "1"` (trimmed). |
| `EVAL_DATASET` | `evals/retrieval-vi-en-expanded.json` | `envOr` (trimmed, blank falls back). |
| `OLLAMA_EMBED_MODEL` | `nomic-embed-text` | `envOr`. |
| `OLLAMA_CHAT_MODEL` | `llama3.2:3b` | `envOr`. |
| `QUERY_CACHE` | `docs/rq-006-query-cache.json` | `envOr`. |
| `GEN_TIMEOUT` | `60` (seconds) | `envInt`. |
| `MIN_SIMILARITY` | `0.6` | `config.Load` (`cfg.MinSimilarity`). |
| `DATABASE_URL`, `OLLAMA_URL`, `REQUEST_TIMEOUT` | config defaults | `config.Load`; the Ollama client's HTTP timeout is `REQUEST_TIMEOUT`, and each generation call is additionally bounded by `GEN_TIMEOUT`. |

Harness-hardcoded options: `candidateK=4`, `finalK=3`, `expandLimit=20`,
`minSim=cfg.MinSimilarity`. Fusion of all rankings uses `FuseRankings(..., 1000)`
followed by `DeduplicateSections`.

Pinned decoding options (`decoding` map): `temperature 0`, `seed 0`,
`num_predict 64`. Generation request: `{"model","prompt","stream":false,"options":decoding}`
posted to `/api/generate`; the response field is `response`, trimmed.

### Prompts

* `expansionPrompt(q)`: `Rewrite the user's question into a short bilingual search query that contains key terms in BOTH English and Vietnamese, so it retrieves matching passages in either language. Return only the query, no quotes or explanation.\n\nQUESTION:\n<q>\n\nQUERY:`
* `translationPrompt(q, lang)`: `Translate the following question into <lang>. Return only the translation, with no quotes, labels, or explanation.\n\nQUESTION:\n<q>\n\nTRANSLATION:` (`lang` is `Vietnamese` or `English`).

### Output line formats (stdout, in order)

```
HEADER arm=%s dataset=%s embed=%s chat=%s floor=%.2f repeats=%d gen_options=%v
CACHE model mismatch (cached=%s requested=%s); starting fresh        (only on mismatch)
GEN id=%s variant=%s source=cache|model                                (exp, vi, en per generating arm)
DIAG arm=%s rep=%d id=%s kind=%s guardOK=%v candidates=%d rejected=%v  (DIAG=1)
DIAG   #%d sim=%.4f %s / %s                                            (DIAG=1, per ranked section)
CASE arm=%s rep=%d id=%s kind=%s r1=%.2f r4=%.2f evidence=%.2f %s
FOOTER wall=%s generated_queries=%d cache=%s
```

`gen_options` prints as `map[num_predict:64 seed:0 temperature:0]` (fmt sorts
map keys). `wall` is `time.Since(start).Round(time.Millisecond)` rendered by
`Duration.String` (`512ms`, `1.5s`, `1m2.345s`). `start` is taken after the
cache is loaded. The HEADER is printed before the dataset is read, so a dataset
error follows the HEADER.

Loop order: arms → repeats (1..REPEATS) → cases (dataset order).

### Arm semantics

Per case: embed the question; `HybridRetrieve` gives the original vector
ranking (similarity floor applied to vector hits only) and keyword ranking
(unfloored). For arms `B`, `C`, `D1`, `D2`, `E` the harness generates (in
order) `exp`, then `vi`, then `en`, embeds them, and retrieves with the trimmed
variant as the keyword query.

| Arm | Rankings fused | Guard | Final ranking |
| --- | -------------- | ----- | ------------- |
| `A` | orig.V, orig.K | always true | fused |
| `B` | orig + exp.V, exp.K | always true | fused |
| `C` | orig + vi.V, vi.K, en.V, en.K, exp.V, exp.K | always true | fused |
| `D1` | as `C` | orig.V non-empty | fused |
| `D2` | as `C` | orig.V non-empty | guard ok: baseline-first; guard fails: **empty** (even for answerable cases) |
| `E` | as `C` | ≥ 2 of {orig.V, vi.V, en.V} non-empty | fused |
| other (e.g. `" D1"`) | orig only, no generation | true | fused |

Then, for every arm: if the case is unanswerable (`expected` empty) and the
guard failed, the ranking is empty. Note that the guard empties `D1` and `E`
rankings **only for unanswerable cases**; `D2` empties on any guard failure.

Baseline-first (`D2`): `DeduplicateSections(FuseRankings(orig, 1))`; if empty,
the full fused list; otherwise fill to 4 from the fused list skipping sections
already present.

### Metrics

* `recallAtK(expected, ranking, k)`: 0 when `expected` is empty; `k` clamped to
  the ranking length; found expected `(source, section)` pairs / expected count.
  `r1` = k 1, `r4` = k 4.
* Rejection label: `n/a` (answerable), `reject` (unanswerable, empty ranking),
  `FALSE_POSITIVE` (unanswerable, non-empty ranking).
* Evidence: 0 for unanswerable cases; otherwise expand the first `finalK=3`
  ranked sections with `SectionChunksFiltered(keys, 20, Filter{})` and compute
  `evidenceRecall`: evidence strings found by case-insensitive substring in a
  chunk of the same `(source, section)` / total evidence strings (0 when none).
* DIAG `sim`: the fused document's similarity, i.e. the maximum similarity over
  all occurrences of that row across the fused rankings (0 for keyword-only).

### Query cache

```json
{
  "model": "<chat model>",
  "entries": {
    "<case id>": {
      "question": "<question>",
      "variants": {
        "en": "...",
        "exp": "...",
        "vi": "..."
      }
    }
  }
}
```

* Keyed by case id; an entry is valid only if its `question` equals the case
  question; a variant hits only if non-blank after trimming (the untrimmed
  value is used).
* Load: an unreadable/malformed file or missing `entries` → fresh cache; a
  non-empty cached `model` different from `OLLAMA_CHAT_MODEL` prints the
  mismatch line and starts fresh.
* A model call stores the trimmed response; a question change replaces the
  entry. The generated-query counter counts model calls only.
* Save: `json.MarshalIndent(cache, "", "  ")` with `model` set to the chat
  model, written after the run, no trailing newline, errors ignored. Map keys
  are sorted and `<`, `>`, `&`, U+2028, U+2029 are `\u`-escaped.

## 2. Capability inventory

| Capability | Status | Evidence | Provided by |
| ---------- | ------ | -------- | ----------- |
| Configuration (`ollama_url`, `database_url`, `min_similarity`, `embed_dim`, `request_timeout`) | EXISTS | `rust/src/config.rs` (`Config::load`) | — |
| Postgres store + corpus-scoped vector / keyword search | EXISTS | `rust/src/storage_pg.rs` (`PgStore::connect`, `vector_search`, `keyword_search`) | — |
| RRF fusion ordered by fused score, best rank, row id | EXISTS | `rust/src/retrieval_pg.rs:20` (`fuse`) | — |
| Ollama JSON client | EXISTS | `rust/src/ollama.rs` (`OllamaClient::new`, `post_json`, `HttpTransport`) | — |
| Embedder with dimension check | EXISTS | `rust/src/embedding.rs` (`OllamaEmbedder::new().with_embed_dim()`) | — |
| FTS configuration normalization | EXISTS | `rust/src/storage_pg.rs` (`normalize_fts_config`) | — |
| `EMBEDDING_DIM` | EXISTS | `rust/src/storage.rs` | — |
| Differential acceptance precedent (stub Ollama, isolation check, run-owned cleanup) | EXISTS | `rust/acceptance/eval_parity/tests/eval_parity.rs` | — |
| Case type with `kind` and `id` | PARTIAL | `evaluation::EvalCase` has neither field (`rust/src/evaluation.rs`) | bilingual module (`BilingualCase`) |
| DIAG similarity passthrough | PARTIAL | `retrieval_pg::fuse` returns `Candidate::new(chunk, score)` and drops similarity and row id | bilingual module (`fuse_sections` reconstructs the max similarity from its own `SqlHit` inputs; `retrieval_pg` unchanged) |
| Per-call generation timeout | PARTIAL | `OllamaClient` timeout fixed at construction | bilingual module / binary (dedicated `HttpTransport` + `OllamaClient` built with `min(GEN_TIMEOUT, REQUEST_TIMEOUT)`) |
| Pinned generation options | MISSING | `generation::GenerateRequest` has no `options` (`rust/src/generation.rs`) | bilingual module (`PinnedGenerateRequest`) |
| Section dedup on `source|section` | MISSING | `retrieval::dedup` keys on `source::section::chunk_index` (`rust/src/retrieval/dedup.rs`, `rust/src/retrieval/mod.rs`) | bilingual module (`dedup_sections`) |
| Section-chunks read (Go `SectionChunksFiltered`, `internal/retrieval/retrieval.go:415`) | MISSING → added | `PgStore::section_chunks`, `section_cap`, `SectionKey` | `rust/src/storage_pg.rs` |
| Query cache load/save in the Go format | MISSING | — | bilingual module (`QueryCache`, `go_json_string`) |
| recall@k at explicit k, evidence recall | MISSING | `evaluation.rs` exposes only `score_sections` / `score_ranking` | bilingual module |
| Go duration rendering for `wall` | MISSING | — | bilingual module (`go_duration_ms`) |
| Live Ollama serving `nomic-embed-text` and `llama3.2:3b` | EXISTS | live run recorded in `RUST-007D-live-model-evidence.md` and `live-007d/` | — |

## 3. Divergences and assumptions

1. **Corpus scoping.** Go passes `Filter{}` so its searches and expansion are
   unfiltered (`internal/retrieval/filter.go`). Every `PgStore` read is
   corpus-scoped. The Rust command scopes all reads to `RAG_CORPUS` (default
   `trusted`, as `rust/src/bin/eval.rs`). Consequence: parity holds only on a
   database holding a single corpus; the acceptance crate asserts that before
   it runs.
2. **FTS language.** Go passes an empty `PipelineOptions.Language`, so every
   keyword search uses `english`. The Rust command passes `""` to
   `keyword_search` regardless of `LANGUAGE`.
3. **Chat-model default.** `config::DEFAULT_CHAT_MODEL` is `qwen3.8:27b-mlx`;
   the harness default is `llama3.2:3b`. The command reads `OLLAMA_CHAT_MODEL`
   and `OLLAMA_EMBED_MODEL` directly with the harness defaults, not through
   `Config`.
4. **Sorted cache keys.** Go `json.MarshalIndent` sorts map keys; the Rust
   cache uses `BTreeMap` and a Go-compatible string encoder, so the file is
   reproduced byte for byte (tested against `docs/rq-006-query-cache.json`).
5. **Similarity reconstruction key.** `retrieval_pg::fuse` drops the row id, so
   the DIAG similarity is matched by `(source, section, chunk_index)`. Two rows
   sharing that triple in one corpus would share the maximum similarity; ingest
   replaces a source atomically, so this does not occur in a well-formed corpus.
6. **Unused Go expansion.** Go's `HybridRetrieve` also fuses and expands each
   query, but the harness uses only its vector and keyword rankings; the Rust
   command skips that unobservable work.
7. **Early database check.** Go fails at `pgx.Connect` before loading the
   cache; the Rust pool connects lazily, so the command issues `SELECT 1`
   (`PgStore::ping`) at the same point.
8. **Cache write failure.** Go ignores it silently; Rust also continues but
   prints a warning on stderr (stdout is unchanged).

## 4. Invocation mapping (Go → Rust)

All Rust invocations run from the repository root with the same environment as
the Go command. The binaries are those declared in `rust/Cargo.toml`.

| Go command | Rust equivalent | Notes |
| ---------- | --------------- | ----- |
| `go run ./cmd/ingest <file>` | `cargo run --manifest-path rust/Cargo.toml --bin ingest -- <file>` | `RAG_CORPUS` default `trusted`. |
| `go run ./cmd/rag "question"` | `cargo run --manifest-path rust/Cargo.toml --bin query -- "question"` | `RAG_CORPUS` default `trusted`. |
| `go run ./cmd/eval` | `cargo run --manifest-path rust/Cargo.toml --bin eval` | Parity: `rust/acceptance/eval_parity`. |
| `go run ./cmd/expbilingual` | `cargo run --manifest-path rust/Cargo.toml --bin expbilingual` | Parity: `rust/acceptance/bilingual_parity`. |

Example:

```sh
ARM=all REPEATS=1 DIAG=1 \
EVAL_DATASET=evals/retrieval-vi-en-expanded.json \
QUERY_CACHE=docs/rq-006-query-cache.json \
RAG_CORPUS=trusted \
cargo run --manifest-path rust/Cargo.toml --bin expbilingual
```

### Environment read by the Rust `expbilingual`

| Variable | Default | Source |
| -------- | ------- | ------ |
| `ARM` | `all` (→ `A,D1,D2`) | harness |
| `REPEATS` | `1` | harness |
| `DIAG` | off (`1` enables) | harness |
| `EVAL_DATASET` | `evals/retrieval-vi-en-expanded.json` | harness |
| `QUERY_CACHE` | `docs/rq-006-query-cache.json` | harness |
| `GEN_TIMEOUT` | `60` seconds | harness |
| `OLLAMA_EMBED_MODEL` | `nomic-embed-text` | harness |
| `OLLAMA_CHAT_MODEL` | `llama3.2:3b` | harness |
| `MIN_SIMILARITY` | `0.6` | `Config` |
| `DATABASE_URL` | `postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable` | `Config` |
| `OLLAMA_URL` | `http://localhost:11434` | `Config` |
| `EMBED_DIM` | `768` | `Config` |
| `REQUEST_TIMEOUT` | `5m` | `Config` (embedding timeout; caps `GEN_TIMEOUT`) |
| `RAG_CORPUS` | `trusted` | Rust only (see divergence 1) |

`Config::load` validates every other configuration variable it reads, as the
Go `config.Load` does; an invalid value exits non-zero naming the variable.

### Divergences relevant to parity runs

* **`RAG_CORPUS` is Rust-only.** Go searches unfiltered; run both on a database
  that holds only the corpus under test, and point `RAG_CORPUS` at it.
* **English FTS for every keyword search**, regardless of `LANGUAGE`, in both.
* **Harness chat-model default `llama3.2:3b`** differs from
  `config::DEFAULT_CHAT_MODEL` (`qwen3.8:27b-mlx`); set `OLLAMA_CHAT_MODEL`
  explicitly when comparing with other commands.

## 5. Verification

* Unit tests: `rust/src/bilingual.rs` (env rules, prompts, pinned request,
  cache format/round-trip/mismatch, arm composition, D2, guards, metrics,
  section dedup, output lines, duration rendering) and `rust/src/storage_pg.rs`
  (`section_cap`, empty-key short-circuit).
* Database tests: `rust/tests/section_chunks_pg.rs` (cap, order, corpus scope)
  and `rust/tests/expbilingual_binary.rs` (missing dataset, unreachable
  database, unreachable model endpoint).
* Differential: `rust/acceptance/bilingual_parity` runs the real Go
  `cmd/ingest` + `cmd/expbilingual` and the Rust `expbilingual` over all 39
  cases for `A`, `D1`, `D2` (fresh cache) and `B`, `C`, `E` (cache hits), and
  compares output within the documented tolerances and both cache files byte
  for byte.

The live-model groundedness and citation criterion is **not** met by any of
the above; see `RUST-007D-live-model-evidence.md`.
