//! RUST-007C clean-checkout acceptance.
//!
//! Copies the Rust-only working tree into a temporary directory, removes Go from
//! PATH, and runs the root build plus the non-database test targets. This proves
//! the final package builds and tests without a Go toolchain.
//!
//! The checkout is temporary but its path is stable per cache partition, and
//! Cargo compiles into a persistent target directory outside it. Together with
//! preserved source mtimes, repeated runs reuse compatible compilation artifacts
//! instead of cold-compiling every dependency and the root crate each time.
//!
//! The cache is partitioned by Rust toolchain, host/target architecture, and
//! relevant build configuration, so incompatible artifacts never mix. The
//! temporary checkout is recreated from source on every run, so reused artifacts
//! never stand in for a missing or changed source file or dependency: Cargo's
//! fingerprints still force a rebuild whenever the source tree changes, and the
//! build/test commands still run with `--locked`.
//!
//! Cargo owns the target-directory lock, so concurrent builds sharing a
//! partition serialize safely. A separate advisory lock serializes the shared
//! source checkout itself, because two runs must not copy into the same
//! directory at once.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use clean_checkout_acceptance::repo_root;

/// Optional override for the persistent cache location. The default lives under
/// the system temporary directory but outside each run's clean checkout.
const CACHE_ENV: &str = "RAG_CLEAN_CHECKOUT_CACHE";

fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
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
            copy_mtime(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Preserve the source modification time on the copy. Cargo's freshness check is
/// mtime-based, so a fresh copy with "now" timestamps would invalidate every
/// cached artifact even when the source is unchanged.
fn copy_mtime(src: &Path, dst: &Path) -> io::Result<()> {
    let modified = fs::metadata(src)?.modified()?;
    let file = fs::OpenOptions::new().write(true).open(dst)?;
    file.set_times(fs::FileTimes::new().set_modified(modified))
}

fn cargo_binary() -> PathBuf {
    match std::env::var_os("CARGO") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from("cargo"),
    }
}

/// Toolchain facts that make two artifact sets incompatible. `cargo -vV` reports
/// the release, host triple, and commit of the selected toolchain without
/// running a build.
struct ToolchainFacts {
    version: String,
    release: String,
    host: String,
    commit: String,
}

fn toolchain_facts(cargo: &Path) -> ToolchainFacts {
    let output = Command::new(cargo)
        .arg("-vV")
        .output()
        .unwrap_or_else(|err| panic!("probe cargo -vV: {err}"));
    let text = if output.status.success() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        String::new()
    };
    let mut facts = ToolchainFacts {
        version: text.lines().next().unwrap_or("cargo").trim().to_string(),
        release: String::new(),
        host: String::new(),
        commit: String::new(),
    };
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("release: ") {
            facts.release = value.trim().to_string();
        }
        if let Some(value) = line.strip_prefix("host: ") {
            facts.host = value.trim().to_string();
        }
        if let Some(value) = line.strip_prefix("commit-hash: ") {
            facts.commit = value.trim().to_string();
        }
    }
    facts
}

/// Restrict a value to a portable path component.
fn sanitize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed
    }
}

/// Stable FNV-1a digest used only for cache partitioning. It avoids a new
/// dependency while staying deterministic across runs and Rust versions.
fn fnv1a_hex(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Build-time environment that changes compiled artifacts. Values that are unset
/// or blank do not participate, so the default configuration has a stable key.
fn build_config_key() -> String {
    const KEYS: [&str; 9] = [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTC_WRAPPER",
        "CARGO_BUILD_TARGET",
        "CARGO_INCREMENTAL",
        "CC",
        "CFLAGS",
        "CXX",
        "CXXFLAGS",
    ];
    let mut parts = Vec::new();
    for key in KEYS {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                parts.push(format!("{key}={value}"));
            }
        }
    }
    if parts.is_empty() {
        "default".to_string()
    } else {
        fnv1a_hex(&parts.join("\n"))
    }
}

