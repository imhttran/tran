# Configuration

All three commands read the same settings from the environment. The defaults
work with the bundled `docker-compose.yml` and a local Ollama.

`.env.example` lists every setting; copy it to `.env` and load it into your
shell (The binaries read the process environment, not `.env`, on their own):

```bash
cp .env.example .env
set -a; source .env; set +a
```

| Variable                  | Default                                                 | Used by     |
| ------------------------- | ------------------------------------------------------- | ----------- |
| `OLLAMA_URL`              | `http://localhost:11434`                                | all         |
| `OLLAMA_EMBED_MODEL`      | `nomic-embed-text`                                      | all         |
| `OLLAMA_CHAT_MODEL`       | `qwen3.8:27b-mlx`                                       | rag, eval   |
| `DATABASE_URL`            | `postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable` | all         |
| `EMBED_DIM`               | `768`                                                   | all         |
| `EMBED_PROVIDER`          | `ollama`                                                | all         |
| `GEN_PROVIDER`            | `ollama`                                                | all         |
| `CHUNK_SIZE`              | `50`                                                    | ingest      |
| `CHUNK_OVERLAP`           | `20`                                                    | ingest      |
| `CORPUS_LANGUAGE`         | _(empty — baseline `english` FTS)_                      | ingest, rag |
| `TOP_K`                   | `4`                                                     | rag, eval   |
| `FINAL_K`                 | `3`                                                     | rag, eval   |
| `EXPAND_LIMIT`            | `20`                                                    | rag, eval   |
| `MIN_SIMILARITY`          | `0.6`                                                   | rag, eval   |
| `CONTEXT_BUDGET`          | `0` (disabled)                                          | rag         |
| `MAX_QUESTION_BYTES`      | `0` (disabled)                                          | rag         |
| `MAX_INPUT_BYTES`         | `0` (disabled)                                          | rag         |
| `REQUEST_TIMEOUT`         | `5m`                                                    | all         |
| `RAG_ANSWERABILITY_GATE`  | `true`                                                  | rag         |
| `RAG_CITATION_VALIDATION` | `false`                                                 | rag         |
| `QUERY_REWRITE`           | `true`                                                  | rag         |
| `QUESTION`                | _(none — pass it as an argument, or type it)_           | rag         |

`all` = every command; `rag` = `src/bin/query.rs` only; `eval` = `src/bin/eval.rs` only;
`ingest` = `src/bin/ingest.rs` only; `rag, eval` = both commands.

### Post-gap-closure settings

The settings below were added by the RAG gap-closure work (RAG-006…RAG-015).
Each defaults to the pre-change behaviour, so an unset value changes nothing:

- **`CORPUS_LANGUAGE`** (`ingest`, `rag`) — BCP-47 tag selecting the PostgreSQL
  full-text search configuration for keyword retrieval and stored per document.
  Empty (default) keeps `english`; a supported language uses its configuration;
  an unsupported one falls back to `simple` (see `src/retrieval` normalization).
- **`CONTEXT_BUDGET`** (`rag`) — byte-based cap on the context sent to the model.
  `0` (default) keeps the chunk-count behaviour. See
  [`src/answer/context_budget.rs`](../../src/answer/context_budget.rs).
- **`RAG_CITATION_VALIDATION`** (`rag`) — validates the model's `[source - section]`
  citations against the retrieved documents and repairs an answer whose citations
  do not resolve (one bounded re-prompt, then strip). Off by default; when off the
  answer path is unchanged. See [`src/answer/citation.rs`](../../src/answer/citation.rs).
- **`MAX_QUESTION_BYTES` / `MAX_INPUT_BYTES`** (`rag`) — fail-fast size limits for
  the question and the assembled retrieved context. `0` (default) disables each
  check; an oversized input errors before any model call, naming the variable.
- **`RAG_ANSWERABILITY_GATE`** (`rag`) — when enabled (default), a question with no
  budgeted passage returns the baseline refusal without a generation call. Set it
  to `false` to generate anyway.
- **`QUERY_REWRITE`** (`rag`) — when enabled (default), the chat model rewrites the
  question into a retrieval query. The answer prompt and citations still use the
  original question. A blank or failed rewrite falls back to the original.
- **`EMBED_PROVIDER` / `GEN_PROVIDER`** (`all`) — provider-registry selectors
  (`ollama` by default), resolved through `provider::embedder` / `provider::generator`
  in [`src/provider.rs`](../../src/provider.rs). An **unregistered name fails fast at
  `Config::load`** with an error naming the variable and the value — it is never
  silently ignored. Only `ollama` is registered today; adding another provider means
  registering a constructor in `src/provider.rs`. The current command binaries still
  construct the Ollama client directly, so the selector is validated but does not yet
  switch the active client.

`EMBED_DIM` must match the stored `documents.embedding` column. The guard
`storage::validate_embedding_dim` fails fast otherwise and points at
[`docs/operations/embedding-dimension.md`](../operations/embedding-dimension.md),
the operator-run procedure for changing the dimension; changing it also
requires re-ingesting the corpus.
