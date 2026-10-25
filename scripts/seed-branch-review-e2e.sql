-- Synthetic USR-005B/USR-006-A fixture for the browser/API E2E suite.
--
-- Run only against the disposable GENEALOGY_DATABASE_URL. It creates synthetic
-- people with verified branch claims, one original source document, two
-- approved profile values, and the reviewer/member account links. It contains
-- no real family names, dates, addresses, or source material.
--
-- psql variables: reviewer_account_id, reviewer_fingerprint,
-- off_account_id, off_fingerprint, family_account_id, family_fingerprint,
-- restrict_account_id, restrict_fingerprint,
-- marker, family_marker, draft_marker, restrict_marker.
--
-- `marker` is a distinctive synthetic value written as an approved,
-- owner-private profile value for the reviewed person, `family_marker` is an
-- approved family-visible biography for the same person, and `draft_marker` is
-- an unsubmitted draft that must never appear on a family page. The browser
-- acceptance suites prove the private value never reaches the branch review API
-- or page, and that the family profile endpoint returns the family value while
-- still withholding the private one.
--
-- REPEATED RUNS. This fixture is idempotent and never deletes a person.
-- `assertion_review_event` is append-only and rejects a delete even when it
-- arrives through `person -> assertion` cascade, and the branch review and
-- family profile suites approve facts for these people. Deleting the person
-- would therefore fail on the second run against the same database. Instead
-- the fixture keeps one fixed synthetic person set, replaces the rows that are
-- safe to replace (proposals, account links, approved profile values), and
-- re-seeds the append-only rows idempotently. The source document and evidence
-- are kept for the same reason: deleting the evidence would leave
-- `assertion_evidence` rows with a null source.

BEGIN;

-- Replace rows that may be re-created. Proposal events are append-only and
-- intentionally survive; they carry no content and are never read by key.
--
-- NOTE: an approved profile revision survives and is immutable, so a value is
-- always rebuilt from that revision's own text. Changing `MARKER` or
-- `FAMILY_MARKER` in scripts/web-e2e.sh therefore requires recreating the
-- disposable databases; the harness fails loudly when the marker is absent
-- rather than silently testing nothing.
DELETE FROM genealogy_fact_proposal
 WHERE person_id IN (
     SELECT person_id FROM person WHERE person_key LIKE 'e2e-branch-%'
 );
-- Account ids change on every harness run, so the reviewed links are rebuilt.
DELETE FROM onboarding_person_account_link
 WHERE person_id IN (
     SELECT person_id FROM person WHERE person_key LIKE 'e2e-branch-%'
 );
-- Approved values are re-created below from the surviving approved revisions,
-- so the marker text can never drift from the revision it is bound to.
DELETE FROM onboarding_profile_value
 WHERE person_id IN (
     SELECT person_id FROM person WHERE person_key LIKE 'e2e-branch-%'
 );

INSERT INTO source_document (relative_path, sha256, source_kind)
VALUES ('synthetic/e2e-branch-review-source.md', repeat('b', 64), 'original')
ON CONFLICT (relative_path) DO NOTHING;

INSERT INTO source_evidence (source_document_id, page, locator)
SELECT source_document_id, 1, 'p.1'
  FROM source_document
 WHERE relative_path = 'synthetic/e2e-branch-review-source.md'
ON CONFLICT (source_document_id, page, locator) DO NOTHING;

-- A review subject in branch 1, a second-branch person the reviewer may not
-- touch, the reviewer's own linked person (linked-owner independence), a
-- dedicated identity the security suite grants and then deactivates, and a
-- dedicated linked member for the family profile suite.
INSERT INTO person (person_key, display_name) VALUES
    ('e2e-branch-subject', 'Synthetic Branch Subject'),
    ('e2e-branch-other', 'Synthetic Second Branch Subject'),
    ('e2e-branch-owner', 'Synthetic Linked Reviewer Person'),
    ('e2e-branch-off', 'Synthetic Offboarding Reviewer Person'),
    ('e2e-branch-family-member', 'Synthetic Family Profile Member'),
    ('e2e-branch-restrictable', 'Synthetic Restrictable Member')
ON CONFLICT (person_key) DO UPDATE SET display_name = EXCLUDED.display_name;

