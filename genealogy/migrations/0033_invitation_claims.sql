-- Shareable invitation links (additive).
--
-- An administrator shares a single-use invitation link through any channel.
-- When a signed-in member redeems it, the invitation records which identity
-- account claimed it (account_id + identity fingerprint, never a raw issuer or
-- subject) and a self-reported name. The claim is not a link: an administrator
-- confirms or rejects it, and only a confirmation writes the reviewed
-- account-to-person link. No existing row is changed.

ALTER TABLE onboarding_invitation
    ADD COLUMN IF NOT EXISTS claimed_fingerprint TEXT,
    ADD COLUMN IF NOT EXISTS claimed_name TEXT,
    ADD COLUMN IF NOT EXISTS claim_decision TEXT,
    ADD COLUMN IF NOT EXISTS claim_decided_by TEXT,
    ADD COLUMN IF NOT EXISTS claim_decided_at TIMESTAMPTZ;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'onboarding_invitation_claim_decision_check'
           AND conrelid = 'onboarding_invitation'::regclass
    ) THEN
        ALTER TABLE onboarding_invitation
            ADD CONSTRAINT onboarding_invitation_claim_decision_check
            CHECK (claim_decision IS NULL OR claim_decision IN ('linked', 'rejected'));
    END IF;
END;
$$;

-- The admin queue reads claims that are redeemed but not yet decided.
CREATE INDEX IF NOT EXISTS onboarding_invitation_open_claim_idx
    ON onboarding_invitation (accepted_at)
 WHERE status = 'accepted' AND account_id IS NOT NULL AND claim_decision IS NULL;
