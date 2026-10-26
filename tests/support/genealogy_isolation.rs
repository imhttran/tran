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
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use ring::digest::{Context, SHA256};

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
    "genealogy/migrations/0025_assertion_dates.sql",
    "genealogy/migrations/0026_authority_decisions.sql",
    "genealogy/migrations/0027_source_narrative.sql",
    "genealogy/migrations/0028_archive_photograph_inventory.sql",
    "genealogy/migrations/0029_narrative_review_search.sql",
    "genealogy/migrations/0030_lowercase_usernames.sql",
    "genealogy/migrations/0031_archive_photograph_assets.sql",
];

static RUN_TOKEN: AtomicU64 = AtomicU64::new(0);

/// Advisory lock that serializes template creation across test processes.
const TEMPLATE_LOCK: i64 = 0x6765_6e65_616c_6f67;

/// The complete-schema marker written into a template after every migration.
const TEMPLATE_MARKER: &str = "_test_template_marker";

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
    /// True when the database already carries the full migration set.
    migrated: bool,
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
    ///
    /// A target copied from the migrated template is already complete, so this
    /// is a no-op there; only an empty target replays the migrations.
    pub fn apply_migrations(&self) {
        if self.migrated {
            return;
        }
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
        // The database is disposable; skip the per-statement flush that made a
        // small schema take seconds to drop on a busy disk.
        let _ = admin.batch_execute("SET synchronous_commit = off");
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
            let _ = admin.batch_execute("SET synchronous_commit = off");
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

/// Read the base DSN, or skip the suite when it is unset.
///
/// `REQUIRE_TEST_DATABASE=1` turns the skip into a hard failure, so the
/// validation and CI entry points cannot report a green suite while the
/// PostgreSQL integration tests never ran.
pub fn base_dsn_or_skip(label: &str) -> Option<String> {
    match base_dsn() {
        Some(dsn) => Some(dsn),
        None if std::env::var("REQUIRE_TEST_DATABASE").is_ok() => panic!(
            "GENEALOGY_DATABASE_URL is not set: {label} is a required PostgreSQL \
             integration test and REQUIRE_TEST_DATABASE forbids skipping"
        ),
        None => {
            eprintln!("GENEALOGY_DATABASE_URL is unset; skipping {label}");
            None
        }
    }
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

/// Hash of the full migration set, used to name the migrated template database.
/// A changed migration file produces a new template, so a stale template can
/// never be served and no in-place rebuild (which would race test copies) is
/// needed.
fn schema_hash() -> String {
    let mut ctx = Context::new(&SHA256);
    for path in GENEALOGY_MIGRATIONS {
        ctx.update(path.as_bytes());
        ctx.update(&[0]);
        let data = fs::read(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        ctx.update(&data);
        ctx.update(&[0]);
    }
    let digest = ctx.finish();
    let mut hex = String::with_capacity(digest.as_ref().len() * 2);
    for byte in digest.as_ref() {
        use std::fmt::Write;
        write!(&mut hex, "{byte:02x}").expect("write hex");
    }
    hex
}

fn template_database_name() -> String {
    let hash = schema_hash();
    format!("genealogy_schema_{}_test", &hash[..16])
}

/// The process-wide schema template name, so the template is checked once per
/// test binary rather than once per test. A per-test marker connection to the
/// template would otherwise contend with concurrent template copies.
static TEMPLATE: OnceLock<String> = OnceLock::new();

fn ensure_template_once(base: &str) -> &'static str {
    TEMPLATE.get_or_init(|| {
        let name = template_database_name();
        ensure_template(base, &name);
        name
    })
}

/// Ensure the migrated template exists. The advisory lock serializes the first
/// creation across test processes; later callers find it and return immediately.
fn ensure_template(base: &str, template: &str) {
    let admin_dsn = with_database(base, "postgres");
    let mut admin = Client::connect(&admin_dsn, NoTls).unwrap_or_else(|err| {
        panic!(
            "connect to postgres for template: {}. The configured role must hold \
             CREATEDB on postgres or template1, and the server must be reachable.",
            describe_pg_error(&err)
        )
    });
    admin
        .execute("SELECT pg_advisory_lock($1)", &[&TEMPLATE_LOCK])
        .expect("acquire template lock");

    let present: bool = admin
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)",
            &[&template],
        )
        .expect("read template presence")
        .get(0);

    let complete = present
        && Client::connect(&with_database(base, template), NoTls)
            .ok()
            .and_then(|mut client| {
                client
                    .query_opt(
                        &format!("SELECT 1 FROM {TEMPLATE_MARKER} WHERE key = 'complete'"),
                        &[],
                    )
                    .ok()
                    .flatten()
            })
            .is_some();

    if !complete {
        if present {
            admin
                .batch_execute(&format!(
                    "DROP DATABASE IF EXISTS \"{template}\" WITH (FORCE);"
                ))
                .expect("drop incomplete template");
        }
        admin
            .batch_execute(&format!("CREATE DATABASE \"{template}\";"))
            .unwrap_or_else(|err| {
                panic!(
                    "create template {template}: {}. The configured role must hold CREATEDB.",
                    describe_pg_error(&err)
                )
            });

        let mut client =
            Client::connect(&with_database(base, template), NoTls).unwrap_or_else(|err| {
                panic!(
                    "connect to template {template}: {}",
                    describe_pg_error(&err)
                )
            });
        let _ = client.batch_execute("SET synchronous_commit = off");
        for path in GENEALOGY_MIGRATIONS {
            let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
            client
                .batch_execute(&sql)
                .unwrap_or_else(|err| panic!("apply {path} to template {template}: {err}"));
        }
        client
            .batch_execute(&format!(
                "CREATE TABLE {TEMPLATE_MARKER} (key text PRIMARY KEY, value text NOT NULL); \
                 INSERT INTO {TEMPLATE_MARKER} (key, value) VALUES ('complete', '1');"
            ))
            .expect("mark template complete");
    }

    admin
        .execute("SELECT pg_advisory_unlock($1)", &[&TEMPLATE_LOCK])
        .expect("release template lock");
}

