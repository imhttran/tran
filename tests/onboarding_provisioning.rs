//! Integration test: audited member provisioning for an invited identity.
//!
//! The identity database is `DATABASE_URL`. The test skips when it is absent and
//! uses a unique synthetic subject, cleaned up in an always-run path. It proves a
//! new invitee receives the viewer role and the archive corpus grant without any
//! operator action, that a retry is idempotent, and that no elevated role is
//! granted.

use std::fs;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::identity::Role;
use rag_foundation::identity_store::IdentityStore;
use rag_foundation::identity_store_pg::PgIdentityClient;
use rag_foundation::onboarding::provision_invited_member;

const IDENTITY_MIGRATIONS: [&str; 3] = [
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
];

const CORPUS: &str = "synthetic-family-corpus";

fn identity_dsn() -> Option<String> {
    let dsn = std::env::var("DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    if dsn.is_empty() {
        return None;
    }
    let config = postgres::Config::from_str(&dsn).expect("DATABASE_URL parses");
    let database = config.get_dbname().unwrap_or_default();
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable identity database {database:?}"
    );
    Some(dsn)
}

fn apply_migrations(client: &mut Client) {
    for path in IDENTITY_MIGRATIONS {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

fn unique_subject() -> (String, String) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    (
        format!("synthetic-provision-{}-{nanos}", std::process::id()),
        "https://issuer.example.test".to_string(),
    )
}

#[test]
fn an_invitee_is_provisioned_with_member_access_only_once() {
    let Some(dsn) = identity_dsn() else {
        eprintln!("DATABASE_URL unset or not disposable; skipping provisioning test");
        return;
    };
    let (subject, issuer) = unique_subject();

    let mut bootstrap = Client::connect(&dsn, NoTls).expect("connect to the identity database");
    apply_migrations(&mut bootstrap);
    bootstrap
        .execute("DELETE FROM users WHERE subject = $1", &[&subject])
        .expect("clean synthetic account");
    bootstrap
        .execute("DELETE FROM audit_events WHERE subject = $1", &[&subject])
        .expect("clean synthetic audit");

    let client = Client::connect(&dsn, NoTls).expect("connect for provisioning");
    let mut pg = PgIdentityClient::from_client(client);
    let user_id = provision_invited_member(&mut pg, &subject, &issuer, CORPUS, "invitation")
        .expect("provision the invited member");

    let store = IdentityStore::new(pg);
    let roles = store.effective_roles(user_id).expect("read roles");
    assert_eq!(roles, vec![Role::Viewer]);
    assert!(!roles.contains(&Role::Admin));
    assert!(!roles.contains(&Role::Editor));
    let grants = store.effective_grants(user_id).expect("read grants");
    assert_eq!(grants, vec![CORPUS.to_string()]);

    // A retried activation is idempotent: one active role, one active grant, and
    // the same user row.
    let client = Client::connect(&dsn, NoTls).expect("connect for retry");
    let mut pg = PgIdentityClient::from_client(client);
    let again = provision_invited_member(&mut pg, &subject, &issuer, CORPUS, "invitation")
        .expect("retry provisioning");
    assert_eq!(again, user_id);

    let mut verify = Client::connect(&dsn, NoTls).expect("connect to verify");
    let user_rows: i64 = verify
        .query_one(
            "SELECT count(*) FROM users WHERE subject = $1 AND issuer = $2",
            &[&subject, &issuer],
        )
        .expect("count users")
        .get(0);
    assert_eq!(user_rows, 1);
    let role_rows: i64 = verify
        .query_one(
            "SELECT count(*) FROM role_assignments
              WHERE user_id = $1 AND revoked_at IS NULL",
            &[&user_id],
        )
        .expect("count roles")
        .get(0);
    assert_eq!(role_rows, 1);
    let grant_rows: i64 = verify
        .query_one(
            "SELECT count(*) FROM corpus_grants
              WHERE user_id = $1 AND revoked_at IS NULL",
            &[&user_id],
        )
        .expect("count grants")
        .get(0);
    assert_eq!(grant_rows, 1);
    let audits: i64 = verify
        .query_one(
            "SELECT count(*) FROM audit_events
              WHERE subject = $1 AND action IN ('user.provision', 'role.grant', 'corpus.grant')",
            &[&subject],
        )
        .expect("count audits")
        .get(0);
    assert!(audits >= 3, "every provisioning write is audited");

    verify
        .execute("DELETE FROM users WHERE subject = $1", &[&subject])
        .expect("clean synthetic account");
    verify
        .execute("DELETE FROM audit_events WHERE subject = $1", &[&subject])
        .expect("clean synthetic audit");
}
