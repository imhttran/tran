//! Per-suite isolation helper for genealogy PostgreSQL integration suites.
//!
//! Each suite that mutates the genealogy schema gets its own disposable target
//! derived from the base DSN in `GENEALOGY_DATABASE_URL`. Every target is a
//! uniquely named database created with `CREATE DATABASE` from the base DSN.
//! Cleanup drops only the database this run created - never a shared table,
//! and never a database that was already present before the run.
//!
//! The helper previously offered a per-suite-schema fallback when
//! `CREATE DATABASE` was unavailable. That fallback is removed: several
//! genealogy migrations (notably `0010_onboarding_foreign_keys.sql` and
//! `0011_onboarding_foreign_key_reconciliation.sql`) look up `pg_constraint`
//! by constraint name, which is database-global and therefore polluted by
//! any sibling schema sharing the base database. Silently running those
//! migrations under a schema fallback produced false passes and false fails.
//! The helper now requires database isolation and fails loudly when the role
//! cannot create a database, so a suite never runs against a shared target.
//!
//! The helper reuses the existing `postgres` crate connection style and adds
//! no new crate or test framework.

#![allow(dead_code)]

use std::fs;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};

/// Sorted list of every genealogy migration the helper applies to a fresh
/// target. It matches the files present in `genealogy/migrations/`.
pub const GENEALOGY_MIGRATIONS: &[&str] = &[
    "genealogy/migrations/0001_genealogy_schema_version.sql",
    "genealogy/migrations/0002_catalog_keys.sql",
    "genealogy/migrations/0003_assertions_evidence_and_links.sql",
    "genealogy/migrations/0004_review_workflow.sql",
    "genealogy/migrations/0005_source_kind.sql",
    "genealogy/migrations/0006_head_of_family.sql",
    "genealogy/migrations/0007_onboarding.sql",
    "genealogy/migrations/0008_onboarding_identity.sql",
    "genealogy/migrations/0009_onboarding_link_audit.sql",
    "genealogy/migrations/0010_onboarding_foreign_keys.sql",
    "genealogy/migrations/0011_onboarding_foreign_key_reconciliation.sql",
    "genealogy/migrations/0012_profile_revisions.sql",
    "genealogy/migrations/0013_profile_revision_actor_scope.sql",
    "genealogy/migrations/0014_profile_revision_draft_withdrawal.sql",
    "genealogy/migrations/0015_profile_media.sql",
    "genealogy/migrations/0016_profile_media_withdrawal.sql",
    "genealogy/migrations/0017_profile_media_withdrawal_audit.sql",
    "genealogy/migrations/0018_onboarding_batch.sql",
    "genealogy/migrations/0019_branch_reviewer_grants.sql",
    "genealogy/migrations/0020_branch_review_proposals.sql",
    "genealogy/migrations/0021_profile_field_restriction.sql",
    "genealogy/migrations/0022_profile_revision_restriction_generation.sql",
    "genealogy/migrations/0023_document_extraction.sql",
    "genealogy/migrations/0024_document_evidence_provenance.sql",
];

static RUN_TOKEN: AtomicU64 = AtomicU64::new(0);

/// PostgreSQL limits identifiers to 63 bytes. `pg_database.datname` is a `name`
/// column, so a longer value is not truncated for a parameterised comparison:
/// the server raises SQLSTATE 42622 (`identifier too long`).
const MAX_IDENTIFIER_BYTES: usize = 63;

/// The isolation mode the helper actually selected for a run.
///
/// Database mode is the only supported mode today. The enum is kept so
/// callers and tests can keep matching on it without churn if a safe schema
/// fallback is reintroduced later.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// A per-suite disposable database was created and is owned by this run.
    Database,
}

/// An isolated genealogy target owned by this run.
pub struct IsolatedTarget {
    /// A DSN that connects to the isolated database owned by this run.
    pub dsn: String,
    pub mode: Mode,
    /// The owned database name.
    pub owned_name: String,
    /// The maintenance-database DSN that actually created this target. Cleanup
    /// and `Drop` use it verbatim instead of re-deriving one from the target DSN.
    admin_dsn: String,
    /// Set once the owned database has been dropped, so `cleanup()` and `Drop`
    /// never issue the drop twice.
    cleaned: bool,
}

impl IsolatedTarget {
    /// Open a connection to the isolated target.
    pub fn connect(&self) -> Client {
        Client::connect(&self.dsn, NoTls).expect("connect to isolated target")
    }

