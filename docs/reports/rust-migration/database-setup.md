# Migration database setup (operator-run)

Purpose: provision a dedicated, disposable PostgreSQL database for the RAG
migration's integration acceptance checks (storage/ingestion/retrieval). Run these
as an operator with local PostgreSQL privileges. **Do not paste passwords into
chat**; supply secrets through the environment or a prompt. **Do not alter any
existing database, role, or data.**

## Inspected environment (2026-10-17)

- A **native PostgreSQL 16 cluster is online at `127.0.0.1:5432`** (owner
  `postgres`).
- The repository's `docker-compose.yml` defines a `pgvector/pgvector:pg17` image on
  **`:5434`**, but **Docker is not installed** on this host — so `:5434` is *not*
  the endpoint to use, and Docker need not be installed for this.
- `:5432` is the suitable instance. The migration's `DATABASE_URL` must point at the
  database you provision here.

## Steps (operator, with admin rights)

1. Confirm `pgvector` is available to the cluster:
   ```sh
   psql -h 127.0.0.1 -p 5432 -U <admin> -d postgres \
     -c "SELECT name, default_version FROM pg_available_extensions WHERE name = 'vector';"
   ```
   If absent, install the pgvector package for PostgreSQL 16 from your distribution,
   then retry. Do not install Docker solely to satisfy the original compose file.

2. Create a **dedicated role** and a **dedicated database** (pick your own secret;
   do not reuse or modify an existing role/database):
   ```sql
   CREATE ROLE rag_migration LOGIN PASSWORD '<set-a-secret>';
   CREATE DATABASE rag_migration_test OWNER rag_migration;
   ```

3. Enable the extension in that database (owner or admin):
   ```sql
   \c rag_migration_test
   CREATE EXTENSION IF NOT EXISTS vector;
   ```

4. Point the migration at it (host shell, not chat):
   ```sh
   export DATABASE_URL='postgres://rag_migration:<secret>@127.0.0.1:5432/rag_migration_test?sslmode=disable'
   export RAG_TEST_DATABASE_URL="$DATABASE_URL"   # the integration harness reads this
   ```
   The harness **refuses** to run when the database name is a shared/`rag_db` name;
   never point these at a shared or production database.

5. Apply the repository's migrations (idempotent, `IF NOT EXISTS`):
   ```sh
   for f in migrations/*.sql; do psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f "$f"; done
   ```

## Safety

- The database is **disposable**; recreate it freely.
- Never reset a shared database, never use production data, and never make a
  destructive schema change as a verification step.
- Keep the secret out of the repository, out of chat, and out of client bundles.
- This document is preparation only; the privileged steps above have **not** been
  executed.
