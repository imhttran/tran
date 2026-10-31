//! Configuration parsing, defaults, and validation.
//!
//! Ported from the Go baseline in `internal/config/config.go`. Every setting can
//! be overridden with an environment variable. A setting that is absent (unset
//! or blank) falls back to its default. A setting that is present but invalid is
//! an error, never a silent fallback: a typo in `CHUNK_SIZE` stops the command
//! with a message naming the variable instead of quietly using the default.

use std::env;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use thiserror::Error;

use crate::admin_members::ResetProvider;
use crate::provider;

// Default values, matching the Go baseline exactly.
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
pub const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";
pub const DEFAULT_CHAT_MODEL: &str = "qwen3.8:27b-mlx";
pub const DEFAULT_DATABASE_URL: &str = "postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable";
pub const DEFAULT_TOP_K: i64 = 4;
pub const DEFAULT_EMBED_DIM: i64 = 768;
pub const DEFAULT_EMBED_PROVIDER: &str = provider::DEFAULT;
pub const DEFAULT_GEN_PROVIDER: &str = provider::DEFAULT;
/// Default server root for `GEN_PROVIDER=openai_compatible` (oMLX's default).
pub const DEFAULT_OPENAI_COMPATIBLE_URL: &str = "http://127.0.0.1:8000";
pub const DEFAULT_CHUNK_SIZE: i64 = 50;
pub const DEFAULT_CHUNK_OVERLAP: i64 = 20;
pub const DEFAULT_MIN_SIMILARITY: f64 = 0.6;
pub const DEFAULT_MEDIA_ROOT: &str = "./data/media";
/// Private root for family document originals. Never statically served.
pub const DEFAULT_DOCUMENT_ROOT: &str = "./data/documents";
pub const DEFAULT_QUERY_REWRITE: bool = true;
pub const DEFAULT_RAG_ANSWERABILITY_GATE: bool = true;
pub const DEFAULT_RAG_CITATION_VALIDATION: bool = false;
pub const DEFAULT_FINAL_K: i64 = 3;
pub const DEFAULT_EXPAND_LIMIT: i64 = 20;
pub const DEFAULT_CONTEXT_BUDGET: i64 = 0;
pub const DEFAULT_MAX_QUESTION_BYTES: i64 = 0;
pub const DEFAULT_MAX_INPUT_BYTES: i64 = 0;
pub const DEFAULT_LANGUAGE: &str = "";
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const DEFAULT_JWKS_CACHE_TTL_SECS: i64 = 300;
pub const DEFAULT_AUTH_CLOCK_SKEW_SECS: i64 = 60;
/// Idle limit for a renewed browser session (30 minutes).
pub const DEFAULT_SESSION_IDLE_SECONDS: i64 = 1_800;
/// Absolute limit for a browser session (8 hours).
pub const DEFAULT_SESSION_ABSOLUTE_SECONDS: i64 = 28_800;
/// How long before token expiry the store refreshes it (60 seconds).
pub const DEFAULT_SESSION_REFRESH_WINDOW_SECONDS: i64 = 60;
pub const DEFAULT_AUTH_RATE_LIMIT: i64 = 60;
pub const DEFAULT_PASSWORD_RESET_COOLDOWN_SECS: i64 = 300;

/// Bulk invitation delivery adapter: `unconfigured` (default), `fake`, or
/// `http`.
pub const DEFAULT_ONBOARDING_MAIL_ADAPTER: &str = "unconfigured";
/// Request timeout for the `http` mail adapter, in seconds.
pub const DEFAULT_ONBOARDING_MAIL_TIMEOUT_SECS: i64 = 10;
/// Base URL used to build the activation link handed to the adapter.
pub const DEFAULT_ONBOARDING_ACTIVATION_URL: &str = "/activate";
/// Minimum delay between bulk invitation sends, in milliseconds.
pub const DEFAULT_ONBOARDING_SEND_INTERVAL_MS: i64 = 0;
/// Default SMTP port for the local Mailpit-style sink.
pub const DEFAULT_ONBOARDING_SMTP_PORT: i64 = 1025;
/// Default SMTP transport security mode. Explicit; never negotiated.
pub const DEFAULT_ONBOARDING_SMTP_TLS: &str = "none";
/// Default bounded send timeout for the SMTP adapter, in seconds.
pub const DEFAULT_ONBOARDING_SMTP_TIMEOUT_SECS: i64 = 10;
/// Upper bound on the SMTP send timeout so a stalled host cannot hang a worker.
pub const MAX_ONBOARDING_SMTP_TIMEOUT_SECS: i64 = 300;

/// Name of the explicitly configured genealogy database DSN variable. It has no
/// default, so a genealogy command can never silently target the RAG database.
pub const GENEALOGY_DATABASE_URL_VAR: &str = "GENEALOGY_DATABASE_URL";

/// Name of the verified third-branch key used by Head of Family writes.
pub const HOF_THIRD_BRANCH_KEY_VAR: &str = "HOF_THIRD_BRANCH_KEY";

/// Error returned by [`Config::load`]. The message names the offending variable
/// so an invalid setting is never silently replaced with a default.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("{0}")]
    Invalid(String),
}

