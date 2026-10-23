//! FAM-RAG-03 integration tests for the server-side browser session store.
//!
//! Synthetic identities only. The suite uses a disposable RAG database, an
//! in-memory fake refresh provider, and a fake token validator, so it is
//! deterministic and needs no identity provider.

use std::fs;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use postgres::{Client, NoTls};
use rag_foundation::browser_session::{
    self, NewSession, RefreshFailure, RefreshGrant, RefreshProvider, RenewOutcome, SessionConfig,
};
use rag_foundation::config::Config;
use rag_foundation::identity::{Role, VerifiedIdentity};
use rag_foundation::oidc::AuthError;

const MIGRATIONS: [&str; 11] = [
    "migrations/001_init.sql",
    "migrations/002_ingestion_metadata.sql",
    "migrations/003_add_language.sql",
    "migrations/004_page.sql",
    "migrations/005_add_corpus.sql",
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
    "migrations/009_document_lifecycle.sql",
    "migrations/010_document_security_hardening.sql",
    "migrations/011_browser_sessions.sql",
];

const ISSUER: &str = "https://issuer.test";
const SUBJECT: &str = "subject-1";

static DB_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn lock() -> MutexGuard<'static, ()> {
    DB_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn dsn() -> Option<String> {
    let dsn = std::env::var("DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    if dsn.is_empty() {
        return None;
    }
    let config = postgres::Config::from_str(&dsn).expect("DATABASE_URL parses");
    let database = config.get_dbname().unwrap_or_default();
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable RAG database {database:?}"
    );
    Some(dsn)
}

fn b64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }
    out
}

