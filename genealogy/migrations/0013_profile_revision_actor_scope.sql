-- USR-004B follow-up: scope open profile drafts to the identity that created
-- them. A person keeps one open revision per field per actor, so replacing the
-- linked account cannot expose the former owner's draft or block the new owner
-- from creating a replacement draft.
--
-- The old index enforced one open revision per (person, field) across all
-- actors. It is replaced by an actor-scoped index. No row is changed or deleted.

DROP INDEX IF EXISTS onboarding_profile_revision_open_key;

CREATE UNIQUE INDEX IF NOT EXISTS onboarding_profile_revision_open_actor_key
    ON onboarding_profile_revision (person_id, field_key, changed_by)
 WHERE status IN ('draft', 'pending_review');

CREATE INDEX IF NOT EXISTS onboarding_profile_revision_actor_history_idx
    ON onboarding_profile_revision (person_id, changed_by, changed_at DESC);