/// Runtime settings, mirroring the Go `Config` struct.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub ollama_url: String,
    pub embed_model: String,
    pub embed_dim: i64,
    pub chat_model: String,
    pub database_url: String,
    /// Explicit genealogy database DSN. `None` when the variable is unset or
    /// blank; there is deliberately no fallback to the RAG `database_url`.
    pub genealogy_database_url: Option<String>,
    /// Explicit verified third-branch key. `None` when unset; Head of Family
    /// writes fail closed rather than guessing a branch.
    pub hof_third_branch_key: Option<String>,
    pub question: String,
    pub embed_provider: String,
    pub gen_provider: String,
    /// Server root for `GEN_PROVIDER=openai_compatible`
    /// (`OPENAI_COMPATIBLE_BASE_URL`, default `http://127.0.0.1:8000`).
    pub openai_compatible_url: String,
    /// Bearer key for that server (`OPENAI_COMPATIBLE_API_KEY`), if it needs one.
    pub openai_compatible_api_key: Option<String>,
    /// Model for that server (`OPENAI_COMPATIBLE_MODEL`); required when the
    /// provider is selected, empty otherwise.
    pub openai_compatible_model: String,
    pub language: String,
    pub chunk_size: i64,
    pub chunk_overlap: i64,
    pub top_k: i64,
    pub final_k: i64,
    pub expand_limit: i64,
    pub context_budget: i64,
    pub max_question_bytes: i64,
    pub max_input_bytes: i64,
    pub min_similarity: f64,
    pub rag_answerability_gate: bool,
    pub rag_citation_validation: bool,
    pub request_timeout: Duration,
    pub query_rewrite: bool,
    pub auth_issuer: String,
    pub auth_audience: String,
    pub jwks_url: String,
    pub jwks_cache_ttl_secs: i64,
    pub auth_clock_skew_secs: i64,
    pub auth_rate_limit: i64,
    /// Bulk invitation delivery adapter name. Defaults to `unconfigured`, which
    /// fails closed without creating an invitation.
    pub onboarding_mail_adapter: String,
    /// HTTPS endpoint for the `http` mail adapter. `None` fails closed even when
    /// the adapter name is `http`.
    pub onboarding_mail_url: Option<String>,
    /// Optional bearer credential for the `http` mail adapter.
    pub onboarding_mail_bearer: Option<String>,
    /// Request timeout for the `http` mail adapter.
    pub onboarding_mail_timeout_secs: u64,
    /// Base URL for the one-time activation link.
    pub onboarding_activation_url: String,
    /// Delay between bulk sends, in milliseconds.
    pub onboarding_send_interval_ms: i64,
    /// Keycloak service-account settings for creating invitees' sign-ins.
    /// `None` unless `ONBOARDING_KEYCLOAK_LOGINS` is on.
    pub onboarding_keycloak_logins: Option<crate::onboarding::KeycloakLoginSettings>,
    /// Validated SMTP settings for the `smtp` mail adapter. `None` for any other
    /// adapter name.
    pub onboarding_smtp: Option<crate::onboarding::SmtpSettings>,
    /// Private filesystem root for owner media. Never served as a static tree.
    pub media_root: PathBuf,
    /// Private filesystem root for family document originals. Never served as a
    /// static tree; originals are returned only by authorized handlers.
    pub document_root: PathBuf,
    /// Provider endpoint that sends password-reset email. `None` when
    /// `PASSWORD_RESET_URL` is unset: reset requests then fail closed.
    pub password_reset: Option<ResetProvider>,
    /// Minimum time between accepted reset requests for one account.
    pub password_reset_cooldown: Duration,
    /// AES-256-GCM key for refresh and ID tokens at rest. `None` disables
    /// automatic renewal; the cookie-only session remains in force.
    pub session_store_key: Option<Vec<u8>>,
    /// Shared secret the Next.js server sends on internal session endpoints.
    pub web_session_secret: Option<String>,
    /// Idle limit for a stored browser session.
    pub session_idle_seconds: i64,
    /// Absolute limit for a stored browser session.
    pub session_absolute_seconds: i64,
    /// How long before expiry the stored token is refreshed.
    pub session_refresh_window_seconds: i64,
    /// OIDC token endpoint used for the refresh grant.
    pub oidc_token_endpoint: Option<String>,
    /// OIDC client id used for the refresh grant.
    pub oidc_client_id: Option<String>,
    /// OIDC client secret for a confidential client. Server-side only.
    pub oidc_client_secret: Option<String>,
    /// Optional provider revocation endpoint used on sign-out.
    pub oidc_revocation_endpoint: Option<String>,
}

impl Config {
    /// Reads the settings from the environment, falling back to the defaults
    /// when a setting is absent.
    ///
    /// Returns a [`ConfigError`] naming the offending variable when a present
    /// setting cannot be parsed or is out of range, when `CHUNK_OVERLAP` is not
    /// smaller than `CHUNK_SIZE`, and when `EMBED_PROVIDER` / `GEN_PROVIDER`
    /// names a provider that is not registered in [`crate::provider`].
    pub fn load() -> Result<Config, ConfigError> {
        Self::load_from(|key| env::var(key).ok())
    }

    /// Like [`Config::load`] but reads variables through the supplied lookup, so
    /// tests can supply a controlled environment.
    pub fn load_from<F>(lookup: F) -> Result<Config, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let get = |key: &str| env_value(&lookup, key);

        let embed_provider =
            get("EMBED_PROVIDER").unwrap_or_else(|| DEFAULT_EMBED_PROVIDER.to_string());
        let gen_provider = get("GEN_PROVIDER").unwrap_or_else(|| DEFAULT_GEN_PROVIDER.to_string());

        validate_provider("EMBED_PROVIDER", &embed_provider)?;
        validate_provider("GEN_PROVIDER", &gen_provider)?;
        let openai_compatible_model = get("OPENAI_COMPATIBLE_MODEL")
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        if provider::resolve_name(&gen_provider) == provider::OPENAI_COMPATIBLE
            && openai_compatible_model.is_empty()
        {
            return Err(ConfigError::Invalid(
                "OPENAI_COMPATIBLE_MODEL is required when GEN_PROVIDER=openai_compatible".into(),
            ));
        }
        let openai_compatible_api_key = get("OPENAI_COMPATIBLE_API_KEY")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let chunk_size = positive_int(&get, "CHUNK_SIZE", DEFAULT_CHUNK_SIZE)?;
        let chunk_overlap = non_negative_int(&get, "CHUNK_OVERLAP", DEFAULT_CHUNK_OVERLAP)?;

        if chunk_overlap >= chunk_size {
            return Err(ConfigError::Invalid(format!(
                "CHUNK_OVERLAP ({chunk_overlap}) must be smaller than CHUNK_SIZE ({chunk_size})"
            )));
        }

