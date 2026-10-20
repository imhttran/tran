//! Synthetic cross-issuer identity tests against a disposable RAG database.
//!
//! These tests prove that identity is immutable on `(issuer, subject)`:
//! a subject string from issuer B cannot take over, reactivate, deactivate, or
//! transfer privileges for an account registered under issuer A. They run only
//! against a disposable database derived from `DATABASE_URL`.

use postgres::{Client, NoTls};
use rag_foundation::identity::Role;
use rag_foundation::identity_store::identity_client::Client as IdentityClient;
use rag_foundation::identity_store::IdentityStoreError;
use rag_foundation::identity_store_pg::PgIdentityClient;

/// Issuer A: the owner of the seeded account.
const ISSUER_A: &str = "https://issuer-a.example";
/// Issuer B: an attacker-controlled issuer that shares the subject string.
const ISSUER_B: &str = "https://issuer-b.example";

fn base_dsn() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
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

fn reset_and_migrate(client: &mut Client) {
    client
        .batch_execute(
            "DROP TABLE IF EXISTS audit_events, corpus_grants, role_assignments, users CASCADE;",
        )
        .expect("reset identity tables");
    client
        .batch_execute(include_str!("../migrations/006_identity.sql"))
        .expect("apply identity migration");
    client
        .batch_execute(include_str!("../migrations/007_user_lifecycle.sql"))
        .expect("apply lifecycle migration");
    client
        .batch_execute(include_str!(
            "../migrations/008_identity_issuer_subject.sql"
        ))
        .expect("apply issuer+subject migration");
}

fn seed_user(client: &mut Client, subject: &str, issuer: &str, role: &str) -> i64 {
    let user_id: i64 = client
        .query_one(
            "INSERT INTO users (subject, issuer) VALUES ($1, $2) RETURNING id",
            &[&subject, &issuer],
        )
        .expect("insert user")
        .get(0);
    client
        .execute(
            "INSERT INTO role_assignments (user_id, role, granted_by) VALUES ($1, $2, 'test')",
            &[&user_id, &role],
        )
        .expect("insert role");
    user_id
}

fn active_user(client: &mut Client, subject: &str, issuer: &str) -> Option<i64> {
    client
        .query_opt(
            "SELECT id FROM users WHERE subject = $1 AND issuer = $2 AND deactivated_at IS NULL",
            &[&subject, &issuer],
        )
        .expect("lookup user")
        .map(|row| row.get(0))
}

fn audit_count(client: &mut Client, subject: &str, issuer: &str) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM audit_events \
              WHERE subject = $1 AND detail->>'issuer' = $2",
            &[&subject, &issuer],
        )
        .expect("count audit")
        .get(0)
}

#[test]
fn provision_for_other_issuer_cannot_reactivate_or_modify() {
    let Some(base) = base_dsn() else {
        eprintln!("DATABASE_URL is unset; skipping cross-issuer provision test");
        return;
    };
    let database = "rag_identity_cross_issuer_provision_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_and_migrate(&mut client);

    let subject = "synthetic-shared";
    let user_a = seed_user(&mut client, subject, ISSUER_A, "editor");

    let mut identity = PgIdentityClient::connect(&dsn).expect("identity client");
    let user_b = identity
        .provision_user(subject, ISSUER_B, "synthetic-admin")
        .expect("provision for issuer B creates a distinct account");
    assert_ne!(user_a, user_b, "issuer B must get its own account row");

    // The issuer A row is unchanged and still active with its own issuer.
    let row = client
        .query_one(
            "SELECT issuer, (deactivated_at IS NULL) FROM users WHERE id = $1",
            &[&user_a],
        )
        .expect("read issuer A row");
    let issuer_a: String = row.get(0);
    let is_active: bool = row.get(1);
    assert_eq!(issuer_a, ISSUER_A, "issuer A must never be rewritten");
    assert!(is_active, "issuer A must stay active");
    assert_eq!(active_user(&mut client, subject, ISSUER_A), Some(user_a));

    // Issuer B's provision call did not deactivate then reactivate A: A is
    // still there, and roles on A are untouched.
    assert_eq!(
        identity.effective_roles(user_a).expect("roles A"),
        vec![Role::Editor]
    );

    // A deactivated A can only be reactivated by a call with issuer A.
    identity
        .deactivate_user(subject, ISSUER_A, "synthetic-admin")
        .expect("deactivate A");
    identity
        .provision_user(subject, ISSUER_B, "synthetic-admin")
        .expect("re-provision B is a no-op on A");
    assert!(
        active_user(&mut client, subject, ISSUER_A).is_none(),
        "issuer B must not reactivate issuer A"
    );
    identity
        .provision_user(subject, ISSUER_A, "synthetic-admin")
        .expect("issuer A reactivates its own account");
    assert_eq!(active_user(&mut client, subject, ISSUER_A), Some(user_a));
}

#[test]
fn deactivate_for_other_issuer_fails_with_unknown_user() {
    let Some(base) = base_dsn() else {
        return;
    };
    let database = "rag_identity_cross_issuer_deactivate_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_and_migrate(&mut client);

    let subject = "synthetic-shared-deactivate";
    let user_a = seed_user(&mut client, subject, ISSUER_A, "viewer");

    let mut identity = PgIdentityClient::connect(&dsn).expect("identity client");
    let err = identity
        .deactivate_user(subject, ISSUER_B, "synthetic-admin")
        .expect_err("issuer B must not deactivate issuer A");
    assert!(matches!(err, IdentityStoreError::UnknownUser(_)));
    assert_eq!(
        active_user(&mut client, subject, ISSUER_A),
        Some(user_a),
        "issuer A account must stay active"
    );
}

