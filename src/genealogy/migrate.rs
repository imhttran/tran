//! Genealogy migration runner.
//!
//! [`plan_migrations`] lists `genealogy/migrations/*.sql` in file-name order and
//! marks each file applied when its name is already recorded in
//! `genealogy_schema_version`. [`run_migrations`] applies every unapplied file
//! inside one transaction and records the version (with an applied timestamp) in
//! the same transaction. A second run applies zero files.
//!
//! The runner calls [`resolve_genealogy_dsn`] first, so it refuses to run when
//! the guard fails, and it never touches the RAG `documents` table or
//! `migrations/*.sql`.

use std::fs;
use std::path::{Path, PathBuf};

use super::{resolve_genealogy_dsn, GenealogyError};
use crate::config::Config;
use crate::storage_pg::create_pool;

/// The default directory that holds the genealogy migration files.
pub const MIGRATIONS_DIR: &str = "genealogy/migrations";

/// The name of the genealogy version table.
pub const VERSION_TABLE: &str = "genealogy_schema_version";

/// One migration file and whether it is already applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    /// The file name, used as the version value.
    pub version: String,
    /// The file path.
    pub path: PathBuf,
    /// `true` when the version is already recorded.
    pub applied: bool,
}

/// The outcome of a migration run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationReport {
    /// Versions applied by this run.
    pub applied: Vec<String>,
    /// Versions skipped because they were already recorded.
    pub already_applied: Vec<String>,
}

/// Lists the migration files in `dir` in file-name order.
///
/// Only regular `.sql` files are returned. An unreadable directory is an error.
pub fn list_migration_files(dir: &Path) -> Result<Vec<PathBuf>, GenealogyError> {
    let entries = fs::read_dir(dir).map_err(|err| {
        GenealogyError::Migration(format!("cannot read {}: {err}", dir.display()))
    })?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    files.sort();
    Ok(files)
}

/// Builds a plan from the migration files and the set of applied versions.
pub fn plan_migrations(
    dir: &Path,
    applied_versions: &[String],
) -> Result<Vec<MigrationPlan>, GenealogyError> {
    let files = list_migration_files(dir)?;
    Ok(files
        .into_iter()
        .map(|path| {
            let version = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            let applied = applied_versions.iter().any(|v| v == &version);
            MigrationPlan {
                version,
                path,
                applied,
            }
        })
        .collect())
}

/// Reads the recorded versions from `genealogy_schema_version`.
///
/// A missing table (a clean database) yields an empty list, because the first
/// migration creates the table.
fn recorded_versions(conn: &mut postgres::Client) -> Result<Vec<String>, GenealogyError> {
    let _ = conn;
    let exists = conn
        .query_opt(
            "SELECT 1 FROM information_schema.tables WHERE table_name = $1",
            &[&VERSION_TABLE],
        )
        .map_err(GenealogyError::Query)?;
    if exists.is_none() {
        return Ok(Vec::new());
    }
    let rows = conn
        .query(&format!("SELECT version FROM {VERSION_TABLE}"), &[])
        .map_err(GenealogyError::Query)?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// Applies every unapplied migration file to the guarded genealogy database.
///
/// Each file runs in one transaction. The version row records the file name and
/// the apply time. A failing file rolls back: no table and no version row from
/// that file survive.
pub fn run_migrations(config: &Config, dir: &Path) -> Result<MigrationReport, GenealogyError> {
    let dsn = resolve_genealogy_dsn(config)?;
    let pool = create_pool(&dsn)?;
    let mut conn = pool
        .get()
        .map_err(|err| GenealogyError::Store(crate::storage_pg::PgStoreError::Connection(err)))?;

    let applied_versions = recorded_versions(&mut conn)?;
    let plan = plan_migrations(dir, &applied_versions)?;

    let mut report = MigrationReport::default();
    for migration in plan {
        if migration.applied {
            report.already_applied.push(migration.version);
            continue;
        }
        let sql = fs::read_to_string(&migration.path).map_err(|err| {
            GenealogyError::Migration(format!("cannot read {}: {err}", migration.path.display()))
        })?;
        let mut tx = conn.transaction().map_err(GenealogyError::Query)?;
        tx.batch_execute(&sql)
            .map_err(|err| GenealogyError::Migration(format!("{}: {err}", migration.version)))?;
        tx.execute(
            &format!("INSERT INTO {VERSION_TABLE} (version) VALUES ($1)"),
            &[&migration.version],
        )
        .map_err(|err| GenealogyError::Migration(format!("{}: {err}", migration.version)))?;
        tx.commit().map_err(GenealogyError::Query)?;
        report.applied.push(migration.version);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write(dir: &Path, name: &str, body: &str) {
        let mut file = fs::File::create(dir.join(name)).unwrap();
        file.write_all(body.as_bytes()).unwrap();
    }

    #[test]
    fn plan_lists_files_in_file_name_order() {
        let dir = temp_dir();
        write(dir.path(), "0002_b.sql", "SELECT 1;");
        write(dir.path(), "0001_a.sql", "SELECT 1;");
        write(dir.path(), "notes.txt", "not sql");
        let plan = plan_migrations(dir.path(), &[]).unwrap();
        let names: Vec<_> = plan.iter().map(|m| m.version.clone()).collect();
        assert_eq!(names, vec!["0001_a.sql", "0002_b.sql"]);
        assert!(plan.iter().all(|m| !m.applied));
    }

    #[test]
    fn plan_skips_recorded_versions() {
        let dir = temp_dir();
        write(dir.path(), "0001_a.sql", "SELECT 1;");
        write(dir.path(), "0002_b.sql", "SELECT 1;");
        let plan = plan_migrations(dir.path(), &["0001_a.sql".to_string()]).unwrap();
        assert!(plan[0].applied);
        assert!(!plan[1].applied);
    }

    #[test]
    fn version_table_name_is_the_contract_name() {
        assert_eq!(VERSION_TABLE, "genealogy_schema_version");
    }
}
