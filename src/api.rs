//! Axum API over the shared Rust application services.
//!
//! The API is an adapter, not a second RAG implementation:
//!
//! * ingestion calls the same [`IngestService`]/[`IngestPipeline`] the CLI uses;
//! * query calls the same [`QueryService`] the CLI uses;
//! * retrieval, generation, and citation validation stay in the library.
//!
//! Phase 2 makes security fail-closed. Every route except `/api/health` runs
//! through [`crate::identity::decide`] against a [`VerifiedIdentity`] produced
//! only by [`crate::oidc::validate_token`]. The authorization step runs before
//! multipart staging, database access, embeddings, or model calls. There is no
//! configuration flag that disables enforcement.

use std::path::Path as FsPath;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::Config;
use crate::corpus::Corpus;
use crate::embedding::OllamaEmbedder;
use crate::fingerprint::{fingerprint, ChunkerConfig};
use crate::genealogy::{
    appointments, query as genealogy_query, resolve_genealogy_dsn, tree as genealogy_tree,
    GenealogyError,
};
use crate::identity::{self, Decision, Operation, Role, VerifiedIdentity};
use crate::identity_store::{IdentityStore, IdentityStoreError};
use crate::ingest::IngestRejection;
use crate::oidc::{validate_token, AuthConfig, AuthError, Jwks, JwksCache};
use crate::ollama::{HttpTransport, OllamaClient, OllamaError};
use crate::onboarding::{
    self, link, AccountMapError, CandidatePreview, LinkDecision, LinkError, PreviewSummary,
    ProfileError,
};
use crate::pipeline::{IngestOutcome, PipelineConfig, PipelineError};
use crate::query_service::QueryService;
use crate::ratelimit::RateLimiter;
use crate::service::IngestService;
use crate::storage_pg::{PgStore, PgStoreError};

/// Default loopback host. The API never binds a public interface by default.
pub const DEFAULT_HOST: &str = "127.0.0.1";
/// Default API port.
pub const DEFAULT_PORT: u16 = 8080;
/// Maximum accepted upload size: 10 MiB, matching the UI contract.
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
/// Accepted upload extensions, matching the implemented loaders.
pub const ALLOWED_EXTENSIONS: &[&str] = &["md", "markdown", "txt", "pdf"];
/// The session cookie name set by the Next.js proxy.
pub const SESSION_COOKIE: &str = "rag_session";
/// The CSRF header name enforced by the Next.js proxy.
pub const CSRF_HEADER: &str = "x-csrf-token";
/// Reads (account, document status, genealogy) get this multiple of
/// `AUTH_RATE_LIMIT` per subject, since one page view issues several.
const READ_RATE_LIMIT_FACTOR: u32 = 5;

/// Errors raised while constructing or running the API server.
#[derive(Debug, Error)]
pub enum ApiError {
    /// The configured corpus identity is invalid.
    #[error(transparent)]
    Corpus(#[from] crate::corpus::CorpusError),
    /// The document store could not be built.
    #[error(transparent)]
    Store(#[from] PgStoreError),
    /// The listener could not bind.
    #[error("bind {address}: {source}")]
    Bind {
        /// The requested address.
        address: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The bound address could not be read.
    #[error("read the bound address: {0}")]
    Address(#[source] std::io::Error),
    /// The server stopped with an error.
    #[error("serve: {0}")]
    Serve(String),
}

/// Shared API state. Every field is immutable after startup.
#[derive(Clone)]
pub struct AppState {
    config: Arc<Config>,
    store: Arc<PgStore>,
    corpus: String,
    auth: Arc<AuthConfig>,
    jwks: Arc<JwksCache<JwksFetcher>>,
    limiter: Arc<RateLimiter>,
    read_limiter: Arc<RateLimiter>,
    /// The query service is built once, on the first query, inside a blocking
    /// task. Rebuilding it per request would create two HTTP clients and two
    /// connection pools each time.
    query_service: Arc<Mutex<Option<Arc<QueryService>>>>,
    /// One shared client for the health probe, so polling does not rebuild a
    /// connection pool each time.
    health_client: Arc<Mutex<Option<reqwest::blocking::Client>>>,
}

/// The JWKS fetcher type held by the cached verifier.
type JwksFetcher = Box<dyn Fn() -> Result<Jwks, AuthError> + Send + Sync>;

impl AppState {
    /// Builds the state from the loaded configuration.
    ///
    /// Only the database pool is created here. The Ollama clients are built per
    /// request inside a blocking task: `reqwest::blocking` must not be
    /// constructed on a Tokio runtime thread.
    pub fn new(config: Config, corpus: impl Into<String>) -> Result<Self, ApiError> {
        let corpus: String = corpus.into();
        let corpus = Corpus::new(&corpus)?;
        let store = Arc::new(PgStore::connect(&config.database_url)?);
        let auth = AuthConfig::new(
            config.auth_issuer.clone(),
            config.auth_audience.clone(),
            Duration::from_secs(config.jwks_cache_ttl_secs as u64),
            Duration::from_secs(config.auth_clock_skew_secs as u64),
        );
        let jwks_url = config.jwks_url.clone();
        let fetcher: JwksFetcher = Box::new(move || fetch_jwks(&jwks_url));
        let jwks = Arc::new(JwksCache::new(
            fetcher,
            Duration::from_secs(config.jwks_cache_ttl_secs as u64),
        ));
        let limiter = Arc::new(RateLimiter::new(
            config.auth_rate_limit as u32,
            Duration::from_secs(60),
        ));
        let read_limiter = Arc::new(RateLimiter::new(
            (config.auth_rate_limit as u32).saturating_mul(READ_RATE_LIMIT_FACTOR),
            Duration::from_secs(60),
        ));
        Ok(Self {
            config: Arc::new(config),
            store,
            corpus: corpus.as_str().to_string(),
            auth: Arc::new(auth),
            jwks,
            limiter,
            read_limiter,
            query_service: Arc::new(Mutex::new(None)),
            health_client: Arc::new(Mutex::new(None)),
        })
    }

    /// Replaces the JWKS fetcher (tests only).
    #[doc(hidden)]
    pub fn with_jwks(
        mut self,
        fetcher: impl Fn() -> Result<Jwks, AuthError> + Send + Sync + 'static,
    ) -> Self {
        self.jwks = Arc::new(JwksCache::new(
            Box::new(fetcher) as JwksFetcher,
            Duration::from_secs(self.config.jwks_cache_ttl_secs as u64),
        ));
        self
    }

    /// Builds the API router.
    pub fn router(self) -> Router {
        Router::new()
            .route("/api/health", get(health))
            .route("/api/documents/ingest", post(ingest))
            .route("/api/documents/{source}/status", get(status))
            .route("/api/query", post(query))
            .route("/api/account", get(account))
            .route("/api/profile", get(profile))
            .route("/api/admin/roles", post(change_role))
            .route("/api/genealogy/records", get(genealogy_records))
            .route("/api/genealogy/tree", get(genealogy_tree))
            .route(
                "/api/genealogy/head-of-family",
                get(genealogy_head_of_family),
            )
            .route("/api/genealogy/person/{key}", get(genealogy_person))
            .route(
                "/api/genealogy/relationship/{from}/{to}",
                get(genealogy_relationship),
            )
            .route(
                "/api/genealogy/nominations",
                get(list_nominations).post(nominate_head_of_family),
            )
            .route("/api/genealogy/nominations/{id}", post(decide_nomination))
            .route("/api/genealogy/appointments/{id}", post(end_head_of_family))
            .route(
                "/api/admin/family-members/candidates",
                get(family_member_candidates),
            )
            .route(
                "/api/admin/family-members/invitations",
                post(create_invitation),
            )
            .route(
                "/api/admin/family-members/invitations/{id}/resend",
                post(resend_invitation),
            )
            .route(
                "/api/admin/family-members/invitations/{id}/revoke",
                post(revoke_invitation),
            )
            .route("/api/admin/family-members/links", post(create_link))
            .route(
                "/api/admin/family-members/links/{id}/revoke",
                post(revoke_link),
            )
            .route("/api/admin/users", post(provision_user))
            .route(
                "/api/admin/users/{subject}/deactivate",
                post(deactivate_user),
            )
            .route(
                "/api/admin/users/{issuer}/{subject}/deactivate",
                post(deactivate_user_scoped),
            )
            // Multipart framing adds a small overhead; the file payload itself
            // is checked against MAX_UPLOAD_BYTES in the handler.
            .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES + 1024 * 1024))
            .with_state(self)
    }
}

/// Fetches JWKS over HTTP. Any failure (network, status, parse) is an error.
fn fetch_jwks(url: &str) -> Result<Jwks, AuthError> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .and_then(|client| client.get(url).send())
        .map_err(|err| AuthError::ProviderUnavailable(err.to_string()))?;
    if !response.status().is_success() {
        return Err(AuthError::ProviderUnavailable(format!(
            "JWKS endpoint returned {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .map_err(|err| AuthError::ProviderUnavailable(err.to_string()))?;
    Jwks::from_json(&body)
}

/// Binds and serves the API until the process stops.
pub async fn serve(config: Config) -> Result<(), ApiError> {
    let host = std::env::var("API_HOST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_HOST.to_string());
    let port = std::env::var("API_PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);
    let corpus = std::env::var("RAG_CORPUS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "trusted".to_string());

    let state = AppState::new(config, corpus)?;
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|source| ApiError::Bind {
            address: format!("{host}:{port}"),
            source,
        })?;
    let address = listener.local_addr().map_err(ApiError::Address)?;
    eprintln!("api: listening on http://{address}");
    axum::serve(listener, state.router())
        .await
        .map_err(|err| ApiError::Serve(err.to_string()))
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
    message: String,
}

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            ok: false,
            error: code.to_string(),
            message: message.into(),
        }),
    )
        .into_response()
}

// --- authentication and authorization --------------------------------------

/// Reads the bearer credential from the request headers only.
///
/// Client-supplied identity headers (`X-User`, `X-Role`, ...) are ignored.
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(axum::http::header::AUTHORIZATION)?;
    let text = value.to_str().ok()?.trim();
    let token = text
        .strip_prefix("Bearer ")
        .or_else(|| text.strip_prefix("bearer "))?
        .trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Verifies the credential. Any error maps to 401; an outage never becomes
/// anonymous access.
#[allow(clippy::result_large_err)]
fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<VerifiedIdentity, Response> {
    let Some(token) = bearer_token(headers) else {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "a verified identity is required",
        ));
    };
    match validate_in_context(&token, &state.auth, &state.jwks) {
        Ok(identity) => Ok(identity),
        Err(err) => {
            eprintln!("api: token rejected: {err}");
            Err(error(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "the credential could not be verified",
            ))
        }
    }
}

/// Runs the synchronous JWKS/token verification without blocking the async
/// executor. The JWKS fetch uses `reqwest::blocking`, so it must run on a
/// blocking thread when a Tokio runtime is active; outside a runtime (unit
/// tests), it runs directly.
fn validate_in_context<F>(
    token: &str,
    config: &AuthConfig,
    jwks: &JwksCache<F>,
) -> Result<VerifiedIdentity, AuthError>
where
    F: Fn() -> Result<Jwks, AuthError> + Send + Sync + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(|| validate_token(token, config, jwks))
    } else {
        validate_token(token, config, jwks)
    }
}

