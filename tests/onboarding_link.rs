//! Synthetic two-database integration test for USR-003C linking.
//!
//! The identity database is `DATABASE_URL`; the genealogy database is the
//! explicitly configured disposable `GENEALOGY_DATABASE_URL`. The test skips
//! when either is absent and refuses a genealogy destination that is not clearly
//! disposable or that equals the identity database.
//!
//! Every synthetic row uses a unique prefix and no real personal data. Both
//! databases are cleaned of only this test's prefixed rows in an always-run path.

use std::fs;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::onboarding::{
    identity_fingerprint, link_for_person, link_reviewed, resolve_account, revoke_link,
    AccountMapError, LinkDecision, LinkError,
};

const IDENTITY_MIGRATIONS: [&str; 3] = [
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
];

const GENEALOGY_MIGRATIONS: [&str; 18] = [
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
    "genealogy/migrations/0013_profile_revision_actor_scope.sql",
    "genealogy/migrations/0014_profile_revision_draft_withdrawal.sql",
    "genealogy/migrations/0015_profile_media.sql",
    "genealogy/migrations/0016_profile_media_withdrawal.sql",
    "genealogy/migrations/0017_profile_media_withdrawal_audit.sql",
    "genealogy/migrations/0018_onboarding_batch.sql",
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
    format!("synthetic-link-{}-{nanos}", std::process::id())
}

fn insert_person(client: &mut Client, key: &str) -> Result<i64, String> {
    client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, 'Synthetic Person') RETURNING person_id",
            &[&key],
        )
        .map_err(|err| format!("insert person: {err}"))
        .map(|row| row.get(0))
}

fn insert_user(client: &mut Client, issuer: &str, subject: &str) -> Result<i64, String> {
    client
        .query_one(
            "INSERT INTO users (subject, issuer) VALUES ($1, $2) RETURNING id",
            &[&subject, &issuer],
        )
        .map_err(|err| format!("insert user: {err}"))
        .map(|row| row.get(0))
}

fn count_audit(client: &mut Client, person_id: i64, decision: &str) -> Result<i64, String> {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_link_audit WHERE person_id = $1 AND decision = $2",
            &[&person_id, &decision],
        )
        .map_err(|err| format!("count audit: {err}"))
        .map(|row| row.get(0))
}

