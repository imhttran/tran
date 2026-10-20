-- USR-003C genealogy migration 0009: linking audit (non-destructive).
--
-- Adds an append-only audit trail for account-to-person link decisions. The
-- genealogy database stores only the opaque identity account id and the
-- one-way identity fingerprint; it never stores a raw issuer, subject, token,
-- display name, or credential.
--
-- The table is new and references only the existing person table. No existing
-- row, column, or constraint is changed.

CREATE TABLE IF NOT EXISTS onboarding_link_audit (
    audit_id             BIGSERIAL PRIMARY KEY,
    person_id            BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    account_id           BIGINT NOT NULL,
    identity_fingerprint TEXT NOT NULL,
    decision             TEXT NOT NULL,
    reviewer             TEXT NOT NULL,
    note                 TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT onboarding_link_audit_decision_check CHECK (
        decision IN ('linked', 'rejected', 'revoked')
    ),
    CONSTRAINT onboarding_link_audit_reviewer_not_blank CHECK (
        length(btrim(reviewer)) > 0
    ),
    CONSTRAINT onboarding_link_audit_fingerprint_check CHECK (
        identity_fingerprint ~ '^[0-9a-f]{64}$'
    )
);

CREATE INDEX IF NOT EXISTS onboarding_link_audit_person_idx
    ON onboarding_link_audit (person_id);
