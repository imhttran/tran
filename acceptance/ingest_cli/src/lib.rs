//! Acceptance harness support for the production ingestion service seam.
//!
//! This crate does not implement a pipeline. Its integration tests build
//! [`rag_foundation::service::IngestService`] and inspect the recorded effects
//! of the library ports.

/// Returns the absolute path of a parity input fixture.
pub fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/parity/inputs")
        .join(name)
}
