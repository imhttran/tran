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
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::admin_members::{
    self, MemberFilter, MemberStatus, ResetError, ResetRequest, ResetState,
};
use crate::browser_session::{
    self, HttpRefreshProvider, NewSession, RenewOutcome, SessionConfig, SessionError,
};
use crate::config::Config;
use crate::corpus::Corpus;
use crate::document_lifecycle::{
    self, AccessScope, Category, DocumentStorage, DocumentVersion, NewSubmission,
};
use crate::document_processing;
use crate::embedding::OllamaEmbedder;
use crate::genealogy::{
    appointments, authority, branch_review, document_extraction, narrative, person_search,
    photograph_review, photograph_store::PhotographStore, query as genealogy_query,
    resolve_genealogy_dsn, tree as genealogy_tree, GenealogyError,
};
use crate::identity::{self, Decision, Operation, Role, VerifiedIdentity};
use crate::identity_store::{IdentityStore, IdentityStoreError};
use crate::oidc::{validate_token, AuthConfig, AuthError, Jwks, JwksCache};
use crate::ollama::{HttpTransport, OllamaClient};
use crate::onboarding::family_profile::{
    AdminArchivePhotograph, ArchivePhotographDownload, ArchivePhotographError,
    FamilyArchivePhotograph, FamilyGalleryPhotograph,
};
use crate::onboarding::{
    self, link, AccountMapError, ApprovedProfileValue, CandidatePreview, FamilyProfileError,
    LinkDecision, LinkError, MediaAsset, MediaDownload, MediaError, MediaKind, MediaStorage,
    OwnerProfile, PreviewSummary, ProfileRevision, ProfileVisibility, ReviewDecision,
    RevisionError, RevisionStatus,
};
use crate::query_service::QueryService;
use crate::ratelimit::RateLimiter;
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
    /// One shared client for password-reset provider calls; redirects are
    /// never followed.
    reset_client: Arc<Mutex<Option<reqwest::blocking::Client>>>,
    /// Server-side browser sessions and refresh-token storage. Disabled when
    /// the renewal configuration is absent.
    session: Arc<SessionConfig>,
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
        let session = Arc::new(SessionConfig::from_config(&config));
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
            reset_client: Arc::new(Mutex::new(None)),
            session,
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
            .route("/api/internal/sessions", post(create_internal_session))
            .route(
                "/api/internal/sessions/{id}/renew",
                post(renew_internal_session),
            )
            .route(
                "/api/internal/sessions/{id}",
                delete(revoke_internal_session),
            )
            .route("/api/documents/ingest", post(submit_document))
            .route(
                "/api/documents/submissions",
                post(submit_document).get(list_document_submissions),
            )
            .route(
                "/api/documents/submissions/{id}",
                get(get_document_submission),
            )
            .route(
                "/api/documents/submissions/{id}/content",
                get(get_document_content),
            )
            .route(
                "/api/documents/submissions/{id}/review",
                post(review_document_submission),
            )
            .route(
                "/api/documents/submissions/{id}/scope",
                post(change_document_submission_scope),
            )
            .route(
                "/api/documents/submissions/{id}/remove",
                post(remove_document_submission),
            )
            .route(
                "/api/documents/submissions/{id}/process",
                post(process_document_submission),
            )
            .route("/api/documents/legacy", get(legacy_document_inventory))
            .route(
                "/api/documents/legacy/{id}/promote",
                post(promote_legacy_document),
            )
            .route("/api/documents/jobs", get(list_document_jobs))
            .route("/api/documents/jobs/run", post(run_document_jobs))
            .route("/api/documents/jobs/{id}/retry", post(retry_document_job))
            .route("/api/documents/{source}/status", get(status))
            .route("/api/query", post(query))
            .route("/api/query/suggestions", get(query_suggestions))
            .route("/api/account", get(account))
            .route("/api/profile", get(profile))
            .route("/api/profile/drafts", post(create_profile_draft))
            .route("/api/profile/drafts/{id}", patch(update_profile_draft))
            .route(
                "/api/profile/drafts/{id}/submit",
                post(submit_profile_draft),
            )
            .route(
                "/api/profile/drafts/{id}/cancel",
                post(cancel_profile_draft),
            )
            .route(
                "/api/profile/revisions/{id}/review",
                post(review_profile_revision),
            )
            .route(
                "/api/profile/fields/{field_key}/restrict",
                post(restrict_profile_field),
            )
            .route(
                "/api/profile/media/photo",
                post(upload_profile_photo)
                    .layer(DefaultBodyLimit::max(MediaKind::Photo.max_body_bytes())),
            )
            .route(
                "/api/profile/media/personal-document",
                post(upload_profile_document).layer(DefaultBodyLimit::max(
                    MediaKind::PersonalDocument.max_body_bytes(),
                )),
            )
            .route(
                "/api/profile/media/{id}",
                get(profile_media).delete(withdraw_profile_media),
            )
            .route(
                "/api/family/persons/{person_key}/profile",
                get(family_profile),
            )
            .route(
                "/api/family/persons/{person_key}/history",
                get(family_history),
            )
            .route(
                "/api/family/persons/{person_key}/media/{media_id}",
                get(family_media),
            )
            .route("/api/family/photographs", get(family_photograph_gallery))
            .route(
                "/api/family/photographs/{asset_id}/thumbnail",
                get(family_photograph_thumbnail),
            )
            .route(
                "/api/family/photographs/{asset_id}/original",
                get(family_photograph_original),
            )
            .route("/api/branch-review/grants/me", get(branch_review_grants))
            .route(
                "/api/branch-review/submissions",
                get(branch_review_queue).post(create_branch_review_proposal),
            )
            .route(
                "/api/branch-review/submissions/{id}",
                get(branch_review_proposal),
            )
            .route(
                "/api/branch-review/persons/{person_key}/evidence",
                get(branch_review_person_evidence),
            )
            .route(
                "/api/branch-review/submissions/{id}/decision",
                post(decide_branch_review_proposal),
            )
            .route("/api/admin/roles", post(change_role))
            .route(
                "/api/admin/branch-reviewers",
                get(list_branch_reviewers).post(grant_branch_reviewers),
            )
            .route(
                "/api/admin/branch-reviewers/branches",
                get(branch_reviewer_branches),
            )
            .route(
                "/api/admin/branch-reviewers/{id}/revoke",
                post(revoke_branch_reviewer),
            )
            .route("/api/admin/reviews", get(review_queue))
            .route("/api/admin/narrative/passages", get(narrative_review_queue))
            .route("/api/admin/narrative/persons", get(narrative_review_people))
            .route(
                "/api/admin/narrative/passages/{id}",
                get(narrative_review_detail),
            )
            .route(
                "/api/admin/narrative/passages/{id}/links",
                post(narrative_review_link),
            )
            .route(
                "/api/admin/narrative/passages/{id}/review",
                post(narrative_review_passage),
            )
            .route("/api/admin/reviews/media/{id}", get(review_profile_media))
            .route("/api/admin/reviews/{id}", get(review_detail))
            .route("/api/admin/photographs", get(admin_photograph_list))
            .route(
                "/api/admin/photographs/{asset_id}/thumbnail",
                get(admin_photograph_thumbnail),
            )
            .route(
                "/api/admin/photographs/{asset_id}/original",
                get(admin_photograph_original),
            )
            .route(
                "/api/admin/photographs/{asset_id}/visibility",
                post(admin_photograph_visibility),
            )
            .route(
                "/api/admin/photographs/{asset_id}/caption",
                post(admin_photograph_caption),
            )
            .route(
                "/api/admin/photographs/{asset_id}/propose",
                post(admin_photograph_propose),
            )
            .route(
                "/api/admin/photographs/{asset_id}/links/{person_id}",
                post(admin_photograph_link),
            )
            .route("/api/genealogy/records", get(genealogy_records))
            .route("/api/genealogy/nomination-persons", get(nomination_people))
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
                "/api/genealogy/documents/{version}/extraction",
                get(read_document_extraction).post(extract_document_records),
            )
            .route(
                "/api/genealogy/documents/{version}/apply",
                post(apply_document_extraction),
            )
            .route(
                "/api/genealogy/document-proposals/{id}/decision",
                post(decide_document_proposal),
            )
            .route(
                "/api/genealogy/documents/{version}/authority/preview",
                post(preview_document_authority),
            )
            .route(
                "/api/genealogy/documents/{version}/authority/apply",
                post(apply_document_authority),
            )
            .route("/api/genealogy/authority", get(list_authority_decisions))
            .route(
                "/api/genealogy/authority/{id}/reverse",
                post(reverse_authority_decision),
            )
            .route(
                "/api/admin/family-members/candidates",
                get(family_member_candidates),
            )
            .route("/api/admin/family-members/bulk/preview", post(bulk_preview))
            .route("/api/admin/family-members/bulk/prepare", post(bulk_prepare))
            .route(
                "/api/admin/family-members/bulk/recipients",
                post(bulk_recipient),
            )
            .route("/api/admin/family-members/bulk/send", post(bulk_send))
            .route(
                "/api/admin/family-members/bulk/batches/{id}",
                get(bulk_batch_status),
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
            .route("/api/admin/family-members/people", get(admin_member_people))
            .route(
                "/api/admin/family-members/links/{id}/revoke",
                post(revoke_link),
            )
            .route("/api/admin/family-members", get(family_members))
            .route(
                "/api/admin/family-members/password-reset",
                post(family_member_password_reset),
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
            | Operation::DocumentSubmit
            | Operation::DocumentRead
            | Operation::DocumentReview
            | Operation::DocumentProcess
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

// --- internal browser-session endpoints ------------------------------------
//
// These are server-to-server only. The Next.js server holds the opaque `rag_sid`
// cookie and calls them with `X-Internal-Secret`; the browser never has that
// secret. They never return a refresh token.

#[derive(Deserialize)]
struct InternalSessionCreate {
    session_id: String,
    id_token: String,
    refresh_token: String,
}

#[derive(Serialize)]
struct InternalSessionCreated {
    ok: bool,
    absolute_expires_in: i64,
    idle_expires_in: i64,
}

#[derive(Serialize)]
struct InternalSessionRenewed {
    ok: bool,
    id_token: String,
    expires_in: i64,
    absolute_expires_in: i64,
}

#[allow(clippy::result_large_err)]
fn authorize_internal(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    let Some(expected) = state.config.web_session_secret.as_deref() else {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_configured",
            "browser session renewal is not configured",
        ));
    };
    let Some(provided) = headers
        .get("x-internal-secret")
        .and_then(|value| value.to_str().ok())
    else {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "the internal credential is required",
        ));
    };
    if !constant_time_eq(expected.as_bytes(), provided.as_bytes()) {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "the internal credential is not valid",
        ));
    }
    Ok(())
}

/// Constant-time byte comparison for the internal shared secret. Length is
/// compared first, which only reveals the secret's length, never its content.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

fn session_error(err: SessionError) -> Response {
    match err {
        SessionError::NotConfigured => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_configured",
            "browser session renewal is not configured",
        ),
        SessionError::AlreadyExists => error(
            StatusCode::CONFLICT,
            "session_exists",
            "the browser session already exists",
        ),
        SessionError::NotFound => error(
            StatusCode::NOT_FOUND,
            "unknown_session",
            "no such browser session",
        ),
        SessionError::Crypto => error(
            StatusCode::BAD_REQUEST,
            "invalid_session",
            "the browser session credentials are invalid",
        ),
        SessionError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "session_store_unavailable",
            "the browser session store is unavailable",
        ),
    }
}

async fn create_internal_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<InternalSessionCreate>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers) {
        return response;
    }
    if !state.session.enabled() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_configured",
            "browser session renewal is not configured",
        );
    }
    let config = state.config.clone();
    let session = state.session.clone();
    let auth = state.auth.clone();
    let jwks = state.jwks.clone();
    let result = tokio::task::spawn_blocking(move || {
        let identity =
            validate_token(&body.id_token, &auth, &jwks).map_err(|_| SessionError::Crypto)?;
        let id_token_exp = crate::oidc::token_exp(&body.id_token).ok_or(SessionError::Crypto)?;
        let mut client = browser_session::connect(&config)?;
        browser_session::create_session(
            &mut client,
            &session,
            NewSession {
                session_id: &body.session_id,
                identity: &identity,
                id_token: &body.id_token,
                id_token_exp,
                refresh_token: &body.refresh_token,
            },
        )
    })
    .await;
    match result {
        Ok(Ok(created)) => Json(InternalSessionCreated {
            ok: true,
            absolute_expires_in: created.absolute_expires_in,
            idle_expires_in: created.idle_expires_in,
        })
        .into_response(),
        Ok(Err(err)) => session_error(err),
        Err(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "session_store_unavailable",
            "the browser session store is unavailable",
        ),
    }
}

async fn renew_internal_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers) {
        return response;
    }
    if !state.session.enabled() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_configured",
            "browser session renewal is not configured",
        );
    }
    let config = state.config.clone();
    let session = state.session.clone();
    let auth = state.auth.clone();
    let jwks = state.jwks.clone();
    let result = tokio::task::spawn_blocking(move || {
        let provider = HttpRefreshProvider::new(&session).ok_or(SessionError::NotConfigured)?;
        let mut client = browser_session::connect(&config)?;
        let validate = |token: &str| validate_token(token, &auth, &jwks);
        browser_session::renew_or_touch(&mut client, &session, &provider, &validate, &session_id)
    })
    .await;
    match result {
        Ok(Ok(RenewOutcome::Current {
            id_token,
            expires_in,
            absolute_expires_in,
        }))
        | Ok(Ok(RenewOutcome::Renewed {
            id_token,
            expires_in,
            absolute_expires_in,
        })) => Json(InternalSessionRenewed {
            ok: true,
            id_token,
            expires_in,
            absolute_expires_in,
        })
        .into_response(),
        Ok(Ok(RenewOutcome::Expired)) => error(
            StatusCode::UNAUTHORIZED,
            "session_expired",
            "the browser session has expired",
        ),
        Ok(Ok(RenewOutcome::Busy { retry_after_ms })) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "ok": false,
                "error": "renewal_in_progress",
                "message": "another request is renewing the session",
                "retry_after_ms": retry_after_ms,
            })),
        )
            .into_response(),
        Ok(Ok(RenewOutcome::Unavailable)) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "provider_unavailable",
            "the identity provider is temporarily unavailable",
        ),
        Ok(Err(err)) => session_error(err),
        Err(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "session_store_unavailable",
            "the browser session store is unavailable",
        ),
    }
}

async fn revoke_internal_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers) {
        return response;
    }
    // Revocation deliberately works even when renewal is disabled: it only
    // clears local credentials and never needs the encryption key.
    let config = state.config.clone();
    let session = state.session.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = browser_session::connect(&config)?;
        browser_session::revoke_session(&mut client, &session, &session_id)
    })
    .await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(err)) => session_error(err),
        Err(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "session_store_unavailable",
            "the browser session store is unavailable",
        ),
    }
}

/// Reads the authenticated member's own private profile.
///
/// Ownership is derived only from the verified `(issuer, subject)` pair and the
/// reviewed link it resolved elsewhere. Approved values and owner-private
/// pending revisions are returned separately. An unlinked account receives
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
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        let profile =
            onboarding::read_owner_profile(&mut genealogy, &account).map_err(revision_error)?;
        let media = if profile.is_some() {
            onboarding::read_owner_media(&mut genealogy, &account).map_err(media_error)?
        } else {
            Vec::new()
        };
        Ok((profile, media))
    })
    .await;

    match result {
        Ok(Ok((Some(view), media))) => Json(serde_json::json!({
            "ok": true,
            "linked": true,
            "profile": owner_profile_json(&view),
            "media": media.iter().map(|item| {
                let mut value = media_json(&item.asset);
                // The owner needs to distinguish "approved for the family" from
                // "approved but restricted from the family" and from an
                // owner-private approval, so the effective state travels with the
                // row. It is the same flag the family predicate uses.
                if let Some(object) = value.as_object_mut() {
                    object.insert(
                        "restricted_from_family".to_string(),
                        serde_json::json!(item.restricted_from_family),
                    );
                }
                value
            }).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Ok((None, _))) => Json(serde_json::json!({
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

#[derive(Deserialize)]
struct CreateProfileDraftRequest {
    field_key: String,
    value: String,
    #[serde(default)]
    visibility: Option<String>,
}

#[derive(Deserialize)]
struct UpdateProfileDraftRequest {
    value: String,
    #[serde(default)]
    visibility: Option<String>,
}

#[derive(Deserialize)]
struct ReviewProfileRevisionRequest {
    decision: String,
    #[serde(default)]
    note: Option<String>,
}

#[allow(clippy::result_large_err)]
async fn create_profile_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateProfileDraftRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileWrite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let visibility = match parse_visibility(request.visibility.as_deref()) {
        Ok(visibility) => visibility,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let field_key = request.field_key;
    let value = request.value;
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::create_owned_draft(&mut genealogy, &account, &field_key, &value, visibility)
            .map_err(revision_error)
    })
    .await;
    revision_response(result)
}

#[allow(clippy::result_large_err)]
async fn update_profile_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(revision_id): Path<i64>,
    Json(request): Json<UpdateProfileDraftRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileWrite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let visibility = match parse_optional_visibility(request.visibility.as_deref()) {
        Ok(visibility) => visibility,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let value = request.value;
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::update_owned_draft(&mut genealogy, &account, revision_id, &value, visibility)
            .map_err(revision_error)
    })
    .await;
    revision_response(result)
}

#[allow(clippy::result_large_err)]
async fn submit_profile_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(revision_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileWrite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::submit_owned_revision(&mut genealogy, &account, revision_id)
            .map_err(revision_error)
    })
    .await;
    revision_response(result)
}

#[allow(clippy::result_large_err)]
async fn cancel_profile_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(revision_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileWrite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::cancel_owned_draft(&mut genealogy, &account, revision_id)
            .map_err(revision_error)
    })
    .await;
    revision_response(result)
}

#[allow(clippy::result_large_err)]
async fn review_profile_revision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(revision_id): Path<i64>,
    Json(request): Json<ReviewProfileRevisionRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let decision = match parse_review_decision(&request.decision) {
        Ok(decision) => decision,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let reviewer = onboarding::identity_fingerprint(&issuer, &subject);
    let note = request.note;
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::review_revision(
            &mut genealogy,
            revision_id,
            &reviewer,
            decision,
            note.as_deref(),
        )
        .map_err(revision_error)
    })
    .await;
    revision_response(result)
}

#[derive(Deserialize)]
struct BranchReviewerGrantRequest {
    issuer: String,
    subject: String,
    branch_keys: Vec<String>,
    reason: String,
}

#[derive(Deserialize)]
struct BranchReviewerRevokeRequest {
    reason: String,
}

#[derive(Deserialize)]
struct BranchReviewerListQuery {
    account_id: Option<i64>,
    grant_cursor: Option<i64>,
    event_cursor: Option<i64>,
    limit: Option<i64>,
}

