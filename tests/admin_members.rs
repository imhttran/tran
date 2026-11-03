//! Admin > Family Members against disposable databases.
//!
//! Each test creates its own `*_test` database on the `DATABASE_URL` server,
//! including a disposable genealogy database for the linkage test.
//! Every account and person is synthetic, and the reset provider is a local
//! listener: no real provider or mailbox is ever contacted.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use postgres::{Client, NoTls};
use rag_foundation::admin_members::{
    linked_people, linked_person_keys, list_members, request_password_reset, MemberFilter,
    MemberStatus, ResetError, ResetProvider, ResetRequest, ResetState,
};
use rag_foundation::genealogy::appointments::search_nomination_people;
use rag_foundation::identity::{decide, Decision, Operation, Role, VerifiedIdentity};

const ISSUER_A: &str = "https://issuer-a.example";
const ISSUER_B: &str = "https://issuer-b.example";

fn base_dsn() -> Option<String> {
    std::env::var("DATABASE_URL")
        .ok()
        .filter(|dsn| !dsn.trim().is_empty())
}

/// Creates (if needed) and resets one disposable identity database.
fn identity_db(base: &str, database: &str) -> Client {
    let mut admin_config = postgres::Config::from_str(base).expect("DATABASE_URL parses");
    let mut admin = admin_config
        .dbname("postgres")
        .connect(NoTls)
        .expect("connect");
    let exists: i64 = admin
        .query_one(
            "SELECT count(*) FROM pg_database WHERE datname = $1",
            &[&database],
        )
        .expect("check database")
        .get(0);
    if exists == 0 {
        admin
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .expect("create disposable database");
    }
    let mut client = postgres::Config::from_str(base)
        .expect("DATABASE_URL parses")
        .dbname(database)
        .connect(NoTls)
        .expect("connect to disposable database");
    client
        .batch_execute(
            "DROP TABLE IF EXISTS audit_events, corpus_grants, role_assignments, users CASCADE;",
        )
        .expect("reset");
    for sql in [
        include_str!("../migrations/006_identity.sql"),
        include_str!("../migrations/007_user_lifecycle.sql"),
        include_str!("../migrations/008_identity_issuer_subject.sql"),
    ] {
        client.batch_execute(sql).expect("migrate");
    }
    client
}

fn seed(
    client: &mut Client,
    issuer: &str,
    subject: &str,
    email: Option<&str>,
    role: Option<&str>,
) -> i64 {
    let id: i64 = client
        .query_one(
            "INSERT INTO users (issuer, subject, email, display_name) VALUES ($1, $2, $3, $4) RETURNING id",
            &[&issuer, &subject, &email, &format!("Synthetic {subject}")],
        )
        .expect("seed user")
        .get(0);
    if let Some(role) = role {
        client
            .execute(
                "INSERT INTO role_assignments (user_id, role, granted_by) VALUES ($1, $2, 'test')",
                &[&id, &role],
            )
            .expect("seed role");
    }
    id
}

