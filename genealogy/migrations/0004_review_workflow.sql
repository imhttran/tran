-- TRAN-002D genealogy migration 0004: evidence-backed review workflow.
--
-- This migration:
-- * extends assertion.review_status to include the terminal 'rejected' state
--   (the 'candidate' state documented by TRAN-002D maps to the existing
--   default 'unverified');
-- * adds assertion_review_event as the append-only review audit trail;
-- * adds assertion_review_baseline to store the evidence fingerprint recorded
--   when an assertion becomes verified;
-- * adds a trigger on assertion_evidence so that evidence mutation on a
--   verified assertion forces it back to needs_review (reopening).
--
-- assertion_kind conventions used by the validation rules (see
-- src/genealogy/validation.rs):
--   * 'branch_claim' : value_text is a family_branch.branch_key. Two distinct
--     verified branch_claim assertions for one person are incompatible.
--   * 'vital_status' : assertion_key is the vital name ('birth' | 'death' |
--     other) and value_text is the status value (e.g. 'alive', 'deceased').
--     Two assertions with the same assertion_key and different value_text are
--     contradictory.
--   * 'parent_child' : value_text names the child person_key; used by the
--     cycle and self-parenting checks alongside parent_child_link.
--
-- The schema stays inside the genealogy database. No RAG migration or
-- documents table is touched.

ALTER TABLE assertion
    DROP CONSTRAINT IF EXISTS assertion_review_status_check;
ALTER TABLE assertion
    ADD CONSTRAINT assertion_review_status_check CHECK (
        review_status IN ('unverified', 'needs_review', 'verified', 'disputed', 'rejected')
    );

-- Append-only review audit trail. There is no UPDATE and no DELETE path: the
-- trigger below rejects any attempt to rewrite history.
CREATE TABLE IF NOT EXISTS assertion_review_event (
    assertion_review_event_id BIGSERIAL PRIMARY KEY,
    assertion_id              BIGINT NOT NULL REFERENCES assertion(assertion_id) ON DELETE CASCADE,
    reviewer                  TEXT NOT NULL,
    event_at                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    from_status               TEXT NOT NULL,
    to_status                 TEXT NOT NULL,
    evidence_ref              TEXT NOT NULL,
    note                      TEXT
);

CREATE INDEX IF NOT EXISTS assertion_review_event_assertion_idx
    ON assertion_review_event (assertion_id);

CREATE OR REPLACE FUNCTION assertion_review_event_append_only() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'assertion_review_event rows are append-only (no UPDATE/DELETE)';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS assertion_review_event_no_update ON assertion_review_event;
CREATE TRIGGER assertion_review_event_no_update
    BEFORE UPDATE OR DELETE ON assertion_review_event
    FOR EACH ROW EXECUTE FUNCTION assertion_review_event_append_only();

-- The evidence fingerprint at the moment an assertion became verified.
CREATE TABLE IF NOT EXISTS assertion_review_baseline (
    assertion_id        BIGINT PRIMARY KEY REFERENCES assertion(assertion_id) ON DELETE CASCADE,
    evidence_fingerprint TEXT NOT NULL,
    recorded_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Evidence mutation on a verified assertion reopens it. The trigger runs on
-- INSERT and DELETE of assertion_evidence; UPDATE is not exercised by the
-- re-import pipeline (it deletes and re-inserts), but is handled for safety by
-- comparing against the stored fingerprint in the application layer.
CREATE OR REPLACE FUNCTION assertion_evidence_change_reopens() RETURNS trigger AS $$
DECLARE
    target_id BIGINT;
BEGIN
    target_id := COALESCE(NEW.assertion_id, OLD.assertion_id);
    UPDATE assertion
        SET review_status = 'needs_review'
        WHERE assertion_id = target_id
          AND review_status = 'verified';
    DELETE FROM assertion_review_baseline WHERE assertion_id = target_id;
    RETURN COALESCE(NEW, OLD);
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS assertion_evidence_change_reopens ON assertion_evidence;
CREATE TRIGGER assertion_evidence_change_reopens
    AFTER INSERT OR DELETE ON assertion_evidence
    FOR EACH ROW EXECUTE FUNCTION assertion_evidence_change_reopens();
