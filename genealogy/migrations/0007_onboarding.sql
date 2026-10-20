-- USR-002 genealogy migration 0007: onboarding schema (additive only).
--
-- This migration adds the tables that support onboarding of living person
-- candidates. It is strictly additive: it creates new tables and indexes and
-- never drops, renames, or narrows a column of an existing table.
--
-- The schema stays inside the separate genealogy database selected by
-- GENEALOGY_DATABASE_URL. No RAG migration or documents table is touched.
--
-- Eligibility for an invitation is derived only from the private genealogy
-- database using the TRAN-002 conventions: a reviewed assertion with
-- assertion_kind = 'vital_status', review_status = 'verified', a living value,
-- and attributed citations through
-- assertion_evidence -> source_evidence -> source_document. Living status is
-- never inferred from a missing death year.

-- Account ids come from the identity layer (`account` rows). The foreign key
-- is declared only if that table exists in the same database so the migration
-- applies cleanly after 0006 on a fresh genealogy database. On a database that
-- hosts both domains the constraint links account and person rows.

CREATE TABLE IF NOT EXISTS onboarding_invitation (
    invitation_id     BIGSERIAL PRIMARY KEY,
    person_id         BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    account_id        BIGINT,
    invitation_token  TEXT NOT NULL,
    invited_email     TEXT,
    status            TEXT NOT NULL DEFAULT 'pending',
    created_by        TEXT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    accepted_at       TIMESTAMPTZ,
    CONSTRAINT onboarding_invitation_token_key UNIQUE (invitation_token),
    CONSTRAINT onboarding_invitation_status_check CHECK (
        status IN ('pending', 'accepted', 'revoked', 'expired')
    )
);

CREATE INDEX IF NOT EXISTS onboarding_invitation_person_idx
    ON onboarding_invitation (person_id);

-- Username reservations. A reserved username is held for one person until it
-- is either claimed or released.
CREATE TABLE IF NOT EXISTS onboarding_username_reservation (
    reservation_id   BIGSERIAL PRIMARY KEY,
    username         TEXT NOT NULL,
    person_id        BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    status           TEXT NOT NULL DEFAULT 'reserved',
    reserved_by      TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_at      TIMESTAMPTZ,
    CONSTRAINT onboarding_username_reservation_username_key UNIQUE (username),
    CONSTRAINT onboarding_username_reservation_status_check CHECK (
        status IN ('reserved', 'claimed', 'released')
    )
);

CREATE INDEX IF NOT EXISTS onboarding_username_reservation_person_idx
    ON onboarding_username_reservation (person_id);

-- Person-account links. A linked person is `already_linked` in the candidate
-- inventory. One account links to at most one person.
CREATE TABLE IF NOT EXISTS onboarding_person_account_link (
    link_id      BIGSERIAL PRIMARY KEY,
    person_id    BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    account_id   BIGINT NOT NULL,
    linked_by    TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT onboarding_person_account_link_person_key UNIQUE (person_id),
    CONSTRAINT onboarding_person_account_link_account_key UNIQUE (account_id)
);

-- Branch reviewer grants. A grant lets an account review one family branch.
CREATE TABLE IF NOT EXISTS onboarding_branch_reviewer_grant (
    grant_id          BIGSERIAL PRIMARY KEY,
    account_id        BIGINT NOT NULL,
    family_branch_id  BIGINT NOT NULL REFERENCES family_branch(family_branch_id) ON DELETE CASCADE,
    granted_by        TEXT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at        TIMESTAMPTZ,
    CONSTRAINT onboarding_branch_reviewer_grant_key UNIQUE (account_id, family_branch_id)
);

CREATE INDEX IF NOT EXISTS onboarding_branch_reviewer_grant_branch_idx
    ON onboarding_branch_reviewer_grant (family_branch_id);

-- Profile values. One current value per (person, field); history lives in
-- onboarding_profile_revision.
CREATE TABLE IF NOT EXISTS onboarding_profile_value (
    profile_value_id  BIGSERIAL PRIMARY KEY,
    person_id         BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    field_key         TEXT NOT NULL,
    value_text        TEXT NOT NULL,
    updated_by        TEXT NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT onboarding_profile_value_key UNIQUE (person_id, field_key)
);

-- Append-only profile revisions. Every change appends one revision row.
CREATE TABLE IF NOT EXISTS onboarding_profile_revision (
    revision_id       BIGSERIAL PRIMARY KEY,
    person_id         BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    field_key         TEXT NOT NULL,
    prior_value_text  TEXT,
    new_value_text    TEXT NOT NULL,
    changed_by        TEXT NOT NULL,
    changed_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS onboarding_profile_revision_person_idx
    ON onboarding_profile_revision (person_id);

-- Media metadata. Binary content lives outside the database; only metadata and
-- the source evidence that supports the media are stored here.
CREATE TABLE IF NOT EXISTS onboarding_media_metadata (
    media_id          BIGSERIAL PRIMARY KEY,
    person_id         BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    source_evidence_id BIGINT REFERENCES source_evidence(source_evidence_id) ON DELETE SET NULL,
    media_kind        TEXT NOT NULL,
    mime_type         TEXT,
    byte_size         BIGINT CHECK (byte_size IS NULL OR byte_size >= 0),
    sha256            TEXT CHECK (sha256 IS NULL OR sha256 ~ '^[0-9a-f]{64}$'),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS onboarding_media_metadata_person_idx
    ON onboarding_media_metadata (person_id);
