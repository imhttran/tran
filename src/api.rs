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
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::corpus::Corpus;
use crate::embedding::OllamaEmbedder;
use crate::fingerprint::{fingerprint, ChunkerConfig};
use crate::identity::{self, Decision, Operation, Role, VerifiedIdentity};
use crate::identity_store::{IdentityStore, IdentityStoreError};
use crate::ingest::IngestRejection;
use crate::oidc::{validate_token, AuthConfig, AuthError, Jwks, JwksCache};
use crate::ollama::{HttpTransport, OllamaClient};
use crate::pipeline::{IngestOutcome, PipelineConfig, PipelineError};
use crate::query_service::QueryService;
use crate::ratelimit::RateLimiter;
use crate::service::IngestService;
use crate::storage_pg::PgStore;

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

/// Returns the default bind host.
pub fn default_host() -> &'static str {
    DEFAULT_HOST
}

/// Returns the default bind port.
pub fn default_port() -> u16 {
    DEFAULT_PORT
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
}

/// The JWKS fetcher type held by the cached verifier.
type JwksFetcher = Box<dyn Fn() -> Result<Jwks, AuthError> + Send + Sync>;

impl AppState {
    /// Builds the state from the loaded configuration.
    ///
    /// Only the database pool is created here. The Ollama clients are built per
    /// request inside a blocking task: `reqwest::blocking` must not be
    /// constructed on a Tokio runtime thread.
    pub fn new(config: Config, corpus: impl Into<String>) -> Result<Self, String> {
        let corpus: String = corpus.into();
        let corpus = Corpus::new(&corpus).map_err(|err| err.to_string())?;
        let store = Arc::new(
            PgStore::connect(&config.database_url)
                .map_err(|err| format!("connect to the document store: {err}"))?,
        );
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
        Ok(Self {
            config: Arc::new(config),
            store,
            corpus: corpus.as_str().to_string(),
            auth: Arc::new(auth),
            jwks,
            limiter,
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
            .route("/api/admin/roles", post(change_role))
            .route(
                "/api/admin/head-of-family/approvals",
                post(approve_head_of_family),
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
pub async fn serve(config: Config) -> Result<(), String> {
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
        .map_err(|err| format!("bind {host}:{port}: {err}"))?;
    let address = listener
        .local_addr()
        .map_err(|err| format!("read the bound address: {err}"))?;
    eprintln!("api: listening on http://{address}");
    axum::serve(listener, state.router())
        .await
        .map_err(|err| format!("serve: {err}"))
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
    match validate_token(&token, &state.auth, &state.jwks) {
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

/// Runs authentication, the central policy decision, and rate limiting in the
/// required order. Every protected handler calls this first.
#[allow(clippy::result_large_err)]
fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    operation: Operation,
) -> Result<VerifiedIdentity, Response> {
    let identity = authenticate(state, headers)?;
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
    if operation.is_rate_limited()
        && !state
            .limiter
            .check(identity.subject(), operation.rate_limit_key())
    {
        return Err(error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many requests for this operation",
        ));
    }
    Ok(identity)
}

// --- health ----------------------------------------------------------------

fn model_reachable(url: &str) -> bool {
    let endpoint = format!("{}/api/tags", url.trim_end_matches('/'));
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .and_then(|client| client.get(endpoint).send())
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
    let model_ok = tokio::task::spawn_blocking(move || model_reachable(&url))
        .await
        .unwrap_or(false);
    let healthy = database_ok && model_ok;
    let body = serde_json::json!({
        "status": if healthy { "ok" } else { "degraded" },
        "service": "rust-api",
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

#[derive(Deserialize)]
struct RoleChangeRequest {
    subject: String,
    role: String,
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
    let config = state.config.clone();
    let actor = identity.subject().to_string();
    let revoke = request.revoke;
    let task_subject = subject.clone();
    let result = tokio::task::spawn_blocking(move || {
        let client = crate::identity_store_pg::PgIdentityClient::connect(&config.database_url)?;
        let mut store = IdentityStore::new(client);
        if revoke {
            store.revoke_role(&task_subject, role, &actor)
        } else {
            store.grant_role(&task_subject, role, &actor)
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

#[derive(Deserialize)]
struct ApprovalRequest {
    subject: String,
}

async fn approve_head_of_family(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ApprovalRequest>,
) -> Response {
    let identity = match authorize(&state, &headers, Operation::HeadOfFamilyApproval) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    Json(serde_json::json!({
        "ok": true,
        "approved": request.subject.trim(),
        "approved_by": identity.subject(),
        "status": "pending_catalog",
    }))
    .into_response()
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
    let text = error.to_string().to_ascii_lowercase();
    text.contains("connection")
        || text.contains("pool")
        || text.contains("timeout")
        || text.contains("request")
        || text.contains("connect")
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

    let temp_dir = std::env::temp_dir().join(format!(
        "rag-api-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0)
    ));
    if let Err(err) = std::fs::create_dir_all(&temp_dir) {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            format!("could not stage the upload: {err}"),
        );
    }
    let extension = extension(&source).unwrap_or_else(|| "bin".to_string());
    let staged = temp_dir.join(format!("source.{extension}"));
    if let Err(err) = std::fs::write(&staged, &bytes) {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            format!("could not stage the upload: {err}"),
        );
    }

    let config = state.config.clone();
    let store = state.store.clone();
    let corpus = state.corpus.clone();
    let source_for_run = source.clone();
    let staged_for_run = staged.clone();
    let chunker = ChunkerConfig::new(config.chunk_size as usize, config.chunk_overlap as usize);
    let content_hash = fingerprint(&bytes, &chunker);

    let outcome = tokio::task::spawn_blocking(move || {
        let transport = Arc::new(HttpTransport::new(config.request_timeout));
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
        service.ingest_source(&staged_for_run, &source_for_run, &content_hash)
    })
    .await;

    let _ = std::fs::remove_dir_all(&temp_dir);

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
        Ok(Err(err)) => {
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
    let answer = tokio::task::spawn_blocking(move || {
        let service = QueryService::from_config(&config, store, corpus)?;
        service.answer(&question)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_binding_is_loopback() {
        assert_eq!(default_host(), "127.0.0.1");
        assert_eq!(default_port(), 8080);
    }

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
    fn extension_detection_is_case_insensitive() {
        assert_eq!(extension("Policy.MD"), Some("md".to_string()));
        assert_eq!(extension("no-extension"), None);
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
