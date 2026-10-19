//! Evidence acceptance harness for the Go-to-Rust transition.
//!
//! Reads `docs/reference/rust-transition-evidence.md`
//! relative to `CARGO_MANIFEST_DIR` and asserts the required subjects.

use std::path::PathBuf;

/// Repo root, two levels above this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// The frozen transition evidence document.
pub fn evidence_path() -> PathBuf {
    repo_root().join("docs/reference/rust-transition-evidence.md")
}

/// Load the evidence document text.
pub fn load_evidence() -> String {
    std::fs::read_to_string(evidence_path()).expect("read transition evidence")
}

/// Return the body under a `## <heading>` section, or `None`.
pub fn section<'a>(doc: &'a str, heading: &str) -> Option<&'a str> {
    let marker = format!("## {heading}");
    let start = doc.find(&marker)? + marker.len();
    let rest = &doc[start..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    Some(&rest[..end])
}

/// True when `s` contains a 40-hex-character lowercase string.
pub fn has_sha40(s: &str) -> bool {
    s.split(|c: char| !c.is_ascii_hexdigit() || c.is_ascii_uppercase())
        .any(|tok| tok.len() == 40 && tok.chars().all(|c| c.is_ascii_hexdigit()))
}

/// True when `s` contains an http(s) URL.
pub fn has_url(s: &str) -> bool {
    s.contains("https://") || s.contains("http://")
}