/// Runs authentication, the central policy decision, and rate limiting in the
/// required order. Every protected handler calls this first.
#[allow(clippy::result_large_err)]
fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    operation: Operation,
) -> Result<VerifiedIdentity, Response> {
    let token_identity = authenticate(state, headers)?;
    let identity = load_identity(state, &token_identity)?;
    match identity::decide(Some(&identity), operation) {
        Decision::Allow => {}
        Decision::Unauthenticated => {
            return Err(error(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "a verified identity is required",
            ))
        }
        Decision::Forbidden => {
            return Err(error(
                StatusCode::FORBIDDEN,
                "forbidden",
                "the verified identity lacks the required role",
            ))
        }
    }
    if operation_requires_corpus(operation)
        && !identity
            .corpus_grants()
            .iter()
            .any(|grant| grant == &state.corpus)
    {
        return Err(error(
            StatusCode::FORBIDDEN,
            "corpus_forbidden",
            "the verified identity has no grant for this corpus",
        ));
    }
    let limiter = if operation.is_read() {
        &state.read_limiter
    } else {
        &state.limiter
    };
    if !limiter.check(identity.subject(), operation.rate_limit_key()) {
        return Err(error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many requests for this operation",
        ));
    }
    Ok(identity)
}

/// Operations that read or write documents in the configured corpus.
fn operation_requires_corpus(operation: Operation) -> bool {
    matches!(
        operation,
        Operation::Ingest
            | Operation::Query
            | Operation::DocumentStatus
            | Operation::GenealogyRead
            | Operation::GenealogyNominate
    )
}

/// Loads the effective roles and corpus grants from the database. The verified
/// token only identifies the subject; it never supplies authorization.
#[allow(clippy::result_large_err)]
fn load_identity(state: &AppState, token: &VerifiedIdentity) -> Result<VerifiedIdentity, Response> {
    let subject = token.subject().to_string();
    let issuer = token.issuer().to_string();
    let store = &state.store;
    let lookup = if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(|| load_identity_blocking(store, &subject, &issuer))
    } else {
        load_identity_blocking(store, &subject, &issuer)
    };
    match lookup {
        Ok((roles, grants)) => Ok(VerifiedIdentity::new(subject, issuer, roles, grants)),
        Err(IdentityStoreError::Database(message)) => {
            eprintln!("api: identity store unavailable: {message}");
            Err(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "identity_store_unavailable",
                "the identity store is unavailable",
            ))
        }
        Err(err) => {
            eprintln!("api: identity lookup failed for {subject}: {err}");
            Err(error(
                StatusCode::FORBIDDEN,
                "account_not_provisioned",
                "the verified identity has no active account",
            ))
        }
    }
}

