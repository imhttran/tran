-- USR-006-P follow-up: durable restriction ordering for profile revisions.
--
-- The restriction lifecycle is ordered by the monotonic per-(person, field)
-- `generation` in `onboarding_profile_field_restriction`, never by timestamps.
-- `now()` is fixed at transaction start, so two overlapping transactions can
-- record timestamps that do not reflect their commit order: a submission created
-- while a restriction transaction was still waiting for its lock can carry a
-- `changed_at` later than the restriction's `created_at` even though the
-- restriction had not yet become authoritative.
--
-- A revision now records the restriction generation that was active when the
-- owner created it (NULL when no restriction was active). Approval may lift a
-- restriction only when this recorded generation exactly matches the currently
-- active generation. Therefore:
--   * a revision created before the restriction (NULL) cannot lift it;
--   * a revision created under an earlier generation cannot lift a later one;
--   * legacy or incomplete generation data fails closed.
--
-- Additive and idempotent. No existing revision is re-interpreted or rewritten.
--
-- No backfill is performed: a pending revision that predates this column keeps
-- NULL and must be re-submitted after the restriction. Deriving a generation for
-- it from timestamps would reintroduce exactly the ordering ambiguity this
-- migration removes, and would silently reinterpret an existing submission as a
-- newly authorized expansion.

BEGIN;

ALTER TABLE onboarding_profile_revision
    ADD COLUMN IF NOT EXISTS restriction_generation BIGINT
        CHECK (restriction_generation IS NULL OR restriction_generation >= 1);

COMMENT ON COLUMN onboarding_profile_revision.restriction_generation IS
    'Active owner-restriction generation observed when this revision was created. Approval may lift only an active restriction with an exactly matching generation; NULL (no restriction, or legacy) fails closed.';

COMMIT;