        // An unset or blank variable stays `None`; it never falls back to the
        // RAG default. A caller that needs the genealogy database must treat
        // `None` as a configuration error naming the variable.
        let genealogy_database_url = get(GENEALOGY_DATABASE_URL_VAR);
        let hof_third_branch_key = get(HOF_THIRD_BRANCH_KEY_VAR)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let media_root = get("MEDIA_ROOT").unwrap_or_else(|| DEFAULT_MEDIA_ROOT.to_string());
        let document_root =
            get("DOCUMENT_ROOT").unwrap_or_else(|| DEFAULT_DOCUMENT_ROOT.to_string());
        if document_root.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "DOCUMENT_ROOT must not be blank".to_string(),
            ));
        }
        if media_root.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "MEDIA_ROOT must not be blank".to_string(),
            ));
        }

        let password_reset = match get("PASSWORD_RESET_URL") {
            None => None,
            Some(url) => {
                let url = url.trim().to_string();
                let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
                    .iter()
                    .any(|prefix| {
                        url.strip_prefix(prefix)
                            .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
                    });
                if !local && !url.starts_with("https://") {
                    return Err(ConfigError::Invalid(
                        "PASSWORD_RESET_URL must use https:// (or http:// on loopback)".into(),
                    ));
                }
                Some(ResetProvider {
                    url,
                    bearer: get("PASSWORD_RESET_TOKEN"),
                })
            }
        };

        let session_store_key = match get("SESSION_STORE_KEY") {
            Some(raw) => Some(decode_session_store_key(&raw)?),
            None => None,
        };
        let web_session_secret = get("WEB_SESSION_SECRET")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let session_idle_seconds =
            positive_int(&get, "SESSION_IDLE_SECONDS", DEFAULT_SESSION_IDLE_SECONDS)?;
        let session_absolute_seconds = positive_int(
            &get,
            "SESSION_ABSOLUTE_SECONDS",
            DEFAULT_SESSION_ABSOLUTE_SECONDS,
        )?;
        if session_idle_seconds < 120 {
            return Err(ConfigError::Invalid(
                "SESSION_IDLE_SECONDS must be at least 120 so active sessions are touched before they idle out"
                    .to_string(),
            ));
        }
        if session_idle_seconds > session_absolute_seconds {
            return Err(ConfigError::Invalid(format!(
                "SESSION_IDLE_SECONDS ({session_idle_seconds}) must not exceed \
                 SESSION_ABSOLUTE_SECONDS ({session_absolute_seconds})"
            )));
        }
        let session_refresh_window_seconds = non_negative_int(
            &get,
            "SESSION_REFRESH_WINDOW_SECONDS",
            DEFAULT_SESSION_REFRESH_WINDOW_SECONDS,
        )?;
        let auth_audience = get("AUTH_AUDIENCE").unwrap_or_default();
        let oidc_client_id = get("OIDC_CLIENT_ID")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                let audience = auth_audience.trim().to_string();
                if audience.is_empty() {
                    None
                } else {
                    Some(audience)
                }
            });
        let oidc_token_endpoint = get("OIDC_TOKEN_ENDPOINT")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let oidc_client_secret = get("OIDC_CLIENT_SECRET")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let oidc_revocation_endpoint = get("OIDC_REVOCATION_ENDPOINT")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let onboarding_mail_adapter = onboarding_mail_adapter(&get)?;
        let onboarding_smtp = onboarding_smtp(&get, &onboarding_mail_adapter)?;
        let onboarding_keycloak_logins = onboarding_keycloak_logins(&get)?;

        Ok(Config {
            ollama_url: get("OLLAMA_URL").unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_string()),
            embed_model: get("OLLAMA_EMBED_MODEL")
                .unwrap_or_else(|| DEFAULT_EMBED_MODEL.to_string()),
            embed_dim: positive_int(&get, "EMBED_DIM", DEFAULT_EMBED_DIM)?,
            chat_model: get("OLLAMA_CHAT_MODEL").unwrap_or_else(|| DEFAULT_CHAT_MODEL.to_string()),
            database_url: get("DATABASE_URL").unwrap_or_else(|| DEFAULT_DATABASE_URL.to_string()),
            genealogy_database_url,
            hof_third_branch_key,
            question: get("QUESTION").unwrap_or_default(),
            embed_provider,
            gen_provider,
            openai_compatible_url: get("OPENAI_COMPATIBLE_BASE_URL")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_OPENAI_COMPATIBLE_URL.to_string()),
            openai_compatible_api_key,
            openai_compatible_model,
            language: get("CORPUS_LANGUAGE").unwrap_or_else(|| DEFAULT_LANGUAGE.to_string()),
            chunk_size,
            chunk_overlap,
            top_k: positive_int(&get, "TOP_K", DEFAULT_TOP_K)?,
            final_k: positive_int(&get, "FINAL_K", DEFAULT_FINAL_K)?,
            expand_limit: positive_int(&get, "EXPAND_LIMIT", DEFAULT_EXPAND_LIMIT)?,
            context_budget: non_negative_int(&get, "CONTEXT_BUDGET", DEFAULT_CONTEXT_BUDGET)?,
            max_question_bytes: non_negative_int(
                &get,
                "MAX_QUESTION_BYTES",
                DEFAULT_MAX_QUESTION_BYTES,
            )?,
            max_input_bytes: non_negative_int(&get, "MAX_INPUT_BYTES", DEFAULT_MAX_INPUT_BYTES)?,
            min_similarity: unit_float(&get, "MIN_SIMILARITY", DEFAULT_MIN_SIMILARITY)?,
            rag_answerability_gate: bool_setting(
                &get,
                "RAG_ANSWERABILITY_GATE",
                DEFAULT_RAG_ANSWERABILITY_GATE,
            )?,
            rag_citation_validation: bool_setting(
                &get,
                "RAG_CITATION_VALIDATION",
                DEFAULT_RAG_CITATION_VALIDATION,
            )?,
            request_timeout: positive_duration(&get, "REQUEST_TIMEOUT", DEFAULT_REQUEST_TIMEOUT)?,
            query_rewrite: bool_setting(&get, "QUERY_REWRITE", DEFAULT_QUERY_REWRITE)?,
            auth_issuer: get("AUTH_ISSUER").unwrap_or_default(),
            auth_audience,
            jwks_url: get("JWKS_URL").unwrap_or_default(),
            jwks_cache_ttl_secs: non_negative_int(
                &get,
                "JWKS_CACHE_TTL_SECS",
                DEFAULT_JWKS_CACHE_TTL_SECS,
            )?,
            auth_clock_skew_secs: non_negative_int(
                &get,
                "AUTH_CLOCK_SKEW_SECS",
                DEFAULT_AUTH_CLOCK_SKEW_SECS,
            )?,
            auth_rate_limit: positive_int(&get, "AUTH_RATE_LIMIT", DEFAULT_AUTH_RATE_LIMIT)?,
            onboarding_mail_adapter,
            onboarding_smtp: onboarding_smtp.clone(),
            onboarding_keycloak_logins,
            onboarding_mail_url: match get("ONBOARDING_MAIL_URL") {
                None => None,
                Some(url) => Some(validate_provider_endpoint("ONBOARDING_MAIL_URL", &url)?),
            },
            onboarding_mail_bearer: get("ONBOARDING_MAIL_BEARER")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            onboarding_mail_timeout_secs: positive_int(
                &get,
                "ONBOARDING_MAIL_TIMEOUT_SECS",
                DEFAULT_ONBOARDING_MAIL_TIMEOUT_SECS,
            )? as u64,
            onboarding_activation_url: get("ONBOARDING_ACTIVATION_URL")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_ONBOARDING_ACTIVATION_URL.to_string()),
            onboarding_send_interval_ms: non_negative_int(
                &get,
                "ONBOARDING_SEND_INTERVAL_MS",
                DEFAULT_ONBOARDING_SEND_INTERVAL_MS,
            )?,
            media_root: PathBuf::from(media_root),
            document_root: PathBuf::from(document_root),
            password_reset,
            password_reset_cooldown: Duration::from_secs(non_negative_int(
                &get,
                "PASSWORD_RESET_COOLDOWN_SECS",
                DEFAULT_PASSWORD_RESET_COOLDOWN_SECS,
            )? as u64),
            session_store_key,
            web_session_secret,
            session_idle_seconds,
            session_absolute_seconds,
            session_refresh_window_seconds,
            oidc_token_endpoint,
            oidc_client_id,
            oidc_client_secret,
            oidc_revocation_endpoint,
        })
    }

    /// Builds the configured bulk delivery adapter.
    ///
    /// `fake` is synthetic and never performs network I/O. `http` posts one
    /// JSON message to `ONBOARDING_MAIL_URL` and reports provider acceptance
    /// only. Any other name, or `http` without a valid URL, fails closed.
    ///
    /// With `ONBOARDING_KEYCLOAK_LOGINS` on, that adapter is wrapped so each
    /// invitation also creates the invitee's Keycloak sign-in.
    pub fn mail_adapter(&self) -> Box<dyn crate::onboarding::MailAdapter> {
        let inner = self.plain_mail_adapter();
        match self.onboarding_keycloak_logins.clone() {
            Some(settings) => Box::new(crate::onboarding::KeycloakLoginAdapter::new(
                settings, inner,
            )),
            None => inner,
        }
    }

    fn plain_mail_adapter(&self) -> Box<dyn crate::onboarding::MailAdapter> {
        match self.onboarding_mail_adapter.as_str() {
            "fake" => Box::new(crate::onboarding::FakeAdapter::accepting()),
            "http" => match self.onboarding_mail_url.as_deref() {
                Some(url) if !url.trim().is_empty() => {
                    Box::new(crate::onboarding::HttpMailAdapter::new(
                        url,
                        self.onboarding_mail_bearer.clone(),
                        Duration::from_secs(self.onboarding_mail_timeout_secs.max(1)),
                    ))
                }
                _ => Box::new(crate::onboarding::UnconfiguredAdapter),
            },
            "smtp" => match self.onboarding_smtp.clone() {
                Some(settings) => Box::new(crate::onboarding::SmtpMailAdapter::new(settings)),
                None => Box::new(crate::onboarding::UnconfiguredAdapter),
            },
            _ => Box::new(crate::onboarding::UnconfiguredAdapter),
        }
    }

    /// The delay between bulk sends.
    pub fn onboarding_send_throttle(&self) -> Duration {
        Duration::from_millis(self.onboarding_send_interval_ms.max(0) as u64)
    }

    /// Returns the explicitly configured genealogy database DSN.
    ///
    /// There is no fallback: an absent or blank `GENEALOGY_DATABASE_URL` is a
    /// [`ConfigError::Invalid`] that names the variable, so a genealogy command
    /// can never silently target the RAG database.
    pub fn genealogy_database_url(&self) -> Result<&str, ConfigError> {
        match self.genealogy_database_url.as_deref() {
            Some(dsn) => Ok(dsn),
            None => Err(ConfigError::Invalid(format!(
                "{GENEALOGY_DATABASE_URL_VAR} must be set to an explicit genealogy database DSN"
            ))),
        }
    }

    /// Returns the explicitly configured verified third-branch key.
    ///
    /// Head of Family writes fail closed when this is unset; there is no
    /// generation-3 or name-based fallback.
    pub fn hof_third_branch_key(&self) -> Result<&str, ConfigError> {
        match self.hof_third_branch_key.as_deref() {
            Some(value) => Ok(value),
            None => Err(ConfigError::Invalid(format!(
                "{HOF_THIRD_BRANCH_KEY_VAR} must be set to the verified third-branch key"
            ))),
        }
    }
}

