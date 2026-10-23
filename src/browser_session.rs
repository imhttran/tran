//! Server-side browser sessions, refresh-token storage, and renewal.
//!
//! The browser holds only an opaque, HttpOnly `rag_sid` cookie. The ID token
//! and the OIDC refresh token live in `browser_session`, encrypted with
//! AES-256-GCM under `SESSION_STORE_KEY`. The Rust API is the only component
//! that reads or writes them.
//!
//! Renewal is coordinated with a generation number and a short lease: one
//! claimant refreshes, and only that generation may write the rotated
//! credentials. A second worker receives [`RenewOutcome::Busy`] instead of
//! racing the provider or overwriting newer tokens.
//!
//! Idle and absolute deadlines are enforced here, not by the browser. Revoked,
//! expired, deactivated, and provider-rejected sessions end in
//! [`RenewOutcome::Expired`] and are never revived.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use thiserror::Error;

use crate::identity::VerifiedIdentity;
use crate::oidc::AuthError;

/// How long a claimed renewal may run before another worker may take over.
/// The HTTP provider timeout is 10 seconds with three bounded attempts plus
/// short backoff (about 31 seconds worst case), so the lease covers the whole
/// provider policy.
const RENEWAL_LEASE_SECONDS: i64 = 45;
/// Expired and revoked rows are kept briefly for audit, then removed.
const RETENTION_DAYS: i64 = 7;
/// Upper bound on rows removed by one opportunistic cleanup.
const CLEANUP_BATCH: i64 = 1000;

/// A browser-session store failure. No variant carries a token.
#[derive(Debug, Error)]
pub enum SessionError {
    /// Renewal configuration is absent; callers must fall back to the
    /// cookie-only session from before automatic renewal.
    #[error("browser session renewal is not configured")]
    NotConfigured,
    #[error("the browser session already exists")]
    AlreadyExists,
    #[error("the browser session was not found")]
    NotFound,
    #[error("browser session credentials could not be encrypted or decrypted")]
    Crypto,
    #[error("browser session database operation failed: {0}")]
    Database(String),
}

impl From<postgres::Error> for SessionError {
    fn from(err: postgres::Error) -> Self {
        SessionError::Database(err.to_string())
    }
}

/// Settings for the server-side session store.
#[derive(Clone)]
pub struct SessionConfig {
    key: Option<Arc<LessSafeKey>>,
    idle_seconds: i64,
    absolute_seconds: i64,
    refresh_window_seconds: i64,
    token_endpoint: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    revocation_endpoint: Option<String>,
}

impl SessionConfig {
    /// Builds the store settings from the loaded configuration.
    pub fn from_config(config: &crate::config::Config) -> Self {
        let key = config
            .session_store_key
            .as_deref()
            .and_then(|bytes| UnboundKey::new(&AES_256_GCM, bytes).ok())
            .map(LessSafeKey::new)
            .map(Arc::new);
        Self {
            key,
            idle_seconds: config.session_idle_seconds,
            absolute_seconds: config.session_absolute_seconds,
            refresh_window_seconds: config.session_refresh_window_seconds,
            token_endpoint: config.oidc_token_endpoint.clone(),
            client_id: config.oidc_client_id.clone(),
            client_secret: config.oidc_client_secret.clone(),
            revocation_endpoint: config.oidc_revocation_endpoint.clone(),
        }
    }

    /// True when the key and refresh configuration are present.
    pub fn enabled(&self) -> bool {
        self.key.is_some() && self.token_endpoint.is_some() && self.client_id.is_some()
    }

    /// Absolute session limit in seconds.
    pub fn absolute_seconds(&self) -> i64 {
        self.absolute_seconds
    }
}

/// The identity and credentials recorded when a browser signs in.
pub struct NewSession<'a> {
    pub session_id: &'a str,
    pub identity: &'a VerifiedIdentity,
    pub id_token: &'a str,
    pub id_token_exp: i64,
    pub refresh_token: &'a str,
}

/// The result of creating a stored session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedSession {
    pub absolute_expires_in: i64,
    pub idle_expires_in: i64,
}

/// The result of a renewal attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenewOutcome {
    /// The stored ID token is still comfortably valid; the idle deadline moved.
    Current {
        id_token: String,
        expires_in: i64,
        absolute_expires_in: i64,
    },
    /// A new ID token was obtained and stored.
    Renewed {
        id_token: String,
        expires_in: i64,
        absolute_expires_in: i64,
    },
    /// The session is revoked, idle, absolute-expired, or its refresh grant was
    /// rejected. The caller must clear the browser cookies.
    Expired,
    /// Another worker holds the renewal lease. Retry shortly.
    Busy { retry_after_ms: u64 },
    /// The provider could not be reached or returned an unusable response. The
    /// session is kept so a transient failure does not sign the user out.
    Unavailable,
}

