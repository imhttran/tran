use evidence::{has_sha40, has_url, load_evidence, section};

const REQUIRED: &[&str] = &[
    "Capability matrix",
    "Remaining limitations",
    "Rollback reference",
    "Pinned Go commit SHA",
    "Source repository URL",
    "Fixture provenance",
];

#[test]
fn required_sections_exist_and_are_non_empty() {
    let doc = load_evidence();
    for heading in REQUIRED {
        let body = section(&doc, heading).unwrap_or_else(|| panic!("missing section: {heading}"));
        assert!(!body.trim().is_empty(), "empty section: {heading}");
    }
}

#[test]
fn pinned_go_commit_sha_is_40_hex() {
    let doc = load_evidence();
    let body = section(&doc, "Pinned Go commit SHA").expect("pinned SHA section");
    assert!(has_sha40(body), "no 40-hex SHA in pinned section");
}

#[test]
fn source_repository_url_present() {
    let doc = load_evidence();
    let body = section(&doc, "Source repository URL").expect("source URL section");
    assert!(has_url(body), "no URL in source repository section");
}

#[test]
fn rollback_reference_names_go_restore() {
    let doc = load_evidence();
    let body = section(&doc, "Rollback reference").expect("rollback section");
    assert!(
        body.to_lowercase().contains("go"),
        "rollback must describe restoring the Go reference"
    );
}

#[test]
fn frozen_assets_have_provenance_entries() {
    let doc = load_evidence();
    let body = section(&doc, "Fixture provenance").expect("provenance section");
    for asset in [
        "migrations/",
        "evals/",
        "examples/",
        "docker-compose.yml",
        "fixtures/",
    ] {
        assert!(body.contains(asset), "provenance missing asset: {asset}");
    }
}
