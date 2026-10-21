-- USR-004B phase 1: profile revision lifecycle (additive).
--
-- The existing onboarding_profile_value row remains the last approved value.
-- onboarding_profile_revision rows hold owner drafts and submitted proposals.
-- This migration adds only the state, visibility, version, and review metadata
-- needed to preserve the approved value until an independent reviewer decides.
--
-- No value is deleted or overwritten by the upgrade. Legacy revision rows are
-- numbered per (person, field) and retained as approved history with an
-- explicit legacy review marker; their content is not changed.

DO $$
BEGIN
    IF to_regclass('onboarding_profile_value') IS NULL
       OR to_regclass('onboarding_profile_revision') IS NULL THEN
        RAISE EXCEPTION
            'migration 0012 requires onboarding profile tables from migration 0007';
    END IF;
END;
$$;

ALTER TABLE onboarding_profile_value
    ADD COLUMN IF NOT EXISTS visibility TEXT NOT NULL DEFAULT 'owner_private',
    ADD COLUMN IF NOT EXISTS approved_revision_number BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS approved_revision_id BIGINT
        REFERENCES onboarding_profile_revision(revision_id) ON DELETE SET NULL;

ALTER TABLE onboarding_profile_revision
    ADD COLUMN IF NOT EXISTS revision_number BIGINT,
    ADD COLUMN IF NOT EXISTS prior_approved_revision_number BIGINT,
    ADD COLUMN IF NOT EXISTS status TEXT,
    ADD COLUMN IF NOT EXISTS visibility TEXT,
    ADD COLUMN IF NOT EXISTS submitted_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS reviewed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS reviewed_by TEXT,
    ADD COLUMN IF NOT EXISTS review_note TEXT;

-- Give existing revisions a stable per-field version. The values themselves are
-- untouched; only the new metadata column is backfilled. Any existing numbered
-- row sets the base, so a retry after a partial run cannot collide with a
-- version that was already allocated.
WITH numbered AS (
    SELECT revision.revision_id,
           (SELECT COALESCE(max(existing.revision_number), 0)
              FROM onboarding_profile_revision AS existing
             WHERE existing.person_id = revision.person_id
               AND existing.field_key = revision.field_key
               AND existing.revision_number IS NOT NULL) +
           row_number() OVER (
               PARTITION BY revision.person_id, revision.field_key
               ORDER BY revision.changed_at, revision.revision_id
           ) AS version
      FROM onboarding_profile_revision AS revision
     WHERE revision.revision_number IS NULL
)
UPDATE onboarding_profile_revision AS revision
   SET revision_number = numbered.version
  FROM numbered
 WHERE revision.revision_id = numbered.revision_id;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM onboarding_profile_revision WHERE revision_number IS NULL
    ) THEN
        RAISE EXCEPTION 'profile revision numbering backfill left NULL rows';
    END IF;
END;
$$;

UPDATE onboarding_profile_revision
   SET prior_approved_revision_number = COALESCE(prior_approved_revision_number, 0)
 WHERE prior_approved_revision_number IS NULL;

ALTER TABLE onboarding_profile_revision
    ALTER COLUMN revision_number SET NOT NULL,
    ALTER COLUMN prior_approved_revision_number SET NOT NULL,
    ALTER COLUMN prior_approved_revision_number SET DEFAULT 0;

-- Existing revisions were written before the review lifecycle existed. Only
-- rows whose status is still NULL are legacy rows: new rows receive the status
-- default, and a re-run must never rewrite current drafts or pending reviews.
-- Legacy rows keep their values and are marked as imported history with no
-- fabricated reviewer or submission timestamps.
UPDATE onboarding_profile_revision
   SET status = 'approved',
       visibility = COALESCE(visibility, 'owner_private'),
       submitted_at = NULL,
       reviewed_at = NULL,
       reviewed_by = NULL,
       review_note = 'pre-0012 history; review provenance unavailable'
 WHERE status IS NULL;

