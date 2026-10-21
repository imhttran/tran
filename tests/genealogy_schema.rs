//! Synthetic PostgreSQL integration tests for the genealogy schema.
//!
//! These tests write only to the explicitly configured disposable genealogy
//! database. They skip when `GENEALOGY_DATABASE_URL` is absent and refuse a
//! destination that is not clearly a test database or that equals `DATABASE_URL`.

use std::fs;
use std::str::FromStr;

use postgres::{Client, NoTls};

const MIGRATIONS: [&str; 5] = [
    "genealogy/migrations/0001_genealogy_schema_version.sql",
    "genealogy/migrations/0002_catalog_keys.sql",
    "genealogy/migrations/0003_assertions_evidence_and_links.sql",
    "genealogy/migrations/0004_review_workflow.sql",
    "genealogy/migrations/0005_source_kind.sql",
];

fn genealogy_dsn() -> Option<String> {
    let dsn = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    if dsn.is_empty() {
        return None;
    }

    let config = postgres::Config::from_str(&dsn).expect("GENEALOGY_DATABASE_URL parses");
    let database = config.get_dbname().unwrap_or_default();
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable genealogy database {database:?}"
    );

    if let Ok(rag) = std::env::var("DATABASE_URL") {
        let rag_config = postgres::Config::from_str(&rag).expect("DATABASE_URL parses");
        assert_ne!(
            (config.get_dbname(), config.get_hosts(), config.get_ports()),
            (
                rag_config.get_dbname(),
                rag_config.get_hosts(),
                rag_config.get_ports()
            ),
            "the genealogy integration test must not target the RAG database"
        );
    }

    Some(dsn)
}

fn connect(dsn: &str) -> Client {
    Client::connect(dsn, NoTls).expect("connect to the disposable genealogy database")
}

fn reset_schema(client: &mut Client) {
    client
        .batch_execute(
            "DROP TABLE IF EXISTS
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
                genealogy_fact_proposal_event,
                genealogy_fact_proposal,
                person_name_variant,
                person,
                genealogy_schema_version
             CASCADE;",
        )
        .expect("drop prior genealogy tables in the disposable database");
}

fn apply_migrations(client: &mut Client) {
    for path in MIGRATIONS {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

#[test]
fn migrations_constraints_and_rollback_are_enforced() {
    let Some(dsn) = genealogy_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping genealogy integration test");
        return;
    };
    let mut client = connect(&dsn);

    reset_schema(&mut client);
    apply_migrations(&mut client);
    // A second application is a no-op because every object is guarded.
    apply_migrations(&mut client);

    client
        .batch_execute(
            "INSERT INTO person (person_key, display_name) VALUES ('synthetic-a', 'Person A');",
        )
        .unwrap();

    let duplicate = client
        .batch_execute(
            "INSERT INTO person (person_key, display_name) VALUES ('synthetic-a', 'Person B');",
        )
        .unwrap_err();
    assert_eq!(
        duplicate.as_db_error().and_then(|err| err.constraint()),
        Some("person_person_key_key"),
        "duplicate catalog keys must be rejected: {duplicate}"
    );

    let invalid_status = client
        .batch_execute(
            "INSERT INTO assertion (person_id, assertion_kind, assertion_key, review_status)
             SELECT person_id, 'date_of_birth', 'synthetic', 'bogus' FROM person LIMIT 1;",
        )
        .unwrap_err();
    assert_eq!(
        invalid_status
            .as_db_error()
            .and_then(|err| err.constraint()),
        Some("assertion_review_status_check"),
        "invalid review statuses must be rejected: {invalid_status}"
    );

    client.batch_execute("BEGIN").unwrap();
    client
        .batch_execute("SET CONSTRAINTS ALL IMMEDIATE")
        .unwrap();
    let missing_evidence = client
        .batch_execute(
            "INSERT INTO assertion
                (person_id, assertion_kind, assertion_key, review_status, reviewed_by, reviewed_at)
             SELECT person_id, 'date_of_birth', 'synthetic', 'verified', 'synthetic-reviewer', now()
             FROM person LIMIT 1;",
        )
        .unwrap_err();
    assert!(
        missing_evidence
            .as_db_error()
            .is_some_and(|err| err.message().contains("requires at least one evidence row")),
        "verified assertions without evidence must be rejected: {missing_evidence}"
    );
    client.batch_execute("ROLLBACK").unwrap();

    client.batch_execute("BEGIN").unwrap();
    client
        .batch_execute(
            "INSERT INTO assertion
                (person_id, assertion_kind, assertion_key, review_status, reviewed_by, reviewed_at)
             SELECT person_id, 'date_of_birth', 'synthetic', 'verified', 'synthetic-reviewer', now()
             FROM person LIMIT 1;
             INSERT INTO assertion_evidence (assertion_id, document_ref, page)
             SELECT assertion_id, 'synthetic-source.md', 1 FROM assertion LIMIT 1;",
        )
        .unwrap();
    client.batch_execute("COMMIT").unwrap();

    let self_parent = client
        .batch_execute(
            "INSERT INTO parent_child_link (parent_id, child_id)
             SELECT person_id, person_id FROM person LIMIT 1;",
        )
        .unwrap_err();
    assert_eq!(
        self_parent.as_db_error().and_then(|err| err.constraint()),
        Some("parent_child_link_no_self"),
        "self-parenting must be rejected: {self_parent}"
    );

    let before: i64 = client
        .query_one("SELECT count(*) FROM person", &[])
        .unwrap()
        .get(0);
    client.batch_execute("BEGIN").unwrap();
    client
        .batch_execute(
            "INSERT INTO person (person_key, display_name) VALUES ('synthetic-rollback', 'Rollback');",
        )
        .unwrap();
    client.batch_execute("ROLLBACK").unwrap();
    let after: i64 = client
        .query_one("SELECT count(*) FROM person", &[])
        .unwrap()
        .get(0);
    assert_eq!(
        before, after,
        "a rolled-back import must leave no partial family graph"
    );

    reset_schema(&mut client);

    // SOP's PostgreSQL integration category requires structured test events. The
    // events below are emitted only after every real assertion above passed.
    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"migrations_constraints_and_rollback_are_enforced\"}}"
    );
    println!("{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"\"}}");
}
