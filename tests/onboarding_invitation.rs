//! USR-003B integration test: random, expiring, single-use invitation tokens.
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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::onboarding::invitation::hash_token;
use rag_foundation::onboarding::{accept, create, expire_due, revoke, InvitationError};

const MIGRATIONS: [&str; 24] = [
    "genealogy/migrations/0001_genealogy_schema_version.sql",
    "genealogy/migrations/0002_catalog_keys.sql",
    "genealogy/migrations/0003_assertions_evidence_and_links.sql",
    "genealogy/migrations/0004_review_workflow.sql",
    "genealogy/migrations/0005_source_kind.sql",
    "genealogy/migrations/0006_head_of_family.sql",
    "genealogy/migrations/0007_onboarding.sql",
    "genealogy/migrations/0008_onboarding_identity.sql",
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
    "genealogy/migrations/0022_profile_revision_restriction_generation.sql",
    "genealogy/migrations/0023_document_extraction.sql",
    "genealogy/migrations/0024_document_evidence_provenance.sql",
    "genealogy/migrations/0025_assertion_dates.sql",
];

const ACTOR: &str = "synthetic-actor";

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
    format!("synthetic-invitation-{}-{nanos}", std::process::id())
}

fn cleanup(client: &mut Client, prefix: &str) {
    let pattern = format!("{prefix}%");
    // Invitations cascade from person, but delete them explicitly so the
    // cleanup does not depend on cascade behavior.
    let _ = client.execute(
        "DELETE FROM onboarding_invitation
          WHERE person_id IN (SELECT person_id FROM person WHERE person_key LIKE $1)",
        &[&pattern],
    );
    let _ = client.execute("DELETE FROM person WHERE person_key LIKE $1", &[&pattern]);
}

fn insert_person(client: &mut Client, person_key: &str) -> Result<i64, String> {
    client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, 'Synthetic Invitation')
             RETURNING person_id",
            &[&person_key],
        )
        .map(|row| row.get(0))
        .map_err(|err| format!("insert person {person_key}: {err}"))
}

fn count_invitations(client: &mut Client, person_id: i64) -> Result<i64, String> {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_invitation WHERE person_id = $1",
            &[&person_id],
        )
        .map(|row| row.get(0))
        .map_err(|err| format!("count invitations: {err}"))
}

#[test]
fn invitation_tokens_are_random_expiring_and_single_use() {
    let Some(dsn) = genealogy_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping onboarding invitation test");
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
        "{{\"Action\":\"pass\",\"Package\":\"onboarding\",\"Test\":\"invitation_tokens_are_random_expiring_and_single_use\"}}"
    );
    println!("{{\"Action\":\"pass\",\"Package\":\"onboarding\",\"Test\":\"\"}}");
}

fn run_scenario(client: &mut Client, prefix: &str) -> Result<(), String> {
    create_returns_token_once_and_stores_hash(client, prefix)?;
    second_create_is_idempotent(client, prefix)?;
    accept_succeeds_once_then_revoke_blocks(client, prefix)?;
    expired_pending_cannot_be_accepted(client, prefix)?;
    revoke_and_expire_reject_wrong_states(client, prefix)?;
    Ok(())
}

/// The plaintext token is returned once; only its SHA-256 hex is stored.
fn create_returns_token_once_and_stores_hash(
    client: &mut Client,
    prefix: &str,
) -> Result<(), String> {
    let person_id = insert_person(client, &format!("{prefix}-create"))?;
    let created = create(client, person_id, Duration::from_secs(3600), ACTOR)
        .map_err(|err| format!("create invitation: {err}"))?;
    if created.person_id != person_id {
        return Err(format!(
            "created person_id: expected {person_id}, got {}",
            created.person_id
        ));
    }
    let token = created
        .token
        .ok_or_else(|| "create did not return a plaintext token".to_string())?;
    if token.len() != 64 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("token is not 64 hex chars: {}", token.len()));
    }
    if token != token.to_lowercase() {
        return Err("token is not lowercase hex".to_string());
    }

    let row = client
        .query_one(
            "SELECT token_hash, invitation_token, status, used_at, expires_at > now()
               FROM onboarding_invitation WHERE invitation_id = $1",
            &[&created.invitation_id],
        )
        .map_err(|err| format!("read stored invitation: {err}"))?;
    let stored: Option<String> = row.get(0);
    let legacy: Option<String> = row.get(1);
    let status: String = row.get(2);
    let used_at_set: bool = row.get::<_, Option<SystemTime>>(3).is_some();
    let future_expiry: bool = row.get(4);
    let stored = stored.ok_or_else(|| "token_hash is null".to_string())?;
    if stored == token {
        return Err("stored hash equals the plaintext token".to_string());
    }
    if stored != hash_token(&token) {
        return Err("stored token_hash is not the SHA-256 hex of the token".to_string());
    }
    if legacy.is_some() {
        return Err("legacy invitation_token must stay null".to_string());
    }
    if status != "pending" {
        return Err(format!("status: expected pending, got {status}"));
    }
    if used_at_set {
        return Err("new invitation must have no used_at".to_string());
    }
    if !future_expiry {
        return Err("expires_at must be in the future".to_string());
    }
    Ok(())
}

