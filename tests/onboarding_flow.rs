//! Synthetic end-to-end closure test for USR-003.
//!
//! Runs the complete flow against two disposable test databases:
//! living-candidate preview -> transactional username reservation -> hashed
//! single-use invitation -> identity resolution -> reviewed account link ->
//! audit history, then re-runs the idempotent steps.
//!
//! The test skips when either `DATABASE_URL` or `GENEALOGY_DATABASE_URL` is
//! absent and refuses a genealogy destination that is not clearly disposable or
//! that equals the identity database. Every row is synthetic and prefixed; both
//! databases are cleaned in an always-run path.

use std::fs;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::onboarding::{
    accept, create as create_invitation, link_for_person, link_reviewed, preview_candidates,
    reserve, resolve_account, Classification, LinkDecision, LinkError, NameParts,
};

const IDENTITY_MIGRATIONS: [&str; 3] = [
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
];

const GENEALOGY_MIGRATIONS: [&str; 12] = [
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
    "genealogy/migrations/0012_profile_revisions.sql",
];

fn identity_dsn() -> Option<String> {
    let dsn = std::env::var("DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    (!dsn.is_empty()).then_some(dsn)
}

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
    if let Ok(identity) = std::env::var("DATABASE_URL") {
        let identity_config = postgres::Config::from_str(&identity).expect("DATABASE_URL parses");
        assert_ne!(
            (config.get_dbname(), config.get_hosts(), config.get_ports()),
            (
                identity_config.get_dbname(),
                identity_config.get_hosts(),
                identity_config.get_ports()
            ),
            "the genealogy integration test must not target the identity database"
        );
    }
    Some(dsn)
}

