-- PIR-003: restricted per-person draft contact for invitation preparation.
--
-- Additive only: this migration creates one new table and never alters an
-- existing table, column, constraint, or default. The draft address is stored
-- separately from the verified `onboarding_recipient_contact` row so preparing
-- an invitation never implies delivery. Re-running the file is safe.

CREATE TABLE IF NOT EXISTS onboarding_invitation_contact_draft (
    person_id  BIGINT PRIMARY KEY REFERENCES person (person_id) ON DELETE CASCADE,
    email      TEXT NOT NULL CHECK (position('@' IN email) > 1),
    created_by TEXT NOT NULL,
    updated_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
