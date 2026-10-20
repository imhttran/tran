-- TRAN-002 provenance hardening: classify every source document.
--
-- Only an `original` source document can support a verified genealogy
-- assertion. Transcriptions, translations, catalogs, and other derivatives
-- remain useful evidence but never become independent proof.

ALTER TABLE source_document
    ADD COLUMN IF NOT EXISTS source_kind TEXT NOT NULL DEFAULT 'derivative';

ALTER TABLE source_document
    DROP CONSTRAINT IF EXISTS source_document_source_kind_check;
ALTER TABLE source_document
    ADD CONSTRAINT source_document_source_kind_check CHECK (
        source_kind IN ('original', 'transcription', 'translation', 'catalog', 'derivative')
    );

CREATE INDEX IF NOT EXISTS source_document_kind_idx
    ON source_document (source_kind);