fn apply_migrations(client: &mut Client, paths: &[&str]) {
    for path in paths {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

fn unique_prefix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("synthetic-flow-{}-{nanos}", std::process::id())
}

#[test]
fn onboarding_flow_is_complete_and_idempotent() {
    let (Some(identity_dsn), Some(genealogy_dsn)) = (identity_dsn(), genealogy_dsn()) else {
        eprintln!("DATABASE_URL or GENEALOGY_DATABASE_URL unset; skipping onboarding flow");
        return;
    };

    let mut identity =
        Client::connect(&identity_dsn, NoTls).expect("connect to the identity database");
    let mut genealogy =
        Client::connect(&genealogy_dsn, NoTls).expect("connect to the disposable genealogy DB");
    apply_migrations(&mut identity, &IDENTITY_MIGRATIONS);
    apply_migrations(&mut genealogy, &GENEALOGY_MIGRATIONS);
    clear_orphans(&mut genealogy);

    let prefix = unique_prefix();
    let result = run_flow(&mut identity, &mut genealogy, &prefix);
    cleanup(&mut genealogy, &prefix);
    cleanup_identity(&mut identity, &prefix);
    result.unwrap_or_else(|err| panic!("{err}"));
}

fn run_flow(identity: &mut Client, genealogy: &mut Client, prefix: &str) -> Result<(), String> {
    let person_key = format!("{prefix}-person");
    let person_id = insert_verified_living_person(genealogy, &person_key)?;

    // 1. The candidate is eligible from verified, cited living evidence.
    let preview = preview_candidates(genealogy).map_err(|err| format!("preview: {err}"))?;
    let row = preview
        .rows
        .iter()
        .find(|row| row.person_key == person_key)
        .ok_or_else(|| "preview did not include the synthetic person".to_string())?;
    if row.classification != Classification::EligibleForInvite {
        return Err(format!(
            "expected EligibleForInvite, got {:?}",
            row.classification
        ));
    }

    // 2. Transactional username reservation.
    let parts = NameParts {
        family: Some("Nguyen".to_string()),
        middle: Some("Van".to_string()),
        given: Some("An".to_string()),
    };
    let first = reserve(genealogy, person_id, &parts, "synthetic-admin")
        .map_err(|err| format!("reserve: {err}"))?;
    if first.username != "NVAn" || first.username_key != "nvan" {
        return Err(format!(
            "unexpected reservation {:?}/{:?}",
            first.username, first.username_key
        ));
    }

    // 3. Hashed invitation; a second create is idempotent while pending.
    let issued = create_invitation(
        genealogy,
        person_id,
        Duration::from_secs(3600),
        "synthetic-admin",
    )
    .map_err(|err| format!("create invitation: {err}"))?;
    let token = issued
        .token
        .clone()
        .ok_or_else(|| "first create returned no token".to_string())?;
    let repeated = create_invitation(
        genealogy,
        person_id,
        Duration::from_secs(3600),
        "synthetic-admin",
    )
    .map_err(|err| format!("repeat invitation: {err}"))?;
    if repeated.invitation_id != issued.invitation_id || repeated.token.is_some() {
        return Err("pending invitation was not idempotent".to_string());
    }
    let stored_hash: String = genealogy
        .query_one(
            "SELECT token_hash FROM onboarding_invitation WHERE invitation_id = $1",
            &[&issued.invitation_id],
        )
        .map_err(|err| format!("read token hash: {err}"))?
        .get(0);
    if stored_hash == token || stored_hash.len() != 64 {
        return Err("invitation token was not stored as a hash".to_string());
    }
    let count: i64 = genealogy
        .query_one(
            "SELECT count(*) FROM onboarding_invitation WHERE person_id = $1",
            &[&person_id],
        )
        .map_err(|err| format!("count invitations: {err}"))?
        .get(0);
    if count != 1 {
        return Err(format!("expected one invitation, got {count}"));
    }

    // 4. The member accepts the token once.
    let accepted_id = accept(genealogy, &token).map_err(|err| format!("accept: {err}"))?;
    if accepted_id != issued.invitation_id {
        return Err("accept returned a different invitation".to_string());
    }
    if accept(genealogy, &token).is_ok() {
        return Err("token was accepted twice".to_string());
    }

    // 5. Reviewed identity link.
    let subject = format!("{prefix}-subject");
    let issuer = "synthetic-flow-issuer";
    identity
        .execute(
            "INSERT INTO users (subject, issuer) VALUES ($1, $2)",
            &[&subject, &issuer],
        )
        .map_err(|err| format!("insert identity: {err}"))?;
    let account = resolve_account(identity, issuer, &subject)
        .map_err(|err| format!("resolve identity: {err}"))?;
    let linked = link_reviewed(
        genealogy,
        person_id,
        &account,
        LinkDecision::Approve,
        "synthetic-reviewer",
    )
    .map_err(|err| format!("link: {err}"))?
    .ok_or_else(|| "approval returned no link".to_string())?;
    if link_for_person(genealogy, person_id)
        .map_err(|err| format!("read link: {err}"))?
        .is_none()
    {
        return Err("link was not stored".to_string());
    }

    // 6. Re-running the idempotent steps creates no duplicate rows and does not
    // silently replace the link.
    let repeated_reservation = reserve(genealogy, person_id, &parts, "synthetic-admin")
        .map_err(|err| format!("reserve again: {err}"))?;
    if repeated_reservation.reservation_id != first.reservation_id {
        return Err("reservation was duplicated".to_string());
    }
    if link_reviewed(
        genealogy,
        person_id,
        &account,
        LinkDecision::Approve,
        "synthetic-reviewer",
    ) != Err(LinkError::AlreadyLinked)
    {
        return Err("second link was not rejected".to_string());
    }
    let linked_audit: i64 = genealogy
        .query_one(
            "SELECT count(*) FROM onboarding_link_audit WHERE person_id = $1 AND decision = 'linked'",
            &[&person_id],
        )
        .map_err(|err| format!("count link audit: {err}"))?
        .get(0);
    if linked_audit != 1 {
        return Err(format!("expected one linked audit row, got {linked_audit}"));
    }

    let _ = linked;
    Ok(())
}

fn insert_verified_living_person(client: &mut Client, key: &str) -> Result<i64, String> {
    let person_id: i64 = client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, 'Synthetic Flow Person') RETURNING person_id",
            &[&key],
        )
        .map_err(|err| format!("insert person: {err}"))?
        .get(0);
    let document = format!("{key}-source.md");
    client
        .execute(
            "INSERT INTO source_document (relative_path, sha256, source_kind) VALUES ($1, repeat('b', 64), 'original')",
            &[&document],
        )
        .map_err(|err| format!("insert source document: {err}"))?;
    let source_evidence_id: i64 = client
        .query_one(
            "INSERT INTO source_evidence (source_document_id, page) \
             SELECT source_document_id, 1 FROM source_document WHERE relative_path = $1 \
             RETURNING source_evidence_id",
            &[&document],
        )
        .map_err(|err| format!("insert source evidence: {err}"))?
        .get(0);
    client
        .execute(
            "INSERT INTO assertion (person_id, assertion_kind, assertion_key, value_text) \
             VALUES ($1, 'vital_status', 'living', 'alive')",
            &[&person_id],
        )
        .map_err(|err| format!("insert assertion: {err}"))?;
    client
        .execute(
            "INSERT INTO assertion_evidence (assertion_id, source_evidence_id, document_ref, page) \
             SELECT assertion_id, $1, $2, 1 FROM assertion WHERE person_id = $3",
            &[&source_evidence_id, &document, &person_id],
        )
        .map_err(|err| format!("insert assertion evidence: {err}"))?;
    client
        .execute(
            "UPDATE assertion SET review_status = 'verified', reviewed_by = 'synthetic-reviewer', reviewed_at = now() \
             WHERE person_id = $1 AND assertion_kind = 'vital_status' AND assertion_key = 'living'",
            &[&person_id],
        )
        .map_err(|err| format!("verify assertion: {err}"))?;
    Ok(person_id)
}