#[test]
fn directory_filters_paginates_and_keeps_issuers_apart() {
    let Some(base) = base_dsn() else {
        eprintln!("DATABASE_URL is unset; skipping admin directory test");
        return;
    };
    let mut client = identity_db(&base, "admin_members_directory_test");
    seed(
        &mut client,
        ISSUER_A,
        "synthetic-admin",
        None,
        Some("admin"),
    );
    seed(
        &mut client,
        ISSUER_A,
        "synthetic-shared",
        Some("shared-a@example.test"),
        Some("viewer"),
    );
    seed(
        &mut client,
        ISSUER_B,
        "synthetic-shared",
        Some("shared-b@example.test"),
        Some("editor"),
    );
    let gone = seed(
        &mut client,
        ISSUER_A,
        "synthetic_gone",
        None,
        Some("viewer"),
    );
    client
        .execute(
            "UPDATE users SET deactivated_at = now() WHERE id = $1",
            &[&gone],
        )
        .expect("deactivate");

    let (all, next) = list_members(&mut client, &MemberFilter::default()).expect("list");
    assert_eq!(all.len(), 4);
    assert_eq!(next, None);

    // The same subject under two issuers is two distinct accounts with their own roles.
    let shared = MemberFilter {
        query: Some("synthetic-shared".into()),
        ..MemberFilter::default()
    };
    let (rows, _) = list_members(&mut client, &shared).expect("search");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].roles, vec![Role::Viewer]);
    assert_eq!(rows[1].roles, vec![Role::Editor]);
    assert_ne!(rows[0].issuer, rows[1].issuer);

    // `_` is literal, not a wildcard.
    let literal = MemberFilter {
        query: Some("synthetic_".into()),
        ..MemberFilter::default()
    };
    let (rows, _) = list_members(&mut client, &literal).expect("literal search");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, MemberStatus::Deactivated);

    let editors = MemberFilter {
        role: Some(Role::Editor),
        ..MemberFilter::default()
    };
    assert_eq!(
        list_members(&mut client, &editors).expect("role").0.len(),
        1
    );
    let active = MemberFilter {
        status: Some(MemberStatus::Active),
        ..MemberFilter::default()
    };
    assert_eq!(
        list_members(&mut client, &active).expect("status").0.len(),
        3
    );

    let first = MemberFilter {
        limit: Some(3),
        ..MemberFilter::default()
    };
    let (page, cursor) = list_members(&mut client, &first).expect("page 1");
    assert_eq!(page.len(), 3);
    let rest = MemberFilter {
        limit: Some(3),
        after_id: cursor,
        ..MemberFilter::default()
    };
    let (page, cursor) = list_members(&mut client, &rest).expect("page 2");
    assert_eq!((page.len(), cursor), (1, None));
}

/// A one-shot local provider. Returns its URL and a channel with each request body.
fn fake_provider(statuses: Vec<u16>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}/reset", listener.local_addr().expect("addr"));
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for status in statuses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut raw = Vec::new();
            let mut buffer = [0u8; 4096];
            // Read headers, then exactly Content-Length body bytes.
            loop {
                let n = stream.read(&mut buffer).unwrap_or(0);
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if raw.len() >= split + 4 + length {
                        sender.send(text[split + 4..].to_string()).ok();
                        break;
                    }
                }
            }
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .ok();
        }
    });
    (url, receiver)
}

fn audit_states(client: &mut Client, subject: &str) -> Vec<String> {
    client
        .query(
            "SELECT detail->>'state', detail::text FROM audit_events
              WHERE action = 'password_reset_requested' AND subject = $1 ORDER BY id",
            &[&subject],
        )
        .expect("audit")
        .iter()
        .map(|row| {
            let detail: String = row.get(1);
            assert!(
                !detail.contains("@example.test"),
                "audit must not hold an address"
            );
            row.get(0)
        })
        .collect()
}