#[test]
fn same_subject_two_issuers_keep_state_separate() {
    let Some(base) = base_dsn() else {
        return;
    };
    let database = "rag_identity_cross_issuer_separate_state_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_and_migrate(&mut client);

    let subject = "synthetic-twin";
    let user_a = seed_user(&mut client, subject, ISSUER_A, "viewer");
    let mut identity = PgIdentityClient::connect(&dsn).expect("identity client");
    let user_b = identity
        .provision_user(subject, ISSUER_B, "synthetic-admin")
        .expect("provision B");
    assert_ne!(user_a, user_b);

    // Distinct role assignments.
    identity
        .change_role(subject, ISSUER_A, Role::Editor, "synthetic-admin", true)
        .expect("grant editor to A");
    identity
        .change_role(subject, ISSUER_B, Role::Admin, "synthetic-admin", true)
        .expect("grant admin to B");
    let mut roles_a = identity.effective_roles(user_a).expect("roles A");
    roles_a.sort_by_key(|role| role.as_str());
    let mut roles_b = identity.effective_roles(user_b).expect("roles B");
    roles_b.sort_by_key(|role| role.as_str());
    assert!(roles_a.contains(&Role::Viewer));
    assert!(roles_a.contains(&Role::Editor));
    assert!(!roles_a.contains(&Role::Admin));
    assert!(roles_b.contains(&Role::Admin));
    assert!(!roles_b.contains(&Role::Editor));

    // Distinct corpus grants.
    identity
        .grant_corpus(subject, ISSUER_A, "trusted", "synthetic-admin")
        .expect("grant corpus to A");
    identity
        .revoke_corpus(subject, ISSUER_A, "trusted", "synthetic-admin")
        .expect("revoke corpus from A");
    assert!(
        identity
            .effective_grants(user_a)
            .expect("grants A")
            .is_empty(),
        "issuer A grant revoke must not affect issuer B"
    );
    identity
        .grant_corpus(subject, ISSUER_B, "trusted", "synthetic-admin")
        .expect("grant corpus to B");
    assert_eq!(
        identity.effective_grants(user_b).expect("grants B"),
        vec!["trusted".to_string()]
    );

    // Distinct deactivation state.
    identity
        .deactivate_user(subject, ISSUER_A, "synthetic-admin")
        .expect("deactivate A");
    assert!(active_user(&mut client, subject, ISSUER_A).is_none());
    assert_eq!(active_user(&mut client, subject, ISSUER_B), Some(user_b));

    // Distinct audit history: each action recorded with its own issuer.
    let audit_a = audit_count(&mut client, subject, ISSUER_A);
    let audit_b = audit_count(&mut client, subject, ISSUER_B);
    assert!(audit_a > 0, "issuer A has its own audit rows");
    assert!(audit_b > 0, "issuer B has its own audit rows");
}

#[test]
fn cross_issuer_role_grant_fails_with_unknown_user() {
    let Some(base) = base_dsn() else {
        return;
    };
    let database = "rag_identity_cross_issuer_role_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_and_migrate(&mut client);

    let subject = "synthetic-role-shared";
    let user_a = seed_user(&mut client, subject, ISSUER_A, "viewer");
    let mut identity = PgIdentityClient::connect(&dsn).expect("identity client");

    let err = identity
        .change_role(subject, ISSUER_B, Role::Admin, "synthetic-admin", true)
        .expect_err("issuer B must not grant a role on issuer A");
    assert!(matches!(err, IdentityStoreError::UnknownUser(_)));
    assert!(!identity
        .effective_roles(user_a)
        .expect("roles A")
        .contains(&Role::Admin));

    let err = identity
        .grant_corpus(subject, ISSUER_B, "trusted", "synthetic-admin")
        .expect_err("issuer B must not grant a corpus on issuer A");
    assert!(matches!(err, IdentityStoreError::UnknownUser(_)));
}

#[test]
fn last_active_admin_guard_holds_across_issuers() {
    let Some(base) = base_dsn() else {
        return;
    };
    let database = "rag_identity_cross_issuer_last_admin_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_and_migrate(&mut client);

    // One active admin under issuer A, one non-admin under issuer B with the
    // same subject string.
    let admin_subject = "synthetic-cross-admin";
    seed_user(&mut client, admin_subject, ISSUER_A, "admin");
    let mut identity = PgIdentityClient::connect(&dsn).expect("identity client");
    let other = identity
        .provision_user(admin_subject, ISSUER_B, "synthetic-admin")
        .expect("provision twin under B");
    identity
        .change_role(
            admin_subject,
            ISSUER_B,
            Role::Viewer,
            "synthetic-admin",
            true,
        )
        .expect("give B viewer only");
    assert!(!identity
        .effective_roles(other)
        .expect("roles B")
        .contains(&Role::Admin));

    // Removing the admin under issuer A is still refused by the guard.
    let err = identity
        .change_role(
            admin_subject,
            ISSUER_A,
            Role::Admin,
            "synthetic-admin",
            false,
        )
        .expect_err("the guard must refuse the last active admin");
    assert!(matches!(err, IdentityStoreError::LastAdmin));
    let err = identity
        .deactivate_user(admin_subject, ISSUER_A, "synthetic-admin")
        .expect_err("the guard must refuse deactivating the last active admin");
    assert!(matches!(err, IdentityStoreError::LastAdmin));
}
