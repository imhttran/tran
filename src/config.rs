//! Configuration parsing, defaults, and validation.
//!
//! Ported from the Go baseline in `internal/config/config.go`. Every setting can
//! be overridden with an environment variable. A setting that is absent (unset
//! or blank) falls back to its default. A setting that is present but invalid is
//! an error, never a silent fallback: a typo in `CHUNK_SIZE` stops the command
//! with a message naming the variable instead of quietly using the default.

use std::env;
use std::path::PathBuf;
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
pub const DEFAULT_CHUNK_SIZE: i64 = 50;
pub const DEFAULT_CHUNK_OVERLAP: i64 = 20;
pub const DEFAULT_MIN_SIMILARITY: f64 = 0.6;
pub const DEFAULT_MEDIA_ROOT: &str = "./data/media";
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
pub const DEFAULT_AUTH_RATE_LIMIT: i64 = 60;
pub const DEFAULT_PASSWORD_RESET_COOLDOWN_SECS: i64 = 300;

/// Bulk invitation delivery adapter: `unconfigured` (default) or `fake`.
pub const DEFAULT_ONBOARDING_MAIL_ADAPTER: &str = "unconfigured";
/// Base URL used to build the activation link handed to the adapter.
pub const DEFAULT_ONBOARDING_ACTIVATION_URL: &str = "/activate";
/// Minimum delay between bulk invitation sends, in milliseconds.
pub const DEFAULT_ONBOARDING_SEND_INTERVAL_MS: i64 = 0;

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
    /// Base URL for the one-time activation link.
    pub onboarding_activation_url: String,
    /// Delay between bulk sends, in milliseconds.
    pub onboarding_send_interval_ms: i64,
    /// Private filesystem root for owner media. Never served as a static tree.
    pub media_root: PathBuf,
    /// Provider endpoint that sends password-reset email. `None` when
    /// `PASSWORD_RESET_URL` is unset: reset requests then fail closed.
    pub password_reset: Option<ResetProvider>,
    /// Minimum time between accepted reset requests for one account.
    pub password_reset_cooldown: Duration,
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
            auth_audience: get("AUTH_AUDIENCE").unwrap_or_default(),
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
            onboarding_mail_adapter: onboarding_mail_adapter(&get)?,
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
            password_reset,
            password_reset_cooldown: Duration::from_secs(non_negative_int(
                &get,
                "PASSWORD_RESET_COOLDOWN_SECS",
                DEFAULT_PASSWORD_RESET_COOLDOWN_SECS,
            )? as u64),
        })
    }

    /// Builds the configured bulk delivery adapter.
    ///
    /// Only the synthetic `fake` adapter is implemented. A production operator
    /// must supply a real provider adapter and approve its configuration before
    /// real invitations can be sent.
    pub fn mail_adapter(&self) -> Box<dyn crate::onboarding::MailAdapter> {
        match self.onboarding_mail_adapter.as_str() {
            "fake" => Box::new(crate::onboarding::FakeAdapter::accepting()),
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

/// Validates the bulk delivery adapter name. Only the fail-closed default and
/// the synthetic test adapter exist today; an unknown name is an error rather
/// than a silent fallback.
fn onboarding_mail_adapter<F>(get: &F) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let value = get("ONBOARDING_MAIL_ADAPTER")
        .unwrap_or_else(|| DEFAULT_ONBOARDING_MAIL_ADAPTER.to_string());
    match value.trim() {
        "unconfigured" | "fake" => Ok(value.trim().to_string()),
        other => Err(ConfigError::Invalid(format!(
            "ONBOARDING_MAIL_ADAPTER must be unconfigured or fake, got {other:?}"
        ))),
    }
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
}