/// A second create for the same person reuses the pending row.
fn second_create_is_idempotent(client: &mut Client, prefix: &str) -> Result<(), String> {
    let person_id = insert_person(client, &format!("{prefix}-idem"))?;
    let first = create(client, person_id, Duration::from_secs(3600), ACTOR)
        .map_err(|err| format!("first create: {err}"))?;
    if first.token.is_none() {
        return Err("first create did not return a token".to_string());
    }
    let second = create(client, person_id, Duration::from_secs(3600), ACTOR)
        .map_err(|err| format!("second create: {err}"))?;
    if second.invitation_id != first.invitation_id {
        return Err(format!(
            "second create invitation_id: expected {}, got {}",
            first.invitation_id, second.invitation_id
        ));
    }
    if second.token.is_some() {
        return Err("second create must not return a token".to_string());
    }
    let count = count_invitations(client, person_id)?;
    if count != 1 {
        return Err(format!("expected exactly 1 invitation row, got {count}"));
    }
    Ok(())
}

/// accept succeeds once; a second accept and an accept after revoke fail.
fn accept_succeeds_once_then_revoke_blocks(
    client: &mut Client,
    prefix: &str,
) -> Result<(), String> {
    let person_id = insert_person(client, &format!("{prefix}-accept"))?;
    let created = create(client, person_id, Duration::from_secs(3600), ACTOR)
        .map_err(|err| format!("create: {err}"))?;
    let token = created
        .token
        .ok_or_else(|| "create did not return a token".to_string())?;
    let accepted = accept(client, &token).map_err(|err| format!("first accept: {err}"))?;
    if accepted != created.invitation_id {
        return Err(format!(
            "accept invitation_id: expected {}, got {accepted}",
            created.invitation_id
        ));
    }
    if !accept(client, &token).is_err() {
        return Err("second accept must fail".to_string());
    }

    let revoked_person = insert_person(client, &format!("{prefix}-revoke"))?;
    let revoked = create(client, revoked_person, Duration::from_secs(3600), ACTOR)
        .map_err(|err| format!("create revoked: {err}"))?;
    let revoked_token = revoked
        .token
        .ok_or_else(|| "revoke create did not return a token".to_string())?;
    revoke(client, revoked.invitation_id).map_err(|err| format!("revoke pending: {err}"))?;
    if !accept(client, &revoked_token).is_err() {
        return Err("accept after revoke must fail".to_string());
    }
    Ok(())
}

/// An expired pending row cannot be accepted after `expire_due`.
fn expired_pending_cannot_be_accepted(client: &mut Client, prefix: &str) -> Result<(), String> {
    let person_id = insert_person(client, &format!("{prefix}-expire"))?;
    let created = create(client, person_id, Duration::from_secs(3600), ACTOR)
        .map_err(|err| format!("create: {err}"))?;
    let token = created
        .token
        .ok_or_else(|| "create did not return a token".to_string())?;
    client
        .execute(
            "UPDATE onboarding_invitation SET expires_at = now() - interval '1 hour'
              WHERE invitation_id = $1",
            &[&created.invitation_id],
        )
        .map_err(|err| format!("force expiry: {err}"))?;
    let affected = expire_due(client).map_err(|err| format!("expire_due: {err}"))?;
    if affected < 1 {
        return Err(format!(
            "expire_due affected {affected} rows, expected >= 1"
        ));
    }
    let status: String = client
        .query_one(
            "SELECT status FROM onboarding_invitation WHERE invitation_id = $1",
            &[&created.invitation_id],
        )
        .map_err(|err| format!("read expired status: {err}"))?
        .get(0);
    if status != "expired" {
        return Err(format!("status: expected expired, got {status}"));
    }
    match accept(client, &token) {
        Err(InvitationError::Expired) | Err(InvitationError::InvalidToken) => Ok(()),
        Err(other) => Err(format!("expired accept returned {other}")),
        Ok(id) => Err(format!("expired accept unexpectedly returned id {id}")),
    }
}

/// revoke and expire_due fail safely on wrong states and unknown ids.
fn revoke_and_expire_reject_wrong_states(client: &mut Client, prefix: &str) -> Result<(), String> {
    let person_id = insert_person(client, &format!("{prefix}-states"))?;
    let created = create(client, person_id, Duration::from_secs(3600), ACTOR)
        .map_err(|err| format!("create: {err}"))?;
    revoke(client, created.invitation_id).map_err(|err| format!("revoke pending: {err}"))?;
    if !revoke(client, created.invitation_id).is_err() {
        return Err("revoke of a revoked row must fail".to_string());
    }
    if !revoke(client, -1).is_err() {
        return Err("revoke of an unknown id must fail".to_string());
    }
    expire_due(client).map_err(|err| format!("expire_due: {err}"))?;
    let status: String = client
        .query_one(
            "SELECT status FROM onboarding_invitation WHERE invitation_id = $1",
            &[&created.invitation_id],
        )
        .map_err(|err| format!("read revoked status: {err}"))?
        .get(0);
    if status != "revoked" {
        return Err(format!("expire_due changed a revoked row to {status}"));
    }
    Ok(())
}