/// Lists grantable family branches.
#[allow(clippy::result_large_err)]
async fn branch_reviewer_branches(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::BranchReviewerAdmin) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::list_branches(&mut genealogy).map_err(branch_grant_error)
    })
    .await;
    match result {
        Ok(Ok(branches)) => Json(serde_json::json!({
            "ok": true,
            "branches": branches.iter().map(branch_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: branch list task failed: {err}");
            branch_admin_unavailable("the family branches could not be loaded")
        }
    }
}

/// Lists current/historical grants and the immutable grant/revocation events.
#[allow(clippy::result_large_err)]
async fn list_branch_reviewers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<BranchReviewerListQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::BranchReviewerAdmin) {
        return response;
    }
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_limit",
            "limit must be between 1 and 100",
        );
    }
    if query.grant_cursor.is_some_and(|cursor| cursor <= 0)
        || query.event_cursor.is_some_and(|cursor| cursor <= 0)
    {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_cursor",
            "cursors must be positive ids",
        );
    }
    let config = state.config.clone();
    let account_id = query.account_id;
    let grant_cursor = query.grant_cursor;
    let event_cursor = query.event_cursor;
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::read_grant_snapshot(
            &mut genealogy,
            account_id,
            grant_cursor,
            event_cursor,
            limit,
        )
        .map_err(branch_grant_error)
    })
    .await;
    match result {
        Ok(Ok(snapshot)) => Json(serde_json::json!({
            "ok": true,
            "limit": snapshot.limit,
            "grants": snapshot.grants.iter().map(grant_json).collect::<Vec<_>>(),
            "events": snapshot.events.iter().map(grant_event_json).collect::<Vec<_>>(),
            "grants_next_cursor": snapshot.grants_next_cursor,
            "events_next_cursor": snapshot.events_next_cursor,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: branch grant list task failed: {err}");
            branch_admin_unavailable("the branch reviewer grants could not be loaded")
        }
    }
}

/// Grants explicit reviewer authority for one or more branches.
#[allow(clippy::result_large_err)]
async fn grant_branch_reviewers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BranchReviewerGrantRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::BranchReviewerAdmin) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let issuer = request.issuer.trim().to_string();
    let subject = request.subject.trim().to_string();
    if issuer.is_empty() || subject.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_identity",
            "issuer and subject are required",
        );
    }
    if request.branch_keys.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_branch_selection",
            "at least one branch is required",
        );
    }
    if !valid_grant_reason(&request.reason) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_branch_reason",
            "a non-empty reason of at most 1000 characters is required",
        );
    }
    let config = state.config.clone();
    let actor = onboarding::identity_fingerprint(identity.issuer(), identity.subject());
    let branch_keys = request.branch_keys;
    let reason = request.reason;
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::grant_branches(&mut genealogy, &account, &branch_keys, &actor, &reason)
            .map_err(branch_grant_error)
    })
    .await;
    match result {
        Ok(Ok(grants)) => Json(serde_json::json!({
            "ok": true,
            "grants": grants.iter().map(grant_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: branch grant task failed: {err}");
            branch_admin_unavailable("the branch reviewer grant failed")
        }
    }
}

/// Revokes one active grant, preserving the grant/revocation history.
#[allow(clippy::result_large_err)]
async fn revoke_branch_reviewer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(grant_id): Path<i64>,
    Json(request): Json<BranchReviewerRevokeRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::BranchReviewerAdmin) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if !valid_grant_reason(&request.reason) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_branch_reason",
            "a non-empty reason of at most 1000 characters is required",
        );
    }
    let config = state.config.clone();
    let actor = onboarding::identity_fingerprint(identity.issuer(), identity.subject());
    let reason = request.reason;
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::revoke_grant(&mut genealogy, grant_id, &actor, &reason)
            .map_err(branch_grant_error)
    })
    .await;
    match result {
        Ok(Ok(grant)) => Json(serde_json::json!({
            "ok": true,
            "grant": grant_json(&grant),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: branch revoke task failed: {err}");
            branch_admin_unavailable("the branch reviewer revocation failed")
        }
    }
}

#[allow(clippy::result_large_err)]
fn valid_grant_reason(reason: &str) -> bool {
    let reason = reason.trim();
    !reason.is_empty() && reason.chars().count() <= onboarding::GRANT_REASON_MAX_CHARS
}

fn branch_grant_error(err: onboarding::BranchGrantError) -> Response {
    eprintln!("api: branch reviewer grant failed: {err}");
    match err {
        onboarding::BranchGrantError::EmptySelection
        | onboarding::BranchGrantError::BlankBranch
        | onboarding::BranchGrantError::UnknownBranch(_)
        | onboarding::BranchGrantError::InvalidActor
        | onboarding::BranchGrantError::InvalidReason => error(
            StatusCode::BAD_REQUEST,
            "invalid_branch_grant",
            "the branch grant request is invalid",
        ),
        onboarding::BranchGrantError::Unlinked => error(
            StatusCode::CONFLICT,
            "account_not_linked",
            "the account has no reviewed person link",
        ),
        onboarding::BranchGrantError::AlreadyActive(_) => error(
            StatusCode::CONFLICT,
            "branch_grant_exists",
            "an active grant already exists for this account and branch",
        ),
        onboarding::BranchGrantError::NotActive(_) => error(
            StatusCode::CONFLICT,
            "branch_grant_not_active",
            "the grant is not active",
        ),
        onboarding::BranchGrantError::UnknownGrant => error(
            StatusCode::NOT_FOUND,
            "branch_grant_not_found",
            "no such branch reviewer grant",
        ),
        onboarding::BranchGrantError::Database(_) => {
            branch_admin_unavailable("the branch reviewer store is unavailable")
        }
    }
}

fn branch_admin_unavailable(message: &'static str) -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "branch_reviewer_unavailable",
        message,
    )
}

fn branch_json(branch: &onboarding::BranchSummary) -> serde_json::Value {
    serde_json::json!({
        "family_branch_id": branch.family_branch_id,
        "branch_key": branch.branch_key,
        "branch_label": branch.branch_label,
    })
}

fn grant_json(grant: &onboarding::BranchReviewerGrant) -> serde_json::Value {
    serde_json::json!({
        "grant_id": grant.grant_id,
        "account_id": grant.account_id,
        "identity_fingerprint": grant.identity_fingerprint,
        "family_branch_id": grant.family_branch_id,
        "branch_key": grant.branch_key,
        "branch_label": grant.branch_label,
        "granted_by": grant.granted_by,
        "granted_reason": grant.granted_reason,
        "created_at": grant.created_at,
        "updated_at": grant.updated_at,
        "revoked_at": grant.revoked_at,
        "revoked_by": grant.revoked_by,
        "revoke_reason": grant.revoke_reason,
        "active": grant.active,
        "usable": grant.usable,
    })
}

fn grant_event_json(event: &onboarding::BranchGrantEvent) -> serde_json::Value {
    serde_json::json!({
        "event_id": event.event_id,
        "grant_id": event.grant_id,
        "account_id": event.account_id,
        "family_branch_id": event.family_branch_id,
        "branch_key": event.branch_key,
        "event": event.event,
        "actor": event.actor,
        "reason": event.reason,
        "created_at": event.created_at,
    })
}

#[derive(Deserialize)]
struct BranchReviewQueueQuery {
    branch: Option<String>,
    status: Option<String>,
    cursor: Option<i64>,
    limit: Option<i64>,
}

#[allow(clippy::result_large_err)]
async fn branch_review_person_evidence(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(person_key): Path<String>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::BranchReviewRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        branch_review::person_evidence(&mut genealogy, &account, &person_key)
            .map_err(branch_review_error)
    })
    .await;
    match result {
        Ok(Ok(evidence)) => Json(serde_json::json!({
            "ok": true,
            "evidence": evidence.iter().map(|item| serde_json::json!({
                "source_evidence_id": item.source_evidence_id,
                "document_ref": item.document_ref,
                "page": item.page,
                "locator": item.locator,
                "assertion_kind": item.assertion_kind,
                "assertion_key": item.assertion_key,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: branch evidence task failed: {err}");
            branch_review_unavailable()
        }
    }
}

#[derive(Deserialize)]
struct BranchProposalRequest {
    person_key: String,
    branch_key: String,
    assertion_kind: String,
    assertion_key: String,
    proposed_value: String,
    evidence: Vec<branch_review::ProposalEvidence>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize)]
struct BranchProposalDecisionRequest {
    decision: String,
    #[serde(default)]
    note: Option<String>,
}

/// Returns only the caller's effective branch grants.
#[allow(clippy::result_large_err)]
async fn branch_review_grants(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let identity = match authorize(&state, &headers, Operation::BranchReviewRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        branch_review::effective_grants(&mut genealogy, &account).map_err(branch_review_error)
    })
    .await;
    match result {
        Ok(Ok(grants)) => Json(serde_json::json!({
            "ok": true,
            "grants": grants.iter().map(|grant| serde_json::json!({
                "branch_key": grant.branch_key,
                "branch_label": grant.branch_label,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: branch grants task failed: {err}");
            branch_review_unavailable()
        }
    }
}

#[allow(clippy::result_large_err)]
async fn branch_review_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<BranchReviewQueueQuery>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::BranchReviewRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let limit = query.limit.unwrap_or(25);
    if !(1..=branch_review::PROPOSAL_LIMIT_MAX).contains(&limit) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_limit",
            "limit must be between 1 and 100",
        );
    }
    if query.cursor.is_some_and(|cursor| cursor <= 0) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_cursor",
            "cursor must be a positive proposal id",
        );
    }
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let branch = query.branch;
    let status = query.status;
    let cursor = query.cursor;
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        branch_review::list_proposals(
            &mut genealogy,
            &account,
            branch.as_deref(),
            status.as_deref(),
            cursor,
            limit,
        )
        .map_err(branch_review_error)
    })
    .await;
    match result {
        Ok(Ok(page)) => Json(serde_json::json!({
            "ok": true,
            "status": page.status,
            "limit": page.limit,
            "next_cursor": page.next_cursor,
            "submissions": page.submissions.iter().map(proposal_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: branch queue task failed: {err}");
            branch_review_unavailable()
        }
    }
}

#[allow(clippy::result_large_err)]
async fn branch_review_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(proposal_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::BranchReviewRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        branch_review::read_proposal(&mut genealogy, &account, proposal_id)
            .map_err(branch_review_error)
    })
    .await;
    match result {
        Ok(Ok(detail)) => Json(serde_json::json!({
            "ok": true,
            "proposal": proposal_json(&detail.summary),
            "current_assertion": detail.current_assertion.as_ref().map(current_assertion_json),
            "events": detail.events.iter().map(proposal_event_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: branch proposal task failed: {err}");
            branch_review_unavailable()
        }
    }
}

#[allow(clippy::result_large_err)]
async fn create_branch_review_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BranchProposalRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::BranchReviewWrite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        branch_review::propose_fact(
            &mut genealogy,
            &account,
            &request.person_key,
            &request.branch_key,
            &request.assertion_kind,
            &request.assertion_key,
            &request.proposed_value,
            &request.evidence,
            request.note.as_deref(),
        )
        .map_err(branch_review_error)
    })
    .await;
    match result {
        Ok(Ok(proposal_id)) => Json(serde_json::json!({
            "ok": true,
            "proposal_id": proposal_id,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: branch proposal create task failed: {err}");
            branch_review_unavailable()
        }
    }
}

#[allow(clippy::result_large_err)]
async fn decide_branch_review_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(proposal_id): Path<i64>,
    Json(request): Json<BranchProposalDecisionRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::BranchReviewWrite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        branch_review::decide_proposal(
            &mut genealogy,
            &account,
            proposal_id,
            &request.decision,
            request.note.as_deref(),
        )
        .map_err(branch_review_error)
    })
    .await;
    match result {
        Ok(Ok(decision)) => Json(serde_json::json!({
            "ok": true,
            "proposal_id": decision.proposal_id,
            "status": decision.status,
            "applied_assertion_id": decision.applied_assertion_id,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: branch proposal decision task failed: {err}");
            branch_review_unavailable()
        }
    }
}

fn proposal_json(proposal: &branch_review::ProposalSummary) -> serde_json::Value {
    serde_json::json!({
        "proposal_id": proposal.proposal_id,
        "person_key": proposal.person_key,
        "display_name": proposal.display_name,
        "branch_key": proposal.branch_key,
        "assertion_kind": proposal.assertion_kind,
        "assertion_key": proposal.assertion_key,
        "prior_value_text": proposal.prior_value_text,
        "prior_review_status": proposal.prior_review_status,
        "proposed_value_text": proposal.proposed_value_text,
        "evidence": proposal.evidence,
        "submitted_at": proposal.submitted_at,
        "status": proposal.status,
        "can_decide": proposal.can_decide,
        "decision_block": proposal.decision_block,
    })
}

fn current_assertion_json(assertion: &branch_review::CurrentAssertion) -> serde_json::Value {
    serde_json::json!({
        "assertion_id": assertion.assertion_id,
        "value_text": assertion.value_text,
        "review_status": assertion.review_status,
        "reviewed_at": assertion.reviewed_at,
    })
}

fn proposal_event_json(event: &branch_review::ProposalEvent) -> serde_json::Value {
    serde_json::json!({
        "event": event.event,
        "actor": event.actor,
        "note": event.note,
        "created_at": event.created_at,
    })
}

#[allow(clippy::result_large_err)]
fn branch_review_error(err: branch_review::BranchReviewError) -> Response {
    eprintln!("api: branch review failed: {err}");
    match err {
        branch_review::BranchReviewError::NotGranted
        | branch_review::BranchReviewError::SelfReview
        | branch_review::BranchReviewError::OwnerReview => error(
            StatusCode::FORBIDDEN,
            "branch_review_forbidden",
            "the account may not review this branch or submission",
        ),
        branch_review::BranchReviewError::UnknownProposal => error(
            StatusCode::NOT_FOUND,
            "branch_review_not_found",
            "no such submission is available to this reviewer",
        ),
        branch_review::BranchReviewError::OpenProposal
        | branch_review::BranchReviewError::InvalidTransition
        | branch_review::BranchReviewError::Stale => error(
            StatusCode::CONFLICT,
            "branch_review_conflict",
            "the submission conflicts with the current review state",
        ),
        branch_review::BranchReviewError::MissingBranch
        | branch_review::BranchReviewError::BranchAmbiguous
        | branch_review::BranchReviewError::NotInBranch
        | branch_review::BranchReviewError::InvalidField
        | branch_review::BranchReviewError::InvalidEvidence
        | branch_review::BranchReviewError::ReviewNoteRequired
        | branch_review::BranchReviewError::Review(_) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "branch_review_invalid",
            "the branch review request is invalid",
        ),
        branch_review::BranchReviewError::Database(_) => branch_review_unavailable(),
    }
}

fn branch_review_unavailable() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "branch_review_unavailable",
        "the branch review store is unavailable",
    )
}

#[derive(Deserialize)]
struct ReviewQueueQuery {
    status: Option<String>,
    cursor: Option<i64>,
    limit: Option<i64>,
}

/// Lists submitted profile revisions for an authorized administrator review.
///
/// Drafts are never returned. The query is bounded and ordered by descending
/// revision id, with the last id used as a stable cursor.
#[allow(clippy::result_large_err)]
async fn review_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReviewQueueQuery>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let (statuses, status_label) = match parse_review_statuses(query.status.as_deref()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let limit = query.limit.unwrap_or(25);
    if !(1..=100).contains(&limit) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_review_limit",
            "limit must be between 1 and 100",
        );
    }
    if query.cursor.is_some_and(|cursor| cursor <= 0) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_review_cursor",
            "cursor must be a positive revision id",
        );
    }
    let config = state.config.clone();
    let reviewer = onboarding::identity_fingerprint(identity.issuer(), identity.subject());
    let cursor = query.cursor;
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::read_review_queue(&mut genealogy, &reviewer, &statuses, cursor, limit)
            .map_err(revision_error)
    })
    .await;
    match result {
        Ok(Ok(queue)) => Json(serde_json::json!({
            "ok": true,
            "status": status_label,
            "limit": queue.limit,
            "next_cursor": queue.next_cursor,
            "entries": queue.entries.iter().map(review_entry_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: review queue task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "review_queue_failed",
                "the review queue could not be loaded",
            )
        }
    }
}

/// Reads one submitted revision, its events, and its same-field history.
#[allow(clippy::result_large_err)]
async fn review_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(revision_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let reviewer = onboarding::identity_fingerprint(identity.issuer(), identity.subject());
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::read_review_detail(&mut genealogy, &reviewer, revision_id)
            .map_err(revision_error)
    })
    .await;
    match result {
        Ok(Ok(detail)) => Json(serde_json::json!({
            "ok": true,
            "entry": review_entry_json(&detail.entry),
            "events": detail.events.iter().map(review_event_json).collect::<Vec<_>>(),
            "history": detail.history.iter().map(revision_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: review detail task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "review_detail_failed",
                "the review detail could not be loaded",
            )
        }
    }
}

const NARRATIVE_REVIEW_NOTE_MAX_CHARS: usize = 2000;

#[derive(Deserialize)]
struct NarrativeReviewQuery {
    status: Option<String>,
    language: Option<String>,
    person: Option<String>,
    unmatched: Option<bool>,
    link: Option<String>,
    visibility: Option<String>,
    document: Option<String>,
    page_from: Option<i32>,
    page_to: Option<i32>,
    search: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
}

#[allow(clippy::result_large_err)]
fn narrative_review_filters(
    query: NarrativeReviewQuery,
) -> Result<narrative::ReviewFilters, Response> {
    let status = query.status.unwrap_or_else(|| "pending".to_string());
    if !matches!(status.as_str(), "pending" | "approved" | "rejected" | "all") {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_narrative_status",
            "status must be pending, approved, rejected, or all",
        ));
    }
    let language = match query.language.as_deref() {
        None | Some("") | Some("all") => None,
        Some("vi") => Some("vi".to_string()),
        Some("en") => Some("en".to_string()),
        Some(_) => {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_narrative_language",
                "language must be all, vi, or en",
            ))
        }
    };
    let person_key = query
        .person
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if person_key.as_ref().is_some_and(|value| value.len() > 200) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_narrative_person",
            "person must be an exact person key of at most 200 characters",
        ));
    }
    let link = query.link.unwrap_or_else(|| "all".to_string());
    if !matches!(
        link.as_str(),
        "all" | "needs_confirmation" | "no_accepted" | "has_accepted" | "no_candidates"
    ) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_narrative_link",
            "link must be all, needs_confirmation, no_accepted, has_accepted, or no_candidates",
        ));
    }
    let visibility = match query.visibility.as_deref() {
        None | Some("") | Some("all") => None,
        Some("admin_only") => Some("admin_only".to_string()),
        Some("family") => Some("family".to_string()),
        Some(_) => {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_narrative_visibility",
                "visibility must be all, admin_only, or family",
            ))
        }
    };
    let document_ref = query
        .document
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if document_ref.as_ref().is_some_and(|value| value.len() > 500) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_narrative_document",
            "document must be a source document path of at most 500 characters",
        ));
    }
    let page_from = match query.page_from {
        Some(value) if value < 1 => {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_narrative_page",
                "page_from must be a positive PDF page number",
            ))
        }
        value => value,
    };
    let page_to = match query.page_to {
        Some(value) if value < 1 => {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_narrative_page",
                "page_to must be a positive PDF page number",
            ))
        }
        value => value,
    };
    if let (Some(from), Some(to)) = (page_from, page_to) {
        if from > to {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_narrative_page_range",
                "page_from must be less than or equal to page_to",
            ));
        }
    }
    let search = query
        .search
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if search
        .as_ref()
        .is_some_and(|value| value.chars().count() > 200)
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_narrative_search",
            "search must be at most 200 characters",
        ));
    }
    let offset = query.offset.unwrap_or(0);
    if offset < 0 {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_narrative_offset",
            "offset must be zero or greater",
        ));
    }
    let limit = query.limit.unwrap_or(narrative::REVIEW_QUEUE_DEFAULT_LIMIT);
    if !(1..=narrative::REVIEW_QUEUE_MAX_LIMIT).contains(&limit) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_narrative_limit",
            "limit must be between 1 and 200",
        ));
    }
    Ok(narrative::ReviewFilters {
        status,
        language,
        person_key,
        unmatched_only: query.unmatched.unwrap_or(false),
        link,
        visibility,
        document_ref,
        page_from,
        page_to,
        search,
        offset,
        limit,
    })
}