-- Verified branch claims for the people the branch review suite reviews. The
-- assertions themselves are append-only in practice: an approval writes review
-- events, so they are inserted only when absent.
WITH branch_claims (person_key, branch_key) AS (
    VALUES ('e2e-branch-subject', 'branch-1'),
           ('e2e-branch-other', 'branch-2'),
           ('e2e-branch-owner', 'branch-1'),
           ('e2e-branch-off', 'branch-1')
)
INSERT INTO assertion
    (person_id, assertion_kind, assertion_key, value_text, review_status)
SELECT p.person_id, 'branch_claim', bc.branch_key, bc.branch_key, 'unverified'
  FROM branch_claims bc
  JOIN person p ON p.person_key = bc.person_key
ON CONFLICT (person_id, assertion_kind, assertion_key) DO NOTHING;

-- Evidence for every seeded claim, not only newly inserted ones, so a partial
-- previous run is repaired. The deferred verified-assertion constraint is
-- checked at COMMIT.
INSERT INTO assertion_evidence
    (assertion_id, source_evidence_id, document_ref, page)
SELECT a.assertion_id, seeded.source_evidence_id, seeded.relative_path, 1
  FROM assertion a
  JOIN person p ON p.person_id = a.person_id
 CROSS JOIN (
     SELECT se.source_evidence_id, sd.relative_path
       FROM source_evidence se
       JOIN source_document sd ON sd.source_document_id = se.source_document_id
      WHERE sd.relative_path = 'synthetic/e2e-branch-review-source.md'
 ) AS seeded
 WHERE p.person_key LIKE 'e2e-branch-%'
   AND a.assertion_kind = 'branch_claim'
ON CONFLICT (assertion_id, document_ref, page) DO NOTHING;

-- Repair evidence left behind by an older teardown that deleted the source:
-- the evidence row survives, but its `source_evidence_id` was set to NULL by
-- the foreign key. Re-link it so the fixture heals on the next run.
UPDATE assertion_evidence ae
   SET source_evidence_id = seeded.source_evidence_id
  FROM assertion a
  JOIN person p ON p.person_id = a.person_id
 CROSS JOIN (
     SELECT se.source_evidence_id, sd.relative_path
       FROM source_evidence se
       JOIN source_document sd ON sd.source_document_id = se.source_document_id
      WHERE sd.relative_path = 'synthetic/e2e-branch-review-source.md'
 ) AS seeded
 WHERE ae.assertion_id = a.assertion_id
   AND p.person_key LIKE 'e2e-branch-%'
   AND a.assertion_kind = 'branch_claim'
   AND ae.source_evidence_id IS NULL;

UPDATE assertion a
   SET review_status = 'verified',
       reviewed_by = 'e2e-seed',
       reviewed_at = now()
  FROM person p
 WHERE p.person_id = a.person_id
   AND p.person_key LIKE 'e2e-branch-%'
   AND a.assertion_kind = 'branch_claim';

-- One approved owner-private profile value for the person under review. The
-- guard only accepts an approved value that matches an approved revision, so
-- this walks the same draft -> submitted -> approved path the product uses.
-- The revision survives between runs (terminal revisions are immutable), so it
-- is only inserted when absent.
INSERT INTO onboarding_profile_revision
    (person_id, field_key, prior_value_text, prior_approved_revision_number,
     new_value_text, changed_by, revision_number, status, visibility)
SELECT p.person_id, 'current_location', NULL, 0, :'marker', 'e2e-seed', 1,
       'draft', 'owner_private'
  FROM person p
 WHERE p.person_key = 'e2e-branch-subject'
   AND NOT EXISTS (
       SELECT 1 FROM onboarding_profile_revision r
        WHERE r.person_id = p.person_id AND r.field_key = 'current_location'
   );

UPDATE onboarding_profile_revision r
   SET status = 'pending_review', submitted_at = now()
  FROM person p
 WHERE p.person_id = r.person_id
   AND p.person_key = 'e2e-branch-subject'
   AND r.field_key = 'current_location'
   AND r.status = 'draft';

UPDATE onboarding_profile_revision r
   SET status = 'approved', reviewed_at = now(),
       reviewed_by = 'e2e-seed-reviewer', review_note = 'synthetic fixture'
  FROM person p
 WHERE p.person_id = r.person_id
   AND p.person_key = 'e2e-branch-subject'
   AND r.field_key = 'current_location'
   AND r.status = 'pending_review';

