//! Synthetic Head of Family workflow tests (TRAN-003E).
//!
//! Uses a dedicated disposable genealogy target derived from
//! `GENEALOGY_DATABASE_URL` through `tests/support/genealogy_isolation.rs`.
//! Each suite owns its own uniquely named target and drops only that target.
//! It never touches real family data.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use std::thread;

use genealogy_isolation::{base_dsn, create_isolated_target, IsolatedTarget};
use postgres::{Client, NoTls};
use rag_foundation::genealogy::appointments;
use rag_foundation::genealogy::eligibility;
use rag_foundation::genealogy::GenealogyError;

const BRANCH: &str = "branch-3";

/// Create and migrate a uniquely named target owned by this test run.
fn prepare(label: &str) -> Option<IsolatedTarget> {
    let base = base_dsn()?;
    let target = create_isolated_target(&base, label);
    target.apply_migrations();
    Some(target)
}

fn seed_person(client: &mut Client, key: &str, branch: &str, living: bool) {
    client
        .execute(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2)",
            &[&key, &format!("Synthetic {key}")],
        )
        .expect("insert person");
    client
        .execute(
            "INSERT INTO source_document (relative_path, sha256, source_kind) \
             VALUES ($1, $2, 'original')",
            &[&format!("synthetic/{key}.pdf"), &"a".repeat(64)],
        )
        .expect("insert source document");
    let source_evidence_id: i64 = client
        .query_one(
            "INSERT INTO source_evidence (source_document_id, page, locator) \
             SELECT source_document_id, 1, 'p.1' FROM source_document \
              WHERE relative_path = $1 RETURNING source_evidence_id",
            &[&format!("synthetic/{key}.pdf")],
        )
        .expect("insert source evidence")
        .get(0);

    let mut assertions = vec![
        ("branch_claim", "branch", branch),
        ("gender", "gender", "male"),
        ("birth_order", "order", "first"),
    ];
    if living {
        assertions.push(("vital_status", "living", "living"));
    }

    for (kind, assertion_key, value) in assertions {
        let assertion_id: i64 = client
            .query_one(
                "INSERT INTO assertion \
                 (person_id, assertion_kind, assertion_key, value_text, review_status) \
                 SELECT person_id, $2, $3, $4, 'unverified' FROM person \
                  WHERE person_key = $1 RETURNING assertion_id",
                &[&key, &kind, &assertion_key, &value],
            )
            .expect("insert assertion")
            .get(0);
        client
            .execute(
                "INSERT INTO assertion_evidence \
                 (assertion_id, source_evidence_id, document_ref, page) \
                 VALUES ($1, $2, $3, 1)",
                &[
                    &assertion_id,
                    &source_evidence_id,
                    &format!("synthetic/{key}.pdf"),
                ],
            )
            .expect("insert evidence");
        client
            .execute(
                "UPDATE assertion SET review_status = 'verified', \
                    reviewed_by = 'synthetic-reviewer', reviewed_at = now() \
                  WHERE assertion_id = $1",
                &[&assertion_id],
            )
            .expect("verify assertion");
    }
}

#[test]
fn eligible_candidate_can_be_nominated_approved_and_revoked() {
    let Some(target) = prepare("appointments_approval") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping appointment test");
        return;
    };
    let mut client = target.connect();
    seed_person(&mut client, "synthetic-first", BRANCH, true);

    let input = eligibility::load_input(&mut client, "synthetic-first", BRANCH).unwrap();
    assert!(eligibility::decide(&input).is_eligible());

    let nomination =
        appointments::nominate(&mut client, "synthetic-first", BRANCH, "editor").expect("nominate");
    let appointment =
        appointments::approve(&mut client, nomination, BRANCH, "admin").expect("approve");
    assert_eq!(appointment.person_key, "synthetic-first");

    let active = appointments::list_active_appointments(&mut client).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].person_key, "synthetic-first");

    appointments::end_appointment(
        &mut client,
        appointment.appointment_id,
        "admin",
        "review",
        false,
    )
    .expect("revoke");
    assert!(appointments::list_active_appointments(&mut client)
        .unwrap()
        .is_empty());

    drop(client);
    target.cleanup();
}

