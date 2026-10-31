//! Last-active-admin and deactivation tests against a disposable RAG database.

use std::thread;

use postgres::{Client, NoTls};
use rag_foundation::identity::Role;
use rag_foundation::identity_store::identity_client::Client as IdentityClient;
use rag_foundation::identity_store::IdentityStoreError;
use rag_foundation::identity_store_pg::PgIdentityClient;

/// The issuer every seeded account belongs to. Identity is keyed on
/// `(issuer, subject)`, so every state-changing call must pass it.
const ISSUER: &str = "https://issuer.example";

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

fn seed_user(client: &mut Client, subject: &str, role: &str) {
    client
        .execute(
            "INSERT INTO users (subject, issuer) VALUES ($1, $2)",
            &[&subject, &ISSUER],
        )
        .expect("insert user");
    client
        .execute(
            "INSERT INTO role_assignments (user_id, role, granted_by) \
             SELECT id, $3, 'test' FROM users WHERE subject = $1 AND issuer = $2",
            &[&subject, &ISSUER, &role],
        )
        .expect("insert role");
}

#[test]
fn last_active_admin_cannot_be_revoked() {
    let Some(base) = base_dsn() else {
        eprintln!("DATABASE_URL is unset; skipping last-admin test");
        return;
    };
    let database = "rag_identity_last_admin_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_and_migrate(&mut client);
    seed_user(&mut client, "synthetic-admin", "admin");
    seed_user(&mut client, "synthetic-viewer", "viewer");

    let mut identity = PgIdentityClient::connect(&dsn).expect("identity client");
    let err = identity
        .change_role(
            "synthetic-admin",
            ISSUER,
            Role::Admin,
            "synthetic-admin",
            false,
        )
        .expect_err("last admin revocation must fail");
    assert!(matches!(err, IdentityStoreError::LastAdmin));

    identity
        .change_role(
            "synthetic-viewer",
            ISSUER,
            Role::Viewer,
            "synthetic-admin",
            false,
        )
        .expect("viewer revocation is allowed");
}

#[test]
fn concurrent_admin_revocations_leave_one_active_admin() {
    let Some(base) = base_dsn() else {
        return;
    };
    let database = "rag_identity_admin_race_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_and_migrate(&mut client);
    seed_user(&mut client, "synthetic-admin-a", "admin");
    seed_user(&mut client, "synthetic-admin-b", "admin");

    let results = thread::scope(|scope| {
        let first = dsn.clone();
        let second = dsn.clone();
        let a = scope.spawn(move || {
            let mut identity = PgIdentityClient::connect(&first).expect("client a");
            identity
                .change_role(
                    "synthetic-admin-b",
                    ISSUER,
                    Role::Admin,
                    "synthetic-admin-a",
                    false,
                )
                .is_ok()
        });
        let b = scope.spawn(move || {
            let mut identity = PgIdentityClient::connect(&second).expect("client b");
            identity
                .change_role(
                    "synthetic-admin-a",
                    ISSUER,
                    Role::Admin,
                    "synthetic-admin-b",
                    false,
                )
                .is_ok()
        });
        (a.join().unwrap(), b.join().unwrap())
    });

    assert_eq!(
        results.0 as i32 + results.1 as i32,
        1,
        "exactly one concurrent revocation must commit"
    );
    let active: i64 = client
        .query_one(
            "SELECT count(*) FROM role_assignments WHERE role = 'admin' AND revoked_at IS NULL",
            &[],
        )
        .expect("count admins")
        .get(0);
    assert_eq!(active, 1, "one active administrator must remain");
}

#[test]
fn deactivated_user_is_not_resolved() {
    let Some(base) = base_dsn() else {
        return;
    };
    let database = "rag_identity_deactivate_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_and_migrate(&mut client);
    seed_user(&mut client, "synthetic-account", "viewer");

    let mut identity = PgIdentityClient::connect(&dsn).expect("identity client");
    identity
        .deactivate_user("synthetic-account", ISSUER, "synthetic-admin")
        .expect("deactivate");
    assert!(identity
        .user_by_subject("synthetic-account", ISSUER)
        .expect("lookup")
        .is_none());
}

#[test]
fn last_active_admin_cannot_be_deactivated() {
    let Some(base) = base_dsn() else {
        return;
    };
    let database = "rag_identity_deactivate_admin_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_and_migrate(&mut client);
    seed_user(&mut client, "synthetic-admin", "admin");
    seed_user(&mut client, "synthetic-viewer", "viewer");

    let mut identity = PgIdentityClient::connect(&dsn).expect("identity client");
    let err = identity
        .deactivate_user("synthetic-admin", ISSUER, "synthetic-admin")
        .expect_err("deactivating the last admin must fail");
    assert!(matches!(err, IdentityStoreError::LastAdmin));
    assert!(
        identity
            .user_by_subject("synthetic-admin", ISSUER)
            .expect("lookup")
            .is_some(),
        "the refused deactivation must leave the admin active"
    );

    identity
        .deactivate_user("synthetic-viewer", ISSUER, "synthetic-admin")
        .expect("a non-admin can still be deactivated");
}

#[test]
fn deactivated_admin_does_not_count_toward_the_last_admin() {
    let Some(base) = base_dsn() else {
        return;
    };
    let database = "rag_identity_deactivated_admin_count_test";
    ensure_database(&with_database(&base, "postgres"), database);
    let dsn = with_database(&base, database);
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    reset_and_migrate(&mut client);
    seed_user(&mut client, "synthetic-admin-a", "admin");
    seed_user(&mut client, "synthetic-admin-b", "admin");

    let mut identity = PgIdentityClient::connect(&dsn).expect("identity client");
    identity
        .deactivate_user("synthetic-admin-a", ISSUER, "synthetic-admin-b")
        .expect("one of two admins can be deactivated");
    let err = identity
        .change_role(
            "synthetic-admin-b",
            ISSUER,
            Role::Admin,
            "synthetic-admin-b",
            false,
        )
        .expect_err("the remaining active admin must not be revocable");
    assert!(matches!(err, IdentityStoreError::LastAdmin));
    let err = identity
        .deactivate_user("synthetic-admin-b", ISSUER, "synthetic-admin-b")
        .expect_err("the remaining active admin must not be deactivatable");
    assert!(matches!(err, IdentityStoreError::LastAdmin));

    // The deactivated admin's dormant role can still be revoked: it does not
    // reduce the active-admin count.
    identity
        .change_role(
            "synthetic-admin-a",
            ISSUER,
            Role::Admin,
            "synthetic-admin-b",
            false,
        )
        .expect("a deactivated user's admin role can be revoked");
    let dormant: i64 = client
        .query_one(
            "SELECT count(*) FROM role_assignments r JOIN users u ON u.id = r.user_id \
              WHERE u.subject = 'synthetic-admin-a' AND r.role = 'admin' AND r.revoked_at IS NULL",
            &[],
        )
        .expect("count dormant admin roles")
        .get(0);
    assert_eq!(dormant, 0, "the dormant admin role must be revoked");
}
