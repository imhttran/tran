//! Synthetic PostgreSQL integration test for transactional username
//! reservation.
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
use rag_foundation::onboarding::{claim, release, reserve, NameParts, ReservationStatus};

const MIGRATIONS: [&str; 8] = [
    "genealogy/migrations/0001_genealogy_schema_version.sql",
    "genealogy/migrations/0002_catalog_keys.sql",
    "genealogy/migrations/0003_assertions_evidence_and_links.sql",
    "genealogy/migrations/0004_review_workflow.sql",
    "genealogy/migrations/0005_source_kind.sql",
    "genealogy/migrations/0006_head_of_family.sql",
    "genealogy/migrations/0007_onboarding.sql",
    "genealogy/migrations/0008_onboarding_identity.sql",
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
    format!("synthetic-reservation-{}-{nanos}", std::process::id())
}

fn cleanup(client: &mut Client, prefix: &str) {
    let pattern = format!("{prefix}%");
    // Reservations cascade from person, but delete them explicitly so cleanup
    // does not depend on cascade behavior.
    let _ = client.execute(
        "DELETE FROM onboarding_username_reservation
          WHERE person_id IN (SELECT person_id FROM person WHERE person_key LIKE $1)",
        &[&pattern],
    );
    let _ = client.execute("DELETE FROM person WHERE person_key LIKE $1", &[&pattern]);
}

fn name_parts() -> NameParts {
    NameParts {
        family: Some("Nguyen".to_string()),
        middle: Some("Van".to_string()),
        given: Some("An".to_string()),
    }
}

fn insert_person(client: &mut Client, person_key: &str) -> Result<i64, String> {
    let row = client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, 'Synthetic Reservation')
             RETURNING person_id",
            &[&person_key],
        )
        .map_err(|err| format!("insert person {person_key}: {err}"))?;
    Ok(row.get(0))
}

fn count_reservations(client: &mut Client, person_id: i64) -> Result<i64, String> {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_username_reservation WHERE person_id = $1",
            &[&person_id],
        )
        .map_err(|err| format!("count reservations: {err}"))
        .map(|row| row.get(0))
}

#[test]
fn reservation_allocates_unique_keys_idempotently_and_reuses_released_keys() {
    let Some(dsn) = genealogy_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping onboarding reservation test");
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
        "{{\"Action\":\"pass\",\"Package\":\"onboarding\",\"Test\":\"reservation_allocates_unique_keys_idempotently_and_reuses_released_keys\"}}"
    );
    println!("{{\"Action\":\"pass\",\"Package\":\"onboarding\",\"Test\":\"\"}}");
}

fn run_scenario(client: &mut Client, prefix: &str) -> Result<(), String> {
    let parts = name_parts();

    let first_key = format!("{prefix}-person-1");
    let first_id = insert_person(client, &first_key)?;
    let first = reserve(client, first_id, &parts, "synthetic-actor")
        .map_err(|err| format!("reserve first: {err}"))?;
    if first.username != "NVAn" {
        return Err(format!(
            "first username: expected NVAn, got {}",
            first.username
        ));
    }
    if first.username_key != "nvan" {
        return Err(format!(
            "first username_key: expected nvan, got {}",
            first.username_key
        ));
    }
    if first.status != ReservationStatus::Reserved {
        return Err(format!(
            "first status: expected reserved, got {:?}",
            first.status
        ));
    }

    let second_key = format!("{prefix}-person-2");
    let second_id = insert_person(client, &second_key)?;
    let second = reserve(client, second_id, &parts, "synthetic-actor")
        .map_err(|err| format!("reserve second: {err}"))?;
    if second.username != "NVAn2" {
        return Err(format!(
            "second username: expected NVAn2, got {}",
            second.username
        ));
    }
    if second.username_key != "nvan2" {
        return Err(format!(
            "second username_key: expected nvan2, got {}",
            second.username_key
        ));
    }

    // Repeating the same reservation for the same person is idempotent.
    let first_count = count_reservations(client, first_id)?;
    let repeated = reserve(client, first_id, &parts, "synthetic-actor")
        .map_err(|err| format!("reserve repeated: {err}"))?;
    if repeated.reservation_id != first.reservation_id {
        return Err(format!(
            "repeat reservation_id: expected {}, got {}",
            first.reservation_id, repeated.reservation_id
        ));
    }
    let after_count = count_reservations(client, first_id)?;
    if after_count != first_count {
        return Err(format!(
            "repeat added a row: before {first_count}, after {after_count}"
        ));
    }

    // Claim transitions reserved -> claimed.
    claim(client, second.reservation_id).map_err(|err| format!("claim second: {err}"))?;
    let claimed_status: String = client
        .query_one(
            "SELECT status FROM onboarding_username_reservation WHERE reservation_id = $1",
            &[&second.reservation_id],
        )
        .map_err(|err| format!("read claimed status: {err}"))?
        .get(0);
    if claimed_status != ReservationStatus::Claimed.as_str() {
        return Err(format!(
            "claimed status: expected claimed, got {claimed_status}"
        ));
    }

    // Release the first reservation; the base key becomes reusable.
    release(client, first.reservation_id).map_err(|err| format!("release first: {err}"))?;
    let released_at_set: bool = client
        .query_one(
            "SELECT released_at IS NOT NULL FROM onboarding_username_reservation
              WHERE reservation_id = $1",
            &[&first.reservation_id],
        )
        .map_err(|err| format!("read released_at: {err}"))?
        .get(0);
    if !released_at_set {
        return Err("release did not set released_at".to_string());
    }

    let third_key = format!("{prefix}-person-3");
    let third_id = insert_person(client, &third_key)?;
    let third = reserve(client, third_id, &parts, "synthetic-actor")
        .map_err(|err| format!("reserve third: {err}"))?;
    if third.username_key != "nvan" {
        return Err(format!(
            "third username_key: expected nvan, got {}",
            third.username_key
        ));
    }

    Ok(())
}
