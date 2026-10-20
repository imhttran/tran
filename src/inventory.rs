//! TRAN-001A source inventory.
//!
//! A local, read-only inventory of a source tree. The module walks a root
//! directory recursively, classifies each regular file by extension, records
//! its byte size and SHA-256, counts PDF pages through the existing text-layer
//! loader ([`crate::loader::pdf`]), and detects missing files and duplicate
//! content. Nothing here writes to the database or modifies a source file.
//!
//! Generated audit output under `docs/tran/audit/` is excluded from the walk, so
//! private reports and manifests never count as source input. The exclusion is a
//! path prefix, not a directory name, so a sibling `audit/` folder elsewhere
//! stays visible.
//!
//! Sources are only ever opened for reading; a manifest write happens in
//! `src/bin/inventory.rs`, never in this module.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::loader::pdf::{load_pdf_bytes, PdfError};

/// The format classifier value for a file whose extension is not recognized.
pub const FORMAT_OTHER: &str = "other";

/// Source directories that hold frozen assets and must never be written.
pub const FROZEN_DIRECTORIES: [&str; 4] = ["examples", "fixtures", "migrations", "evals"];

/// Relative path prefixes that hold generated output and are never inventoried.
///
/// An entry is matched against the manifest form of a relative path (see
/// [`relative_path_string`]) and must end with `/`, so only paths under that
/// exact prefix are dropped.
pub const GENERATED_OUTPUT_PREFIXES: [&str; 1] = ["docs/tran/audit/"];

/// True when the manifest-form relative path lies under a generated prefix.
pub fn is_generated_output(relative_path: &str) -> bool {
    GENERATED_OUTPUT_PREFIXES
        .iter()
        .any(|prefix| relative_path.starts_with(prefix))
}

/// One inventory record; the manifest stores a list of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    /// Path relative to the source root, using `/` separators, never absolute.
    pub relative_path: String,
    /// Format classifier value: `pdf`, `md`, `txt`, or `other`.
    pub format: String,
    /// Byte length read from disk at hash time.
    pub size_bytes: u64,
    /// Lowercase 64-hex-digit SHA-256 of the file bytes; `None` when the file
    /// could not be read.
    pub sha256: Option<String>,
    /// Page count for PDFs, `None` for non-PDFs and unreadable PDFs.
    pub pdf_page_count: Option<u32>,
    /// Free-form notes: `scanned`, `incomplete-text-layer`, `not-a-pdf`,
    /// `missing`/`unreadable` error text.
    pub notes: Option<String>,
}

impl SourceRecord {
    /// A record whose bytes could not be read at hash time.
    pub fn missing(relative_path: String, format: String, error: String) -> Self {
        SourceRecord {
            relative_path,
            format,
            size_bytes: 0,
            sha256: None,
            pdf_page_count: None,
            notes: Some(error),
        }
    }

    /// True when the record has no hash, i.e. the file has gone missing.
    pub fn is_missing(&self) -> bool {
        self.sha256.is_none()
    }
}

/// A manifest: records plus the duplicate groups found by SHA-256.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Inventory records, sorted by `relative_path`.
    pub records: Vec<SourceRecord>,
    /// Groups of two or more records sharing a SHA-256, each sorted by path.
    pub duplicates: Vec<Vec<String>>,
}

impl Manifest {
    /// True when at least one record is missing, so the caller exits non-zero.
    pub fn has_missing(&self) -> bool {
        self.records.iter().any(SourceRecord::is_missing)
    }
}

