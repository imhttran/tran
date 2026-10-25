//! Genealogy domain boundary.
//!
//! The genealogy schema lives in a separate PostgreSQL database selected by
//! `GENEALOGY_DATABASE_URL`. This module owns the explicit destination guard and
//! the migration runner. It never falls back to the RAG `DATABASE_URL`.

pub mod appointments;
pub mod authority;
pub mod branch_review;
pub mod current;
pub mod dates;
pub mod document_card;
pub mod document_extraction;
pub mod eligibility;
pub mod label;
pub mod migrate;
pub mod parser;
pub mod person_search;
pub mod query;
pub mod reimport;
pub mod relationships;
pub mod report;
pub mod review;
pub mod tree;
pub mod validation;

use std::str::FromStr;

use postgres::config::{Config as PgConfig, Host};
use thiserror::Error;

use crate::config::{Config, ConfigError};
use crate::storage_pg::PgStoreError;

/// Errors raised by the genealogy domain.
#[derive(Debug, Error)]
pub enum GenealogyError {
    /// The genealogy database setting is absent or invalid.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The destination guard rejected the configured database.
    #[error("genealogy database guard: {0}")]
    Guard(String),
    /// The shared PostgreSQL pool failed.
    #[error(transparent)]
    Store(#[from] PgStoreError),
    /// A genealogy query failed.
    #[error("genealogy query failed: {0}")]
    Query(#[from] postgres::Error),
    /// A report or export could not be encoded.
    #[error("genealogy encoding failed: {0}")]
    Encode(#[from] serde_json::Error),
    /// A report, migration, or export could not be read or written.
    #[error("genealogy I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// A review operation failed.
    #[error("genealogy review failed: {0}")]
    Review(String),
    /// A domain rule rejected the requested operation.
    #[error("genealogy domain error: {0}")]
    Domain(String),
    /// A migration file could not be applied.
    #[error("genealogy migration failed: {0}")]
    Migration(String),
}

/// Resolves and guards the explicitly configured genealogy database DSN.
///
/// The guard is intentionally strict:
/// - the DSN must be set (there is no RAG fallback);
/// - it must point at a local database;
/// - it must not resolve to the same database as `DATABASE_URL`.
pub fn resolve_genealogy_dsn(config: &Config) -> Result<String, GenealogyError> {
    let dsn = config.genealogy_database_url()?.trim().to_string();
    let genealogy = parse_dsn(&dsn)?;
    let rag = parse_dsn(&config.database_url)?;

    let genealogy_db = genealogy
        .get_dbname()
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| {
            GenealogyError::Guard("GENEALOGY_DATABASE_URL has no database name".into())
        })?;

    if !is_local(&genealogy) {
        return Err(GenealogyError::Guard(
            "GENEALOGY_DATABASE_URL must target a local database".into(),
        ));
    }

    if same_database(&genealogy, &rag) {
        return Err(GenealogyError::Guard(format!(
            "GENEALOGY_DATABASE_URL must not reference the RAG database ({genealogy_db})"
        )));
    }

    Ok(dsn)
}

fn parse_dsn(dsn: &str) -> Result<PgConfig, GenealogyError> {
    PgConfig::from_str(dsn)
        .map_err(|err| GenealogyError::Guard(format!("invalid database DSN: {err}")))
}

fn is_local(config: &PgConfig) -> bool {
    let hosts = config.get_hosts();
    if hosts.is_empty() {
        return true; // libpq's default is a local Unix socket / localhost.
    }
    hosts.iter().all(|host| match host {
        Host::Tcp(name) => matches!(
            name.to_ascii_lowercase().as_str(),
            "localhost" | "127.0.0.1" | "::1" | "[::1]"
        ),
        #[cfg(unix)]
        Host::Unix(_) => true,
    })
}

fn same_database(left: &PgConfig, right: &PgConfig) -> bool {
    if left.get_dbname() != right.get_dbname() {
        return false;
    }
    let left_hosts = normalized_hosts(left);
    let right_hosts = normalized_hosts(right);
    let left_ports = normalized_ports(left);
    let right_ports = normalized_ports(right);
    left_hosts.iter().any(|host| right_hosts.contains(host))
        && left_ports.iter().any(|port| right_ports.contains(port))
}

fn normalized_hosts(config: &PgConfig) -> Vec<String> {
    let hosts = config.get_hosts();
    if hosts.is_empty() {
        return vec!["loopback".to_string()];
    }
    hosts
        .iter()
        .map(|host| match host {
            Host::Tcp(name) => canonical_host(name),
            #[cfg(unix)]
            Host::Unix(_) => "loopback".to_string(),
        })
        .collect()
}

/// Canonicalizes every loopback spelling to one value, so `localhost`,
/// `127.0.0.1`, `::1`, a default host, and a local Unix socket cannot disguise
/// the same database as a different destination.
fn canonical_host(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "localhost" | "127.0.0.1" | "::1" | "[::1]" => "loopback".to_string(),
        other => other.to_string(),
    }
}

fn normalized_ports(config: &PgConfig) -> Vec<u16> {
    let ports = config.get_ports();
    if ports.is_empty() {
        return vec![5432];
    }
    ports.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(genealogy: Option<&str>, rag: Option<&str>) -> Config {
        Config::load_from(|key| match key {
            "GENEALOGY_DATABASE_URL" => genealogy.map(str::to_string),
            "DATABASE_URL" => rag.map(str::to_string),
            _ => None,
        })
        .unwrap()
    }

    #[test]
    fn missing_genealogy_dsn_is_a_config_error() {
        let cfg = config(None, Some("postgres://rag:rag@127.0.0.1:5434/rag"));
        assert!(matches!(
            resolve_genealogy_dsn(&cfg),
            Err(GenealogyError::Config(_))
        ));
    }

    #[test]
    fn same_dsn_as_rag_is_rejected() {
        let dsn = "postgres://rag:rag@127.0.0.1:5434/rag";
        let cfg = config(Some(dsn), Some(dsn));
        assert!(matches!(
            resolve_genealogy_dsn(&cfg),
            Err(GenealogyError::Guard(message)) if message.contains("RAG database")
        ));
    }

    #[test]
    fn same_database_with_different_query_is_rejected() {
        let cfg = config(
            Some("postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable"),
            Some("postgres://rag:rag@127.0.0.1:5434/rag?sslmode=require"),
        );
        assert!(matches!(
            resolve_genealogy_dsn(&cfg),
            Err(GenealogyError::Guard(_))
        ));
    }

    #[test]
    fn nonlocal_dsn_is_rejected() {
        let cfg = config(
            Some("postgres://rag:rag@db.example.com:5434/genealogy"),
            Some("postgres://rag:rag@127.0.0.1:5434/rag"),
        );
        assert!(matches!(
            resolve_genealogy_dsn(&cfg),
            Err(GenealogyError::Guard(message)) if message.contains("local")
        ));
    }

    #[test]
    fn loopback_alias_cannot_bypass_the_rag_database_guard() {
        let cfg = config(
            Some("postgres://rag:rag@127.0.0.1:5434/rag"),
            Some("postgres://rag:rag@localhost:5434/rag"),
        );
        assert!(matches!(
            resolve_genealogy_dsn(&cfg),
            Err(GenealogyError::Guard(_))
        ));
    }

    #[test]
    fn default_host_cannot_bypass_the_rag_database_guard() {
        let cfg = config(
            Some("postgres://rag:rag@localhost:5434/rag"),
            Some("postgres://rag:rag@127.0.0.1:5434/rag"),
        );
        assert!(matches!(
            resolve_genealogy_dsn(&cfg),
            Err(GenealogyError::Guard(_))
        ));
    }

    #[test]
    fn local_separate_database_is_accepted() {
        let cfg = config(
            Some("postgres://rag:rag@127.0.0.1:5434/genealogy_test"),
            Some("postgres://rag:rag@127.0.0.1:5434/rag"),
        );
        assert_eq!(
            resolve_genealogy_dsn(&cfg).unwrap(),
            "postgres://rag:rag@127.0.0.1:5434/genealogy_test"
        );
    }
}
