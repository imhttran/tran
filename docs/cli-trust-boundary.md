# CLI trust boundary

The command-line tools run in the operator's trust domain. The API authorization
policy does not apply to them, and they never call it.

## What the CLI is

The CLI binaries (`ingest`, `query`, `eval`, and `admin`) are operator tools. The
operator starts them from a shell with operator credentials. A process that can
run a binary already has the operator's file, network, and database access. So
the CLI has no separate identity token and no separate RBAC check.

## What trusts the CLI, and what does not

- PostgreSQL trusts the CLI. The CLI connects with `DATABASE_URL` and the
database role in that URL. The CLI reads and writes documents, chunks, users,
role assignments, corpus grants, and audit events directly.
- Ollama trusts the CLI. The CLI calls the model with `OLLAMA_URL` directly for
  embeddings (`OLLAMA_EMBED_MODEL`) and generation (`OLLAMA_CHAT_MODEL`).
- The Rust API does not trust the CLI as a caller identity. Requests to the API
  are authorized only by the OIDC token path in `src/oidc.rs` and the policy in
  `src/identity.rs`.
- The Next.js proxy does not trust the CLI. It forwards only a session-derived
  bearer credential.

## What the CLI never does

- The CLI never obtains roles from OIDC tokens. It does not read `AUTH_ISSUER`,
  `AUTH_AUDIENCE`, `JWKS_URL`, `JWKS_CACHE_TTL_SECS`, or
  `AUTH_CLOCK_SKEW_SECS`.
- The CLI never calls the API authorization path: no `validate_token`, no
  `identity::decide`, no bearer token, no rate limiting.
- CLI actions are therefore outside the API RBAC boundary. They are logged by
  the shell and by the database, not by the API.

## Initial-admin bootstrap

`src/bin/admin.rs` is the only supported way to create the first admin. It is an
explicit operator action with no configuration switch and no HTTP route:

```
cargo run --locked --bin admin -- bootstrap --subject <sub> --issuer <issuer> [--actor <actor>]
```

The command requires `--subject` and `--issuer` and refuses to run without them.
In one transaction it inserts the `users` row, inserts the `admin` row in
`role_assignments`, and appends a `bootstrap.initial_admin` row to `audit_events`.
It fails with an error when an active admin already exists, so the bootstrap
cannot be replayed to mint a second initial admin.

Later role changes use the same audited path:

```
cargo run --locked --bin admin -- grant  --subject <sub> --role viewer|editor|admin [--actor <actor>]
cargo run --locked --bin admin -- revoke --subject <sub> --role viewer|editor|admin [--actor <actor>]
```

Each grant or revoke writes the `role_assignments` change and the `audit_events`
row in one transaction.

## Configuration keys

| Key | Used by | Meaning |
| --- | --- | --- |
| `DATABASE_URL` | CLI, API | PostgreSQL DSN with operator credentials |
| `OLLAMA_URL` | CLI, API | Model endpoint |
| `OLLAMA_EMBED_MODEL` | CLI, API | Embedding model |
| `OLLAMA_CHAT_MODEL` | CLI, API | Generation model |
| `AUTH_ISSUER` | API only | Expected OIDC issuer |
| `AUTH_AUDIENCE` | API only | Expected OIDC audience |
| `JWKS_URL` | API only | JWKS document endpoint |
| `JWKS_CACHE_TTL_SECS` | API only | JWKS cache lifetime |
| `AUTH_CLOCK_SKEW_SECS` | API only | Allowed clock skew |
| `AUTH_RATE_LIMIT` | API only | Requests per subject per 60 seconds |

## File paths

- `src/bin/admin.rs` — initial-admin bootstrap and role commands
- `src/bin/ingest.rs`, `src/bin/query.rs`, `src/bin/eval.rs` — data and model CLI
- `src/identity.rs` — central permission policy (API only)
- `src/oidc.rs` — OIDC verification adapter (API only)
- `src/identity_store.rs`, `src/identity_store_pg.rs` — transactional, audited identity store
- `migrations/006_identity.sql` — `users`, `role_assignments`, `corpus_grants`, `audit_events`
- `src/config.rs` — configuration keys above
