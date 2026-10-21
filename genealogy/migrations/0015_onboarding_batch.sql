-- BULK-003/004 genealogy migration 0015: bulk onboarding persistence (additive only).
--
-- Minimal additive storage for durable, resumable, idempotent preparation of a
-- reviewed batch of living candidates, plus the separately approved invitation
-- delivery state and the explicitly verified recipient contacts.
--
-- Existing tables cannot express per-person batch outcome, reason codes, chunk
-- cursor, administrative audit history, or delivery state, so this migration
-- adds three new tables and additive columns to the new batch item table. It is
-- strictly additive inside the GENEALOGY_DATABASE_URL database: no existing
-- table, column, index or constraint is dropped, renamed or narrowed, and no
-- statement deletes data.
--
-- Idempotency keys:
--   * batch_idempotency_key is derived by the application from the confirmed
--     request (actor + sorted selected person ids + confirmation count).
--   * person_idempotency_key is derived from (batch_idempotency_key, person_id).
-- Both are unique, so duplicate submissions and restarts collapse to one row.
--
-- No foreign key references the identity database. Person rows are the only FK
-- target; identity data needed for re-evaluation is read through the identity
-- connection and never copied into this schema.
--
-- The application never stores a plaintext invitation token or a password in
-- any of these tables. Invitation tokens remain hashed in onboarding_invitation.

CREATE TABLE IF NOT EXISTS onboarding_batch (
    batch_id                 BIGSERIAL PRIMARY KEY,
    batch_idempotency_key    TEXT NOT NULL,
    status                   TEXT NOT NULL DEFAULT 'confirmed',
    confirmed_count          INTEGER NOT NULL CHECK (confirmed_count >= 0),
    selected_count           INTEGER NOT NULL CHECK (selected_count >= 0),
    cursor_person_id         BIGINT,
    created_by               TEXT NOT NULL,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at             TIMESTAMPTZ,
    note                     TEXT,
    CONSTRAINT onboarding_batch_idempotency_key_key UNIQUE (batch_idempotency_key),
    CONSTRAINT onboarding_batch_status_check CHECK (
        status IN ('confirmed', 'running', 'completed', 'partially_failed', 'cancelled')
    ),
    CONSTRAINT onboarding_batch_confirmed_matches_selected CHECK (
        confirmed_count = selected_count
    ),
    CONSTRAINT onboarding_batch_created_by_not_blank CHECK (
        length(btrim(created_by)) > 0
    )
);

-- Batch items: one durable outcome row per (batch, person). The unique person key
-- makes replays return the existing row instead of inserting a duplicate.
CREATE TABLE IF NOT EXISTS onboarding_batch_item (
    batch_item_id            BIGSERIAL PRIMARY KEY,
    batch_id                 BIGINT NOT NULL REFERENCES onboarding_batch(batch_id) ON DELETE CASCADE,
    person_id                BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    person_idempotency_key   TEXT NOT NULL,
    status                   TEXT NOT NULL DEFAULT 'pending',
    reason_code              TEXT,
    reservation_id           BIGINT REFERENCES onboarding_username_reservation(reservation_id) ON DELETE SET NULL,
    invitation_id            BIGINT REFERENCES onboarding_invitation(invitation_id) ON DELETE SET NULL,
    evidence_snapshot        TEXT,
    attempts                 INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error               TEXT,
    delivery_status          TEXT NOT NULL DEFAULT 'not_sent',
    delivery_provider        TEXT,
    delivery_attempts        INTEGER NOT NULL DEFAULT 0 CHECK (delivery_attempts >= 0),
    delivered_at             TIMESTAMPTZ,
    last_delivery_error      TEXT,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at             TIMESTAMPTZ,
    CONSTRAINT onboarding_batch_item_person_key UNIQUE (person_idempotency_key),
    CONSTRAINT onboarding_batch_item_batch_person_key UNIQUE (batch_id, person_id),
    CONSTRAINT onboarding_batch_item_status_check CHECK (
        status IN (
            'pending',
            'prepared',
            'skipped_already_linked',
            'skipped_already_invited',
            'skipped_already_prepared',
            'skipped_not_eligible',
            'skipped_duplicate',
            'needs_review',
            'failed'
        )
    ),
    CONSTRAINT onboarding_batch_item_reason_check CHECK (
        reason_code IS NULL OR reason_code IN (
            'confirmed',
            'already_linked',
            'already_invited',
            'already_prepared',
            'not_eligible',
            'duplicate_selection',
            'evidence_revoked',
            'evidence_disputed',
            'evidence_missing',
            'contradictory_vital',
            'missing_citation',
            'unresolved_duplicate',
            'ambiguous_segmentation',
            'minor_guardian_review',
            'username_collision',
            'username_policy',
            'missing_verified_contact',
            'unknown_person',
            'database_error',
            'resumed'
        )
    ),
    CONSTRAINT onboarding_batch_item_delivery_status_check CHECK (
        delivery_status IN (
            'not_sent', 'queued', 'sent', 'accepted', 'failed', 'expired'
        )
    )
);