/// A successful refresh grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshGrant {
    pub id_token: String,
    pub refresh_token: Option<String>,
}

/// Why a refresh grant failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshFailure {
    /// The provider rejected the refresh token (`invalid_grant`).
    InvalidGrant,
    /// A transport, `429`, or `5xx` failure. Retrying later may succeed.
    Unavailable,
}

/// Obtains a new token pair from the identity provider.
pub trait RefreshProvider {
    fn refresh(&self, refresh_token: &str) -> Result<RefreshGrant, RefreshFailure>;
}

/// HTTP implementation of the refresh grant, with bounded retries for
/// transient failures. It never logs a request or response body.
pub struct HttpRefreshProvider {
    endpoint: String,
    client_id: String,
    client_secret: Option<String>,
    http: reqwest::blocking::Client,
}

impl HttpRefreshProvider {
    /// Builds the provider from the session configuration. `None` when the
    /// token endpoint or client id is missing.
    pub fn new(config: &SessionConfig) -> Option<Self> {
        let endpoint = config.token_endpoint.clone()?;
        let client_id = config.client_id.clone()?;
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;
        Some(Self {
            endpoint,
            client_id,
            client_secret: config.client_secret.clone(),
            http,
        })
    }
}

impl RefreshProvider for HttpRefreshProvider {
    fn refresh(&self, refresh_token: &str) -> Result<RefreshGrant, RefreshFailure> {
        for attempt in 0..3u32 {
            let mut form = vec![
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", self.client_id.as_str()),
            ];
            if let Some(secret) = self.client_secret.as_deref() {
                form.push(("client_secret", secret));
            }
            if let Ok(response) = self.http.post(&self.endpoint).form(&form).send() {
                let status = response.status();
                if status.is_success() {
                    let body = response.text().map_err(|_| RefreshFailure::Unavailable)?;
                    let payload: serde_json::Value =
                        serde_json::from_str(&body).map_err(|_| RefreshFailure::Unavailable)?;
                    let id_token = payload
                        .get("id_token")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned)
                        .ok_or(RefreshFailure::Unavailable)?;
                    let refresh_token = payload
                        .get("refresh_token")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned);
                    return Ok(RefreshGrant {
                        id_token,
                        refresh_token,
                    });
                }
                if status.as_u16() == 400 || status.as_u16() == 401 {
                    // Only `invalid_grant` means the refresh credential is dead.
                    // `invalid_client` or another 4xx is a configuration error;
                    // treating it as invalid_grant would sign every session out.
                    let body = response.text().unwrap_or_default();
                    let payload: serde_json::Value =
                        serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                    let error = payload.get("error").and_then(|value| value.as_str());
                    if status.as_u16() == 400 && error == Some("invalid_grant") {
                        return Err(RefreshFailure::InvalidGrant);
                    }
                    return Err(RefreshFailure::Unavailable);
                }
                // 429 and 5xx are transient.
            }
            if attempt < 2 {
                let delay = if attempt == 0 { 100 } else { 300 };
                std::thread::sleep(Duration::from_millis(delay));
            }
        }
        Err(RefreshFailure::Unavailable)
    }
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}

/// Hashes the opaque cookie value. Only the hash is stored.
pub fn session_hash(session_id: &str) -> String {
    digest(&SHA256, session_id.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn encrypt(key: &LessSafeKey, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, SessionError> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| SessionError::Crypto)?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut in_out = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::from(aad), &mut in_out)
        .map_err(|_| SessionError::Crypto)?;
    let mut out = Vec::with_capacity(NONCE_LEN + in_out.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&in_out);
    Ok(out)
}

fn decrypt(key: &LessSafeKey, aad: &[u8], blob: &[u8]) -> Result<Vec<u8>, SessionError> {
    if blob.len() <= NONCE_LEN {
        return Err(SessionError::Crypto);
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce_bytes).map_err(|_| SessionError::Crypto)?;
    let mut in_out = ciphertext.to_vec();
    let plaintext = key
        .open_in_place(nonce, Aad::from(aad), &mut in_out)
        .map_err(|_| SessionError::Crypto)?;
    Ok(plaintext.to_vec())
}

