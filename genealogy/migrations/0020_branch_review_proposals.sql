-- USR-005B: minimal versioned shared-genealogy fact proposals for branch review.
--
-- A proposal holds a proposed value, its prior approved snapshot, and the
-- evidence references needed for an independent decision. The authoritative
-- `assertion` row is not changed until an in-scope reviewer approves the
-- proposal; approval applies the value through the existing assertion review
-- validation and event workflow.
--
-- This is intentionally not a general-purpose document/photo pipeline and does
-- not expose member-private profile content.

CREATE TABLE IF NOT EXISTS genealogy_fact_proposal (
    proposal_id         BIGSERIAL PRIMARY KEY,
    person_id           BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    branch_key          TEXT NOT NULL,
    assertion_kind      TEXT NOT NULL,
    assertion_key       TEXT NOT NULL,
    prior_value_text    TEXT,
    prior_review_status TEXT,
    proposed_value_text TEXT NOT NULL,
    evidence_json       JSONB NOT NULL,
    note                TEXT,
    submitted_by        TEXT NOT NULL,
    submitted_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    status              TEXT NOT NULL DEFAULT 'pending_review',
    reviewed_by         TEXT,
    reviewed_at         TIMESTAMPTZ,
    review_note         TEXT,
    applied_assertion_id BIGINT REFERENCES assertion(assertion_id) ON DELETE SET NULL,
    CONSTRAINT genealogy_fact_proposal_status_check CHECK (
        status IN ('pending_review', 'approved', 'rejected', 'needs_changes')
    ),
    CONSTRAINT genealogy_fact_proposal_evidence_check CHECK (
        jsonb_typeof(evidence_json) = 'array' AND jsonb_array_length(evidence_json) > 0
    ),
    CONSTRAINT genealogy_fact_proposal_submitted_by_check CHECK (
        length(btrim(submitted_by)) > 0
    ),
    CONSTRAINT genealogy_fact_proposal_review_check CHECK (
        (status = 'pending_review'
         AND reviewed_by IS NULL AND reviewed_at IS NULL AND review_note IS NULL)
        OR
        (status <> 'pending_review'
         AND reviewed_by IS NOT NULL AND reviewed_at IS NOT NULL
         AND (status NOT IN ('rejected', 'needs_changes')
              OR (review_note IS NOT NULL AND length(btrim(review_note)) > 0)))
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS genealogy_fact_proposal_open_key
    ON genealogy_fact_proposal (person_id, assertion_kind, assertion_key)
 WHERE status = 'pending_review';

CREATE INDEX IF NOT EXISTS genealogy_fact_proposal_branch_status_idx
    ON genealogy_fact_proposal (branch_key, status, proposal_id);

CREATE INDEX IF NOT EXISTS genealogy_fact_proposal_person_idx
    ON genealogy_fact_proposal (person_id, proposal_id);

CREATE OR REPLACE FUNCTION genealogy_fact_proposal_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.status <> 'pending_review' THEN
        RAISE EXCEPTION 'a terminal fact proposal is immutable';
    END IF;
    IF NEW.proposal_id IS DISTINCT FROM OLD.proposal_id
       OR NEW.person_id IS DISTINCT FROM OLD.person_id
       OR NEW.branch_key IS DISTINCT FROM OLD.branch_key
       OR NEW.assertion_kind IS DISTINCT FROM OLD.assertion_kind
       OR NEW.assertion_key IS DISTINCT FROM OLD.assertion_key
       OR NEW.prior_value_text IS DISTINCT FROM OLD.prior_value_text
       OR NEW.prior_review_status IS DISTINCT FROM OLD.prior_review_status
       OR NEW.proposed_value_text IS DISTINCT FROM OLD.proposed_value_text
       OR NEW.evidence_json IS DISTINCT FROM OLD.evidence_json
       OR NEW.note IS DISTINCT FROM OLD.note
       OR NEW.submitted_by IS DISTINCT FROM OLD.submitted_by
       OR NEW.submitted_at IS DISTINCT FROM OLD.submitted_at THEN
        RAISE EXCEPTION 'fact proposal content is immutable';
    END IF;
    IF NEW.status NOT IN ('approved', 'rejected', 'needs_changes') THEN
        RAISE EXCEPTION 'a pending fact proposal may only move to a terminal decision';
    END IF;
    IF NEW.reviewed_by IS NULL OR btrim(NEW.reviewed_by) = '' THEN
        RAISE EXCEPTION 'a reviewer is required';
    END IF;
    IF NEW.reviewed_by = OLD.submitted_by THEN
        RAISE EXCEPTION 'a submitter cannot review their own fact proposal';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS genealogy_fact_proposal_guard
    ON genealogy_fact_proposal;
CREATE TRIGGER genealogy_fact_proposal_guard
    BEFORE UPDATE ON genealogy_fact_proposal
    FOR EACH ROW EXECUTE FUNCTION genealogy_fact_proposal_guard();

CREATE OR REPLACE FUNCTION genealogy_fact_proposal_insert_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.status <> 'pending_review'
       OR NEW.reviewed_by IS NOT NULL
       OR NEW.reviewed_at IS NOT NULL
       OR NEW.review_note IS NOT NULL
       OR NEW.applied_assertion_id IS NOT NULL THEN
        RAISE EXCEPTION 'a new fact proposal must start pending review';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS genealogy_fact_proposal_insert_guard
    ON genealogy_fact_proposal;
CREATE TRIGGER genealogy_fact_proposal_insert_guard
    BEFORE INSERT ON genealogy_fact_proposal
    FOR EACH ROW EXECUTE FUNCTION genealogy_fact_proposal_insert_guard();

CREATE TABLE IF NOT EXISTS genealogy_fact_proposal_event (
    event_id        BIGSERIAL PRIMARY KEY,
    proposal_id     BIGINT NOT NULL,
    person_id       BIGINT NOT NULL,
    branch_key      TEXT NOT NULL,
    event           TEXT NOT NULL,
    actor           TEXT NOT NULL,
    note            TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT genealogy_fact_proposal_event_event_check CHECK (
        event IN ('submitted', 'approved', 'rejected', 'needs_changes')
    ),
    CONSTRAINT genealogy_fact_proposal_event_actor_check CHECK (
        length(btrim(actor)) > 0
    )
);

CREATE INDEX IF NOT EXISTS genealogy_fact_proposal_event_proposal_idx
    ON genealogy_fact_proposal_event (proposal_id, event_id);

CREATE INDEX IF NOT EXISTS genealogy_fact_proposal_event_branch_idx
    ON genealogy_fact_proposal_event (branch_key, event_id);

CREATE OR REPLACE FUNCTION genealogy_fact_proposal_event_no_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'fact proposal events are append-only';
END;
$$;

DROP TRIGGER IF EXISTS genealogy_fact_proposal_event_no_mutation
    ON genealogy_fact_proposal_event;
CREATE TRIGGER genealogy_fact_proposal_event_no_mutation
    BEFORE UPDATE OR DELETE ON genealogy_fact_proposal_event
    FOR EACH ROW EXECUTE FUNCTION genealogy_fact_proposal_event_no_mutation();