fn narrative_review_link_json(link: &narrative::ReviewLink) -> serde_json::Value {
    serde_json::json!({
        "person_key": link.person_key,
        "display_name": link.display_name,
        "link_status": link.link_status,
        "match_kind": link.match_kind,
        "evidence_note": link.evidence_note,
        "reviewed_by": link.reviewed_by,
        "reviewed_at": link.reviewed_at,
    })
}

fn narrative_review_passage_json(
    passage: &narrative::ReviewPassage,
    include_body: bool,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "passage_id": passage.passage_id,
        "language": passage.language,
        "section_title": passage.section_title,
        "document_ref": passage.document_ref,
        "page_start": passage.page_start,
        "page_end": passage.page_end,
        "review_status": passage.review_status,
        "visibility": passage.visibility,
        "body_excerpt": passage.excerpt,
        "updated_at": passage.updated_at,
    });
    if include_body {
        body["body"] = serde_json::Value::String(passage.body.clone());
    }
    body
}

fn narrative_review_person_json(person: &narrative::ReviewPersonContext) -> serde_json::Value {
    serde_json::json!({
        "person_key": person.person_key,
        "display_name": person.display_name,
        "aliases": person.aliases,
        "birth": person.birth,
        "death": person.death,
        "parents": person.parents,
        "spouses": person.spouses,
        "children": person.children,
    })
}

fn narrative_review_detail_json(detail: &narrative::ReviewPassageDetail) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "passage": narrative_review_passage_json(&detail.passage, true),
        "links": detail.links.iter().map(narrative_review_link_json).collect::<Vec<_>>(),
        "people": detail.people.iter().map(narrative_review_person_json).collect::<Vec<_>>(),
        "events": detail.events.iter().map(|event| serde_json::json!({
            "event_id": event.event_id,
            "person_key": event.person_key,
            "actor": event.actor,
            "action": event.action,
            "from_status": event.from_status,
            "to_status": event.to_status,
            "visibility": event.visibility,
            "note": event.note,
            "created_at": event.created_at,
        })).collect::<Vec<_>>(),
    })
}

fn narrative_review_error(err: GenealogyError) -> Response {
    eprintln!("api: narrative review failed: {err}");
    match err {
        GenealogyError::Review(message) | GenealogyError::Domain(message) => {
            let status = if message.starts_with("no such") {
                StatusCode::NOT_FOUND
            } else if message.contains("required") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::CONFLICT
            };
            error(status, "narrative_review_rejected", &message)
        }
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "genealogy_unavailable",
            "the genealogy store is unavailable",
        ),
    }
}

/// Lists passages for the administrator-only narrative review screen.
#[allow(clippy::result_large_err)]
async fn narrative_review_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NarrativeReviewQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::ProfileReview) {
        return response;
    }
    let filters = match narrative_review_filters(query) {
        Ok(filters) => filters,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        narrative::review_queue(&mut genealogy, &filters).map_err(narrative_review_error)
    })
    .await;
    match result {
        Ok(Ok(queue)) => {
            let entries = queue
                .entries
                .iter()
                .map(|entry| {
                    serde_json::json!({
                        "passage": narrative_review_passage_json(&entry.passage, false),
                        "links": entry.links.iter().map(narrative_review_link_json).collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>();
            Json(serde_json::json!({
                "ok": true,
                "status": queue.filters.status,
                "language": queue.filters.language,
                "person": queue.filters.person_key,
                "unmatched": queue.filters.unmatched_only,
                "link": queue.filters.link,
                "visibility": queue.filters.visibility,
                "document": queue.filters.document_ref,
                "page_from": queue.filters.page_from,
                "page_to": queue.filters.page_to,
                "search": queue.filters.search,
                "offset": queue.filters.offset,
                "limit": queue.filters.limit,
                "total": queue.total,
                "truncated": queue.truncated,
                "documents": queue.documents,
                "entries": entries,
            }))
            .into_response()
        }
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: narrative review queue task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "narrative_review_failed",
                "the narrative review queue could not be loaded",
            )
        }
    }
}

/// Reads one narrative passage and its complete review context.
#[allow(clippy::result_large_err)]
async fn narrative_review_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(passage_id): Path<i64>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::ProfileReview) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        narrative::review_detail(&mut genealogy, passage_id).map_err(narrative_review_error)
    })
    .await;
    match result {
        Ok(Ok(Some(detail))) => Json(narrative_review_detail_json(&detail)).into_response(),
        Ok(Ok(None)) => error(
            StatusCode::NOT_FOUND,
            "narrative_passage_not_found",
            "no such narrative passage",
        ),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: narrative review detail task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "narrative_review_failed",
                "the narrative review detail could not be loaded",
            )
        }
    }
}

#[derive(Deserialize)]
struct NarrativePersonQuery {
    q: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PersonSearchQuery {
    Empty,
    Search { term: String, limit: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersonSearchQueryError {
    TooLong,
    LimitOutOfRange,
}

/// Applies the shared q/limit rules without changing either endpoint's codes.
fn validate_person_search(
    query: &NarrativePersonQuery,
) -> Result<PersonSearchQuery, PersonSearchQueryError> {
    let term = query.q.clone().unwrap_or_default().trim().to_string();
    if term.chars().count() > 200 {
        return Err(PersonSearchQueryError::TooLong);
    }
    let limit = query.limit.unwrap_or(10);
    if !(1..=20).contains(&limit) {
        return Err(PersonSearchQueryError::LimitOutOfRange);
    }
    if term.is_empty() {
        return Ok(PersonSearchQuery::Empty);
    }
    Ok(PersonSearchQuery::Search { term, limit })
}

/// Searches people for the narrative review person selector.
#[allow(clippy::result_large_err)]
async fn narrative_review_people(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NarrativePersonQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::ProfileReview) {
        return response;
    }
    let (term, limit) = match validate_person_search(&query) {
        Ok(PersonSearchQuery::Empty) => {
            return Json(serde_json::json!({ "ok": true, "people": [] })).into_response()
        }
        Ok(PersonSearchQuery::Search { term, limit }) => (term, limit),
        Err(PersonSearchQueryError::TooLong) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_narrative_person_search",
                "q must be at most 200 characters",
            )
        }
        Err(PersonSearchQueryError::LimitOutOfRange) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_narrative_person_limit",
                "limit must be between 1 and 20",
            )
        }
    };
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        narrative::search_review_people(&mut genealogy, &term, limit)
            .map_err(narrative_review_error)
    })
    .await;
    match result {
        Ok(Ok(people)) => Json(serde_json::json!({
            "ok": true,
            "people": people.iter().map(narrative_review_person_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: narrative person search task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "narrative_review_failed",
                "the narrative person search failed",
            )
        }
    }
}

/// Searches the complete catalog with the same permission as nomination.
#[allow(clippy::result_large_err)]
async fn nomination_people(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NarrativePersonQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::GenealogyNominate) {
        return response;
    }
    let (term, limit) = match validate_person_search(&query) {
        Ok(PersonSearchQuery::Empty) => {
            return Json(serde_json::json!({ "ok": true, "people": [] })).into_response()
        }
        Ok(PersonSearchQuery::Search { term, limit }) => (term, limit),
        Err(PersonSearchQueryError::TooLong) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_nomination_person_search",
                "q must be at most 200 characters",
            )
        }
        Err(PersonSearchQueryError::LimitOutOfRange) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_nomination_person_limit",
                "limit must be between 1 and 20",
            )
        }
    };
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        let people = appointments::search_nomination_people(&mut genealogy, &term, limit)
            .map_err(narrative_review_error)?;
        Ok::<_, Response>(
            people
                .into_iter()
                .map(|person| {
                    serde_json::json!({
                        "record_id": person.person_key,
                        "person_label": person.display_name,
                        "birth": person.birth,
                        "branches": person.branches,
                    })
                })
                .collect::<Vec<_>>(),
        )
    })
    .await;
    match result {
        Ok(Ok(people)) => Json(serde_json::json!({
            "ok": true,
            "people": people,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: nomination person search task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nomination_search_failed",
                "the nomination person search failed",
            )
        }
    }
}

/// Admin person search for the link form. Read-only; shares the q/limit rules
/// and the catalog search with `nomination_people`, and adds each person's
/// active link state.
#[allow(clippy::result_large_err)]
async fn admin_member_people(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NarrativePersonQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::OnboardingLink) {
        return response;
    }
    let (term, limit) = match validate_person_search(&query) {
        Ok(PersonSearchQuery::Empty) => {
            return Json(serde_json::json!({ "ok": true, "people": [] })).into_response()
        }
        Ok(PersonSearchQuery::Search { term, limit }) => (term, limit),
        Err(PersonSearchQueryError::TooLong) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_member_person_search",
                "q must be at most 200 characters",
            )
        }
        Err(PersonSearchQueryError::LimitOutOfRange) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_member_person_limit",
                "limit must be between 1 and 20",
            )
        }
    };
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        let people = appointments::search_nomination_people(&mut genealogy, &term, limit)
            .map_err(narrative_review_error)?;
        let keys: Vec<String> = people
            .iter()
            .map(|person| person.person_key.clone())
            .collect();
        let linked = admin_members::linked_person_keys(&mut genealogy, &keys)
            .map_err(genealogy_unavailable)?;
        Ok::<_, Response>(
            people
                .into_iter()
                .map(|person| {
                    let is_linked = linked.contains(&person.person_key);
                    serde_json::json!({
                        "record_id": person.person_key,
                        "person_label": person.display_name,
                        "birth": person.birth,
                        "branches": person.branches,
                        "linked": is_linked,
                    })
                })
                .collect::<Vec<_>>(),
        )
    })
    .await;
    match result {
        Ok(Ok(people)) => Json(serde_json::json!({
            "ok": true,
            "people": people,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: admin member person search task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "member_person_search_failed",
                "the admin person search failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct NarrativeLinkRequest {
    person_key: String,
    decision: String,
    expected_status: String,
    #[serde(default)]
    note: Option<String>,
}

/// Confirms or rejects one person link without touching passage approval.
#[allow(clippy::result_large_err)]
async fn narrative_review_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(passage_id): Path<i64>,
    Json(request): Json<NarrativeLinkRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let accept = match request.decision.trim().to_ascii_lowercase().as_str() {
        "confirm" | "confirmed" => true,
        "reject" | "rejected" => false,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_link_decision",
                "decision must be confirm or reject",
            )
        }
    };
    let person_key = request.person_key.trim().to_string();
    if person_key.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "missing_person_key",
            "person_key is required",
        );
    }
    let expected_status = request.expected_status.trim().to_string();
    if expected_status.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "missing_expected_status",
            "expected_status is required",
        );
    }
    let note = request
        .note
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if note
        .as_ref()
        .is_some_and(|value| value.len() > NARRATIVE_REVIEW_NOTE_MAX_CHARS)
    {
        return error(
            StatusCode::BAD_REQUEST,
            "note_too_long",
            "the review note is too long",
        );
    }
    let reviewer = onboarding::identity_fingerprint(identity.issuer(), identity.subject());
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        narrative::review_link_checked(
            &mut genealogy,
            passage_id,
            &person_key,
            accept,
            &reviewer,
            note.as_deref(),
            Some(&expected_status),
        )
        .map_err(narrative_review_error)?;
        narrative::review_detail(&mut genealogy, passage_id).map_err(narrative_review_error)
    })
    .await;
    match result {
        Ok(Ok(Some(detail))) => Json(narrative_review_detail_json(&detail)).into_response(),
        Ok(Ok(None)) => error(
            StatusCode::NOT_FOUND,
            "narrative_passage_not_found",
            "no such narrative passage",
        ),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: narrative link review task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "narrative_review_failed",
                "the narrative link review failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct NarrativePassageRequest {
    decision: String,
    #[serde(default)]
    visibility: Option<String>,
    expected_status: String,
    expected_version: String,
    #[serde(default)]
    note: Option<String>,
}

/// Reviews one passage and its visibility without confirming any person link.
#[allow(clippy::result_large_err)]
async fn narrative_review_passage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(passage_id): Path<i64>,
    Json(request): Json<NarrativePassageRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let decision = match request.decision.trim().to_ascii_lowercase().as_str() {
        "approved" | "approve" => narrative::PassageDecision::Approved,
        "rejected" | "reject" => narrative::PassageDecision::Rejected,
        "pending" => narrative::PassageDecision::Pending,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_passage_decision",
                "decision must be approved, rejected, or pending",
            )
        }
    };
    let visibility = match request.visibility.as_deref() {
        None | Some("") => None,
        Some("family") => Some(narrative::PassageVisibility::Family),
        Some("admin_only") => Some(narrative::PassageVisibility::AdminOnly),
        Some(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_passage_visibility",
                "visibility must be family or admin_only",
            )
        }
    };
    let expected_status = request.expected_status.trim().to_string();
    if expected_status.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "missing_expected_status",
            "expected_status is required",
        );
    }
    let expected_version = request.expected_version.trim().to_string();
    if expected_version.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "missing_expected_version",
            "expected_version is required",
        );
    }
    let note = request
        .note
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if note
        .as_ref()
        .is_some_and(|value| value.len() > NARRATIVE_REVIEW_NOTE_MAX_CHARS)
    {
        return error(
            StatusCode::BAD_REQUEST,
            "note_too_long",
            "the review note is too long",
        );
    }
    let reviewer = onboarding::identity_fingerprint(identity.issuer(), identity.subject());
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut genealogy = open_genealogy_response(&config)?;
        narrative::review_passage_checked(
            &mut genealogy,
            passage_id,
            decision,
            visibility,
            &reviewer,
            note.as_deref(),
            Some(&expected_status),
            Some(&expected_version),
        )
        .map_err(narrative_review_error)?;
        narrative::review_detail(&mut genealogy, passage_id).map_err(narrative_review_error)
    })
    .await;
    match result {
        Ok(Ok(Some(detail))) => Json(narrative_review_detail_json(&detail)).into_response(),
        Ok(Ok(None)) => error(
            StatusCode::NOT_FOUND,
            "narrative_passage_not_found",
            "no such narrative passage",
        ),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: narrative passage review task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "narrative_review_failed",
                "the narrative passage review failed",
            )
        }
    }
}

/// Serves the pending media attached to a review decision. Approved
/// owner-private media, rejected media, and media owned by the reviewer are
/// intentionally not reachable here.
#[allow(clippy::result_large_err)]
async fn review_profile_media(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let reviewer = onboarding::identity_fingerprint(identity.issuer(), identity.subject());
    let result = tokio::task::spawn_blocking(move || {
        let storage = MediaStorage::new(config.media_root.clone());
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::read_review_media(&mut genealogy, &storage, &reviewer, media_id)
            .map_err(media_error)
    })
    .await;
    match result {
        Ok(Ok(download)) => media_download_response(download),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: review media task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "review_media_failed",
                "the review media could not be loaded",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
fn parse_review_statuses(value: Option<&str>) -> Result<(Vec<RevisionStatus>, String), Response> {
    let raw = value
        .unwrap_or("pending_review")
        .trim()
        .to_ascii_lowercase();
    let statuses = match raw.as_str() {
        "" | "pending" | "pending_review" => vec![RevisionStatus::PendingReview],
        "approved" => vec![RevisionStatus::Approved],
        "rejected" => vec![RevisionStatus::Rejected],
        "needs_changes" | "needs-changes" => vec![RevisionStatus::NeedsChanges],
        "all" => vec![
            RevisionStatus::PendingReview,
            RevisionStatus::Approved,
            RevisionStatus::Rejected,
            RevisionStatus::NeedsChanges,
        ],
        _ => {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_review_status",
                "status must be pending_review, approved, rejected, needs_changes, or all",
            ))
        }
    };
    Ok((statuses, raw))
}

fn review_entry_json(entry: &onboarding::ReviewEntry) -> serde_json::Value {
    serde_json::json!({
        "display_name": entry.display_name,
        "revision": revision_json(&entry.revision),
        "current_approved": entry.current_approved.as_ref().map(approved_json),
        "current_media": entry.current_media.as_ref().map(review_media_json),
        "media": entry.media.as_ref().map(review_media_json),
        "can_decide": entry.can_decide,
        "decision_block": entry.decision_block,
    })
}

fn review_media_json(media: &onboarding::ReviewMedia) -> serde_json::Value {
    serde_json::json!({
        "media_id": media.media_id,
        "kind": media.kind,
        "mime_type": media.mime_type,
        "file_name": media.file_name,
        "byte_size": media.byte_size,
        "width": media.width,
        "height": media.height,
    })
}

fn review_event_json(event: &onboarding::ReviewEvent) -> serde_json::Value {
    serde_json::json!({
        "event": event.event,
        "actor": event.actor,
        "note": event.note,
        "created_at": event.created_at,
    })
}

#[allow(clippy::result_large_err)]
fn revision_response(
    result: Result<Result<ProfileRevision, Response>, tokio::task::JoinError>,
) -> Response {
    match result {
        Ok(Ok(revision)) => Json(serde_json::json!({
            "ok": true,
            "revision": revision_json(&revision),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: profile revision task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "profile_revision_failed",
                "the profile revision operation failed",
            )
        }
    }
}

fn revision_json(revision: &ProfileRevision) -> serde_json::Value {
    serde_json::json!({
        "revision_id": revision.revision_id,
        "field_key": revision.field_key,
        "revision_number": revision.revision_number,
        "prior_value_text": revision.prior_value_text,
        "prior_approved_revision_number": revision.prior_approved_revision_number,
        "value": revision.new_value_text,
        "visibility": revision.visibility.as_str(),
        "status": revision.status.as_str(),
        "changed_by": revision.changed_by,
        "changed_at": revision.changed_at,
        "submitted_at": revision.submitted_at,
        "reviewed_at": revision.reviewed_at,
        "reviewed_by": revision.reviewed_by,
        "review_note": revision.review_note,
    })
}

fn approved_json(value: &ApprovedProfileValue) -> serde_json::Value {
    serde_json::json!({
        "field_key": value.field_key,
        "value": value.value_text,
        "visibility": value.visibility.as_str(),
        "updated_by": value.updated_by,
        "updated_at": value.updated_at,
        "approved_revision_number": value.approved_revision_number,
        "approved_revision_id": value.approved_revision_id,
        "restricted_from_family": value.restricted_from_family,
        "restriction_generation": value.restriction_generation,
    })
}

/// The authority decision that made one assertion current, or `null`.
fn assertion_authority_json(
    authority: &Option<genealogy_query::AssertionAuthority>,
) -> serde_json::Value {
    match authority {
        Some(authority) => serde_json::json!({
            "decision_id": authority.decision_id,
            "actor": authority.actor,
            "reason": authority.reason,
            "created_at": authority.created_at,
        }),
        None => serde_json::Value::Null,
    }
}

/// The structured date of one genealogy assertion, or `null`.
fn assertion_date_json(date: &Option<genealogy_query::AssertionDate>) -> serde_json::Value {
    match date {
        Some(date) => serde_json::json!({
            "start": date.start,
            "end": date.end,
            "precision": date.precision.as_str(),
            "qualifier": date.qualifier.as_str(),
        }),
        None => serde_json::Value::Null,
    }
}

/// One verified lifespan date on a tree person, or `null`.
fn tree_date_json(date: &Option<genealogy_tree::TreeDate>) -> serde_json::Value {
    match date {
        Some(date) => serde_json::json!({
            "start": date.start,
            "end": date.end,
            "precision": date.precision.as_str(),
            "qualifier": date.qualifier.as_str(),
        }),
        None => serde_json::Value::Null,
    }
}

fn owner_profile_json(profile: &OwnerProfile) -> serde_json::Value {
    let approved: Vec<serde_json::Value> = profile.approved.iter().map(approved_json).collect();
    let pending: Vec<serde_json::Value> = profile.pending.iter().map(revision_json).collect();
    let history: Vec<serde_json::Value> = profile.history.iter().map(revision_json).collect();
    serde_json::json!({
        "person_key": profile.person_key,
        "display_name": profile.display_name,
        "approved": approved,
        "pending": pending,
        "history": history,
    })
}

#[allow(clippy::result_large_err)]
fn safe_download_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

fn media_json(asset: &MediaAsset) -> serde_json::Value {
    serde_json::json!({
        "media_id": asset.media_id,
        "revision_id": asset.revision_id,
        "kind": asset.kind.as_str(),
        "mime_type": asset.mime_type,
        "byte_size": asset.byte_size,
        "file_name": asset.file_name,
        "visibility": asset.visibility.as_str(),
        "width": asset.width,
        "height": asset.height,
        "status": asset.status.as_str(),
        "created_at": asset.created_at,
    })
}

fn media_error(err: MediaError) -> Response {
    eprintln!("api: profile media failed: {err}");
    match err {
        MediaError::NotFound | MediaError::MissingFile => {
            error(StatusCode::NOT_FOUND, "media_not_found", "no such media")
        }
        MediaError::Unlinked => error(
            StatusCode::FORBIDDEN,
            "profile_forbidden",
            "the authenticated account may not access this media",
        ),
        MediaError::TooLarge => error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "media_too_large",
            "the uploaded file is too large",
        ),
        MediaError::UnsupportedType | MediaError::InvalidImage | MediaError::InvalidDocument => {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_media",
                "the uploaded file is not a valid supported media file",
            )
        }
        MediaError::OpenRevisionExists => error(
            StatusCode::CONFLICT,
            "media_revision_conflict",
            "an open revision already exists for this media slot",
        ),
        MediaError::InvalidTransition => error(
            StatusCode::CONFLICT,
            "media_revision_conflict",
            "only a pending media submission can be withdrawn",
        ),
        MediaError::Io(_) | MediaError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "media_unavailable",
            "the media store is unavailable",
        ),
    }
}

