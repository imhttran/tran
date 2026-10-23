# Identity, RBAC, and Operations

This page describes the application identity model, the role matrix, initial
setup, account and role administration, session handling, audit rules, backup
and migration safety, and recovery. It applies to the Rust Axum API and the
Next.js browser UI.

## Identity model

- A **user** is an authenticated application account.
- A **person** is a cataloged genealogy subject. A user and a person are not
  automatically the same entity.
- Authentication is standards-based OIDC verified by the Rust API. The API
  verifies the token issuer, audience, signature against a cached JWKS,
  expiration, and subject. A verification error is a rejection, never anonymous
  access.
- Accounts bind to an immutable provider subject, never to an email address.
- There is no authentication-bypass switch. The CLI remains a local operator
  interface and is not protected by browser RBAC.

Required API environment:

| Variable | Purpose |
| --- | --- |
| `AUTH_ISSUER` | Expected OIDC issuer. |
| `AUTH_AUDIENCE` | Expected token audience. |
| `JWKS_URL` | JWKS endpoint used for signature verification. |
| `AUTH_CLOCK_SKEW_SECS` | Optional clock-skew allowance. |
| `JWKS_CACHE_TTL_SECS` | Optional JWKS cache lifetime. |
| `AUTH_RATE_LIMIT` | Per-subject, per-operation limit per 60 seconds; reads (account, document status, genealogy) get 5x. |
| `DATABASE_URL` | PostgreSQL/pgvector connection string. |
| `GENEALOGY_DATABASE_URL` | Separate genealogy database; no fallback to `DATABASE_URL`. |
| `HOF_THIRD_BRANCH_KEY` | Verified third-branch identifier used by Head of Family writes. |

Browser sign-in environment:

| Variable | Purpose |
| --- | --- |
| `OIDC_ISSUER` | Provider issuer used by the login/callback routes. |
| `OIDC_CLIENT_ID` | Registered OIDC client id. |
| `OIDC_CLIENT_SECRET` | Optional confidential-client secret; server-side only. |
| `OIDC_REDIRECT_URI` | Registered callback URL, for example `https://host/api/auth/callback`. |
| `OIDC_AUTHORIZATION_ENDPOINT` | Optional explicit authorization endpoint. |
| `OIDC_TOKEN_ENDPOINT` | Optional explicit token endpoint. |
| `OIDC_END_SESSION_ENDPOINT` | Optional provider logout endpoint. |
| `OIDC_SCOPES` | Optional scope list; default `openid profile email`. |
| `OIDC_COOKIE_SECURE` | Set `false` only for local HTTP E2E; production remains Secure. |

The browser never receives a token, subject, email, issuer, JWKS value, corpus
grant list, or database URL. The HttpOnly `rag_session` cookie carries the
credential server-to-server; `rag_csrf` supports the double-submit check.

## Role matrix

| Capability | Viewer | Editor | Admin |
| --- | --- | --- | --- |
| Read permitted documents, search, and Q&A | Yes | Yes | Yes |
| Read approved family tree and Head of Family | Yes | Yes | Yes |
| Upload/replace source documents | No | Yes | Yes |
| Edit catalog entries and propose corrections | No | Yes | Yes |
| Propose Head of Family with citations | No | Yes | Yes |
| Approve, assign, revoke, or replace Head of Family | No | No | Yes |
| Invite/deactivate users or assign roles | No | No | Yes |
| Read audit events | No | No | Yes |

All permission checks happen in the Rust permission policy. UI visibility is a
convenience, never an authorization decision. Missing identity or permission
fails closed with `401` or `403` before document staging, database access,
embeddings, or model calls.

## Initial setup