ALTER TABLE onboarding_profile_revision
    ALTER COLUMN status SET DEFAULT 'draft',
    ALTER COLUMN status SET NOT NULL,
    ALTER COLUMN visibility SET DEFAULT 'owner_private',
    ALTER COLUMN visibility SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_profile_value_visibility_check'
           AND conrelid = 'onboarding_profile_value'::regclass
    ) THEN
        ALTER TABLE onboarding_profile_value
            ADD CONSTRAINT onboarding_profile_value_visibility_check
            CHECK (visibility IN ('owner_private', 'family'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_profile_revision_status_check'
           AND conrelid = 'onboarding_profile_revision'::regclass
    ) THEN
        ALTER TABLE onboarding_profile_revision
            ADD CONSTRAINT onboarding_profile_revision_status_check
            CHECK (status IN (
                'draft', 'pending_review', 'approved', 'rejected', 'needs_changes'
            ));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_profile_revision_visibility_check'
           AND conrelid = 'onboarding_profile_revision'::regclass
    ) THEN
        ALTER TABLE onboarding_profile_revision
            ADD CONSTRAINT onboarding_profile_revision_visibility_check
            CHECK (visibility IN ('owner_private', 'family'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_profile_revision_review_note_len_check'
           AND conrelid = 'onboarding_profile_revision'::regclass
    ) THEN
        ALTER TABLE onboarding_profile_revision
            ADD CONSTRAINT onboarding_profile_revision_review_note_len_check
            CHECK (review_note IS NULL OR char_length(review_note) <= 2000);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_profile_revision_review_note_required_check'
           AND conrelid = 'onboarding_profile_revision'::regclass
    ) THEN
        ALTER TABLE onboarding_profile_revision
            ADD CONSTRAINT onboarding_profile_revision_review_note_required_check
            CHECK (
                status NOT IN ('rejected', 'needs_changes')
                OR (review_note IS NOT NULL AND char_length(btrim(review_note)) > 0)
            );
    END IF;
END;
$$;

CREATE UNIQUE INDEX IF NOT EXISTS onboarding_profile_revision_version_key
    ON onboarding_profile_revision (person_id, field_key, revision_number);

-- At most one open draft or pending revision per field for the pre-0013
-- lifecycle. Migration 0013 replaces this with an actor-scoped index; re-running
-- 0012 after 0013 must not recreate the stricter index.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_class
         WHERE relname = 'onboarding_profile_revision_open_actor_key'
           AND relkind = 'i'
    ) THEN
        EXECUTE 'CREATE UNIQUE INDEX IF NOT EXISTS onboarding_profile_revision_open_key
                   ON onboarding_profile_revision (person_id, field_key)
                WHERE status IN (''draft'', ''pending_review'')';
    END IF;
END;
$$;

CREATE INDEX IF NOT EXISTS onboarding_profile_revision_status_idx
    ON onboarding_profile_revision (status, person_id, changed_at);

-- Append-only transition log. The revision row keeps its current state for
-- efficient reads; this table preserves every submitted/review decision.
CREATE TABLE IF NOT EXISTS onboarding_profile_review_event (
    event_id      BIGSERIAL PRIMARY KEY,
    revision_id   BIGINT NOT NULL
                  REFERENCES onboarding_profile_revision(revision_id) ON DELETE CASCADE,
    person_id     BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    event         TEXT NOT NULL,
    actor         TEXT NOT NULL,
    note          TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT onboarding_profile_review_event_event_check CHECK (
        event IN ('draft', 'pending_review', 'approved', 'rejected', 'needs_changes')
    ),
    CONSTRAINT onboarding_profile_review_event_actor_not_blank CHECK (
        length(btrim(actor)) > 0
    )
);

CREATE INDEX IF NOT EXISTS onboarding_profile_review_event_revision_idx
    ON onboarding_profile_review_event (revision_id, created_at);

