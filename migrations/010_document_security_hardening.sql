-- Migration 010: document lifecycle security and worker hardening.
--
-- Additive only. Existing document content, chunks, citations, and lifecycle
-- history are preserved.
--
--   * owner_issuer binds an editor's ownership to the issuer-qualified
--     (issuer, subject) identity, not a subject string alone.
--   * lease_expires_at lets another worker reclaim a job abandoned by a crashed
--     worker without allowing the old worker to publish afterwards.
--   * a unique (corpus, source) index guarantees that two submissions cannot
--     share a chunk source and overwrite one another.
--
-- Apply with:
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
--     -f migrations/010_document_security_hardening.sql

BEGIN;

ALTER TABLE document_version
    ADD COLUMN IF NOT EXISTS owner_issuer TEXT;

ALTER TABLE document_processing_job
    ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ;

-- Existing operator/legacy versions keep a NULL owner_issuer and remain
-- admin-only for ownership purposes. New submissions record the issuer.

-- A source must identify exactly one version. The full document key is embedded
-- in new sources; this index makes an accidental collision fail closed instead
-- of silently replacing another submission's chunks.
CREATE UNIQUE INDEX IF NOT EXISTS document_version_corpus_source_uniq
    ON document_version (corpus, source);

CREATE INDEX IF NOT EXISTS document_processing_job_lease_idx
    ON document_processing_job (status, lease_expires_at);

COMMIT;
