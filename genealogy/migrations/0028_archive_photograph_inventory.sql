-- AP-101: deterministic per-page and per-image-object inventory of a document
-- version's original PDF.
--
-- The inventory is additive. It stores only document properties: page numbers,
-- readability, failure text, image object ids, pixel dimensions, colour space,
-- filters, placement, tint, repetition, classification signals, and captions.
-- It never stores image bytes, stream contents, or a storage path.
--
-- A repeated inventory of the same version and content hash is a no-op: every
-- table has a unique key over (version_id, content_sha256, ...) and the writer
-- uses ON CONFLICT DO NOTHING. Every statement is safe on a second run.

-- One row per inventory run. `content_sha256` pins the run to the exact bytes
-- that were walked, so a replaced original cannot inherit an old inventory.
CREATE TABLE IF NOT EXISTS archive_photograph_inventory (
    inventory_id        BIGSERIAL PRIMARY KEY,
    document_version_id BIGINT NOT NULL,
    corpus              TEXT NOT NULL,
    content_sha256      TEXT NOT NULL,
    byte_size           BIGINT NOT NULL,
    page_count          INTEGER NOT NULL,
    image_count         INTEGER NOT NULL,
    created_by          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT archive_photograph_inventory_sha_check CHECK (
        content_sha256 ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT archive_photograph_inventory_counts_check CHECK (
        page_count >= 0 AND image_count >= 0
    ),
    CONSTRAINT archive_photograph_inventory_actor_check CHECK (
        length(btrim(created_by)) > 0
    )
);

-- One inventory per document version and content hash.
CREATE UNIQUE INDEX IF NOT EXISTS archive_photograph_inventory_version_key
    ON archive_photograph_inventory (document_version_id, content_sha256);

CREATE INDEX IF NOT EXISTS archive_photograph_inventory_corpus_idx
    ON archive_photograph_inventory (corpus, created_at DESC);

-- One row per page, 1-based. A page with no image still has a row, so the
-- accounting covers every page exactly once.
CREATE TABLE IF NOT EXISTS archive_photograph_inventory_page (
    page_id             BIGSERIAL PRIMARY KEY,
    inventory_id        BIGINT NOT NULL
                            REFERENCES archive_photograph_inventory(inventory_id)
                            ON DELETE CASCADE,
    page_number         INTEGER NOT NULL CHECK (page_number >= 1),
    readable            BOOLEAN NOT NULL,
    -- The exact failure text when readable is false; NULL otherwise.
    failure_text        TEXT,
    image_count         INTEGER NOT NULL DEFAULT 0 CHECK (image_count >= 0),
    caption             TEXT,
    CONSTRAINT archive_photograph_inventory_page_failure_check CHECK (
        (readable AND failure_text IS NULL)
        OR ((NOT readable) AND failure_text IS NOT NULL AND length(failure_text) > 0)
    )
);

-- A page appears exactly once per run.
CREATE UNIQUE INDEX IF NOT EXISTS archive_photograph_inventory_page_key
    ON archive_photograph_inventory_page (inventory_id, page_number);

-- One row per image object a page draws. The object id is the lopdf object id
-- (number and generation) rendered as text; no stream bytes are stored.
CREATE TABLE IF NOT EXISTS archive_photograph_inventory_image (
    image_id            BIGSERIAL PRIMARY KEY,
    inventory_id        BIGINT NOT NULL
                            REFERENCES archive_photograph_inventory(inventory_id)
                            ON DELETE CASCADE,
    page_id             BIGINT NOT NULL
                            REFERENCES archive_photograph_inventory_page(page_id)
                            ON DELETE CASCADE,
    page_number         INTEGER NOT NULL CHECK (page_number >= 1),
    occurrence_ordinal  INTEGER NOT NULL CHECK (occurrence_ordinal >= 1),
    object_id           TEXT NOT NULL,
    object_number       BIGINT NOT NULL,
    object_generation   INTEGER NOT NULL CHECK (object_generation >= 0),
    pixel_width         INTEGER,
    pixel_height        INTEGER,
    color_space         TEXT,
    filters             TEXT,
    -- True only when the decoded stream length matches width*height*components
    -- for an accepted colours/bits combination.
    complete            BOOLEAN NOT NULL,
    completeness_reason TEXT,
    -- Placement rectangle in default user space, when the content stream
    -- operators resolved; NULL when they did not.
    placement_x         DOUBLE PRECISION,
    placement_y         DOUBLE PRECISION,
    placement_width     DOUBLE PRECISION,
    placement_height    DOUBLE PRECISION,
    object_name         TEXT,
    tinted              BOOLEAN,
    tint_signal         TEXT,
    occurrence_count    INTEGER NOT NULL DEFAULT 1 CHECK (occurrence_count >= 1),
    classification      TEXT NOT NULL DEFAULT 'unknown',
    classification_rule TEXT NOT NULL DEFAULT 'unknown-no-signal',
    classification_detail TEXT,
    caption             TEXT,
    CONSTRAINT archive_photograph_inventory_image_class_check CHECK (
        classification IN ('photograph', 'decorative', 'watermark', 'diagram', 'unknown')
    ),
    CONSTRAINT archive_photograph_inventory_image_complete_check CHECK (
        complete OR (completeness_reason IS NOT NULL AND length(completeness_reason) > 0)
    ),
    CONSTRAINT archive_photograph_inventory_image_rule_check CHECK (
        length(btrim(classification_rule)) > 0
    ),
    CONSTRAINT archive_photograph_inventory_image_unknown_check CHECK (
        classification <> 'unknown'
        OR (classification_detail IS NOT NULL AND length(btrim(classification_detail)) > 0)
    )
);

-- One row per drawn occurrence, so a repeated placement is explicit.
CREATE UNIQUE INDEX IF NOT EXISTS archive_photograph_inventory_image_key
    ON archive_photograph_inventory_image (inventory_id, page_number, occurrence_ordinal);

CREATE INDEX IF NOT EXISTS archive_photograph_inventory_image_object_idx
    ON archive_photograph_inventory_image (inventory_id, object_number, object_generation);

-- One asset row per distinct image object per inventory.
CREATE TABLE IF NOT EXISTS archive_photograph_asset (
    asset_id            BIGSERIAL PRIMARY KEY,
    inventory_id        BIGINT NOT NULL
                            REFERENCES archive_photograph_inventory(inventory_id)
                            ON DELETE CASCADE,
    object_id           TEXT NOT NULL,
    object_number       BIGINT NOT NULL,
    object_generation   INTEGER NOT NULL CHECK (object_generation >= 0),
    pixel_width         INTEGER,
    pixel_height        INTEGER,
    color_space         TEXT,
    filters             TEXT,
    complete            BOOLEAN NOT NULL,
    classification      TEXT NOT NULL DEFAULT 'unknown',
    classification_rule TEXT NOT NULL DEFAULT 'unknown-no-signal',
    occurrence_count    INTEGER NOT NULL DEFAULT 1 CHECK (occurrence_count >= 1),
    CONSTRAINT archive_photograph_asset_class_check CHECK (
        classification IN ('photograph', 'decorative', 'watermark', 'diagram', 'unknown')
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS archive_photograph_asset_key
    ON archive_photograph_asset (inventory_id, object_number, object_generation);

-- Links an asset to each page and placement where it is drawn.
CREATE TABLE IF NOT EXISTS archive_photograph_occurrence (
    occurrence_id       BIGSERIAL PRIMARY KEY,
    asset_id            BIGINT NOT NULL
                            REFERENCES archive_photograph_asset(asset_id) ON DELETE CASCADE,
    inventory_id        BIGINT NOT NULL
                            REFERENCES archive_photograph_inventory(inventory_id)
                            ON DELETE CASCADE,
    page_number         INTEGER NOT NULL CHECK (page_number >= 1),
    occurrence_ordinal  INTEGER NOT NULL CHECK (occurrence_ordinal >= 1)
);

CREATE UNIQUE INDEX IF NOT EXISTS archive_photograph_occurrence_key
    ON archive_photograph_occurrence (asset_id, page_number, occurrence_ordinal);

CREATE INDEX IF NOT EXISTS archive_photograph_occurrence_page_idx
    ON archive_photograph_occurrence (inventory_id, page_number);

-- Reviewer-confirmed links from a photograph asset to a person record.
CREATE TABLE IF NOT EXISTS archive_photograph_person_link (
    link_id             BIGSERIAL PRIMARY KEY,
    asset_id            BIGINT NOT NULL
                            REFERENCES archive_photograph_asset(asset_id) ON DELETE CASCADE,
    person_id           BIGINT NOT NULL
                            REFERENCES person(person_id) ON DELETE CASCADE,
    status              TEXT NOT NULL DEFAULT 'proposed',
    decided_by          TEXT,
    decided_at          TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT archive_photograph_person_link_status_check CHECK (
        status IN ('proposed', 'confirmed', 'rejected')
    ),
    CONSTRAINT archive_photograph_person_link_decision_check CHECK (
        status = 'proposed' OR (decided_by IS NOT NULL AND decided_at IS NOT NULL)
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS archive_photograph_person_link_key
    ON archive_photograph_person_link (asset_id, person_id);

CREATE INDEX IF NOT EXISTS archive_photograph_person_link_person_idx
    ON archive_photograph_person_link (person_id, status);

-- Append-only review history for an inventory run.
CREATE TABLE IF NOT EXISTS archive_photograph_inventory_event (
    event_id            BIGSERIAL PRIMARY KEY,
    inventory_id        BIGINT NOT NULL
                            REFERENCES archive_photograph_inventory(inventory_id)
                            ON DELETE CASCADE,
    action              TEXT NOT NULL,
    actor               TEXT NOT NULL,
    detail              JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT archive_photograph_inventory_event_action_check CHECK (
        action IN ('inventoried', 'reused', 'classified', 'reviewed')
    ),
    CONSTRAINT archive_photograph_inventory_event_actor_check CHECK (
        length(btrim(actor)) > 0
    )
);

CREATE INDEX IF NOT EXISTS archive_photograph_inventory_event_inventory_idx
    ON archive_photograph_inventory_event (inventory_id, event_id);

CREATE OR REPLACE FUNCTION archive_photograph_inventory_event_append_only()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'archive photograph inventory events are append-only';
END;
$$;

DROP TRIGGER IF EXISTS archive_photograph_inventory_event_append_only
    ON archive_photograph_inventory_event;
CREATE TRIGGER archive_photograph_inventory_event_append_only
    BEFORE UPDATE OR DELETE ON archive_photograph_inventory_event
    FOR EACH ROW EXECUTE FUNCTION archive_photograph_inventory_event_append_only();

COMMENT ON TABLE archive_photograph_inventory IS
    'AP-101: one deterministic per-page and per-image-object inventory of a document version original.';
COMMENT ON COLUMN archive_photograph_inventory_image.complete IS
    'True only when the decoded stream length matches width*height*components.';
COMMENT ON COLUMN archive_photograph_inventory_image.classification_detail IS
    'The property values the firing rule used, or the reason the object is unknown.';
