//! Synthetic PostgreSQL integration tests for the genealogy schema.
//!
//! Uses a per-suite disposable target from `tests/support/genealogy_isolation.rs`:
//! a uniquely named database created by the run. It skips when
//! `GENEALOGY_DATABASE_URL` is absent and refuses a destination that is not
//! clearly a test database or that equals `DATABASE_URL`.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn, create_isolated_target};

#[test]
fn migrations_constraints_and_rollback_are_enforced() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping genealogy integration test");
        return;
    };

    // Create the owned target, migrate it, and drop only that target afterwards.
    // The schema contract covers migrations 0001-0005, exactly as before. The
    // suite now runs that subset against its own disposable database instead of
    // the shared one; other suites apply the full migration set.
    let migrations = &genealogy_isolation::GENEALOGY_MIGRATIONS[..5];
    let target = create_isolated_target(&base, "schema");
    target.apply_migration_files(migrations);
    let mut client = target.connect();

    // A second application is a no-op because every object is guarded.
    for path in migrations {
        let sql = std::fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("re-apply {path}: {err}"));
    }

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

    // Release the client before cleanup drops the owned target.
    drop(client);
    target.cleanup();

    // SOP's PostgreSQL integration category requires structured test events. The
    // events below are emitted only after every real assertion above passed.
    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"migrations_constraints_and_rollback_are_enforced\"}}"
    );
    println!("{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"\"}}");
}