fn load_identity_blocking(
    store: &PgStore,
    subject: &str,
    issuer: &str,
) -> Result<(Vec<Role>, Vec<String>), IdentityStoreError> {
    let client = identity_client(store)?;
    let store = IdentityStore::new(client);
    let user = store
        .user_by_subject(subject, issuer)?
        .ok_or_else(|| IdentityStoreError::UnknownUser(subject.to_string()))?;
    let roles = store.effective_roles(user.id)?;
    let grants = store.effective_grants(user.id)?;
    Ok((roles, grants))
}

/// Borrows an identity-store client from the shared pool. The identity tables
/// live in the same database as the document store.
fn identity_client(
    store: &PgStore,
) -> Result<crate::identity_store_pg::PgIdentityClient, IdentityStoreError> {
    store
        .pooled_connection()
        .map(crate::identity_store_pg::PgIdentityClient::from_pooled)
        .map_err(|err| IdentityStoreError::Database(err.to_string()))
}

// --- health ----------------------------------------------------------------

fn model_reachable(client: &reqwest::blocking::Client, url: &str) -> bool {
    let endpoint = format!("{}/api/tags", url.trim_end_matches('/'));
    client
        .get(endpoint)
        .send()
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

// Public and non-sensitive: no identity, no database content, no model output.
async fn health(State(state): State<AppState>) -> Response {
    let store = state.store.clone();
    let database_ok = tokio::task::spawn_blocking(move || store.ping().is_ok())
        .await
        .unwrap_or(false);
    let url = state.config.ollama_url.clone();
    let health_client = state.health_client.clone();
    let model_ok = tokio::task::spawn_blocking(move || {
        let client = {
            let Ok(mut guard) = health_client.lock() else {
                return false;
            };
            if guard.is_none() {
                *guard = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(3))
                    .build()
                    .ok();
            }
            guard.clone()
        };
        client.is_some_and(|client| model_reachable(&client, &url))
    })
    .await
    .unwrap_or(false);
    let healthy = database_ok && model_ok;
    let body = serde_json::json!({
        "status": if healthy { "ok" } else { "degraded" },
        "service": "rust-api",
        "database": if database_ok { "ok" } else { "down" },
        "model": if model_ok { "ok" } else { "down" },
    });
    let status = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body)).into_response()
}

// --- account and admin -----------------------------------------------------

#[derive(Serialize)]
struct AccountBody {
    ok: bool,
    subject: String,
    roles: Vec<&'static str>,
    corpus_grants: Vec<String>,
}

async fn account(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match authorize(&state, &headers, Operation::Account) {
        Ok(identity) => Json(AccountBody {
            ok: true,
            subject: identity.subject().to_string(),
            roles: identity.roles().iter().map(Role::as_str).collect(),
            corpus_grants: identity.corpus_grants().to_vec(),
        })
        .into_response(),
        Err(response) => response,
    }
}

/// Reads the authenticated member's own private profile.
///
/// Ownership is derived only from the verified `(issuer, subject)` pair and the
/// reviewed link it resolved elsewhere. The handler accepts no person key,
/// account id, or reviewer from the client. An unlinked account receives
/// `linked: false` and no private field.
#[allow(clippy::result_large_err)]
async fn profile(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut identity_client = postgres::Client::connect(&config.database_url, postgres::NoTls)
            .map_err(|_| {
                error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "identity_unavailable",
                    "the identity store is unavailable",
                )
            })?;
        let account = onboarding::resolve_account(&mut identity_client, &issuer, &subject)
            .map_err(account_map_error)?;
        let mut genealogy = open_genealogy(&config).map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        })?;
        onboarding::read_own_profile(&mut genealogy, &account).map_err(profile_error)
    })
    .await;

    match result {
        Ok(Ok(Some(view))) => {
            let fields: Vec<serde_json::Value> = view
                .fields
                .iter()
                .map(|field| {
                    serde_json::json!({
                        "field_key": field.field_key,
                        "value": field.value,
                        "updated_at": field.updated_at,
                    })
                })
                .collect();
            Json(serde_json::json!({
                "ok": true,
                "linked": true,
                "profile": {
                    "person_key": view.person_key,
                    "display_name": view.display_name,
                    "fields": fields,
                },
            }))
            .into_response()
        }
        Ok(Ok(None)) => Json(serde_json::json!({
            "ok": true,
            "linked": false,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: profile task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "profile_failed",
                "the profile read failed",
            )
        }
    }
}

fn profile_error(err: ProfileError) -> Response {
    eprintln!("api: profile read failed: {err}");
    match err {
        ProfileError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "profile_unavailable",
            "the profile store is unavailable",
        ),
    }
}

#[derive(Deserialize)]
struct RoleChangeRequest {
    subject: String,
    role: String,
    /// The issuer that owns the target account. Identity is keyed on
    /// `(issuer, subject)`, so a same-subject account under another issuer is
    /// never touched.
    issuer: String,
    #[serde(default)]
    revoke: bool,
}

fn identity_store_error(err: IdentityStoreError) -> Response {
    eprintln!("api: role change failed: {err}");
    match err {
        IdentityStoreError::UnknownUser(_) => error(
            StatusCode::NOT_FOUND,
            "unknown_user",
            "no such user; no role assignment was written",
        ),
        IdentityStoreError::InvalidRole(_) => error(
            StatusCode::BAD_REQUEST,
            "invalid_role",
            "role must be viewer, editor, or admin",
        ),
        IdentityStoreError::LastAdmin => error(
            StatusCode::CONFLICT,
            "last_admin",
            "refusing to remove the last active administrator",
        ),
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "role_change_failed",
            "the role change failed; nothing was written",
        ),
    }
}

async fn change_role(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RoleChangeRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::RoleChange) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let Some(role) = Role::parse(&request.role) else {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_role",
            "role must be viewer, editor, or admin",
        );
    };
    let subject = request.subject.trim().to_string();
    if subject.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_subject",
            "subject must not be empty",
        );
    }
    let issuer = request.issuer.trim().to_string();
    if issuer.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_issuer",
            "issuer must not be empty",
        );
    }
    let store = state.store.clone();
    let actor = identity.subject().to_string();
    let revoke = request.revoke;
    let task_subject = subject.clone();
    let task_issuer = issuer.clone();
    let result = tokio::task::spawn_blocking(move || {
        let client = identity_client(&store)?;
        let mut store = IdentityStore::new(client);
        if revoke {
            store.revoke_role(&task_subject, &task_issuer, role, &actor)
        } else {
            store.grant_role(&task_subject, &task_issuer, role, &actor)
        }
    })
    .await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({
            "ok": true,
            "subject": subject,
            "role": role.as_str(),
            "revoked": revoke,
        }))
        .into_response(),
        Ok(Err(err)) => identity_store_error(err),
        Err(err) => {
            eprintln!("api: role change task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "role_change_failed",
                "the role change task failed",
            )
        }
    }
}

// --- ingestion -------------------------------------------------------------

fn sanitize_file_name(name: &str) -> Option<String> {
    let name = FsPath::new(name)
        .file_name()?
        .to_string_lossy()
        .into_owned();
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(name.to_string())
}

fn extension(name: &str) -> Option<String> {
    FsPath::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
}