INSERT INTO onboarding_profile_value
    (person_id, field_key, value_text, visibility, updated_by,
     approved_revision_number, approved_revision_id)
SELECT r.person_id, r.field_key, r.new_value_text, r.visibility, 'e2e-seed',
       r.revision_number, r.revision_id
  FROM onboarding_profile_revision r
  JOIN person p ON p.person_id = r.person_id
 WHERE p.person_key = 'e2e-branch-subject'
   AND r.field_key = 'current_location'
   AND r.status = 'approved';

-- One approved family-visible biography for the same person, so the family
-- profile endpoint has exactly one value to publish beside the private one.
INSERT INTO onboarding_profile_revision
    (person_id, field_key, prior_value_text, prior_approved_revision_number,
     new_value_text, changed_by, revision_number, status, visibility)
SELECT p.person_id, 'biography', NULL, 0, :'family_marker', 'e2e-seed', 1,
       'draft', 'family'
  FROM person p
 WHERE p.person_key = 'e2e-branch-subject'
   AND NOT EXISTS (
       SELECT 1 FROM onboarding_profile_revision r
        WHERE r.person_id = p.person_id AND r.field_key = 'biography'
   );

UPDATE onboarding_profile_revision r
   SET status = 'pending_review', submitted_at = now()
  FROM person p
 WHERE p.person_id = r.person_id
   AND p.person_key = 'e2e-branch-subject'
   AND r.field_key = 'biography'
   AND r.status = 'draft';

UPDATE onboarding_profile_revision r
   SET status = 'approved', reviewed_at = now(),
       reviewed_by = 'e2e-seed-reviewer', review_note = 'synthetic fixture'
  FROM person p
 WHERE p.person_id = r.person_id
   AND p.person_key = 'e2e-branch-subject'
   AND r.field_key = 'biography'
   AND r.status = 'pending_review';

INSERT INTO onboarding_profile_value
    (person_id, field_key, value_text, visibility, updated_by,
     approved_revision_number, approved_revision_id)
SELECT r.person_id, r.field_key, r.new_value_text, r.visibility, 'e2e-seed',
       r.revision_number, r.revision_id
  FROM onboarding_profile_revision r
  JOIN person p ON p.person_id = r.person_id
 WHERE p.person_key = 'e2e-branch-subject'
   AND r.field_key = 'biography'
   AND r.status = 'approved';

-- A draft for the same person that is never approved. It has no approved
-- value row, so the family profile page must never render it. The guard is by
-- field, not by status: the fixture never approves this field, and the guard
-- keeps a rerun from creating a second revision for it.
INSERT INTO onboarding_profile_revision
    (person_id, field_key, prior_value_text, prior_approved_revision_number,
     new_value_text, changed_by, revision_number, status, visibility)
SELECT p.person_id, 'my_story', NULL, 0, :'draft_marker', 'e2e-owner', 1,
       'draft', 'family'
  FROM person p
 WHERE p.person_key = 'e2e-branch-subject'
   AND NOT EXISTS (
       SELECT 1 FROM onboarding_profile_revision r
        WHERE r.person_id = p.person_id AND r.field_key = 'my_story'
   );

INSERT INTO onboarding_person_account_link
    (person_id, account_id, linked_by, identity_fingerprint)
SELECT p.person_id, :reviewer_account_id, 'e2e-seed', :'reviewer_fingerprint'
  FROM person p
 WHERE p.person_key = 'e2e-branch-owner';

-- Dedicated to the deactivation scenario so the shared reviewer stays active.
INSERT INTO onboarding_person_account_link
    (person_id, account_id, linked_by, identity_fingerprint)
SELECT p.person_id, :off_account_id, 'e2e-seed', :'off_fingerprint'
  FROM person p
 WHERE p.person_key = 'e2e-branch-off';

-- Dedicated linked member for the family profile revocation/deactivation
-- scenario, so the shared reviewer's link is never touched.
-- Fixture reset for the owner-privacy scenario. A previous interrupted run
-- may have left the dedicated field restricted; lift it the way the product
-- records a lift, so the next run starts from family visibility again. The
-- product path itself (a new approved family revision) is exercised by the
-- browser suite, not by this reset.
WITH active AS (
    SELECT r.restriction_id, r.generation, r.person_id, r.field_key
      FROM onboarding_profile_field_restriction r
      JOIN person p ON p.person_id = r.person_id
     WHERE p.person_key = 'e2e-branch-restrictable'
       AND r.field_key = 'biography'
       AND r.lifted_at IS NULL
), lifted AS (
    UPDATE onboarding_profile_field_restriction r
       SET lifted_at = now(), lifted_by = 'e2e-seed',
           lifted_generation = a.generation
      FROM active a
     WHERE r.restriction_id = a.restriction_id
    RETURNING r.restriction_id, r.person_id, r.field_key, r.generation
)
INSERT INTO onboarding_profile_field_restriction_event
    (restriction_id, person_id, field_key, generation, event, actor,
     prior_visibility, new_visibility)
