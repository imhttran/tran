-- TRAN-002B genealogy migration 0002: catalog tables and stable keys.
--
-- The person table is the catalog. `person_key` is the stable catalog key and
-- carries a UNIQUE constraint. Name variants, business names, and gravesites
-- reference the person row by foreign key.

CREATE TABLE IF NOT EXISTS person (
    person_id         BIGSERIAL PRIMARY KEY,
    person_key        TEXT NOT NULL,
    display_name      TEXT NOT NULL,
    family_name       TEXT,
    given_name        TEXT,
    generation        INTEGER,
    birth_year        TEXT,
    death_year        TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT person_person_key_key UNIQUE (person_key)
);

CREATE INDEX IF NOT EXISTS person_generation_idx ON person (generation);

CREATE TABLE IF NOT EXISTS person_name_variant (
    name_variant_id   BIGSERIAL PRIMARY KEY,
    person_id         BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    name_variant      TEXT NOT NULL,
    CONSTRAINT person_name_variant_person_id_name_variant_key UNIQUE (person_id, name_variant)
);

CREATE TABLE IF NOT EXISTS business_name (
    business_name_id  BIGSERIAL PRIMARY KEY,
    person_id         BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    trade_name        TEXT NOT NULL,
    CONSTRAINT business_name_person_id_trade_name_key UNIQUE (person_id, trade_name)
);

CREATE TABLE IF NOT EXISTS gravesite (
    gravesite_id      BIGSERIAL PRIMARY KEY,
    person_id         BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    cemetery          TEXT NOT NULL,
    CONSTRAINT gravesite_person_id_cemetery_key UNIQUE (person_id, cemetery)
);