#[test]
fn links_are_reviewed_scoped_and_reject_cross_issuer_ambiguity() {
    let (Some(identity_dsn), Some(genealogy_dsn)) = (identity_dsn(), genealogy_dsn()) else {
        eprintln!("DATABASE_URL or GENEALOGY_DATABASE_URL unset; skipping link test");
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
    let result = run_scenario(&mut identity, &mut genealogy, &prefix);
    cleanup(&mut genealogy, &prefix);
    cleanup_identity(&mut identity, &prefix);
    result.unwrap_or_else(|err| panic!("{err}"));
}

fn run_scenario(identity: &mut Client, genealogy: &mut Client, prefix: &str) -> Result<(), String> {
    let issuer_a = "synthetic-issuer-a";
    let issuer_b = "synthetic-issuer-b";
    let subject = format!("{prefix}-subject");
    let deactivated = format!("{prefix}-deactivated");
    let ambiguous = format!("{prefix}-ambiguous");
    let second_subject = format!("{prefix}-second-subject");

    insert_user(identity, issuer_a, &subject)?;
    let deactivated_id = insert_user(identity, issuer_a, &deactivated)?;
    identity
        .execute(
            "UPDATE users SET deactivated_at = now() WHERE id = $1",
            &[&deactivated_id],
        )
        .map_err(|err| format!("deactivate synthetic user: {err}"))?;
    insert_user(identity, issuer_a, &ambiguous)?;
    insert_user(identity, issuer_b, &ambiguous)?;
    insert_user(identity, issuer_a, &second_subject)?;

    let person_one = insert_person(genealogy, &format!("{prefix}-person-1"))?;
    let person_two = insert_person(genealogy, &format!("{prefix}-person-2"))?;

    // Exact pair resolves and the fingerprint is the documented hash.
    let account = resolve_account(identity, issuer_a, &subject)
        .map_err(|err| format!("resolve active identity: {err}"))?;
    if account.identity_fingerprint != identity_fingerprint(issuer_a, &subject) {
        return Err("fingerprint does not match issuer/subject".to_string());
    }

    if resolve_account(identity, issuer_a, &format!("{prefix}-missing"))
        != Err(AccountMapError::UnknownIdentity)
    {
        return Err("missing identity was not rejected".to_string());
    }
    if resolve_account(identity, issuer_a, &deactivated)
        != Err(AccountMapError::DeactivatedIdentity)
    {
        return Err("deactivated identity was not rejected".to_string());
    }
    if resolve_account(identity, "", &ambiguous) != Err(AccountMapError::AmbiguousIdentity) {
        return Err("ambiguous subject was not rejected".to_string());
    }

    // Approved link: one link and one audit row.
    let linked = link_reviewed(
        genealogy,
        person_one,
        &account,
        LinkDecision::Approve,
        "synthetic-reviewer",
    )
    .map_err(|err| format!("approve link: {err}"))?
    .ok_or_else(|| "approval returned no link".to_string())?;
    if linked.person_id != person_one || linked.account_id != account.account_id {
        return Err("link record does not match the reviewed pair".to_string());
    }
    if link_for_person(genealogy, person_one)
        .map_err(|err| format!("read link: {err}"))?
        .is_none()
    {
        return Err("approved link was not stored".to_string());
    }
    if count_audit(genealogy, person_one, "linked")? != 1 {
        return Err("approved link did not append one audit row".to_string());
    }

    // One account cannot link to a second person, and one person cannot hold a
    // second account.
    let second_account = resolve_account(identity, issuer_a, &second_subject)
        .map_err(|err| format!("resolve second identity: {err}"))?;
    if link_reviewed(
        genealogy,
        person_one,
        &second_account,
        LinkDecision::Approve,
        "synthetic-reviewer",
    ) != Err(LinkError::AlreadyLinked)
    {
        return Err("second account for the same person was not rejected".to_string());
    }
    if link_reviewed(
        genealogy,
        person_two,
        &account,
        LinkDecision::Approve,
        "synthetic-reviewer",
    ) != Err(LinkError::AlreadyLinked)
    {
        return Err("second person for the same account was not rejected".to_string());
    }

    // Rejection writes an audit row and creates no link.
    if link_reviewed(
        genealogy,
        person_two,
        &second_account,
        LinkDecision::Reject,
        "synthetic-reviewer",
    )
    .map_err(|err| format!("reject link: {err}"))?
    .is_some()
    {
        return Err("rejection created a link".to_string());
    }
    if count_audit(genealogy, person_two, "rejected")? != 1 {
        return Err("rejection did not append one audit row".to_string());
    }
    if link_for_person(genealogy, person_two)
        .map_err(|err| format!("read rejected link: {err}"))?
        .is_some()
    {
        return Err("rejection stored a link".to_string());
    }

    // Revocation removes the link and appends a revoked row.
    revoke_link(genealogy, linked.link_id, "synthetic-reviewer")
        .map_err(|err| format!("revoke link: {err}"))?;
    if link_for_person(genealogy, person_one)
        .map_err(|err| format!("read revoked link: {err}"))?
        .is_some()
    {
        return Err("revocation did not remove the link".to_string());
    }
    if count_audit(genealogy, person_one, "revoked")? != 1 {
        return Err("revocation did not append one audit row".to_string());
    }

    // A missing person and a blank reviewer are rejected before any write.
    if link_reviewed(
        genealogy,
        999_999_999,
        &account,
        LinkDecision::Approve,
        "synthetic-reviewer",
    ) != Err(LinkError::UnknownPerson)
    {
        return Err("unknown person was not rejected".to_string());
    }
    if link_reviewed(genealogy, person_two, &account, LinkDecision::Approve, "  ")
        != Err(LinkError::InvalidReviewer)
    {
        return Err("blank reviewer was not rejected".to_string());
    }

    Ok(())
}

fn cleanup(genealogy: &mut Client, prefix: &str) {
    let pattern = format!("{prefix}%");
    // Delete child rows explicitly so cleanup is correct even on a database
    // upgraded from a schema version whose foreign keys were missing.
    let _ = genealogy.execute(
        "DELETE FROM onboarding_link_audit WHERE person_id IN \
         (SELECT person_id FROM person WHERE person_key LIKE $1)",
        &[&pattern],
    );
    let _ = genealogy.execute(
        "DELETE FROM onboarding_person_account_link WHERE person_id IN \
         (SELECT person_id FROM person WHERE person_key LIKE $1)",
        &[&pattern],
    );
    let _ = genealogy.execute("DELETE FROM person WHERE person_key LIKE $1", &[&pattern]);
}

fn cleanup_identity(identity: &mut Client, prefix: &str) {
    let pattern = format!("{prefix}%");
    let _ = identity.execute("DELETE FROM users WHERE subject LIKE $1", &[&pattern]);
}

/// Removes orphaned link rows left by an earlier schema version without foreign
/// keys. Safe on the disposable test database only.
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
}
