-- AP-105 extension: provenance for layout-aware photograph person proposals.
--
-- A proposal is still a row in archive_photograph_person_link with status
-- 'proposed'. These additive columns record why the proposal exists: the source
-- page, the supporting text, the confidence level, the match kind, and the
-- ordering the review screen uses. Confirmed and rejected rows keep their
-- decisions; only proposed rows are refreshed by a new proposal run.
--
-- Every statement is additive and safe on a second run.

ALTER TABLE archive_photograph_person_link
    ADD COLUMN IF NOT EXISTS source_page INTEGER;
ALTER TABLE archive_photograph_person_link
    ADD COLUMN IF NOT EXISTS supporting_text TEXT;
ALTER TABLE archive_photograph_person_link
    ADD COLUMN IF NOT EXISTS confidence TEXT;
ALTER TABLE archive_photograph_person_link
    ADD COLUMN IF NOT EXISTS confidence_basis TEXT;
ALTER TABLE archive_photograph_person_link
    ADD COLUMN IF NOT EXISTS ambiguity_note TEXT;
ALTER TABLE archive_photograph_person_link
    ADD COLUMN IF NOT EXISTS match_kind TEXT;
ALTER TABLE archive_photograph_person_link
    ADD COLUMN IF NOT EXISTS candidate_rank INTEGER;
ALTER TABLE archive_photograph_person_link
    ADD COLUMN IF NOT EXISTS proposal_origin TEXT;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'archive_photograph_person_link_source_page_check'
    ) THEN
        ALTER TABLE archive_photograph_person_link
            ADD CONSTRAINT archive_photograph_person_link_source_page_check CHECK (
                source_page IS NULL OR source_page >= 1
            );
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'archive_photograph_person_link_confidence_check'
    ) THEN
        ALTER TABLE archive_photograph_person_link
            ADD CONSTRAINT archive_photograph_person_link_confidence_check CHECK (
                confidence IS NULL OR confidence IN ('high', 'medium', 'low')
            );
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'archive_photograph_person_link_candidate_rank_check'
    ) THEN
        ALTER TABLE archive_photograph_person_link
            ADD CONSTRAINT archive_photograph_person_link_candidate_rank_check CHECK (
                candidate_rank IS NULL OR candidate_rank >= 1
            );
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'archive_photograph_person_link_origin_check'
    ) THEN
        ALTER TABLE archive_photograph_person_link
            ADD CONSTRAINT archive_photograph_person_link_origin_check CHECK (
                proposal_origin IS NULL
                OR proposal_origin IN ('caption', 'layout', 'manual')
            );
    END IF;
END;
$$;

-- The review screen reads proposed candidates strongest first, with confirmed
-- links ahead of proposals so an existing decision is never obscured.
CREATE INDEX IF NOT EXISTS archive_photograph_person_link_review_idx
    ON archive_photograph_person_link (asset_id, status, candidate_rank);

COMMENT ON COLUMN archive_photograph_person_link.source_page IS
    'The PDF page whose caption, heading, or nearby text supports the proposal.';
COMMENT ON COLUMN archive_photograph_person_link.supporting_text IS
    'The bounded source text the proposal relies on.';
COMMENT ON COLUMN archive_photograph_person_link.confidence IS
    'high, medium, or low confidence in the proposed person link.';
COMMENT ON COLUMN archive_photograph_person_link.confidence_basis IS
    'The signals that set the confidence, including date or relationship evidence.';
COMMENT ON COLUMN archive_photograph_person_link.ambiguity_note IS
    'Why a namesake or weak match needs an explicit reviewer choice.';
COMMENT ON COLUMN archive_photograph_person_link.match_kind IS
    'exact, alias, reordered, or partial name matching.';
COMMENT ON COLUMN archive_photograph_person_link.candidate_rank IS
    '1-based strength order shown to the reviewer.';
COMMENT ON COLUMN archive_photograph_person_link.proposal_origin IS
    'caption, layout, or manual. Only proposed layout rows are refreshed.';