/// Inventory errors that abort the walk itself.
#[derive(Debug, Error)]
pub enum InventoryError {
    /// The source root cannot be read.
    #[error("inventory: cannot read source root {path}: {source}")]
    Root {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A directory entry inside the tree cannot be read.
    #[error("inventory: cannot read directory {path}: {source}")]
    Directory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A nested entry could not be examined.
    #[error("inventory: cannot stat {path}: {source}")]
    Entry {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Classifies a path by extension. Unknown extensions map to `other`.
pub fn classify(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => "pdf",
        Some("md") | Some("markdown") => "md",
        Some("txt") => "txt",
        _ => FORMAT_OTHER,
    }
}

/// Walks `root` recursively and returns the discovered regular-file paths
/// relative to `root`, sorted for deterministic output.
///
/// Directories and symlinks are not recorded. Directory read failures abort the
/// walk; a file that vanishes later is reported as a missing record by the
/// record builder.
pub fn discover(root: &Path) -> Result<Vec<PathBuf>, InventoryError> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|source| InventoryError::Directory {
            path: dir.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| InventoryError::Directory {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|source| InventoryError::Entry {
                path: path.clone(),
                source,
            })?;
            let file_type = meta.file_type();
            if file_type.is_symlink() {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .map_err(|_| InventoryError::Entry {
                    path: path.clone(),
                    source: std::io::Error::other("path outside source root"),
                })?
                .to_path_buf();
            let rel_string = relative_path_string(&rel);
            if file_type.is_dir() {
                // Do not enter a directory that is itself a generated prefix.
                if !is_generated_output(&format!("{rel_string}/")) {
                    stack.push(path);
                }
            } else if file_type.is_file() && !is_generated_output(&rel_string) {
                found.push(rel);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Renders a relative path in the manifest form: `/` separated, no `.` parts.
pub fn relative_path_string(rel: &Path) -> String {
    let mut parts = Vec::new();
    for component in rel.components() {
        if let Component::Normal(part) = component {
            parts.push(part.to_string_lossy().into_owned());
        }
    }
    parts.join("/")
}

/// Builds one record from the file bytes, classifying PDFs and counting pages.
pub fn build_record(root: &Path, rel: &Path) -> SourceRecord {
    let relative_path = relative_path_string(rel);
    let format = classify(rel).to_string();
    let absolute = root.join(rel);
    let bytes = match fs::read(&absolute) {
        Ok(bytes) => bytes,
        Err(err) => {
            return SourceRecord::missing(relative_path, format, format!("missing: {err}"));
        }
    };
    let size_bytes = bytes.len() as u64;
    let sha256 = sha256_hex(&bytes);
    let (pdf_page_count, notes) = if format == "pdf" {
        pdf_notes(&bytes)
    } else {
        (None, None)
    };
    SourceRecord {
        relative_path,
        format,
        size_bytes,
        sha256: Some(sha256),
        pdf_page_count,
        notes,
    }
}

/// Counts PDF pages from the byte buffer and derives a text-layer note.
pub fn pdf_notes(bytes: &[u8]) -> (Option<u32>, Option<String>) {
    match load_pdf_bytes(bytes) {
        Ok(extraction) => {
            let pages = extraction
                .sections
                .iter()
                .filter_map(|section| section.page)
                .chain(extraction.incomplete_pages.iter().copied())
                .max()
                .unwrap_or(0);
            let note = if extraction.incomplete_pages.is_empty() {
                None
            } else {
                Some("incomplete-text-layer".to_string())
            };
            (Some(pages), note)
        }
        Err(PdfError::Scanned) => {
            let pages = pdf_page_count(bytes);
            (pages, Some("scanned".to_string()))
        }
        Err(PdfError::Invalid(err)) => (pdf_page_count(bytes), Some(format!("not-a-pdf: {err}"))),
    }
}

fn pdf_page_count(bytes: &[u8]) -> Option<u32> {
    match lopdf::Document::load_mem(bytes) {
        Ok(doc) => Some(doc.get_pages().len() as u32),
        Err(_) => None,
    }
}

/// Builds a manifest from a root: discovers, hashes, classifies, and groups.
pub fn inventory(root: &Path) -> Result<Manifest, InventoryError> {
    if !root.is_dir() {
        return Err(InventoryError::Root {
            path: root.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not a directory"),
        });
    }
    let rels = discover(root)?;
    let records: Vec<SourceRecord> = rels.iter().map(|rel| build_record(root, rel)).collect();
    let duplicates = duplicate_groups(&records);
    Ok(Manifest {
        records,
        duplicates,
    })
}

/// Groups records by SHA-256 and returns the groups with two or more paths.
pub fn duplicate_groups(records: &[SourceRecord]) -> Vec<Vec<String>> {
    let mut by_hash: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for record in records {
        if let Some(hash) = record.sha256.as_deref() {
            by_hash
                .entry(hash)
                .or_default()
                .push(record.relative_path.clone());
        }
    }
    let mut groups: Vec<Vec<String>> = by_hash
        .into_values()
        .filter(|paths| paths.len() > 1)
        .map(|mut paths| {
            paths.sort();
            paths
        })
        .collect();
    groups.sort();
    groups
}

/// Serializes a manifest as pretty JSON with a trailing newline, byte-stable
/// across runs.
pub fn to_json(manifest: &Manifest) -> String {
    let mut out = serde_json::to_string_pretty(manifest).expect("manifest serializes");
    out.push('\n');
    out
}

/// Rejects a manifest path that collides with a source file or a frozen tree.
pub fn reject_manifest_path(root: &Path, manifest: &Path) -> Result<(), String> {
    let manifest_abs = absolutize(manifest)?;
    let root_abs = absolutize(root)?;
    for frozen in FROZEN_DIRECTORIES {
        let frozen_abs = absolutize(Path::new(frozen))?;
        if manifest_abs.starts_with(&frozen_abs) {
            return Err(format!(
                "manifest path {} lies under frozen asset directory {frozen}/",
                manifest.display()
            ));
        }
    }
    if manifest_abs.starts_with(&root_abs) && manifest_abs.is_file() {
        let rel = manifest_abs
            .strip_prefix(&root_abs)
            .unwrap_or(&manifest_abs);
        return Err(format!(
            "manifest path {} equals a source file ({})",
            manifest.display(),
            relative_path_string(rel)
        ));
    }
    Ok(())
}

fn absolutize(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(normalize(path))
    } else {
        let cwd = std::env::current_dir().map_err(|err| format!("cwd: {err}"))?;
        Ok(normalize(&cwd.join(path)))
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Computes a lowercase 64-hex-digit SHA-256 of the buffer.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        write!(out, "{byte:02x}").expect("write to String");
    }
    out
}

// A small, dependency-free SHA-256 (FIPS 180-4). The repository does not list
// a SHA-256 crate as a direct dependency; this keeps the inventory read-only
// and avoids a Cargo.toml change.
struct Sha256;

impl Sha256 {
    fn digest(data: &[u8]) -> [u8; 32] {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut h: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];

        let bit_len = (data.len() as u64).wrapping_mul(8);
        let mut padded = data.to_vec();
        padded.push(0x80);
        while padded.len() % 64 != 56 {
            padded.push(0);
        }
        padded.extend_from_slice(&bit_len.to_be_bytes());

        for chunk in padded.as_chunks::<64>().0 {
            let mut w = [0u32; 64];
            for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
                w[i] = u32::from_be_bytes(*word);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }

            let mut a = h[0];
            let mut b = h[1];
            let mut c = h[2];
            let mut d = h[3];
            let mut e = h[4];
            let mut f = h[5];
            let mut g = h[6];
            let mut hh = h[7];

            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let temp1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let temp2 = s0.wrapping_add(maj);

                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(temp1);
                d = c;
                c = b;
                b = a;
                a = temp1.wrapping_add(temp2);
            }

            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }

        let mut out = [0u8; 32];
        for (i, word) in h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn discovers_nested_files_sorted_and_ignores_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "b/second.txt", b"two");
        write(root, "a/first.md", b"one");
        write(root, "weird.xyz", b"?");
        write(root, "empty.bin", b"");
        fs::create_dir_all(root.join("empty-dir")).unwrap();
        let found = discover(root).unwrap();
        let rels: Vec<String> = found.iter().map(|p| relative_path_string(p)).collect();
        assert_eq!(
            rels,
            vec!["a/first.md", "b/second.txt", "empty.bin", "weird.xyz"]
        );
    }

