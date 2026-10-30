-- PIR-004: per-invitation delivery receipt state for explicit sends.
--
-- Additive only: this migration adds nullable/defaulted columns to
-- `onboarding_invitation` so one explicit send can record the adapter's actual
-- outcome. No existing column, constraint, index, or default is altered or
-- dropped, and no statement deletes data. Re-running the file is safe.
--
-- The stored values never include a plaintext token, a full recipient address,
-- or a provider credential. `last_delivery_error` records only a stable safe
-- code such as `not_configured`, `provider_failed`, `delivery_failed`, or
-- `revoked`.

ALTER TABLE onboarding_invitation
    ADD COLUMN IF NOT EXISTS delivery_state TEXT NOT NULL DEFAULT 'pending',
    ADD COLUMN IF NOT EXISTS delivery_provider TEXT,
    ADD COLUMN IF NOT EXISTS last_delivery_error TEXT,
    ADD COLUMN IF NOT EXISTS delivery_attempts INTEGER NOT NULL DEFAULT 0;
