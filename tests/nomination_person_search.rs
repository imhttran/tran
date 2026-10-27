//! Nomination person search: authorization, branch labels, and full-catalog reach.
//!
//! Synthetic fixtures only. Each suite gets a disposable genealogy database
//! derived from `GENEALOGY_DATABASE_URL`.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use std::collections::BTreeSet;

use genealogy_isolation::{base_dsn, create_isolated_target, IsolatedTarget};
use postgres::Client;

use rag_foundation::genealogy::appointments;
use rag_foundation::genealogy::query::list_records;
use rag_foundation::identity::{decide, Decision, Operation, Role, VerifiedIdentity};

fn target() -> Option<IsolatedTarget> {
    let base = base_dsn()?;
    let target = create_isolated_target(&base, "nomination-person-search");
    target.apply_migrations();
    Some(target)
}

fn identity(role: Role) -> VerifiedIdentity {
    VerifiedIdentity::new(
        "synthetic-subject",
        "https://synthetic-issuer",
        vec![role],
        vec![],
    )
}

fn seed_person(client: &mut Client, key: &str, name: &str) -> i64 {
    client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2) \
             RETURNING person_id",
            &[&key, &name],
        )
        .expect("seed person")
        .get(0)
}

fn seed_branch_claim(
    client: &mut Client,
    person_id: i64,
    assertion_key: &str,
    value_text: &str,
    review_status: &str,
    superseded: bool,
) {
    let assertion_id: i64 = client
        .query_one(
            "INSERT INTO assertion \
                 (person_id, assertion_kind, assertion_key, value_text, review_status) \
             VALUES ($1, 'branch_claim', $2, $3, 'unverified') \
             RETURNING assertion_id",
            &[&person_id, &assertion_key, &value_text],
        )
        .expect("seed branch claim")
        .get(0);
    client
        .execute(
            "INSERT INTO assertion_evidence (assertion_id, document_ref, page) \
             VALUES ($1, 'synthetic/nomination.pdf', 1)",
            &[&assertion_id],
        )
        .expect("seed branch claim evidence");
    if review_status == "verified" {
        client
            .execute(
                "UPDATE assertion \
                    SET review_status = 'verified', reviewed_by = 'synthetic-reviewer', \
                        reviewed_at = now(), \
                        superseded_at = CASE WHEN $2 THEN now() END \
                  WHERE assertion_id = $1",
                &[&assertion_id, &superseded],
            )
            .expect("verify branch claim");
    }
}

#[test]
fn nomination_person_search_requires_nomination_permission() {
    assert_eq!(
        decide(None, Operation::GenealogyNominate),
        Decision::Unauthenticated
    );
    assert_eq!(
        decide(Some(&identity(Role::Viewer)), Operation::GenealogyNominate),
        Decision::Forbidden
    );
    assert_eq!(
        decide(Some(&identity(Role::Editor)), Operation::GenealogyNominate),
        Decision::Allow
    );
    assert_eq!(
        decide(Some(&identity(Role::Admin)), Operation::GenealogyNominate),
        Decision::Allow
    );

    let source = include_str!("../src/api.rs");
    assert!(source.contains("/api/genealogy/nomination-persons"));
    let handler = source
        .split("async fn nomination_people")
        .nth(1)
        .expect("nomination_people handler");
    let handler = &handler[..handler.find("async fn").unwrap_or(handler.len())];
    assert!(handler.contains("Operation::GenealogyNominate"));
}

#[test]
fn nomination_search_reaches_past_the_first_hundred_and_labels_current_branches() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is not set; skipping the database test");
        return;
    };
    let mut client = target.connect();

    // The old dropdown read the first 100 keys. These fillers sort before the
    // target so the target is beyond that page.
    client
        .execute(
            "INSERT INTO person (person_key, display_name)
             SELECT 'aaa-filler-' || n, 'Synthetic Filler ' || n
               FROM generate_series(1, 105) n",
            &[],
        )
        .expect("seed initial page");
    let person_id = seed_person(
        &mut client,
        "synthetic-nomination-target",
        "Synthetic Nomination Target",
    );

    client
        .execute(
            "INSERT INTO family_branch (branch_key, branch_label) VALUES
                 ('synthetic-branch-visible', 'Synthetic Branch Visible'),
                 ('synthetic-branch-unverified', 'Synthetic Branch Unverified'),
                 ('synthetic-branch-superseded', 'Synthetic Branch Superseded')",
            &[],
        )
        .expect("seed family branches");
    seed_branch_claim(
        &mut client,
        person_id,
        "branch-visible",
        "synthetic-branch-visible",
        "verified",
        false,
    );
    seed_branch_claim(
        &mut client,
        person_id,
        "branch-unverified",
        "synthetic-branch-unverified",
        "unverified",
        false,
    );
    seed_branch_claim(
        &mut client,
        person_id,
        "branch-superseded",
        "synthetic-branch-superseded",
        "verified",
        true,
    );
    seed_branch_claim(
        &mut client,
        person_id,
        "branch-missing",
        "synthetic-branch-missing",
        "verified",
        false,
    );

    let first_page = list_records(&mut client, 100, 0).expect("initial catalog page");
    assert!(!first_page
        .iter()
        .any(|person| person.person_key == "synthetic-nomination-target"));

    let people = appointments::search_nomination_people(&mut client, "synthetic nomination", 10)
        .expect("nomination search");
    let found = people
        .iter()
        .find(|person| person.person_key == "synthetic-nomination-target")
        .expect("the endpoint-level search returns a person beyond the first 100 keys");
    assert_eq!(found.display_name, "Synthetic Nomination Target");
    let branches: BTreeSet<&str> = found.branches.iter().map(String::as_str).collect();
    assert_eq!(
        branches,
        BTreeSet::from(["Synthetic Branch Visible", "synthetic-branch-missing"])
    );
}

#[test]
fn pending_nominations_carry_the_candidate_label() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is not set; skipping the database test");
        return;
    };
    let mut client = target.connect();

    seed_person(
        &mut client,
        "synthetic-nomination-target",
        "Synthetic Nomination Target",
    );
    let known: i64 = client
        .query_one(
            "INSERT INTO head_of_family_nomination
                 (person_key, branch_key, nominated_by, status)
             VALUES ('synthetic-nomination-target', 'synthetic-branch-visible',
                     'synthetic-nominator', 'pending')
             RETURNING nomination_id",
            &[],
        )
        .expect("seed nomination")
        .get(0);
    let missing: i64 = client
        .query_one(
            "INSERT INTO head_of_family_nomination
                 (person_key, branch_key, nominated_by, status)
             VALUES ('synthetic-missing-person', 'synthetic-branch-visible',
                     'synthetic-nominator', 'pending')
             RETURNING nomination_id",
            &[],
        )
        .expect("seed unknown-person nomination")
        .get(0);

    let pending = appointments::list_pending_nominations(&mut client).expect("pending");
    let known_row = pending
        .iter()
        .find(|row| row.nomination_id == known)
        .expect("known-person nomination");
    assert_eq!(known_row.candidate_label, "Synthetic Nomination Target");
    let missing_row = pending
        .iter()
        .find(|row| row.nomination_id == missing)
        .expect("unknown-person nomination");
    assert_eq!(missing_row.candidate_label, "synthetic-missing-person");
}