    #[test]
    fn generated_audit_output_is_never_counted_and_stays_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // The four source-like inputs at the root.
        write(root, "Family Genealogy.pdf", b"%PDF-1.4 fake");
        write(root, "Tran-Family-English.md", b"english");
        write(root, "Tran-Family-People-Catalog.md", b"catalog");
        write(root, "Tran-Family-Vietnamese.md", b"vietnamese");
        // Generated audit output, including a nested file.
        write(root, "docs/tran/audit/manifest.json", b"{}");
        write(root, "docs/tran/audit/nested/report.md", b"# report");
        // A legitimate nested source file outside the audit folder.
        write(root, "docs/tran/notes/synthetic.md", b"# synthetic");
        // A sibling `audit` directory elsewhere must stay visible.
        write(root, "docs/other/audit/keep.md", b"# keep");

        let first = inventory(root).unwrap();
        let second = inventory(root).unwrap();

        let first_paths: Vec<String> = first
            .records
            .iter()
            .map(|record| record.relative_path.clone())
            .collect();
        let second_paths: Vec<String> = second
            .records
            .iter()
            .map(|record| record.relative_path.clone())
            .collect();
        assert_eq!(first_paths, second_paths, "repeat runs must be identical");
        assert_eq!(to_json(&first), to_json(&second));