/// Validates the bulk delivery adapter name. The fail-closed default, the
/// synthetic test adapter, and the HTTP provider adapter are the supported
/// values; an unknown name is an error rather than a silent fallback.
fn onboarding_mail_adapter<F>(get: &F) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let value = get("ONBOARDING_MAIL_ADAPTER")
        .unwrap_or_else(|| DEFAULT_ONBOARDING_MAIL_ADAPTER.to_string());
    match value.trim() {
        "unconfigured" | "fake" | "http" | "smtp" => Ok(value.trim().to_string()),
        other => Err(ConfigError::Invalid(format!(
            "ONBOARDING_MAIL_ADAPTER must be unconfigured, fake, http, or smtp, got {other:?}"
        ))),
    }
}

/// Builds the SMTP settings for the `smtp` adapter.
///
/// Every field is validated here rather than at send time: a missing host or
/// sender, an unknown TLS mode, an out-of-range port or timeout, and a
/// half-configured credential pair are configuration errors that name the
/// offending variable. Settings are `None` for every other adapter name, so a
/// stale SMTP block cannot affect the HTTP, fake, or unconfigured adapters.
/// Default lifetime of Keycloak's password link: the invitation's own seven days.
pub const DEFAULT_KEYCLOAK_ACTIONS_LIFESPAN_SECS: i64 = 7 * 24 * 60 * 60;

fn onboarding_keycloak_logins<F>(
    get: &F,
) -> Result<Option<crate::onboarding::KeycloakLoginSettings>, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    if !bool_setting(get, "ONBOARDING_KEYCLOAK_LOGINS", false)? {
        return Ok(None);
    }
    let required = |key: &str| -> Result<String, ConfigError> {
        get(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ConfigError::Invalid(format!(
                    "{key} is required when ONBOARDING_KEYCLOAK_LOGINS is on"
                ))
            })
    };
    let base_url = validate_provider_endpoint("KEYCLOAK_URL", &required("KEYCLOAK_URL")?)?;
    if base_url.contains("/realms/") {
        return Err(ConfigError::Invalid(
            "KEYCLOAK_URL must be the Keycloak base URL, not a realm URL".into(),
        ));
    }
    let settings = crate::onboarding::KeycloakLoginSettings {
        base_url,
        realm: required("KEYCLOAK_REALM")?,
        client_id: required("KEYCLOAK_LOGIN_CLIENT_ID")?,
        client_secret: required("KEYCLOAK_LOGIN_CLIENT_SECRET")?,
        actions_client_id: required("KEYCLOAK_ACTIONS_CLIENT_ID")?,
        actions_lifespan: Duration::from_secs(positive_int(
            get,
            "KEYCLOAK_ACTIONS_LIFESPAN_SECS",
            DEFAULT_KEYCLOAK_ACTIONS_LIFESPAN_SECS,
        )? as u64),
        timeout: Duration::from_secs(positive_int(
            get,
            "ONBOARDING_MAIL_TIMEOUT_SECS",
            DEFAULT_ONBOARDING_MAIL_TIMEOUT_SECS,
        )? as u64),
    };
    // A login created in another realm would have a subject this API never
    // accepts, so the realm must be the one that issues the API's tokens.
    let issuer = get("AUTH_ISSUER").unwrap_or_default();
    if issuer.trim().trim_end_matches('/') != settings.issuer() {
        return Err(ConfigError::Invalid(
            "AUTH_ISSUER must equal KEYCLOAK_URL/realms/KEYCLOAK_REALM when ONBOARDING_KEYCLOAK_LOGINS is on".into(),
        ));
    }
    // Keycloak returns the person to the activation link after they choose a
    // password, so it must be an absolute URL.
    validate_provider_endpoint(
        "ONBOARDING_ACTIVATION_URL",
        &get("ONBOARDING_ACTIVATION_URL").unwrap_or_default(),
    )
    .map_err(|_| {
        ConfigError::Invalid(
            "ONBOARDING_ACTIVATION_URL must be an absolute https:// URL (or http:// on loopback) when ONBOARDING_KEYCLOAK_LOGINS is on".into(),
        )
    })?;
    Ok(Some(settings))
}