fn is_dependency_error(error: &PipelineError) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(err) = source {
        if let Some(ollama) = err.downcast_ref::<OllamaError>() {
            return matches!(
                ollama,
                OllamaError::Transport { .. }
                    | OllamaError::Timeout { .. }
                    | OllamaError::Status { .. }
                    | OllamaError::Cancelled { .. }
            );
        }
        if let Some(store) = err.downcast_ref::<PgStoreError>() {
            return matches!(
                store,
                PgStoreError::Config(_)
                    | PgStoreError::Pool(_)
                    | PgStoreError::Connection(_)
                    | PgStoreError::Query(_)
                    | PgStoreError::Embedding(_)
                    | PgStoreError::Decode(_)
            );
        }
        source = err.source();
    }
    false
}

/// Why an ingest blocking task failed.
enum IngestTaskError {
    /// The upload could not be staged to disk.
    Storage(String),
    /// The pipeline ran and failed.
    Pipeline(PipelineError),
}

async fn ingest(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    // Authorization runs before any field is read, staged, or embedded.
    if let Err(response) = authorize(&state, &headers, Operation::Ingest) {
        return response;
    }

    let mut file: Option<(String, Vec<u8>)> = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(err) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_multipart",
                    err.to_string(),
                )
            }
        };
        if field.name() != Some("file") {
            return error(
                StatusCode::BAD_REQUEST,
                "unexpected_field",
                format!("unexpected form field {:?}; expected `file`", field.name()),
            );
        }
        if file.is_some() {
            return error(
                StatusCode::BAD_REQUEST,
                "multiple_files",
                "exactly one file is accepted per request",
            );
        }
        let Some(name) = field.file_name().and_then(sanitize_file_name) else {
            return error(
                StatusCode::BAD_REQUEST,
                "missing_filename",
                "the uploaded file must have a name",
            );
        };
        let Some(ext) = extension(&name) else {
            return error(
                StatusCode::BAD_REQUEST,
                "unsupported_type",
                "supported types: .md, .markdown, .txt, .pdf",
            );
        };
        if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
            return error(
                StatusCode::BAD_REQUEST,
                "unsupported_type",
                format!("unsupported file type .{ext}; supported: .md, .markdown, .txt, .pdf"),
            );
        }
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_multipart",
                    err.to_string(),
                )
            }
        };
        if bytes.len() > MAX_UPLOAD_BYTES {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "file_too_large",
                format!("the upload exceeds the {} byte limit", MAX_UPLOAD_BYTES),
            );
        }
        let max_input = state.config.max_input_bytes;
        if max_input > 0 && bytes.len() > usize::try_from(max_input).unwrap_or(usize::MAX) {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "file_too_large",
                format!("the upload exceeds the {max_input} byte input limit"),
            );
        }
        if bytes.is_empty() {
            return error(
                StatusCode::BAD_REQUEST,
                "empty_file",
                "the uploaded file is empty",
            );
        }
        file = Some((name, bytes.to_vec()));
    }

    let Some((source, bytes)) = file else {
        return error(
            StatusCode::BAD_REQUEST,
            "missing_file",
            "no file field was supplied",
        );
    };

    let temp_extension = extension(&source).unwrap_or_else(|| "bin".to_string());
    let config = state.config.clone();
    let store = state.store.clone();
    let corpus = state.corpus.clone();
    let source_for_run = source.clone();

    // Staging, hashing, and cleanup are blocking filesystem work; keep them off
    // the async executor.
    let outcome = tokio::task::spawn_blocking(move || {
        let temp_dir = std::env::temp_dir().join(format!(
            "rag-api-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&temp_dir).map_err(|err| {
            IngestTaskError::Storage(format!("could not stage the upload: {err}"))
        })?;
        let staged = temp_dir.join(format!("source.{temp_extension}"));
        if let Err(err) = std::fs::write(&staged, &bytes) {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Err(IngestTaskError::Storage(format!(
                "could not stage the upload: {err}"
            )));
        }

        let chunker = ChunkerConfig::new(config.chunk_size as usize, config.chunk_overlap as usize);
        let content_hash = fingerprint(&bytes, &chunker);
        let result = (|| -> Result<IngestOutcome, PipelineError> {
            let transport = Arc::new(HttpTransport::new(config.request_timeout)?);
            let client =
                OllamaClient::new(config.ollama_url.clone(), transport, config.request_timeout);
            let embedder = OllamaEmbedder::new(client, config.embed_model.clone())
                .with_embed_dim(config.embed_dim as usize);
            let pipeline_config = PipelineConfig::new(
                config.language.clone(),
                chunker,
                config.embed_model.clone(),
                corpus,
            );
            let service = IngestService::new(&embedder, &*store, &*store, pipeline_config);
            service.ingest_source(&staged, &source_for_run, &content_hash)
        })();

        let _ = std::fs::remove_dir_all(&temp_dir);
        result.map_err(IngestTaskError::Pipeline)
    })
    .await;

    match outcome {
        Ok(Ok(IngestOutcome::Replaced { chunks, .. })) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "source": source,
                "chunks": chunks,
                "incomplete_pages": [],
                "unchanged": false,
            })),
        )
            .into_response(),
        Ok(Ok(IngestOutcome::Unchanged { rows, .. })) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "source": source,
                "chunks": rows,
                "incomplete_pages": [],
                "unchanged": true,
            })),
        )
            .into_response(),
        Ok(Ok(IngestOutcome::Rejected(IngestRejection::Incomplete { pages }))) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "ok": false,
                "error": "incomplete_pdf",
                "message": format!(
                    "page(s) {} have no text layer; OCR is required and no data was written",
                    pages.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
                ),
                "incomplete_pages": pages,
            })),
        )
            .into_response(),
        Ok(Ok(IngestOutcome::Rejected(IngestRejection::NoChunks { .. }))) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_sections",
            "the source produced no sections; no data was written",
        ),
        Ok(Ok(IngestOutcome::Rejected(IngestRejection::Loader(message)))) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "extraction_failed",
            format!("{message}; no data was written"),
        ),
        Ok(Err(IngestTaskError::Pipeline(err))) => {
            eprintln!("api: ingest failed for {source}: {err}");
            if is_dependency_error(&err) {
                error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "dependency_unavailable",
                    "a database or model dependency is unavailable",
                )
            } else {
                error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "ingestion_failed",
                    "ingestion failed; no data was written",
                )
            }
        }
        Ok(Err(IngestTaskError::Storage(message))) => {
            eprintln!("api: ingest staging failed for {source}: {message}");
            error(StatusCode::INTERNAL_SERVER_ERROR, "storage_error", message)
        }
        Err(err) => {
            eprintln!("api: ingest task failed for {source}: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ingestion_failed",
                "ingestion task failed",
            )
        }
    }
}

// --- ingestion status ------------------------------------------------------

