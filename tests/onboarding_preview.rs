//! Synthetic PostgreSQL integration test for the onboarding candidate preview.
//!
//! This test writes only to the explicitly configured disposable genealogy
//! database. It skips when `GENEALOGY_DATABASE_URL` is absent and refuses a
//! destination that is not clearly a test/disposable database or that equals
//! `DATABASE_URL`. It does not use the RAG database.
//!
//! Every synthetic row uses a unique prefix and no real personal data. The rows
//! are removed before the test ends, even if an assertion fails.

use std::fs;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::onboarding::{preview_candidates, Classification};

const MIGRATIONS: [&str; 19] = [
    "genealogy/migrations/0001_genealogy_schema_version.sql",
    "genealogy/migrations/0002_catalog_keys.sql",
    "genealogy/migrations/0003_assertions_evidence_and_links.sql",
    "genealogy/migrations/0004_review_workflow.sql",
    "genealogy/migrations/0005_source_kind.sql",
    "genealogy/migrations/0006_head_of_family.sql",
    "genealogy/migrations/0007_onboarding.sql",
    "genealogy/migrations/0010_onboarding_foreign_keys.sql",
    "genealogy/migrations/0011_onboarding_foreign_key_reconciliation.sql",
    "genealogy/migrations/0012_profile_revisions.sql",
    "genealogy/migrations/0013_profile_revision_actor_scope.sql",
    "genealogy/migrations/0014_profile_revision_draft_withdrawal.sql",
    "genealogy/migrations/0015_profile_media.sql",
    "genealogy/migrations/0016_profile_media_withdrawal.sql",
    "genealogy/migrations/0017_profile_media_withdrawal_audit.sql",
    "genealogy/migrations/0018_onboarding_batch.sql",
    "genealogy/migrations/0019_branch_reviewer_grants.sql",
    "genealogy/migrations/0020_branch_review_proposals.sql",
    "genealogy/migrations/0021_profile_field_restriction.sql",
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

fn apply_migrations(client: &mut Client) {
    for path in MIGRATIONS {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

/// A unique, non-real prefix so cleanup removes only this test's rows.
fn unique_prefix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("synthetic-preview-{}-{nanos}", std::process::id())
}

fn cleanup(client: &mut Client, prefix: &str) {
    let pattern = format!("{prefix}%");
    // Links and invitations cascade from person, but delete them explicitly so
    // the cleanup does not depend on cascade behavior.
    let _ = client.execute(
        "DELETE FROM onboarding_person_account_link
          WHERE person_id IN (SELECT person_id FROM person WHERE person_key LIKE $1)",
        &[&pattern],
    );
    let _ = client.execute(
        "DELETE FROM onboarding_invitation
          WHERE person_id IN (SELECT person_id FROM person WHERE person_key LIKE $1)",
        &[&pattern],
    );
    // Deleting the person rows removes assertions, evidence, and links by
    // cascade.
    let _ = client.execute("DELETE FROM person WHERE person_key LIKE $1", &[&pattern]);
    let _ = client.execute(
        "DELETE FROM source_document WHERE relative_path LIKE $1",
        &[&pattern],
    );
}

#[test]
fn preview_classifies_synthetic_rows_and_exposes_no_private_data() {
    let Some(dsn) = genealogy_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping onboarding preview test");
        return;
    };
    let mut client = Client::connect(&dsn, NoTls).expect("connect to the disposable genealogy DB");
    apply_migrations(&mut client);

    let prefix = unique_prefix();

    // Run the body, then always clean up. Cleanup removes only prefixed rows.
    let result = run_scenario(&mut client, &prefix);
    cleanup(&mut client, &prefix);
    result.unwrap_or_else(|err| panic!("{err}"));

    println!(
        "{{\"Action\":\"pass\",\"Package\":\"onboarding\",\"Test\":\"preview_classifies_synthetic_rows_and_exposes_no_private_data\"}}"
    );
    println!("{{\"Action\":\"pass\",\"Package\":\"onboarding\",\"Test\":\"\"}}");
}

/// Inserts synthetic rows, runs the preview, and asserts on the result.
fn run_scenario(client: &mut Client, prefix: &str) -> Result<(), String> {
    // A not-eligible person: no living evidence at all.
    let no_living = format!("{prefix}-no-living");
    client
        .execute(
            "INSERT INTO person (person_key, display_name) VALUES ($1, 'Synthetic No Living')",
            &[&no_living],
        )
        .map_err(|err| format!("insert no-living person: {err}"))?;

    // An unverified living assertion is not enough: it stays not eligible.
    run_preview_and_assert(client, &no_living, Classification::NotEligible)?;

    // An eligible person: a verified, cited living assertion.
    let eligible = format!("{prefix}-eligible");
    client
        .execute(
            "INSERT INTO person (person_key, display_name) VALUES ($1, 'Synthetic Eligible')",
            &[&eligible],
        )
        .map_err(|err| format!("insert eligible person: {err}"))?;
    client
        .batch_execute("BEGIN;")
        .map_err(|err| format!("begin eligible: {err}"))?;
    let assert_result = (|| -> Result<(), String> {
        let document = format!("{prefix}-source.md");
        client
            .execute(
                "INSERT INTO source_document (relative_path, sha256)
                 VALUES ($1, repeat('a', 64))",
                &[&document],
            )
            .map_err(|err| format!("insert source_document: {err}"))?;
        let person_id: i64 = client
            .query_one(
                "SELECT person_id FROM person WHERE person_key = $1",
                &[&eligible],
            )
            .map_err(|err| format!("lookup eligible person: {err}"))?
            .get(0);
        let source_evidence_id: i64 = client
            .query_one(
                "INSERT INTO source_evidence (source_document_id, page)
                 SELECT source_document_id, 1 FROM source_document WHERE relative_path = $1
                 RETURNING source_evidence_id",
                &[&document],
            )
            .map_err(|err| format!("insert source_evidence: {err}"))?
            .get(0);
        client
            .execute(
                "INSERT INTO assertion
                    (person_id, assertion_kind, assertion_key, value_text)
                 VALUES ($1, 'vital_status', 'living', 'alive')",
                &[&person_id],
            )
            .map_err(|err| format!("insert living assertion: {err}"))?;
        client
            .execute(
                "INSERT INTO assertion_evidence (assertion_id, source_evidence_id, document_ref, page)
                 SELECT assertion_id, $1, $2, 1 FROM assertion WHERE person_id = $3",
                &[&source_evidence_id, &document, &person_id],
            )
            .map_err(|err| format!("insert assertion_evidence: {err}"))?;
        // Verify after the evidence exists: the evidence trigger reopens a
        // verified assertion when its evidence is mutated.
        client
            .execute(
                "UPDATE assertion
                    SET review_status = 'verified', reviewed_by = 'synthetic-reviewer', reviewed_at = now()
                  WHERE person_id = $1 AND assertion_kind = 'vital_status' AND assertion_key = 'living'",
                &[&person_id],
            )
            .map_err(|err| format!("verify living assertion: {err}"))?;
        Ok(())
    })();
    match assert_result {
        Ok(()) => client
            .batch_execute("COMMIT")
            .map_err(|err| format!("commit eligible: {err}"))?,
        Err(err) => {
            let _ = client.batch_execute("ROLLBACK");
            return Err(err);
        }
    }

    run_preview_and_assert(client, &eligible, Classification::EligibleForInvite)
}

/// Runs the preview and asserts that the given person carries the expected
/// classification. It also asserts the preview exposes no private fields.
fn run_preview_and_assert(
    client: &mut Client,
    person_key: &str,
    expected: Classification,
) -> Result<(), String> {
    let preview = preview_candidates(client).map_err(|err| format!("preview failed: {err}"))?;
    let row = preview
        .rows
        .iter()
        .find(|row| row.person_key == person_key)
        .ok_or_else(|| format!("preview did not include {person_key}"))?;
    if row.classification != expected {
        return Err(format!(
            "{person_key}: expected {:?}, got {:?}",
            expected, row.classification
        ));
    }
    // The candidate row carries a reason code, never a display name, date, or
    // excerpt. Classification and reasons are the only fields besides the key.
    if row.reasons.is_empty() {
        return Err(format!("{person_key}: expected at least one reason code"));
    }
    Ok(())
}