/// Create a uniquely named disposable database for `suite_label`, copied from
/// the migrated template.
///
/// Replaying the 30 fsync-bound migrations per test dominated the suite (about
/// 24 s per test in this environment); a template copy takes under a second.
/// The per-suite database is still unique and isolated. The schema-contract
/// suite uses [`create_empty_isolated_target`] so its 0001-0005 subset stays
/// meaningful.
pub fn create_isolated_target(base: &str, suite_label: &str) -> IsolatedTarget {
    let template = ensure_template_once(base);
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
        // A per-run token makes a genuine name collision a bug worth surfacing.
        match admin.batch_execute(&format!(
            "CREATE DATABASE \"{database_name}\" TEMPLATE \"{template}\";"
        )) {
            Ok(()) => {
                return IsolatedTarget {
                    dsn: with_database(base, &database_name),
                    mode: Mode::Database,
                    owned_name: database_name,
                    migrated: true,
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

    // The template copy was refused (for example, a concurrent connection kept
    // the template busy). Fall back to an empty database with an explicit
    // replay so the suite stays correct even though it is slower.
    let detail = if errors.is_empty() {
        "no maintenance database was attempted".to_string()
    } else {
        errors.join("; ")
    };
    eprintln!("isolated target: template copy failed, replaying migrations instead: {detail}");
    let mut target = create_empty_isolated_target(base, suite_label);
    target.apply_migration_files(GENEALOGY_MIGRATIONS);
    target.migrated = true;
    target
}

/// Create a uniquely named empty database for a suite that applies its own
/// migration subset (the schema-contract suite).
pub fn create_empty_isolated_target(base: &str, suite_label: &str) -> IsolatedTarget {
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
                    migrated: false,
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
