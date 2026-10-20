//! Synthetic Head of Family workflow tests (TRAN-003E).
//!
//! Uses a dedicated disposable genealogy database derived from
//! `GENEALOGY_DATABASE_URL`. It never touches real family data.

use std::str::FromStr;
use std::thread;

use postgres::{Client, NoTls};
use rag_foundation::genealogy::appointments;
use rag_foundation::genealogy::eligibility;
use rag_foundation::genealogy::GenealogyError;

const BRANCH: &str = "branch-3";

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
                head_of_family_audit_event,
                head_of_family_appointment,
                head_of_family_nomination,
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

fn migrate(client: &mut Client) {
    for migration in [
        include_str!("../genealogy/migrations/0001_genealogy_schema_version.sql"),
        include_str!("../genealogy/migrations/0002_catalog_keys.sql"),
        include_str!("../genealogy/migrations/0003_assertions_evidence_and_links.sql"),
        include_str!("../genealogy/migrations/0004_review_workflow.sql"),
        include_str!("../genealogy/migrations/0005_source_kind.sql"),
        include_str!("../genealogy/migrations/0006_head_of_family.sql"),
    ] {
        client
            .batch_execute(migration)
            .expect("apply genealogy migration");
    }
}

fn seed_person(client: &mut Client, key: &str, branch: &str, living: bool) {
    client
        .execute(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2)",
            &[&key, &format!("Synthetic {key}")],
        )
        .expect("insert person");
    client
        .execute(
            "INSERT INTO source_document (relative_path, sha256, source_kind) \
             VALUES ($1, $2, 'original')",
            &[&format!("synthetic/{key}.pdf"), &"a".repeat(64)],
        )
        .expect("insert source document");
    let source_evidence_id: i64 = client
        .query_one(
            "INSERT INTO source_evidence (source_document_id, page, locator) \
             SELECT source_document_id, 1, 'p.1' FROM source_document \
              WHERE relative_path = $1 RETURNING source_evidence_id",
            &[&format!("synthetic/{key}.pdf")],
        )
        .expect("insert source evidence")
        .get(0);

    let mut assertions = vec![
        ("branch_claim", "branch", branch),
        ("gender", "gender", "male"),
        ("birth_order", "order", "first"),
    ];
    if living {
        assertions.push(("vital_status", "living", "living"));
    }

    for (kind, assertion_key, value) in assertions {
        let assertion_id: i64 = client
            .query_one(
                "INSERT INTO assertion \
                 (person_id, assertion_kind, assertion_key, value_text, review_status) \
                 SELECT person_id, $2, $3, $4, 'unverified' FROM person \
                  WHERE person_key = $1 RETURNING assertion_id",
                &[&key, &kind, &assertion_key, &value],
            )
            .expect("insert assertion")
            .get(0);
        client
            .execute(
                "INSERT INTO assertion_evidence \
                 (assertion_id, source_evidence_id, document_ref, page) \
                 VALUES ($1, $2, $3, 1)",
                &[
                    &assertion_id,
                    &source_evidence_id,
                    &format!("synthetic/{key}.pdf"),
                ],
            )
            .expect("insert evidence");
        client
            .execute(
                "UPDATE assertion SET review_status = 'verified', \
                    reviewed_by = 'synthetic-reviewer', reviewed_at = now() \
                  WHERE assertion_id = $1",
                &[&assertion_id],
            )
            .expect("verify assertion");
    }
}

#[test]
fn eligible_candidate_can_be_nominated_approved_and_revoked() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping appointment test");
        return;
    };
    let database = "genealogy_appointments_approval_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_schema(&mut client);
    migrate(&mut client);
    seed_person(&mut client, "synthetic-first", BRANCH, true);

    let input = eligibility::load_input(&mut client, "synthetic-first", BRANCH).unwrap();
    assert!(eligibility::decide(&input).is_eligible());

    let nomination =
        appointments::nominate(&mut client, "synthetic-first", BRANCH, "editor").expect("nominate");
    let appointment =
        appointments::approve(&mut client, nomination, BRANCH, "admin").expect("approve");
    assert_eq!(appointment.person_key, "synthetic-first");

    let active = appointments::list_active_appointments(&mut client).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].person_key, "synthetic-first");

    appointments::end_appointment(
        &mut client,
        appointment.appointment_id,
        "admin",
        "review",
        false,
    )
    .expect("revoke");
    assert!(appointments::list_active_appointments(&mut client)
        .unwrap()
        .is_empty());
}

#[test]
fn ineligible_candidate_is_rejected_without_an_appointment() {
    let Some(base) = base_dsn() else {
        return;
    };
    let database = "genealogy_appointments_ineligible_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_schema(&mut client);
    migrate(&mut client);
    seed_person(&mut client, "synthetic-no-living", BRANCH, false);

    let nomination =
        appointments::nominate(&mut client, "synthetic-no-living", BRANCH, "editor").unwrap();
    let err = appointments::approve(&mut client, nomination, BRANCH, "admin").unwrap_err();
    match err {
        GenealogyError::Domain(message) => {
            assert!(message.contains("missing_living_evidence"), "{message}");
        }
        other => panic!("unexpected error: {other}"),
    }
    assert!(appointments::list_active_appointments(&mut client)
        .unwrap()
        .is_empty());
}

#[test]
fn concurrent_approvals_leave_at_most_one_active_appointment() {
    let Some(base) = base_dsn() else {
        return;
    };
    let database = "genealogy_appointments_concurrent_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_schema(&mut client);
    migrate(&mut client);
    seed_person(&mut client, "synthetic-a", BRANCH, true);
    seed_person(&mut client, "synthetic-b", BRANCH, true);

    let first = appointments::nominate(&mut client, "synthetic-a", BRANCH, "editor").unwrap();
    let second = appointments::nominate(&mut client, "synthetic-b", BRANCH, "editor").unwrap();

    let results = thread::scope(|scope| {
        let first_dsn = dsn.clone();
        let second_dsn = dsn.clone();
        let one = scope.spawn(move || {
            let mut client = Client::connect(&first_dsn, NoTls).expect("connect a");
            appointments::approve(&mut client, first, BRANCH, "admin-a").is_ok()
        });
        let two = scope.spawn(move || {
            let mut client = Client::connect(&second_dsn, NoTls).expect("connect b");
            appointments::approve(&mut client, second, BRANCH, "admin-b").is_ok()
        });
        (one.join().unwrap(), two.join().unwrap())
    });

    assert_eq!(
        results.0 as i32 + results.1 as i32,
        1,
        "exactly one concurrent approval must commit"
    );
    assert_eq!(
        appointments::list_active_appointments(&mut client)
            .unwrap()
            .len(),
        1
    );
}
