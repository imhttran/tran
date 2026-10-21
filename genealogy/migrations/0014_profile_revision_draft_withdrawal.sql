-- USR-004B follow-up: allow an owner to withdraw an unsubmitted draft.
--
-- Drafts are mutable working copies. Deleting a draft releases the
-- actor-scoped open slot and removes its draft-only history. Submitted and
-- terminal revisions remain immutable: the delete guard still rejects them
-- while the owning person exists, and person deletion still cascades.
--
-- No submitted or terminal revision is deleted by this migration.

CREATE OR REPLACE FUNCTION onboarding_profile_revision_no_delete()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.status <> 'draft'
       AND EXISTS (SELECT 1 FROM person WHERE person_id = OLD.person_id) THEN
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