fn clear_lease(client: &mut Client, hash: &str, generation: i64) -> Result<(), SessionError> {
    client.execute(
        "UPDATE browser_session
            SET renewal_lease_until = NULL
          WHERE session_id_hash = $1 AND refresh_generation = $2",
        &[&hash, &generation],
    )?;
    Ok(())
}

fn revoke_hash(client: &mut Client, hash: &str) -> Result<(), SessionError> {
    client.execute(
        "UPDATE browser_session
            SET revoked_at = now(), renewal_lease_until = NULL,
                id_token = ''::bytea, refresh_token = ''::bytea, last_error = NULL
          WHERE session_id_hash = $1 AND revoked_at IS NULL",
        &[&hash],
    )?;
    Ok(())
}

/// Revokes only while `generation` still owns the session. A stale claim whose
/// lease expired must not revoke a newer generation that already renewed.
fn revoke_hash_generation(
    client: &mut Client,
    hash: &str,
    generation: i64,
) -> Result<bool, SessionError> {
    let updated = client.execute(
        "UPDATE browser_session
            SET revoked_at = now(), renewal_lease_until = NULL,
                id_token = ''::bytea, refresh_token = ''::bytea, last_error = NULL
          WHERE session_id_hash = $1 AND refresh_generation = $2
            AND revoked_at IS NULL",
        &[&hash, &generation],
    )?;
    Ok(updated > 0)
}

/// Records a signed-in browser session.
pub fn create_session(
    client: &mut Client,
    config: &SessionConfig,
    new: NewSession<'_>,
) -> Result<CreatedSession, SessionError> {
    let key = config.key.as_ref().ok_or(SessionError::NotConfigured)?;
    if new.session_id.trim().is_empty()
        || new.id_token.trim().is_empty()
        || new.refresh_token.trim().is_empty()
    {
        return Err(SessionError::Crypto);
    }
    // Bind the session to the token's own claims. A caller cannot record a
    // session for a different account than the credential it presents.
    let (issuer, subject, token_exp) =
        crate::oidc::token_claims(new.id_token).ok_or(SessionError::Crypto)?;
    if issuer != new.identity.issuer()
        || subject != new.identity.subject()
        || token_exp != new.id_token_exp
    {
        return Err(SessionError::Crypto);
    }
    if token_exp <= now_seconds() {
        return Err(SessionError::Crypto);
    }
    let hash = session_hash(new.session_id);
    let aad = hash.as_bytes();
    let id_token = encrypt(key, aad, new.id_token.as_bytes())?;
    let refresh_token = encrypt(key, aad, new.refresh_token.as_bytes())?;
    let row = client.query_opt(
        "INSERT INTO browser_session
                 (session_id_hash, issuer, subject, id_token, id_token_exp,
                  refresh_token, idle_expires_at, absolute_expires_at)
             VALUES ($1, $2, $3, $4, to_timestamp($5), $6,
                     now() + make_interval(secs => $7),
                     now() + make_interval(secs => $8))
             ON CONFLICT (session_id_hash) DO NOTHING
             RETURNING EXTRACT(EPOCH FROM (absolute_expires_at - now()))::bigint,
                       EXTRACT(EPOCH FROM (idle_expires_at - now()))::bigint",
        &[
            &hash,
            &new.identity.issuer(),
            &new.identity.subject(),
            &id_token,
            &(new.id_token_exp as f64),
            &refresh_token,
            &(config.idle_seconds as f64),
            &(config.absolute_seconds as f64),
        ],
    )?;
    let Some(row) = row else {
        return Err(SessionError::AlreadyExists);
    };
    let _ = cleanup_expired(client);
    Ok(CreatedSession {
        absolute_expires_in: row.get(0),
        idle_expires_in: row.get(1),
    })
}