/// Builds a token the validator and `token_exp` can read. The signature is
/// irrelevant because the validator is a fake.
fn fake_token(exp: i64) -> String {
    let header = b64url(br#"{"alg":"none","typ":"JWT"}"#);
    let payload =
        b64url(format!(r#"{{"iss":"{ISSUER}","sub":"{SUBJECT}","exp":{exp}}}"#).as_bytes());
    format!("{header}.{payload}.signature")
}

fn test_config(dsn: &str, idle: i64, absolute: i64, window: i64) -> Config {
    let key = b64url(&[7u8; 32]);
    Config::load_from(|name| match name {
        "DATABASE_URL" => Some(dsn.to_string()),
        "SESSION_STORE_KEY" => Some(key.clone()),
        "WEB_SESSION_SECRET" => Some("test-secret".to_string()),
        "SESSION_IDLE_SECONDS" => Some(idle.to_string()),
        "SESSION_ABSOLUTE_SECONDS" => Some(absolute.to_string()),
        "SESSION_REFRESH_WINDOW_SECONDS" => Some(window.to_string()),
        "OIDC_TOKEN_ENDPOINT" => Some("http://127.0.0.1:9/token".to_string()),
        "OIDC_CLIENT_ID" => Some("test-client".to_string()),
        _ => None,
    })
    .expect("test configuration")
}

fn identity() -> VerifiedIdentity {
    VerifiedIdentity::new(
        SUBJECT,
        ISSUER,
        vec![Role::Admin],
        vec!["family-docs".to_string()],
    )
}

fn prepare(client: &mut Client) {
    for path in MIGRATIONS {
        let sql = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|error| panic!("apply {path}: {error}"));
    }
    client
        .batch_execute(
            "TRUNCATE browser_session, audit_events, corpus_grants, role_assignments, users \
             RESTART IDENTITY CASCADE",
        )
        .expect("reset session tables");
    client
        .execute(
            "INSERT INTO users (subject, issuer) VALUES ($1, $2)",
            &[&SUBJECT, &ISSUER],
        )
        .expect("insert user");
}

fn validator() -> impl Fn(&str) -> Result<VerifiedIdentity, AuthError> {
    |_token| Ok(identity())
}

#[derive(Clone)]
enum FakeMode {
    Grant {
        exp: i64,
    },
    InvalidGrant,
    Unavailable,
    Blocking {
        exp: i64,
        delay: Duration,
    },
    BlockingInvalidGrant {
        started: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    },
}

struct FakeProvider {
    calls: Arc<AtomicUsize>,
    mode: Mutex<FakeMode>,
}

impl FakeProvider {
    fn new(mode: FakeMode) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            mode: Mutex::new(mode),
        }
    }

    fn set(&self, mode: FakeMode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl RefreshProvider for FakeProvider {
    fn refresh(&self, _refresh_token: &str) -> Result<RefreshGrant, RefreshFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mode = self.mode.lock().unwrap().clone();
        match mode {
            FakeMode::Grant { exp } => Ok(RefreshGrant {
                id_token: fake_token(exp),
                refresh_token: Some("rotated-refresh".to_string()),
            }),
            FakeMode::InvalidGrant => Err(RefreshFailure::InvalidGrant),
            FakeMode::Unavailable => Err(RefreshFailure::Unavailable),
            FakeMode::Blocking { exp, delay } => {
                std::thread::sleep(delay);
                Ok(RefreshGrant {
                    id_token: fake_token(exp),
                    refresh_token: Some("rotated-refresh".to_string()),
                })
            }
            FakeMode::BlockingInvalidGrant { started, release } => {
                started.wait();
                release.wait();
                Err(RefreshFailure::InvalidGrant)
            }
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}

fn create(client: &mut Client, config: &SessionConfig, session_id: &str, exp: i64) -> String {
    let token = fake_token(exp);
    browser_session::create_session(
        client,
        config,
        NewSession {
            session_id,
            identity: &identity(),
            id_token: &token,
            id_token_exp: exp,
            refresh_token: "refresh-token-plaintext",
        },
    )
    .expect("create session");
    token
}

fn expire(client: &mut Client, session_id: &str, column: &str) {
    client
        .execute(
            &format!(
                "UPDATE browser_session SET {column} = now() - interval '1 second' \
                  WHERE session_id_hash = $1"
            ),
            &[&browser_session::session_hash(session_id)],
        )
        .expect("expire session");
}

#[test]
fn fresh_token_is_returned_and_touched_without_a_provider_call() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping browser session test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    prepare(&mut client);
    let config = test_config(&dsn, 3600, 7200, 60);
    let sessions = SessionConfig::from_config(&config);
    assert!(sessions.enabled());
    let token = create(&mut client, &sessions, "session-fresh", now() + 3600);
    let provider = FakeProvider::new(FakeMode::Grant { exp: now() + 3600 });
    let validate = validator();
    let outcome = browser_session::renew_or_touch(
        &mut client,
        &sessions,
        &provider,
        &validate,
        "session-fresh",
    )
    .expect("renew");
    match outcome {
        RenewOutcome::Current { id_token, .. } => assert_eq!(id_token, token),
        other => panic!("expected Current, got {other:?}"),
    }
    assert_eq!(provider.calls(), 0, "a fresh token must not refresh");
}

#[test]
fn idle_and_absolute_limits_expire_the_session() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping browser session test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    prepare(&mut client);
    let config = test_config(&dsn, 1800, 3600, 60);
    let sessions = SessionConfig::from_config(&config);
    let provider = FakeProvider::new(FakeMode::Grant { exp: now() + 3600 });
    let validate = validator();

    create(&mut client, &sessions, "session-idle", now() + 3600);
    expire(&mut client, "session-idle", "idle_expires_at");
    let outcome = browser_session::renew_or_touch(
        &mut client,
        &sessions,
        &provider,
        &validate,
        "session-idle",
    )
    .expect("renew");
    assert_eq!(outcome, RenewOutcome::Expired, "idle limit");

    create(&mut client, &sessions, "session-absolute", now() + 3600);
    expire(&mut client, "session-absolute", "absolute_expires_at");
    let outcome = browser_session::renew_or_touch(
        &mut client,
        &sessions,
        &provider,
        &validate,
        "session-absolute",
    )
    .expect("renew");
    assert_eq!(outcome, RenewOutcome::Expired, "absolute limit");
    assert_eq!(provider.calls(), 0);
}

#[test]
fn renewal_rotates_the_refresh_token_and_keeps_credentials_encrypted() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping browser session test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    prepare(&mut client);
    let config = test_config(&dsn, 1800, 3600, 60);
    let sessions = SessionConfig::from_config(&config);
    let new_exp = now() + 1800;
    let provider = FakeProvider::new(FakeMode::Grant { exp: new_exp });
    let validate = validator();

    create(&mut client, &sessions, "session-rotate", now() + 10);
    let outcome = browser_session::renew_or_touch(
        &mut client,
        &sessions,
        &provider,
        &validate,
        "session-rotate",
    )
    .expect("renew");
    match outcome {
        RenewOutcome::Renewed {
            id_token,
            expires_in,
            ..
        } => {
            assert_eq!(id_token, fake_token(new_exp));
            assert!(expires_in > 0);
        }
        other => panic!("expected Renewed, got {other:?}"),
    }
    assert_eq!(provider.calls(), 1);

    // The plaintext refresh token is never stored.
    let row = client
        .query_one(
            "SELECT refresh_token, id_token FROM browser_session WHERE session_id_hash = $1",
            &[&browser_session::session_hash("session-rotate")],
        )
        .expect("read row");
    let refresh: Vec<u8> = row.get(0);
    let id_token: Vec<u8> = row.get(1);
    assert!(!refresh
        .windows(b"refresh-token-plaintext".len())
        .any(|window| window == b"refresh-token-plaintext"));
    assert!(
        !id_token
            .windows(b"eyJ".len())
            .any(|window| window == b"eyJ"),
        "the stored ID token must be ciphertext"
    );

    // The rotated token is now fresh, so a second call uses the store without
    // contacting the provider again.
    let again = browser_session::renew_or_touch(
        &mut client,
        &sessions,
        &provider,
        &validate,
        "session-rotate",
    )
    .expect("renew");
    assert!(matches!(
        again,
        RenewOutcome::Current { .. } | RenewOutcome::Renewed { .. }
    ));
    assert_eq!(provider.calls(), 1);
}

