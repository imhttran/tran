-- USR-004C: private photo and personal-document metadata (additive).
--
-- Extends the existing onboarding_media_metadata table rather than creating a
-- second asset system. Binary content lives under MEDIA_ROOT and is served only
-- through authorized handlers. Each media row is attached to the existing
-- versioned profile revision that carries its review state.

ALTER TABLE onboarding_media_metadata
    ADD COLUMN IF NOT EXISTS revision_id BIGINT
        REFERENCES onboarding_profile_revision(revision_id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS storage_key TEXT,
    ADD COLUMN IF NOT EXISTS file_name TEXT,
    ADD COLUMN IF NOT EXISTS visibility TEXT NOT NULL DEFAULT 'owner_private',
    ADD COLUMN IF NOT EXISTS width INTEGER,
    ADD COLUMN IF NOT EXISTS height INTEGER;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_media_metadata_visibility_check'
           AND conrelid = 'onboarding_media_metadata'::regclass
    ) THEN
        ALTER TABLE onboarding_media_metadata
            ADD CONSTRAINT onboarding_media_metadata_visibility_check
            CHECK (visibility IN ('owner_private', 'family'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_media_metadata_dimensions_check'
           AND conrelid = 'onboarding_media_metadata'::regclass
    ) THEN
        ALTER TABLE onboarding_media_metadata
            ADD CONSTRAINT onboarding_media_metadata_dimensions_check
            CHECK (
                (width IS NULL OR width > 0)
                AND (height IS NULL OR height > 0)
            );
    END IF;

    -- New uploads are always validated by the service. Enforce the kind check
    -- for new rows even when legacy rows use older labels, then validate it
    -- when every existing row already conforms.
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_media_metadata_kind_check'
           AND conrelid = 'onboarding_media_metadata'::regclass
    ) THEN
        ALTER TABLE onboarding_media_metadata
            ADD CONSTRAINT onboarding_media_metadata_kind_check
            CHECK (media_kind IN ('profile_photo', 'personal_document')) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM onboarding_media_metadata
         WHERE media_kind NOT IN ('profile_photo', 'personal_document')
    ) THEN
        ALTER TABLE onboarding_media_metadata
            VALIDATE CONSTRAINT onboarding_media_metadata_kind_check;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_media_metadata_required_fields_check'
           AND conrelid = 'onboarding_media_metadata'::regclass
    ) THEN
        ALTER TABLE onboarding_media_metadata
            ADD CONSTRAINT onboarding_media_metadata_required_fields_check
            CHECK (
                storage_key IS NOT NULL
                AND file_name IS NOT NULL
                AND mime_type IS NOT NULL
                AND byte_size IS NOT NULL
                AND sha256 IS NOT NULL
                AND revision_id IS NOT NULL
            ) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM onboarding_media_metadata
         WHERE storage_key IS NULL
            OR file_name IS NULL
            OR mime_type IS NULL
            OR byte_size IS NULL
            OR sha256 IS NULL
            OR revision_id IS NULL
    ) THEN
        ALTER TABLE onboarding_media_metadata
            VALIDATE CONSTRAINT onboarding_media_metadata_required_fields_check;
    END IF;
END;
$$;

CREATE UNIQUE INDEX IF NOT EXISTS onboarding_media_metadata_storage_key_key
    ON onboarding_media_metadata (storage_key)
 WHERE storage_key IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS onboarding_media_metadata_revision_key
    ON onboarding_media_metadata (revision_id)
 WHERE revision_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS onboarding_media_metadata_revision_person_idx
    ON onboarding_media_metadata (person_id, media_kind, revision_id);
