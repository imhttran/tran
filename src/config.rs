//! Configuration parsing, defaults, and validation.
//!
//! Ported from the Go baseline in `internal/config/config.go`. Every setting can
//! be overridden with an environment variable. A setting that is absent (unset
//! or blank) falls back to its default. A setting that is present but invalid is
//! an error, never a silent fallback: a typo in `CHUNK_SIZE` stops the command
//! with a message naming the variable instead of quietly using the default.

use std::env;
use std::time::Duration;

use thiserror::Error;

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
pub const DEFAULT_EMBED_WORKERS: i64 = 4;
pub const DEFAULT_EMBED_RETRIES: i64 = 3;
pub const DEFAULT_MIN_SIMILARITY: f64 = 0.6;
pub const DEFAULT_LLM_RERANK: bool = false;
pub const DEFAULT_LEXICAL_RERANK: bool = false;
pub const DEFAULT_RERANK_TIMEOUT: Duration = Duration::from_secs(0);
pub const DEFAULT_ANSWERABILITY_GATE: bool = false;
pub const DEFAULT_FACT_JUDGE: bool = false;
pub const DEFAULT_REWRITE_ONLY: bool = false;
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
pub const OUTPUT_HUMAN: &str = "human";
pub const OUTPUT_JSON: &str = "json";
pub const DEFAULT_OBSERVABILITY_FORMAT: &str = OUTPUT_HUMAN;

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
    pub embed_workers: i64,
    pub embed_retries: i64,
    pub min_similarity: f64,
    pub lexical_rerank: bool,
    pub llm_rerank: bool,
    pub rag_llm_rerank: bool,
    pub rerank_timeout: Duration,
    pub answerability_gate: bool,
    pub fact_judge: bool,
    pub rewrite_only: bool,
    pub rag_answerability_gate: bool,
    pub rag_citation_validation: bool,
    pub request_timeout: Duration,
    pub query_rewrite: bool,
    pub observability_format: String,
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

        Ok(Config {
            ollama_url: get("OLLAMA_URL").unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_string()),
            embed_model: get("OLLAMA_EMBED_MODEL")
                .unwrap_or_else(|| DEFAULT_EMBED_MODEL.to_string()),
            embed_dim: positive_int(&get, "EMBED_DIM", DEFAULT_EMBED_DIM)?,
            chat_model: get("OLLAMA_CHAT_MODEL").unwrap_or_else(|| DEFAULT_CHAT_MODEL.to_string()),
            database_url: get("DATABASE_URL").unwrap_or_else(|| DEFAULT_DATABASE_URL.to_string()),
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
            embed_workers: positive_int(&get, "EMBED_WORKERS", DEFAULT_EMBED_WORKERS)?,
            embed_retries: non_negative_int(&get, "EMBED_RETRIES", DEFAULT_EMBED_RETRIES)?,
            min_similarity: unit_float(&get, "MIN_SIMILARITY", DEFAULT_MIN_SIMILARITY)?,
            lexical_rerank: bool_setting(&get, "EVAL_LEXICAL_RERANK", DEFAULT_LEXICAL_RERANK)?,
            llm_rerank: bool_setting(&get, "EVAL_LLM_RERANK", DEFAULT_LLM_RERANK)?,
            rag_llm_rerank: bool_setting(&get, "RAG_LLM_RERANK", false)?,
            rerank_timeout: non_negative_duration(&get, "RERANK_TIMEOUT", DEFAULT_RERANK_TIMEOUT)?,
            answerability_gate: bool_setting(
                &get,
                "EVAL_ANSWERABILITY_GATE",
                DEFAULT_ANSWERABILITY_GATE,
            )?,
            fact_judge: bool_setting(&get, "EVAL_FACT_JUDGE", DEFAULT_FACT_JUDGE)?,
            rewrite_only: bool_setting(&get, "EVAL_REWRITE_ONLY", DEFAULT_REWRITE_ONLY)?,
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
            observability_format: output_format(
                &get,
                "OBSERVABILITY_FORMAT",
                DEFAULT_OBSERVABILITY_FORMAT,
            )?,
        })
    }

    /// Reports whether the configured output format selects the opt-in JSON
    /// structured record instead of the human-readable stream.
    pub fn observability_json(&self) -> bool {
        self.observability_format == OUTPUT_JSON
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

fn non_negative_duration<F>(get: &F, key: &str, fallback: Duration) -> Result<Duration, ConfigError>
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

        let unit_duration = match unit.as_str() {
            "ns" => Duration::from_nanos(1),
            "us" | "µs" => Duration::from_micros(1),
            "ms" => Duration::from_millis(1),
            "s" => Duration::from_secs(1),
            "m" => Duration::from_secs(60),
            "h" => Duration::from_secs(60 * 60),
            _ => return None,
        };

        total += unit_duration.mul_f64(value);
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

/// Reads the observability output-format setting. It accepts only the
/// enumerated values (`human`, `json`); a present but unknown value is an error
/// naming the variable, never a silent fallback to the default.
fn output_format<F>(get: &F, key: &str, fallback: &str) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = get(key) else {
        return Ok(fallback.to_string());
    };

    match raw.to_lowercase().as_str() {
        OUTPUT_HUMAN => Ok(OUTPUT_HUMAN.to_string()),
        OUTPUT_JSON => Ok(OUTPUT_JSON.to_string()),
        _ => Err(ConfigError::Invalid(format!(
            "{key} must be \"{OUTPUT_HUMAN}\" or \"{OUTPUT_JSON}\", got \"{raw}\""
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
        assert_eq!(cfg.embed_workers, 4);
        assert_eq!(cfg.embed_retries, 3);
        assert_eq!(cfg.min_similarity, 0.6);
        assert!(!cfg.lexical_rerank);
        assert!(!cfg.llm_rerank);
        assert!(!cfg.rag_llm_rerank);
        assert_eq!(cfg.rerank_timeout, Duration::from_secs(0));
        assert!(!cfg.answerability_gate);
        assert!(!cfg.fact_judge);
        assert!(!cfg.rewrite_only);
        assert!(cfg.rag_answerability_gate);
        assert!(!cfg.rag_citation_validation);
        assert_eq!(cfg.request_timeout, Duration::from_secs(300));
        assert!(cfg.query_rewrite);
        assert_eq!(cfg.observability_format, "human");
        assert_eq!(cfg.language, "");
        assert_eq!(cfg.question, "");
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
            ("OBSERVABILITY_FORMAT", "JSON"),
        ])
        .unwrap();
        assert_eq!(cfg.chunk_size, 100);
        assert_eq!(cfg.chunk_overlap, 10);
        assert_eq!(cfg.min_similarity, 0.9);
        assert!(!cfg.query_rewrite);
        assert_eq!(cfg.request_timeout, Duration::from_secs(30));
        assert!(cfg.observability_json());
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
        let err = load(&[("EMBED_RETRIES", "-1")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid("EMBED_RETRIES must be 0 or greater, got -1".into())
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
    fn invalid_output_format_is_rejected() {
        let err = load(&[("OBSERVABILITY_FORMAT", "xml")]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid(
                "OBSERVABILITY_FORMAT must be \"human\" or \"json\", got \"xml\"".into()
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
}
