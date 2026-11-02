//! RAG-101 script harness: runs scripts/sop-rag/test-sop-rag.sh.
//!
//! The shell script drives env.sh and retrieval.sh against a stub binary in a
//! temporary directory, so this test needs no database and no Ollama. On a
//! non-zero exit it prints the script's stdout and stderr to help diagnosis.

use std::process::Command;

#[test]
fn sop_rag_scripts_pass() {
    let output = Command::new("sh")
        .arg("scripts/sop-rag/test-sop-rag.sh")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to run scripts/sop-rag/test-sop-rag.sh");

    assert!(
        output.status.success(),
        "scripts/sop-rag/test-sop-rag.sh failed with status {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