async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(source): Path<String>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::DocumentStatus) {
        return response;
    }
    let store = state.store.clone();
    let corpus = state.corpus.clone();
    let result = tokio::task::spawn_blocking(move || store.source_status(&corpus, &source)).await;
    match result {
        Ok(Ok(Some(status))) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "source": status.source,
                "corpus": status.corpus,
                "chunks": status.chunks,
                "content_hash": status.content_hash,
                "ingested_at": status.ingested_at,
                "sections": status.sections.iter().map(|section| serde_json::json!({
                    "section": section.section,
                    "chunk_index": section.chunk_index,
                    "page": section.page,
                })).collect::<Vec<_>>(),
            })),
        )
            .into_response(),
        Ok(Ok(None)) => error(
            StatusCode::NOT_FOUND,
            "not_found",
            "no stored ingestion exists for that source",
        ),
        Ok(Err(err)) => {
            eprintln!("api: status failed: {err}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "dependency_unavailable",
                "the database is unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: status task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "status_failed",
                "the status request failed",
            )
        }
    }
}

// --- query -----------------------------------------------------------------

#[derive(Deserialize)]
struct QueryRequest {
    question: String,
    #[serde(default)]
    top_k: Option<usize>,
}

async fn query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<QueryRequest>,
) -> Response {
    // The body is parsed by Axum before this point, but no retrieval, embedding,
    // or model call happens until authorization succeeds.
    if let Err(response) = authorize(&state, &headers, Operation::Query) {
        return response;
    }
    let question = request.question.trim().to_string();
    if question.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "empty_question",
            "the question must not be empty",
        );
    }
    let max_question = state.config.max_question_bytes;
    if max_question > 0 && question.len() > usize::try_from(max_question).unwrap_or(usize::MAX) {
        return error(
            StatusCode::BAD_REQUEST,
            "question_too_large",
            format!("the question exceeds the {max_question} byte limit"),
        );
    }
    if let Some(top_k) = request.top_k {
        if !(1..=50).contains(&top_k) {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_top_k",
                "top_k must be between 1 and 50",
            );
        }
    }
    let config = state.config.clone();
    let store = state.store.clone();
    let corpus = state.corpus.clone();
    let cell = state.query_service.clone();
    let answer = tokio::task::spawn_blocking(move || {
        let service = {
            let mut guard = cell
                .lock()
                .map_err(|_| "the query service lock is poisoned".to_string())?;
            if guard.is_none() {
                *guard = Some(Arc::new(
                    QueryService::from_config(&config, store, corpus)
                        .map_err(|err| err.to_string())?,
                ));
            }
            guard
                .as_ref()
                .cloned()
                .ok_or_else(|| "the query service was not initialized".to_string())?
        };
        service.answer(&question).map_err(|err| err.to_string())
    })
    .await;
    match answer {
        Ok(Ok(answer)) => {
            let sources: Vec<_> = answer
                .citations
                .iter()
                .map(|passage| {
                    serde_json::json!({
                        "source": passage.source,
                        "section": passage.section,
                        "page": passage.page,
                        "text": passage.content,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "answer": answer.text,
                    "insufficient": answer.refused,
                    "sources": sources,
                })),
            )
                .into_response()
        }
        Ok(Err(err)) => {
            eprintln!("api: query failed: {err}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "query_failed",
                "retrieval or generation is unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: query task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "query_failed",
                "the query task failed",
            )
        }
    }
}

// --- genealogy reads and Head of Family -----------------------------------

fn open_genealogy(config: &Config) -> Result<postgres::Client, String> {
    let dsn = resolve_genealogy_dsn(config).map_err(|err| err.to_string())?;
    postgres::Client::connect(&dsn, postgres::NoTls).map_err(|err| err.to_string())
}

fn genealogy_error(err: GenealogyError) -> Response {
    eprintln!("api: genealogy operation failed: {err}");
    match err {
        GenealogyError::Domain(message) => {
            error(StatusCode::CONFLICT, "genealogy_rejected", &message)
        }
        GenealogyError::Guard(_) | GenealogyError::Config(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "genealogy_unavailable",
            "the genealogy store is not configured for this operation",
        ),
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "genealogy_unavailable",
            "the genealogy store is unavailable",
        ),
    }
}

fn evidence_json(citations: &[genealogy_query::Citation]) -> Vec<serde_json::Value> {
    citations
        .iter()
        .map(|citation| {
            serde_json::json!({
                "evidence_id": format!("{}#{}", citation.document_ref, citation.page),
                "citation": format!("{} p.{}", citation.document_ref, citation.page),
                "document_ref": citation.document_ref,
                "page": citation.page,
            })
        })
        .collect()
}

async fn genealogy_records(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::GenealogyRead) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config)?;
        genealogy_query::list_records(&mut client, 100, 0).map_err(|err| err.to_string())
    })
    .await;
    match result {
        Ok(Ok(rows)) => {
            let records: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "record_id": row.person_key,
                        "person_label": row.display_name,
                        "relationship": row.relationship,
                        "evidence": evidence_json(&row.citations),
                        "evidence_status": row.evidence_status,
                    })
                })
                .collect();
            Json(serde_json::json!({ "records": records })).into_response()
        }
        Ok(Err(message)) => {
            eprintln!("api: genealogy records failed: {message}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: genealogy records task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "genealogy_unavailable",
                "the genealogy task failed",
            )
        }
    }
}

/// Authorized, bounded, read-only genealogy graph.
async fn genealogy_tree(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::GenealogyRead) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config)?;
        genealogy_tree::load_tree(&mut client).map_err(|err| err.to_string())
    })
    .await;

    match result {
        Ok(Ok(graph)) => {
            let people: Vec<_> = graph
                .people
                .iter()
                .map(|person| {
                    serde_json::json!({
                        "record_id": person.person_key,
                        "person_label": person.display_name,
                        "evidence_status": person.evidence_status,
                        "aliases": person.aliases,
                    })
                })
                .collect();
            let links: Vec<_> = graph
                .links
                .iter()
                .map(|link| {
                    serde_json::json!({
                        "from_id": link.from_key,
                        "to_id": link.to_key,
                        "kind": link.kind,
                        "evidence_status": link.evidence_status,
                        "evidence": evidence_json(&link.citations),
                    })
                })
                .collect();
            Json(serde_json::json!({
                "people": people,
                "links": links,
                "truncated": graph.truncated,
            }))
            .into_response()
        }
        Ok(Err(message)) => {
            eprintln!("api: genealogy tree failed: {message}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: genealogy tree task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "genealogy_unavailable",
                "the genealogy task failed",
            )
        }
    }
}

async fn genealogy_head_of_family(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::GenealogyRead) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config)?;
        let appointments =
            appointments::list_active_appointments(&mut client).map_err(|err| err.to_string())?;
        let mut conclusions = Vec::with_capacity(appointments.len());
        for appointment in &appointments {
            let citations = appointments::citations_for_assertions(
                &mut client,
                &appointment.evidence_assertion_ids,
            )
            .map_err(|err| err.to_string())?;
            conclusions.push(serde_json::json!({
                "conclusion": "Head of Family",
                "record_id": appointment.person_key,
                "appointment_id": appointment.appointment_id,
                "head_label": appointment.display_name,
                "evidence": evidence_json(&citations),
                "evidence_status": "verified",
            }));
        }
        Ok::<_, String>(conclusions)
    })
    .await;
    match result {
        Ok(Ok(conclusions)) => Json(serde_json::json!({
            "catalog": "available",
            "conclusions": conclusions,
        }))
        .into_response(),
        Ok(Err(message)) => {
            eprintln!("api: head of family read failed: {message}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: head of family task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "genealogy_unavailable",
                "the genealogy task failed",
            )
        }
    }
}