#[allow(clippy::result_large_err)]
async fn upload_profile_photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    upload_media(state, headers, multipart, MediaKind::Photo).await
}

#[allow(clippy::result_large_err)]
async fn upload_profile_document(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    upload_media(state, headers, multipart, MediaKind::PersonalDocument).await
}

#[allow(clippy::result_large_err)]
async fn upload_media(
    state: AppState,
    headers: HeaderMap,
    mut multipart: Multipart,
    kind: MediaKind,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileWrite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Some(length) = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
    {
        if length > kind.max_body_bytes() {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "media_too_large",
                "the uploaded file is too large",
            );
        }
    }
    let mut file: Option<(Option<String>, Vec<u8>)> = None;
    let mut visibility_value: Option<String> = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(_) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_multipart",
                    "the upload could not be parsed",
                )
            }
        };
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "file" => {
                if file.is_some() {
                    return error(
                        StatusCode::BAD_REQUEST,
                        "duplicate_file",
                        "only one file may be uploaded",
                    );
                }
                let file_name = field.file_name().map(str::to_string);
                let bytes = match field.bytes().await {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_multipart",
                            "the uploaded file could not be read",
                        )
                    }
                };
                file = Some((file_name, bytes.to_vec()));
            }
            "visibility" => {
                let text = match field.text().await {
                    Ok(text) => text,
                    Err(_) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_multipart",
                            "the visibility field could not be read",
                        )
                    }
                };
                visibility_value = Some(text);
            }
            _ => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "unknown_multipart_field",
                    "only file and visibility fields are accepted",
                )
            }
        }
    }
    let Some((file_name, bytes)) = file else {
        return error(
            StatusCode::BAD_REQUEST,
            "missing_file",
            "a file field is required",
        );
    };
    if bytes.len() > kind.max_bytes() {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "media_too_large",
            "the uploaded file is too large",
        );
    }
    let visibility = match parse_visibility(visibility_value.as_deref()) {
        Ok(visibility) => visibility,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let storage = MediaStorage::new(config.media_root.clone());
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::store_submission(
            &mut genealogy,
            &storage,
            &account,
            kind,
            file_name.as_deref(),
            &bytes,
            visibility,
        )
        .map_err(media_error)
    })
    .await;
    match result {
        Ok(Ok(asset)) => Json(serde_json::json!({
            "ok": true,
            "media": media_json(&asset),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: media upload task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "media_upload_failed",
                "the media upload failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
fn media_download_response(download: MediaDownload) -> Response {
    let disposition = if download.asset.kind == MediaKind::Photo {
        "inline"
    } else {
        "attachment"
    };
    let content_disposition = format!(
        "{disposition}; filename=\"{}\"",
        safe_download_name(&download.asset.file_name)
    );
    let Ok(content_type) = axum::http::HeaderValue::from_str(&download.asset.mime_type) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "media_response_failed",
            "the media response could not be built",
        );
    };
    let Ok(content_disposition) = axum::http::HeaderValue::from_str(&content_disposition) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "media_response_failed",
            "the media response could not be built",
        );
    };
    let content_length = download.bytes.len();
    let content_length = match axum::http::HeaderValue::from_str(&content_length.to_string()) {
        Ok(value) => value,
        Err(_) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "media_response_failed",
                "the media response could not be built",
            )
        }
    };
    let mut response = Response::new(Body::from(download.bytes));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, content_type);
    headers.insert(CONTENT_DISPOSITION, content_disposition);
    headers.insert(
        CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-content-type-options"),
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("content-security-policy"),
        axum::http::HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    headers.insert(CONTENT_LENGTH, content_length);
    response
}

#[allow(clippy::result_large_err)]
async fn profile_media(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let storage = MediaStorage::new(config.media_root.clone());
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::read_authorized_media(&mut genealogy, &storage, &account, media_id)
            .map_err(media_error)
    })
    .await;
    match result {
        Ok(Ok(download)) => media_download_response(download),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: media download task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "media_download_failed",
                "the media download failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn withdraw_profile_media(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileWrite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let storage = MediaStorage::new(config.media_root.clone());
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::withdraw_owned_media(&mut genealogy, &storage, &account, media_id)
            .map_err(media_error)
    })
    .await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({
            "ok": true,
            "withdrawn": true,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: media withdrawal task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "media_withdrawal_failed",
                "the media withdrawal failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
fn parse_optional_visibility(value: Option<&str>) -> Result<Option<ProfileVisibility>, Response> {
    match value {
        None => Ok(None),
        Some("owner_private") => Ok(Some(ProfileVisibility::OwnerPrivate)),
        Some("family") => Ok(Some(ProfileVisibility::Family)),
        Some(_) => Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_visibility",
            "visibility must be owner_private or family",
        )),
    }
}

#[allow(clippy::result_large_err)]
fn parse_visibility(value: Option<&str>) -> Result<ProfileVisibility, Response> {
    Ok(parse_optional_visibility(value)?.unwrap_or(ProfileVisibility::OwnerPrivate))
}

#[allow(clippy::result_large_err)]
fn parse_review_decision(value: &str) -> Result<ReviewDecision, Response> {
    match value.trim().to_ascii_lowercase().as_str() {
        "approve" | "approved" => Ok(ReviewDecision::Approve),
        "reject" | "rejected" => Ok(ReviewDecision::Reject),
        "needs_changes" => Ok(ReviewDecision::NeedsChanges),
        _ => Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_decision",
            "decision must be approve, reject, or needs_changes",
        )),
    }
}

/// Reads another member's approved, family-visible profile.
///
/// The Rust API is the authorization authority: the caller's identity, active
/// account, and reviewed person link are re-established here, and the service
/// re-checks membership in the same transaction as the read. No client-supplied
/// identity, role, visibility, or authorization status is trusted.
#[allow(clippy::result_large_err)]
async fn family_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(person_key): Path<String>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::FamilyProfileRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        let profile = onboarding::read_family_profile(&mut genealogy, &account, &person_key)
            .map_err(family_profile_error)?;
        let archive_photographs = onboarding::family_profile::read_family_archive_photographs(
            &mut genealogy,
            &account,
            &person_key,
        )
        .map_err(family_profile_error)?;
        Ok((profile, archive_photographs))
    })
    .await;
    match result {
        Ok(Ok((profile, archive_photographs))) => {
            let state_label = if profile.fields.is_empty()
                && profile.media.is_empty()
                && profile.dates.is_empty()
                && profile.history.is_empty()
                && archive_photographs.is_empty()
            {
                "empty"
            } else {
                "published"
            };
            Json(serde_json::json!({
                "ok": true,
                "state": state_label,
                "person": {
                    "person_key": profile.person_key,
                    "display_name": profile.display_name,
                },
                "fields": profile.fields.iter().map(|field| serde_json::json!({
                    "field_key": field.field_key,
                    "value": field.value,
                    "visibility": field.visibility,
                    "approved_revision_number": field.approved_revision_number,
                    "updated_at": field.updated_at,
                })).collect::<Vec<_>>(),
                // Verified genealogy dates only. A date under review, disputed,
                // or unclassifiable is never presented as a family fact.
                "dates": profile.dates.iter().map(|date| serde_json::json!({
                    "kind": date.kind,
                    "value": date.value_text,
                    "start": date.start,
                    "end": date.end,
                    "precision": date.precision,
                    "qualifier": date.qualifier,
                })).collect::<Vec<_>>(),
                // Approved archive history, separate from the member's own
                // profile fields. Pending, rejected, admin-only, restricted, and
                // superseded passages are excluded server-side.
                "history": profile.history.iter().map(|passage| serde_json::json!({
                    "passage_id": passage.passage_id,
                    "language": passage.language,
                    "document_ref": passage.document_ref,
                    "section_title": passage.section_title,
                    "page_start": passage.page_start,
                    "page_end": passage.page_end,
                    "body": passage.body,
                })).collect::<Vec<_>>(),
                // Approved, family-visible, unrestricted assets only. The bytes
                // are served by the person-scoped family media route, which
                // applies the same predicate.
                "media": profile.media.iter().map(|item| serde_json::json!({
                    "media_id": item.media_id,
                    "kind": item.kind,
                    "mime_type": item.mime_type,
                    "byte_size": item.byte_size,
                    "file_name": item.file_name,
                    "width": item.width,
                    "height": item.height,
                    "approved_revision_number": item.approved_revision_number,
                    "created_at": item.created_at,
                })).collect::<Vec<_>>(),
                // Published archive photographs whose person link a reviewer
                // confirmed. Separate from `media`, which is owner-uploaded
                // profile media. The bytes are served by the archive photograph
                // route, which applies the same publication predicate.
                "archive_photographs": archive_photographs.iter().map(archive_photograph_json).collect::<Vec<_>>(),
            }))
            .into_response()
        }
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: family profile task failed: {err}");
            family_profile_unavailable()
        }
    }
}

/// Reads one person's approved, family-visible archive history.
///
/// Administrators may read it without a family-person link; every other caller
/// keeps the transaction-bound membership check. The response contains only the
/// [`onboarding::read_family_history`] projection, so pending, rejected,
/// admin-only, restricted, and superseded text is never serialized.
#[allow(clippy::result_large_err)]
async fn family_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(person_key): Path<String>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::FamilyProfileRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let is_admin = identity.has_role(Role::Admin);
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::read_family_history(&mut genealogy, &account, &person_key, is_admin)
            .map_err(family_profile_error)
    })
    .await;
    match result {
        Ok(Ok(history)) => Json(serde_json::json!({
            "ok": true,
            "person": {
                "person_key": history.person_key,
                "display_name": history.display_name,
            },
            "history": family_history_json(&history.history),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: family history task failed: {err}");
            family_profile_unavailable()
        }
    }
}

/// The one JSON shape for a published passage in both family responses.
fn family_history_json(passages: &[narrative::FamilyPassage]) -> Vec<serde_json::Value> {
    passages
        .iter()
        .map(|passage| {
            serde_json::json!({
                "passage_id": passage.passage_id,
                "language": passage.language,
                "document_ref": passage.document_ref,
                "section_title": passage.section_title,
                "page_start": passage.page_start,
                "page_end": passage.page_end,
                "body": passage.body,
            })
        })
        .collect()
}

/// Maps a family profile failure without revealing restricted content.
///
/// An unlinked caller is refused before the person is looked up, so a 403 and a
/// 404 cannot be combined to probe which person keys exist.
fn family_profile_error(err: FamilyProfileError) -> Response {
    eprintln!("api: family profile request refused: {err}");
    match err {
        FamilyProfileError::NotFamilyMember => error(
            StatusCode::FORBIDDEN,
            "family_profile_forbidden",
            "the authenticated account may not read this family profile",
        ),
        FamilyProfileError::UnknownPerson => error(
            StatusCode::NOT_FOUND,
            "family_profile_not_found",
            "no such family profile",
        ),
        FamilyProfileError::Database(_) => family_profile_unavailable(),
    }
}

fn family_profile_unavailable() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "family_profile_unavailable",
        "the family profile store is unavailable",
    )
}

/// Serves one approved, family-visible media asset to a linked family member.
///
/// The person key scopes the asset, so a valid media id belonging to another
/// person is `404`. Pending, rejected, owner-private, restricted, superseded, and
/// unprovenanced assets are indistinguishable from "not found", and the error
/// body never names the field, the revision, or the owner.
#[allow(clippy::result_large_err)]
async fn family_media(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((person_key, media_id)): Path<(String, i64)>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::FamilyProfileRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let storage = MediaStorage::new(config.media_root.clone());
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::read_family_media(&mut genealogy, &storage, &account, &person_key, media_id)
            .map_err(family_media_error)
    })
    .await;
    match result {
        Ok(Ok(download)) => media_download_response(download),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: family media task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "family_media_failed",
                "the family media download failed",
            )
        }
    }
}

fn family_media_error(err: MediaError) -> Response {
    eprintln!("api: family media request refused: {err}");
    match err {
        MediaError::Unlinked => error(
            StatusCode::FORBIDDEN,
            "family_media_forbidden",
            "the authenticated account may not read this family media",
        ),
        MediaError::MissingFile | MediaError::Io(_) | MediaError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "family_media_unavailable",
            "the family media store is unavailable",
        ),
        _ => error(
            StatusCode::NOT_FOUND,
            "family_media_not_found",
            "no such family media",
        ),
    }
}

// --- archive photograph read surfaces (AP-106) -------------------------------

/// The JSON shape shared by the family profile section and the gallery.
fn archive_photograph_json(photo: &FamilyArchivePhotograph) -> serde_json::Value {
    serde_json::json!({
        "asset_id": photo.asset_id,
        "caption": photo.caption,
        "mime_type": photo.mime_type,
        "byte_size": photo.byte_size,
        "width": photo.width,
        "height": photo.height,
        "document_version_id": photo.document_version_id,
        "corpus": photo.corpus,
        "page_number": photo.page_number,
        // The PDF page citation is provenance, not a storage location.
        "citation": format!("PDF page {}", photo.page_number),
        "created_at": photo.created_at,
        "thumbnail_url": format!("/api/family/photographs/{}/thumbnail", photo.asset_id),
        "original_url": format!("/api/family/photographs/{}/original", photo.asset_id),
    })
}

fn gallery_photograph_json(photo: &FamilyGalleryPhotograph) -> serde_json::Value {
    let mut value = archive_photograph_json(&photo.photograph);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "people".to_string(),
            serde_json::json!(photo
                .people
                .iter()
                .map(|person| serde_json::json!({
                    "person_key": person.person_key,
                    "display_name": person.display_name,
                }))
                .collect::<Vec<_>>()),
        );
    }
    value
}

/// The administrator read model. It carries provenance and review state only:
/// no storage key, content hash, filesystem path, or credential.
fn admin_photograph_json(photo: &AdminArchivePhotograph) -> serde_json::Value {
    let citation = photo
        .page_numbers
        .first()
        .map(|page| format!("PDF page {page}"))
        .unwrap_or_else(|| "PDF page unknown".to_string());
    serde_json::json!({
        "asset_id": photo.asset_id,
        "caption": photo.caption,
        "publication_state": photo.publication_state,
        "mime_type": photo.mime_type,
        "byte_size": photo.byte_size,
        "width": photo.width,
        "height": photo.height,
        "object_id": photo.object_id,
        "classification": photo.classification,
        "classification_rule": photo.classification_rule,
        "document_version_id": photo.document_version_id,
        "corpus": photo.corpus,
        "page_numbers": photo.page_numbers,
        "citation": citation,
        "created_at": photo.created_at,
        "thumbnail_url": format!("/api/admin/photographs/{}/thumbnail", photo.asset_id),
        "original_url": format!("/api/admin/photographs/{}/original", photo.asset_id),
        "links": photo
            .links
            .iter()
            .map(|link| serde_json::json!({
                "person_id": link.person_id,
                "person_key": link.person_key,
                "display_name": link.display_name,
                "status": link.status,
                "decided_by": link.decided_by,
                "decided_at": link.decided_at,
                "source_page": link.source_page,
                "supporting_text": link.supporting_text,
                "confidence": link.confidence,
                "confidence_basis": link.confidence_basis,
                "ambiguity_note": link.ambiguity_note,
                "match_kind": link.match_kind,
                "candidate_rank": link.candidate_rank,
                "proposal_origin": link.proposal_origin,
            }))
            .collect::<Vec<_>>(),
    })
}

fn photograph_filename(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "photograph.jpg",
        "image/png" => "photograph.png",
        "image/webp" => "photograph.webp",
        "image/gif" => "photograph.gif",
        _ => "photograph",
    }
}

#[allow(clippy::result_large_err)]
fn photograph_download_response(download: ArchivePhotographDownload) -> Response {
    let content_disposition = format!(
        "inline; filename=\"{}\"",
        photograph_filename(&download.mime_type)
    );
    let Ok(content_type) = axum::http::HeaderValue::from_str(&download.mime_type) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "photograph_response_failed",
            "the photograph response could not be built",
        );
    };
    let Ok(content_disposition) = axum::http::HeaderValue::from_str(&content_disposition) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "photograph_response_failed",
            "the photograph response could not be built",
        );
    };
    let content_length = match axum::http::HeaderValue::from_str(&download.bytes.len().to_string())
    {
        Ok(value) => value,
        Err(_) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "photograph_response_failed",
                "the photograph response could not be built",
            )
        }
    };
    let mut response = Response::new(Body::from(download.bytes));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, content_type);
    headers.insert(CONTENT_DISPOSITION, content_disposition);
    headers.insert(
        CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-content-type-options"),
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("content-security-policy"),
        axum::http::HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    headers.insert(CONTENT_LENGTH, content_length);
    response
}

fn archive_photograph_error(err: ArchivePhotographError) -> Response {
    eprintln!("api: archive photograph request refused: {err}");
    match err {
        ArchivePhotographError::NotFound => error(
            StatusCode::NOT_FOUND,
            "photograph_not_found",
            "no such photograph",
        ),
        ArchivePhotographError::NotFamilyMember => error(
            StatusCode::FORBIDDEN,
            "photograph_forbidden",
            "the authenticated account may not read this photograph",
        ),
        ArchivePhotographError::StoreUnavailable | ArchivePhotographError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "photograph_unavailable",
            "the archive photograph store is unavailable",
        ),
    }
}

fn photograph_gallery_error(err: FamilyProfileError) -> Response {
    eprintln!("api: photograph gallery request refused: {err}");
    match err {
        FamilyProfileError::NotFamilyMember => error(
            StatusCode::FORBIDDEN,
            "photographs_forbidden",
            "the authenticated account may not read the family photograph gallery",
        ),
        FamilyProfileError::UnknownPerson => error(
            StatusCode::NOT_FOUND,
            "photographs_not_found",
            "no such photograph gallery",
        ),
        FamilyProfileError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "photographs_unavailable",
            "the archive photograph store is unavailable",
        ),
    }
}

