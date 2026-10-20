-- TRAN-002B genealogy migration 0001: schema version tracking.
--
-- This file creates the genealogy-owned version table. It is the only table
-- the migration runner reads to decide which files are already applied. It
-- must be the first file applied.

CREATE TABLE IF NOT EXISTS genealogy_schema_version (
    version       TEXT PRIMARY KEY,
    applied_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE genealogy_schema_version IS
    'Records each applied genealogy migration file name and its apply time.';
