//! TRAN-002 provenance verification regressions.
//!
//! These tests use a dedicated disposable target derived from
//! `GENEALOGY_DATABASE_URL` through `tests/support/genealogy_isolation.rs`.
//! They never touch the RAG database and drop only their own target.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn, create_isolated_target};
use postgres::Client;
use rag_foundation::genealogy::review::{self, ReviewError, ReviewRequest, ReviewStatus};

fn insert_source(client: &mut Client, path: &str, kind: &str, derived_from: Option<i64>) -> i64 {
    let sha = "c".repeat(64);
    client
        .query_one(
            "INSERT INTO source_document
                (relative_path, sha256, source_kind, derived_from_source_document_id)
             VALUES ($1, $2, $3, $4) RETURNING source_document_id",
            &[&path, &sha, &kind, &derived_from],
        )
        .expect("insert source document")
        .get(0)
}

fn insert_evidence(client: &mut Client, source_id: i64, page: i32, locator: &str) -> i64 {
    client
        .query_one(
            "INSERT INTO source_evidence (source_document_id, page, locator)
             VALUES ($1, $2, $3) RETURNING source_evidence_id",
            &[&source_id, &page, &locator],
        )
        .expect("insert source evidence")
        .get(0)
}

fn insert_assertion(client: &mut Client, person_id: i64, key: &str, evidence: Option<i64>) -> i64 {
    let assertion_id: i64 = client
        .query_one(
            "INSERT INTO assertion
                (person_id, assertion_kind, assertion_key, value_text, review_status)
             VALUES ($1, 'display_name', $2, 'Synthetic Person', 'unverified')
             RETURNING assertion_id",
            &[&person_id, &key],
        )
        .expect("insert assertion")
        .get(0);
    if let Some(source_evidence_id) = evidence {
        client
            .execute(
                "INSERT INTO assertion_evidence
                    (assertion_id, source_evidence_id, document_ref, page)
                 VALUES ($1, $2, 'synthetic.md', 3)",
                &[&assertion_id, &source_evidence_id],
            )
            .expect("insert linked evidence");
    }
    assertion_id
}

fn review_verified(client: &mut Client, assertion_id: i64) -> Result<(), ReviewError> {
    review::review_assertion(
        client,
        &ReviewRequest {
            assertion_id,
            new_status: ReviewStatus::Verified,
            reviewer: "synthetic-reviewer",
            evidence_ref: "synthetic evidence",
            note: None,
        },
    )
    .map(|_| ())
}

#[test]
fn verification_requires_authoritative_linked_evidence() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping provenance test");
        return;
    };
    let target = create_isolated_target(&base, "provenance");
    target.apply_migrations();

    let mut client = target.connect();

    let original = insert_source(&mut client, "Family Genealogy.pdf", "original", None);
    let catalog = insert_source(
        &mut client,
        "Tran-Family-People-Catalog.md",
        "catalog",
        Some(original),
    );
    let original_evidence = insert_evidence(&mut client, original, 3, "page");
    let catalog_evidence = insert_evidence(&mut client, catalog, 3, "page");

    let person_id: i64 = client
        .query_one(
            "INSERT INTO person (person_key, display_name)
             VALUES ('provenance-person', 'Synthetic Person') RETURNING person_id",
            &[],
        )
        .expect("insert person")
        .get(0);

    // 1. No evidence at all.
    let no_evidence = insert_assertion(&mut client, person_id, "no-evidence", None);
    assert!(matches!(
        review_verified(&mut client, no_evidence),
        Err(ReviewError::NoEvidence(id)) if id == no_evidence
    ));

    // 2. Evidence row with a null source_evidence_id.
    let null_link: i64 = client
        .query_one(
            "INSERT INTO assertion
                (person_id, assertion_kind, assertion_key, value_text, review_status)
             VALUES ($1, 'display_name', 'null-link', 'Synthetic Person', 'unverified')
             RETURNING assertion_id",
            &[&person_id],
        )
        .expect("insert null-link assertion")
        .get(0);
    client
        .execute(
            "INSERT INTO assertion_evidence (assertion_id, document_ref, page)
             VALUES ($1, 'synthetic.md', 3)",
            &[&null_link],
        )
        .expect("insert unlinked evidence");
    assert!(matches!(
        review_verified(&mut client, null_link),
        Err(ReviewError::Unprovenanced(id, reason))
            if id == null_link && reason.contains("not linked")
    ));

    // 3. Evidence linked only to a derivative catalog.
    let derivative = insert_assertion(&mut client, person_id, "derivative", Some(catalog_evidence));
    client
        .execute(
            "INSERT INTO assertion_review_event
                (assertion_id, reviewer, from_status, to_status, evidence_ref, note)
             VALUES ($1, 'prior-reviewer', 'unverified', 'needs_review', 'prior', 'prior')",
            &[&derivative],
        )
        .expect("seed prior review history");
    assert!(matches!(
        review_verified(&mut client, derivative),
        Err(ReviewError::Unprovenanced(id, reason))
            if id == derivative && reason.contains("derivative")
    ));
    let history: i64 = client
        .query_one(
            "SELECT count(*) FROM assertion_review_event WHERE assertion_id = $1",
            &[&derivative],
        )
        .expect("count review history")
        .get(0);
    assert_eq!(
        history, 1,
        "failed verification must preserve review history"
    );

    // 4. Valid original-source evidence with a reviewed claim.
    let valid = insert_assertion(&mut client, person_id, "valid", Some(original_evidence));
    review_verified(&mut client, valid).expect("authoritative original evidence verifies");
    let row = client
        .query_one(
            "SELECT a.review_status,
                    (SELECT evidence_fingerprint FROM assertion_review_baseline b
                      WHERE b.assertion_id = a.assertion_id)
               FROM assertion a WHERE a.assertion_id = $1",
            &[&valid],
        )
        .expect("read verified assertion");
    let status: String = row.get(0);
    let baseline: Option<String> = row.get(1);
    assert_eq!(status, "verified");
    assert!(
        baseline.is_some(),
        "verified claim must record its baseline"
    );
    let events: i64 = client
        .query_one(
            "SELECT count(*) FROM assertion_review_event WHERE assertion_id = $1",
            &[&valid],
        )
        .expect("count valid review history")
        .get(0);
    assert_eq!(events, 1, "verification must append one review event");

    drop(client);
    target.cleanup();
}