1. Apply every migration in `migrations/` to the target database.
2. Configure the API environment above and start the API on loopback.
3. Create the first administrator with the explicit `make admin-setup` command
   (see [Keycloak administrator setup](#keycloak-administrator-setup)). It
   resolves a Keycloak username to the provider's immutable user id, shows the
   result, and then uses the bootstrap or the provision/grant commands. It is
   never run automatically by `make app-update`.

4. Start the Next.js UI with `API_URL` pointing at the loopback API.
5. Do not expose the API publicly until a real OIDC provider, TLS, secure
   session configuration, and the bootstrap procedure are in place.

## Keycloak administrator setup

`make admin-setup` is the supported way to configure an administrator from a
Keycloak username. It resolves the username to the **immutable Keycloak user
id** (the OIDC subject) before provisioning, so no username, email, or guessed
subject reaches the application.

### Required configuration

| Variable | Purpose |
| --- | --- |
| `KEYCLOAK_URL` | Keycloak **base** URL, for example `https://kc.example.com`. Never the realm URL. |
| `KEYCLOAK_REALM` | Realm that issues the application tokens. |
| `AUTH_ISSUER` | Must equal `KEYCLOAK_URL/realms/KEYCLOAK_REALM`; setup refuses a mismatch. |
| `DATABASE_URL` | Application database that holds users, roles, grants, and audit events. |
| `KEYCLOAK_USERNAME` or `--username` | Username to resolve. Prompted when neither is supplied. |
| `ADMIN_CORPUS` or `--corpus` | Corpus to grant. Defaults to `trusted`. |
| `KEYCLOAK_ADMIN_CLIENT_ID` | Optional admin client id. Defaults to `admin-cli`. |
| `KEYCLOAK_ADMIN_CLIENT_SECRET` | Optional. When set, a client-credentials service account is used instead of a password grant. |
| `KEYCLOAK_ADMIN_USERNAME`, `KEYCLOAK_ADMIN_PASSWORD` | Optional provider credentials. Missing values are prompted for; the password is read without echo. |

Caller environment values win over `.env` defaults. Secrets are never printed,
written to audit events, or stored by the script.

### Command

```bash
make admin-setup KEYCLOAK_USERNAME=alice
```

The command:

1. Prints the realm, username, resolved user id, issuer, corpus, and whether it
   is creating the first administrator or provisioning an existing one.
2. Rejects a missing user, an ambiguous (case-insensitive) match, and a
   placeholder or template user id.
3. Uses `admin bootstrap` when no administrative role exists, and the existing
   `admin provision`, `admin grant --role admin`, and `admin grant-corpus`
   commands otherwise.
4. Runs only the commands that are still needed, so a repeated run is a no-op
   that writes no duplicate rows and no extra audit events.
5. Never writes genealogy data and never creates an account-to-person link.

To resolve and display without writing anything:

```bash
scripts/admin-setup.sh --username alice --dry-run
```

### Equivalent manual CLI commands

Resolve the user id in Keycloak (Admin console → Users → the user → **ID**),
then use it as `--subject`. The subject is the immutable user id, never the
username or email, and `--issuer` is required.

```bash
# First administrator, only while no admin exists
cargo run --locked --bin admin -- bootstrap \
  --subject <resolved-user-id> --issuer <issuer> --corpus trusted

# An additional administrator
cargo run --locked --bin admin -- provision \
  --subject <resolved-user-id> --issuer <issuer>
cargo run --locked --bin admin -- grant \
  --subject <resolved-user-id> --issuer <issuer> --role admin
cargo run --locked --bin admin -- grant-corpus \
  --subject <resolved-user-id> --issuer <issuer> --corpus trusted
```

### Verify administrator access

1. Sign in at `/signin` as that Keycloak user.
2. `GET /api/account` returns `roles` containing `admin` and the expected corpus
   grants; the review console and administrative pages become available.
3. Optionally confirm in the database with a read-only query:

   ```sql
   SELECT u.subject, u.issuer, r.role, c.corpus
     FROM users u
     LEFT JOIN role_assignments r
       ON r.user_id = u.id AND r.revoked_at IS NULL
     LEFT JOIN corpus_grants c
       ON c.user_id = u.id AND c.revoked_at IS NULL
    WHERE u.subject = '<resolved-user-id>'
      AND u.issuer = '<issuer>'
      AND u.deactivated_at IS NULL;
   ```

### Recover from an incorrect subject

Never remove the last valid administrator before the replacement works.

1. Resolve the correct Keycloak user and run `make admin-setup` for it. Because
   an administrator already exists, it provisions the account and grants the
   admin role and corpus.
2. Verify the new administrator can sign in and reach the admin surfaces.
3. Only then revoke the admin role from the incorrect account:

   ```bash
   cargo run --locked --bin admin -- revoke \
     --subject <incorrect-user-id> --issuer <issuer> --role admin
   ```

   The last-admin safeguard refuses a revocation that would leave no active
   administrator.
4. Deactivate the incorrect account if it should no longer sign in:

   ```bash
   cargo run --locked --bin admin -- deactivate \
     --subject <incorrect-user-id> --issuer <issuer>
   ```

## Invite and role administration

The admin CLI is the supported operator interface for account and grant
changes. Every write is transactional and records an `audit_events` row.

```bash
# Grant or revoke an application role
cargo run --locked --bin admin -- grant  --subject <sub> --issuer <issuer> --role viewer
cargo run --locked --bin admin -- revoke --subject <sub> --issuer <issuer> --role editor

# Grant or revoke a corpus grant
cargo run --locked --bin admin -- grant-corpus  --subject <sub> --issuer <issuer> --corpus <corpus>
cargo run --locked --bin admin -- revoke-corpus --subject <sub> --issuer <issuer> --corpus <corpus>
```

Admin HTTP operations are available for role and Head of Family workflows and
are protected by the same policy. Account provisioning and deactivation are
available through the admin API and the CLI. A role change is rejected if it
would leave no active admin or if the target account does not exist; nothing is
written on failure. Concurrent admin revocations are serialized so one active
administrator remains.

```bash
# Provision/reactivate or deactivate an account
cargo run --locked --bin admin -- provision --subject <sub> --issuer <issuer>
cargo run --locked --bin admin -- deactivate --subject <sub> --issuer <issuer>
```

Audit rules:

- Actor, action, target, and timestamp are recorded for each role, corpus, and
  Head of Family change.
- Tokens, passwords, secrets, database URLs, and retrieved document content are
  never written to audit events.
- Audit access itself requires the admin role.

## Sessions, proxy, and CSRF

- `/api/*` is served by the Next.js server-side proxy, not an unrestricted
  rewrite.
- The proxy forwards only the verified bearer credential, strips client-supplied
  identity headers (`X-User`, `X-Role`, `X-Subject`, and similar), and performs
  no authorization decision of its own.
- State-changing proxy requests without a credential are rejected with `401`
  before reaching the API.
- State-changing requests with a session require a CSRF header that matches the
  `rag_csrf` cookie.
- Session and CSRF cookies use `SameSite=Lax` (needed for the authorization-code
  redirect), `HttpOnly` for the session, and `Secure` outside the explicit local
  HTTP E2E setting.

### OIDC browser flow

1. `/signin` links to `/api/auth/login` only when `OIDC_ISSUER` and
   `OIDC_CLIENT_ID` are configured.
2. `/api/auth/login` creates state, nonce, and a PKCE verifier in short-lived
   HttpOnly cookies and redirects to the provider authorization endpoint.
3. `/api/auth/callback` verifies state and nonce, exchanges the code with the
   PKCE verifier, checks issuer/audience/expiry, and asks Rust `/api/account` to
   verify the credential and load database roles/grants.
4. Only a successful Rust verification sets the `rag_session` cookie and a CSRF
   cookie. Tokens, client secrets, and provider configuration never reach the
   browser.
5. A failed exchange, mismatched state/nonce, expired token, provider outage, or
   unprovisioned account redirects to `/signin` with an error and no session.

## Backup, migrations, and rollback

Before applying migrations to existing data:

1. Take a PostgreSQL backup, for example:

   ```bash
   pg_dump "$DATABASE_URL" --format=custom --file tran-before-rbac.dump
   ```

2. Apply migrations idempotently in filename order:

   ```bash
   for f in migrations/*.sql; do
     psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f "$f" || exit 1
   done
   ```

3. Verify the expected tables and indexes exist (`users`, `role_assignments`,
   `corpus_grants`, `audit_events`, and the genealogy/appointment tables).

Rollback:

- Restore the backup into a fresh database and point `DATABASE_URL` at it:

  ```bash
  createdb tran_rbac_rollback
  pg_restore --dbname tran_rbac_rollback --clean --if-exists tran-before-rbac.dump
  ```

- Do not run destructive down-migrations against the live database. If a
  forward migration must be reverted, restore the backup and reapply only the
  migrations that are known-good.

Migration safety rules:

- Migrations are additive and idempotent where practical.
- Existing indexed family documents are preserved.
- Rows are replaced and read by `(corpus, source)`, never by filename alone, so
  identical filenames in distinct corpora stay isolated.
- Legacy `NULL`-corpus rows are not exposed automatically.

## Recovery

- A task stopped for missing identity configuration is an operator/configuration
  blocker, not a human approval. Configure OIDC/JWKS and re-run.
- A token verification failure or identity-provider outage is an error. It never
  becomes anonymous access.
- If a role change fails, inspect the audit table and the account state; no
  partial assignment is written.
- If the web session is lost, sign in again. An expired session shows an explicit
  sign-in state rather than cached records.
- If the proxy is unreachable, the UI reports the failure and does not render
  stale protected data.
- The supported recovery for a blocked SOP task is `sop retry <task-id>` after
  the concrete blocker is resolved, followed by
  `sop run docs/plans/PLAN-TRAN-RBAC.md`.
