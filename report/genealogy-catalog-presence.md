# Phase 0 Genealogy Catalog Presence

## Result: people/branch catalog is ABSENT

Inspection of `migrations/` shows five migrations:

- `001_init.sql` — creates `documents` and `chunks`.
- `002_ingestion_metadata.sql` — creates `ingestion_sources`.
- `003_add_language.sql` — adds a language column.
- `004_page.sql` — adds a page column.
- `005_add_corpus.sql` — creates `corpora` and a corpus column.

No genealogy catalog tables exist. The following tables are absent from every
migration:

- `people`
- `branches`
- `relationships`
- person/assertion tables
- `head_of_family_appointments`
- `users`, `role_assignments`, `corpus_grants`, `audit_events` (RBAC tables)

## Consequence

Canonical third-branch identifiers, parentage, birth order, and living-status
evidence are not established in the schema. Phase 3 must add the minimum cited
schema and evidence workflow. Any real Third Branch Head of Family designation
stays pending until verified evidence exists; approval tests use only test
fixtures and synthetic data.