#[test]
fn active_appointment_is_revalidated_when_living_evidence_is_withdrawn() {
    let Some(target) = prepare("appointments_revalidate") else {
        return;
    };
    let mut client = target.connect();
    seed_person(&mut client, "synthetic-later", BRANCH, true);

    let nomination =
        appointments::nominate(&mut client, "synthetic-later", BRANCH, "editor").unwrap();
    appointments::approve(&mut client, nomination, BRANCH, "admin").expect("approve");
    let active = appointments::list_active_appointments(&mut client).unwrap();
    assert_eq!(
        active[0].current_eligibility,
        eligibility::ReasonCode::Eligible
    );

    // Later review disputes the living-status evidence (for example, a death
    // record surfaces). The stored appointment stays active until an
    // administrator ends it, but it must no longer read as eligible.
    client
        .execute(
            "UPDATE assertion SET review_status = 'disputed' \
              WHERE assertion_kind = 'vital_status' AND person_id = \
                    (SELECT person_id FROM person WHERE person_key = 'synthetic-later')",
            &[],
        )
        .expect("dispute living evidence");

    let active = appointments::list_active_appointments(&mut client).unwrap();
    assert_eq!(active.len(), 1, "no appointment is silently ended");
    assert_eq!(
        active[0].current_eligibility,
        eligibility::ReasonCode::MissingLivingEvidence
    );

    drop(client);
    target.cleanup();
}

#[test]
fn nominator_cannot_approve_their_own_nomination() {
    let Some(target) = prepare("appointments_self_approval") else {
        return;
    };
    let mut client = target.connect();
    seed_person(&mut client, "synthetic-self", BRANCH, true);

    let nomination =
        appointments::nominate(&mut client, "synthetic-self", BRANCH, "admin").unwrap();
    let err = appointments::approve(&mut client, nomination, BRANCH, "admin").unwrap_err();
    match err {
        GenealogyError::Domain(message) => assert!(message.contains("nominator"), "{message}"),
        other => panic!("unexpected error: {other}"),
    }
    assert!(appointments::list_active_appointments(&mut client)
        .unwrap()
        .is_empty());
    appointments::approve(&mut client, nomination, BRANCH, "other-admin").expect("approve");

    drop(client);
    target.cleanup();
}

#[test]
fn ineligible_candidate_is_rejected_without_an_appointment() {
    let Some(target) = prepare("appointments_ineligible") else {
        return;
    };
    let mut client = target.connect();
    seed_person(&mut client, "synthetic-no-living", BRANCH, false);

    let nomination =
        appointments::nominate(&mut client, "synthetic-no-living", BRANCH, "editor").unwrap();
    let err = appointments::approve(&mut client, nomination, BRANCH, "admin").unwrap_err();
    match err {
        GenealogyError::Domain(message) => {
            assert!(message.contains("missing_living_evidence"), "{message}");
        }
        other => panic!("unexpected error: {other}"),
    }
    assert!(appointments::list_active_appointments(&mut client)
        .unwrap()
        .is_empty());

    drop(client);
    target.cleanup();
}

#[test]
fn concurrent_approvals_leave_at_most_one_active_appointment() {
    let Some(target) = prepare("appointments_concurrent") else {
        return;
    };
    let mut client = target.connect();
    seed_person(&mut client, "synthetic-a", BRANCH, true);
    seed_person(&mut client, "synthetic-b", BRANCH, true);

    let first = appointments::nominate(&mut client, "synthetic-a", BRANCH, "editor").unwrap();
    let second = appointments::nominate(&mut client, "synthetic-b", BRANCH, "editor").unwrap();

    let dsn = target.dsn.clone();
    let results = thread::scope(|scope| {
        let first_dsn = dsn.clone();
        let second_dsn = dsn.clone();
        let one = scope.spawn(move || {
            let mut client = Client::connect(&first_dsn, NoTls).expect("connect a");
            appointments::approve(&mut client, first, BRANCH, "admin-a").is_ok()
        });
        let two = scope.spawn(move || {
            let mut client = Client::connect(&second_dsn, NoTls).expect("connect b");
            appointments::approve(&mut client, second, BRANCH, "admin-b").is_ok()
        });
        (one.join().unwrap(), two.join().unwrap())
    });

    assert_eq!(
        results.0 as i32 + results.1 as i32,
        1,
        "exactly one concurrent approval must commit"
    );
    assert_eq!(
        appointments::list_active_appointments(&mut client)
            .unwrap()
            .len(),
        1
    );

    drop(client);
    target.cleanup();
}
