-- USR-004D: preserve an audit record when an owner withdraws pending media.
--
-- Withdrawal removes the pending revision, and the revision's
-- onboarding_profile_review_event rows cascade with it. This append-only ledger
-- keeps the withdrawal actor, the media/revision references, the media kind,
-- the action, and the timestamp. It stores no file content, file name, hash, or
-- storage path, so the sensitive payload is not retained.
--
-- The media and revision identifiers intentionally have no foreign keys: the
-- whole point is that the audit row outlives the rows it describes. Person
-- deletion still cascades so a synthetic person can be cleaned up.

CREATE TABLE IF NOT EXISTS onboarding_media_withdrawal (
    withdrawal_id BIGSERIAL PRIMARY KEY,
    media_id      BIGINT NOT NULL,
    revision_id   BIGINT NOT NULL,
    person_id     BIGINT NOT NULL REFERENCES person(person_id) ON DELETE CASCADE,
    media_kind    TEXT NOT NULL,
    action        TEXT NOT NULL DEFAULT 'withdraw',
    actor         TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT onboarding_media_withdrawal_kind_check CHECK (
        media_kind IN ('profile_photo', 'personal_document')
    ),
    CONSTRAINT onboarding_media_withdrawal_action_check CHECK (
        action = 'withdraw'
    ),
    CONSTRAINT onboarding_media_withdrawal_actor_not_blank CHECK (
        length(btrim(actor)) > 0
    )
);

CREATE INDEX IF NOT EXISTS onboarding_media_withdrawal_person_idx
    ON onboarding_media_withdrawal (person_id, created_at);

CREATE INDEX IF NOT EXISTS onboarding_media_withdrawal_media_idx
    ON onboarding_media_withdrawal (media_id);

-- A media submission can be withdrawn once. The unique key makes a retried or
-- duplicated withdrawal insert fail closed instead of recording two events.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_media_withdrawal_media_action_key'
           AND conrelid = 'onboarding_media_withdrawal'::regclass
    ) THEN
        ALTER TABLE onboarding_media_withdrawal
            ADD CONSTRAINT onboarding_media_withdrawal_media_action_key
            UNIQUE (media_id, action);
    END IF;
END;
$$;

CREATE OR REPLACE FUNCTION onboarding_media_withdrawal_no_update()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'media withdrawal audit records are append-only';
END;
$$;

DROP TRIGGER IF EXISTS onboarding_media_withdrawal_no_update
    ON onboarding_media_withdrawal;
CREATE TRIGGER onboarding_media_withdrawal_no_update
    BEFORE UPDATE ON onboarding_media_withdrawal
    FOR EACH ROW EXECUTE FUNCTION onboarding_media_withdrawal_no_update();

-- Direct deletion would erase audit evidence, but deleting the owning person
-- must still cascade. A direct child delete sees its person row; a parent
-- cascade runs after the person row is gone.
CREATE OR REPLACE FUNCTION onboarding_media_withdrawal_no_delete()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM person WHERE person_id = OLD.person_id) THEN
        RAISE EXCEPTION 'media withdrawal audit records cannot be deleted directly';
    END IF;
    RETURN OLD;
END;
$$;

DROP TRIGGER IF EXISTS onboarding_media_withdrawal_no_delete
    ON onboarding_media_withdrawal;
CREATE TRIGGER onboarding_media_withdrawal_no_delete
    BEFORE DELETE ON onboarding_media_withdrawal
    FOR EACH ROW EXECUTE FUNCTION onboarding_media_withdrawal_no_delete();