fn admin_photograph_error(err: FamilyProfileError) -> Response {
    eprintln!("api: admin photograph read failed: {err}");
    match err {
        FamilyProfileError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "photographs_unavailable",
            "the archive photograph store is unavailable",
        ),
        _ => error(
            StatusCode::NOT_FOUND,
            "photograph_not_found",
            "no such photograph",
        ),
    }
}

fn photograph_review_error(err: GenealogyError) -> Response {
    eprintln!("api: photograph review failed: {err}");
    match err {
        GenealogyError::Domain(_) | GenealogyError::Review(_) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_photograph_review",
            "the photograph review request is not valid",
        ),
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "photograph_review_unavailable",
            "the photograph review store is unavailable",
        ),
    }
}

fn review_decision_json(decision: photograph_review::Decision) -> serde_json::Value {
    match decision {
        photograph_review::Decision::Changed { prior, next, event } => serde_json::json!({
            "ok": true,
            "changed": true,
            "prior": prior,
            "next": next,
            "operation": event.operation,
        }),
        photograph_review::Decision::NoChange => serde_json::json!({
            "ok": true,
            "changed": false,
        }),
    }
}

fn stored_layout_proposal_json(
    proposal: &photograph_review::StoredLayoutProposal,
) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "candidates": proposal
            .candidates
            .iter()
            .map(|candidate| serde_json::json!({
                "person_id": candidate.person_id,
                "person_key": candidate.person_key,
                "display_name": candidate.display_name,
                "match_kind": candidate.match_kind,
                "confidence": candidate.confidence,
                "source_kind": candidate.source_kind,
                "source_page": candidate.source_page,
                "supporting_text": candidate.supporting_text,
                "confidence_basis": candidate.confidence_basis,
                "ambiguity": candidate.ambiguity,
                "candidate_rank": candidate.candidate_rank,
            }))
            .collect::<Vec<_>>(),
        "unlinked_reason": proposal.unlinked_reason,
        "preserved_decisions": proposal.preserved_decisions,
    })
}

/// The family gallery. `Operation::GenealogyRead` is the family read operation;
/// the service additionally requires an active family link.
#[allow(clippy::result_large_err)]
async fn family_photograph_gallery(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let identity = match authorize(&state, &headers, Operation::GenealogyRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::family_profile::read_family_gallery(&mut genealogy, &account)
            .map_err(photograph_gallery_error)
    })
    .await;
    match result {
        Ok(Ok(photographs)) => Json(serde_json::json!({
            "ok": true,
            "photographs": photographs.iter().map(gallery_photograph_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: photograph gallery task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "photographs_failed",
                "the photograph gallery request failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn family_photograph_thumbnail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset_id): Path<i64>,
) -> Response {
    family_photograph_bytes(state, headers, asset_id).await
}

#[allow(clippy::result_large_err)]
async fn family_photograph_original(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset_id): Path<i64>,
) -> Response {
    family_photograph_bytes(state, headers, asset_id).await
}

/// Thumbnail and original use the same authorization and publication predicate.
/// No derived thumbnail is stored; both routes return the stored photograph so a
/// pending or restricted asset cannot be probed through either one.
#[allow(clippy::result_large_err)]
async fn family_photograph_bytes(state: AppState, headers: HeaderMap, asset_id: i64) -> Response {
    let identity = match authorize(&state, &headers, Operation::GenealogyRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        let store = PhotographStore::default();
        onboarding::family_profile::read_family_archive_photograph(
            &mut genealogy,
            &store,
            &account,
            asset_id,
        )
        .map_err(archive_photograph_error)
    })
    .await;
    match result {
        Ok(Ok(download)) => photograph_download_response(download),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: photograph download task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "photograph_download_failed",
                "the photograph download failed",
            )
        }
    }
}

/// The administrator review list. `Operation::DocumentReview` is the
/// document-review operation; every publication state is visible to the reviewer.
#[allow(clippy::result_large_err)]
async fn admin_photograph_list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::DocumentReview) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::family_profile::read_admin_archive_photographs(&mut genealogy)
            .map_err(admin_photograph_error)
    })
    .await;
    match result {
        Ok(Ok(photographs)) => Json(serde_json::json!({
            "ok": true,
            "photographs": photographs.iter().map(admin_photograph_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: admin photograph list task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "photographs_failed",
                "the photograph review list failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn admin_photograph_thumbnail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset_id): Path<i64>,
) -> Response {
    admin_photograph_bytes(state, headers, asset_id).await
}

#[allow(clippy::result_large_err)]
async fn admin_photograph_original(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset_id): Path<i64>,
) -> Response {
    admin_photograph_bytes(state, headers, asset_id).await
}

#[allow(clippy::result_large_err)]
async fn admin_photograph_bytes(state: AppState, headers: HeaderMap, asset_id: i64) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::DocumentReview) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut genealogy = open_genealogy_response(&config)?;
        let store = PhotographStore::default();
        onboarding::family_profile::read_admin_archive_photograph(&mut genealogy, &store, asset_id)
            .map_err(archive_photograph_error)
    })
    .await;
    match result {
        Ok(Ok(download)) => photograph_download_response(download),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: admin photograph download task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "photograph_download_failed",
                "the photograph download failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct PhotographVisibilityRequest {
    visibility: String,
}

#[derive(Deserialize)]
struct PhotographCaptionRequest {
    caption: String,
}

#[derive(Deserialize)]
struct PhotographLinkRequest {
    decision: String,
}

#[allow(clippy::result_large_err)]
async fn admin_photograph_visibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset_id): Path<i64>,
    Json(request): Json<PhotographVisibilityRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let visibility = request.visibility;
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut genealogy = open_genealogy_response(&config)?;
        photograph_review::set_visibility_db(&mut genealogy, asset_id, &visibility, &actor)
            .map(review_decision_json)
            .map_err(photograph_review_error)
    })
    .await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: photograph visibility task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "photograph_review_failed",
                "the photograph visibility change failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn admin_photograph_caption(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset_id): Path<i64>,
    Json(request): Json<PhotographCaptionRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let caption = request.caption;
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut genealogy = open_genealogy_response(&config)?;
        photograph_review::edit_caption_db(&mut genealogy, asset_id, &caption, &actor)
            .map(review_decision_json)
            .map_err(photograph_review_error)
    })
    .await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: photograph caption task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "photograph_review_failed",
                "the photograph caption change failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn admin_photograph_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((asset_id, person_id)): Path<(i64, i64)>,
    Json(request): Json<PhotographLinkRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let decision = request.decision.trim().to_ascii_lowercase();
    let confirm = match decision.as_str() {
        "confirm" | "confirmed" => true,
        "reject" | "rejected" => false,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_decision",
                "decision must be confirm or reject",
            )
        }
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut genealogy = open_genealogy_response(&config)?;
        let decided = if confirm {
            photograph_review::confirm_db(&mut genealogy, asset_id, person_id, &actor)
        } else {
            photograph_review::reject_db(&mut genealogy, asset_id, person_id, &actor)
        };
        decided
            .map(review_decision_json)
            .map_err(photograph_review_error)
    })
    .await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: photograph link task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "photograph_review_failed",
                "the photograph link decision failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn admin_photograph_propose(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut rag = open_rag(&config)?;
        let mut genealogy = open_genealogy_response(&config)?;
        let source =
            onboarding::family_profile::archive_photograph_source(&mut genealogy, asset_id)
                .map_err(admin_photograph_error)?
                .ok_or_else(|| {
                    error(
                        StatusCode::NOT_FOUND,
                        "photograph_not_found",
                        "no such photograph",
                    )
                })?;
        let storage = DocumentStorage::new(config.document_root.clone());
        let (_, bytes) = document_lifecycle::read_original(
            &mut rag,
            &storage,
            source.document_version_id,
            &source.corpus,
        )
        .map_err(lifecycle_error)?;
        let proposal =
            photograph_review::propose_layout_links_db(&mut genealogy, asset_id, &bytes, &actor)
                .map_err(photograph_review_error)?;
        Ok(stored_layout_proposal_json(&proposal))
    })
    .await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: photograph proposal task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "photograph_review_failed",
                "the photograph link proposal failed",
            )
        }
    }
}

/// Records an owner-initiated privacy restriction for one approved field.
///
/// Decision A2: the owner may stop family presentation immediately. The call is
/// idempotent for a field that is already restricted, never accepts a person,
/// issuer, subject, or visibility from the browser, and only reports success
/// after the authoritative restriction and its audit event have committed.
#[allow(clippy::result_large_err)]
async fn restrict_profile_field(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(field_key): Path<String>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::ProfileWrite) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let account = resolve_profile_account(&config, &issuer, &subject)?;
        let mut genealogy = open_genealogy_response(&config)?;
        onboarding::restrict_own_field(&mut genealogy, &account, &field_key)
            .map_err(restriction_error)
    })
    .await;
    match result {
        Ok(Ok(outcome)) => Json(serde_json::json!({
            "ok": true,
            "field_key": outcome.field_key,
            "generation": outcome.generation,
            "already_restricted": outcome.already_restricted,
            "restricted_from_family": true,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: profile restriction task failed: {err}");
            restriction_unavailable()
        }
    }
}

/// Maps a restriction failure without echoing content.
fn restriction_error(err: onboarding::RestrictionError) -> Response {
    eprintln!("api: profile restriction refused: {err}");
    match err {
        onboarding::RestrictionError::Unlinked => error(
            StatusCode::FORBIDDEN,
            "profile_forbidden",
            "the authenticated account may not restrict this profile field",
        ),
        onboarding::RestrictionError::InvalidField
        | onboarding::RestrictionError::NoApprovedValue
        | onboarding::RestrictionError::NotFamilyVisible => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_profile_restriction",
            "only an approved, family-visible profile value can be made private",
        ),
        onboarding::RestrictionError::Database(_) => restriction_unavailable(),
    }
}

fn restriction_unavailable() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "profile_restriction_unavailable",
        "the profile store is unavailable",
    )
}

fn revision_error(err: RevisionError) -> Response {
    eprintln!("api: profile revision failed: {err}");
    match err {
        RevisionError::UnknownPerson | RevisionError::UnknownRevision => error(
            StatusCode::NOT_FOUND,
            "profile_revision_not_found",
            "no such profile or revision",
        ),
        RevisionError::Unlinked | RevisionError::NotOwner | RevisionError::SelfReview => error(
            StatusCode::FORBIDDEN,
            "profile_forbidden",
            "the authenticated account may not modify this revision",
        ),
        RevisionError::OpenRevisionExists
        | RevisionError::InvalidTransition
        | RevisionError::VersionConflict
        | RevisionError::RestrictionConflict
        | RevisionError::MediaRevisionMissing => error(
            StatusCode::CONFLICT,
            "profile_revision_conflict",
            "the profile revision conflicts with the current state",
        ),
        RevisionError::InvalidField
        | RevisionError::InvalidValue
        | RevisionError::InvalidVisibility
        | RevisionError::ReviewNoteRequired => error(
            StatusCode::BAD_REQUEST,
            "invalid_profile_revision",
            "the profile revision request is invalid",
        ),
        RevisionError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "profile_revision_unavailable",
            "the profile revision store is unavailable",
        ),
    }
}

#[allow(clippy::result_large_err)]
fn resolve_profile_account(
    config: &Config,
    issuer: &str,
    subject: &str,
) -> Result<onboarding::IdentityAccount, Response> {
    let mut client =
        postgres::Client::connect(&config.database_url, postgres::NoTls).map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "identity_unavailable",
                "the identity store is unavailable",
            )
        })?;
    onboarding::resolve_account(&mut client, issuer, subject).map_err(account_map_error)
}

#[allow(clippy::result_large_err)]
fn open_genealogy_response(config: &Config) -> Result<postgres::Client, Response> {
    open_genealogy(config).map_err(|_| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "genealogy_unavailable",
            "the genealogy store is unavailable",
        )
    })
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
    /// An explicit answer to a "several possible matches" step. When present,
    /// the named person is answered directly instead of ranking the question
    /// again.
    #[serde(default)]
    person_key: Option<String>,
}

/// What the person path should resolve for this request.
enum PersonPlan {
    /// The question names a person; resolve the name deterministically.
    Named(person_search::PersonQuestion),
    /// The caller answered a selection step with a stable person key. The
    /// question is kept so the answer can still report the asked name.
    Selected {
        key: String,
        question: Option<person_search::PersonQuestion>,
    },
}

/// Builds the deterministic person question, or `None` when the question is
/// not about a named person and the document path keeps ownership of it.
fn person_plan(request: &QueryRequest, question: &str) -> Option<PersonPlan> {
    let extracted = person_search::extract_person_question(question);
    match request.person_key.as_deref().map(str::trim) {
        Some(key) if !key.is_empty() => Some(PersonPlan::Selected {
            key: key.to_string(),
            question: extracted,
        }),
        _ => extracted.map(PersonPlan::Named),
    }
}

/// Lists deterministic question suggestions drawn from approved archive sections.
///
/// The endpoint reads only `approved_documents`, the same publication boundary
/// retrieval uses, and returns labels rather than passage bodies. An empty
/// archive returns an empty list so the page can show an explicit empty state.
async fn query_suggestions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::Query) {
        return response;
    }
    let store = state.store.clone();
    let corpus = state.corpus.clone();
    let result = tokio::task::spawn_blocking(move || {
        store
            .suggestion_sections(&corpus, crate::suggestions::SUGGESTION_SCAN)
            .map(|sections| {
                crate::suggestions::questions(&sections, crate::suggestions::SUGGESTION_LIMIT)
            })
            .map_err(|err| err.to_string())
    })
    .await;
    match result {
        Ok(Ok(questions)) => Json(serde_json::json!({
            "ok": true,
            "questions": questions,
        }))
        .into_response(),
        Ok(Err(message)) => {
            eprintln!("api: query suggestions failed: {message}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "query_suggestions_unavailable",
                "question suggestions are unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: query suggestions task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "query_suggestions_failed",
                "the question suggestions task failed",
            )
        }
    }
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
    // A named-person question is answered deterministically from the
    // authorized genealogy store. Anything else keeps the document path below,
    // including a person question when the genealogy store is not configured.
    if let Some(plan) = person_plan(&request, &question) {
        if let Some(response) = person_answer(&state, plan).await {
            return response;
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
                        "corpus": passage.identity.corpus,
                        "document_version_id": passage.identity.document_version_id,
                        "chunk_id": passage.identity.chunk_id,
                        "content_hash": passage.identity.content_hash,
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

// --- person questions ------------------------------------------------------

/// Answers a person question from the authorized genealogy store, or returns
/// `None` so the caller falls back to the document path.
///
/// Authorization already happened in the caller. A genealogy configuration or
/// connection failure is not a query failure: the document path still owns the
/// request, and the failure is logged without credentials or source text.
async fn person_answer(state: &AppState, plan: PersonPlan) -> Option<Response> {
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config)?;
        let outcome = match &plan {
            PersonPlan::Named(question) => {
                person_search::resolve(&mut client, Some(question), None)
            }
            PersonPlan::Selected { key, question } => {
                person_search::resolve(&mut client, question.as_ref(), Some(key))
            }
        };
        outcome.map_err(|err| err.to_string())
    })
    .await;

    match result {
        Ok(Ok(person_search::PersonResult::NotPersonQuestion)) => None,
        Ok(Ok(outcome)) => {
            let (answer, insufficient, person) = person_payload(&outcome);
            Some(
                Json(serde_json::json!({
                    "answer": answer,
                    "insufficient": insufficient,
                    // A person answer cites stored assertions, never document
                    // chunks, so the document source list stays empty.
                    "sources": [],
                    "person": person,
                }))
                .into_response(),
            )
        }
        Ok(Err(message)) => {
            eprintln!("api: person lookup unavailable: {message}");
            None
        }
        Err(err) => {
            eprintln!("api: person lookup task failed: {err}");
            None
        }
    }
}

fn candidate_json(candidate: &person_search::Candidate) -> serde_json::Value {
    serde_json::json!({
        "person_key": candidate.person_key,
        "display_name": candidate.display_name,
        "generation": candidate.generation,
        "evidence_status": candidate.evidence_status,
        "match": candidate.match_kind.map(person_search::MatchKind::as_str),
        "matched_name": candidate.matched_name,
        "profile_url": format!("/family/{}", candidate.person_key),
        "recorded_parents": candidate
            .recorded_parents
            .iter()
            .map(|parent| serde_json::json!({
                "person_key": parent.person_key,
                "display_name": parent.display_name,
                "evidence_status": parent.evidence_status,
            }))
            .collect::<Vec<_>>(),
    })
}