#[test]
fn reset_fails_closed_throttles_and_never_claims_unconfirmed_success() {
    let Some(base) = base_dsn() else {
        eprintln!("DATABASE_URL is unset; skipping password reset test");
        return;
    };
    let mut client = identity_db(&base, "admin_members_reset_test");
    seed(
        &mut client,
        ISSUER_A,
        "synthetic-admin",
        None,
        Some("admin"),
    );
    seed(
        &mut client,
        ISSUER_A,
        "synthetic-member",
        Some("member@example.test"),
        Some("viewer"),
    );
    seed(
        &mut client,
        ISSUER_A,
        "synthetic-no-email",
        None,
        Some("viewer"),
    );
    let gone = seed(
        &mut client,
        ISSUER_A,
        "synthetic-gone",
        Some("gone@example.test"),
        None,
    );
    client
        .execute(
            "UPDATE users SET deactivated_at = now() WHERE id = $1",
            &[&gone],
        )
        .expect("deactivate");

    let http = reqwest::blocking::Client::new();
    let cooldown = Duration::from_secs(300);
    let request = |subject| ResetRequest {
        actor_issuer: ISSUER_A,
        actor_subject: "synthetic-admin",
        issuer: ISSUER_A,
        subject,
    };

    // Unknown, other-issuer, and deactivated accounts are refused, and audited.
    let other_issuer = ResetRequest {
        issuer: ISSUER_B,
        ..request("synthetic-member")
    };
    for (req, expected) in [
        (request("synthetic-nobody"), ResetError::UnknownUser),
        (other_issuer, ResetError::UnknownUser),
        (request("synthetic-gone"), ResetError::Deactivated),
    ] {
        let err = request_password_reset(&mut client, None, &http, &req, cooldown).unwrap_err();
        assert_eq!(err, expected);
    }
    assert_eq!(
        audit_states(&mut client, "synthetic-gone"),
        vec!["rejected_deactivated"]
    );

    // No local address is required: the provider resolves its own recipient.
    let state = request_password_reset(
        &mut client,
        None,
        &http,
        &request("synthetic-no-email"),
        cooldown,
    )
    .expect("an account without a stored address is not refused locally");
    assert_eq!(state, ResetState::NotConfigured);

    // No provider: fail closed and audit it.
    let state = request_password_reset(
        &mut client,
        None,
        &http,
        &request("synthetic-member"),
        cooldown,
    )
    .expect("not configured");
    assert_eq!(state, ResetState::NotConfigured);

    // A provider error is a failure, not a success, and does not start the cooldown.
    let (url, bodies) = fake_provider(vec![500, 202]);
    let provider = ResetProvider {
        url,
        bearer: Some("synthetic-bearer".into()),
    };
    let state = request_password_reset(
        &mut client,
        Some(&provider),
        &http,
        &request("synthetic-member"),
        cooldown,
    )
    .expect("failed");
    assert_eq!(state, ResetState::Failed);
    bodies
        .recv_timeout(Duration::from_secs(5))
        .expect("first call");

    // Accepted: the provider receives the identity only, never an address.
    let state = request_password_reset(
        &mut client,
        Some(&provider),
        &http,
        &request("synthetic-member"),
        cooldown,
    )
    .expect("requested");
    assert_eq!(state, ResetState::Requested);
    let body: serde_json::Value = serde_json::from_str(
        &bodies
            .recv_timeout(Duration::from_secs(5))
            .expect("second call"),
    )
    .expect("json body");
    assert_eq!(
        body,
        serde_json::json!({ "issuer": ISSUER_A, "subject": "synthetic-member" })
    );

    // A repeat inside the cooldown is throttled and never reaches the provider.
    let state = request_password_reset(
        &mut client,
        Some(&provider),
        &http,
        &request("synthetic-member"),
        cooldown,
    )
    .expect("throttled");
    assert_eq!(state, ResetState::Throttled);
    assert!(bodies.recv_timeout(Duration::from_millis(300)).is_err());

    assert_eq!(
        audit_states(&mut client, "synthetic-member"),
        vec![
            "rejected_unknown_account",
            "not_configured",
            "failed",
            "requested",
            "throttled"
        ]
    );
}

