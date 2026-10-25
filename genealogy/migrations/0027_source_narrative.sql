-- TRAN-002I: source-linked historical narratives.
--
-- A narrative passage is a verbatim excerpt of a source document, with the PDF
-- page range and section it came from. Passages are reviewed and published
-- separately from structured genealogy facts: approving a passage never promotes
-- a relationship or a date to verified.
--
-- Passages live beside member contributions, not inside them. `biography`,
-- `my_story`, and `family_memories` remain owned by the profile revision tables
-- and are never written by the importer.

BEGIN;

CREATE TABLE IF NOT EXISTS source_narrative_passage (
    passage_id         BIGSERIAL PRIMARY KEY,
    -- Stable identity of the source slot (document, page, section).
    slot_key           TEXT NOT NULL,
    -- Identity of this exact revision. A changed source text gets a new key so
    -- the approved wording is never overwritten before review.
    passage_key        TEXT NOT NULL,
    source_document_id BIGINT NOT NULL REFERENCES source_document(source_document_id) ON DELETE CASCADE,
    language           TEXT NOT NULL CHECK (language IN ('vi', 'en')),
    section_title      TEXT,
    page_start         INTEGER NOT NULL CHECK (page_start >= 1),
    page_end           INTEGER NOT NULL CHECK (page_end >= page_start),
    ordinal            INTEGER NOT NULL CHECK (ordinal >= 1),
    body               TEXT NOT NULL CHECK (length(btrim(body)) > 0),
    content_sha256     TEXT NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    review_status      TEXT NOT NULL DEFAULT 'pending'
                       CHECK (review_status IN ('pending', 'approved', 'rejected')),
    visibility         TEXT NOT NULL DEFAULT 'admin_only'
                       CHECK (visibility IN ('admin_only', 'family')),
    reviewed_by        TEXT,
    reviewed_at        TIMESTAMPTZ,
    review_note        TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT source_narrative_passage_key_key UNIQUE (passage_key),
    CONSTRAINT source_narrative_passage_approved_metadata CHECK (
        review_status <> 'approved'
        OR (reviewed_by IS NOT NULL AND reviewed_at IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS source_narrative_passage_slot_idx
    ON source_narrative_passage (slot_key);
CREATE INDEX IF NOT EXISTS source_narrative_passage_source_idx
    ON source_narrative_passage (source_document_id, page_start, ordinal);
CREATE INDEX IF NOT EXISTS source_narrative_passage_queue_idx
    ON source_narrative_passage (review_status, created_at);

-- One link row per (passage, person). `resolved` comes from a structural signal
-- (a section heading or an explicit catalog identifier); `ambiguous` comes from a
-- name mention and must be decided by a reviewer. Only `resolved` and
-- `confirmed` links can place a passage on a family profile.
CREATE TABLE IF NOT EXISTS source_narrative_link (
    passage_id   BIGINT NOT NULL REFERENCES source_narrative_passage(passage_id) ON DELETE CASCADE,
    person_id    BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    link_status  TEXT NOT NULL CHECK (link_status IN ('resolved', 'ambiguous', 'confirmed', 'rejected')),
    match_kind   TEXT NOT NULL CHECK (match_kind IN ('section_heading', 'explicit_id', 'name_mention')),
    evidence_note TEXT,
    reviewed_by  TEXT,
    reviewed_at  TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (passage_id, person_id),
    CONSTRAINT source_narrative_link_review_metadata CHECK (
        link_status NOT IN ('confirmed', 'rejected')
        OR (reviewed_by IS NOT NULL AND reviewed_at IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS source_narrative_link_person_idx
    ON source_narrative_link (person_id, link_status);

-- Append-only audit for passage and link decisions.
CREATE TABLE IF NOT EXISTS source_narrative_review_event (
    event_id   BIGSERIAL PRIMARY KEY,
    passage_id BIGINT NOT NULL,
    person_id  BIGINT,
    actor      TEXT NOT NULL CHECK (length(btrim(actor)) > 0),
    action     TEXT NOT NULL CHECK (action IN ('passage_review', 'link_review')),
    from_status TEXT,
    to_status  TEXT NOT NULL,
    visibility TEXT,
    note       TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS source_narrative_review_event_passage_idx
    ON source_narrative_review_event (passage_id, event_id);

CREATE OR REPLACE FUNCTION source_narrative_review_event_append_only()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'narrative review events are append-only';
END;
$$;

DROP TRIGGER IF EXISTS source_narrative_review_event_append_only
    ON source_narrative_review_event;
CREATE TRIGGER source_narrative_review_event_append_only
    BEFORE UPDATE OR DELETE ON source_narrative_review_event
    FOR EACH ROW EXECUTE FUNCTION source_narrative_review_event_append_only();

COMMIT;
