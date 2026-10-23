//! Synthetic CLI integration tests for TRAN-002E.
//!
//! The test creates and uses a dedicated disposable target derived from
//! `GENEALOGY_DATABASE_URL` through `tests/support/genealogy_isolation.rs`.
//! It never touches the RAG database and drops only the target it created.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use std::fs;
use std::process::Command;

use genealogy_isolation::{base_dsn, create_isolated_target};

const SYNTHETIC_CATALOG: &str = "\
1. Synthetic Alpha\n   Trang: 3\n2. Synthetic Beta\n   Trang: 4\n";

fn run_cli(dsn: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_genealogy"))
        .args(args)
        .env("GENEALOGY_DATABASE_URL", dsn)
        .output()
        .expect("run genealogy CLI")
}

#[test]
fn apply_is_idempotent_and_queries_never_invent_relationships() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping genealogy CLI test");
        return;
    };
    let target = create_isolated_target(&base, "cli");
    target.apply_migrations();
    let cli_dsn = target.dsn.clone();

    let mut client = target.connect();

    let temp = tempfile::tempdir().expect("temp dir");
    let catalog = temp.path().join("catalog.md");
    let report = temp.path().join("unresolved.json");
    fs::write(&catalog, SYNTHETIC_CATALOG).expect("write synthetic catalog");
    let catalog_arg = catalog.to_str().expect("utf-8 path");
    let report_arg = report.to_str().expect("utf-8 path");

    for _ in 0..2 {
        let output = run_cli(&cli_dsn, &["import", "--apply", catalog_arg]);
        assert!(
            output.status.success(),
            "apply must succeed; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let person_key: String = client
        .query_one(
            "SELECT person_key FROM person WHERE display_name = 'Synthetic Alpha'",
            &[],
        )
        .expect("synthetic person exists")
        .get(0);
    let assertions: i64 = client
        .query_one(
            "SELECT count(*) FROM assertion a
               JOIN person p ON p.person_id = a.person_id
              WHERE p.person_key = $1",
            &[&person_key],
        )
        .expect("count assertions")
        .get(0);
    assert!(assertions > 0, "import must create candidate assertions");
    let people: i64 = client
        .query_one(
            "SELECT count(*) FROM person WHERE person_key = $1",
            &[&person_key],
        )
        .expect("count people")
        .get(0);
    assert_eq!(people, 1, "re-import must not duplicate people");
    let assertion_rows: i64 = client
        .query_one("SELECT count(*) FROM assertion", &[])
        .expect("count all assertions")
        .get(0);
    assert_eq!(
        assertions, 1,
        "each synthetic person has one display-name assertion"
    );
    assert_eq!(
        assertion_rows, 2,
        "re-import must not duplicate assertions across both synthetic people"
    );

    let output = run_cli(&cli_dsn, &["person", &person_key]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("person: {person_key}")));
    assert!(stdout.contains("status=unverified"));

    let output = run_cli(&cli_dsn, &["relationship", &person_key, "missing-person"]);
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("status=unknown"),
        "insufficient evidence must not produce a relationship"
    );

    let output = run_cli(&cli_dsn, &["unresolved", report_arg]);
    assert!(output.status.success());
    let written = fs::read_to_string(report).expect("unresolved report exists");
    assert!(written.contains("\"entries\""));

    drop(client);
    target.cleanup();
}
