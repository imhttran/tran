-- USR-005A: branch reviewer grants and immutable grant/revocation audit.
--
-- The existing onboarding_branch_reviewer_grant row remains the active grant
-- record (one row per account and branch). This migration binds that row to the
-- exact reviewed identity fingerprint, records grant/revoke reasons and actors,
-- and adds an append-only event ledger that survives branch/account cleanup.
--
-- Nothing is inferred from a genealogy title, branch membership, or Head of
-- Family appointment. A grant is usable only while the account has the reviewed
-- person link carrying the same identity fingerprint.

ALTER TABLE onboarding_branch_reviewer_grant
    ADD COLUMN IF NOT EXISTS identity_fingerprint TEXT,
    ADD COLUMN IF NOT EXISTS granted_reason TEXT,
    ADD COLUMN IF NOT EXISTS revoked_by TEXT,
    ADD COLUMN IF NOT EXISTS revoke_reason TEXT,
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now();

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_branch_reviewer_grant_fingerprint_check'
           AND conrelid = 'onboarding_branch_reviewer_grant'::regclass
    ) THEN
        ALTER TABLE onboarding_branch_reviewer_grant
            ADD CONSTRAINT onboarding_branch_reviewer_grant_fingerprint_check
            CHECK (
                identity_fingerprint IS NULL
                OR identity_fingerprint ~ '^[0-9a-f]{64}$'
            );
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_branch_reviewer_grant_reason_check'
           AND conrelid = 'onboarding_branch_reviewer_grant'::regclass
    ) THEN
        ALTER TABLE onboarding_branch_reviewer_grant
            ADD CONSTRAINT onboarding_branch_reviewer_grant_reason_check
            CHECK (
                (granted_reason IS NULL OR char_length(granted_reason) <= 1000)
                AND (revoke_reason IS NULL OR char_length(revoke_reason) <= 1000)
            );
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_branch_reviewer_grant_actor_check'
           AND conrelid = 'onboarding_branch_reviewer_grant'::regclass
    ) THEN
        ALTER TABLE onboarding_branch_reviewer_grant
            ADD CONSTRAINT onboarding_branch_reviewer_grant_actor_check
            CHECK (
                length(btrim(granted_by)) > 0
                AND (revoked_by IS NULL OR length(btrim(revoked_by)) > 0)
            ) NOT VALID;
    END IF;
END;
$$;

CREATE INDEX IF NOT EXISTS onboarding_branch_reviewer_grant_active_account_idx
    ON onboarding_branch_reviewer_grant (account_id, identity_fingerprint)
 WHERE revoked_at IS NULL;

-- One active grant per account and branch, while keeping revoked rows as
-- immutable history. The legacy table-wide unique constraint would prevent a
-- second grant after a revocation, so replace it with this active-only index.
ALTER TABLE onboarding_branch_reviewer_grant
    DROP CONSTRAINT IF EXISTS onboarding_branch_reviewer_grant_key;

CREATE UNIQUE INDEX IF NOT EXISTS onboarding_branch_reviewer_grant_active_key
    ON onboarding_branch_reviewer_grant (account_id, family_branch_id)
 WHERE revoked_at IS NULL;

CREATE TABLE IF NOT EXISTS onboarding_branch_reviewer_grant_event (
    event_id            BIGSERIAL PRIMARY KEY,
    grant_id            BIGINT NOT NULL,
    account_id          BIGINT NOT NULL,
    family_branch_id    BIGINT NOT NULL,
    identity_fingerprint TEXT,
    event               TEXT NOT NULL,
    actor               TEXT NOT NULL,
    reason              TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT onboarding_branch_reviewer_grant_event_event_check CHECK (
        event IN ('granted', 'revoked')
    ),
    CONSTRAINT onboarding_branch_reviewer_grant_event_actor_check CHECK (
        length(btrim(actor)) > 0
    ),
    CONSTRAINT onboarding_branch_reviewer_grant_event_reason_check CHECK (
        reason IS NULL OR char_length(reason) <= 1000
    )
);

ALTER TABLE onboarding_branch_reviewer_grant_event
    ADD COLUMN IF NOT EXISTS branch_key TEXT;

CREATE INDEX IF NOT EXISTS onboarding_branch_reviewer_grant_event_account_idx
    ON onboarding_branch_reviewer_grant_event (account_id, event_id);

CREATE INDEX IF NOT EXISTS onboarding_branch_reviewer_grant_event_branch_idx
    ON onboarding_branch_reviewer_grant_event (family_branch_id, event_id);

-- The event ledger is append-only. It intentionally has no FK to the grant,
-- branch, or identity tables so revocation and later cleanup cannot erase it.
CREATE OR REPLACE FUNCTION onboarding_branch_reviewer_grant_event_no_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'branch reviewer grant events are append-only';
END;
$$;

DROP TRIGGER IF EXISTS onboarding_branch_reviewer_grant_event_no_mutation
    ON onboarding_branch_reviewer_grant_event;
CREATE TRIGGER onboarding_branch_reviewer_grant_event_no_mutation
    BEFORE UPDATE OR DELETE ON onboarding_branch_reviewer_grant_event
    FOR EACH ROW EXECUTE FUNCTION onboarding_branch_reviewer_grant_event_no_mutation();