/// Renders a person outcome as answer text plus the additive `person` payload.
///
/// The text states only what the stored records support. It never turns an
/// unverified assertion into a fact and never invents a biography.
fn person_payload(result: &person_search::PersonResult) -> (String, bool, serde_json::Value) {
    match result {
        person_search::PersonResult::NotPersonQuestion => {
            (String::new(), true, serde_json::json!(null))
        }
        person_search::PersonResult::NoMatch {
            query_name,
            catalog_empty,
            catalog_truncated,
        } => {
            let answer = if *catalog_empty {
                "The family record catalog has no imported people yet, so a person question \
                 cannot be answered. An operator must import the family catalog first."
                    .to_string()
            } else {
                format!(
                    "No person matching \"{query_name}\" is recorded in the family archive. \
                     A missing person record is separate from whether the archive's documents \
                     mention that name."
                )
            };
            (
                answer,
                true,
                serde_json::json!({
                    "outcome": "none",
                    "catalog": if *catalog_empty { "empty" } else { "available" },
                    "query_name": query_name,
                    "match": serde_json::Value::Null,
                    "truncated": catalog_truncated,
                    "candidates": [],
                    "candidates_truncated": false,
                    "selected": serde_json::Value::Null,
                }),
            )
        }
        person_search::PersonResult::Ambiguous {
            query_name,
            candidates,
            more,
            catalog_truncated,
        } => {
            let answer = format!(
                "\"{query_name}\" matches several people in the archive. Choose the person you \
                 mean; the archive never merges people who share a name."
            );
            (
                answer,
                true,
                serde_json::json!({
                    "outcome": "ambiguous",
                    "catalog": "available",
                    "query_name": query_name,
                    "match": serde_json::Value::Null,
                    "truncated": catalog_truncated,
                    "candidates": candidates.iter().map(candidate_json).collect::<Vec<_>>(),
                    "candidates_truncated": more,
                    "selected": serde_json::Value::Null,
                }),
            )
        }
        person_search::PersonResult::Found {
            query_name,
            candidate,
            detail,
            name_only,
            catalog_truncated,
        } => {
            let answer = if *name_only {
                format!(
                    "The archive records a person named {}. Only the name is stored, so no \
                     further detail can be given.",
                    candidate.display_name
                )
            } else {
                format!(
                    "These are the assertions the archive stores for {}. Each item keeps its \
                     recorded review status and source citation; an unverified assertion is not \
                     a confirmed fact.",
                    candidate.display_name
                )
            };
            let assertions: Vec<serde_json::Value> = detail
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
                        "authority": assertion_authority_json(&assertion.authority),
                    })
                })
                .collect();
            let selected = serde_json::json!({
                "person_key": candidate.person_key,
                "display_name": candidate.display_name,
                "generation": candidate.generation,
                "evidence_status": candidate.evidence_status,
                "match": candidate.match_kind.map(person_search::MatchKind::as_str),
                "matched_name": candidate.matched_name,
                "profile_url": format!("/family/{}", candidate.person_key),
                "name_only": name_only,
                "assertions": assertions,
            });
            (
                answer,
                false,
                serde_json::json!({
                    "outcome": if *name_only { "name_only" } else { "found" },
                    "catalog": "available",
                    "query_name": query_name,
                    "match": candidate.match_kind.map(person_search::MatchKind::as_str),
                    "truncated": catalog_truncated,
                    "candidates": [],
                    "candidates_truncated": false,
                    "selected": selected,
                }),
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
                "evidence_id": format!(
                    "{}#{}#{}",
                    citation.document_ref,
                    citation.page,
                    citation.locator.as_deref().unwrap_or("")
                ),
                "citation": citation.label(),
                "document_ref": citation.document_ref,
                "page": citation.page,
                "locator": citation.locator,
                "excerpt": citation.excerpt,
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
///
/// History counts are populated for an administrator and for an active linked
/// family member, because those are the callers allowed to read the passages.
/// Identity or link resolution fails closed (zero counts) without failing the
/// tree.
async fn genealogy_tree(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let identity = match authorize(&state, &headers, Operation::GenealogyRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let is_admin = identity.has_role(Role::Admin);
    let config = state.config.clone();
    let issuer = identity.issuer().to_string();
    let subject = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config)?;
        let include_history = match resolve_profile_account(&config, &issuer, &subject) {
            Ok(account) => onboarding::can_read_family_history(&mut client, &account, is_admin)
                .unwrap_or(false),
            Err(_) => false,
        };
        genealogy_tree::load_tree(&mut client, include_history).map_err(|err| err.to_string())
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
                        "birth": tree_date_json(&person.birth),
                        "death": tree_date_json(&person.death),
                        "history_passages": person.history_passages,
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
                        "date": assertion_date_json(&assertion.date),
                        "evidence": evidence_json(&assertion.citations),
                        "authority": assertion_authority_json(&assertion.authority),
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

// --- reviewed document extraction -----------------------------------------

#[derive(Deserialize)]
struct ExtractionDecisionRequest {
    decision: String,
    #[serde(default)]
    person_key: Option<String>,
    #[serde(default)]
    create_person: bool,
    #[serde(default)]
    value_text: Option<String>,
}

fn extraction_error(err: document_extraction::ExtractionError) -> Response {
    eprintln!("api: document extraction failed: {err}");
    match err {
        document_extraction::ExtractionError::NoRecords => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_records",
            "the document states no family record to extract",
        ),
        document_extraction::ExtractionError::NotFound => error(
            StatusCode::NOT_FOUND,
            "no_extraction",
            "no extraction exists for this document version",
        ),
        document_extraction::ExtractionError::Stale => error(
            StatusCode::CONFLICT,
            "stale_extraction",
            "the document changed or was withdrawn; extract it again",
        ),
        document_extraction::ExtractionError::Decided => error(
            StatusCode::CONFLICT,
            "already_decided",
            "that proposal already has a decision",
        ),
        document_extraction::ExtractionError::UnresolvedPerson(name) => error(
            StatusCode::CONFLICT,
            "unresolved_person",
            format!("resolve the person reference first: {name}"),
        ),
        document_extraction::ExtractionError::Invalid(message) => {
            error(StatusCode::BAD_REQUEST, "invalid_extraction", message)
        }
        document_extraction::ExtractionError::Genealogy(err) => genealogy_error(err),
    }
}

/// Only an approved, processed, family-visible text document can be extracted.
/// An admin-only document stays out of genealogy extraction entirely.
#[allow(clippy::result_large_err)]
fn ensure_extractable(version: &document_lifecycle::DocumentVersion) -> Result<(), Response> {
    if !version.is_approved() || !version.is_ready() || version.is_removed() {
        return Err(error(
            StatusCode::CONFLICT,
            "not_extractable",
            "only an approved, processed document can be extracted",
        ));
    }
    if version.access_scope.as_deref() != Some("family") {
        return Err(error(
            StatusCode::CONFLICT,
            "not_extractable",
            "only a family-visible document can be extracted",
        ));
    }
    let extension = version
        .original_filename
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(extension.as_str(), "md" | "markdown" | "txt") {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "not_extractable",
            "extraction reads text documents (.md, .markdown, .txt)",
        ));
    }
    Ok(())
}

fn extraction_json(detail: &document_extraction::ExtractionDetail) -> serde_json::Value {
    let proposals: Vec<serde_json::Value> = detail
        .proposals
        .iter()
        .map(|proposal| {
            serde_json::json!({
                "proposal_id": proposal.proposal_id,
                "ordinal": proposal.ordinal,
                "kind": proposal.kind,
                "subject_name": proposal.subject_name,
                "target_name": proposal.target_name,
                "resolution": proposal.resolution,
                "resolved_person_key": proposal.resolved_person_key,
                "candidates": proposal.candidates,
                "value_text": proposal.value_text,
                "support_text": proposal.support_text,
                "status": proposal.status,
                "decided_by": proposal.decided_by,
                "applied_result": proposal.applied_result,
            })
        })
        .collect();
    serde_json::json!({
        "ok": true,
        "extraction": {
            "extraction_id": detail.extraction.extraction_id,
            "document_version_id": detail.extraction.document_version_id,
            "document_source": detail.extraction.document_source,
            "content_sha256": detail.extraction.content_sha256,
            "status": detail.extraction.status,
            "created_by": detail.extraction.created_by,
            "created_at": detail.extraction.created_at,
            "applied_by": detail.extraction.applied_by,
            "applied_at": detail.extraction.applied_at,
        },
        "proposals": proposals,
    })
}

#[allow(clippy::result_large_err)]
async fn read_document_extraction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::DocumentReview) {
        return response;
    }
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config).map_err(genealogy_unavailable)?;
        document_extraction::detail(&mut client, version_id)
            .map_err(|err| extraction_error(document_extraction::ExtractionError::Genealogy(err)))
    })
    .await;
    match result {
        Ok(Ok(Some(detail))) => Json(extraction_json(&detail)).into_response(),
        Ok(Ok(None)) => error(
            StatusCode::NOT_FOUND,
            "no_extraction",
            "no extraction exists for this document version",
        ),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: extraction read task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "extraction_failed",
                "the extraction request failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn extract_document_records(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut rag = open_rag(&config)?;
        let storage = DocumentStorage::new(config.document_root.clone());
        let (version, bytes) =
            document_lifecycle::read_original(&mut rag, &storage, version_id, &corpus)
                .map_err(lifecycle_error)?;
        ensure_extractable(&version)?;
        let text = String::from_utf8(bytes).map_err(|_| {
            error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "not_extractable",
                "the document text is not valid UTF-8",
            )
        })?;
        let mut genealogy = open_genealogy(&config).map_err(genealogy_unavailable)?;
        document_extraction::extract(
            &mut genealogy,
            document_extraction::ExtractionInput {
                document_version_id: version.version_id,
                corpus: &corpus,
                document_source: &version.source,
                content_sha256: &version.content_sha256,
                text: &text,
                actor: &actor,
            },
        )
        .map_err(extraction_error)
    })
    .await;
    match result {
        Ok(Ok(detail)) => Json(extraction_json(&detail)).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: extraction task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "extraction_failed",
                "the extraction request failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn decide_document_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(proposal_id): Path<i64>,
    Json(request): Json<ExtractionDecisionRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config).map_err(genealogy_unavailable)?;
        document_extraction::decide(
            &mut client,
            document_extraction::DecisionInput {
                proposal_id,
                decision: &request.decision,
                person_key: request.person_key.as_deref(),
                create_person: request.create_person,
                value_text: request.value_text.as_deref(),
                actor: &actor,
            },
        )
        .map_err(extraction_error)
    })
    .await;
    match result {
        Ok(Ok(detail)) => Json(extraction_json(&detail)).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: extraction decision task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "extraction_failed",
                "the extraction decision failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn apply_document_extraction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut rag = open_rag(&config)?;
        let version = document_lifecycle::get_version(&mut rag, version_id, &corpus)
            .map_err(lifecycle_error)?;
        ensure_extractable(&version)?;
        let mut genealogy = open_genealogy(&config).map_err(genealogy_unavailable)?;
        document_extraction::apply(
            &mut genealogy,
            version.version_id,
            &version.content_sha256,
            &actor,
        )
        .map_err(extraction_error)
    })
    .await;
    match result {
        Ok(Ok(report)) => Json(serde_json::json!({
            "ok": true,
            "extraction_id": report.extraction_id,
            "already_applied": report.already_applied,
            "applied": report.applied,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: extraction apply task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "extraction_failed",
                "the extraction apply failed",
            )
        }
    }
}

fn genealogy_unavailable(message: String) -> Response {
    eprintln!("api: genealogy unavailable: {message}");
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "genealogy_unavailable",
        "the genealogy store is unavailable",
    )
}

// --- authoritative source decisions ---------------------------------------

/// One preview or apply request. The scope selects which assertions of the
/// document version the reviewer designates as authoritative.
#[derive(serde::Deserialize)]
struct AuthorityScopeRequest {
    /// `document`, `person`, or `assertions`.
    scope: String,
    #[serde(default)]
    person_key: Option<String>,
    #[serde(default)]
    assertion_ids: Vec<i64>,
    /// Current assertions the reviewer explicitly confirms removing.
    #[serde(default)]
    remove_assertion_ids: Vec<i64>,
}

#[derive(serde::Deserialize)]
struct AuthorityApplyRequest {
    scope: String,
    #[serde(default)]
    person_key: Option<String>,
    #[serde(default)]
    assertion_ids: Vec<i64>,
    #[serde(default)]
    remove_assertion_ids: Vec<i64>,
    reason: String,
    preview_hash: String,
    #[serde(default)]
    confirm: bool,
    #[serde(default)]
    acknowledge_conflicts: bool,
}

#[derive(serde::Deserialize)]
struct AuthorityReverseRequest {
    reason: String,
}

fn authority_error(err: authority::AuthorityError) -> Response {
    eprintln!("api: authority decision failed: {err}");
    match err {
        authority::AuthorityError::NotFound => error(
            StatusCode::NOT_FOUND,
            "authority_not_found",
            "no reviewed extraction exists for this document version",
        ),
        authority::AuthorityError::Stale => error(
            StatusCode::CONFLICT,
            "authority_stale",
            "the document or the current records changed since this preview; review it again",
        ),
        authority::AuthorityError::Withdrawn => error(
            StatusCode::CONFLICT,
            "authority_source_withdrawn",
            "the source document was removed, so its evidence can no longer support a decision",
        ),
        authority::AuthorityError::Conflicts(count) => error(
            StatusCode::CONFLICT,
            "authority_conflicts",
            format!(
                "{count} reviewed record(s) would lose authority; acknowledge them to continue"
            ),
        ),
        authority::AuthorityError::Invalid(message) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "authority_invalid",
            &message,
        ),
        authority::AuthorityError::Genealogy(err) => genealogy_error(err),
    }
}

fn authority_preview_json(preview: &authority::AuthorityPreview) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "preview": preview,
    })
}

#[allow(clippy::result_large_err)]
async fn preview_document_authority(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
    Json(request): Json<AuthorityScopeRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let _ = identity;
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut rag = open_rag(&config)?;
        let version = document_lifecycle::get_version(&mut rag, version_id, &corpus)
            .map_err(lifecycle_error)?;
        ensure_extractable(&version)?;
        let mut genealogy = open_genealogy(&config).map_err(genealogy_unavailable)?;
        let spec = authority::ScopeSpec {
            scope: &request.scope,
            person_key: request.person_key.as_deref(),
            assertion_ids: &request.assertion_ids,
            remove_assertion_ids: &request.remove_assertion_ids,
        };
        authority::preview(&mut genealogy, version_id, &version.content_sha256, &spec)
            .map_err(authority_error)
    })
    .await;
    match result {
        Ok(Ok(preview)) => Json(authority_preview_json(&preview)).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: authority preview task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authority_failed",
                "the authority preview failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn apply_document_authority(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
    Json(request): Json<AuthorityApplyRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut rag = open_rag(&config)?;
        let version = document_lifecycle::get_version(&mut rag, version_id, &corpus)
            .map_err(lifecycle_error)?;
        ensure_extractable(&version)?;
        let mut genealogy = open_genealogy(&config).map_err(genealogy_unavailable)?;
        let spec = authority::ScopeSpec {
            scope: &request.scope,
            person_key: request.person_key.as_deref(),
            assertion_ids: &request.assertion_ids,
            remove_assertion_ids: &request.remove_assertion_ids,
        };
        authority::apply(
            &mut genealogy,
            authority::ApplyInput {
                document_version_id: version_id,
                content_sha256: &version.content_sha256,
                spec,
                reason: &request.reason,
                actor: &actor,
                preview_hash: &request.preview_hash,
                confirm: request.confirm,
                acknowledge_conflicts: request.acknowledge_conflicts,
            },
        )
        .map_err(authority_error)
    })
    .await;
    match result {
        Ok(Ok(report)) => Json(serde_json::json!({
            "ok": true,
            "decision": report,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: authority apply task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authority_failed",
                "the authority decision failed",
            )
        }
    }
}

#[derive(serde::Deserialize)]
struct AuthorityListQuery {
    #[serde(default)]
    person_key: Option<String>,
}

#[allow(clippy::result_large_err)]
async fn list_authority_decisions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AuthorityListQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::DocumentReview) {
        return response;
    }
    let config = state.config.clone();
    let person_key = query
        .person_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string);
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config).map_err(genealogy_unavailable)?;
        authority::list_decisions(&mut client, person_key.as_deref()).map_err(authority_error)
    })
    .await;
    match result {
        Ok(Ok(decisions)) => Json(serde_json::json!({
            "ok": true,
            "decisions": decisions,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: authority history task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authority_failed",
                "the authority history could not be read",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn reverse_authority_decision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(decision_id): Path<i64>,
    Json(request): Json<AuthorityReverseRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<_, Response> {
        let mut client = open_genealogy(&config).map_err(genealogy_unavailable)?;
        authority::reverse(
            &mut client,
            authority::ReverseInput {
                decision_id,
                reason: &request.reason,
                actor: &actor,
            },
        )
        .map_err(authority_error)
    })
    .await;
    match result {
        Ok(Ok(report)) => Json(serde_json::json!({
            "ok": true,
            "decision": report,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: authority reversal task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authority_failed",
                "the authority reversal failed",
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
                        "candidate_label": row.candidate_label,
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

// --- admin family members ---------------------------------------------------

#[derive(Deserialize)]
struct FamilyMembersQuery {
    q: Option<String>,
    status: Option<String>,
    role: Option<String>,
    after: Option<i64>,
    limit: Option<i64>,
}

/// One directory row: the account plus its linked person, if any.
#[derive(Serialize)]
struct FamilyMemberBody {
    #[serde(flatten)]
    member: admin_members::Member,
    linked_person: Option<admin_members::LinkedPerson>,
}

/// Validates the directory query. Unknown status or role names are rejected,
/// never ignored.
#[allow(clippy::result_large_err)]
fn member_filter(query: FamilyMembersQuery) -> Result<MemberFilter, Response> {
    let status = match query.status.as_deref().filter(|value| !value.is_empty()) {
        None => None,
        Some(value) => Some(MemberStatus::parse(value).ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_status",
                "status must be active or deactivated",
            )
        })?),
    };
    let role = match query.role.as_deref().filter(|value| !value.is_empty()) {
        None => None,
        Some(value) => Some(Role::parse(value).ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_role",
                "role must be viewer, editor, or admin",
            )
        })?),
    };
    Ok(MemberFilter {
        query: query.q,
        status,
        role,
        after_id: query.after,
        limit: query.limit,
    })
}

/// The admin account directory. Authorization runs before any database read.
/// Linked-person status comes from the genealogy database when it is reachable;
/// otherwise every row reports no link and `linkage` says it is unavailable,
/// so a missing store is never shown as "not linked".
async fn family_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FamilyMembersQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::MemberDirectory) {
        return response;
    }
    let filter = match member_filter(query) {
        Ok(filter) => filter,
        Err(response) => return response,
    };
    let store = state.store.clone();
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = store.pooled_connection().map_err(|err| err.to_string())?;
        let (members, next) = admin_members::list_members(&mut client, &filter)?;
        let ids: Vec<i64> = members.iter().map(|member| member.user_id).collect();
        let links = open_genealogy(&config)
            .and_then(|mut genealogy| admin_members::linked_people(&mut genealogy, &ids));
        Ok::<_, String>((members, next, links))
    })
    .await;
    match result {
        Ok(Ok((members, next, links))) => {
            let (mut links, linkage) = match links {
                Ok(links) => (links, "available"),
                Err(message) => {
                    eprintln!("api: member linkage unavailable: {message}");
                    (Default::default(), "unavailable")
                }
            };
            let members: Vec<FamilyMemberBody> = members
                .into_iter()
                .map(|member| FamilyMemberBody {
                    linked_person: links.remove(&member.user_id),
                    member,
                })
                .collect();
            Json(serde_json::json!({
                "ok": true,
                "members": members,
                "next_after": next,
                "linkage": linkage,
            }))
            .into_response()
        }
        Ok(Err(message)) => {
            eprintln!("api: member directory unavailable: {message}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "identity_store_unavailable",
                "the identity store is unavailable",
            )
        }
        Err(err) => {
            eprintln!("api: member directory task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "directory_failed",
                "the member directory failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct PasswordResetBody {
    issuer: String,
    subject: String,
}

/// Maps a reset outcome to the wire. Only an accepted provider request is a
/// success; every other state says no email was sent.
fn reset_response(outcome: Result<ResetState, ResetError>) -> Response {
    match outcome {
        Ok(ResetState::Requested) => {
            Json(serde_json::json!({ "ok": true, "state": "requested" })).into_response()
        }
        Ok(ResetState::NotConfigured) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "reset_not_configured",
            "password reset is not configured for this identity provider; no email was sent",
        ),
        Ok(ResetState::Failed) => error(
            StatusCode::BAD_GATEWAY,
            "reset_failed",
            "the identity provider did not accept the reset request; no email was sent",
        ),
        Ok(ResetState::Throttled) => error(
            StatusCode::TOO_MANY_REQUESTS,
            "reset_throttled",
            "a reset was requested for this account recently",
        ),
        Err(ResetError::UnknownUser) => {
            error(StatusCode::NOT_FOUND, "unknown_user", "no such account")
        }
        Err(ResetError::Deactivated) => error(
            StatusCode::CONFLICT,
            "account_deactivated",
            "the account is deactivated",
        ),
        Err(ResetError::Database(message)) => {
            eprintln!("api: password reset unavailable: {message}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "identity_store_unavailable",
                "the identity store is unavailable",
            )
        }
    }
}

/// Asks the configured identity provider to email a reset link to one active
/// account. Success is reported only when the provider accepted the request;
/// the admin never sees a password, link, or token.
async fn family_member_password_reset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PasswordResetBody>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::PasswordReset) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let issuer = body.issuer.trim().to_string();
    let subject = body.subject.trim().to_string();
    if issuer.is_empty() || subject.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_identity",
            "issuer and subject are required",
        );
    }
    let store = state.store.clone();
    let config = state.config.clone();
    let actor_issuer = identity.issuer().to_string();
    let actor_subject = identity.subject().to_string();
    let reset_client = state.reset_client.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = store
            .pooled_connection()
            .map_err(|err| ResetError::Database(err.to_string()))?;
        let http = {
            let mut guard = reset_client
                .lock()
                .map_err(|_| ResetError::Database("reset client lock poisoned".into()))?;
            if guard.is_none() {
                *guard = Some(
                    reqwest::blocking::Client::builder()
                        .timeout(Duration::from_secs(10))
                        .redirect(reqwest::redirect::Policy::none())
                        .build()
                        .map_err(|err| ResetError::Database(err.to_string()))?,
                );
            }
            guard.clone().expect("reset client was just built")
        };
        admin_members::request_password_reset(
            &mut client,
            config.password_reset.as_ref(),
            &http,
            &ResetRequest {
                actor_issuer: &actor_issuer,
                actor_subject: &actor_subject,
                issuer: &issuer,
                subject: &subject,
            },
            config.password_reset_cooldown,
        )
    })
    .await;
    match result {
        Ok(outcome) => reset_response(outcome),
        Err(err) => {
            eprintln!("api: password reset task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "reset_failed",
                "the reset request failed",
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

// --- bulk onboarding -------------------------------------------------------

/// A preview/dry-run request. The selector is explicit: selecting all means the
/// eligible set as evaluated now, never a future stream of imported people.
#[derive(Deserialize)]
struct BulkPreviewRequest {
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    person_keys: Vec<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Serialize)]
struct BulkPreviewRowBody {
    person_key: String,
    display_name: String,
    contact: BulkContactBody,
    classification: &'static str,
    reasons: Vec<&'static str>,
    proposed_username: Option<String>,
    preparable: bool,
    blocked_missing_recipient: bool,
    sendable: bool,
}

/// The verified contact for one preview row. The email variant carries only the
/// server-masked address; the full address never leaves the server.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum BulkContactBody {
    Email {
        #[serde(skip_serializing_if = "Option::is_none")]
        masked: Option<String>,
    },
    None,
}

impl BulkContactBody {
    fn from_masked(masked: &Option<String>) -> Self {
        match masked {
            Some(masked) => BulkContactBody::Email {
                masked: Some(masked.clone()),
            },
            None => BulkContactBody::None,
        }
    }
}

#[derive(Serialize)]
struct BulkSummaryBody {
    eligible_for_invite: usize,
    needs_review: usize,
    already_linked: usize,
    already_prepared: usize,
    invited: usize,
    not_eligible: usize,
    blocked_missing_recipient: usize,
}

#[derive(Serialize)]
struct BulkPreviewBody {
    ok: bool,
    rows: Vec<BulkPreviewRowBody>,
    summary: BulkSummaryBody,
    review_needed: Option<String>,
}

/// An admin-only dry run. It is strictly read-only: no reservation, invitation,
/// link, role, or audit row is written.
#[allow(clippy::result_large_err)]
async fn bulk_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BulkPreviewRequest>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::OnboardingDryRun) {
        return response;
    }
    let selector = match request.selector.as_deref().unwrap_or("all") {
        "explicit" => onboarding::Selector::ExplicitSelection(request.person_keys.clone()),
        "all_eligible" => onboarding::Selector::AllCurrentlyEligible,
        "all" => onboarding::Selector::All,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_selector",
                "selector must be all_eligible or explicit",
            )
        }
    };
    let page = onboarding::Page {
        limit: request.limit.unwrap_or(50).min(200),
        offset: request.offset.unwrap_or(0),
    };
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config).map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        })?;
        let preview = onboarding::preview_candidates(&mut client).map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "preview_failed",
                "the candidate preview failed",
            )
        })?;
        Ok::<_, Response>(onboarding::preview_dry_run(
            &preview,
            &selector,
            &onboarding::DryRunFilters::default(),
            page,
        ))
    })
    .await;

    match result {
        Ok(Ok(report)) => {
            let rows = report
                .rows
                .iter()
                .map(|row| BulkPreviewRowBody {
                    person_key: row.person_key.clone(),
                    display_name: row.display_name.clone(),
                    contact: BulkContactBody::from_masked(&row.contact_email_masked),
                    classification: row.classification.as_str(),
                    reasons: row.reasons.iter().map(|reason| reason.as_str()).collect(),
                    proposed_username: row.proposed_username.clone(),
                    preparable: row.preparable,
                    blocked_missing_recipient: row.blocked_missing_recipient,
                    sendable: row.sendable,
                })
                .collect();
            Json(BulkPreviewBody {
                ok: true,
                rows,
                summary: BulkSummaryBody {
                    eligible_for_invite: report.eligible_for_invite,
                    needs_review: report.needs_review,
                    already_linked: report.already_linked,
                    already_prepared: report.already_prepared,
                    invited: report.invited,
                    not_eligible: report.not_eligible,
                    blocked_missing_recipient: report.blocked_missing_recipient,
                },
                review_needed: report.review_needed,
            })
            .into_response()
        }
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: bulk preview task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "preview_failed",
                "the candidate preview failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct BulkPrepareRequest {
    person_keys: Vec<String>,
    confirmed_count: usize,
}

