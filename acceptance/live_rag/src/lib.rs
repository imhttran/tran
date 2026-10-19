//! Shared helpers for the RUST-007E live production RAG acceptance.
//!
//! The crate contains no retrieval, embedding, or generation logic; the test
//! drives the production binaries and inspects the dedicated database.

use std::path::PathBuf;

/// Repository root, two levels above this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives three levels below the repository root")
        .to_path_buf()
}
