-- TRAN-002B genealogy migration 0003: source evidence, family links,
-- assertions, and the verified-assertion evidence rule.
--
-- The schema is additive and versioned. Every table lives in the separate
-- genealogy database selected by GENEALOGY_DATABASE_URL; no RAG migration or
-- documents table is touched.

CREATE TABLE IF NOT EXISTS source_document (
    source_document_id              BIGSERIAL PRIMARY KEY,
    relative_path                   TEXT NOT NULL,
    sha256                          TEXT NOT NULL,
    language                        TEXT,
    derived_from_source_document_id BIGINT REFERENCES source_document(source_document_id) ON DELETE SET NULL,
    created_at                      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT source_document_relative_path_key UNIQUE (relative_path),
    CONSTRAINT source_document_sha256_check CHECK (sha256 ~ '^[0-9a-f]{64}$')
);

CREATE TABLE IF NOT EXISTS source_evidence (
    source_evidence_id BIGSERIAL PRIMARY KEY,
    source_document_id BIGINT NOT NULL REFERENCES source_document(source_document_id) ON DELETE CASCADE,
    page               INTEGER CHECK (page IS NULL OR page >= 1),
    locator            TEXT,
    excerpt            TEXT,
    char_offset        INTEGER CHECK (char_offset IS NULL OR char_offset >= 0),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT source_evidence_key UNIQUE (source_document_id, page, locator)
);

CREATE TABLE IF NOT EXISTS family_branch (
    family_branch_id   BIGSERIAL PRIMARY KEY,
    branch_key         TEXT NOT NULL,
    branch_label       TEXT,
    branch_number      INTEGER,
    source_evidence_id BIGINT REFERENCES source_evidence(source_evidence_id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT family_branch_branch_key_key UNIQUE (branch_key)
);

CREATE TABLE IF NOT EXISTS catalog_entry (
    catalog_entry_id  BIGSERIAL PRIMARY KEY,
    entry_key         TEXT NOT NULL,
    source_document_id BIGINT NOT NULL REFERENCES source_document(source_document_id) ON DELETE CASCADE,
    raw_reference     TEXT,
    parse_status      TEXT NOT NULL DEFAULT 'candidate',
    parse_reason      TEXT,
    person_id         BIGINT REFERENCES person(person_id) ON DELETE SET NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT catalog_entry_entry_key_key UNIQUE (entry_key),
    CONSTRAINT catalog_entry_parse_status_check CHECK (
        parse_status IN ('parsed', 'ambiguous', 'rejected', 'unmatched')
    )
);

CREATE TABLE IF NOT EXISTS spouse_link (
    spouse_link_id     BIGSERIAL PRIMARY KEY,
    person_id          BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    spouse_id          BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    source_order       INTEGER NOT NULL CHECK (source_order >= 1),
    source_evidence_id BIGINT REFERENCES source_evidence(source_evidence_id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT spouse_link_key UNIQUE (person_id, spouse_id, source_order),
    CONSTRAINT spouse_link_no_self CHECK (person_id <> spouse_id)
);

CREATE TABLE IF NOT EXISTS parent_child_link (
    parent_child_link_id BIGSERIAL PRIMARY KEY,
    parent_id            BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    child_id             BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    source_evidence_id   BIGINT REFERENCES source_evidence(source_evidence_id) ON DELETE SET NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT parent_child_link_key UNIQUE (parent_id, child_id),
    CONSTRAINT parent_child_link_no_self CHECK (parent_id <> child_id)
);

CREATE TABLE IF NOT EXISTS assertion (
    assertion_id      BIGSERIAL PRIMARY KEY,
    person_id         BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    assertion_kind    TEXT NOT NULL,
    assertion_key     TEXT NOT NULL,
    value_text        TEXT,
    object_person_id  BIGINT REFERENCES person(person_id) ON DELETE SET NULL,
    review_status     TEXT NOT NULL DEFAULT 'unverified',
    reviewed_by       TEXT,
    reviewed_at       TIMESTAMPTZ,
    review_note       TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT assertion_person_kind_key_key UNIQUE (person_id, assertion_kind, assertion_key),
    CONSTRAINT assertion_review_status_check CHECK (
        review_status IN ('unverified', 'needs_review', 'verified', 'disputed')
    ),
    CONSTRAINT assertion_verified_review_metadata_check CHECK (
        review_status <> 'verified' OR (reviewed_by IS NOT NULL AND reviewed_at IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS assertion_evidence (
    assertion_evidence_id BIGSERIAL PRIMARY KEY,
    assertion_id          BIGINT NOT NULL REFERENCES assertion(assertion_id) ON DELETE CASCADE,
    source_evidence_id    BIGINT REFERENCES source_evidence(source_evidence_id) ON DELETE SET NULL,
    document_ref          TEXT NOT NULL,
    page                  INTEGER NOT NULL CHECK (page >= 1),
    locator               TEXT,
    excerpt               TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT assertion_evidence_key UNIQUE (assertion_id, document_ref, page)
);

CREATE INDEX IF NOT EXISTS parent_child_link_child_idx
    ON parent_child_link (child_id);
CREATE INDEX IF NOT EXISTS parent_child_link_parent_idx
    ON parent_child_link (parent_id);
CREATE INDEX IF NOT EXISTS spouse_link_spouse_idx
    ON spouse_link (spouse_id);
CREATE INDEX IF NOT EXISTS assertion_person_idx
    ON assertion (person_id);
CREATE INDEX IF NOT EXISTS assertion_evidence_assertion_idx
    ON assertion_evidence (assertion_id);
CREATE INDEX IF NOT EXISTS source_evidence_document_idx
    ON source_evidence (source_document_id);
CREATE INDEX IF NOT EXISTS catalog_entry_source_idx
    ON catalog_entry (source_document_id);

-- A verified assertion must reference at least one evidence row before the
-- transaction commits. The constraint trigger is deferred so the evidence row
-- can be inserted after the assertion in the same transaction.
CREATE OR REPLACE FUNCTION assertion_verified_requires_evidence() RETURNS trigger AS $$
BEGIN
    IF NEW.review_status = 'verified'
       AND NOT EXISTS (
           SELECT 1 FROM assertion_evidence WHERE assertion_id = NEW.assertion_id
       )
    THEN
        RAISE EXCEPTION
            'verified assertion % requires at least one evidence row',
            NEW.assertion_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS assertion_verified_requires_evidence ON assertion;
CREATE CONSTRAINT TRIGGER assertion_verified_requires_evidence
    AFTER INSERT OR UPDATE ON assertion
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assertion_verified_requires_evidence();
