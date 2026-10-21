-- Synthetic USR-005B branch-review fixture for the browser/API E2E suite.
--
-- Run only against the disposable GENEALOGY_DATABASE_URL. It creates three
-- synthetic people with verified branch claims, one original source document,
-- and the reviewer's own account link. It contains no real family names,
-- dates, addresses, or source material.
--
-- psql variables: reviewer_account_id, reviewer_fingerprint.

BEGIN;

-- Remove only this fixture's synthetic rows. Proposals are deleted before the
-- people so a reference from an approved proposal can never reach a deleted
-- assertion. Proposal events are append-only and intentionally survive.
DELETE FROM genealogy_fact_proposal
 WHERE person_id IN (
     SELECT person_id FROM person WHERE person_key LIKE 'e2e-branch-%'
 );
DELETE FROM person WHERE person_key LIKE 'e2e-branch-%';

INSERT INTO source_document (relative_path, sha256, source_kind)
VALUES ('synthetic/e2e-branch-review-source.md', repeat('b', 64), 'original')
ON CONFLICT (relative_path) DO NOTHING;

INSERT INTO source_evidence (source_document_id, page, locator)
SELECT source_document_id, 1, 'p.1'
  FROM source_document
 WHERE relative_path = 'synthetic/e2e-branch-review-source.md'
ON CONFLICT (source_document_id, page, locator) DO NOTHING;

-- A review subject in branch 1, a second-branch person the reviewer may not
-- touch, and the reviewer's own linked person (linked-owner independence).
INSERT INTO person (person_key, display_name) VALUES
    ('e2e-branch-subject', 'Synthetic Branch Subject'),
    ('e2e-branch-other', 'Synthetic Second Branch Subject'),
    ('e2e-branch-owner', 'Synthetic Linked Reviewer Person');

WITH branch_claims (person_key, branch_key) AS (
    VALUES ('e2e-branch-subject', 'branch-1'),
           ('e2e-branch-other', 'branch-2'),
           ('e2e-branch-owner', 'branch-1')
), claimed AS (
    INSERT INTO assertion
        (person_id, assertion_kind, assertion_key, value_text, review_status)
    SELECT p.person_id, 'branch_claim', bc.branch_key, bc.branch_key, 'unverified'
      FROM branch_claims bc
      JOIN person p ON p.person_key = bc.person_key
    RETURNING assertion_id
)
INSERT INTO assertion_evidence
    (assertion_id, source_evidence_id, document_ref, page)
SELECT claimed.assertion_id, seeded.source_evidence_id, seeded.relative_path, 1
  FROM claimed
 CROSS JOIN (
     SELECT se.source_evidence_id, sd.relative_path
       FROM source_evidence se
       JOIN source_document sd ON sd.source_document_id = se.source_document_id
      WHERE sd.relative_path = 'synthetic/e2e-branch-review-source.md'
 ) AS seeded;

-- The deferred verified-assertion constraint is checked at COMMIT, so the
-- evidence rows above satisfy it without a second transaction.
UPDATE assertion a
   SET review_status = 'verified',
       reviewed_by = 'e2e-seed',
       reviewed_at = now()
  FROM person p
 WHERE p.person_id = a.person_id
   AND p.person_key LIKE 'e2e-branch-%'
   AND a.assertion_kind = 'branch_claim';

INSERT INTO onboarding_person_account_link
    (person_id, account_id, linked_by, identity_fingerprint)
SELECT p.person_id, :reviewer_account_id, 'e2e-seed', :'reviewer_fingerprint'
  FROM person p
 WHERE p.person_key = 'e2e-branch-owner';

COMMIT;
