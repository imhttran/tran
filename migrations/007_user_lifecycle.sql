-- TRAN-003C: account deactivation.
--
-- Deactivated accounts keep their historical role/grant/audit rows but are no
-- longer resolved for authorization. The identity store filters this column on
-- every lookup; no role is inferred from the token.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS deactivated_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS users_active_subject_idx
    ON users (subject)
    WHERE deactivated_at IS NULL;
