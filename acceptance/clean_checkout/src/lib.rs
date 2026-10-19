//! Shared helpers for the RUST-007C clean-checkout acceptance.

use std::path::PathBuf;

/// Repository root, two levels above this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("resolve the repository root")
}
