-- TRAN-DOC-EXTRACT: reviewed document-to-genealogy proposals.
--
-- An uploaded family document can be parsed into draft proposals. Nothing here
-- touches `person`, `assertion`, or the link tables until a reviewer approves
-- the individual proposal and applies the extraction. Proposals keep the exact
-- supporting text and the document-version provenance so a decision can be
-- audited and a stale extraction cannot be applied unnoticed.
--
-- Proposals live in the genealogy database because they create genealogy facts.
-- The RAG document version id is recorded as an opaque provenance value; the
-- API revalidates it against the RAG database before applying.

CREATE TABLE IF NOT EXISTS document_extraction (
    extraction_id       BIGSERIAL PRIMARY KEY,
    document_version_id BIGINT NOT NULL,
    corpus              TEXT NOT NULL,
    document_source     TEXT NOT NULL,
    content_sha256      TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'draft',
    created_by          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    applied_by          TEXT,
    applied_at          TIMESTAMPTZ,
    CONSTRAINT document_extraction_status_check CHECK (
        status IN ('draft', 'applied', 'superseded')
    ),
    CONSTRAINT document_extraction_created_by_check CHECK (
        length(btrim(created_by)) > 0
    ),
    CONSTRAINT document_extraction_applied_check CHECK (
        (status <> 'applied')
        OR (applied_by IS NOT NULL AND applied_at IS NOT NULL)
    )
);

-- One extraction per reviewed document version and content hash. A re-import or
-- replacement changes the hash and therefore cannot inherit an old decision.
CREATE UNIQUE INDEX IF NOT EXISTS document_extraction_version_key
    ON document_extraction (document_version_id, content_sha256);

CREATE INDEX IF NOT EXISTS document_extraction_version_idx
    ON document_extraction (document_version_id, extraction_id DESC);

CREATE TABLE IF NOT EXISTS document_proposal (
    proposal_id          BIGSERIAL PRIMARY KEY,
    extraction_id        BIGINT NOT NULL
                             REFERENCES document_extraction(extraction_id) ON DELETE CASCADE,
    ordinal              INTEGER NOT NULL CHECK (ordinal >= 1),
    -- person: create or match one person reference
    -- parent_child / spouse: relation between subject_name and target_name
    -- alias: searchable name variant for subject_name
    -- social_link: explicit http(s) URL for subject_name
    kind                 TEXT NOT NULL,
    subject_name         TEXT NOT NULL,
    target_name          TEXT,
    resolution           TEXT NOT NULL,
    resolved_person_key  TEXT,
    candidates_json      JSONB NOT NULL DEFAULT '[]'::jsonb,
    value_text           TEXT,
    support_text         TEXT NOT NULL,
    status               TEXT NOT NULL DEFAULT 'proposed',
    decided_by           TEXT,
    decided_at           TIMESTAMPTZ,
    applied_at           TIMESTAMPTZ,
    applied_result       JSONB,
    CONSTRAINT document_proposal_kind_check CHECK (
        kind IN ('person', 'parent_child', 'spouse', 'alias', 'social_link')
    ),
    CONSTRAINT document_proposal_resolution_check CHECK (
        resolution IN ('matched', 'ambiguous', 'unresolved', 'create')
    ),
    CONSTRAINT document_proposal_status_check CHECK (
        status IN ('proposed', 'approved', 'rejected', 'applied')
    ),
    CONSTRAINT document_proposal_subject_check CHECK (
        length(btrim(subject_name)) > 0
    ),
    CONSTRAINT document_proposal_candidates_check CHECK (
        jsonb_typeof(candidates_json) = 'array'
    ),
    CONSTRAINT document_proposal_decision_check CHECK (
        (status IN ('proposed', 'applied'))
        OR (decided_by IS NOT NULL AND decided_at IS NOT NULL)
    ),
    CONSTRAINT document_proposal_target_check CHECK (
        (kind NOT IN ('parent_child', 'spouse')) OR (target_name IS NOT NULL)
    ),
    CONSTRAINT document_proposal_value_check CHECK (
        (kind NOT IN ('alias', 'social_link')) OR (value_text IS NOT NULL)
    ),
    CONSTRAINT document_proposal_applied_check CHECK (
        (status <> 'applied') OR (applied_at IS NOT NULL)
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS document_proposal_extraction_ordinal_key
    ON document_proposal (extraction_id, ordinal);

CREATE INDEX IF NOT EXISTS document_proposal_extraction_status_idx
    ON document_proposal (extraction_id, status, ordinal);

-- A decided proposal keeps its content and decision; only the apply fields may
-- change afterwards. This stops a concurrent reviewer from rewriting history.
CREATE OR REPLACE FUNCTION document_proposal_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.status IN ('rejected', 'applied') THEN
        RAISE EXCEPTION 'a decided document proposal is immutable';
    END IF;
    IF NEW.proposal_id IS DISTINCT FROM OLD.proposal_id
       OR NEW.extraction_id IS DISTINCT FROM OLD.extraction_id
       OR NEW.ordinal IS DISTINCT FROM OLD.ordinal
       OR NEW.kind IS DISTINCT FROM OLD.kind
       OR NEW.subject_name IS DISTINCT FROM OLD.subject_name
       OR NEW.target_name IS DISTINCT FROM OLD.target_name
       -- `value_text` may be corrected while the proposal is still proposed;
       -- `support_text` is provenance and never changes.
       OR NEW.support_text IS DISTINCT FROM OLD.support_text THEN
        RAISE EXCEPTION 'document proposal content is immutable';
    END IF;
    -- An approved proposal may still be revised or rejected before it is
    -- applied; once applied it is terminal.
    IF OLD.status = 'approved' AND NEW.status NOT IN ('approved', 'rejected', 'applied') THEN
        RAISE EXCEPTION 'an approved document proposal may only be revised, rejected, or applied';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS document_proposal_guard ON document_proposal;
CREATE TRIGGER document_proposal_guard
    BEFORE UPDATE ON document_proposal
    FOR EACH ROW EXECUTE FUNCTION document_proposal_guard();

-- Append-only extraction history: who extracted, who decided, what was applied.
CREATE TABLE IF NOT EXISTS document_extraction_event (
    event_id      BIGSERIAL PRIMARY KEY,
    extraction_id BIGINT NOT NULL
                      REFERENCES document_extraction(extraction_id) ON DELETE CASCADE,
    action        TEXT NOT NULL,
    actor         TEXT NOT NULL,
    detail        JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT document_extraction_event_action_check CHECK (
        action IN ('extracted', 'decided', 'applied', 'superseded')
    ),
    CONSTRAINT document_extraction_event_actor_check CHECK (
        length(btrim(actor)) > 0
    )
);

CREATE INDEX IF NOT EXISTS document_extraction_event_extraction_idx
    ON document_extraction_event (extraction_id, event_id);

CREATE OR REPLACE FUNCTION document_extraction_event_append_only()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'document extraction events are append-only';
END;
$$;

DROP TRIGGER IF EXISTS document_extraction_event_append_only ON document_extraction_event;
CREATE TRIGGER document_extraction_event_append_only
    BEFORE UPDATE OR DELETE ON document_extraction_event
    FOR EACH ROW EXECUTE FUNCTION document_extraction_event_append_only();
