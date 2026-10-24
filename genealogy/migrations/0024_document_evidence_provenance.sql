-- TRAN-DOC-EXTRACT follow-up: text-document provenance and withdrawal.
--
-- Uploaded text documents have no PDF page. Their evidence records the source
-- line instead, and carries the document version so a withdrawn document can
-- take its evidence out of current answers without deleting the history.

ALTER TABLE assertion_evidence
    ADD COLUMN IF NOT EXISTS document_version_id BIGINT,
    ADD COLUMN IF NOT EXISTS withdrawn_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS assertion_evidence_document_version_idx
    ON assertion_evidence (document_version_id)
 WHERE document_version_id IS NOT NULL;

-- The line a proposal quoted, so the applied evidence can cite "line N".
ALTER TABLE document_proposal
    ADD COLUMN IF NOT EXISTS support_line INTEGER NOT NULL DEFAULT 1;

COMMENT ON COLUMN assertion_evidence.document_version_id IS
    'The uploaded document version that produced this evidence, when known.';
COMMENT ON COLUMN assertion_evidence.withdrawn_at IS
    'Set when the producing document is withdrawn; withdrawn evidence is not shown in answers.';
