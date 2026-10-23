//! Migration 0011 regression tests: fresh schema, legacy schema, and orphans.
//!
//! Each scenario runs in its own disposable target created from
//! `GENEALOGY_DATABASE_URL` through `tests/support/genealogy_isolation.rs`.
//! The test never targets the configured genealogy database itself, never
//! targets RAG data, and drops only the target it created. Orphan validation is
//! asserted without deleting the orphan row.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use std::fs;

use genealogy_isolation::{base_dsn, create_isolated_target, IsolatedTarget};
use postgres::Client;

const LINK_TABLES: [&str; 4] = [
    "onboarding_invitation",
    "onboarding_username_reservation",
    "onboarding_person_account_link",
    "onboarding_link_audit",
];

const ALL_MIGRATIONS: [&str; 11] = [
    "genealogy/migrations/0001_genealogy_schema_version.sql",
    "genealogy/migrations/0002_catalog_keys.sql",
    "genealogy/migrations/0003_assertions_evidence_and_links.sql",
    "genealogy/migrations/0004_review_workflow.sql",
    "genealogy/migrations/0005_source_kind.sql",
    "genealogy/migrations/0006_head_of_family.sql",
    "genealogy/migrations/0007_onboarding.sql",
    "genealogy/migrations/0008_onboarding_identity.sql",
    "genealogy/migrations/0009_onboarding_link_audit.sql",
    "genealogy/migrations/0010_onboarding_foreign_keys.sql",
    "genealogy/migrations/0011_onboarding_foreign_key_reconciliation.sql",
];

/// Migrations available before the 0010/0011 reconciliation.
const PRE_RECONCILIATION: [&str; 9] = [
    "genealogy/migrations/0001_genealogy_schema_version.sql",
    "genealogy/migrations/0002_catalog_keys.sql",
    "genealogy/migrations/0003_assertions_evidence_and_links.sql",
    "genealogy/migrations/0004_review_workflow.sql",
    "genealogy/migrations/0005_source_kind.sql",
    "genealogy/migrations/0006_head_of_family.sql",
    "genealogy/migrations/0007_onboarding.sql",
    "genealogy/migrations/0008_onboarding_identity.sql",
    "genealogy/migrations/0009_onboarding_link_audit.sql",
];

#[derive(Debug)]
struct ForeignKey {
    table: String,
    name: String,
    validated: bool,
    definition: String,
    delete_action: String,
}

fn apply(client: &mut Client, paths: &[&str]) {
    for path in paths {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

fn onboarding_foreign_keys(client: &mut Client) -> Vec<ForeignKey> {
    let rows = client
        .query(
            "SELECT conrelid::regclass::text, conname, convalidated, \
                    pg_get_constraintdef(oid), confdeltype::text \
               FROM pg_constraint \
              WHERE contype = 'f' \
                AND conrelid IN ('onboarding_invitation'::regclass, \
                                 'onboarding_username_reservation'::regclass, \
                                 'onboarding_person_account_link'::regclass, \
                                 'onboarding_link_audit'::regclass) \
              ORDER BY conrelid::regclass::text, conname",
            &[],
        )
        .expect("read onboarding foreign keys");
    rows.iter()
        .map(|row| ForeignKey {
            table: row.get(0),
            name: row.get(1),
            validated: row.get(2),
            definition: row.get(3),
            delete_action: row.get(4),
        })
        .collect()
}

fn assert_cascade_definition(key: &ForeignKey) {
    assert!(
        key.definition.contains("REFERENCES person(person_id)"),
        "{} must reference person(person_id): {}",
        key.name,
        key.definition
    );
    assert!(
        key.definition.contains("ON DELETE CASCADE"),
        "{} must cascade on delete: {}",
        key.name,
        key.definition
    );
    assert_eq!(
        key.delete_action, "c",
        "{} must use the cascade delete action",
        key.name
    );
}

fn fks_for<'a>(keys: &'a [ForeignKey], table: &str) -> Vec<&'a ForeignKey> {
    keys.iter().filter(|key| key.table == table).collect()
}

fn assert_single_validated_cascade(keys: &[ForeignKey]) {
    for table in LINK_TABLES {
        let entries = fks_for(keys, table);
        assert_eq!(
            entries.len(),
            1,
            "{table} must have exactly one person foreign key, got {entries:?}"
        );
        assert!(
            entries[0].validated,
            "{} must be validated",
            entries[0].name
        );
        assert_cascade_definition(entries[0]);
    }
}

fn prepare(label: &str) -> Option<IsolatedTarget> {
    let base = base_dsn()?;
    Some(create_isolated_target(&base, label))
}