    /// Apply the full sorted genealogy migration set to this target.
    pub fn apply_migrations(&self) {
        self.apply_migration_files(GENEALOGY_MIGRATIONS);
    }

    /// Apply an explicit subset of migration files to this target. The schema
    /// contract suite keeps its original 0001-0005 scope; every suite still gets
    /// its own database.
    pub fn apply_migration_files(&self, paths: &[&str]) {
        let mut client = self.connect();
        for path in paths {
            let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
            client
                .batch_execute(&sql)
                .unwrap_or_else(|err| panic!("apply {path} to isolated target: {err}"));
        }
    }

    /// Drop only the database this run created. No DROP or TRUNCATE is issued
    /// against any shared table name. A failed drop is reported to the caller.
    pub fn cleanup(mut self) {
        self.drop_owned_strict();
    }

    fn drop_owned_strict(&mut self) {
        if self.cleaned {
            return;
        }
        let mut admin =
            Client::connect(&self.admin_dsn, NoTls).expect("connect to postgres for cleanup");
        admin
            .batch_execute(&drop_database_sql(&self.owned_name))
            .expect("drop owned disposable database");
        self.cleaned = true;
    }

    /// Best-effort drop used by `Drop`, so a failing assertion does not leave the
    /// owned database behind. Never panics: a panic inside `Drop` would abort the
    /// process.
    fn drop_owned_best_effort(&mut self) {
        if self.cleaned {
            return;
        }
        self.cleaned = true;
        if let Ok(mut admin) = Client::connect(&self.admin_dsn, NoTls) {
            let _ = admin.batch_execute(&drop_database_sql(&self.owned_name));
        }
    }
}

/// The single DROP statement the helper ever issues. It names one owned database
/// and never a table, so a shared table can never be reset by a test run. The
/// name is generated (lower-case `[a-z0-9_]`) and quoted, so case folding and
/// reserved words cannot change which database is dropped. `WITH (FORCE)` is a
/// safety net for a connection leaked by a panicking test; the database is owned
/// by this run, so no other process is a legitimate client.
fn drop_database_sql(owned_name: &str) -> String {
    format!("DROP DATABASE IF EXISTS \"{owned_name}\" WITH (FORCE);")
}

impl Drop for IsolatedTarget {
    fn drop(&mut self) {
        self.drop_owned_best_effort();
    }
}

/// A per-run token that keeps target names unique within a process.
fn next_run_token() -> String {
    let seq = RUN_TOKEN.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let pid = std::process::id();
    format!("{pid}_{nanos}_{seq}")
}

fn with_database(dsn: &str, database: &str) -> String {
    let scheme_end = dsn.find("://").expect("dsn scheme") + 3;
    let authority_end = dsn[scheme_end..]
        .find('/')
        .map(|index| scheme_end + index)
        .unwrap_or(dsn.len());
    let query = dsn[authority_end..]
        .find('?')
        .map(|index| &dsn[authority_end + index..])
        .unwrap_or("");
    format!("{}/{}{}", &dsn[..authority_end], database, query)
}

fn sanitize(label: &str) -> String {
    // PostgreSQL folds unquoted identifiers to lower case, so the name must be
    // lower case or `CREATE DATABASE` and the DSN would disagree.
    let cleaned: String = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "suite".to_string()
    } else {
        cleaned
    }
}

/// Build the owned database name inside PostgreSQL's identifier limit.
///
/// The readable suite label absorbs the truncation, never the unique token, so
/// two suites that share a long prefix stay distinct. The `_test` suffix is
/// part of the safety guard and is always preserved.
fn isolated_database_name(label: &str, token: &str) -> String {
    const PREFIX: &str = "genealogy_";
    const SUFFIX: &str = "_test";
    let reserved = PREFIX.len() + 1 + token.len() + SUFFIX.len();
    let budget = MAX_IDENTIFIER_BYTES.saturating_sub(reserved).max(1);
    let label: String = label.chars().take(budget).collect();
    let name = format!("{PREFIX}{label}_{token}{SUFFIX}");
    assert!(
        name.len() <= MAX_IDENTIFIER_BYTES,
        "generated database name exceeds the identifier limit: {name}"
    );
    assert!(
        name.chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
        "generated database name is not a safe unquoted identifier: {name}"
    );
    name
}

