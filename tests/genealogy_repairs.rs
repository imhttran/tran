//! Focused TRAN-002 repair regressions.
//!
//! These tests use a dedicated disposable target derived from
//! `GENEALOGY_DATABASE_URL` through `tests/support/genealogy_isolation.rs`.
//! They never touch the RAG database and drop only their own target.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn, create_isolated_target};
use postgres::Client;
use rag_foundation::genealogy::eligibility::{self, ReasonCode};
use rag_foundation::genealogy::reimport::{
    self, ImportAssertion, ImportDocument, ImportLink, ImportPerson,
};
use rag_foundation::genealogy::review::{self, ReviewError, ReviewRequest, ReviewStatus};
use rag_foundation::genealogy::tree;

fn import_document(sha: &str, evidence: bool) -> ImportDocument {
    ImportDocument {
        relative_path: "repair-catalog.md".to_string(),
        sha256: sha.to_string(),
        source_kind: "original".to_string(),
        persons: vec![ImportPerson {
            person_key: "repair-person".to_string(),
            display_name: "Repair Person".to_string(),
        }],
        assertions: vec![ImportAssertion {
            person_key: "repair-person".to_string(),
            kind: "display_name".to_string(),
            key: "display:1".to_string(),
            value_text: Some("Repair Person".to_string()),
            object_key: None,
            evidence: if evidence {
                vec![("repair-catalog.md".to_string(), 3)]
            } else {
                Vec::new()
            },
            date: None,
        }],
        links: Vec::new(),
    }
}

fn insert_verified(client: &mut Client, person_id: i64, kind: &str, key: &str, value: &str) -> i64 {
    let mut tx = client.transaction().expect("begin verified assertion");
    let assertion_id: i64 = tx
        .query_one(
            "INSERT INTO assertion
                (person_id, assertion_kind, assertion_key, value_text, review_status)
             VALUES ($1, $2, $3, $4, 'unverified')
             RETURNING assertion_id",
            &[&person_id, &kind, &key, &value],
        )
        .expect("insert verified assertion")
        .get(0);
    tx.execute(
        "INSERT INTO assertion_evidence (assertion_id, document_ref, page)
         VALUES ($1, 'repair-catalog.md', 3)",
        &[&assertion_id],
    )
    .expect("insert evidence");
    tx.execute(
        "UPDATE assertion
            SET review_status = 'verified', reviewed_by = 'synthetic-reviewer', reviewed_at = now()
          WHERE assertion_id = $1",
        &[&assertion_id],
    )
    .expect("mark verified");
    tx.commit().expect("commit verified assertion");
    assertion_id
}

