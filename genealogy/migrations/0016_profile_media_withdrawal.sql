-- USR-004C follow-up: allow an owner to withdraw a pending media submission.
--
-- A pending media revision is an unapproved submission. Deleting it removes
-- the metadata, the private file, and the pending revision, freeing the
-- actor-scoped open slot. Approved, rejected, and needs-changes revisions
-- remain immutable while the owning person exists; person deletion still
-- cascades.

CREATE OR REPLACE FUNCTION onboarding_profile_revision_no_delete()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.status <> 'draft'
       AND NOT (
           OLD.status = 'pending_review'
           AND OLD.field_key IN ('profile_photo', 'personal_document')
       )
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