/// Read and validate the base DSN. Returns `None` when `GENEALOGY_DATABASE_URL`
/// is unset or empty so callers can skip cleanly.
pub fn base_dsn() -> Option<String> {
    let dsn = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    if dsn.is_empty() {
        return None;
    }
    let rag = std::env::var("DATABASE_URL").ok();
    validate_dsn(&dsn, rag.as_deref());
    Some(dsn)
}

/// The single guarded entry point every mutating genealogy suite uses.
///
/// It refuses a database whose name does not end in `_test` or `_disposable`,
/// and it refuses a target equal to the RAG `DATABASE_URL`. Both refusals are
/// assertion-based, so a bad target fails loudly before any DDL or DML runs.
pub fn validate_dsn(dsn: &str, rag_dsn: Option<&str>) {
    let config = postgres::Config::from_str(dsn).expect("genealogy DSN parses");
    let database = config.get_dbname().unwrap_or_default();
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable genealogy database {database:?}"
    );
    if let Some(rag) = rag_dsn {
        let rag_config = postgres::Config::from_str(rag).expect("DATABASE_URL parses");
        assert_ne!(
            (config.get_dbname(), config.get_hosts(), config.get_ports()),
            (
                rag_config.get_dbname(),
                rag_config.get_hosts(),
                rag_config.get_ports()
            ),
            "the genealogy integration test must not target the RAG database"
        );
    }
}

/// Renders a PostgreSQL error with its server-side detail when one exists.
/// `postgres::Error`'s own `Display` is generic ("db error"), which hides the
/// SQLSTATE and message a driver or server failure carries.
fn describe_pg_error(err: &postgres::Error) -> String {
    match err.as_db_error() {
        Some(db) => format!(
            "{} (SQLSTATE {}, severity {:?})",
            db.message(),
            db.code().code(),
            db.severity()
        ),
        None => err.to_string(),
    }
}

/// Create a uniquely named disposable database for `suite_label`.
///
/// The admin connection tries the `postgres` maintenance database, then
/// `template1`. If neither can create a database, the helper panics with an
/// actionable message rather than silently running the suite against a shared
/// target. That loud failure is required because genealogy migrations 0010 and
/// 0011 look up `pg_constraint` by name, which is database-global; a shared
/// base database would let sibling suites pollute each other's results.
pub fn create_isolated_target(base: &str, suite_label: &str) -> IsolatedTarget {
    let token = next_run_token();
    let label = sanitize(suite_label);
    let database_name = isolated_database_name(&label, &token);

    let mut errors: Vec<String> = Vec::new();
    for maintenance_db in ["postgres", "template1"] {
        let admin_dsn = with_database(base, maintenance_db);
        let mut admin = match Client::connect(&admin_dsn, NoTls) {
            Ok(client) => client,
            Err(err) => {
                errors.push(format!("{maintenance_db}: {}", describe_pg_error(&err)));
                continue;
            }
        };
        // Create directly instead of checking pg_database first: the check and
        // the create cannot be made atomic, and a per-run token makes a genuine
        // collision a bug worth surfacing. A failure on this maintenance database
        // is recorded and the loop falls through to `template1`, so the final
        // message names every attempt and never hides a permission error.
        match admin.batch_execute(&format!("CREATE DATABASE \"{database_name}\";")) {
            Ok(()) => {
                return IsolatedTarget {
                    dsn: with_database(base, &database_name),
                    mode: Mode::Database,
                    owned_name: database_name,
                    admin_dsn,
                    cleaned: false,
                };
            }
            Err(err) => {
                if err.code() == Some(&postgres::error::SqlState::DUPLICATE_DATABASE) {
                    panic!("per-run database name collided unexpectedly: {database_name}");
                }
                errors.push(format!("{maintenance_db}: {}", describe_pg_error(&err)));
            }
        }
    }

    let detail = if errors.is_empty() {
        "no maintenance database was attempted".to_string()
    } else {
        errors.join("; ")
    };
    panic!(
        "cannot create isolated genealogy database {database_name} for suite {suite_label}: \
         the configured role must hold CREATEDB on {maintenance_list}; {detail}. \
         Schema fallback is intentionally not offered because genealogy migrations \
         0010/0011 resolve constraints through database-global pg_constraint lookups, \
         so a shared base database would corrupt the results.",
        maintenance_list = "postgres or template1",
    )
}