fn onboarding_smtp<F>(
    get: &F,
    adapter: &str,
) -> Result<Option<crate::onboarding::SmtpSettings>, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    if adapter != "smtp" {
        return Ok(None);
    }

    let host = get("ONBOARDING_SMTP_HOST").ok_or_else(|| {
        ConfigError::Invalid(
            "ONBOARDING_SMTP_HOST is required when ONBOARDING_MAIL_ADAPTER=smtp".into(),
        )
    })?;
    let port = positive_int(get, "ONBOARDING_SMTP_PORT", DEFAULT_ONBOARDING_SMTP_PORT)?;
    if port > 65535 {
        return Err(ConfigError::Invalid(format!(
            "ONBOARDING_SMTP_PORT must be between 1 and 65535, got {port}"
        )));
    }
    let tls_raw =
        get("ONBOARDING_SMTP_TLS").unwrap_or_else(|| DEFAULT_ONBOARDING_SMTP_TLS.to_string());
    let tls = crate::onboarding::SmtpTlsMode::parse(&tls_raw).ok_or_else(|| {
        ConfigError::Invalid(format!(
            "ONBOARDING_SMTP_TLS must be none, implicit, or starttls, got {tls_raw:?}"
        ))
    })?;
    if tls == crate::onboarding::SmtpTlsMode::None
        && !crate::onboarding::delivery::smtp_host_is_loopback(&host)
        && !bool_setting(get, "ONBOARDING_SMTP_ALLOW_PLAINTEXT", false)?
    {
        return Err(ConfigError::Invalid(format!(
            "ONBOARDING_SMTP_TLS=none is permitted only for a loopback \
             ONBOARDING_SMTP_HOST; set ONBOARDING_SMTP_ALLOW_PLAINTEXT=true \
             for an explicitly local container host, got {host:?}"
        )));
    }
    let from = get("ONBOARDING_SMTP_FROM").ok_or_else(|| {
        ConfigError::Invalid(
            "ONBOARDING_SMTP_FROM is required when ONBOARDING_MAIL_ADAPTER=smtp".into(),
        )
    })?;
    lettre::Address::from_str(&from).map_err(|_| {
        ConfigError::Invalid(format!(
            "ONBOARDING_SMTP_FROM must be a single email address, got {from:?}"
        ))
    })?;
    let timeout = positive_int(
        get,
        "ONBOARDING_SMTP_TIMEOUT_SECS",
        DEFAULT_ONBOARDING_SMTP_TIMEOUT_SECS,
    )?;
    if timeout > MAX_ONBOARDING_SMTP_TIMEOUT_SECS {
        return Err(ConfigError::Invalid(format!(
            "ONBOARDING_SMTP_TIMEOUT_SECS must not exceed \
             {MAX_ONBOARDING_SMTP_TIMEOUT_SECS}, got {timeout}"
        )));
    }
    let username = get("ONBOARDING_SMTP_USERNAME");
    let password = get("ONBOARDING_SMTP_PASSWORD");
    if username.is_some() != password.is_some() {
        return Err(ConfigError::Invalid(
            "ONBOARDING_SMTP_USERNAME and ONBOARDING_SMTP_PASSWORD must be set together".into(),
        ));
    }

    Ok(Some(crate::onboarding::SmtpSettings {
        host,
        port: port as u16,
        tls,
        from,
        username,
        password,
        timeout: Duration::from_secs(timeout as u64),
    }))
}

/// Requires a provider endpoint to use https, or http only on loopback.
fn validate_provider_endpoint(variable: &str, raw: &str) -> Result<String, ConfigError> {
    let url = raw.trim().to_string();
    let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|prefix| {
            url.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
        });
    if url.is_empty() || (!local && !url.starts_with("https://")) {
        return Err(ConfigError::Invalid(format!(
            "{variable} must use https:// (or http:// on loopback)"
        )));
    }
    Ok(url)
}

/// Validates a configured provider name against the provider registry,
/// mirroring the Go `validateEmbedProvider` / `validateGenProvider` helpers:
/// an empty name resolves to the registry default, and an unknown name is an
/// error naming the offending variable.
fn validate_provider(variable: &str, name: &str) -> Result<(), ConfigError> {
    let known = if variable == "EMBED_PROVIDER" {
        provider::has_embedder(name)
    } else {
        provider::has_generator(name)
    };

    if known {
        return Ok(());
    }

    Err(ConfigError::Invalid(format!(
        "{variable}=\"{name}\" is not a registered provider"
    )))
}

/// Decodes `SESSION_STORE_KEY` (base64 or base64url, 32 bytes).
///
/// A present but malformed key is a configuration error rather than a silent
/// fallback to unencrypted storage.
fn decode_session_store_key(raw: &str) -> Result<Vec<u8>, ConfigError> {
    let normalized = raw.trim().replace('+', "-").replace('/', "_");
    let normalized = normalized.trim_end_matches('=');
    let bytes = crate::oidc::base64url_decode(normalized).ok_or_else(|| {
        ConfigError::Invalid("SESSION_STORE_KEY must be base64 or base64url".to_string())
    })?;
    if bytes.len() != 32 {
        return Err(ConfigError::Invalid(
            "SESSION_STORE_KEY must decode to exactly 32 bytes".to_string(),
        ));
    }
    Ok(bytes)
}

/// Returns the trimmed value of `key` through `lookup`, or `None` when it is
/// absent or blank. An unset or blank variable counts as absent, so an empty
/// `.env` entry uses the default.
fn env_value<F>(lookup: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    let raw = lookup(key)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn positive_int<F>(get: &F, key: &str, fallback: i64) -> Result<i64, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = get(key) else {
        return Ok(fallback);
    };

    let value: i64 = raw
        .parse()
        .map_err(|_| ConfigError::Invalid(format!("{key} must be an integer, got \"{raw}\"")))?;

    if value <= 0 {
        return Err(ConfigError::Invalid(format!(
            "{key} must be greater than 0, got {value}"
        )));
    }

    Ok(value)
}

fn non_negative_int<F>(get: &F, key: &str, fallback: i64) -> Result<i64, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = get(key) else {
        return Ok(fallback);
    };

    let value: i64 = raw
        .parse()
        .map_err(|_| ConfigError::Invalid(format!("{key} must be an integer, got \"{raw}\"")))?;

    if value < 0 {
        return Err(ConfigError::Invalid(format!(
            "{key} must be 0 or greater, got {value}"
        )));
    }

    Ok(value)
}