/// Renews, or simply touches, an active session.
///
/// The identity provider is called only when the stored ID token is inside the
/// refresh window. An active session with a fresh token only has its idle
/// deadline moved. The provider call is made outside every database
/// transaction, so no lock is held across the network.
pub fn renew_or_touch(
    client: &mut Client,
    config: &SessionConfig,
    provider: &dyn RefreshProvider,
    validate: &dyn Fn(&str) -> Result<VerifiedIdentity, AuthError>,
    session_id: &str,
) -> Result<RenewOutcome, SessionError> {
    let key = config.key.as_ref().ok_or(SessionError::NotConfigured)?;
    let hash = session_hash(session_id);
    let aad = hash.as_bytes();

    let mut tx = client.transaction()?;
    let row = tx.query_opt(
        "SELECT issuer, subject, id_token, refresh_token, refresh_generation,
                (renewal_lease_until IS NOT NULL AND renewal_lease_until > now()) AS leased,
                EXTRACT(EPOCH FROM (id_token_exp - now()))::bigint AS expires_in,
                EXTRACT(EPOCH FROM (absolute_expires_at - now()))::bigint AS absolute_in,
                (revoked_at IS NOT NULL OR idle_expires_at <= now()
                 OR absolute_expires_at <= now()) AS expired
           FROM browser_session
          WHERE session_id_hash = $1
          FOR UPDATE",
        &[&hash],
    )?;
    let Some(row) = row else {
        tx.rollback()?;
        return Ok(RenewOutcome::Expired);
    };
    if row.get::<_, bool>("expired") {
        tx.rollback()?;
        return Ok(RenewOutcome::Expired);
    }
    let expires_in: i64 = row.get("expires_in");
    let absolute_in: i64 = row.get("absolute_in");
    if expires_in > config.refresh_window_seconds {
        tx.execute(
            "UPDATE browser_session
                SET last_seen_at = now(),
                    idle_expires_at = now() + make_interval(secs => $2)
              WHERE session_id_hash = $1",
            &[&hash, &(config.idle_seconds as f64)],
        )?;
        let id_token = decrypt(key, aad, &row.get::<_, Vec<u8>>("id_token"))?;
        let id_token = String::from_utf8(id_token).map_err(|_| SessionError::Crypto)?;
        tx.commit()?;
        return Ok(RenewOutcome::Current {
            id_token,
            expires_in,
            absolute_expires_in: absolute_in,
        });
    }
    if row.get::<_, bool>("leased") {
        tx.rollback()?;
        return Ok(RenewOutcome::Busy {
            retry_after_ms: 250,
        });
    }
    let generation: i64 = row.get("refresh_generation");
    let new_generation = generation + 1;
    tx.execute(
        "UPDATE browser_session
            SET renewal_lease_until = now() + make_interval(secs => $2),
                refresh_generation = $3
          WHERE session_id_hash = $1",
        &[&hash, &(RENEWAL_LEASE_SECONDS as f64), &new_generation],
    )?;
    let issuer: String = row.get("issuer");
    let subject: String = row.get("subject");
    let stored_refresh = decrypt(key, aad, &row.get::<_, Vec<u8>>("refresh_token"))?;
    let stored_refresh = String::from_utf8(stored_refresh).map_err(|_| SessionError::Crypto)?;
    tx.commit()?;

    let grant = match provider.refresh(&stored_refresh) {
        Ok(grant) => grant,
        Err(RefreshFailure::InvalidGrant) => {
            // Generation-safe: a stale claim whose lease expired while the
            // provider was slow must not revoke a newer generation.
            if revoke_hash_generation(client, &hash, new_generation)? {
                return Ok(RenewOutcome::Expired);
            }
            return Ok(RenewOutcome::Busy {
                retry_after_ms: 250,
            });
        }
        Err(RefreshFailure::Unavailable) => {
            clear_lease(client, &hash, new_generation)?;
            return Ok(RenewOutcome::Unavailable);
        }
    };

    // The renewed credential must be genuine and must belong to the same
    // account. A different subject or issuer means the stored grant was swapped.
    let identity = match validate(&grant.id_token) {
        Ok(identity) => identity,
        Err(_) => {
            clear_lease(client, &hash, new_generation)?;
            return Ok(RenewOutcome::Unavailable);
        }
    };
    if identity.issuer() != issuer || identity.subject() != subject {
        if revoke_hash_generation(client, &hash, new_generation)? {
            return Ok(RenewOutcome::Expired);
        }
        return Ok(RenewOutcome::Busy {
            retry_after_ms: 250,
        });
    }
    let Some(id_token_exp) = crate::oidc::token_exp(&grant.id_token) else {
        clear_lease(client, &hash, new_generation)?;
        return Ok(RenewOutcome::Unavailable);
    };
    let rotated_refresh = grant.refresh_token.unwrap_or(stored_refresh);

    let id_token = encrypt(key, aad, grant.id_token.as_bytes())?;
    let refresh_token = encrypt(key, aad, rotated_refresh.as_bytes())?;
    let mut tx = client.transaction()?;
    let active = tx
        .query_opt(
            "SELECT 1 FROM users
              WHERE issuer = $1 AND subject = $2 AND deactivated_at IS NULL",
            &[&issuer, &subject],
        )?
        .is_some();
    if !active {
        tx.rollback()?;
        revoke_hash(client, &hash)?;
        return Ok(RenewOutcome::Expired);
    }
    let updated = tx.execute(
        "UPDATE browser_session
            SET id_token = $3,
                id_token_exp = to_timestamp($4),
                refresh_token = $5,
                renewal_lease_until = NULL,
                last_seen_at = now(),
                idle_expires_at = now() + make_interval(secs => $6),
                last_error = NULL
          WHERE session_id_hash = $1
            AND refresh_generation = $2
            AND renewal_lease_until IS NOT NULL
            AND idle_expires_at > now()
            AND absolute_expires_at > now()",
        &[
            &hash,
            &new_generation,
            &id_token,
            &(id_token_exp as f64),
            &refresh_token,
            &(config.idle_seconds as f64),
        ],
    )?;
    if updated == 0 {
        tx.rollback()?;
        // Distinguish a session that expired during the provider call from a
        // generation that another worker replaced.
        let expired = client
            .query_opt(
                "SELECT (revoked_at IS NOT NULL OR idle_expires_at <= now()
                         OR absolute_expires_at <= now())
                   FROM browser_session WHERE session_id_hash = $1",
                &[&hash],
            )?
            .map(|row| row.get::<_, bool>(0))
            .unwrap_or(true);
        if expired {
            revoke_hash(client, &hash)?;
            return Ok(RenewOutcome::Expired);
        }
        return Ok(RenewOutcome::Busy {
            retry_after_ms: 250,
        });
    }
    let absolute_in: i64 = tx
        .query_one(
            "SELECT EXTRACT(EPOCH FROM (absolute_expires_at - now()))::bigint
               FROM browser_session WHERE session_id_hash = $1",
            &[&hash],
        )?
        .get(0);
    tx.commit()?;
    Ok(RenewOutcome::Renewed {
        id_token: grant.id_token,
        expires_in: (id_token_exp - now_seconds()).max(0),
        absolute_expires_in: absolute_in,
    })
}