#[test]
fn concurrent_renewal_calls_the_provider_once() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping browser session test");
        return;
    };
    let _guard = lock();
    let mut setup = Client::connect(&dsn, NoTls).expect("setup connect");
    prepare(&mut setup);
    let config = test_config(&dsn, 1800, 3600, 60);
    let sessions = SessionConfig::from_config(&config);
    create(&mut setup, &sessions, "session-concurrent", now() + 10);

    let exp = now() + 1800;
    let provider = Arc::new(FakeProvider::new(FakeMode::Blocking {
        exp,
        delay: Duration::from_millis(400),
    }));
    let sessions = Arc::new(sessions);

    let start = Arc::new(std::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for index in 0..2 {
        let dsn = dsn.clone();
        let sessions = Arc::clone(&sessions);
        let provider = Arc::clone(&provider);
        let start = Arc::clone(&start);
        handles.push(std::thread::spawn(move || {
            let mut client = Client::connect(&dsn, NoTls).expect("connect");
            let validate = validator();
            start.wait();
            let _ = index;
            browser_session::renew_or_touch(
                &mut client,
                &sessions,
                provider.as_ref(),
                &validate,
                "session-concurrent",
            )
            .expect("renew")
        }));
    }
    let outcomes: Vec<RenewOutcome> = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread"))
        .collect();
    assert_eq!(provider.calls(), 1, "one refresh for two callers");
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, RenewOutcome::Renewed { .. }))
            .count(),
        1
    );
    assert!(outcomes.iter().all(|outcome| matches!(
        outcome,
        RenewOutcome::Renewed { .. } | RenewOutcome::Busy { .. } | RenewOutcome::Current { .. }
    )));
}

#[test]
fn revoked_invalid_and_inactive_sessions_end_in_expiry() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping browser session test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    prepare(&mut client);
    let config = test_config(&dsn, 1800, 3600, 60);
    let sessions = SessionConfig::from_config(&config);
    let validate = validator();

    // Sign-out revokes the stored session.
    create(&mut client, &sessions, "session-revoked", now() + 10);
    browser_session::revoke_session(&mut client, &sessions, "session-revoked").expect("revoke");
    let provider = FakeProvider::new(FakeMode::Grant { exp: now() + 1800 });
    assert_eq!(
        browser_session::renew_or_touch(
            &mut client,
            &sessions,
            &provider,
            &validate,
            "session-revoked"
        )
        .expect("renew"),
        RenewOutcome::Expired
    );

    // A provider that rejects the refresh grant revokes the session.
    create(&mut client, &sessions, "session-invalid", now() + 10);
    let invalid = FakeProvider::new(FakeMode::InvalidGrant);
    assert_eq!(
        browser_session::renew_or_touch(
            &mut client,
            &sessions,
            &invalid,
            &validate,
            "session-invalid"
        )
        .expect("renew"),
        RenewOutcome::Expired
    );
    assert!(client
        .query_one(
            "SELECT revoked_at IS NOT NULL FROM browser_session WHERE session_id_hash = $1",
            &[&browser_session::session_hash("session-invalid")],
        )
        .expect("row")
        .get::<_, bool>(0));

    // A deactivated user cannot renew even with a valid grant.
    create(&mut client, &sessions, "session-inactive", now() + 10);
    client
        .execute(
            "UPDATE users SET deactivated_at = now() WHERE issuer = $1 AND subject = $2",
            &[&ISSUER, &SUBJECT],
        )
        .expect("deactivate");
    let provider = FakeProvider::new(FakeMode::Grant { exp: now() + 1800 });
    assert_eq!(
        browser_session::renew_or_touch(
            &mut client,
            &sessions,
            &provider,
            &validate,
            "session-inactive"
        )
        .expect("renew"),
        RenewOutcome::Expired
    );
}

