-- USR-004A verification follow-up: reconcile migration 0010 foreign keys.
--
-- On a fresh database, migration 0007 already creates the four onboarding
-- `person_id` foreign keys with the column-level names
-- `<table>_person_id_fkey`. Migration 0010 checks only its own proposed names
-- (`<table>_person_fk`), so it adds a second, redundant foreign key on the same
-- column. Because 0010 uses `NOT VALID`, those duplicates also remain
-- unvalidated forever.
--
-- On an upgraded database where 0007 created the table before the foreign key
-- existed, 0010's constraint is the only one and is still `NOT VALID`.
--
-- This migration is corrective and non-destructive:
--   * when a validated equivalent constraint already exists, drop the redundant
--     0010 constraint;
--   * otherwise validate the surviving 0010 constraint, which scans rows and
--     fails loudly if orphaned legacy rows exist.
--
-- No row is inserted, updated, or deleted. The migration never rewrites source
-- evidence or family data. The single DO block is one statement, so it applies
-- atomically: either every targeted constraint is reconciled or none is.

DO $$
DECLARE
    target record;
BEGIN
    FOR target IN
        SELECT *
          FROM (VALUES
              (
                  'onboarding_invitation',
                  'onboarding_invitation_person_fk',
                  'onboarding_invitation_person_id_fkey'
              ),
              (
                  'onboarding_username_reservation',
                  'onboarding_username_reservation_person_fk',
                  'onboarding_username_reservation_person_id_fkey'
              ),
              (
                  'onboarding_person_account_link',
                  'onboarding_person_account_link_person_fk',
                  'onboarding_person_account_link_person_id_fkey'
              ),
              (
                  'onboarding_link_audit',
                  'onboarding_link_audit_person_fk',
                  'onboarding_link_audit_person_id_fkey'
              )
          ) AS t(table_name, duplicate_name, canonical_name)
    LOOP
        IF EXISTS (
            SELECT 1
              FROM pg_constraint
             WHERE conrelid = to_regclass(target.table_name)
               AND conname = target.canonical_name
               AND convalidated
        ) THEN
            EXECUTE format(
                'ALTER TABLE %I DROP CONSTRAINT IF EXISTS %I',
                target.table_name,
                target.duplicate_name
            );
        ELSIF EXISTS (
            SELECT 1
              FROM pg_constraint
             WHERE conrelid = to_regclass(target.table_name)
               AND conname = target.duplicate_name
               AND NOT convalidated
        ) THEN
            EXECUTE format(
                'ALTER TABLE %I VALIDATE CONSTRAINT %I',
                target.table_name,
                target.duplicate_name
            );
        END IF;
    END LOOP;
END;
$$;
