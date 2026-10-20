-- USR-003 upgrade safety: ensure onboarding child foreign keys exist.
--
-- Migration 0007 creates the onboarding tables with foreign keys, but
-- `CREATE TABLE IF NOT EXISTS` cannot add a foreign key to a table that an
-- earlier schema version already created. Databases upgraded from that earlier
-- version therefore had orphan-capable child tables.
--
-- This migration adds only the missing constraints. It uses NOT VALID so it
-- applies without scanning existing rows; new rows are enforced immediately and
-- ON DELETE CASCADE applies to rows written after the constraint exists. It
-- never deletes or rewrites data.
--
-- Fresh databases already have the constraints; each block is a no-op there.

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_invitation_person_fk'
    ) THEN
        ALTER TABLE onboarding_invitation
            ADD CONSTRAINT onboarding_invitation_person_fk
            FOREIGN KEY (person_id) REFERENCES person(person_id)
            ON DELETE CASCADE NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_username_reservation_person_fk'
    ) THEN
        ALTER TABLE onboarding_username_reservation
            ADD CONSTRAINT onboarding_username_reservation_person_fk
            FOREIGN KEY (person_id) REFERENCES person(person_id)
            ON DELETE CASCADE NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_person_account_link_person_fk'
    ) THEN
        ALTER TABLE onboarding_person_account_link
            ADD CONSTRAINT onboarding_person_account_link_person_fk
            FOREIGN KEY (person_id) REFERENCES person(person_id)
            ON DELETE CASCADE NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_link_audit_person_fk'
    ) THEN
        ALTER TABLE onboarding_link_audit
            ADD CONSTRAINT onboarding_link_audit_person_fk
            FOREIGN KEY (person_id) REFERENCES person(person_id)
            ON DELETE CASCADE NOT VALID;
    END IF;
END;
$$;