fn unit_float<F>(get: &F, key: &str, fallback: f64) -> Result<f64, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = get(key) else {
        return Ok(fallback);
    };

    let value: f64 = raw
        .parse()
        .map_err(|_| ConfigError::Invalid(format!("{key} must be a number, got \"{raw}\"")))?;

    if value.is_nan() || !(0.0..=1.0).contains(&value) {
        return Err(ConfigError::Invalid(format!(
            "{key} must be between 0 and 1, got \"{raw}\""
        )));
    }

    Ok(value)
}

fn positive_duration<F>(get: &F, key: &str, fallback: Duration) -> Result<Duration, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = get(key) else {
        return Ok(fallback);
    };

    let value = parse_duration(&raw).ok_or_else(|| {
        ConfigError::Invalid(format!(
            "{key} must be a duration such as 30s or 5m, got \"{raw}\""
        ))
    })?;

    if value.is_zero() {
        return Err(ConfigError::Invalid(format!(
            "{key} must be greater than 0, got \"{raw}\""
        )));
    }

    Ok(value)
}

/// Parses a Go-style duration such as `30s`, `5m`, or `1h30m`. A leading sign
/// and a `0` (meaning disabled) are accepted, mirroring `time.ParseDuration`.
fn parse_duration(raw: &str) -> Option<Duration> {
    if raw == "0" {
        return Some(Duration::ZERO);
    }

    let (negative, body) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };

    if body.is_empty() {
        return None;
    }

    let mut total = Duration::ZERO;
    let mut number = String::new();
    let mut matched = false;

    let mut chars = body.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch.is_ascii_digit() || ch == '.' {
            number.push(ch);
            chars.next();
            continue;
        }

        if number.is_empty() {
            return None;
        }

        let mut unit = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() || c == '.' {
                break;
            }
            unit.push(c);
            chars.next();
        }
        let value: f64 = number.parse().ok()?;
        number.clear();

        let seconds = match unit.as_str() {
            "ns" => value / 1_000_000_000.0,
            "us" | "µs" => value / 1_000_000.0,
            "ms" => value / 1_000.0,
            "s" => value,
            "m" => value * 60.0,
            "h" => value * 3600.0,
            _ => return None,
        };

        // `Duration::mul_f64` panics on NaN or overflow; a malformed setting
        // must surface as `ConfigError`, so use the checked constructor.
        let scaled = Duration::try_from_secs_f64(seconds).ok()?;
        total = total.checked_add(scaled)?;
        matched = true;
    }

    if !number.is_empty() {
        return None;
    }

    if !matched {
        return None;
    }

    Some(if negative { Duration::ZERO } else { total })
}

