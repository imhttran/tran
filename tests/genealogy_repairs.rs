//! Focused TRAN-002 repair regressions.
//!
//! These tests use a dedicated disposable database derived from
//! `GENEALOGY_DATABASE_URL`; they never touch the RAG database.

use std::fs;
use std::str::FromStr;

use postgres::{Client, NoTls};
use rag_foundation::genealogy::eligibility::{self, ReasonCode};
use rag_foundation::genealogy::reimport::{self, ImportAssertion, ImportDocument, ImportPerson};
use rag_foundation::genealogy::review::{self, ReviewError, ReviewRequest, ReviewStatus};

const MIGRATIONS: [&str; 4] = [
    "genealogy/migrations/0001_genealogy_schema_version.sql",
    "genealogy/migrations/0002_catalog_keys.sql",
    "genealogy/migrations/0003_assertions_evidence_and_links.sql",
    "genealogy/migrations/0004_review_workflow.sql",
];

fn base_dsn() -> Option<String> {
    let dsn = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let config = postgres::Config::from_str(&dsn).ok()?;
    let database = config.get_dbname()?;
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable genealogy database {database:?}"
    );
    Some(dsn)
}

fn with_database(dsn: &str, database: &str) -> String {
    let scheme_end = dsn.find("://").expect("dsn scheme") + 3;
    let authority_end = dsn[scheme_end..]
        .find('/')
        .map(|index| scheme_end + index)
        .unwrap_or(dsn.len());
    let query = dsn[authority_end..]
        .find('?')
        .map(|index| &dsn[authority_end + index..])
        .unwrap_or("");
    format!("{}/{}{}", &dsn[..authority_end], database, query)
}

fn ensure_database(admin_dsn: &str, database: &str) {
    let mut client = Client::connect(admin_dsn, NoTls).expect("connect to postgres");
    let exists: i64 = client
        .query_one(
            "SELECT count(*) FROM pg_database WHERE datname = $1",
            &[&database],
        )
        .expect("check database")
        .get(0);
    if exists == 0 {
        client
            .batch_execute(&format!("CREATE DATABASE {database};"))
            .expect("create disposable database");
    }
}

fn reset_schema(client: &mut Client) {
    client
        .batch_execute(
            "DROP TABLE IF EXISTS
                assertion_review_event,
                assertion_review_baseline,
                assertion_evidence,
                assertion,
                parent_child_link,
                spouse_link,
                catalog_entry,
                family_branch,
                source_evidence,
                source_document,
                gravesite,
                business_name,
                person_name_variant,
                person,
                genealogy_schema_version
             CASCADE;",
        )
        .expect("reset disposable genealogy schema");
}

fn apply_migrations(client: &mut Client) {
    for path in MIGRATIONS {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

fn import_document(sha: &str, evidence: bool) -> ImportDocument {
    ImportDocument {
        relative_path: "repair-catalog.md".to_string(),
        sha256: sha.to_string(),
        persons: vec![ImportPerson {
            person_key: "repair-person".to_string(),
            display_name: "Repair Person".to_string(),
        }],
        assertions: vec![ImportAssertion {
            person_key: "repair-person".to_string(),
            kind: "display_name".to_string(),
            key: "display:1".to_string(),
            value_text: Some("Repair Person".to_string()),
            evidence: if evidence {
                vec![("repair-catalog.md".to_string(), 3)]
            } else {
                Vec::new()
            },
        }],
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
    let repair_dsn = with_database(&base, "genealogy_repair_test");
    let admin_dsn = with_database(&base, "postgres");
    ensure_database(&admin_dsn, "genealogy_repair_test");

    let mut client = Client::connect(&repair_dsn, NoTls).expect("connect to repair test database");
    reset_schema(&mut client);
    apply_migrations(&mut client);

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

    reset_schema(&mut client);
}