#[test]
fn linked_people_reads_links_without_writing() {
    let Some(base) = base_dsn() else {
        eprintln!("DATABASE_URL is unset; skipping linkage test");
        return;
    };
    // A dedicated disposable genealogy database, so no other suite's rows or
    // sequences interfere, and no real genealogy database is ever touched.
    let mut client = genealogy_db(&base, "admin_members_genealogy_test");
    let prefix = format!("adm-test-{}", std::process::id());
    let account_id = -(std::process::id() as i64);
    let person_id: i64 = client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, 'Synthetic Person') RETURNING person_id",
            &[&format!("{prefix}-person")],
        )
        .expect("person")
        .get(0);
    client
        .execute(
            "INSERT INTO onboarding_person_account_link (person_id, account_id, linked_by) VALUES ($1, $2, 'test')",
            &[&person_id, &account_id],
        )
        .expect("link");
    client
        .execute(
            "INSERT INTO onboarding_username_reservation (username, username_key, person_id, reserved_by)
             VALUES ($1, $1, $2, 'test')",
            &[&format!("{prefix}-user"), &person_id],
        )
        .expect("reservation");
    let links = linked_people(&mut client, &[account_id, account_id - 1_000_000]);
    client
        .execute("DELETE FROM person WHERE person_id = $1", &[&person_id])
        .expect("cleanup");
    let links = links.expect("linked people");
    assert_eq!(links.len(), 1);
    let link = &links[&account_id];
    assert!(link.link_id > 0);
    assert_eq!(link.person_key, format!("{prefix}-person"));
    assert_eq!(link.display_name, "Synthetic Person");
    assert_eq!(
        link.username.as_deref(),
        Some(format!("{prefix}-user").as_str())
    );
}

/// Recreates a disposable genealogy database with every genealogy migration.
fn genealogy_db(base: &str, database: &str) -> Client {
    let mut admin = postgres::Config::from_str(base)
        .expect("DATABASE_URL parses")
        .dbname("postgres")
        .connect(NoTls)
        .expect("connect");
    for statement in [
        format!("DROP DATABASE IF EXISTS {database}"),
        format!("CREATE DATABASE {database}"),
    ] {
        admin
            .batch_execute(&statement)
            .expect("recreate disposable genealogy database");
    }
    let mut client = postgres::Config::from_str(base)
        .expect("DATABASE_URL parses")
        .dbname(database)
        .connect(NoTls)
        .expect("connect genealogy");
    let mut migrations: Vec<_> = std::fs::read_dir("genealogy/migrations")
        .expect("read migrations")
        .map(|entry| entry.expect("migration entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    migrations.sort();
    for path in migrations {
        client
            .batch_execute(&std::fs::read_to_string(&path).expect("read migration"))
            .unwrap_or_else(|err| panic!("migrate {}: {err}", path.display()));
    }
    client
}

fn identity(role: Role) -> VerifiedIdentity {
    VerifiedIdentity::new(
        "synthetic-subject",
        "https://synthetic-issuer",
        vec![role],
        vec![],
    )
}

#[test]
fn admin_person_search_requires_link_permission() {
    assert_eq!(
        decide(None, Operation::OnboardingLink),
        Decision::Unauthenticated
    );
    assert_eq!(
        decide(Some(&identity(Role::Viewer)), Operation::OnboardingLink),
        Decision::Forbidden
    );
    assert_eq!(
        decide(Some(&identity(Role::Admin)), Operation::OnboardingLink),
        Decision::Allow
    );
}

#[test]
fn admin_person_search_finds_unaccented_and_reports_link_state() {
    let Some(base) = base_dsn() else {
        eprintln!("DATABASE_URL is unset; skipping admin person search test");
        return;
    };
    let mut client = genealogy_db(&base, "admin_members_people_search_test");
    client
        .batch_execute(
            "INSERT INTO person (person_key, display_name) VALUES
                 ('synthetic-unlinked', 'Synthetic Ren\u{00e9} Target'),
                 ('synthetic-linked', 'Synthetic Linked Target');
             INSERT INTO onboarding_person_account_link (person_id, account_id, linked_by)
             SELECT person_id, 42, 'test' FROM person WHERE person_key = 'synthetic-linked';",
        )
        .expect("seed people");

    // The unaccented query matches the accented display name.
    let people = search_nomination_people(&mut client, "synthetic rene", 10).expect("search");
    assert!(people.iter().any(|p| p.person_key == "synthetic-unlinked"));

    let keys = vec![
        "synthetic-unlinked".to_string(),
        "synthetic-linked".to_string(),
    ];
    let linked = linked_person_keys(&mut client, &keys).expect("linked keys");
    assert_eq!(linked.into_iter().collect::<Vec<_>>(), ["synthetic-linked"]);
}
