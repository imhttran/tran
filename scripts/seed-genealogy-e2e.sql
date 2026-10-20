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
    DELETE FROM head_of_family_audit_event;
    DELETE FROM head_of_family_appointment;
    DELETE FROM head_of_family_nomination;
    DELETE FROM person WHERE person_key = 'synthetic-e2e-person';
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
END $$;