        assert_eq!(
            first_paths,
            vec![
                "Family Genealogy.pdf".to_string(),
                "Tran-Family-English.md".to_string(),
                "Tran-Family-People-Catalog.md".to_string(),
                "Tran-Family-Vietnamese.md".to_string(),
                "docs/other/audit/keep.md".to_string(),
                "docs/tran/notes/synthetic.md".to_string(),
            ]
        );
        assert!(
            !first_paths
                .iter()
                .any(|path| path.starts_with("docs/tran/audit/")),
            "audit output must never be counted"
        );
    }

    #[test]
    fn classifies_formats() {
        assert_eq!(classify(Path::new("a.PDF")), "pdf");
        assert_eq!(classify(Path::new("a.md")), "md");
        assert_eq!(classify(Path::new("a.txt")), "txt");
        assert_eq!(classify(Path::new("a.bin")), FORMAT_OTHER);
        assert_eq!(classify(Path::new("noext")), FORMAT_OTHER);
    }

    #[test]
    fn record_zero_byte_file_has_known_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "empty.txt", b"");
        let record = build_record(root, Path::new("empty.txt"));
        assert_eq!(record.size_bytes, 0);
        assert_eq!(
            record.sha256.as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(record.pdf_page_count, None);
    }

    #[test]
    fn duplicate_group_lists_both_paths_and_size_only_is_not_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "a.md", b"same");
        write(root, "b.md", b"same");
        write(root, "c.md", b"diff");
        let manifest = inventory(root).unwrap();
        assert_eq!(
            manifest.duplicates,
            vec![vec!["a.md".to_string(), "b.md".to_string()]]
        );
        assert!(!manifest.has_missing());
    }

    #[test]
    fn missing_file_yields_error_record() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "gone.txt", b"x");
        let rels = discover(root).unwrap();
        fs::remove_file(root.join("gone.txt")).unwrap();
        let record = build_record(root, &rels[0]);
        assert!(record.is_missing());
        assert!(record.notes.unwrap().starts_with("missing"));
    }

    #[test]
    fn pdf_page_counts_and_notes() {
        let root = Path::new(".");
        let text = build_record(root, Path::new("fixtures/parity/inputs/text-layer.pdf"));
        assert_eq!(text.pdf_page_count, Some(2));
        assert_eq!(text.notes, None);
        let mixed = build_record(
            root,
            Path::new("fixtures/parity/inputs/mixed-text-scan.pdf"),
        );
        assert_eq!(mixed.pdf_page_count, Some(2));
        assert_eq!(mixed.notes.as_deref(), Some("incomplete-text-layer"));
        let scanned = build_record(root, Path::new("fixtures/parity/inputs/no-text-layer.pdf"));
        assert_eq!(scanned.pdf_page_count, Some(1));
        assert_eq!(scanned.notes.as_deref(), Some("scanned"));
    }

    #[test]
    fn malformed_pdf_gets_null_count_and_note() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "broken.pdf", b"not a pdf");
        let record = build_record(root, Path::new("broken.pdf"));
        assert_eq!(record.pdf_page_count, None);
        assert!(record.notes.unwrap().starts_with("not-a-pdf"));
    }

    #[test]
    fn byte_stable_json_across_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "a.txt", b"a");
        write(root, "b.md", b"b");
        let first = to_json(&inventory(root).unwrap());
        let second = to_json(&inventory(root).unwrap());
        assert_eq!(first, second);
    }

    #[test]
    fn rejects_frozen_and_source_manifest_paths() {
        assert!(reject_manifest_path(Path::new("."), Path::new("fixtures/out.json")).is_err());
        assert!(reject_manifest_path(Path::new("."), Path::new("examples/x.json")).is_err());
        assert!(reject_manifest_path(Path::new("."), Path::new("evals/x.json")).is_err());
        assert!(reject_manifest_path(Path::new("."), Path::new("migrations/x.json")).is_err());
        assert!(reject_manifest_path(Path::new("."), Path::new("target/inventory.json")).is_ok());
    }

    #[test]
    fn record_roundtrips_through_json() {
        let record = SourceRecord {
            relative_path: "a.txt".into(),
            format: "txt".into(),
            size_bytes: 1,
            sha256: Some("00".repeat(32)),
            pdf_page_count: None,
            notes: None,
        };
        let text = serde_json::to_string(&record).unwrap();
        let back: SourceRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(record, back);
    }
}
