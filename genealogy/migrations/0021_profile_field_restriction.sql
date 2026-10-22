-- USR-006-P: owner-initiated privacy restriction for approved profile values.
--
-- Decision A2: an owner may immediately restrict their own approved,
-- family-visible profile value from `family` to `owner_private`. The historical
-- approved revision is never rewritten; the restriction is an additive record
-- that the family read must honour.
--
-- Publication rule after this migration:
--   family-visible value  AND  an approved, family-visible revision
--                          AND  no active owner restriction.
--
-- Lifting a restriction is not a write to this table by the owner: it happens
-- only when an independent reviewer approves a *new* revision that was created
-- after the restriction (see the profile review transaction).

BEGIN;

CREATE TABLE IF NOT EXISTS onboarding_profile_field_restriction (
    restriction_id        BIGSERIAL PRIMARY KEY,
    person_id             BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    field_key             TEXT NOT NULL,
    -- Monotonic per (person, field); a stale approval from before a restriction
    -- carries an older generation and cannot lift it.
    generation            BIGINT NOT NULL CHECK (generation >= 1),
    approved_revision_id  BIGINT REFERENCES onboarding_profile_revision(revision_id),
    approved_revision_number BIGINT NOT NULL CHECK (approved_revision_number >= 1),
    prior_visibility      TEXT NOT NULL CHECK (prior_visibility = 'family'),
    new_visibility        TEXT NOT NULL CHECK (new_visibility = 'owner_private'),
    restricted_by         TEXT NOT NULL CHECK (length(btrim(restricted_by)) > 0),
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    lifted_at             TIMESTAMPTZ,
    lifted_by             TEXT,
    lifted_generation     BIGINT,
    CONSTRAINT onboarding_profile_field_restriction_lift_check CHECK (
        (lifted_at IS NULL AND lifted_by IS NULL AND lifted_generation IS NULL)
        OR (lifted_at IS NOT NULL AND lifted_by IS NOT NULL
            AND lifted_generation = generation)
    )
);

-- At most one active restriction per field.
CREATE UNIQUE INDEX IF NOT EXISTS onboarding_profile_field_restriction_active_key
    ON onboarding_profile_field_restriction (person_id, field_key)
    WHERE lifted_at IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS onboarding_profile_field_restriction_generation_key
    ON onboarding_profile_field_restriction (person_id, field_key, generation);

CREATE INDEX IF NOT EXISTS onboarding_profile_field_restriction_person_idx
    ON onboarding_profile_field_restriction (person_id, field_key);

-- A restriction row is immutable except for the one-way lift. A direct delete
-- is blocked while its person exists, matching the revision history guard.
CREATE OR REPLACE FUNCTION onboarding_profile_field_restriction_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF EXISTS (SELECT 1 FROM person WHERE person_id = OLD.person_id) THEN
            RAISE EXCEPTION 'profile field restrictions cannot be deleted directly';
        END IF;
        RETURN OLD;
    END IF;

    IF OLD.lifted_at IS NOT NULL THEN
        RAISE EXCEPTION 'a lifted profile field restriction is immutable';
    END IF;
    IF NEW.restriction_id IS DISTINCT FROM OLD.restriction_id
       OR NEW.person_id IS DISTINCT FROM OLD.person_id
       OR NEW.field_key IS DISTINCT FROM OLD.field_key
       OR NEW.generation IS DISTINCT FROM OLD.generation
       OR NEW.approved_revision_id IS DISTINCT FROM OLD.approved_revision_id
       OR NEW.approved_revision_number IS DISTINCT FROM OLD.approved_revision_number
       OR NEW.prior_visibility IS DISTINCT FROM OLD.prior_visibility
       OR NEW.new_visibility IS DISTINCT FROM OLD.new_visibility
       OR NEW.restricted_by IS DISTINCT FROM OLD.restricted_by
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'profile field restriction content is immutable';
    END IF;
    IF NEW.lifted_at IS NULL THEN
        RAISE EXCEPTION 'a restriction may only be lifted, not edited';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS onboarding_profile_field_restriction_guard
    ON onboarding_profile_field_restriction;
CREATE TRIGGER onboarding_profile_field_restriction_guard
    BEFORE UPDATE OR DELETE ON onboarding_profile_field_restriction
    FOR EACH ROW EXECUTE FUNCTION onboarding_profile_field_restriction_guard();

-- Append-only audit trail. It records the actor, the field, the generation, the
-- approved revision that was current when the decision was made, and the
-- visibility transition. It has no foreign key to the restriction row so the
-- history survives a person cascade.
CREATE TABLE IF NOT EXISTS onboarding_profile_field_restriction_event (
    event_id              BIGSERIAL PRIMARY KEY,
    restriction_id        BIGINT NOT NULL,
    person_id             BIGINT NOT NULL,
    field_key             TEXT NOT NULL,
    generation            BIGINT NOT NULL,
    event                 TEXT NOT NULL CHECK (event IN ('restricted', 'lifted')),
    actor                 TEXT NOT NULL CHECK (length(btrim(actor)) > 0),
    approved_revision_id  BIGINT,
    approved_revision_number BIGINT,
    prior_visibility      TEXT,
    new_visibility        TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS onboarding_profile_field_restriction_event_field_idx
    ON onboarding_profile_field_restriction_event (person_id, field_key, event_id);

CREATE OR REPLACE FUNCTION onboarding_profile_field_restriction_event_no_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'profile field restriction events are append-only';
END;
$$;

DROP TRIGGER IF EXISTS onboarding_profile_field_restriction_event_no_mutation
    ON onboarding_profile_field_restriction_event;
CREATE TRIGGER onboarding_profile_field_restriction_event_no_mutation
    BEFORE UPDATE OR DELETE ON onboarding_profile_field_restriction_event
    FOR EACH ROW EXECUTE FUNCTION onboarding_profile_field_restriction_event_no_mutation();

COMMIT;