#[test]
fn import_atomicity_provenance_hash_reopen_transitions_and_eligibility_inputs() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping repair integration test");
        return;
    };
    let target = create_isolated_target(&base, "repair");
    target.apply_migrations();

    let mut client = target.connect();

    // Atomicity: the valid person and the source row must roll back when a
    // later assertion references a missing person.
    let mut broken = import_document(&"a".repeat(64), false);
    broken.assertions[0].person_key = "missing-person".to_string();
    let err = reimport::reimport_document(&mut client, &broken)
        .expect_err("an assertion for a missing person must fail");
    assert!(err.to_string().contains("no rows returned") || err.to_string().contains("row"));
    let people: i64 = client
        .query_one(
            "SELECT count(*) FROM person WHERE person_key = 'repair-person'",
            &[],
        )
        .unwrap()
        .get(0);
    let documents: i64 = client
        .query_one(
            "SELECT count(*) FROM source_document WHERE relative_path = 'repair-catalog.md'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(people, 0, "failed import must not leave a person");
    assert_eq!(
        documents, 0,
        "failed import must not leave a source document"
    );

    // Provenance: evidence rows must link through source_evidence to the source
    // document; this is what makes hash changes visible.
    let report = reimport::reimport_document(&mut client, &import_document(&"a".repeat(64), true))
        .expect("provenance import succeeds");
    assert_eq!(report.inserted, 1);
    let (assertion_id, source_evidence_id): (i64, Option<i64>) = client
        .query_one(
            "SELECT ae.assertion_id, ae.source_evidence_id
               FROM assertion_evidence ae ORDER BY ae.assertion_evidence_id LIMIT 1",
            &[],
        )
        .map(|row| (row.get(0), row.get(1)))
        .expect("evidence row exists");
    let source_id: i64 = source_evidence_id.expect("source_evidence_id must be populated");
    let linked: i64 = client
        .query_one(
            "SELECT count(*) FROM source_evidence se
               JOIN source_document sd ON sd.source_document_id = se.source_document_id
              WHERE se.source_evidence_id = $1 AND sd.relative_path = 'repair-catalog.md'",
            &[&source_id],
        )
        .unwrap()
        .get(0);
    assert_eq!(linked, 1, "evidence must link to the source document");

    // Verify the assertion, then change the source hash and re-import.
    let outcome = review::review_assertion(
        &mut client,
        &ReviewRequest {
            assertion_id,
            new_status: ReviewStatus::Verified,
            reviewer: "synthetic-reviewer",
            evidence_ref: "repair-catalog.md p.3",
            note: None,
        },
    )
    .expect("synthetic verification succeeds");
    assert_eq!(outcome.to_status, "verified");

    let reopened =
        reimport::reimport_document(&mut client, &import_document(&"b".repeat(64), true))
            .expect("re-import with a changed source hash succeeds");
    assert_eq!(
        reopened.reopened, 1,
        "changed source hash must reopen evidence"
    );
    let status: String = client
        .query_one(
            "SELECT review_status FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .unwrap()
        .get(0);
    assert_eq!(status, "needs_review");

    // Transition enforcement: rejected cannot jump directly back to verified.
    review::review_assertion(
        &mut client,
        &ReviewRequest {
            assertion_id,
            new_status: ReviewStatus::Rejected,
            reviewer: "synthetic-reviewer",
            evidence_ref: "review decision",
            note: None,
        },
    )
    .expect("rejected transition is allowed");
    let invalid = review::review_assertion(
        &mut client,
        &ReviewRequest {
            assertion_id,
            new_status: ReviewStatus::Verified,
            reviewer: "synthetic-reviewer",
            evidence_ref: "repair-catalog.md p.3",
            note: None,
        },
    )
    .expect_err("rejected -> verified must be rejected");
    assert!(matches!(invalid, ReviewError::InvalidTransition(_, _)));

    // Eligibility inputs: load_input must populate deceased/contested evidence.
    let person_id: i64 = client
        .query_one(
            "INSERT INTO person (person_key, display_name)
             VALUES ('eligibility-person', 'Eligibility Person')
             RETURNING person_id",
            &[],
        )
        .unwrap()
        .get(0);
    insert_verified(&mut client, person_id, "branch_claim", "branch", "branch-3");
    insert_verified(&mut client, person_id, "gender", "sex", "male");
    insert_verified(&mut client, person_id, "birth_order", "order", "1");
    insert_verified(&mut client, person_id, "vital_status", "living", "living");
    insert_verified(
        &mut client,
        person_id,
        "contested_first_born",
        "contested",
        "disputed",
    );
    let input = eligibility::load_input(&mut client, "eligibility-person", "branch-3")
        .expect("load eligibility input");
    assert!(input.contested_first_born.is_some());
    let decision = eligibility::decide(&input);
    assert_eq!(decision.decision, "undetermined");
    assert_eq!(decision.reason_code, ReasonCode::ContestedFirstBorn);
    assert!(decision.requires_succession_policy);

    // Deceased first-born is loaded independently and also blocks succession.
    let deceased_id: i64 = client
        .query_one(
            "INSERT INTO person (person_key, display_name)
             VALUES ('eligibility-deceased', 'Eligibility Deceased')
             RETURNING person_id",
            &[],
        )
        .unwrap()
        .get(0);
    insert_verified(
        &mut client,
        deceased_id,
        "branch_claim",
        "branch",
        "branch-3",
    );
    insert_verified(&mut client, deceased_id, "gender", "sex", "male");
    insert_verified(&mut client, deceased_id, "birth_order", "order", "1");
    insert_verified(&mut client, deceased_id, "vital_status", "living", "living");
    insert_verified(
        &mut client,
        deceased_id,
        "deceased_first_born",
        "deceased",
        "deceased",
    );
    let deceased_input = eligibility::load_input(&mut client, "eligibility-deceased", "branch-3")
        .expect("load deceased eligibility input");
    assert!(deceased_input.deceased_first_born.is_some());
    let deceased_decision = eligibility::decide(&deceased_input);
    assert_eq!(deceased_decision.reason_code, ReasonCode::DeceasedFirstBorn);
    assert!(deceased_decision.requires_succession_policy);

    drop(client);
    target.cleanup();
}

/// Regression for the re-import evidence path: the database trigger that
/// reopens a verified assertion removes its baseline, so the application must
/// capture the baseline before syncing and must reconcile removed evidence.
#[test]
fn evidence_changes_are_reconciled_and_reported_as_reopened() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping evidence reconciliation test");
        return;
    };
    let target = create_isolated_target(&base, "reimport");
    target.apply_migrations();

    let mut client = target.connect();

    let mut document = import_document(&"c".repeat(64), true);
    let report = reimport::reimport_document(&mut client, &document).expect("initial import");
    assert_eq!(report.inserted, 1);

    let assertion_id: i64 = client
        .query_one("SELECT assertion_id FROM assertion", &[])
        .expect("read assertion")
        .get(0);

    let verify = || ReviewRequest {
        assertion_id,
        new_status: ReviewStatus::Verified,
        reviewer: "synthetic-reviewer",
        evidence_ref: "repair-catalog.md p.3",
        note: None,
    };
    review::review_assertion(&mut client, &verify()).expect("verify the assertion");
    let baseline: i64 = client
        .query_one(
            "SELECT count(*) FROM assertion_review_baseline WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .expect("read baseline")
        .get(0);
    assert_eq!(baseline, 1, "verification records a baseline");

    // Change the evidence page. This INSERT fires the reopen trigger, which
    // used to delete the baseline before the decision was made.
    document.assertions[0].evidence = vec![("repair-catalog.md".to_string(), 4)];
    let report =
        reimport::reimport_document(&mut client, &document).expect("re-import changed evidence");
    assert_eq!(
        report.reopened, 1,
        "changed evidence must be reported as reopened"
    );
    assert_eq!(report.unchanged, 0);

    let status: String = client
        .query_one(
            "SELECT review_status FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .expect("read status")
        .get(0);
    assert_eq!(status, "needs_review");

    let pages: Vec<i32> = client
        .query(
            "SELECT page FROM assertion_evidence WHERE assertion_id = $1 ORDER BY page",
            &[&assertion_id],
        )
        .expect("read evidence")
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(pages, vec![4], "the removed page-3 row is reconciled away");

    let events = |client: &mut Client| -> i64 {
        client
            .query_one(
                "SELECT count(*) FROM assertion_review_event
                  WHERE assertion_id = $1 AND note = 'evidence_changed'",
                &[&assertion_id],
            )
            .expect("count reopen events")
            .get(0)
    };
    assert_eq!(events(&mut client), 1, "exactly one reopen event");

    // Idempotent: the same input again must not reopen or duplicate the event.
    let report = reimport::reimport_document(&mut client, &document).expect("repeat re-import");
    assert_eq!(report.reopened, 0);
    assert_eq!(report.unchanged, 1);
    assert_eq!(events(&mut client), 1, "no duplicate audit event");

    // Removing all evidence must also reopen and leave no stale provenance.
    review::review_assertion(&mut client, &verify()).expect("re-verify the assertion");
    document.assertions[0].evidence = Vec::new();
    let report =
        reimport::reimport_document(&mut client, &document).expect("re-import without evidence");
    assert_eq!(report.reopened, 1, "removed evidence reopens the assertion");

    let status: String = client
        .query_one(
            "SELECT review_status FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .expect("read status after removal")
        .get(0);
    assert_eq!(status, "needs_review");
    let remaining: i64 = client
        .query_one(
            "SELECT count(*) FROM assertion_evidence WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .expect("count remaining evidence")
        .get(0);
    assert_eq!(remaining, 0, "no stale evidence survives the re-import");

    drop(client);
    target.cleanup();
}

/// A resolved relationship is imported once, reaches the authorized tree, and
/// survives a re-import without losing a review decision.
#[test]
fn resolved_relationship_links_import_once_and_survive_reimport() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping relationship import test");
        return;
    };
    let target = create_isolated_target(&base, "relationships");
    target.apply_migrations();
    let mut client = target.connect();

    let document = ImportDocument {
        relative_path: "relationship-source.md".to_string(),
        sha256: "b".repeat(64),
        // The review gate only verifies claims against an `original` source, so
        // this fixture models the source document itself. Imported assertions
        // stay `unverified` until a reviewer acts.
        source_kind: "original".to_string(),
        persons: vec![
            ImportPerson {
                person_key: "P901".to_string(),
                display_name: "Person A".to_string(),
            },
            ImportPerson {
                person_key: "P902".to_string(),
                display_name: "Person B".to_string(),
            },
        ],
        assertions: vec![ImportAssertion {
            person_key: "P902".to_string(),
            kind: "parent_child".to_string(),
            key: "parent_child:1".to_string(),
            value_text: Some("Person A".to_string()),
            object_key: Some("P901".to_string()),
            evidence: vec![("relationship-source.md".to_string(), 7)],
            date: None,
        }],
        links: vec![ImportLink {
            kind: "parent_child".to_string(),
            from_key: "P901".to_string(),
            to_key: "P902".to_string(),
        }],
    };

    let first = reimport::reimport_document(&mut client, &document).expect("first import");
    assert_eq!(first.links_inserted, 1);

    let second = reimport::reimport_document(&mut client, &document).expect("repeat import");
    assert_eq!(
        second.links_inserted, 0,
        "re-import must not duplicate the edge"
    );
    assert_eq!(second.links_unchanged, 1);
    assert_eq!(
        second.inserted, 0,
        "re-import must not duplicate the assertion"
    );

    let links: i64 = client
        .query_one("SELECT count(*) FROM parent_child_link", &[])
        .expect("count links")
        .get(0);
    assert_eq!(links, 1);

    let (object_key, status): (Option<String>, String) = client
        .query_one(
            "SELECT p.person_key, a.review_status
               FROM assertion a
               LEFT JOIN person p ON p.person_id = a.object_person_id
              WHERE a.assertion_key = 'parent_child:1'",
            &[],
        )
        .map(|row| (row.get(0), row.get(1)))
        .expect("assertion binding");
    assert_eq!(object_key.as_deref(), Some("P901"));
    assert_eq!(
        status, "unverified",
        "an imported relationship is never verified"
    );

    let graph = tree::load_tree(&mut client).expect("tree");
    assert_eq!(graph.links.len(), 1);
    assert_eq!(graph.links[0].kind, "parent_child");
    assert_eq!(graph.links[0].from_key, "P901");
    assert_eq!(graph.links[0].to_key, "P902");
    assert_eq!(graph.links[0].evidence_status, "unverified");

    let assertion_id: i64 = client
        .query_one(
            "SELECT assertion_id FROM assertion WHERE assertion_key = 'parent_child:1'",
            &[],
        )
        .expect("assertion id")
        .get(0);
    review::review_assertion(
        &mut client,
        &ReviewRequest {
            assertion_id,
            new_status: ReviewStatus::Verified,
            reviewer: "synthetic-reviewer",
            evidence_ref: "relationship-source.md p.7",
            note: None,
        },
    )
    .expect("verify the relationship assertion");

    let third = reimport::reimport_document(&mut client, &document).expect("third import");
    assert_eq!(
        third.reopened, 0,
        "unchanged evidence must not reopen a decision"
    );

    let status: String = client
        .query_one(
            "SELECT review_status FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .expect("status")
        .get(0);
    assert_eq!(
        status, "verified",
        "re-import must preserve the review decision"
    );

    let graph = tree::load_tree(&mut client).expect("tree after review");
    assert_eq!(graph.links.len(), 1, "the edge is still a single row");
    assert_eq!(graph.links[0].evidence_status, "verified");
    assert!(
        graph.links[0]
            .citations
            .iter()
            .any(|citation| citation.page == 7),
        "a verified edge keeps its source citation"
    );
}

/// A spouse edge is one relationship, not two rows. If the source orders the
/// couple the other way round from an existing row, the import reuses it.
#[test]
fn a_reversed_spouse_edge_is_reused_not_duplicated() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping spouse orientation test");
        return;
    };
    let target = create_isolated_target(&base, "spouse-orientation");
    target.apply_migrations();
    let mut client = target.connect();

    for (key, name) in [("P801", "Person A"), ("P802", "Person B")] {
        client
            .execute(
                "INSERT INTO person (person_key, display_name) VALUES ($1, $2)",
                &[&key, &name],
            )
            .expect("insert person");
    }
    // The pre-existing row stores the couple in the opposite orientation.
    client
        .execute(
            "INSERT INTO spouse_link (person_id, spouse_id, source_order)
             SELECT second.person_id, first.person_id, 1
               FROM person first, person second
              WHERE first.person_key = 'P801' AND second.person_key = 'P802'",
            &[],
        )
        .expect("insert reversed spouse link");

    let document = ImportDocument {
        relative_path: "relationship-source.md".to_string(),
        sha256: "d".repeat(64),
        source_kind: "original".to_string(),
        persons: vec![
            ImportPerson {
                person_key: "P801".to_string(),
                display_name: "Person A".to_string(),
            },
            ImportPerson {
                person_key: "P802".to_string(),
                display_name: "Person B".to_string(),
            },
        ],
        assertions: Vec::new(),
        links: vec![ImportLink {
            kind: "spouse".to_string(),
            from_key: "P801".to_string(),
            to_key: "P802".to_string(),
        }],
    };

    let report = reimport::reimport_document(&mut client, &document).expect("import spouse link");
    assert_eq!(report.links_inserted, 0, "the reversed row is reused");
    assert_eq!(report.links_unchanged, 1);

    let rows: i64 = client
        .query_one("SELECT count(*) FROM spouse_link", &[])
        .expect("count spouse links")
        .get(0);
    assert_eq!(rows, 1, "one couple stays one row");
}
