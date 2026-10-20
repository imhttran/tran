-- TRAN-003E: Head of Family nominations, appointments, and audit.
--
-- This migration lives only in the genealogy database. It extends the TRAN-002
-- catalog/evidence/review model; it never recreates people, branches, or
-- assertions. The pure eligibility decision remains in src/genealogy/eligibility.rs.

CREATE TABLE IF NOT EXISTS head_of_family_nomination (
    nomination_id        BIGSERIAL PRIMARY KEY,
    person_key           TEXT NOT NULL,
    branch_key           TEXT NOT NULL,
    nominated_by         TEXT NOT NULL,
    status               TEXT NOT NULL DEFAULT 'pending',
    evidence_assertion_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at           TIMESTAMPTZ,
    decided_by           TEXT,
    note                 TEXT,
    CONSTRAINT hof_nomination_status_check CHECK (
        status IN ('pending', 'approved', 'rejected', 'withdrawn')
    ),
    CONSTRAINT hof_nomination_person_not_blank CHECK (length(btrim(person_key)) > 0),
    CONSTRAINT hof_nomination_branch_not_blank CHECK (length(btrim(branch_key)) > 0),
    CONSTRAINT hof_nomination_actor_not_blank CHECK (length(btrim(nominated_by)) > 0)
);

CREATE INDEX IF NOT EXISTS hof_nomination_pending_idx
    ON head_of_family_nomination (branch_key, created_at DESC)
    WHERE status = 'pending';

CREATE TABLE IF NOT EXISTS head_of_family_appointment (
    appointment_id       BIGSERIAL PRIMARY KEY,
    nomination_id        BIGINT REFERENCES head_of_family_nomination(nomination_id) ON DELETE SET NULL,
    person_key           TEXT NOT NULL,
    branch_key           TEXT NOT NULL,
    approved_by          TEXT NOT NULL,
    evidence_assertion_ids JSONB NOT NULL,
    status               TEXT NOT NULL DEFAULT 'active',
    appointed_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at             TIMESTAMPTZ,
    end_reason           TEXT,
    CONSTRAINT hof_appointment_status_check CHECK (
        status IN ('active', 'suspended', 'revoked')
    ),
    CONSTRAINT hof_appointment_person_not_blank CHECK (length(btrim(person_key)) > 0),
    CONSTRAINT hof_appointment_branch_not_blank CHECK (length(btrim(branch_key)) > 0),
    CONSTRAINT hof_appointment_approver_not_blank CHECK (length(btrim(approved_by)) > 0),
    CONSTRAINT hof_appointment_active_has_no_end CHECK (
        status <> 'active' OR ended_at IS NULL
    )
);

-- At most one active designation per branch. Concurrent approvals for the same
-- branch are serialized by an advisory lock in the approval transaction and this
-- partial unique index is the final database guard.
CREATE UNIQUE INDEX IF NOT EXISTS hof_appointment_one_active_per_branch
    ON head_of_family_appointment (branch_key)
    WHERE status = 'active';

CREATE INDEX IF NOT EXISTS hof_appointment_person_idx
    ON head_of_family_appointment (person_key);
CREATE INDEX IF NOT EXISTS hof_appointment_active_idx
    ON head_of_family_appointment (branch_key, appointed_at DESC)
    WHERE status = 'active';

CREATE TABLE IF NOT EXISTS head_of_family_audit_event (
    event_id       BIGSERIAL PRIMARY KEY,
    appointment_id BIGINT,
    nomination_id  BIGINT,
    actor          TEXT NOT NULL,
    action         TEXT NOT NULL,
    detail         JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT hof_audit_actor_not_blank CHECK (length(btrim(actor)) > 0),
    CONSTRAINT hof_audit_action_not_blank CHECK (length(btrim(action)) > 0)
);

CREATE INDEX IF NOT EXISTS hof_audit_appointment_idx
    ON head_of_family_audit_event (appointment_id);
CREATE INDEX IF NOT EXISTS hof_audit_nomination_idx
    ON head_of_family_audit_event (nomination_id);

CREATE OR REPLACE FUNCTION hof_audit_append_only() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'head_of_family_audit_event rows are append-only (no UPDATE/DELETE)';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS hof_audit_no_update ON head_of_family_audit_event;
CREATE TRIGGER hof_audit_no_update
    BEFORE UPDATE OR DELETE ON head_of_family_audit_event
    FOR EACH ROW EXECUTE FUNCTION hof_audit_append_only();