#[derive(Serialize)]
struct BulkItemBody {
    person_key: String,
    status: &'static str,
    reason: Option<String>,
    reservation_id: Option<i64>,
}

#[derive(Serialize)]
struct BulkPrepareBody {
    ok: bool,
    batch_id: i64,
    batch_key: String,
    created: bool,
    prepared: usize,
    skipped: usize,
    needs_review: usize,
    failed: usize,
    items: Vec<BulkItemBody>,
}

/// Resolves opaque person keys to ids and rejects an unknown key before any
/// write. Duplicate keys collapse to one id.
#[allow(clippy::result_large_err)]
fn resolve_person_keys(
    client: &mut postgres::Client,
    keys: &[String],
) -> Result<Vec<(String, i64)>, Response> {
    let mut resolved: Vec<(String, i64)> = Vec::new();
    let mut seen: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
    for key in keys {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let row = client
            .query_opt(
                "SELECT person_id FROM person WHERE person_key = $1",
                &[&key],
            )
            .map_err(|_| {
                error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "genealogy_unavailable",
                    "the genealogy store is unavailable",
                )
            })?;
        let Some(row) = row else {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "unknown_person",
                "one or more selected people no longer exist",
            ));
        };
        let person_id: i64 = row.get(0);
        if seen.insert(person_id) {
            resolved.push((key.to_string(), person_id));
        }
    }
    Ok(resolved)
}

/// Prepares an explicitly confirmed batch. It revalidates eligibility and
/// reserves usernames, but never activates an account or sends a message.
#[allow(clippy::result_large_err)]
async fn bulk_prepare(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BulkPrepareRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::OnboardingBulkPrepare) {
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
        let resolved = resolve_person_keys(&mut client, &request.person_keys)?;
        let ids: Vec<i64> = resolved.iter().map(|(_, id)| *id).collect();
        let report = onboarding::confirm_batch(&mut client, &actor, request.confirmed_count, &ids)
            .map_err(batch_error)?;
        let mut key_by_id: std::collections::BTreeMap<i64, String> =
            std::collections::BTreeMap::new();
        for (key, id) in resolved {
            key_by_id.insert(id, key);
        }
        Ok::<_, Response>((report, key_by_id))
    })
    .await;

    match result {
        Ok(Ok((report, keys))) => {
            let prepared = report.count(onboarding::ItemStatus::Prepared);
            let needs_review = report.count(onboarding::ItemStatus::NeedsReview);
            let failed = report.count(onboarding::ItemStatus::Failed);
            let skipped = report
                .items
                .iter()
                .filter(|item| {
                    !matches!(
                        item.status,
                        onboarding::ItemStatus::Prepared
                            | onboarding::ItemStatus::NeedsReview
                            | onboarding::ItemStatus::Failed
                            | onboarding::ItemStatus::Pending
                    )
                })
                .count();
            let items = report
                .items
                .iter()
                .map(|item| BulkItemBody {
                    person_key: keys.get(&item.person_id).cloned().unwrap_or_default(),
                    status: item.status.as_str(),
                    reason: item.reason_code.clone(),
                    reservation_id: item.reservation_id,
                })
                .collect();
            Json(BulkPrepareBody {
                ok: true,
                batch_id: report.batch_id,
                batch_key: report.batch_key,
                created: report.created,
                prepared,
                skipped,
                needs_review,
                failed,
                items,
            })
            .into_response()
        }
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: bulk prepare task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "prepare_failed",
                "the batch preparation failed",
            )
        }
    }
}

fn batch_error(err: onboarding::BatchError) -> Response {
    match err {
        onboarding::BatchError::CountMismatch { .. } => error(
            StatusCode::BAD_REQUEST,
            "count_mismatch",
            "the confirmation count does not match the selected people",
        ),
        onboarding::BatchError::EmptySelection => error(
            StatusCode::BAD_REQUEST,
            "empty_selection",
            "select at least one person",
        ),
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "prepare_failed",
            "the batch preparation failed",
        ),
    }
}

#[derive(Deserialize)]
struct BulkRecipientRequest {
    person_key: String,
    email: String,
    /// Must be explicitly true. The admin is asserting that they verified the
    /// address independently; the server never guesses one.
    #[serde(default)]
    confirmed: bool,
}

/// Records an explicitly verified recipient email for one person.
#[allow(clippy::result_large_err)]
async fn bulk_recipient(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BulkRecipientRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::OnboardingBulkPrepare) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if !request.confirmed {
        return error(
            StatusCode::BAD_REQUEST,
            "recipient_confirmation_required",
            "the recipient address must be explicitly confirmed",
        );
    }
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
        let person_id = person_id_for_key(&mut client, request.person_key.trim())
            .map_err(|_| {
                error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "genealogy_unavailable",
                    "the genealogy store is unavailable",
                )
            })?
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "unknown_person", "no such person"))?;
        onboarding::verify_recipient(&mut client, person_id, &request.email, &actor)
            .map_err(delivery_error)
    })
    .await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true, "verified": true })).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: bulk recipient task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "recipient_failed",
                "the recipient could not be recorded",
            )
        }
    }
}

fn delivery_error(err: onboarding::DeliveryError) -> Response {
    match err {
        onboarding::DeliveryError::InvalidRecipient => error(
            StatusCode::BAD_REQUEST,
            "invalid_recipient",
            "the recipient address is not valid",
        ),
        onboarding::DeliveryError::NotConfigured => error(
            StatusCode::CONFLICT,
            "not_configured",
            "no invitation delivery adapter is configured",
        ),
        onboarding::DeliveryError::MissingVerifiedContact => error(
            StatusCode::CONFLICT,
            "missing_verified_contact",
            "a verified recipient address is required",
        ),
        onboarding::DeliveryError::NotPrepared => error(
            StatusCode::CONFLICT,
            "not_prepared",
            "the person is not prepared for sending",
        ),
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "delivery_failed",
            "invitation delivery failed",
        ),
    }
}

#[derive(Deserialize)]
struct BulkSendRequest {
    batch_id: i64,
    #[serde(default)]
    person_keys: Vec<String>,
    /// Must be explicitly true. Preparation never authorizes sending.
    #[serde(default)]
    approve: bool,
    #[serde(default)]
    ttl_secs: Option<u64>,
}

#[derive(Serialize)]
struct BulkSendResultBody {
    person_key: String,
    status: &'static str,
    provider: Option<String>,
    reason: Option<&'static str>,
}

/// Sends invitations only after a separate explicit approval and a configured
/// adapter. A failure never reports delivery. Tokens never leave the adapter.
#[allow(clippy::result_large_err)]
async fn bulk_send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BulkSendRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::OnboardingBulkSend) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if !request.approve {
        return error(
            StatusCode::BAD_REQUEST,
            "approval_required",
            "sending requires an explicit approval",
        );
    }
    let config = state.config.clone();
    let adapter = config.mail_adapter();
    if !adapter.is_configured() {
        return error(
            StatusCode::CONFLICT,
            "not_configured",
            "no invitation delivery adapter is configured",
        );
    }
    let actor = identity.subject().to_string();
    let activation = config.onboarding_activation_url.clone();
    let throttle = config.onboarding_send_throttle();
    let ttl = Duration::from_secs(
        request
            .ttl_secs
            .unwrap_or(DEFAULT_INVITATION_TTL_SECS)
            .clamp(1, MAX_INVITATION_TTL_SECS),
    );
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_genealogy(&config).map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "genealogy_unavailable",
                "the genealogy store is unavailable",
            )
        })?;
        let selected = if request.person_keys.is_empty() {
            None
        } else {
            let resolved = resolve_person_keys(&mut client, &request.person_keys)?;
            Some(resolved)
        };
        let (ids, key_by_id): (Option<Vec<i64>>, std::collections::BTreeMap<i64, String>) =
            match selected {
                Some(rows) => {
                    let ids: Vec<i64> = rows.iter().map(|(_, id)| *id).collect();
                    let keys = rows.into_iter().map(|(key, id)| (id, key)).collect();
                    (Some(ids), keys)
                }
                None => {
                    let rows = client
                        .query(
                            "SELECT item.person_id, person.person_key
                           FROM onboarding_batch_item item
                           JOIN person ON person.person_id = item.person_id
                          WHERE item.batch_id = $1",
                            &[&request.batch_id],
                        )
                        .map_err(|_| {
                            error(
                                StatusCode::SERVICE_UNAVAILABLE,
                                "genealogy_unavailable",
                                "the genealogy store is unavailable",
                            )
                        })?;
                    let keys = rows
                        .iter()
                        .map(|row| (row.get::<_, i64>(0), row.get::<_, String>(1)))
                        .collect();
                    (None, keys)
                }
            };
        let outcomes = onboarding::send_batch(
            &mut client,
            request.batch_id,
            ids.as_deref(),
            adapter.as_ref(),
            &activation,
            ttl,
            &actor,
            throttle,
        )
        .map_err(delivery_error)?;
        Ok::<_, Response>((outcomes, key_by_id))
    })
    .await;

    match result {
        Ok(Ok((outcomes, keys))) => {
            let results = outcomes
                .into_iter()
                .map(|outcome| BulkSendResultBody {
                    person_key: keys.get(&outcome.person_id).cloned().unwrap_or_default(),
                    status: outcome.status.as_str(),
                    provider: outcome.provider,
                    reason: outcome.reason,
                })
                .collect::<Vec<_>>();
            Json(serde_json::json!({
                "ok": true,
                "configured": true,
                "results": results,
            }))
            .into_response()
        }
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: bulk send task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "send_failed",
                "invitation sending failed",
            )
        }
    }
}

/// Reports batch and per-item preparation and delivery state. It never returns
/// an invitation token or a recipient address.
#[allow(clippy::result_large_err)]
async fn bulk_batch_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(batch_id): Path<i64>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::OnboardingDryRun) {
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
        let _ = onboarding::sync_invitation_status(&mut client, batch_id);
        let batch = client
            .query_opt(
                "SELECT batch_id, status, confirmed_count, created_by, created_at::text, completed_at::text
                   FROM onboarding_batch WHERE batch_id = $1",
                &[&batch_id],
            )
            .map_err(|_| {
                error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "status_failed",
                    "the batch status failed",
                )
            })?
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "unknown_batch", "no such batch"))?;
        let item_rows = onboarding::batch_status_items(&mut client, batch_id).map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "status_failed",
                "the batch status failed",
            )
        })?;
        Ok::<_, Response>(serde_json::json!({
            "ok": true,
            "batch_id": batch.get::<_, i64>(0),
            "status": batch.get::<_, String>(1),
            "confirmed_count": batch.get::<_, i32>(2),
            "created_by": batch.get::<_, String>(3),
            "created_at": batch.get::<_, String>(4),
            "completed_at": batch.get::<_, Option<String>>(5),
            "items": item_rows,
        }))
    })
    .await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: bulk status task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "status_failed",
                "the batch status failed",
            )
        }
    }
}

// --- family document lifecycle ---------------------------------------------

/// Opens the RAG database for a document lifecycle operation.
#[allow(clippy::result_large_err)]
fn open_rag(config: &Config) -> Result<postgres::Client, Response> {
    postgres::Client::connect(&config.database_url, postgres::NoTls).map_err(|_| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "document_store_unavailable",
            "the document store is unavailable",
        )
    })
}

fn lifecycle_error(err: document_lifecycle::LifecycleError) -> Response {
    eprintln!("api: document lifecycle failed: {err}");
    match err {
        document_lifecycle::LifecycleError::NotIndexable(detail) => error(
            StatusCode::BAD_REQUEST,
            "not_indexable",
            format!("the document cannot be indexed: {detail}"),
        ),
        document_lifecycle::LifecycleError::Invalid(_, _) => error(
            StatusCode::BAD_REQUEST,
            "invalid_document",
            "the document request is invalid",
        ),
        document_lifecycle::LifecycleError::NotFound => error(
            StatusCode::NOT_FOUND,
            "unknown_document",
            "no such document submission",
        ),
        document_lifecycle::LifecycleError::ScopeChange => error(
            StatusCode::CONFLICT,
            "scope_change_requires_explicit_scope",
            "approval cannot change the document scope; change the scope explicitly first",
        ),
        document_lifecycle::LifecycleError::NotApproved
        | document_lifecycle::LifecycleError::InvalidTransition => error(
            StatusCode::CONFLICT,
            "invalid_document_state",
            "the document is not in a state that permits this action",
        ),
        document_lifecycle::LifecycleError::MissingFile => error(
            StatusCode::NOT_FOUND,
            "document_original_missing",
            "the document original is unavailable",
        ),
        document_lifecycle::LifecycleError::Storage
        | document_lifecycle::LifecycleError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "document_store_unavailable",
            "the document store is unavailable",
        ),
    }
}

fn can_access_document(identity: &VerifiedIdentity, version: &DocumentVersion) -> bool {
    identity.has_role(Role::Admin)
        || (version.created_by == identity.subject()
            && version.owner_issuer.as_deref() == Some(identity.issuer()))
}

struct DocumentSubmissionForm {
    original_filename: String,
    bytes: Vec<u8>,
    title: String,
    description: Option<String>,
    category: Category,
    scope: AccessScope,
    document_key: Option<String>,
}

#[allow(clippy::result_large_err)]
async fn parse_document_submission(
    mut multipart: Multipart,
    max_input_bytes: i64,
) -> Result<DocumentSubmissionForm, Response> {
    let mut filename: Option<String> = None;
    let mut bytes: Option<Vec<u8>> = None;
    let mut title: Option<String> = None;
    let mut description: Option<String> = None;
    let mut category: Option<String> = None;
    let mut scope: Option<String> = None;
    let mut document_key: Option<String> = None;

    while let Some(field) = multipart.next_field().await.map_err(|err| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_multipart",
            err.to_string(),
        )
    })? {
        match field.name().unwrap_or("") {
            "file" => {
                if bytes.is_some() {
                    return Err(error(
                        StatusCode::BAD_REQUEST,
                        "multiple_files",
                        "exactly one file is accepted per request",
                    ));
                }
                let name = field
                    .file_name()
                    .and_then(sanitize_file_name)
                    .ok_or_else(|| {
                        error(
                            StatusCode::BAD_REQUEST,
                            "missing_filename",
                            "the uploaded file must have a name",
                        )
                    })?;
                let ext = extension(&name).ok_or_else(|| {
                    error(
                        StatusCode::BAD_REQUEST,
                        "unsupported_type",
                        "supported types: .md, .markdown, .txt, .pdf",
                    )
                })?;
                if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
                    return Err(error(
                        StatusCode::BAD_REQUEST,
                        "unsupported_type",
                        format!("unsupported file type .{ext}"),
                    ));
                }
                let payload = field.bytes().await.map_err(|err| {
                    error(
                        StatusCode::BAD_REQUEST,
                        "invalid_multipart",
                        err.to_string(),
                    )
                })?;
                if payload.is_empty() {
                    return Err(error(
                        StatusCode::BAD_REQUEST,
                        "empty_file",
                        "the uploaded file is empty",
                    ));
                }
                if payload.len() > MAX_UPLOAD_BYTES {
                    return Err(error(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "file_too_large",
                        format!("the upload exceeds the {} byte limit", MAX_UPLOAD_BYTES),
                    ));
                }
                if max_input_bytes > 0
                    && payload.len() > usize::try_from(max_input_bytes).unwrap_or(usize::MAX)
                {
                    return Err(error(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "file_too_large",
                        format!("the upload exceeds the {max_input_bytes} byte input limit"),
                    ));
                }
                filename = Some(name);
                bytes = Some(payload.to_vec());
            }
            "title" => {
                title = Some(field.text().await.map_err(|err| {
                    error(
                        StatusCode::BAD_REQUEST,
                        "invalid_multipart",
                        err.to_string(),
                    )
                })?);
            }
            "description" => {
                description = Some(field.text().await.map_err(|err| {
                    error(
                        StatusCode::BAD_REQUEST,
                        "invalid_multipart",
                        err.to_string(),
                    )
                })?);
            }
            "category" => {
                category = Some(field.text().await.map_err(|err| {
                    error(
                        StatusCode::BAD_REQUEST,
                        "invalid_multipart",
                        err.to_string(),
                    )
                })?);
            }
            "scope" => {
                scope = Some(field.text().await.map_err(|err| {
                    error(
                        StatusCode::BAD_REQUEST,
                        "invalid_multipart",
                        err.to_string(),
                    )
                })?);
            }
            "document_key" => {
                document_key = Some(field.text().await.map_err(|err| {
                    error(
                        StatusCode::BAD_REQUEST,
                        "invalid_multipart",
                        err.to_string(),
                    )
                })?);
            }
            other => {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "unexpected_field",
                    format!("unexpected form field {other:?}"),
                ));
            }
        }
    }

    let original_filename = filename.ok_or_else(|| {
        error(
            StatusCode::BAD_REQUEST,
            "missing_file",
            "no file field was supplied",
        )
    })?;
    let bytes = bytes.unwrap_or_default();
    let default_title = original_filename
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| original_filename.clone());
    let title = title
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or(default_title);
    if title.chars().count() > 200 {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_title",
            "the title must be 200 characters or fewer",
        ));
    }
    let description = description
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if description
        .as_deref()
        .map(|value| value.chars().count() > 2000)
        .unwrap_or(false)
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_description",
            "the description must be 2000 characters or fewer",
        ));
    }
    let category = category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.parse::<Category>())
        .transpose()
        .map_err(|_| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_category",
                "the document category is not recognized",
            )
        })?
        .unwrap_or(Category::Other);
    let scope = scope
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.parse::<AccessScope>())
        .transpose()
        .map_err(|_| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_scope",
                "the document scope is not recognized",
            )
        })?
        .unwrap_or(AccessScope::Family);
    let document_key = document_key
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    Ok(DocumentSubmissionForm {
        original_filename,
        bytes,
        title,
        description,
        category,
        scope,
        document_key,
    })
}