SELECT restriction_id, person_id, field_key, generation, 'lifted',
       'e2e-seed', 'owner_private', 'family'
  FROM lifted;

-- One approved family-visible biography for the restrictable member, so the
-- owner can make it private and the family read can be verified.
INSERT INTO onboarding_profile_revision
    (person_id, field_key, prior_value_text, prior_approved_revision_number,
     new_value_text, changed_by, revision_number, status, visibility)
SELECT p.person_id, 'biography', NULL, 0, :'restrict_marker', 'e2e-seed', 1,
       'draft', 'family'
  FROM person p
 WHERE p.person_key = 'e2e-branch-restrictable'
   AND NOT EXISTS (
       SELECT 1 FROM onboarding_profile_revision r
        WHERE r.person_id = p.person_id AND r.field_key = 'biography'
   );

UPDATE onboarding_profile_revision r
   SET status = 'pending_review', submitted_at = now()
  FROM person p
 WHERE p.person_id = r.person_id
   AND p.person_key = 'e2e-branch-restrictable'
   AND r.field_key = 'biography'
   AND r.status = 'draft';

UPDATE onboarding_profile_revision r
   SET status = 'approved', reviewed_at = now(),
       reviewed_by = 'e2e-seed-reviewer', review_note = 'synthetic fixture'
  FROM person p
 WHERE p.person_id = r.person_id
   AND p.person_key = 'e2e-branch-restrictable'
   AND r.field_key = 'biography'
   AND r.status = 'pending_review';

INSERT INTO onboarding_profile_value
    (person_id, field_key, value_text, visibility, updated_by,
     approved_revision_number, approved_revision_id)
SELECT r.person_id, r.field_key, r.new_value_text, r.visibility, 'e2e-seed',
       r.revision_number, r.revision_id
  FROM onboarding_profile_revision r
  JOIN person p ON p.person_id = r.person_id
 WHERE p.person_key = 'e2e-branch-restrictable'
   AND r.field_key = 'biography'
   AND r.status = 'approved'
ON CONFLICT (person_id, field_key) DO NOTHING;

INSERT INTO onboarding_person_account_link
    (person_id, account_id, linked_by, identity_fingerprint)
SELECT p.person_id, :family_account_id, 'e2e-seed', :'family_fingerprint'
  FROM person p
 WHERE p.person_key = 'e2e-branch-family-member';

-- Dedicated owner for the privacy-restriction scenario.
INSERT INTO onboarding_person_account_link
    (person_id, account_id, linked_by, identity_fingerprint)
SELECT p.person_id, :restrict_account_id, 'e2e-seed', :'restrict_fingerprint'
  FROM person p
 WHERE p.person_key = 'e2e-branch-restrictable';

COMMIT;

-- TRAN-002H: verified lifespan for the family-profile fixture, so the browser
-- suite proves a reviewed date reaches the family page. Synthetic years only.
-- The assertion is inserted unverified, given evidence, and only then verified,
-- because the verified-assertion trigger is checked per statement here.
INSERT INTO assertion
    (person_id, assertion_kind, assertion_key, value_text, review_status,
     value_date_start, value_date_end, value_date_precision, value_date_qualifier)
SELECT p.person_id, seed.assertion_kind, seed.assertion_key, seed.value_text,
       'unverified', seed.value_date_start, NULL, seed.value_date_precision,
       seed.value_date_qualifier
  FROM (
      VALUES
          ('birth_date', 'birth_date:synthetic-e2e', '1901', '1901', 'year', 'exact'),
          ('death_date', 'death_date:synthetic-e2e', 'khoảng 1980', '1980', 'year', 'circa')
  ) AS seed(assertion_kind, assertion_key, value_text, value_date_start,
            value_date_precision, value_date_qualifier)
  JOIN person p ON p.person_key = 'e2e-branch-subject'
