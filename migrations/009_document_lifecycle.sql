-- Migration 009: family document lifecycle (additive only).
--
-- Existing `documents` rows are preserved. This migration adds version,
-- review, processing, and audit metadata, quarantines one `legacy_import`
-- version per existing `(corpus, source)` group, and creates the approved-only
-- retrieval view. It never deletes, rewrites, re-embeds, or implicitly approves
-- an existing chunk or source.
--
-- Apply with:
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/009_document_lifecycle.sql

BEGIN;

CREATE TABLE IF NOT EXISTS document_version (
    version_id            BIGSERIAL PRIMARY KEY,
    document_key          TEXT NOT NULL,
    version_number        INTEGER NOT NULL DEFAULT 1 CHECK (version_number >= 1),
    corpus                TEXT NOT NULL,
    source                TEXT NOT NULL,
    title                 TEXT NOT NULL,
    description           TEXT,
    category              TEXT NOT NULL CHECK (
        category IN ('genealogy', 'photograph', 'letter', 'certificate', 'story', 'other', 'legacy')
    ),
    origin                TEXT NOT NULL CHECK (
        origin IN ('submission', 'operator_import', 'legacy_import')
    ),
    access_scope          TEXT CHECK (access_scope IN ('family', 'admin_only')),
    owner_subject         TEXT,
    content_sha256        TEXT NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    byte_size             BIGINT NOT NULL DEFAULT 0 CHECK (byte_size >= 0),
    mime_type             TEXT NOT NULL,
    original_filename     TEXT NOT NULL,
    storage_key           TEXT,
    review_status         TEXT NOT NULL DEFAULT 'pending_review' CHECK (
        review_status IN ('pending_review', 'approved', 'rejected')
    ),
    processing_status     TEXT NOT NULL DEFAULT 'unprocessed' CHECK (
        processing_status IN ('legacy_indexed', 'unprocessed', 'queued', 'processing', 'ready', 'failed')
    ),
    reviewed_by           TEXT,
    reviewed_at           TIMESTAMPTZ,
    review_note           TEXT,
    approved_sha256       TEXT,
    approved_scope        TEXT,
    approved_version_number INTEGER,
    removed_at            TIMESTAMPTZ,
    removed_by            TEXT,
    created_by            TEXT NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT document_version_group_key UNIQUE (corpus, document_key, version_number),
    CONSTRAINT document_version_storage_required CHECK (
        origin = 'legacy_import' OR storage_key IS NOT NULL
    ),
    CONSTRAINT document_version_approval_binding CHECK (
        review_status <> 'approved' OR (
            reviewed_by IS NOT NULL
            AND reviewed_at IS NOT NULL
            AND access_scope IS NOT NULL
            AND approved_sha256 = content_sha256
            AND approved_scope = access_scope
            AND approved_version_number = version_number
        )
    ),
    CONSTRAINT document_version_ready_requires_approval CHECK (
        processing_status <> 'ready' OR review_status = 'approved'
    )
);

CREATE INDEX IF NOT EXISTS document_version_corpus_source_idx
    ON document_version (corpus, source);
CREATE INDEX IF NOT EXISTS document_version_review_processing_idx
    ON document_version (review_status, processing_status);
CREATE INDEX IF NOT EXISTS document_version_owner_idx
    ON document_version (owner_subject);
-- One quarantine version per legacy group. `source` is the existing chunk
-- source; `document_key` is a deterministic legacy identifier.
CREATE UNIQUE INDEX IF NOT EXISTS document_version_legacy_group_uniq
    ON document_version (corpus, source)
    WHERE origin = 'legacy_import';

CREATE TABLE IF NOT EXISTS document_processing_job (
    job_id               BIGSERIAL PRIMARY KEY,
    version_id           BIGINT NOT NULL REFERENCES document_version(version_id) ON DELETE CASCADE,
    job_key              TEXT NOT NULL,
    status               TEXT NOT NULL DEFAULT 'queued' CHECK (
        status IN ('queued', 'processing', 'ready', 'failed')
    ),
    attempts             INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts         INTEGER NOT NULL DEFAULT 3 CHECK (max_attempts >= 1),
    claimed_by           TEXT,
    claimed_at           TIMESTAMPTZ,
    last_error           TEXT,
    created_by           TEXT NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at          TIMESTAMPTZ,
    CONSTRAINT document_processing_job_key UNIQUE (job_key)
);

CREATE INDEX IF NOT EXISTS document_processing_job_version_idx
    ON document_processing_job (version_id, job_id);
