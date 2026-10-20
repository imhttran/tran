-- USR-001: immutable identity keyed on (issuer, subject).
--
-- Upgrade safety from migrations 006 + 007:
--   006 created users with subject TEXT NOT NULL UNIQUE, which produced the
--       implicit constraint users_subject_key on subject alone.
--   007 added deactivated_at and users_active_subject_idx on subject.
-- This migration is additive and non-destructive for data. It drops the global
-- subject uniqueness constraint, adds a unique index on (issuer, subject), and
-- replaces the active subject-only index with an active (issuer, subject) index.
-- No user row, role_assignment, corpus_grant, or audit_event is modified. No
-- statement deletes, truncates, or drops a table. The subject column itself is
-- unchanged. Applying this to a database already at 007 preserves all rows.
--
-- Precondition (see the plan assumptions): no existing row has a duplicate
-- (issuer, subject) pair. Because 006..007 enforced global subject uniqueness,
-- that precondition holds; if a database violates it, the unique index creation
-- fails loudly and the migration is not applied.

ALTER TABLE users DROP CONSTRAINT IF EXISTS users_subject_key;

CREATE UNIQUE INDEX IF NOT EXISTS users_issuer_subject_key
    ON users (issuer, subject);

DROP INDEX IF EXISTS users_active_subject_idx;

CREATE INDEX IF NOT EXISTS users_active_issuer_subject_idx
    ON users (issuer, subject)
    WHERE deactivated_at IS NULL;
