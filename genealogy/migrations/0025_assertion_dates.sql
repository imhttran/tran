-- TRAN-002H: explicit birth and death dates as evidence-backed assertions.
--
-- Dates are claims, not person columns. A date belongs to a source, carries a
-- review status, and may have a conflicting sibling; `person.birth_year` and
-- `person.death_year` stay untouched so a catalog import cannot silently change
-- age or living-status classification.
--
-- The structured columns are additive and nullable, so existing assertions keep
-- their exact meaning. A `date_unresolved` assertion deliberately leaves them
-- NULL: the source wording is preserved without claiming a date the source does
-- not state.
--
-- The values are partial ISO-8601 text (`YYYY`, `YYYY-MM`, `YYYY-MM-DD`), which
-- sorts chronologically as text and cannot invent a missing day or month.

BEGIN;

ALTER TABLE assertion
    ADD COLUMN IF NOT EXISTS value_date_start TEXT,
    ADD COLUMN IF NOT EXISTS value_date_end TEXT,
    ADD COLUMN IF NOT EXISTS value_date_precision TEXT,
    ADD COLUMN IF NOT EXISTS value_date_qualifier TEXT;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'assertion_value_date_precision_check'
           AND conrelid = 'assertion'::regclass
    ) THEN
        ALTER TABLE assertion
            ADD CONSTRAINT assertion_value_date_precision_check
            CHECK (
                value_date_precision IS NULL
                OR value_date_precision IN ('day', 'month', 'year', 'range')
            );
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'assertion_value_date_qualifier_check'
           AND conrelid = 'assertion'::regclass
    ) THEN
        ALTER TABLE assertion
            ADD CONSTRAINT assertion_value_date_qualifier_check
            CHECK (
                value_date_qualifier IS NULL
                OR value_date_qualifier IN ('exact', 'circa', 'before', 'after')
            );
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'assertion_value_date_start_check'
           AND conrelid = 'assertion'::regclass
    ) THEN
        ALTER TABLE assertion
            ADD CONSTRAINT assertion_value_date_start_check
            CHECK (
                value_date_start IS NULL
                OR value_date_start ~ '^[0-9]{4}(-[0-9]{2}(-[0-9]{2})?)?$'
            );
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'assertion_value_date_end_check'
           AND conrelid = 'assertion'::regclass
    ) THEN
        ALTER TABLE assertion
            ADD CONSTRAINT assertion_value_date_end_check
            CHECK (
                value_date_end IS NULL
                OR value_date_end ~ '^[0-9]{4}(-[0-9]{2}(-[0-9]{2})?)?$'
            );
    END IF;

    -- The range precision and the end value must agree in both directions: a
    -- range always has two bounds, and a single date never has an end.
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'assertion_value_date_pair_check'
           AND conrelid = 'assertion'::regclass
    ) THEN
        ALTER TABLE assertion
            ADD CONSTRAINT assertion_value_date_pair_check
            CHECK (
                (value_date_precision = 'range') = (value_date_end IS NOT NULL)
            );
    END IF;
END $$;

-- Date lookups are per person and per kind; the existing assertion_person_idx
-- already serves that. This partial index keeps "the verified dates" cheap for
-- the tree projection without scanning unrelated assertions.
CREATE INDEX IF NOT EXISTS assertion_verified_date_idx
    ON assertion (person_id, assertion_kind)
    WHERE review_status = 'verified'
      AND assertion_kind IN ('birth_date', 'death_date');

COMMIT;