async fn genealogy_person(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::GenealogyRead) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config)?;
        genealogy_query::lookup_person(&mut client, &key).map_err(|err| err.to_string())
    })
    .await;
    match result {
        Ok(Ok(Some(person))) => {
            let assertions: Vec<serde_json::Value> = person
                .assertions
                .iter()
                .map(|assertion| {
                    serde_json::json!({
                        "assertion_id": assertion.assertion_id,
                        "kind": assertion.kind,
                        "key": assertion.key,
                        "value_text": assertion.value_text,
                        "review_status": assertion.review_status,
                        "evidence": evidence_json(&assertion.citations),
                    })
                })
                .collect();
            Json(serde_json::json!({
                "record_id": person.person_key,
                "person_label": person.display_name,
                "assertions": assertions,
            }))
            .into_response()
        }
        Ok(Ok(None)) => error(
            StatusCode::NOT_FOUND,
            "genealogy_not_found",
            "no authorized record exists",
        ),
        Ok(Err(message)) => {
            eprintln!("api: genealogy person failed: {message}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: genealogy person task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "genealogy_unavailable",
                "the genealogy task failed",
            )
        }
    }
}

async fn genealogy_relationship(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((from, to)): Path<(String, String)>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::GenealogyRead) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config)?;
        genealogy_query::lookup_relationship(&mut client, &from, &to).map_err(|err| err.to_string())
    })
    .await;
    match result {
        Ok(Ok(relationship)) => Json(serde_json::json!({
            "record_id": relationship.to_person_key,
            "person_label": relationship.from_person_key,
            "relationship": relationship.relationship_kind,
            "evidence_status": relationship.status,
            "evidence": evidence_json(&relationship.citations),
        }))
        .into_response(),
        Ok(Err(message)) => {
            eprintln!("api: genealogy relationship failed: {message}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: genealogy relationship task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "genealogy_unavailable",
                "the genealogy task failed",
            )
        }
    }
}

async fn list_nominations(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::GenealogyRead) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config)?;
        appointments::list_pending_nominations(&mut client).map_err(|err| err.to_string())
    })
    .await;
    match result {
        Ok(Ok(rows)) => {
            let nominations: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "nomination_id": row.nomination_id,
                        "record_id": row.person_key,
                        "status": row.status,
                        "created_at": row.created_at,
                    })
                })
                .collect();
            Json(serde_json::json!({ "nominations": nominations })).into_response()
        }
        Ok(Err(message)) => {
            eprintln!("api: nomination list failed: {message}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: nomination list task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "genealogy_unavailable",
                "the genealogy task failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct NominationBody {
    #[serde(alias = "person_key")]
    record_id: String,
}

async fn nominate_head_of_family(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<NominationBody>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::GenealogyNominate) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let branch = match state.config.hof_third_branch_key() {
        Ok(branch) => branch.to_string(),
        Err(_) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "third_branch_not_configured",
                "the verified third branch is not configured",
            )
        }
    };
    let config = state.config.clone();
    let person_key = body.record_id.trim().to_string();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config).map_err(GenealogyError::Guard)?;
        appointments::nominate(&mut client, &person_key, &branch, &actor)
    })
    .await;
    match result {
        Ok(Ok(nomination_id)) => Json(serde_json::json!({
            "ok": true,
            "nomination_id": nomination_id,
            "status": "pending",
        }))
        .into_response(),
        Ok(Err(err)) => genealogy_error(err),
        Err(err) => {
            eprintln!("api: nomination task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nomination_failed",
                "the nomination task failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct NominationDecision {
    /// Required: a missing field is rejected rather than read as approval.
    approve: bool,
    #[serde(default)]
    note: Option<String>,
}

async fn decide_nomination(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<NominationDecision>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::HeadOfFamilyApproval) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let branch = match state.config.hof_third_branch_key() {
        Ok(branch) => branch.to_string(),
        Err(_) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "third_branch_not_configured",
                "the verified third branch is not configured",
            )
        }
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let approve = body.approve;
    let note = body.note;
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config).map_err(GenealogyError::Guard)?;
        if approve {
            appointments::approve(&mut client, id, &branch, &actor)
                .map(|appointment| (true, appointment.appointment_id))
        } else {
            appointments::reject(&mut client, id, &actor, note.as_deref()).map(|_| (false, 0))
        }
    })
    .await;
    match result {
        Ok(Ok((true, appointment_id))) => Json(serde_json::json!({
            "ok": true,
            "appointment_id": appointment_id,
            "status": "approved",
        }))
        .into_response(),
        Ok(Ok((false, _))) => Json(serde_json::json!({
            "ok": true,
            "status": "rejected",
        }))
        .into_response(),
        Ok(Err(err)) => genealogy_error(err),
        Err(err) => {
            eprintln!("api: nomination decision task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nomination_failed",
                "the nomination decision task failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct AppointmentEndBody {
    reason: String,
    #[serde(default)]
    suspend: Option<bool>,
}

async fn end_head_of_family(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<AppointmentEndBody>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::HeadOfFamilyApproval) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let suspend = body.suspend.unwrap_or(false);
    let reason = body.reason;
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config).map_err(GenealogyError::Guard)?;
        appointments::end_appointment(&mut client, id, &actor, &reason, suspend)
    })
    .await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({
            "ok": true,
            "status": if suspend { "suspended" } else { "revoked" },
        }))
        .into_response(),
        Ok(Err(err)) => genealogy_error(err),
        Err(err) => {
            eprintln!("api: appointment end task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "appointment_failed",
                "the appointment task failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct ProvisionUserBody {
    subject: String,
    issuer: String,
}

async fn provision_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ProvisionUserBody>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::RoleChange) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let store = state.store.clone();
    let actor = identity.subject().to_string();
    let subject = body.subject.trim().to_string();
    let issuer = body.issuer.trim().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let client = identity_client(&store)?;
        let mut store = IdentityStore::new(client);
        store.client().provision_user(&subject, &issuer, &actor)
    })
    .await;
    match result {
        Ok(Ok(user_id)) => {
            Json(serde_json::json!({ "ok": true, "user_id": user_id })).into_response()
        }
        Ok(Err(err)) => identity_store_error(err),
        Err(err) => {
            eprintln!("api: provision task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "provision_failed",
                "the provisioning task failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct DeactivateQuery {
    /// The issuer that owns the target account. Identity is keyed on
    /// `(issuer, subject)`, so a same-subject account under another issuer is
    /// never deactivated.
    issuer: String,
}

async fn deactivate_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subject): Path<String>,
    Query(query): Query<DeactivateQuery>,
) -> Response {
    let issuer = query.issuer.trim().to_string();
    deactivate_identity(state, headers, subject, issuer).await
}

async fn deactivate_user_scoped(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((issuer, subject)): Path<(String, String)>,
) -> Response {
    deactivate_identity(state, headers, subject, issuer).await
}

