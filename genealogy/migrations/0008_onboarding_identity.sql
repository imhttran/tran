-- USR-003A genealogy migration 0008: onboarding identity keys (non-destructive).
--
-- This migration adds case-insensitive username reservation keys, invitation
-- token hash columns, and a person-account identity fingerprint. It is strictly
-- non-destructive: it adds columns and indexes to the three onboarding tables
-- created by 0007 and drops only the superseded case-sensitive username
-- uniqueness constraint. It never drops a table, a column, or a row.
--
-- No other table is created or changed. The schema stays inside the separate
-- genealogy database selected by GENEALOGY_DATABASE_URL.

-- Case-insensitive username reservations. `username_key` carries the canonical
-- lowercase form of the display username and is the uniqueness guard used by
-- reservation. The legacy case-sensitive UNIQUE(username) constraint is dropped
-- because two people may legitimately share a display name; uniqueness now lives
-- on the canonical key. Dropping the constraint removes no row and no column.
ALTER TABLE onboarding_username_reservation
    ADD COLUMN IF NOT EXISTS username_key TEXT;

UPDATE onboarding_username_reservation
   SET username_key = lower(username)
 WHERE username_key IS NULL;

ALTER TABLE onboarding_username_reservation
    ALTER COLUMN username_key SET NOT NULL;

DROP INDEX IF EXISTS onboarding_username_reservation_username_key_idx;

CREATE UNIQUE INDEX IF NOT EXISTS onboarding_username_reservation_username_key_idx
    ON onboarding_username_reservation (username_key)
 WHERE status <> 'released';

-- One active reservation per person. A concurrent duplicate insert fails
-- instead of creating two active rows for the same person.
CREATE UNIQUE INDEX IF NOT EXISTS onboarding_username_reservation_person_active_idx
    ON onboarding_username_reservation (person_id)
 WHERE status <> 'released';

ALTER TABLE onboarding_username_reservation
    DROP CONSTRAINT IF EXISTS onboarding_username_reservation_username_key;

-- Invitation token hardening. New invitations store only a hash of the token,
-- an expiry, and a used timestamp. The legacy plaintext `invitation_token`
-- column is made nullable so new rows can omit it. No token logic lives here.
ALTER TABLE onboarding_invitation
    ADD COLUMN IF NOT EXISTS token_hash TEXT;

ALTER TABLE onboarding_invitation
    ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;

ALTER TABLE onboarding_invitation
    ADD COLUMN IF NOT EXISTS used_at TIMESTAMPTZ;

ALTER TABLE onboarding_invitation
    ALTER COLUMN invitation_token DROP NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS onboarding_invitation_token_hash_idx
    ON onboarding_invitation (token_hash)
 WHERE token_hash IS NOT NULL;

-- One account links to one person. `identity_fingerprint` gives a stable
-- identity anchor for a link; only non-null values are unique. No linking logic
-- lives here.
ALTER TABLE onboarding_person_account_link
    ADD COLUMN IF NOT EXISTS identity_fingerprint TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS onboarding_person_account_link_identity_fingerprint_idx
    ON onboarding_person_account_link (identity_fingerprint)
 WHERE identity_fingerprint IS NOT NULL;
