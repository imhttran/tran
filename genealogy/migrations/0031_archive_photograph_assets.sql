-- AP-102: asset byte-store columns and the versioned occurrence key.
--
-- AP-101 shipped migration 0028. The runner decides what to apply by file name
-- alone, so an edited 0028 is skipped on any database that already recorded it.
-- These additions therefore live in a new additive migration so deployed
-- databases upgrade without losing data.
--
-- Every statement is additive and safe on a second run: ADD COLUMN IF NOT
-- EXISTS, guarded constraints, and CREATE INDEX IF NOT EXISTS. No existing
-- object is dropped, renamed, or retyped, and existing rows keep their values.

-- Byte-store columns. The content hash ties one asset to one byte sequence; the
-- storage key is relative to the private store root.
ALTER TABLE archive_photograph_asset
    ADD COLUMN IF NOT EXISTS content_sha256 TEXT;
ALTER TABLE archive_photograph_asset
    ADD COLUMN IF NOT EXISTS mime_type TEXT;
ALTER TABLE archive_photograph_asset
    ADD COLUMN IF NOT EXISTS byte_size BIGINT;
ALTER TABLE archive_photograph_asset
    ADD COLUMN IF NOT EXISTS storage_key TEXT;

-- Publication state, defaulting to unpublished. Existing rows receive the
-- default and stay unpublished.
ALTER TABLE archive_photograph_asset
    ADD COLUMN IF NOT EXISTS publication_state TEXT NOT NULL DEFAULT 'unpublished';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'archive_photograph_asset_content_sha_check'
    ) THEN
        ALTER TABLE archive_photograph_asset
            ADD CONSTRAINT archive_photograph_asset_content_sha_check CHECK (
                content_sha256 IS NULL OR content_sha256 ~ '^[0-9a-f]{64}$'
            );
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'archive_photograph_asset_byte_size_check'
    ) THEN
        ALTER TABLE archive_photograph_asset
            ADD CONSTRAINT archive_photograph_asset_byte_size_check CHECK (
                byte_size IS NULL OR byte_size >= 0
            );
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'archive_photograph_asset_storage_key_check'
    ) THEN
        ALTER TABLE archive_photograph_asset
            ADD CONSTRAINT archive_photograph_asset_storage_key_check CHECK (
                storage_key IS NULL
                OR (length(btrim(storage_key)) > 0 AND storage_key !~ '^/')
            );
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'archive_photograph_asset_publication_state_check'
    ) THEN
        ALTER TABLE archive_photograph_asset
            ADD CONSTRAINT archive_photograph_asset_publication_state_check CHECK (
                publication_state IN ('unpublished', 'published')
            );
    END IF;
END;
$$;

-- A content hash is unique per asset. Multiple NULLs stay allowed for rows that
-- predate the byte store.
CREATE UNIQUE INDEX IF NOT EXISTS archive_photograph_asset_content_sha_key
    ON archive_photograph_asset (content_sha256);

-- The occurrence key names the document version, so the new key is
-- (asset, document version, page, ordinal). The column stays nullable so
-- existing rows remain valid.
ALTER TABLE archive_photograph_occurrence
    ADD COLUMN IF NOT EXISTS document_version_id BIGINT;

CREATE UNIQUE INDEX IF NOT EXISTS archive_photograph_occurrence_version_key
    ON archive_photograph_occurrence
        (asset_id, document_version_id, page_number, occurrence_ordinal);

COMMENT ON COLUMN archive_photograph_asset.content_sha256 IS
    'AP-102: SHA-256 of the stored bytes; one byte sequence maps to one asset.';
COMMENT ON COLUMN archive_photograph_asset.mime_type IS
    'AP-102: media type of the stored bytes.';
COMMENT ON COLUMN archive_photograph_asset.byte_size IS
    'AP-102: stored byte length.';
COMMENT ON COLUMN archive_photograph_asset.storage_key IS
    'AP-102: relative key under the private photograph store root.';
COMMENT ON COLUMN archive_photograph_asset.publication_state IS
    'AP-102: published only after review; defaults to unpublished.';
COMMENT ON COLUMN archive_photograph_occurrence.document_version_id IS
    'AP-102: document version the drawn occurrence belongs to.';