async fn deactivate_identity(
    state: AppState,
    headers: HeaderMap,
    subject: String,
    issuer: String,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::RoleChange) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if issuer.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_issuer",
            "issuer must not be empty",
        );
    }
    let store = state.store.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let client = identity_client(&store)?;
        let mut store = IdentityStore::new(client);
        store.client().deactivate_user(&subject, &issuer, &actor)
    })
    .await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true, "deactivated": true })).into_response(),
        Ok(Err(err)) => identity_store_error(err),
        Err(err) => {
            eprintln!("api: deactivate task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "deactivate_failed",
                "the deactivation task failed",
            )
        }
    }
}

// --- onboarding preview ----------------------------------------------------

/// The privacy-safe summary of one preview. Counts only; never a person.
#[derive(Serialize)]
struct PreviewSummaryBody {
    eligible_for_invite: usize,
    needs_review: usize,
    already_linked: usize,
    invited: usize,
    not_eligible: usize,
}

/// One classified candidate for the wire. Carries the opaque key and reason
/// codes only.
#[derive(Serialize)]
struct PreviewRowBody {
    person_key: String,
    classification: &'static str,
    reasons: Vec<&'static str>,
}

/// The full response envelope.
#[derive(Serialize)]
struct PreviewBody {
    ok: bool,
    rows: Vec<PreviewRowBody>,
    summary: PreviewSummaryBody,
}

/// An admin-only, strictly read-only preview of living-person candidates.
///
/// Authorization runs before any database connection. The genealogy read runs
/// in a blocking task. Rows are never logged; the response carries opaque keys
/// and reason codes only.
async fn family_member_candidates(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::OnboardingPreview) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let dsn = resolve_genealogy_dsn(&config).map_err(|err| err.to_string())?;
        let mut client =
            postgres::Client::connect(&dsn, postgres::NoTls).map_err(|err| err.to_string())?;
        onboarding::preview_candidates(&mut client).map_err(|err| err.to_string())
    })
    .await;
    match result {
        Ok(Ok(preview)) => preview_response(&preview),
        Ok(Err(message)) => {
            // No row detail is logged. The message names a rule or a
            // destination, never a person.
            eprintln!("api: onboarding preview unavailable: {message}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: onboarding preview task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "preview_failed",
                "the candidate preview failed",
            )
        }
    }
}

/// Maps a preview to the wire body. The summary is derived from the preview.
fn preview_response(preview: &CandidatePreview) -> Response {
    let summary = PreviewSummary::from_preview(preview);
    let rows = preview
        .rows
        .iter()
        .map(|row| PreviewRowBody {
            person_key: row.person_key.clone(),
            classification: row.classification.as_str(),
            reasons: row.reasons.iter().map(|reason| reason.as_str()).collect(),
        })
        .collect();
    let body = PreviewBody {
        ok: true,
        rows,
        summary: PreviewSummaryBody {
            eligible_for_invite: summary.eligible_for_invite,
            needs_review: summary.needs_review,
            already_linked: summary.already_linked,
            invited: summary.invited,
            not_eligible: summary.not_eligible,
        },
    };
    Json(body).into_response()
}

// --- onboarding administration ---------------------------------------------

/// Default invitation lifetime: seven days.
const DEFAULT_INVITATION_TTL_SECS: u64 = 7 * 24 * 60 * 60;
/// Longest accepted invitation lifetime: thirty days.
const MAX_INVITATION_TTL_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Deserialize)]
struct CreateInvitationRequest {
    person_key: String,
    #[serde(default)]
    ttl_secs: Option<u64>,
}

#[derive(Deserialize)]
struct CreateLinkRequest {
    person_key: String,
    issuer: String,
    subject: String,
    decision: String,
}

/// Resolves an opaque person key to its genealogy id.
fn person_id_for_key(
    client: &mut postgres::Client,
    person_key: &str,
) -> Result<Option<i64>, String> {
    client
        .query_opt(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&person_key],
        )
        .map_err(|err| err.to_string())
        .map(|row| row.map(|row| row.get(0)))
}

fn invitation_error(err: onboarding::InvitationError) -> Response {
    match err {
        onboarding::InvitationError::UnknownPerson => {
            error(StatusCode::NOT_FOUND, "unknown_person", "no such person")
        }
        onboarding::InvitationError::InvalidToken => error(
            StatusCode::NOT_FOUND,
            "unknown_invitation",
            "no such invitation",
        ),
        onboarding::InvitationError::Expired => error(
            StatusCode::CONFLICT,
            "invitation_expired",
            "the invitation has expired",
        ),
        onboarding::InvitationError::AlreadyAccepted => error(
            StatusCode::CONFLICT,
            "invitation_accepted",
            "the invitation was already accepted",
        ),
        onboarding::InvitationError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "invitation_unavailable",
            "the invitation store is unavailable",
        ),
    }
}

fn account_map_error(err: AccountMapError) -> Response {
    match err {
        AccountMapError::InvalidIdentity => error(
            StatusCode::BAD_REQUEST,
            "invalid_identity",
            "issuer and subject are required",
        ),
        AccountMapError::UnknownIdentity => error(
            StatusCode::NOT_FOUND,
            "identity_not_registered",
            "the identity is not registered",
        ),
        AccountMapError::DeactivatedIdentity => error(
            StatusCode::CONFLICT,
            "identity_deactivated",
            "the identity is deactivated",
        ),
        AccountMapError::AmbiguousIdentity => error(
            StatusCode::CONFLICT,
            "identity_ambiguous",
            "the identity is ambiguous across issuers",
        ),
        AccountMapError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity_unavailable",
            "the identity store is unavailable",
        ),
    }
}

fn link_error(err: LinkError) -> Response {
    match err {
        LinkError::UnknownPerson | LinkError::UnknownLink => error(
            StatusCode::NOT_FOUND,
            "unknown_link",
            "no such person or link",
        ),
        LinkError::AlreadyLinked => error(
            StatusCode::CONFLICT,
            "already_linked",
            "the person or account is already linked",
        ),
        LinkError::InvalidReviewer => error(
            StatusCode::BAD_REQUEST,
            "invalid_reviewer",
            "a reviewer is required",
        ),
        LinkError::Rejected => error(
            StatusCode::CONFLICT,
            "link_rejected",
            "the review rejected the link",
        ),
        LinkError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "link_unavailable",
            "the link store is unavailable",
        ),
    }
}