fn bool_setting<F>(get: &F, key: &str, fallback: bool) -> Result<bool, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = get(key) else {
        return Ok(fallback);
    };

    match raw.as_str() {
        "1" | "t" | "T" | "true" | "TRUE" | "True" => Ok(true),
        "0" | "f" | "F" | "false" | "FALSE" | "False" => Ok(false),
        _ => Err(ConfigError::Invalid(format!(
            "{key} must be a boolean such as true or false, got \"{raw}\""
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn load(env: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::load_from(|key| map.get(key).cloned())
    }

    #[test]
    fn defaults_match_baseline() {
        let cfg = load(&[]).unwrap();
        assert_eq!(cfg.ollama_url, "http://localhost:11434");
        assert_eq!(cfg.embed_model, "nomic-embed-text");
        assert_eq!(cfg.chat_model, "qwen3.8:27b-mlx");
        assert_eq!(
            cfg.database_url,
            "postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable"
        );
        assert_eq!(cfg.embed_dim, 768);
        assert_eq!(cfg.embed_provider, "ollama");
        assert_eq!(cfg.gen_provider, "ollama");
        assert_eq!(cfg.chunk_size, 50);
        assert_eq!(cfg.chunk_overlap, 20);
        assert_eq!(cfg.top_k, 4);
        assert_eq!(cfg.final_k, 3);
        assert_eq!(cfg.expand_limit, 20);
        assert_eq!(cfg.context_budget, 0);
        assert_eq!(cfg.max_question_bytes, 0);
        assert_eq!(cfg.max_input_bytes, 0);
        assert_eq!(cfg.min_similarity, 0.6);
        assert!(cfg.rag_answerability_gate);
        assert!(!cfg.rag_citation_validation);
        assert_eq!(cfg.request_timeout, Duration::from_secs(300));
        assert!(cfg.query_rewrite);
        assert_eq!(cfg.language, "");
        assert_eq!(cfg.question, "");
        // No genealogy fallback: an absent key stays absent.
        assert_eq!(cfg.genealogy_database_url, None);
    }

    #[test]
    fn blank_value_falls_back_to_default() {
        let cfg = load(&[("CHUNK_SIZE", "   ")]).unwrap();
        assert_eq!(cfg.chunk_size, DEFAULT_CHUNK_SIZE);
    }

    #[test]
    fn overrides_are_parsed() {
        let cfg = load(&[
            ("CHUNK_SIZE", "100"),
            ("CHUNK_OVERLAP", "10"),
            ("MIN_SIMILARITY", "0.9"),
            ("QUERY_REWRITE", "false"),
            ("REQUEST_TIMEOUT", "30s"),
        ])
        .unwrap();
        assert_eq!(cfg.chunk_size, 100);
        assert_eq!(cfg.chunk_overlap, 10);
        assert_eq!(cfg.min_similarity, 0.9);
        assert!(!cfg.query_rewrite);
        assert_eq!(cfg.request_timeout, Duration::from_secs(30));
    }

    #[test]
    fn invalid_integer_names_variable() {
        let err = load(&[("CHUNK_SIZE", "abc")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid("CHUNK_SIZE must be an integer, got \"abc\"".into())
        );
    }

    #[test]
    fn non_positive_integer_is_rejected() {
        let err = load(&[("TOP_K", "0")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid("TOP_K must be greater than 0, got 0".into())
        );
    }

    #[test]
    fn zero_is_valid_for_non_negative_settings() {
        let cfg = load(&[("CONTEXT_BUDGET", "0")]).unwrap();
        assert_eq!(cfg.context_budget, 0);
    }

    #[test]
    fn negative_non_negative_setting_is_rejected() {
        let err = load(&[("CONTEXT_BUDGET", "-1")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid("CONTEXT_BUDGET must be 0 or greater, got -1".into())
        );
    }

    #[test]
    fn similarity_out_of_range_is_rejected() {
        let err = load(&[("MIN_SIMILARITY", "1.5")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid("MIN_SIMILARITY must be between 0 and 1, got \"1.5\"".into())
        );
    }

    #[test]
    fn overlap_not_smaller_than_size_is_rejected() {
        let err = load(&[("CHUNK_SIZE", "10"), ("CHUNK_OVERLAP", "10")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid("CHUNK_OVERLAP (10) must be smaller than CHUNK_SIZE (10)".into())
        );
    }

    #[test]
    fn invalid_bool_is_rejected() {
        let err = load(&[("QUERY_REWRITE", "maybe")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid(
                "QUERY_REWRITE must be a boolean such as true or false, got \"maybe\"".into()
            )
        );
    }

    #[test]
    fn unknown_provider_is_rejected() {
        let err = load(&[("EMBED_PROVIDER", "bogus")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid("EMBED_PROVIDER=\"bogus\" is not a registered provider".into())
        );
    }

    #[test]
    fn unknown_gen_provider_is_rejected() {
        let err = load(&[("GEN_PROVIDER", "bogus")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid("GEN_PROVIDER=\"bogus\" is not a registered provider".into())
        );
    }

    #[test]
    fn registered_providers_resolve() {
        let cfg = load(&[("EMBED_PROVIDER", "ollama"), ("GEN_PROVIDER", "ollama")]).unwrap();
        assert_eq!(cfg.embed_provider, "ollama");
        assert_eq!(cfg.gen_provider, "ollama");
    }

    #[test]
    fn openai_compatible_generation_needs_a_model() {
        let err = load(&[("GEN_PROVIDER", "openai_compatible")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid(
                "OPENAI_COMPATIBLE_MODEL is required when GEN_PROVIDER=openai_compatible".into()
            )
        );
    }

    #[test]
    fn openai_compatible_settings_load_with_defaults() {
        let cfg = load(&[
            ("GEN_PROVIDER", "openai_compatible"),
            ("OPENAI_COMPATIBLE_MODEL", " Qwen3-4B-4bit "),
            ("OPENAI_COMPATIBLE_API_KEY", " "),
        ])
        .unwrap();
        assert_eq!(cfg.gen_provider, "openai_compatible");
        assert_eq!(cfg.embed_provider, "ollama");
        assert_eq!(cfg.openai_compatible_model, "Qwen3-4B-4bit");
        assert_eq!(cfg.openai_compatible_url, "http://127.0.0.1:8000");
        assert_eq!(cfg.openai_compatible_api_key, None);

        let cfg = load(&[
            ("GEN_PROVIDER", "openai_compatible"),
            ("OPENAI_COMPATIBLE_MODEL", "m"),
            ("OPENAI_COMPATIBLE_BASE_URL", "http://localhost:9000/v1"),
            ("OPENAI_COMPATIBLE_API_KEY", "local"),
        ])
        .unwrap();
        assert_eq!(cfg.openai_compatible_url, "http://localhost:9000/v1");
        assert_eq!(cfg.openai_compatible_api_key.as_deref(), Some("local"));
    }

    #[test]
    fn the_ollama_default_does_not_need_an_openai_model() {
        let cfg = load(&[]).unwrap();
        assert_eq!(cfg.gen_provider, "ollama");
        assert_eq!(cfg.openai_compatible_model, "");
    }

    #[test]
    fn invalid_duration_is_rejected() {
        let err = load(&[("REQUEST_TIMEOUT", "soon")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid(
                "REQUEST_TIMEOUT must be a duration such as 30s or 5m, got \"soon\"".into()
            )
        );
    }

    #[test]
    fn duration_supports_compound_units() {
        let cfg = load(&[("REQUEST_TIMEOUT", "1h30m")]).unwrap();
        assert_eq!(cfg.request_timeout, Duration::from_secs(5400));
    }

    #[test]
    fn overflowing_duration_is_rejected_without_panicking() {
        let huge = format!("{}h", "9".repeat(30));
        let err = load(&[("REQUEST_TIMEOUT", &huge)]).unwrap_err();
        assert!(
            matches!(&err, ConfigError::Invalid(message) if message.contains("REQUEST_TIMEOUT")),
            "an out-of-range duration must be a config error, got {err:?}"
        );
    }

    #[test]
    fn genealogy_key_is_read_when_set() {
        let cfg = load(&[(
            "GENEALOGY_DATABASE_URL",
            "postgres://u:p@127.0.0.1:5434/genealogy?sslmode=disable",
        )])
        .unwrap();
        assert_eq!(
            cfg.genealogy_database_url,
            Some("postgres://u:p@127.0.0.1:5434/genealogy?sslmode=disable".to_string())
        );
        assert_eq!(
            cfg.genealogy_database_url().unwrap(),
            "postgres://u:p@127.0.0.1:5434/genealogy?sslmode=disable"
        );
    }

    #[test]
    fn absent_genealogy_key_is_an_error_naming_the_variable() {
        let cfg = load(&[]).unwrap();
        let err = cfg.genealogy_database_url().unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid(
                "GENEALOGY_DATABASE_URL must be set to an explicit genealogy database DSN".into()
            )
        );
    }

    #[test]
    fn blank_genealogy_key_is_treated_as_absent_with_no_rag_fallback() {
        let cfg = load(&[("GENEALOGY_DATABASE_URL", "   ")]).unwrap();
        assert_eq!(cfg.genealogy_database_url, None);
        // The RAG default must never be used as the genealogy DSN.
        let genealogy = cfg.genealogy_database_url.clone().unwrap_or_default();
        assert_ne!(cfg.database_url, genealogy);
        let err = cfg.genealogy_database_url().unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(ref m) if m.contains("GENEALOGY_DATABASE_URL")));
    }

    #[test]
    fn password_reset_is_off_by_default_and_rejects_plain_remote_http() {
        let cfg = load(&[]).unwrap();
        assert_eq!(cfg.password_reset, None);
        assert_eq!(cfg.password_reset_cooldown, Duration::from_secs(300));

        for url in [
            "http://reset.example/hook",
            "http://localhost.evil.example/x",
            "ftp://x",
        ] {
            assert!(load(&[("PASSWORD_RESET_URL", url)]).is_err(), "{url}");
        }
        for url in [
            "https://reset.example/hook",
            "http://127.0.0.1:9100/reset",
            "http://localhost/r",
        ] {
            let cfg = load(&[("PASSWORD_RESET_URL", url), ("PASSWORD_RESET_TOKEN", "t")]).unwrap();
            let provider = cfg.password_reset.unwrap();
            assert_eq!(
                (provider.url.as_str(), provider.bearer.as_deref()),
                (url, Some("t"))
            );
        }
    }

    #[test]
    fn the_http_mail_adapter_requires_a_valid_endpoint() {
        // Unknown names and plain remote HTTP are configuration errors.
        assert!(load(&[("ONBOARDING_MAIL_ADAPTER", "unknown")]).is_err());
        assert!(load(&[
            ("ONBOARDING_MAIL_ADAPTER", "http"),
            ("ONBOARDING_MAIL_URL", "http://mail.example/send"),
        ])
        .is_err());

        // https, or loopback http, is accepted with an optional bearer.
        let cfg = load(&[
            ("ONBOARDING_MAIL_ADAPTER", "http"),
            ("ONBOARDING_MAIL_URL", "https://mail.example/send"),
            ("ONBOARDING_MAIL_BEARER", "synthetic-bearer"),
        ])
        .unwrap();
        let adapter = cfg.mail_adapter();
        assert!(adapter.is_configured());
        assert_eq!(adapter.name(), "http");

        // The name without a URL still fails closed.
        let cfg = load(&[("ONBOARDING_MAIL_ADAPTER", "http")]).unwrap();
        assert!(!cfg.mail_adapter().is_configured());
    }

    #[test]
    fn the_smtp_adapter_requires_valid_settings() {
        let base = [
            ("ONBOARDING_MAIL_ADAPTER", "smtp"),
            ("ONBOARDING_SMTP_HOST", "127.0.0.1"),
            ("ONBOARDING_SMTP_PORT", "1025"),
            ("ONBOARDING_SMTP_TLS", "none"),
            ("ONBOARDING_SMTP_FROM", "tran@example.test"),
        ];

        let cfg = load(&base).unwrap();
        let adapter = cfg.mail_adapter();
        assert!(adapter.is_configured());
        assert_eq!(adapter.name(), "smtp");
        let smtp = cfg.onboarding_smtp.as_ref().expect("smtp settings");
        assert_eq!(smtp.host, "127.0.0.1");
        assert_eq!(smtp.port, 1025);
        assert_eq!(smtp.timeout, Duration::from_secs(10));
        assert!(smtp.username.is_none());

        // A missing host or sender is a configuration error, not a fallback.
        for missing in ["ONBOARDING_SMTP_HOST", "ONBOARDING_SMTP_FROM"] {
            let vars: Vec<(&str, &str)> = base
                .iter()
                .copied()
                .filter(|(key, _)| *key != missing)
                .collect();
            assert!(load(&vars).is_err(), "{missing} must be required");
        }

        // Plaintext is loopback-only unless the operator explicitly accepts a
        // local container hostname.
        let mut container_host = base.to_vec();
        container_host[1] = ("ONBOARDING_SMTP_HOST", "mailpit");
        assert!(load(&container_host).is_err());
        container_host.push(("ONBOARDING_SMTP_ALLOW_PLAINTEXT", "true"));
        let cfg = load(&container_host).unwrap();
        assert_eq!(cfg.onboarding_smtp.as_ref().unwrap().host, "mailpit");

        // An unknown TLS mode fails closed instead of negotiating one.
        let mut bad_tls = base.to_vec();
        bad_tls[3] = ("ONBOARDING_SMTP_TLS", "opportunistic");
        assert!(load(&bad_tls).is_err());

        // A username without a password (or the reverse) is refused.
        let mut half_auth = base.to_vec();
        half_auth.push(("ONBOARDING_SMTP_USERNAME", "synthetic-user"));
        assert!(load(&half_auth).is_err());

        // A non-smtp adapter ignores a stale SMTP block.
        let cfg = load(&[("ONBOARDING_MAIL_ADAPTER", "fake")]).unwrap();
        assert!(cfg.onboarding_smtp.is_none());
        assert_eq!(cfg.mail_adapter().name(), "fake");
    }

    fn keycloak_env() -> Vec<(&'static str, &'static str)> {
        vec![
            ("ONBOARDING_KEYCLOAK_LOGINS", "true"),
            ("KEYCLOAK_URL", "http://127.0.0.1:8180"),
            ("KEYCLOAK_REALM", "tran"),
            ("KEYCLOAK_LOGIN_CLIENT_ID", "tran-invitations"),
            ("KEYCLOAK_LOGIN_CLIENT_SECRET", "synthetic-secret"),
            ("KEYCLOAK_ACTIONS_CLIENT_ID", "tran-web"),
            ("AUTH_ISSUER", "http://127.0.0.1:8180/realms/tran"),
            (
                "ONBOARDING_ACTIVATION_URL",
                "http://127.0.0.1:3000/activate",
            ),
            ("ONBOARDING_MAIL_ADAPTER", "fake"),
        ]
    }

    #[test]
    fn keycloak_logins_are_off_by_default() {
        let cfg = load(&[]).expect("defaults load");
        assert!(cfg.onboarding_keycloak_logins.is_none());
        assert!(!cfg.mail_adapter().provisions_logins());
    }

    #[test]
    fn keycloak_logins_wrap_the_mail_adapter() {
        let cfg = load(&keycloak_env()).expect("keycloak settings load");
        let settings = cfg.onboarding_keycloak_logins.as_ref().expect("settings");
        assert_eq!(settings.actions_client_id, "tran-web");
        assert_eq!(
            settings.actions_lifespan,
            Duration::from_secs(DEFAULT_KEYCLOAK_ACTIONS_LIFESPAN_SECS as u64)
        );
        let adapter = cfg.mail_adapter();
        assert_eq!(adapter.name(), "keycloak");
        assert!(adapter.is_configured());
        assert!(adapter.provisions_logins());
        assert!(!format!("{settings:?}").contains("synthetic-secret"));
    }

    #[test]
    fn keycloak_logins_require_every_setting() {
        for missing in [
            "KEYCLOAK_URL",
            "KEYCLOAK_REALM",
            "KEYCLOAK_LOGIN_CLIENT_ID",
            "KEYCLOAK_LOGIN_CLIENT_SECRET",
            "KEYCLOAK_ACTIONS_CLIENT_ID",
        ] {
            let env: Vec<_> = keycloak_env()
                .into_iter()
                .filter(|(key, _)| *key != missing)
                .collect();
            let err = load(&env).expect_err(missing);
            assert!(err.to_string().contains(missing), "{err}");
        }
    }

    #[test]
    fn keycloak_logins_refuse_another_realm_or_a_relative_link() {
        let mut env = keycloak_env();
        env.retain(|(key, _)| *key != "AUTH_ISSUER");
        env.push(("AUTH_ISSUER", "http://127.0.0.1:8180/realms/other"));
        assert!(load(&env).unwrap_err().to_string().contains("AUTH_ISSUER"));

        let mut env = keycloak_env();
        env.retain(|(key, _)| *key != "ONBOARDING_ACTIVATION_URL");
        env.push(("ONBOARDING_ACTIVATION_URL", "/activate"));
        assert!(load(&env)
            .unwrap_err()
            .to_string()
            .contains("ONBOARDING_ACTIVATION_URL"));

        let mut env = keycloak_env();
        env.retain(|(key, _)| *key != "KEYCLOAK_URL");
        env.push(("KEYCLOAK_URL", "http://kc.example.test"));
        assert!(load(&env).unwrap_err().to_string().contains("KEYCLOAK_URL"));

        let mut env = keycloak_env();
        env.retain(|(key, _)| *key != "KEYCLOAK_URL");
        env.push(("KEYCLOAK_URL", "http://127.0.0.1:8180/realms/tran"));
        assert!(load(&env).unwrap_err().to_string().contains("realm URL"));
    }
}
