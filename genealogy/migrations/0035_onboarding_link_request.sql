-- Reviewed linking for an accepted invitation.
--
-- Additive only: this migration creates one new table. An accepted invitation
-- records a *request*; an administrator makes the explicit reviewed-link
-- decision through the existing link review path. Possessing the invitation token
-- therefore never approves a person link by itself.
--
-- The table stores only the opaque identity account id and the one-way identity
-- fingerprint. It never stores a subject, issuer, token, hash, or address.

CREATE TABLE IF NOT EXISTS onboarding_link_request (
    request_id           BIGSERIAL PRIMARY KEY,
    invitation_id        BIGINT NOT NULL
                         REFERENCES onboarding_invitation(invitation_id) ON DELETE CASCADE,
    person_id            BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    account_id           BIGINT NOT NULL,
    identity_fingerprint TEXT NOT NULL,
    status               TEXT NOT NULL DEFAULT 'pending',
    requested_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at           TIMESTAMPTZ,
    decided_by           TEXT,
    CONSTRAINT onboarding_link_request_status_check CHECK (
        status IN ('pending', 'approved', 'rejected')
    ),
    CONSTRAINT onboarding_link_request_account_not_blank
        CHECK (account_id > 0),
    CONSTRAINT onboarding_link_request_fingerprint_not_blank
        CHECK (length(btrim(identity_fingerprint)) > 0)
);

-- One request per invitation: a retry of the same link is idempotent.
CREATE UNIQUE INDEX IF NOT EXISTS onboarding_link_request_invitation_key
    ON onboarding_link_request (invitation_id);

-- One outstanding request per account and per person.
CREATE UNIQUE INDEX IF NOT EXISTS onboarding_link_request_pending_account
    ON onboarding_link_request (account_id)
 WHERE status = 'pending';

CREATE UNIQUE INDEX IF NOT EXISTS onboarding_link_request_pending_person
    ON onboarding_link_request (person_id)
 WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS onboarding_link_request_status_idx
    ON onboarding_link_request (status, requested_at DESC);