#[test]
fn transient_provider_failure_keeps_the_session_renewable() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping browser session test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    prepare(&mut client);
    let config = test_config(&dsn, 1800, 3600, 60);
    let sessions = SessionConfig::from_config(&config);
    let validate = validator();

    create(&mut client, &sessions, "session-outage", now() + 10);
    let provider = FakeProvider::new(FakeMode::Unavailable);
    assert_eq!(
        browser_session::renew_or_touch(
            &mut client,
            &sessions,
            &provider,
            &validate,
            "session-outage"
        )
        .expect("renew"),
        RenewOutcome::Unavailable
    );
    let revoked: bool = client
        .query_one(
            "SELECT revoked_at IS NOT NULL FROM browser_session WHERE session_id_hash = $1",
            &[&browser_session::session_hash("session-outage")],
        )
        .expect("row")
        .get(0);
    assert!(!revoked, "a transient failure must not revoke the session");

    provider.set(FakeMode::Grant { exp: now() + 1800 });
    assert!(matches!(
        browser_session::renew_or_touch(
            &mut client,
            &sessions,
            &provider,
            &validate,
            "session-outage"
        )
        .expect("renew"),
        RenewOutcome::Renewed { .. }
    ));
}

#[test]
fn cleanup_removes_dead_sessions() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping browser session test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    prepare(&mut client);
    let config = test_config(&dsn, 1800, 3600, 60);
    let sessions = SessionConfig::from_config(&config);
    create(&mut client, &sessions, "session-old", now() + 3600);
    client
        .execute(
            "UPDATE browser_session SET absolute_expires_at = now() - interval '30 days'",
            &[],
        )
        .expect("age session");
    let removed = browser_session::cleanup_expired(&mut client).expect("cleanup");
    assert_eq!(removed, 1);
}

#[test]
fn a_stale_invalid_grant_does_not_revoke_a_newer_generation() {
    let Some(dsn) = dsn() else {
        eprintln!("DATABASE_URL unset; skipping browser session test");
        return;
    };
    let _guard = lock();
    let mut client = Client::connect(&dsn, NoTls).expect("connect");
    prepare(&mut client);
    let config = test_config(&dsn, 1800, 3600, 60);
    let sessions = SessionConfig::from_config(&config);
    create(&mut client, &sessions, "session-stale", now() + 10);

    let started = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let stale_provider = Arc::new(FakeProvider::new(FakeMode::BlockingInvalidGrant {
        started: Arc::clone(&started),
        release: Arc::clone(&release),
    }));
    let worker_dsn = dsn.clone();
    let worker_sessions = sessions.clone();
    let handle = std::thread::spawn(move || {
        let mut worker = Client::connect(&worker_dsn, NoTls).expect("worker connect");
        let validate = validator();
        stale_provider.calls(); // keep the counter alive in this closure
        browser_session::renew_or_touch(
            &mut worker,
            &worker_sessions,
            stale_provider.as_ref(),
            &validate,
            "session-stale",
        )
        .expect("stale renew")
    });

    // The stale worker has claimed the lease and is inside the provider.
    started.wait();
    client
        .execute(
            "UPDATE browser_session SET renewal_lease_until = now() - interval '1 second'
              WHERE session_id_hash = $1",
            &[&browser_session::session_hash("session-stale")],
        )
        .expect("expire stale lease");

    // A newer generation renews and stores its token.
    let new_exp = now() + 1800;
    let fresh_provider = FakeProvider::new(FakeMode::Grant { exp: new_exp });
    let validate = validator();
    let fresh = browser_session::renew_or_touch(
        &mut client,
        &sessions,
        &fresh_provider,
        &validate,
        "session-stale",
    )
    .expect("fresh renew");
    assert!(matches!(fresh, RenewOutcome::Renewed { .. }));

    // The stale worker's invalid_grant must not revoke the newer generation.
    release.wait();
    let stale = handle.join().expect("stale thread");
    assert_eq!(
        stale,
        RenewOutcome::Busy {
            retry_after_ms: 250
        }
    );

    let row = client
        .query_one(
            "SELECT revoked_at IS NULL,
                    EXTRACT(EPOCH FROM id_token_exp)::bigint,
                    refresh_generation
               FROM browser_session WHERE session_id_hash = $1",
            &[&browser_session::session_hash("session-stale")],
        )
        .expect("read session");
    assert!(
        row.get::<_, bool>(0),
        "the newer generation must stay active"
    );
    assert_eq!(row.get::<_, i64>(1), new_exp, "the newer token is stored");
    assert_eq!(row.get::<_, i64>(2), 2);
}