ON CONFLICT (person_id, assertion_kind, assertion_key) DO NOTHING;

INSERT INTO assertion_evidence
    (assertion_id, source_evidence_id, document_ref, page)
SELECT a.assertion_id, seeded.source_evidence_id, seeded.relative_path, 1
  FROM assertion a
  JOIN person p ON p.person_id = a.person_id
 CROSS JOIN (
     SELECT se.source_evidence_id, sd.relative_path
       FROM source_evidence se
       JOIN source_document sd ON sd.source_document_id = se.source_document_id
      WHERE sd.relative_path = 'synthetic/e2e-branch-review-source.md'
 ) AS seeded
 WHERE p.person_key = 'e2e-branch-subject'
   AND a.assertion_kind IN ('birth_date', 'death_date')
   AND a.assertion_key IN ('birth_date:synthetic-e2e', 'death_date:synthetic-e2e')
ON CONFLICT (assertion_id, document_ref, page) DO NOTHING;

UPDATE assertion a
   SET review_status = 'verified',
       reviewed_by = 'e2e-seed',
       reviewed_at = now()
  FROM person p
 WHERE p.person_id = a.person_id
   AND p.person_key = 'e2e-branch-subject'
   AND a.assertion_kind IN ('birth_date', 'death_date')
   AND a.assertion_key IN ('birth_date:synthetic-e2e', 'death_date:synthetic-e2e');

-- TRAN-002I: reviewed archive history for the family-profile fixture.
-- The approved, family-visible passage proves that source narrative reaches the
-- family page; the pending and admin-only passages prove it does not leak.
-- Wording is synthetic and contains no family content.
INSERT INTO source_narrative_passage
    (slot_key, passage_key, source_document_id, language, section_title,
     page_start, page_end, ordinal, body, content_sha256,
     review_status, visibility, reviewed_by, reviewed_at)
SELECT seed.slot_key, seed.passage_key, sd.source_document_id, 'vi',
       seed.section_title, seed.page_start, seed.page_end, seed.ordinal,
       seed.body, seed.content_sha256, seed.review_status, seed.visibility,
       seed.reviewed_by, now()
  FROM (
      SELECT
          'synthetic-e2e-history-approved'::text AS slot_key,
          'synthetic-e2e-history-approved'::text AS passage_key,
          'Synthetic archive section'::text AS section_title,
          8 AS page_start,
          8 AS page_end,
          1 AS ordinal,
          'SYNTHETIC-ARCHIVE-HISTORY-APPROVED paragraph one.' || chr(10) || chr(10) ||
          'SYNTHETIC-ARCHIVE-HISTORY-APPROVED paragraph two.' AS body,
          repeat('c', 64)::text AS content_sha256,
          'approved'::text AS review_status,
          'family'::text AS visibility,
          'e2e-seed'::text AS reviewed_by
      UNION ALL
      SELECT
          'synthetic-e2e-history-pending',
          'synthetic-e2e-history-pending',
          'Synthetic pending section',
          9, 9, 2,
          'SYNTHETIC-ARCHIVE-HISTORY-PENDING must never be published.',
          repeat('d', 64),
          'pending',
          'admin_only',
          NULL
      UNION ALL
      SELECT
          'synthetic-e2e-history-admin',
          'synthetic-e2e-history-admin',
          'Synthetic admin-only section',
          10, 10, 3,
          'SYNTHETIC-ARCHIVE-HISTORY-ADMIN must never be published to the family.',
          repeat('e', 64),
          'approved',
          'admin_only',
          'e2e-seed'
  ) AS seed
  JOIN source_document sd
    ON sd.relative_path = 'synthetic/e2e-branch-review-source.md'
ON CONFLICT (passage_key) DO NOTHING;

INSERT INTO source_narrative_link
    (passage_id, person_id, link_status, match_kind, reviewed_by, reviewed_at)
SELECT p.passage_id, pe.person_id, 'confirmed', 'section_heading', 'e2e-seed', now()
  FROM source_narrative_passage p
  JOIN person pe ON pe.person_key = 'e2e-branch-subject'
 WHERE p.passage_key IN (
     'synthetic-e2e-history-approved',
     'synthetic-e2e-history-pending',
     'synthetic-e2e-history-admin'
 )
ON CONFLICT (passage_id, person_id) DO NOTHING;