CREATE INDEX IF NOT EXISTS onboarding_batch_item_batch_status_idx
    ON onboarding_batch_item (batch_id, status);

CREATE INDEX IF NOT EXISTS onboarding_batch_item_person_idx
    ON onboarding_batch_item (person_id);

CREATE INDEX IF NOT EXISTS onboarding_batch_item_invitation_idx
    ON onboarding_batch_item (invitation_id);

-- One logical prepared entry per person, across every batch. A later batch that
-- reselects the same person records `skipped_already_prepared` instead of
-- creating a second prepared item. The table is new in this migration, so the
-- index cannot conflict with historical rows.
CREATE UNIQUE INDEX IF NOT EXISTS onboarding_batch_item_prepared_person_key
    ON onboarding_batch_item (person_id) WHERE status = 'prepared';

-- Append-only administrative audit for batch lifecycle events.
CREATE TABLE IF NOT EXISTS onboarding_batch_audit (
    audit_id        BIGSERIAL PRIMARY KEY,
    batch_id        BIGINT NOT NULL REFERENCES onboarding_batch(batch_id) ON DELETE CASCADE,
    person_id       BIGINT,
    event           TEXT NOT NULL,
    from_status     TEXT,
    to_status       TEXT,
    reason_code     TEXT,
    actor           TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT onboarding_batch_audit_event_check CHECK (
        event IN (
            'batch_confirmed',
            'batch_resumed',
            'batch_completed',
            'item_prepared',
            'item_skipped',
            'item_needs_review',
            'item_failed',
            'item_retried',
            'invitation_queued',
            'invitation_sent',
            'invitation_failed'
        )
    ),
    CONSTRAINT onboarding_batch_audit_actor_not_blank CHECK (
        length(btrim(actor)) > 0
    )
);

CREATE INDEX IF NOT EXISTS onboarding_batch_audit_batch_idx
    ON onboarding_batch_audit (batch_id, audit_id);

CREATE INDEX IF NOT EXISTS onboarding_batch_audit_person_idx
    ON onboarding_batch_audit (person_id);

-- Verified recipient contacts. An address is usable for bulk invitations only
-- after an administrator explicitly records it here with a non-null
-- verified_at. The application never derives an address from genealogy
-- documents and never guesses one.
CREATE TABLE IF NOT EXISTS onboarding_recipient_contact (
    person_id       BIGINT PRIMARY KEY REFERENCES person(person_id) ON DELETE CASCADE,
    email           TEXT NOT NULL,
    verified_at     TIMESTAMPTZ NOT NULL,
    verified_by     TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT onboarding_recipient_contact_email_check CHECK (
        position('@' in email) > 1 AND length(btrim(email)) >= 3
    ),
    CONSTRAINT onboarding_recipient_contact_verified_by_not_blank CHECK (
        length(btrim(verified_by)) > 0
    )
);
