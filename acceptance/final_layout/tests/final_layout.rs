//! RUST-007C final Rust-only layout.
//!
//! Asserts that the repository is Rust-only, the Go implementation and its
//! tooling are gone, and the frozen fixtures, provenance, shared assets, Axum
//! API, CLI binaries, and Next.js/Playwright integration remain.

use std::fs;
use std::path::{Path, PathBuf};

use final_layout_acceptance::repo_root;

fn path(relative: &str) -> PathBuf {
    repo_root().join(relative)
}

fn read(relative: &str) -> String {
    fs::read_to_string(path(relative))
        .unwrap_or_else(|err| panic!("read {}: {err}", path(relative).display()))
}

fn assert_exists(relative: &str) {
    assert!(
        path(relative).exists(),
        "required path is missing: {relative}"
    );
}

fn assert_absent(relative: &str) {
    assert!(
        !path(relative).exists(),
        "Go-era path still exists: {relative}"
    );
}

/// Recursively collects files with `extension`, skipping generated and
/// historical state directories.
fn files_with_extension(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if matches!(
                    name.as_str(),
                    "target" | "node_modules" | ".git" | ".agent-sdlc" | ".next" | ".next-build"
                ) {
                    continue;
                }
                stack.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
                out.push(path);
            }
        }
    }
    out
}

#[test]
fn rust_package_sits_at_the_repository_root() {
    for required in [
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "src/lib.rs",
        "src/api.rs",
        "src/bin/api.rs",
        "src/bin/ingest.rs",
        "src/bin/query.rs",
        "src/bin/eval.rs",
        "src/bin/expbilingual.rs",
        "tests",
        "acceptance",
        "fixtures",
        "evals",
        "examples",
        "migrations",
        "docker-compose.yml",
        "web",
        "scripts/web-e2e.sh",
        "web/e2e/rag.spec.ts",
        "web/playwright.config.ts",
    ] {
        assert_exists(required);
    }

    let manifest = read("Cargo.toml");
    for binary in ["ingest", "query", "eval", "expbilingual", "api"] {
        assert!(
            manifest.contains(&format!("name = \"{binary}\"")),
            "Cargo.toml must declare the {binary} binary"
        );
    }
    assert!(
        manifest.contains("name = \"rag-foundation\""),
        "the root package must remain rag-foundation"
    );
}

#[test]
fn go_implementation_and_tooling_are_gone() {
    for removed in [
        "go.mod",
        "go.sum",
        "cmd",
        "internal",
        "tools",
        "scripts/sweep.sh",
        "scripts/eval-model-sweep.sh",
        "acceptance/clean_checkout_go",
    ] {
        assert_absent(removed);
    }

    let go_files = files_with_extension(&repo_root(), "go");
    assert!(go_files.is_empty(), "Go source files remain: {go_files:?}");
}

#[test]
fn development_tooling_describes_the_rust_only_layout() {
    let makefile = read("Makefile");
    for removed in [
        "go build ./",
        "go test ./",
        "go vet ./",
        "go run ./",
        "gofmt -l",
    ] {
        assert!(
            !makefile.contains(removed),
            "Makefile still contains the Go command {removed:?}"
        );
    }
    assert!(makefile.contains("cargo build --locked"));
    assert!(makefile.contains("cargo test --locked"));
    assert!(makefile.contains("cargo clippy --locked"));

    for hook in [".githooks/pre-commit", ".githooks/pre-push"] {
        let text = read(hook);
        for removed in [
            "go build ./",
            "go test ./",
            "go vet ./",
            "go run ./",
            "gofmt -l",
            "go mod ",
        ] {
            assert!(
                !text.contains(removed),
                "{hook} still contains the Go command {removed:?}"
            );
        }
        assert!(
            text.contains("cargo") || text.contains("make"),
            "{hook} must run the Rust checks"
        );
    }

    let ci = read(".github/workflows/ci.yml");
    for removed in [
        "setup-go",
        "go build ./",
        "go test ./",
        "go vet ./",
        "go mod",
        "gofmt",
    ] {
        assert!(
            !ci.contains(removed),
            "CI still contains the Go command {removed:?}"
        );
    }
    assert!(ci.contains("cargo"));
    assert!(ci.contains("npm ci"));
}

#[test]
fn frozen_provenance_and_shared_assets_are_preserved() {
    for preserved in [
        "migrations/001_init.sql",
        "migrations/002_ingestion_metadata.sql",
        "migrations/003_add_language.sql",
        "migrations/004_page.sql",
        "migrations/005_add_corpus.sql",
        "evals/retrieval.json",
        "examples/loan-policy.md",
        "fixtures/parity/section_chunk_parity.json",
        "fixtures/parity/inputs/text-layer.pdf",
        "fixtures/parity/frozen/README.md",
        "docs/reports/rust-migration/RUST-001-go-baseline.md",
    ] {
        assert_exists(preserved);
    }

    let evidence = read("docs/reference/rust-transition-evidence.md");
    assert!(
        evidence.contains("d72fb398e8959eea83ed24be03f6df8cdeca86a2"),
        "the pinned Go reference SHA must be preserved in the transition evidence"
    );
    assert!(
        evidence.contains("https://github.com/imhttran/rag-template-rust"),
        "the source repository URL must be preserved in the transition evidence"
    );

    let fixture = read("fixtures/parity/section_chunk_parity.json");
    assert!(
        fixture.contains("d72fb398e8959eea83ed24be03f6df8cdeca86a2"),
        "the frozen fixture must keep its Go provenance"
    );
    let frozen = read("fixtures/parity/frozen/README.md");
    assert!(frozen.contains("d72fb398e8959eea83ed24be03f6df8cdeca86a2"));
}

#[test]
fn rust_only_validation_crates_replace_the_go_dependent_ones() {
    for required in [
        "acceptance/final_layout/Cargo.toml",
        "acceptance/clean_checkout/Cargo.toml",
        "acceptance/api_contract/Cargo.toml",
        "acceptance/evidence/Cargo.toml",
    ] {
        assert_exists(required);
    }
}