CREATE INDEX IF NOT EXISTS document_processing_job_status_idx
    ON document_processing_job (status, job_id);

CREATE TABLE IF NOT EXISTS document_audit_event (
    audit_id             BIGSERIAL PRIMARY KEY,
    version_id           BIGINT REFERENCES document_version(version_id) ON DELETE CASCADE,
    job_id               BIGINT,
    action               TEXT NOT NULL CHECK (
        action IN (
            'submitted', 'review_approved', 'review_rejected', 'scope_changed',
            'queued', 'processing_started', 'processing_ready', 'processing_failed',
            'retry', 'removed', 'legacy_inventoried', 'legacy_quarantined',
            'operator_import'
        )
    ),
    actor                TEXT NOT NULL,
    detail               JSONB,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS document_audit_event_version_idx
    ON document_audit_event (version_id, audit_id);
CREATE INDEX IF NOT EXISTS document_audit_event_created_idx
    ON document_audit_event (created_at);

CREATE OR REPLACE FUNCTION document_audit_event_reject_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'document_audit_event is append-only (% rejected)', TG_OP;
END;
$$;

DROP TRIGGER IF EXISTS document_audit_event_append_only ON document_audit_event;
CREATE TRIGGER document_audit_event_append_only
    BEFORE UPDATE OR DELETE ON document_audit_event
    FOR EACH ROW
    EXECUTE FUNCTION document_audit_event_reject_mutation();

-- Additive linkage. Existing rows keep their content, hashes, corpus/source,
-- and citations; only this new column is populated by the legacy backfill.
ALTER TABLE documents
    ADD COLUMN IF NOT EXISTS document_version_id BIGINT
    REFERENCES document_version(version_id);

CREATE INDEX IF NOT EXISTS documents_document_version_idx
    ON documents (document_version_id);

-- Quarantine one legacy version per existing `(corpus, source)` group. No owner
-- or approved scope is invented, and neither `approved` nor `ready` is written.
-- One-time quarantine backfill. The CTE links only the versions inserted by
-- THIS run, so a later `make db-schema` / migration rerun cannot attach
-- post-migration chunks to an already approved legacy version.
WITH inserted AS (
    INSERT INTO document_version (
        document_key, version_number, corpus, source, title, category, origin,
        access_scope, owner_subject, content_sha256, byte_size, mime_type,
        original_filename, storage_key, review_status, processing_status, created_by
    )
    SELECT
        'legacy-' || substr(encode(sha256(convert_to(coalesce(d.corpus, '') || ':' || coalesce(d.source, ''), 'UTF8')), 'hex'), 1, 24),
        1,
        coalesce(d.corpus, ''),
        coalesce(d.source, ''),
        coalesce(d.source, 'Legacy document'),
        'legacy',
        'legacy_import',
        NULL,
        NULL,
        encode(sha256(convert_to(coalesce(d.corpus, '') || ':' || coalesce(d.source, ''), 'UTF8')), 'hex'),
        0,
        'application/octet-stream',
        coalesce(d.source, 'legacy'),
        NULL,
        'pending_review',
        'legacy_indexed',
        'legacy_backfill'
    FROM documents d
    WHERE d.document_version_id IS NULL
      -- A rerun must not mint a second legacy version for a group that already
      -- has one, including when migration 010's unique (corpus, source) index
      -- is not present yet.
      AND NOT EXISTS (
          SELECT 1 FROM document_version v
           WHERE v.corpus = coalesce(d.corpus, '')
             AND v.source = coalesce(d.source, '')
      )
    GROUP BY d.corpus, d.source
    ON CONFLICT DO NOTHING
    RETURNING version_id, corpus, source
)
UPDATE documents d
SET document_version_id = inserted.version_id
FROM inserted
WHERE d.document_version_id IS NULL
  AND d.corpus = inserted.corpus
  AND d.source = inserted.source;

-- The only retrieval view. It requires explicit approval bound to the exact
-- content fingerprint, version, and scope, plus successful processing and no
-- removal. `admin_only` documents are review-only and never enter family
-- retrieval.
CREATE OR REPLACE VIEW approved_documents AS
SELECT d.*
FROM documents d
JOIN document_version v ON v.version_id = d.document_version_id
WHERE v.review_status = 'approved'
  AND v.processing_status = 'ready'
  AND v.removed_at IS NULL
  AND v.access_scope = 'family'
  AND v.approved_sha256 = v.content_sha256
  AND v.approved_scope = v.access_scope
  AND v.approved_version_number = v.version_number;

COMMIT;
