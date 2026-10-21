-- Synthetic genealogy fixture for the browser/API E2E suite.
--
-- This file is executed only against the disposable GENEALOGY_DATABASE_URL.
-- It never contains real family names or source material.

DO $$
DECLARE
    v_person_id BIGINT;
    v_source_id BIGINT;
    v_evidence_id BIGINT;
    v_assertion_id BIGINT;
    item RECORD;
BEGIN
    -- Check the verified-assertion evidence rule immediately. At commit time a
    -- deferred check can observe the fixture's earlier source-document cleanup
    -- rather than the evidence inserted by this block.
    EXECUTE 'SET CONSTRAINTS ALL IMMEDIATE';
    DELETE FROM head_of_family_audit_event;
    DELETE FROM head_of_family_appointment;
    DELETE FROM head_of_family_nomination;
    DELETE FROM person WHERE person_key = 'synthetic-e2e-person';
    DELETE FROM person WHERE person_key LIKE 'synthetic-spouse-%';
    DELETE FROM source_document WHERE relative_path = 'synthetic/e2e.pdf';

    INSERT INTO person (person_key, display_name)
        VALUES ('synthetic-e2e-person', 'Synthetic E2E Person')
        RETURNING person_id INTO v_person_id;

    INSERT INTO source_document (relative_path, sha256, source_kind)
        VALUES (
            'synthetic/e2e.pdf',
            'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
            'original'
        )
        RETURNING source_document_id INTO v_source_id;

    INSERT INTO source_evidence (source_document_id, page, locator)
        VALUES (v_source_id, 1, 'p.1')
        RETURNING source_evidence_id INTO v_evidence_id;

    FOR item IN
        SELECT * FROM (VALUES
            ('branch_claim', 'branch', 'branch-3'),
            ('gender', 'gender', 'male'),
            ('birth_order', 'order', 'first'),
            ('vital_status', 'living', 'living')
        ) AS t(kind, assertion_key, assertion_value)
    LOOP
        INSERT INTO assertion
            (person_id, assertion_kind, assertion_key, value_text, review_status)
            VALUES (v_person_id, item.kind, item.assertion_key, item.assertion_value, 'unverified')
            RETURNING assertion_id INTO v_assertion_id;

        INSERT INTO assertion_evidence
            (assertion_id, source_evidence_id, document_ref, page)
            VALUES (v_assertion_id, v_evidence_id, 'synthetic/e2e.pdf', 1);

        UPDATE assertion
           SET review_status = 'verified',
               reviewed_by = 'synthetic-reviewer',
               reviewed_at = now()
         WHERE assertion_id = v_assertion_id;
    END LOOP;

    -- Two direction-sensitive spouse cases for the read-only graph:
    -- pair A/B: B->A verified, A->B disputed => needs_review
    -- pair C/D: D->C verified only => verified with a citation.
    INSERT INTO person (person_key, display_name)
    VALUES
      ('synthetic-spouse-a', 'Synthetic Partner A'),
      ('synthetic-spouse-b', 'Synthetic Partner B'),
      ('synthetic-spouse-c', 'Synthetic Partner C'),
      ('synthetic-spouse-d', 'Synthetic Partner D');

    INSERT INTO spouse_link (person_id, spouse_id, source_order)
    SELECT a.person_id, b.person_id, 1
      FROM person a CROSS JOIN person b
     WHERE a.person_key = 'synthetic-spouse-a'
       AND b.person_key = 'synthetic-spouse-b';

    INSERT INTO spouse_link (person_id, spouse_id, source_order)
    SELECT a.person_id, b.person_id, 1
      FROM person a CROSS JOIN person b
     WHERE a.person_key = 'synthetic-spouse-c'
       AND b.person_key = 'synthetic-spouse-d';

    INSERT INTO assertion
      (person_id, object_person_id, assertion_kind, assertion_key,
       value_text, review_status)
    SELECT b.person_id, a.person_id, 'spouse', 'reverse_verified',
           a.person_key, 'unverified'
      FROM person a CROSS JOIN person b
     WHERE a.person_key = 'synthetic-spouse-a'
       AND b.person_key = 'synthetic-spouse-b'
    RETURNING assertion_id INTO v_assertion_id;
    INSERT INTO assertion_evidence
      (assertion_id, source_evidence_id, document_ref, page)
    VALUES (v_assertion_id, v_evidence_id, 'synthetic/e2e.pdf', 1);
    UPDATE assertion
       SET review_status = 'verified',
           reviewed_by = 'synthetic-reviewer', reviewed_at = now()
     WHERE assertion_id = v_assertion_id;

    INSERT INTO assertion
      (person_id, object_person_id, assertion_kind, assertion_key,
       value_text, review_status)
    SELECT a.person_id, b.person_id, 'spouse', 'forward_disputed',
           b.person_key, 'disputed'
      FROM person a CROSS JOIN person b
     WHERE a.person_key = 'synthetic-spouse-a'
       AND b.person_key = 'synthetic-spouse-b';

    INSERT INTO assertion
      (person_id, object_person_id, assertion_kind, assertion_key,
       value_text, review_status)
    SELECT d.person_id, c.person_id, 'spouse', 'reverse_only',
           c.person_key, 'unverified'
      FROM person c CROSS JOIN person d
     WHERE c.person_key = 'synthetic-spouse-c'
       AND d.person_key = 'synthetic-spouse-d'
    RETURNING assertion_id INTO v_assertion_id;
    INSERT INTO assertion_evidence
      (assertion_id, source_evidence_id, document_ref, page)
    VALUES (v_assertion_id, v_evidence_id, 'synthetic/e2e.pdf', 1);
    UPDATE assertion
       SET review_status = 'verified',
           reviewed_by = 'synthetic-reviewer', reviewed_at = now()
     WHERE assertion_id = v_assertion_id;
END $$;