#[allow(clippy::result_large_err)]
async fn submit_document(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentSubmit) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let form = match parse_document_submission(multipart, state.config.max_input_bytes).await {
        Ok(form) => form,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let actor_issuer = identity.issuer().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        let storage = DocumentStorage::new(config.document_root.clone());
        document_lifecycle::create_submission(
            &mut client,
            &storage,
            NewSubmission {
                actor: &actor,
                corpus: &corpus,
                title: &form.title,
                description: form.description.as_deref(),
                category: form.category,
                access_scope: form.scope,
                owner_subject: Some(&actor),
                owner_issuer: Some(&actor_issuer),
                original_filename: &form.original_filename,
                bytes: &form.bytes,
                document_key: form.document_key.as_deref(),
            },
        )
        .map_err(lifecycle_error)
    })
    .await;
    match result {
        Ok(Ok(version)) => Json(serde_json::json!({
            "ok": true,
            "version_id": version.version_id,
            "review_status": version.review_status,
            "processing_status": version.processing_status,
            "chunks": 0,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document submission task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "submission_failed",
                "the document submission failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn list_document_submissions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let is_admin = identity.has_role(Role::Admin);
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        document_lifecycle::list_submissions(&mut client, &actor, is_admin, &corpus)
            .map_err(lifecycle_error)
    })
    .await;
    match result {
        Ok(Ok(submissions)) => Json(serde_json::json!({
            "ok": true,
            "submissions": submissions,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document list task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "document_list_failed",
                "the document submissions could not be listed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn get_document_submission(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        document_lifecycle::get_version(&mut client, version_id, &corpus).map_err(lifecycle_error)
    })
    .await;
    match result {
        Ok(Ok(version)) if can_access_document(&identity, &version) => Json(serde_json::json!({
            "ok": true,
            "submission": version,
        }))
        .into_response(),
        Ok(Ok(_)) => error(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the authenticated account may not access this document",
        ),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document read task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "document_read_failed",
                "the document submission could not be read",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn get_document_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentRead) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        let storage = DocumentStorage::new(config.document_root.clone());
        document_lifecycle::read_original(&mut client, &storage, version_id, &corpus)
            .map_err(lifecycle_error)
    })
    .await;
    match result {
        Ok(Ok((version, bytes))) if can_access_document(&identity, &version) => {
            let filename = sanitize_file_name(&version.original_filename)
                .unwrap_or_else(|| "document".to_string());
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, version.mime_type)
                .header(
                    CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename}\""),
                )
                .header(CACHE_CONTROL, "no-store")
                .body(Body::from(bytes))
                .unwrap_or_else(|_| {
                    error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "document_read_failed",
                        "the document original could not be read",
                    )
                })
        }
        Ok(Ok(_)) => error(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the authenticated account may not access this document",
        ),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document content task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "document_read_failed",
                "the document original could not be read",
            )
        }
    }
}

#[derive(Deserialize)]
struct DocumentReviewRequest {
    approve: bool,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[allow(clippy::result_large_err)]
async fn review_document_submission(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
    Json(request): Json<DocumentReviewRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let note = request.note.unwrap_or_default();
    if !request.approve && note.trim().is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "review_reason_required",
            "a rejection requires a reason",
        );
    }
    let scope = match request.scope.as_deref() {
        Some(value) => match value.trim().parse::<AccessScope>() {
            Ok(scope) => Some(scope),
            Err(_) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_scope",
                    "the document scope is not recognized",
                )
            }
        },
        None => None,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        document_lifecycle::review(
            &mut client,
            version_id,
            &corpus,
            &actor,
            request.approve,
            Some(note.as_str()),
            scope,
        )
        .map_err(lifecycle_error)
    })
    .await;
    match result {
        Ok(Ok(version)) => Json(serde_json::json!({
            "ok": true,
            "submission": version,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document review task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "review_failed",
                "the document review failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct DocumentScopeRequest {
    scope: String,
}

#[allow(clippy::result_large_err)]
async fn change_document_submission_scope(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
    Json(request): Json<DocumentScopeRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentReview) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let scope = match request.scope.trim().parse::<AccessScope>() {
        Ok(scope) => scope,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_scope",
                "the document scope is not recognized",
            )
        }
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        document_lifecycle::change_scope(&mut client, version_id, &corpus, &actor, scope)
            .map_err(lifecycle_error)
    })
    .await;
    match result {
        Ok(Ok(version)) => Json(serde_json::json!({
            "ok": true,
            "submission": version,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document scope task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "scope_failed",
                "the document scope change failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn remove_document_submission(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentSubmit) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let actor_issuer = identity.issuer().to_string();
    let is_admin = identity.has_role(Role::Admin);
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        let version = document_lifecycle::get_version(&mut client, version_id, &corpus)
            .map_err(lifecycle_error)?;
        if !is_admin
            && (version.created_by != actor
                || version.owner_issuer.as_deref() != Some(actor_issuer.as_str()))
        {
            return Err(error(
                StatusCode::FORBIDDEN,
                "forbidden",
                "the authenticated account may not remove this document",
            ));
        }
        let source = version.source.clone();
        document_lifecycle::remove(&mut client, version_id, &corpus, &actor)
            .map_err(lifecycle_error)
            .and_then(|removed| {
                // Take this version's evidence out of current answers. The rows
                // stay for the audit trail; reads filter them out.
                let mut genealogy = open_genealogy(&config).map_err(|message| {
                    eprintln!("api: genealogy unavailable for evidence withdrawal: {message}");
                    error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "evidence_withdrawal_failed",
                        "the document was removed, but its evidence could not be withdrawn; retry the removal",
                    )
                })?;
                let withdrawn = document_extraction::withdraw_document_evidence(
                    &mut genealogy,
                    version_id,
                    &source,
                )
                        .map_err(|err| {
                            eprintln!(
                                "api: evidence withdrawal failed for version {version_id}: {err}"
                            );
                            error(
                                StatusCode::SERVICE_UNAVAILABLE,
                                "evidence_withdrawal_failed",
                                "the document was removed, but its evidence could not be withdrawn; retry the removal",
                            )
                        })?;
                Ok((removed, withdrawn))
            })
    })
    .await;
    match result {
        Ok(Ok((version, withdrawn))) => Json(serde_json::json!({
            "ok": true,
            "submission": version,
            "evidence_withdrawn": withdrawn,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document removal task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "remove_failed",
                "the document removal failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn process_document_submission(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentProcess) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        let version = document_lifecycle::get_version(&mut client, version_id, &corpus)
            .map_err(lifecycle_error)?;
        if version.origin == "legacy_import" {
            let promoted =
                document_lifecycle::promote_legacy(&mut client, version_id, &corpus, &actor)
                    .map_err(lifecycle_error)?;
            return Ok(serde_json::json!({
                "ok": true,
                "promoted": true,
                "submission": promoted,
            }));
        }
        let job_id = document_lifecycle::queue_processing(&mut client, version_id, &corpus, &actor)
            .map_err(lifecycle_error)?;
        Ok(serde_json::json!({
            "ok": true,
            "job_id": job_id,
            "processing_status": "queued",
        }))
    })
    .await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document process task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "process_failed",
                "the document processing request failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn legacy_document_inventory(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::DocumentReview) {
        return response;
    }
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        document_lifecycle::legacy_inventory(&mut client, &corpus).map_err(lifecycle_error)
    })
    .await;
    match result {
        Ok(Ok(rows)) => Json(serde_json::json!({ "ok": true, "inventory": rows })).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: legacy inventory task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "inventory_failed",
                "the legacy document inventory failed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn promote_legacy_document(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentProcess) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        document_lifecycle::promote_legacy(&mut client, version_id, &corpus, &actor)
            .map_err(lifecycle_error)
    })
    .await;
    match result {
        Ok(Ok(version)) => Json(serde_json::json!({
            "ok": true,
            "submission": version,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: legacy promotion task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "promotion_failed",
                "the legacy document promotion failed",
            )
        }
    }
}

fn processing_error(err: document_processing::ProcessingError) -> Response {
    eprintln!("api: document processing failed: {err}");
    match err {
        document_processing::ProcessingError::NotFound => error(
            StatusCode::NOT_FOUND,
            "unknown_job",
            "no such processing job",
        ),
        document_processing::ProcessingError::StaleApproval => error(
            StatusCode::CONFLICT,
            "stale_approval",
            "the document approval is no longer valid",
        ),
        document_processing::ProcessingError::ClaimLost => error(
            StatusCode::CONFLICT,
            "claim_lost",
            "the processing claim was taken over by another worker",
        ),
        document_processing::ProcessingError::MissingOriginal => error(
            StatusCode::CONFLICT,
            "document_original_missing",
            "the document original is unavailable",
        ),
        document_processing::ProcessingError::Ingest(_) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "processing_failed",
            "the document could not be processed",
        ),
        document_processing::ProcessingError::InvalidLease(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_lease",
            "the processing lease configuration is invalid",
        ),
        document_processing::ProcessingError::Storage
        | document_processing::ProcessingError::Database(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "processing_unavailable",
            "the document processor is unavailable",
        ),
    }
}

#[allow(clippy::result_large_err)]
async fn list_document_jobs(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::DocumentProcess) {
        return response;
    }
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        document_processing::list_jobs(&mut client, &corpus).map_err(processing_error)
    })
    .await;
    match result {
        Ok(Ok(jobs)) => Json(serde_json::json!({ "ok": true, "jobs": jobs })).into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document jobs task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "jobs_failed",
                "the document jobs could not be listed",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn run_document_jobs(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, Operation::DocumentProcess) {
        return response;
    }
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        let store = PgStore::connect(&config.database_url).map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "processing_unavailable",
                "the document processor is unavailable",
            )
        })?;
        let storage = DocumentStorage::new(config.document_root.clone());
        let transport = Arc::new(HttpTransport::new(config.request_timeout).map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "processing_unavailable",
                "the model transport is unavailable",
            )
        })?);
        let model = OllamaClient::new(config.ollama_url.clone(), transport, config.request_timeout);
        let embedder = OllamaEmbedder::new(model, config.embed_model.clone())
            .with_embed_dim(config.embed_dim as usize);
        document_processing::run_due_jobs(
            &mut client,
            &store,
            &embedder,
            &storage,
            &config,
            "api-worker",
            10,
            Some(&corpus),
            300,
        )
        .map_err(processing_error)
    })
    .await;
    match result {
        Ok(Ok(outcomes)) => Json(serde_json::json!({
            "ok": true,
            "processed": outcomes,
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document run task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "jobs_failed",
                "the document jobs could not be run",
            )
        }
    }
}

#[allow(clippy::result_large_err)]
async fn retry_document_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<i64>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::DocumentProcess) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let config = state.config.clone();
    let corpus = state.corpus.clone();
    let actor = identity.subject().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let mut client = open_rag(&config)?;
        document_processing::retry_job(&mut client, job_id, &actor, &corpus)
            .map_err(processing_error)
    })
    .await;
    match result {
        Ok(Ok(job)) => Json(serde_json::json!({
            "ok": true,
            "job_id": job.job_id,
            "version_id": job.version_id,
            "status": "queued",
        }))
        .into_response(),
        Ok(Err(response)) => response,
        Err(err) => {
            eprintln!("api: document retry task failed: {err}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "jobs_failed",
                "the document job could not be retried",
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
    fn download_file_names_are_header_safe() {
        assert_eq!(safe_download_name("family.md"), "family.md");
        assert_eq!(
            safe_download_name("bad\"name\r\nX-Injected: yes"),
            "bad_name__X-Injected_ yes"
        );
        assert_eq!(safe_download_name(".."), "download");
    }

    fn person_query(q: Option<&str>, limit: Option<i64>) -> NarrativePersonQuery {
        NarrativePersonQuery {
            q: q.map(str::to_string),
            limit,
        }
    }

    #[test]
    fn person_search_query_validation_is_shared_and_bounded() {
        assert_eq!(
            validate_person_search(&person_query(None, None)),
            Ok(PersonSearchQuery::Empty)
        );
        assert_eq!(
            validate_person_search(&person_query(Some("   "), Some(5))),
            Ok(PersonSearchQuery::Empty)
        );
        assert_eq!(
            validate_person_search(&person_query(Some("synthetic"), None)),
            Ok(PersonSearchQuery::Search {
                term: "synthetic".to_string(),
                limit: 10,
            })
        );

        let long = "x".repeat(201);
        assert_eq!(
            validate_person_search(&person_query(Some(&long), None)),
            Err(PersonSearchQueryError::TooLong)
        );
        for limit in [0, 21] {
            assert_eq!(
                validate_person_search(&person_query(Some("x"), Some(limit))),
                Err(PersonSearchQueryError::LimitOutOfRange)
            );
        }
        for limit in [1, 20] {
            assert_eq!(
                validate_person_search(&person_query(Some("x"), Some(limit))),
                Ok(PersonSearchQuery::Search {
                    term: "x".to_string(),
                    limit,
                })
            );
        }
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
    fn bulk_contact_body_serializes_only_the_masked_email() {
        let masked = Some("a•••@example.test".to_string());
        let email = BulkContactBody::from_masked(&masked);
        let value = serde_json::to_value(&email).expect("serialize email contact");
        assert_eq!(
            value,
            serde_json::json!({ "type": "email", "masked": "a•••@example.test" })
        );
        assert!(!value.to_string().contains("ada@example.test"));

        let none = BulkContactBody::from_masked(&None);
        assert_eq!(
            serde_json::to_value(&none).expect("serialize no contact"),
            serde_json::json!({ "type": "none" })
        );
    }

    #[test]
    fn only_an_accepted_reset_request_is_reported_as_success() {
        assert_eq!(
            reset_response(Ok(ResetState::Requested)).status(),
            StatusCode::OK
        );
        for (outcome, status) in [
            (
                Ok(ResetState::NotConfigured),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (Ok(ResetState::Failed), StatusCode::BAD_GATEWAY),
            (Ok(ResetState::Throttled), StatusCode::TOO_MANY_REQUESTS),
            (Err(ResetError::UnknownUser), StatusCode::NOT_FOUND),
            (Err(ResetError::Deactivated), StatusCode::CONFLICT),
        ] {
            assert_eq!(reset_response(outcome).status(), status);
        }
    }

    #[test]
    fn unknown_directory_filters_are_rejected() {
        let query = |status: &str, role: &str| FamilyMembersQuery {
            q: None,
            status: Some(status.into()),
            role: Some(role.into()),
            after: None,
            limit: None,
        };
        assert!(member_filter(query("invited", "")).is_err());
        assert!(member_filter(query("", "root")).is_err());
        let filter = member_filter(query("active", "admin")).ok().unwrap();
        assert_eq!(filter.status, Some(MemberStatus::Active));
        assert_eq!(filter.role, Some(Role::Admin));
    }

    #[test]
    fn photograph_json_omits_storage_details() {
        let photo = FamilyArchivePhotograph {
            asset_id: 7,
            caption: Some("Synthetic caption".to_string()),
            mime_type: "image/jpeg".to_string(),
            byte_size: 42,
            width: Some(8),
            height: Some(8),
            document_version_id: 3,
            corpus: "family".to_string(),
            page_number: 4,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let family = archive_photograph_json(&photo).to_string();
        assert!(family.contains("PDF page 4"));
        assert!(!family.contains("storage_key"));
        assert!(!family.contains("content_sha256"));

        let admin = AdminArchivePhotograph {
            asset_id: 7,
            caption: Some("Synthetic caption".to_string()),
            publication_state: "unpublished".to_string(),
            mime_type: "image/jpeg".to_string(),
            byte_size: 42,
            width: Some(8),
            height: Some(8),
            object_id: "12 0".to_string(),
            classification: "photograph".to_string(),
            classification_rule: "photograph-inset-opaque".to_string(),
            document_version_id: 3,
            corpus: "family".to_string(),
            page_numbers: vec![4],
            created_at: "2026-01-01T00:00:00Z".to_string(),
            links: Vec::new(),
        };
        let admin = admin_photograph_json(&admin).to_string();
        assert!(admin.contains("PDF page 4"));
        assert!(!admin.contains("storage_key"));
        assert!(!admin.contains("content_sha256"));
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
    #[test]
    fn document_ownership_is_issuer_qualified() {
        let version = DocumentVersion {
            version_id: 1,
            document_key: "synthetic".to_string(),
            version_number: 1,
            corpus: "synthetic-corpus".to_string(),
            source: "synthetic.md".to_string(),
            title: "Synthetic".to_string(),
            description: None,
            category: "other".to_string(),
            origin: "submission".to_string(),
            access_scope: Some("family".to_string()),
            owner_subject: Some("sub-1".to_string()),
            owner_issuer: Some("https://issuer-a".to_string()),
            review_status: "pending_review".to_string(),
            processing_status: "unprocessed".to_string(),
            content_sha256: "a".repeat(64),
            byte_size: 1,
            mime_type: "text/plain".to_string(),
            original_filename: "synthetic.md".to_string(),
            reviewed_by: None,
            reviewed_at: None,
            review_note: None,
            removed_at: None,
            created_by: "sub-1".to_string(),
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
        };
        let same_issuer =
            VerifiedIdentity::new("sub-1", "https://issuer-a", vec![Role::Editor], vec![]);
        let other_issuer =
            VerifiedIdentity::new("sub-1", "https://issuer-b", vec![Role::Editor], vec![]);
        let admin = VerifiedIdentity::new("other", "https://issuer-b", vec![Role::Admin], vec![]);
        assert!(can_access_document(&same_issuer, &version));
        assert!(!can_access_document(&other_issuer, &version));
        assert!(can_access_document(&admin, &version));
    }
}
