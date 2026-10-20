//! TRAN-001A source inventory command.
//!
//! Discovers every regular file under a source root recursively and writes a
//! local manifest with the relative path, format, size, SHA-256, PDF page
//! count, missing-file records, and duplicate groups. The module
//! [`rag_foundation::inventory`] is read-only; only this binary writes the
//! manifest, to a caller-supplied path.
//!
//! Usage:
//!
//! ```text
//! inventory [SOURCE_ROOT] MANIFEST_PATH
//! ```
//!
//! `SOURCE_ROOT` defaults to `docs/tran`; `MANIFEST_PATH` is required.
//!
//! Exit status:
//!
//! - `0` — complete inventory with no missing files (duplicates are reported
//!   but do not fail the run).
//! - `2` — usage error, rejected manifest path, or inventory error.
//! - `1` — one or more source files were missing or unreadable.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rag_foundation::inventory::{inventory, reject_manifest_path, to_json, Manifest};

const DEFAULT_SOURCE_ROOT: &str = "docs/tran";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let root = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOURCE_ROOT));
    let Some(manifest_arg) = args.get(1) else {
        eprintln!("usage: inventory [SOURCE_ROOT] MANIFEST_PATH");
        return ExitCode::from(2);
    };
    let manifest_path = PathBuf::from(manifest_arg);

    match run(&root, &manifest_path) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("inventory: {err}");
            ExitCode::from(2)
        }
    }
}

fn run(root: &Path, manifest_path: &Path) -> Result<ExitCode, String> {
    reject_manifest_path(root, manifest_path)?;

    let manifest = inventory(root).map_err(|err| err.to_string())?;

    let json = to_json(&manifest);
    write_manifest(manifest_path, &json)?;

    report(&manifest, root, manifest_path);

    if manifest.has_missing() {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn write_manifest(manifest_path: &Path, json: &str) -> Result<(), String> {
    if let Some(parent) = manifest_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        }
    }
    fs::write(manifest_path, json)
        .map_err(|err| format!("cannot write {}: {err}", manifest_path.display()))
}

fn report(manifest: &Manifest, root: &Path, manifest_path: &Path) {
    eprintln!(
        "inventory: {} record(s) from {}",
        manifest.records.len(),
        root.display()
    );
    eprintln!("manifest: {}", manifest_path.display());

    let missing: Vec<&str> = manifest
        .records
        .iter()
        .filter(|record| record.is_missing())
        .map(|record| record.relative_path.as_str())
        .collect();
    if missing.is_empty() {
        eprintln!("missing files: none");
    } else {
        eprintln!("missing files: {}", missing.len());
        for path in &missing {
            eprintln!("  missing: {path}");
        }
    }

    let pdfs = manifest
        .records
        .iter()
        .filter(|record| record.format == "pdf")
        .count();
    if pdfs > 0 {
        eprintln!("pdf files: {pdfs}");
    }

    if manifest.duplicates.is_empty() {
        eprintln!("duplicate groups: none");
    } else {
        eprintln!("duplicate groups: {}", manifest.duplicates.len());
        for group in &manifest.duplicates {
            eprintln!("  duplicates: {}", group.join(", "));
        }
    }
}