fn cache_root() -> PathBuf {
    if let Some(value) = std::env::var_os(CACHE_ENV) {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    std::env::temp_dir().join("rag-clean-checkout-cache")
}

/// The persistent partition for this toolchain, architecture, and build
/// configuration. The target directory and the temporary checkout are siblings
/// inside it, so removing the checkout can never remove the cache.
fn cache_partition(cargo: &Path) -> PathBuf {
    let facts = toolchain_facts(cargo);
    let release = if facts.release.is_empty() {
        facts
            .version
            .split_whitespace()
            .nth(1)
            .unwrap_or("unknown")
            .to_string()
    } else {
        facts.release
    };
    let host = if facts.host.is_empty() {
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
    } else {
        facts.host
    };
    let commit = if facts.commit.len() >= 8 {
        facts.commit[..8].to_string()
    } else {
        facts.commit
    };
    let build_target = std::env::var("CARGO_BUILD_TARGET")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| host.clone());

    let dir = cache_root()
        .join("v1")
        .join(format!(
            "{}-{}-{}",
            sanitize(&release),
            sanitize(&host),
            sanitize(&commit)
        ))
        .join(sanitize(&build_target))
        .join(build_config_key());
    fs::create_dir_all(&dir).expect("create the persistent cargo cache partition");
    dir
}

/// Serialize the shared source checkout. Cargo still locks the target directory
/// for concurrent builds; this lock only prevents two runs from copying into the
/// same checkout at once.
fn lock_checkout(partition: &Path) -> fs::File {
    let path = partition.join("checkout.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap_or_else(|err| panic!("open {}: {err}", path.display()));
    file.lock()
        .unwrap_or_else(|err| panic!("lock {}: {err}", path.display()));
    file
}

/// Recreate the clean checkout at a stable path and copy only the final Rust
/// package plus the assets its tests read. The web app and acceptance crates are
/// separate and are not needed for this gate. Source mtimes are preserved so an
/// unchanged tree stays fresh for Cargo.
fn prepare_checkout(partition: &Path, source: &Path) -> PathBuf {
    let copy = partition.join("checkout");
    if copy.exists() {
        fs::remove_dir_all(&copy).expect("remove the previous clean checkout");
    }
    fs::create_dir_all(&copy).expect("create the clean checkout");

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
            copy_mtime(&src, &dst).expect("preserve the copied file's modification time");
        }
    }
    copy
}

fn run(copy: &Path, cargo: &Path, path: &str, target_dir: &Path, args: &[&str]) -> String {
    let output = Command::new(cargo)
        .args(args)
        .current_dir(copy)
        .env("PATH", path)
        .env("CARGO_TARGET_DIR", target_dir)
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

fn modified_before(path: &Path, cutoff: SystemTime) -> usize {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .metadata()
                .and_then(|meta| meta.modified())
                .map(|when| when > cutoff)
                .unwrap_or(false)
        })
        .count()
}

#[test]
fn clean_checkout_builds_and_tests_without_go() {
    let source = repo_root();
    let cargo = cargo_binary();
    let partition = cache_partition(&cargo);
    let _lock = lock_checkout(&partition);
    let copy = prepare_checkout(&partition, &source);
    let target_dir = partition.join("target");
    fs::create_dir_all(&target_dir).expect("create the persistent cargo target directory");
    assert!(
        !target_dir.starts_with(&copy),
        "the persistent cargo target directory must live outside the temporary checkout"
    );

    assert!(
        copy.join("Cargo.toml").is_file(),
        "the clean checkout must contain the root Cargo.toml"
    );
    assert!(
        !copy.join("go.mod").exists(),
        "the clean checkout must not contain go.mod"
    );

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

    println!(
        "clean-checkout cargo target cache: {}",
        target_dir.display()
    );

    let started = SystemTime::now();
    let build = run(
        &copy,
        &cargo,
        &path,
        &target_dir,
        &["build", "--locked", "--manifest-path", "Cargo.toml"],
    );
    assert!(build.contains("Finished") || build.is_empty());
    assert!(
        target_dir.join("debug").is_dir(),
        "cargo must write artifacts into the persistent target directory: {}",
        target_dir.display()
    );

    run(
        &copy,
        &cargo,
        &path,
        &target_dir,
        &["test", "--locked", "--manifest-path", "Cargo.toml", "--lib"],
    );
    run(
        &copy,
        &cargo,
        &path,
        &target_dir,
        &[
            "test",
            "--locked",
            "--manifest-path",
            "Cargo.toml",
            "--test",
            "section_chunk_parity",
        ],
    );

    // Evidence for the operator: a warm run leaves most artifacts untouched.
    let rebuilt = modified_before(&target_dir.join("debug/deps"), started);
    println!("clean-checkout artifacts rewritten during this run: {rebuilt}");

    // Only the temporary checkout is removed. The cargo target directory is a
    // sibling, so cleanup can never remove the persistent cache.
    assert!(
        !target_dir.starts_with(&copy),
        "cleanup must never remove the persistent cargo cache"
    );
    fs::remove_dir_all(&copy).expect("remove the clean checkout");
}