-- The approved value is not an independent writable row: an update must point
-- at an approved revision for the same person/field, copy that revision's text
-- and visibility, and advance the approved revision number monotonically. This
-- makes a stale or direct value update unable to defeat version conflicts.
CREATE OR REPLACE FUNCTION onboarding_profile_value_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    approved_revision record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.approved_revision_id IS NULL THEN
            RAISE EXCEPTION 'an updated approved value requires an approved revision';
        END IF;
        IF NEW.approved_revision_number <= OLD.approved_revision_number THEN
            RAISE EXCEPTION 'approved revision number must advance';
        END IF;
    END IF;

    IF NEW.approved_revision_id IS NULL THEN
        RAISE EXCEPTION 'a new approved value requires an approved revision';
    END IF;

    SELECT person_id, field_key, revision_number, new_value_text, visibility, status
      INTO approved_revision
      FROM onboarding_profile_revision
     WHERE revision_id = NEW.approved_revision_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'approved revision does not exist';
    END IF;
    IF approved_revision.status <> 'approved' THEN
        RAISE EXCEPTION 'only an approved revision may update the approved value';
    END IF;
    IF approved_revision.person_id <> NEW.person_id
       OR approved_revision.field_key <> NEW.field_key THEN
        RAISE EXCEPTION 'approved revision belongs to another person or field';
    END IF;
    IF approved_revision.revision_number <> NEW.approved_revision_number THEN
        RAISE EXCEPTION 'approved revision number does not match the value row';
    END IF;
    IF approved_revision.new_value_text <> NEW.value_text
       OR approved_revision.visibility <> NEW.visibility THEN
        RAISE EXCEPTION 'approved value does not match the approved revision';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS onboarding_profile_value_guard
    ON onboarding_profile_value;
CREATE TRIGGER onboarding_profile_value_guard
    BEFORE INSERT OR UPDATE ON onboarding_profile_value
    FOR EACH ROW EXECUTE FUNCTION onboarding_profile_value_guard();

-- Revision immutability and review rules. Draft content may change; submitted
-- content may not. A terminal decision must name a reviewer distinct from the
-- submitter, and terminal rows are frozen. New revisions must start as drafts.
CREATE OR REPLACE FUNCTION onboarding_profile_revision_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.status IN ('approved', 'rejected', 'needs_changes') THEN
        RAISE EXCEPTION 'a terminal profile revision is immutable';
    END IF;

    IF NEW.revision_id IS DISTINCT FROM OLD.revision_id
       OR NEW.person_id IS DISTINCT FROM OLD.person_id
       OR NEW.field_key IS DISTINCT FROM OLD.field_key
       OR NEW.revision_number IS DISTINCT FROM OLD.revision_number
       OR NEW.prior_value_text IS DISTINCT FROM OLD.prior_value_text
       OR NEW.prior_approved_revision_number IS DISTINCT FROM OLD.prior_approved_revision_number
       OR NEW.changed_by IS DISTINCT FROM OLD.changed_by THEN
        RAISE EXCEPTION 'profile revision identity and provenance are immutable';
    END IF;

    IF OLD.status <> 'draft'
       AND NEW.changed_at IS DISTINCT FROM OLD.changed_at THEN
        RAISE EXCEPTION 'changed_at is immutable after submission';
    END IF;

    IF OLD.status = 'draft' THEN
        IF NEW.status NOT IN ('draft', 'pending_review') THEN
            RAISE EXCEPTION 'a draft may only remain draft or be submitted';
        END IF;
        IF NEW.status = 'draft' THEN
            IF NEW.submitted_at IS NOT NULL
               OR NEW.reviewed_at IS NOT NULL
               OR NEW.reviewed_by IS NOT NULL
               OR NEW.review_note IS NOT NULL THEN
                RAISE EXCEPTION 'draft review metadata must be empty';
            END IF;
        ELSE
            IF NEW.submitted_at IS NULL THEN
                RAISE EXCEPTION 'a submitted revision requires submitted_at';
            END IF;
            IF NEW.reviewed_at IS NOT NULL
               OR NEW.reviewed_by IS NOT NULL
               OR NEW.review_note IS NOT NULL THEN
                RAISE EXCEPTION 'a submitted revision cannot carry review metadata';
            END IF;
        END IF;
    ELSIF OLD.status = 'pending_review' THEN
        IF NEW.status NOT IN ('approved', 'rejected', 'needs_changes') THEN
            RAISE EXCEPTION 'pending review may only move to a terminal decision';
        END IF;
        IF NEW.new_value_text IS DISTINCT FROM OLD.new_value_text
           OR NEW.visibility IS DISTINCT FROM OLD.visibility THEN
            RAISE EXCEPTION 'submitted revision content is immutable';
        END IF;
        IF NEW.reviewed_by IS NULL OR btrim(NEW.reviewed_by) = '' THEN
            RAISE EXCEPTION 'a reviewer is required';
        END IF;
        IF NEW.reviewed_by = OLD.changed_by THEN
            RAISE EXCEPTION 'a submitter cannot review their own revision';
        END IF;
        IF NEW.reviewed_at IS NULL THEN
            RAISE EXCEPTION 'a review decision requires reviewed_at';
        END IF;
        IF NEW.submitted_at IS DISTINCT FROM OLD.submitted_at THEN
            RAISE EXCEPTION 'submitted_at is immutable';
        END IF;
    ELSE
        RAISE EXCEPTION 'a terminal profile revision is immutable';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS onboarding_profile_revision_guard
    ON onboarding_profile_revision;