#[allow(clippy::result_large_err)]
async fn create_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateInvitationRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::OnboardingInvite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let person_key = request.person_key.trim().to_string();
    if person_key.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_person",
            "person_key must not be empty",
        );
    }
    let ttl = request
        .ttl_secs
        .unwrap_or(DEFAULT_INVITATION_TTL_SECS)
        .clamp(1, MAX_INVITATION_TTL_SECS);
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let task_person_key = person_key.clone();

    let result =
        tokio::task::spawn_blocking(move || -> Result<onboarding::IssuedInvitation, Response> {
            let mut client = open_genealogy(&config).map_err(|_| {
                error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "genealogy_unavailable",
                    "the genealogy store is unavailable",
                )
            })?;
            let person_id = person_id_for_key(&mut client, &task_person_key)
                .map_err(|_| {
                    error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "invitation_failed",
                        "the invitation failed",
                    )
                })?
                .ok_or_else(|| error(StatusCode::NOT_FOUND, "unknown_person", "no such person"))?;
            onboarding::create(&mut client, person_id, Duration::from_secs(ttl), &actor)
                .map_err(invitation_error)
        })
        .await;

    match result {
        Ok(Ok(invitation)) => Json(serde_json::json!({
            "ok": true,
            "invitation_id": invitation.invitation_id,
            "person_key": person_key,
            "token": invitation.token,
            "expires_at": invitation.expires_at,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: invitation task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invitation_failed",
                "the invitation failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn resend_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(invitation_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::OnboardingInvite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();

    let result =
        tokio::task::spawn_blocking(move || -> Result<onboarding::IssuedInvitation, Response> {
            let mut client = open_genealogy(&config).map_err(|_| {
                error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "genealogy_unavailable",
                    "the genealogy store is unavailable",
                )
            })?;
            let row = client
                .query_opt(
                    "SELECT person_id, status FROM onboarding_invitation WHERE invitation_id = $1",
                    &[&invitation_id],
                )
                .map_err(|_| {
                    error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "invitation_failed",
                        "the invitation failed",
                    )
                })?
                .ok_or_else(|| {
                    error(
                        StatusCode::NOT_FOUND,
                        "unknown_invitation",
                        "no such invitation",
                    )
                })?;
            let person_id: i64 = row.get(0);
            let status: String = row.get(1);
            if status == "accepted" {
                return Err(error(
                    StatusCode::CONFLICT,
                    "invitation_accepted",
                    "the invitation was already accepted",
                ));
            }
            // A pending invitation is revoked before a replacement is issued. An
            // already revoked or expired row is left as history.
            let _ = onboarding::revoke(&mut client, invitation_id);
            onboarding::create(
                &mut client,
                person_id,
                Duration::from_secs(DEFAULT_INVITATION_TTL_SECS),
                &actor,
            )
            .map_err(invitation_error)
        })
        .await;

    match result {
        Ok(Ok(invitation)) => Json(serde_json::json!({
            "ok": true,
            "invitation_id": invitation.invitation_id,
            "token": invitation.token,
            "expires_at": invitation.expires_at,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: invitation resend task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invitation_failed",
                "the invitation failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn revoke_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(invitation_id): Path<i64>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::OnboardingInvite) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config).map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        })?;
        onboarding::revoke(&mut client, invitation_id).map_err(invitation_error)
    })
    .await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true, "revoked": true })).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: invitation revoke task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invitation_failed",
                "the invitation failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn create_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateLinkRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::OnboardingLink) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let person_key = request.person_key.trim().to_string();
    let issuer = request.issuer.trim().to_string();
    let subject = request.subject.trim().to_string();
    if person_key.is_empty() || issuer.is_empty() || subject.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_link_request",
            "person_key, issuer, and subject are required",
        );
    }
    let decision = match request.decision.trim().to_ascii_lowercase().as_str() {
        "approve" | "approved" => LinkDecision::Approve,
        "reject" | "rejected" => LinkDecision::Reject,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_decision",
                "decision must be approve or reject",
            )
        }
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let task_person_key = person_key.clone();

    let result = tokio::task::spawn_blocking(move || {
        let mut identity_client = postgres::Client::connect(&config.database_url, postgres::NoTls)
            .map_err(|_| {
                error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "identity_unavailable",
                    "the identity store is unavailable",
                )
            })?;
        let account = onboarding::resolve_account(&mut identity_client, &issuer, &subject)
            .map_err(account_map_error)?;
        let mut genealogy = open_genealogy(&config).map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        })?;
        let person_id = person_id_for_key(&mut genealogy, &task_person_key)
            .map_err(|_| {
                error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "link_failed",
                    "the link failed",
                )
            })?
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "unknown_person", "no such person"))?;
        link::link_reviewed(&mut genealogy, person_id, &account, decision, &actor)
            .map_err(link_error)
    })
    .await;

    match result {
        Ok(Ok(Some(record))) => Json(serde_json::json!({
            "ok": true,
            "linked": true,
            "link_id": record.link_id,
            "person_key": person_key,
        }))
        .into_response(),
        Ok(Ok(None)) => Json(serde_json::json!({
            "ok": true,
            "linked": false,
            "person_key": person_key,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: link task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "link_failed",
                "the link failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn revoke_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(link_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::OnboardingLink) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config).map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        })?;
        link::revoke_link(&mut client, link_id, &actor).map_err(link_error)
    })
    .await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true, "revoked": true })).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: link revoke task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "link_failed",
                "the link failed",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_expected_extensions_are_allowed() {
        for extension in ["md", "markdown", "txt", "pdf"] {
            assert!(ALLOWED_EXTENSIONS.contains(&extension));
        }
        for extension in ["exe", "html", "docx", ""] {
            assert!(!ALLOWED_EXTENSIONS.contains(&extension));
        }
    }

    #[test]
    fn file_names_are_reduced_to_a_base_name() {
        assert_eq!(
            sanitize_file_name("../../etc/passwd"),
            Some("passwd".to_string())
        );
        assert_eq!(
            sanitize_file_name("loan-policy.md"),
            Some("loan-policy.md".to_string())
        );
        assert_eq!(sanitize_file_name("../"), None);
        assert_eq!(sanitize_file_name(""), None);
    }

    #[test]
    fn nomination_decision_requires_an_explicit_approve() {
        assert!(serde_json::from_str::<NominationDecision>("{}").is_err());
        assert!(serde_json::from_str::<NominationDecision>(r#"{"note":"ok"}"#).is_err());
        let rejected: NominationDecision =
            serde_json::from_str(r#"{"approve":false,"note":"no"}"#).unwrap();
        assert!(!rejected.approve);
    }

    #[test]
    fn extension_detection_is_case_insensitive() {
        assert_eq!(extension("Policy.MD"), Some("md".to_string()));
        assert_eq!(extension("no-extension"), None);
    }

    #[test]
    fn onboarding_preview_is_admin_only() {
        let viewer = VerifiedIdentity::new("sub-1", "https://issuer", vec![Role::Viewer], vec![]);
        let editor = VerifiedIdentity::new("sub-2", "https://issuer", vec![Role::Editor], vec![]);
        let admin = VerifiedIdentity::new("sub-3", "https://issuer", vec![Role::Admin], vec![]);
        assert_eq!(
            identity::decide(Some(&viewer), Operation::OnboardingPreview),
            Decision::Forbidden
        );
        assert_eq!(
            identity::decide(Some(&editor), Operation::OnboardingPreview),
            Decision::Forbidden
        );
        assert_eq!(
            identity::decide(Some(&admin), Operation::OnboardingPreview),
            Decision::Allow
        );
    }

    #[test]
    fn client_identity_headers_are_ignored() {
        let mut headers = HeaderMap::new();
        headers.insert("x-user", "admin".parse().unwrap());
        headers.insert("x-role", "admin".parse().unwrap());
        assert!(bearer_token(&headers).is_none());
    }

    #[test]
    fn bearer_token_is_read_from_authorization_only() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer token-value".parse().unwrap(),
        );
        assert_eq!(bearer_token(&headers), Some("token-value".to_string()));
    }
}
