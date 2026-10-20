-- Phase 2: identity, role assignment, corpus grants, and audit trail.
--
-- Default deny is a schema property as well as a policy property: there is no
-- table that lets the application mint a role assignment without an audit row
-- in the same transaction (see src/identity_store.rs).

CREATE TABLE IF NOT EXISTS users (
    id          BIGSERIAL PRIMARY KEY,
    subject     TEXT NOT NULL UNIQUE,
    issuer      TEXT NOT NULL,
    email       TEXT,
    display_name TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT users_subject_not_blank CHECK (length(btrim(subject)) > 0),
    CONSTRAINT users_issuer_not_blank CHECK (length(btrim(issuer)) > 0)
);

CREATE TABLE IF NOT EXISTS role_assignments (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role        TEXT NOT NULL,
    granted_by  TEXT NOT NULL,
    granted_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at  TIMESTAMPTZ,
    CONSTRAINT role_assignments_role_valid CHECK (role IN ('viewer', 'editor', 'admin')),
    CONSTRAINT role_assignments_granted_by_not_blank CHECK (length(btrim(granted_by)) > 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS role_assignments_active_role
    ON role_assignments (user_id, role)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS role_assignments_user_idx
    ON role_assignments (user_id);

-- The initial-admin bootstrap refuses to run after any admin exists, but the
-- schema does not cap the number of admins: later explicit grants may add more.
DROP INDEX IF EXISTS role_assignments_single_active_admin;

CREATE TABLE IF NOT EXISTS corpus_grants (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    corpus      TEXT NOT NULL,
    granted_by  TEXT NOT NULL,
    granted_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at  TIMESTAMPTZ,
    CONSTRAINT corpus_grants_corpus_not_blank CHECK (length(btrim(corpus)) > 0),
    CONSTRAINT corpus_grants_granted_by_not_blank CHECK (length(btrim(granted_by)) > 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS corpus_grants_active_grant
    ON corpus_grants (user_id, corpus)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS corpus_grants_user_idx
    ON corpus_grants (user_id);

CREATE TABLE IF NOT EXISTS audit_events (
    id          BIGSERIAL PRIMARY KEY,
    actor       TEXT NOT NULL,
    action      TEXT NOT NULL,
    subject     TEXT NOT NULL,
    detail      JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT audit_events_actor_not_blank CHECK (length(btrim(actor)) > 0),
    CONSTRAINT audit_events_action_not_blank CHECK (length(btrim(action)) > 0),
    CONSTRAINT audit_events_subject_not_blank CHECK (length(btrim(subject)) > 0)
);

CREATE INDEX IF NOT EXISTS audit_events_subject_idx ON audit_events (subject);
CREATE INDEX IF NOT EXISTS audit_events_action_idx ON audit_events (action);
CREATE INDEX IF NOT EXISTS audit_events_created_at_idx ON audit_events (created_at DESC);
