# Frozen Go reference outputs

These files are captured from the pinned Go reference (`d72fb398e8959eea83ed24be03f6df8cdeca86a2`)
before RUST-007C removed the Go implementation. They are the expected outputs for
the Rust-only acceptance crates and must never be regenerated from Rust.

| File | Source |
| --- | --- |
| `eval_k3_go_stdout.txt` | `cmd/eval` with `TOP_K=3`, `FINAL_K=3`, deterministic embeddings |
| `eval_k4_go_stdout.txt` | `cmd/eval` with `TOP_K=4`, `FINAL_K=4`, deterministic embeddings |
| `k3_rankdump_go_stdout.txt` | `tools/rankdump` with `TOP_K=3`, `FINAL_K=3`, same corpus |
| `bilingual_go_all_stdout.txt` | `cmd/expbilingual` `ARM=all`, empty cache, scripted generator |
| `bilingual_go_all_cache.json` | Cache written by the `ARM=all` run |
| `bilingual_go_rest_stdout.txt` | `cmd/expbilingual` `ARM=B,C,E`, cache seeded from the first run |
| `bilingual_go_rest_cache.json` | Cache rewritten by the `ARM=B,C,E` run |

Captured on 2026-10-19 with the dedicated PostgreSQL test database and the
deterministic Ollama server embedded in the acceptance tests. The bilingual
outputs contain a dynamic `wall=` duration and a scratch `cache=` path; the
Rust-only tests normalize both before comparing.

After RUST-007C, the tests ingest with the production Rust `ingest` binary and
compare the production Rust commands against these frozen outputs. No expected
value is produced by Rust.
