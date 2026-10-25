-- TRAN-AUTHORITY: designate an approved document version as the current source
-- of truth for a defined scope.
--
-- A reviewer may decide that one reviewed document version is authoritative for
-- a document, for one person, or for a set of selected assertions. Applying the
-- decision makes the assertions that version supports the *current* ones and
-- supersedes the conflicting current assertions inside the same scope.
--
-- Supersession is additive and reversible:
--
--   * a superseded assertion keeps its value, its citations, its review status,
--     and its review events; reads simply stop treating it as current;
--   * nothing here changes `review_status`, so an authoritative assertion is
--     still unverified until a separate review verifies it;
--   * a reversal is a new decision that restores what the reversed decision
--     replaced, without deleting either record.

-- The authority decision itself. One active decision per target, enforced by the
-- application inside a transaction-scoped advisory lock.
CREATE TABLE IF NOT EXISTS authority_decision (
    decision_id          BIGSERIAL PRIMARY KEY,
    -- 'document'   : every assertion this document version supports
    -- 'person'     : the assertions this version supports for one person
    -- 'assertions' : only the explicitly selected assertions
    scope                TEXT NOT NULL,
    document_version_id  BIGINT NOT NULL,
    document_ref         TEXT NOT NULL,
    content_sha256       TEXT NOT NULL,
    scope_person_id      BIGINT REFERENCES person(person_id) ON DELETE SET NULL,
    -- 'designate' makes a version authoritative; 'reversal' restores what the
    -- reversed decision replaced.
    kind                 TEXT NOT NULL DEFAULT 'designate',
    reason               TEXT NOT NULL,
    actor                TEXT NOT NULL,
    -- The preview the reviewer confirmed. An apply whose recomputed preview
    -- differs is refused, so a stale review cannot overwrite a newer change.
    preview_hash         TEXT NOT NULL,
    status               TEXT NOT NULL DEFAULT 'active',
    reverses_decision_id BIGINT REFERENCES authority_decision(decision_id),
    -- The decision this one retired on the same target, when there was one.
    supersedes_decision_id BIGINT REFERENCES authority_decision(decision_id),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_at           TIMESTAMPTZ,
    retired_by_decision_id BIGINT REFERENCES authority_decision(decision_id),
    CONSTRAINT authority_decision_scope_check CHECK (
        scope IN ('document', 'person', 'assertions')
    ),
    CONSTRAINT authority_decision_kind_check CHECK (
        kind IN ('designate', 'reversal')
    ),
    CONSTRAINT authority_decision_status_check CHECK (
        status IN ('active', 'superseded', 'reversed')
    ),
    CONSTRAINT authority_decision_sha_check CHECK (
        content_sha256 ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT authority_decision_reason_check CHECK (
        length(btrim(reason)) > 0
    ),
    CONSTRAINT authority_decision_actor_check CHECK (
        length(btrim(actor)) > 0
    ),
    CONSTRAINT authority_decision_person_scope_check CHECK (
        (scope = 'person') = (scope_person_id IS NOT NULL)
    ),
    CONSTRAINT authority_decision_reversal_check CHECK (
        (kind = 'reversal') = (reverses_decision_id IS NOT NULL)
    ),
    CONSTRAINT authority_decision_retired_check CHECK (
        (status = 'active') OR (retired_at IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS authority_decision_document_idx
    ON authority_decision (document_version_id, decision_id DESC);

CREATE INDEX IF NOT EXISTS authority_decision_person_idx
    ON authority_decision (scope_person_id, decision_id DESC)
 WHERE scope_person_id IS NOT NULL;

-- At most one active decision per scope target. The application retires the
-- previous one in the same transaction; this index is the backstop.
CREATE UNIQUE INDEX IF NOT EXISTS authority_decision_active_document_key
    ON authority_decision (document_version_id)
 WHERE status = 'active' AND scope = 'document';

CREATE UNIQUE INDEX IF NOT EXISTS authority_decision_active_assertions_key
    ON authority_decision (document_version_id)
 WHERE status = 'active' AND scope = 'assertions';

CREATE UNIQUE INDEX IF NOT EXISTS authority_decision_active_person_key
    ON authority_decision (scope_person_id)
 WHERE status = 'active' AND scope = 'person';

-- One row per assertion this decision made current. `superseded_assertion_id`
-- is the previous current assertion in the same slot, when there was one; its
-- value and evidence stay in place.
CREATE TABLE IF NOT EXISTS authority_decision_item (
    item_id                 BIGSERIAL PRIMARY KEY,
    decision_id             BIGINT NOT NULL
                                REFERENCES authority_decision(decision_id) ON DELETE CASCADE,
    assertion_id            BIGINT NOT NULL
                                REFERENCES assertion(assertion_id) ON DELETE CASCADE,
    superseded_assertion_id BIGINT
                                REFERENCES assertion(assertion_id) ON DELETE SET NULL,
    action                  TEXT NOT NULL,
    CONSTRAINT authority_decision_item_action_check CHECK (
        action IN ('added', 'replaced', 'removed', 'unchanged')
    ),
    CONSTRAINT authority_decision_item_key UNIQUE (decision_id, assertion_id)
);

CREATE INDEX IF NOT EXISTS authority_decision_item_assertion_idx
    ON authority_decision_item (assertion_id);

-- Append-only decision history: previewed, applied, reversed.
CREATE TABLE IF NOT EXISTS authority_decision_event (
    event_id    BIGSERIAL PRIMARY KEY,
    decision_id BIGINT NOT NULL
                    REFERENCES authority_decision(decision_id) ON DELETE CASCADE,
    action      TEXT NOT NULL,
    actor       TEXT NOT NULL,
    detail      JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT authority_decision_event_action_check CHECK (
        action IN ('previewed', 'applied', 'reversed')
    ),
    CONSTRAINT authority_decision_event_actor_check CHECK (
        length(btrim(actor)) > 0
    )
);

CREATE INDEX IF NOT EXISTS authority_decision_event_decision_idx
    ON authority_decision_event (decision_id, event_id);

-- Supersession is a property of the assertion row, so every existing read keeps
-- working once it filters on `superseded_at IS NULL`.
ALTER TABLE assertion
    ADD COLUMN IF NOT EXISTS superseded_at TIMESTAMPTZ;

ALTER TABLE assertion
    ADD COLUMN IF NOT EXISTS superseded_by_decision_id BIGINT
        REFERENCES authority_decision(decision_id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS assertion_current_idx
    ON assertion (person_id, assertion_kind, assertion_key)
 WHERE superseded_at IS NULL;

-- A decided decision's content is immutable. Only its lifecycle columns may
-- change, and only a reversal retires it.
CREATE OR REPLACE FUNCTION authority_decision_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.status <> 'active' THEN
        RAISE EXCEPTION 'a retired authority decision is immutable';
    END IF;
    IF NEW.decision_id IS DISTINCT FROM OLD.decision_id
       OR NEW.scope IS DISTINCT FROM OLD.scope
       OR NEW.document_version_id IS DISTINCT FROM OLD.document_version_id
       OR NEW.document_ref IS DISTINCT FROM OLD.document_ref
       OR NEW.content_sha256 IS DISTINCT FROM OLD.content_sha256
       OR NEW.scope_person_id IS DISTINCT FROM OLD.scope_person_id
       OR NEW.kind IS DISTINCT FROM OLD.kind
       OR NEW.reason IS DISTINCT FROM OLD.reason
       OR NEW.actor IS DISTINCT FROM OLD.actor
       OR NEW.preview_hash IS DISTINCT FROM OLD.preview_hash
       OR NEW.reverses_decision_id IS DISTINCT FROM OLD.reverses_decision_id
       OR NEW.supersedes_decision_id IS DISTINCT FROM OLD.supersedes_decision_id THEN
        RAISE EXCEPTION 'authority decision content is immutable';
    END IF;
    IF NEW.status = 'active' THEN
        RAISE EXCEPTION 'an authority decision cannot become active again';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS authority_decision_guard ON authority_decision;
CREATE TRIGGER authority_decision_guard
    BEFORE UPDATE ON authority_decision
    FOR EACH ROW EXECUTE FUNCTION authority_decision_guard();

CREATE OR REPLACE FUNCTION authority_decision_item_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'authority decision items are append-only';
END;
$$;

DROP TRIGGER IF EXISTS authority_decision_item_guard ON authority_decision_item;
CREATE TRIGGER authority_decision_item_guard
    BEFORE UPDATE OR DELETE ON authority_decision_item
    FOR EACH ROW EXECUTE FUNCTION authority_decision_item_guard();

CREATE OR REPLACE FUNCTION authority_decision_event_append_only()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'authority decision events are append-only';
END;
$$;

DROP TRIGGER IF EXISTS authority_decision_event_append_only ON authority_decision_event;
CREATE TRIGGER authority_decision_event_append_only
    BEFORE UPDATE OR DELETE ON authority_decision_event
    FOR EACH ROW EXECUTE FUNCTION authority_decision_event_append_only();

COMMENT ON COLUMN assertion.superseded_at IS
    'Set when an authority decision replaced this assertion; reads treat it as history.';
COMMENT ON COLUMN assertion.superseded_by_decision_id IS
    'The authority decision that superseded this assertion, for the audited history view.';