#[test]
fn fresh_schema_0010_duplicates_are_reconciled_by_0011() {
    let Some(target) = prepare("migration_0011_fresh") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping migration 0011 fresh test");
        return;
    };
    let mut client = target.connect();

    apply(&mut client, &ALL_MIGRATIONS[..10]);
    let before = onboarding_foreign_keys(&mut client);
    for table in LINK_TABLES {
        let entries = fks_for(&before, table);
        assert_eq!(
            entries.len(),
            2,
            "{table} must reproduce the 0010 duplicate on a fresh schema: {entries:?}"
        );
        let validated = entries.iter().filter(|key| key.validated).count();
        assert_eq!(validated, 1, "{table} must have one validated inline FK");
        let unvalidated = entries.iter().filter(|key| !key.validated).count();
        assert_eq!(unvalidated, 1, "{table} must have one NOT VALID 0010 FK");
        for key in &entries {
            assert_cascade_definition(key);
        }
    }

    apply(&mut client, &ALL_MIGRATIONS[10..]);
    let after = onboarding_foreign_keys(&mut client);
    assert_single_validated_cascade(&after);
    let unvalidated: i64 = client
        .query_one(
            "SELECT count(*) FROM pg_constraint \
              WHERE connamespace = 'public'::regnamespace AND NOT convalidated",
            &[],
        )
        .expect("count unvalidated constraints")
        .get(0);
    assert_eq!(unvalidated, 0, "0011 must leave no unvalidated constraint");

    drop(client);
    target.cleanup();
}

#[test]
fn legacy_schema_0010_constraints_are_validated_by_0011() {
    let Some(target) = prepare("migration_0011_legacy") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping migration 0011 legacy test");
        return;
    };
    let mut client = target.connect();

    apply(&mut client, &PRE_RECONCILIATION);
    // Simulate the earlier schema that created the tables without the inline
    // person foreign keys.
    for table in LINK_TABLES {
        let name = format!("{table}_person_id_fkey");
        client
            .batch_execute(&format!(
                "ALTER TABLE {table} DROP CONSTRAINT IF EXISTS {name};"
            ))
            .expect("drop inline legacy foreign key");
    }

    apply(&mut client, &ALL_MIGRATIONS[9..10]);
    let before = onboarding_foreign_keys(&mut client);
    for table in LINK_TABLES {
        let entries = fks_for(&before, table);
        assert_eq!(entries.len(), 1, "{table} must gain one 0010 FK");
        assert!(!entries[0].validated, "the 0010 FK must start NOT VALID");
        assert_cascade_definition(entries[0]);
    }

    apply(&mut client, &ALL_MIGRATIONS[10..]);
    let after = onboarding_foreign_keys(&mut client);
    assert_single_validated_cascade(&after);

    drop(client);
    target.cleanup();
}

#[test]
fn legacy_orphan_rows_fail_validation_without_being_deleted() {
    let Some(target) = prepare("migration_0011_orphan") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping migration 0011 orphan test");
        return;
    };
    let mut client = target.connect();

    apply(&mut client, &PRE_RECONCILIATION);
    client
        .batch_execute(
            "ALTER TABLE onboarding_invitation \
             DROP CONSTRAINT IF EXISTS onboarding_invitation_person_id_fkey;",
        )
        .expect("drop inline legacy invitation FK");
    client
        .execute(
            "INSERT INTO onboarding_invitation (person_id, created_by) \
             VALUES (999999, 'synthetic-orphan-test')",
            &[],
        )
        .expect("insert synthetic orphan row");

    apply(&mut client, &ALL_MIGRATIONS[9..10]);
    let error = client
        .batch_execute(&fs::read_to_string(ALL_MIGRATIONS[10]).expect("read 0011"))
        .expect_err("0011 must fail when a legacy orphan exists");
    let db_error = error.as_db_error().expect("PostgreSQL validation error");
    assert!(
        db_error
            .message()
            .contains("violates foreign key constraint"),
        "unexpected validation error: {}",
        db_error.message()
    );

    // Reconnect because the validation error rolled back the DO statement.
    drop(client);
    let mut client = target.connect();
    let orphan_count: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_invitation WHERE person_id = 999999",
            &[],
        )
        .expect("count synthetic orphan")
        .get(0);
    assert_eq!(orphan_count, 1, "validation must not delete the orphan row");
    let validated: bool = client
        .query_one(
            "SELECT convalidated FROM pg_constraint \
              WHERE conname = 'onboarding_invitation_person_fk'",
            &[],
        )
        .expect("read invitation constraint")
        .get(0);
    assert!(!validated, "the failed constraint must remain NOT VALID");

    drop(client);
    target.cleanup();
}
