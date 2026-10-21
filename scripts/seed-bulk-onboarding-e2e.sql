-- Synthetic bulk-onboarding fixture for the browser/API E2E suite.
--
-- Run only against the disposable GENEALOGY_DATABASE_URL. It creates six
-- synthetic people: two eligible living candidates and four blocked cases
-- (deceased, disputed, already linked, already invited). It contains no real
-- family names, addresses, dates, or source material.

DO $$
DECLARE
    v_person_id BIGINT;
    v_source_id BIGINT;
    v_evidence_id BIGINT;
    v_assertion_id BIGINT;
    item RECORD;
BEGIN
    DELETE FROM onboarding_batch_audit;
    DELETE FROM onboarding_batch_item;
    DELETE FROM onboarding_batch;
    DELETE FROM onboarding_recipient_contact;
    DELETE FROM onboarding_link_audit;
    DELETE FROM onboarding_person_account_link;
    DELETE FROM onboarding_invitation;
    DELETE FROM onboarding_username_reservation;
    DELETE FROM person WHERE person_key LIKE 'synthetic-bulk-%';
    DELETE FROM source_document WHERE relative_path LIKE 'synthetic/bulk-%';

    FOR item IN
        SELECT * FROM (VALUES
            ('synthetic-bulk-eligible-1', 'Synthetic Bulk Eligible One', '1980', 'living', 'verified'),
            ('synthetic-bulk-eligible-2', 'Synthetic Bulk Eligible Two', '1985', 'living', 'verified'),
            ('synthetic-bulk-disputed', 'Synthetic Bulk Disputed Person', '1975', 'living', 'disputed'),
            ('synthetic-bulk-linked', 'Synthetic Bulk Linked Person', '1970', 'living', 'verified'),
            ('synthetic-bulk-invited', 'Synthetic Bulk Invited Person', '1965', 'living', 'verified')
        ) AS t(person_key, display_name, birth_year, vital_key, review_status)
    LOOP
        INSERT INTO person (person_key, display_name, birth_year)
            VALUES (item.person_key, item.display_name, item.birth_year)
            RETURNING person_id INTO v_person_id;

        INSERT INTO source_document (relative_path, sha256, source_kind)
            VALUES ('synthetic/bulk-' || item.person_key || '.md', repeat('a', 64), 'original')
            RETURNING source_document_id INTO v_source_id;

        INSERT INTO source_evidence (source_document_id, page, locator)
            VALUES (v_source_id, 1, 'p.1')
            RETURNING source_evidence_id INTO v_evidence_id;

        INSERT INTO assertion
            (person_id, assertion_kind, assertion_key, value_text)
            VALUES (v_person_id, 'vital_status', item.vital_key, 'alive')
            RETURNING assertion_id INTO v_assertion_id;

        INSERT INTO assertion_evidence
            (assertion_id, source_evidence_id, document_ref, page)
            VALUES (v_assertion_id, v_evidence_id,
                    'synthetic/bulk-' || item.person_key || '.md', 1);

        IF item.review_status = 'verified' THEN
            UPDATE assertion
               SET review_status = 'verified',
                   reviewed_by = 'synthetic-reviewer',
                   reviewed_at = now()
             WHERE assertion_id = v_assertion_id;
        ELSE
            UPDATE assertion
               SET review_status = item.review_status,
                   reviewed_by = 'synthetic-reviewer',
                   reviewed_at = now()
             WHERE assertion_id = v_assertion_id;
        END IF;
    END LOOP;

    -- Deceased: no verified living evidence and an explicit death assertion.
    INSERT INTO person (person_key, display_name, birth_year)
        VALUES ('synthetic-bulk-deceased', 'Synthetic Bulk Deceased Person', '1940')
        RETURNING person_id INTO v_person_id;
    INSERT INTO assertion (person_id, assertion_kind, assertion_key, value_text)
        VALUES (v_person_id, 'vital_status', 'death', 'deceased');

    -- Already linked.
    INSERT INTO onboarding_person_account_link (person_id, account_id, linked_by)
        SELECT person_id, 9001, 'synthetic-reviewer'
          FROM person WHERE person_key = 'synthetic-bulk-linked';

    -- Already invited.
    INSERT INTO onboarding_invitation
        (person_id, token_hash, expires_at, status, created_by)
    SELECT person_id, lpad(to_hex(person_id::bigint), 64, '0'),
           now() + interval '1 hour', 'pending', 'synthetic-admin'
      FROM person WHERE person_key = 'synthetic-bulk-invited';
END;
$$;
