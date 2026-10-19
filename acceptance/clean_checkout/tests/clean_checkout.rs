//! RUST-007C clean-checkout acceptance.
//!
//! Copies the Rust-only working tree into a temporary directory, removes Go from
//! PATH, and runs the root build plus the non-database test targets. This proves
//! the final package builds and tests without a Go toolchain.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clean_checkout_acceptance::repo_root;

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();
        let target = dst.join(&name);
        if entry.file_type()?.is_dir() {
            if matches!(
                name.as_str(),
                "target"
                    | "node_modules"
                    | ".git"
                    | ".agent-sdlc"
                    | ".run"
                    | ".next"
                    | ".next-build"
            ) {
                continue;
            }
            copy_tree(&entry.path(), &target)?;
        } else if name != ".env" {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn cargo_binary() -> PathBuf {
    match std::env::var_os("CARGO") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from("cargo"),
    }
}

fn run(copy: &Path, cargo: &Path, path: &str, args: &[&str]) -> String {
    let output = Command::new(cargo)
        .args(args)
        .current_dir(copy)
        .env("PATH", path)
        .env_remove("DATABASE_URL")
        .env_remove("OLLAMA_URL")
        .env("RUST_BACKTRACE", "1")
        .output()
        .unwrap_or_else(|err| panic!("run cargo {args:?}: {err}"));
    if !output.status.success() {
        panic!(
            "cargo {args:?} failed in the clean checkout:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn clean_checkout_builds_and_tests_without_go() {
    let source = repo_root();
    let copy = std::env::temp_dir().join(format!(
        "rag-rust-only-clean-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0)
    ));
    if copy.exists() {
        fs::remove_dir_all(&copy).expect("remove the previous clean checkout");
    }
    fs::create_dir_all(&copy).expect("create the clean checkout");

    // Copy only the final Rust package and the assets its tests read. The web
    // app and acceptance crates are separate and are not needed for this gate.
    for item in [
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "src",
        "tests",
        "fixtures",
        "evals",
        "examples",
        "migrations",
        "docs",
        "docker-compose.yml",
        "README.md",
        "Makefile",
    ] {
        let src = source.join(item);
        let dst = copy.join(item);
        if src.is_dir() {
            copy_tree(&src, &dst).expect("copy directory into the clean checkout");
        } else {
            fs::copy(&src, &dst).expect("copy file into the clean checkout");
        }
    }

    assert!(
        copy.join("Cargo.toml").is_file(),
        "the clean checkout must contain the root Cargo.toml"
    );
    assert!(
        !copy.join("go.mod").exists(),
        "the clean checkout must not contain go.mod"
    );

    let cargo = cargo_binary();
    let cargo_dir = cargo
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/usr/bin"));
    // The Rust toolchain directory plus the base system tools needed by the
    // compiler/linker. Go lives outside this PATH (for example /usr/local/go/bin).
    let path = format!("{}:/usr/bin:/bin", cargo_dir.display());

    let go = Command::new("/bin/sh")
        .arg("-c")
        .arg("command -v go")
        .env("PATH", &path)
        .output()
        .expect("probe for Go on the sanitized PATH");
    assert!(
        !go.status.success() || go.stdout.is_empty(),
        "Go is unexpectedly available on the sanitized PATH: {}",
        String::from_utf8_lossy(&go.stdout)
    );

    let build = run(
        &copy,
        &cargo,
        &path,
        &["build", "--locked", "--manifest-path", "Cargo.toml"],
    );
    assert!(build.contains("Finished") || build.is_empty());

    run(
        &copy,
        &cargo,
        &path,
        &["test", "--locked", "--manifest-path", "Cargo.toml", "--lib"],
    );
    run(
        &copy,
        &cargo,
        &path,
        &[
            "test",
            "--locked",
            "--manifest-path",
            "Cargo.toml",
            "--test",
            "section_chunk_parity",
        ],
    );

    fs::remove_dir_all(&copy).expect("remove the clean checkout");
}