CREATE TRIGGER onboarding_profile_revision_guard
    BEFORE UPDATE ON onboarding_profile_revision
    FOR EACH ROW EXECUTE FUNCTION onboarding_profile_revision_guard();

-- Direct deletion removes immutable history, but deleting the owning person must
-- still cascade. A direct child delete sees its person row; a parent cascade
-- runs after the person row is gone.
CREATE OR REPLACE FUNCTION onboarding_profile_revision_no_delete()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM person WHERE person_id = OLD.person_id) THEN
        RAISE EXCEPTION 'profile revision history cannot be deleted directly';
    END IF;
    RETURN OLD;
END;
$$;

DROP TRIGGER IF EXISTS onboarding_profile_revision_no_delete
    ON onboarding_profile_revision;
CREATE TRIGGER onboarding_profile_revision_no_delete
    BEFORE DELETE ON onboarding_profile_revision
    FOR EACH ROW EXECUTE FUNCTION onboarding_profile_revision_no_delete();

-- New revisions begin as owner drafts. A direct INSERT cannot mint an already
-- approved or already reviewed row.
CREATE OR REPLACE FUNCTION onboarding_profile_revision_insert_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.status <> 'draft' THEN
        RAISE EXCEPTION 'a new profile revision must start as a draft';
    END IF;
    IF NEW.submitted_at IS NOT NULL
       OR NEW.reviewed_at IS NOT NULL
       OR NEW.reviewed_by IS NOT NULL
       OR NEW.review_note IS NOT NULL THEN
        RAISE EXCEPTION 'a new profile revision cannot carry review metadata';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS onboarding_profile_revision_insert_guard
    ON onboarding_profile_revision;
CREATE TRIGGER onboarding_profile_revision_insert_guard
    BEFORE INSERT ON onboarding_profile_revision
    FOR EACH ROW EXECUTE FUNCTION onboarding_profile_revision_insert_guard();

-- The transition log is append-only for normal writes. Person deletion still
-- cascades through the revision and event foreign keys.
CREATE OR REPLACE FUNCTION onboarding_profile_review_event_no_update()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'profile review events are append-only';
END;
$$;

DROP TRIGGER IF EXISTS onboarding_profile_review_event_no_update
    ON onboarding_profile_review_event;
CREATE TRIGGER onboarding_profile_review_event_no_update
    BEFORE UPDATE ON onboarding_profile_review_event
    FOR EACH ROW EXECUTE FUNCTION onboarding_profile_review_event_no_update();

-- Direct event deletion is forbidden; deleting the owning revision or person
-- must still cascade.
CREATE OR REPLACE FUNCTION onboarding_profile_review_event_no_delete()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM onboarding_profile_revision WHERE revision_id = OLD.revision_id
    ) THEN
        RAISE EXCEPTION 'profile review events cannot be deleted directly';
    END IF;
    RETURN OLD;
END;
$$;

DROP TRIGGER IF EXISTS onboarding_profile_review_event_no_delete
    ON onboarding_profile_review_event;
CREATE TRIGGER onboarding_profile_review_event_no_delete
    BEFORE DELETE ON onboarding_profile_review_event
    FOR EACH ROW EXECUTE FUNCTION onboarding_profile_review_event_no_delete();