fn cleanup(genealogy: &mut Client, prefix: &str) {
    let pattern = format!("{prefix}%");
    // Delete child rows explicitly so cleanup is correct even on a database
    // upgraded from a schema version whose foreign keys were missing.
    for statement in [
        "DELETE FROM onboarding_link_audit WHERE person_id IN \
         (SELECT person_id FROM person WHERE person_key LIKE $1)",
        "DELETE FROM onboarding_person_account_link WHERE person_id IN \
         (SELECT person_id FROM person WHERE person_key LIKE $1)",
        "DELETE FROM onboarding_invitation WHERE person_id IN \
         (SELECT person_id FROM person WHERE person_key LIKE $1)",
        "DELETE FROM onboarding_username_reservation WHERE person_id IN \
         (SELECT person_id FROM person WHERE person_key LIKE $1)",
    ] {
        let _ = genealogy.execute(statement, &[&pattern]);
    }
    let _ = genealogy.execute("DELETE FROM person WHERE person_key LIKE $1", &[&pattern]);
    let _ = genealogy.execute(
        "DELETE FROM source_document WHERE relative_path LIKE $1",
        &[&pattern],
    );
}

/// Removes orphaned link rows left by an earlier schema version without
/// foreign keys. Safe on the disposable test database only.
fn clear_orphans(genealogy: &mut Client) {
    let _ = genealogy.execute(
        "DELETE FROM onboarding_link_audit a \
          WHERE NOT EXISTS (SELECT 1 FROM person p WHERE p.person_id = a.person_id)",
        &[],
    );
    let _ = genealogy.execute(
        "DELETE FROM onboarding_person_account_link l \
          WHERE NOT EXISTS (SELECT 1 FROM person p WHERE p.person_id = l.person_id)",
        &[],
    );
    let _ = genealogy.execute(
        "DELETE FROM onboarding_invitation i \
          WHERE NOT EXISTS (SELECT 1 FROM person p WHERE p.person_id = i.person_id)",
        &[],
    );
    let _ = genealogy.execute(
        "DELETE FROM onboarding_username_reservation r \
          WHERE NOT EXISTS (SELECT 1 FROM person p WHERE p.person_id = r.person_id)",
        &[],
    );
}

fn cleanup_identity(identity: &mut Client, prefix: &str) {
    let pattern = format!("{prefix}%");
    let _ = identity.execute("DELETE FROM users WHERE subject LIKE $1", &[&pattern]);
}
