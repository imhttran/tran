//! TRAN-001A end-to-end test for the `inventory` binary.
//!
//! Builds a temporary source tree with nested files, a duplicate pair, and a
//! PDF. Runs the real binary, then re-hashes every source file and compares the
//! result with the written manifest. Also asserts the sources are unchanged
//! after the run and that the exit codes are 0 (clean) and 1 (missing file).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rag_foundation::inventory::sha256_hex;

fn inventory_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_inventory"))
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, bytes).unwrap();
}

/// Reads every regular file under `root` and returns a relative-path -> hash map.
fn hash_tree(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.file_type().is_dir() {
                stack.push(path);
            } else if meta.file_type().is_file() {
                let rel = path.strip_prefix(root).unwrap();
                let key = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(key, sha256_hex(&fs::read(&path).unwrap()));
            }
        }
    }
    out
}

fn build_tree(root: &Path) {
    write(root, "a/first.md", b"# Title\n\nbody\n");
    write(root, "a/b/second.txt", b"second body\n");
    write(root, "dup-one.txt", b"identical content\n");
    write(root, "nested/dup-two.txt", b"identical content\n");
    write(root, "weird.xyz", b"opaque\n");
    write(root, "empty.bin", b"");
    let pdf = minimal_pdf();
    write(root, "doc/one-page.pdf", &pdf);
}

/// Returns a valid single-page PDF with an empty content stream.
fn minimal_pdf() -> Vec<u8> {
    let objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << >> >>"
            .to_string(),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
    ];

    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{}\nendobj\n", index + 1, body));
    }
    let xref_pos = out.len();
    out.push_str(&format!("xref\n0 {}\n", objects.len() + 1));
    out.push_str("0000000000 65535 f \n");
    for offset in &offsets {
        out.push_str(&format!("{:010} 00000 n \n", offset));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        objects.len() + 1,
        xref_pos
    ));
    out.into_bytes()
}

fn read_manifest(path: &Path) -> serde_json::Value {
    let text = fs::read_to_string(path).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn manifest_hashes_match_sources_and_sources_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("sources");
    let manifest_path = tmp.path().join("out").join("inventory.json");
    fs::create_dir_all(&root).unwrap();
    build_tree(&root);

    let before = hash_tree(&root);

    let output = Command::new(inventory_bin())
        .arg(&root)
        .arg(&manifest_path)
        .output()
        .expect("run inventory binary");
    assert!(
        output.status.success(),
        "clean tree must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after = hash_tree(&root);
    assert_eq!(before, after, "no source file may change after the run");

    let manifest = read_manifest(&manifest_path);
    let records = manifest["records"].as_array().unwrap();

    assert_eq!(
        records.len(),
        before.len(),
        "record count must equal discovered file count"
    );

    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for record in records {
        let rel = record["relative_path"].as_str().unwrap().to_string();
        let size = record["size_bytes"].as_u64().unwrap();
        let sha = record["sha256"].as_str().unwrap().to_string();

        let on_disk = fs::read(root.join(&rel)).unwrap();
        assert_eq!(size, on_disk.len() as u64, "size mismatch for {rel}");
        assert_eq!(sha, sha256_hex(&on_disk), "manifest hash differs for {rel}");
        assert!(seen.insert(rel, sha).is_none(), "duplicate record");
    }
    let seen_keys: Vec<String> = seen.keys().cloned().collect();
    let before_keys: Vec<String> = before.keys().cloned().collect();
    assert_eq!(seen_keys, before_keys);

    let pdf = records
        .iter()
        .find(|r| r["relative_path"] == "doc/one-page.pdf")
        .unwrap();
    assert_eq!(pdf["format"], "pdf");
    assert_eq!(pdf["pdf_page_count"], 1);

    let duplicates = manifest["duplicates"].as_array().unwrap();
    assert!(
        duplicates.iter().any(|group| {
            let paths: Vec<&str> = group
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap())
                .collect();
            paths == vec!["dup-one.txt", "nested/dup-two.txt"]
        }),
        "duplicate group missing: {duplicates:?}"
    );
}

#[test]
fn unreadable_source_file_exits_one_and_is_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("sources");
    let manifest_path = tmp.path().join("inventory.json");
    fs::create_dir_all(&root).unwrap();
    write(&root, "present.txt", b"here\n");
    write(&root, "unreadable.txt", b"x");

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    {
        let path = root.join("unreadable.txt");
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&path, perms).unwrap();
    }

    let output = Command::new(inventory_bin())
        .arg(&root)
        .arg(&manifest_path)
        .output()
        .expect("run inventory binary");

    #[cfg(unix)]
    {
        assert_eq!(
            output.status.code(),
            Some(1),
            "unreadable source must exit 1; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let manifest = read_manifest(&manifest_path);
        let bad = manifest["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["relative_path"] == "unreadable.txt")
            .unwrap();
        assert!(bad["sha256"].is_null());
    }
    #[cfg(not(unix))]
    {
        let _ = output;
    }
}

#[test]
fn rejects_manifest_under_frozen_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("sources");
    fs::create_dir_all(&root).unwrap();
    write(&root, "a.txt", b"a\n");

    let manifest = tmp.path().join("fixtures").join("manifest.json");
    let output = Command::new(inventory_bin())
        .current_dir(tmp.path())
        .arg(&root)
        .arg(&manifest)
        .output()
        .expect("run inventory binary");
    assert_eq!(output.status.code(), Some(2));
    assert!(!manifest.exists(), "must not write under frozen dir");
}