/// Revokes a session and asks the provider to revoke its refresh token when a
/// revocation endpoint is configured. Provider failures are ignored: the local
/// session is already unusable.
pub fn revoke_session(
    client: &mut Client,
    config: &SessionConfig,
    session_id: &str,
) -> Result<(), SessionError> {
    let hash = session_hash(session_id);
    // Read the plaintext refresh token before clearing the row; the provider
    // revocation is best-effort and must not delay sign-out.
    let token = match config.key.as_ref() {
        Some(key) => client
            .query_opt(
                "SELECT refresh_token FROM browser_session
                  WHERE session_id_hash = $1 AND revoked_at IS NULL",
                &[&hash],
            )?
            .and_then(|row| decrypt(key, hash.as_bytes(), &row.get::<_, Vec<u8>>(0)).ok())
            .and_then(|plain| String::from_utf8(plain).ok()),
        None => None,
    };
    revoke_hash(client, &hash)?;
    if let (Some(token), Some(endpoint), Some(client_id)) = (
        token,
        config.revocation_endpoint.clone(),
        config.client_id.clone(),
    ) {
        // Fire-and-forget: the local session is already unusable, and a slow
        // provider must not hold the sign-out response open.
        std::thread::spawn(move || {
            let form = [
                ("token", token.as_str()),
                ("token_type_hint", "refresh_token"),
                ("client_id", client_id.as_str()),
            ];
            if let Ok(http) = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
            {
                let _ = http.post(endpoint).form(&form).send();
            }
        });
    }
    Ok(())
}

/// Removes expired and long-revoked rows in a bounded batch.
pub fn cleanup_expired(client: &mut Client) -> Result<u64, SessionError> {
    let removed = client.execute(
        "DELETE FROM browser_session
          WHERE ctid IN (
              SELECT ctid FROM browser_session
               WHERE absolute_expires_at < now() - make_interval(days => $1)
                  OR (revoked_at IS NOT NULL
                      AND revoked_at < now() - make_interval(days => $1))
               LIMIT $2
          )",
        &[&(RETENTION_DAYS as i32), &CLEANUP_BATCH],
    )?;
    Ok(removed)
}

/// Opens the identity database for a session operation. The caller runs inside
/// a blocking task, matching every other database-backed handler.
pub fn connect(config: &crate::config::Config) -> Result<Client, SessionError> {
    Client::connect(&config.database_url, NoTls)
        .map_err(|err| SessionError::Database(err.to_string()))
}
