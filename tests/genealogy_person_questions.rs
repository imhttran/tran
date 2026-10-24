//! Synthetic PostgreSQL integration tests for person questions in "Ask the
//! archive".
//!
//! The suite runs against its own disposable genealogy database from
//! `tests/support/genealogy_isolation.rs`. It inserts synthetic people only and
//! never touches the RAG database or a live family catalog.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn, create_isolated_target};
use rag_foundation::genealogy::person_search::{
    extract_person_question, resolve, MatchKind, PersonResult,
};
use rag_foundation::genealogy::query::lookup_person;

const VERIFIED_EVIDENCE: &str = "synthetic-catalog.md";

/// Inserts one person with a display-name assertion. The optional `status`
/// controls the assertion's review status so the evidence badge is exercised.
///
/// The assertion starts unverified because the schema's deferred trigger
/// requires evidence at commit time for a verified row; evidence is inserted in
/// a separate statement before the status is raised.
fn insert_person(
    client: &mut postgres::Client,
    key: &str,
    name: &str,
    generation: i32,
    status: &str,
) {
    client
        .execute(
            "INSERT INTO person (person_key, display_name, generation) VALUES ($1, $2, $3)",
            &[&key, &name, &generation],
        )
        .expect("insert synthetic person");
    client
        .execute(
            "INSERT INTO assertion
                (person_id, assertion_kind, assertion_key, value_text, review_status)
             SELECT person_id, 'display_name', $2, $3, 'unverified'
               FROM person WHERE person_key = $1",
            &[&key, &format!("display:{key}"), &name],
        )
        .expect("insert display-name assertion");
    if status == "verified" {
        client
            .execute(
                "INSERT INTO assertion_evidence (assertion_id, document_ref, page)
                 SELECT a.assertion_id, $2, 3
                   FROM assertion a
                   JOIN person p ON p.person_id = a.person_id
                  WHERE p.person_key = $1 AND a.assertion_kind = 'display_name'",
                &[&key, &VERIFIED_EVIDENCE],
            )
            .expect("insert display-name evidence");
    }
    if status != "unverified" {
        client
            .execute(
                "UPDATE assertion a
                    SET review_status = $2,
                        reviewed_by = CASE WHEN $2::text = 'verified'
                                          THEN 'synthetic-reviewer' ELSE NULL END,
                        reviewed_at = CASE WHEN $2::text = 'verified'
                                          THEN now() ELSE NULL END
                   FROM person p
                  WHERE p.person_id = a.person_id
                    AND p.person_key = $1
                    AND a.assertion_kind = 'display_name'",
                &[&key, &status],
            )
            .expect("set display-name review status");
    }
}

fn insert_parent_link(
    client: &mut postgres::Client,
    parent_key: &str,
    child_key: &str,
    status: &str,
) {
    client
        .execute(
            "INSERT INTO parent_child_link (parent_id, child_id)
             SELECT parent.person_id, child.person_id
               FROM person parent, person child
              WHERE parent.person_key = $1 AND child.person_key = $2",
            &[&parent_key, &child_key],
        )
        .expect("insert synthetic parent link");
    client
        .execute(
            "INSERT INTO assertion
                (person_id, assertion_kind, assertion_key, value_text, object_person_id,
                 review_status)
             SELECT parent.person_id, 'parent_child', $3, $2, child.person_id, $4
               FROM person parent, person child
              WHERE parent.person_key = $1 AND child.person_key = $2",
            &[
                &parent_key,
                &child_key,
                &format!("parent_child:{parent_key}:{child_key}"),
                &status,
            ],
        )
        .expect("insert synthetic parent assertion");
}

fn seed(client: &mut postgres::Client) {
    insert_person(client, "P013", "Trần Đức Hân", 3, "verified");
    // A branch assertion gives the found answer something beyond the name.
    client
        .execute(
            "INSERT INTO assertion (person_id, assertion_kind, assertion_key, value_text,
                                    review_status)
             SELECT person_id, 'branch', 'branch:third', 'Third branch', 'unverified'
               FROM person WHERE person_key = 'P013'",
            &[],
        )
        .expect("insert synthetic branch assertion");
    insert_person(client, "P021", "Trần Thị Huyền", 4, "verified");
    insert_person(client, "P022", "Lê Thanh Phong", 4, "needs_review");
    insert_person(client, "P023", "Lê Thanh Phong", 4, "unverified");
    // Only the name is recorded, so the answer must not invent a biography.
    insert_person(client, "P030", "Phạm Minh Châu", 5, "unverified");
    // A recorded English alias for a Vietnamese name.
    insert_person(client, "P040", "Nguyễn Văn Bình", 3, "unverified");
    client
        .execute(
            "INSERT INTO assertion (person_id, assertion_kind, assertion_key, value_text,
                                    review_status)
             SELECT person_id, 'name_variant', 'variant:binh', 'Binh Nguyen', 'unverified'
               FROM person WHERE person_key = 'P040'",
            &[],
        )
        .expect("insert synthetic alias assertion");
    insert_person(client, "P100", "Nguyễn Thị Mai", 3, "unverified");
    insert_person(client, "P101", "Trần Thị Lan", 3, "unverified");
    // The two namesakes stay distinguishable only by their recorded mothers.
    insert_parent_link(client, "P100", "P021", "needs_review");
    insert_parent_link(client, "P101", "P023", "unverified");
}

fn resolve_named(client: &mut postgres::Client, question: &str) -> PersonResult {
    let extracted = extract_person_question(question).expect("question names a person");
    resolve(client, Some(&extracted), None).expect("resolve person question")
}

#[test]
fn person_questions_answer_from_stored_assertions_only() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping person-question test");
        return;
    };
    let target = create_isolated_target(&base, "person_questions");
    target.apply_migrations();
    let mut client = target.connect();
    seed(&mut client);

    // The reported failure: a Western name order plus accent folding finds the
    // Vietnamese catalog spelling.
    let found = resolve_named(&mut client, "Who is Han Duc Tran");
    let PersonResult::Found {
        candidate,
        detail,
        name_only,
        ..
    } = found
    else {
        panic!("expected a single person for the reported name variation");
    };
    assert_eq!(candidate.person_key, "P013");
    assert_eq!(candidate.display_name, "Trần Đức Hân");
    assert_eq!(candidate.match_kind, Some(MatchKind::Reordered));
    assert_eq!(candidate.evidence_status, "verified");
    assert!(!name_only);
    assert_eq!(detail.display_name.as_deref(), Some("Trần Đức Hân"));
    let display = detail
        .assertions
        .iter()
        .find(|assertion| assertion.kind == "display_name")
        .expect("display-name assertion");
    assert_eq!(display.review_status, "verified");
    assert_eq!(display.citations.len(), 1);
    assert_eq!(display.citations[0].document_ref, VERIFIED_EVIDENCE);
    assert_eq!(display.citations[0].page, 3);

    // A recorded alias is recognized and reported as an alias.
    let found = resolve_named(&mut client, "Who is Binh Nguyen");
    let PersonResult::Found { candidate, .. } = found else {
        panic!("expected a single alias match");
    };
    assert_eq!(candidate.person_key, "P040");
    assert_eq!(candidate.match_kind, Some(MatchKind::Alias));

    // Two namesakes must require a human choice and stay separate.
    let ambiguous = resolve_named(&mut client, "Who is Le Thanh Phong");
    let PersonResult::Ambiguous {
        candidates, more, ..
    } = ambiguous
    else {
        panic!("namesakes must not be merged");
    };
    assert!(!more);
    assert_eq!(candidates.len(), 2);
    let keys: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.person_key.as_str())
        .collect();
    assert_eq!(keys, vec!["P022", "P023"]);
    let mothers: Vec<Vec<&str>> = candidates
        .iter()
        .map(|candidate| {
            candidate
                .recorded_parents
                .iter()
                .map(|parent| parent.display_name.as_str())
                .collect()
        })
        .collect();
    assert_eq!(mothers, vec![vec![], vec!["Trần Thị Lan"]]);

    // Selecting a candidate answers exactly that person.
    let selected = resolve(&mut client, None, Some("P022")).expect("resolve selected person");
    let PersonResult::Found {
        candidate, detail, ..
    } = selected
    else {
        panic!("expected the selected person");
    };
    assert_eq!(candidate.person_key, "P022");
    assert_eq!(detail.assertions.len(), 1);
    assert_eq!(detail.assertions[0].review_status, "needs_review");

    // A person with only a name assertion reports name-only evidence.
    let found = resolve_named(&mut client, "Who is Minh Chau Pham");
    let PersonResult::Found {
        candidate,
        name_only,
        ..
    } = found
    else {
        panic!("expected the name-only person");
    };
    assert_eq!(candidate.person_key, "P030");
    assert!(name_only);

    // No match in a populated catalog is distinct from an empty catalog.
    let none = resolve_named(&mut client, "Who is Quang Vinh");
    let PersonResult::NoMatch {
        catalog_empty,
        catalog_truncated,
        ..
    } = none
    else {
        panic!("an unknown name must not match");
    };
    assert!(!catalog_empty);
    assert!(!catalog_truncated);

    // A stale selection key is reported as no match, not as another person.
    let stale = resolve(&mut client, None, Some("P999")).expect("resolve stale key");
    assert!(matches!(stale, PersonResult::NoMatch { .. }));

    // Ordinary archive questions keep the document path.
    assert!(extract_person_question("Where did the family live?").is_none());
    assert!(matches!(
        resolve(&mut client, None, None).expect("resolve without a person"),
        PersonResult::NotPersonQuestion
    ));
    // A bare name that matches nothing must not hijack the document path.
    assert!(matches!(
        resolve_named(&mut client, "Alpha Beta"),
        PersonResult::NotPersonQuestion
    ));
    // A broader topic phrasing without a matching person keeps the document path.
    assert!(matches!(
        resolve_named(&mut client, "Tell me about Alpha Beta"),
        PersonResult::NotPersonQuestion
    ));

    // The lookup is read-only: repeated runs leave the store untouched.
    let before: i64 = client
        .query_one("SELECT count(*) FROM assertion", &[])
        .expect("count assertions")
        .get(0);
    let first = resolve_named(&mut client, "Who is Han Duc Tran");
    let second = resolve_named(&mut client, "Who is Han Duc Tran");
    let after: i64 = client
        .query_one("SELECT count(*) FROM assertion", &[])
        .expect("count assertions")
        .get(0);
    assert_eq!(before, after, "person questions must never write");
    let PersonResult::Found {
        candidate: first_candidate,
        ..
    } = first
    else {
        panic!("expected the first answer");
    };
    let PersonResult::Found {
        candidate: second_candidate,
        ..
    } = second
    else {
        panic!("expected the second answer");
    };
    assert_eq!(first_candidate.person_key, second_candidate.person_key);
    assert_eq!(
        lookup_person(&mut client, "P013")
            .expect("lookup person")
            .expect("person exists")
            .assertions
            .len(),
        2,
        "the lookup must not create assertions"
    );

    drop(client);
    target.cleanup();
}

#[test]
fn an_empty_catalog_is_reported_as_an_operator_condition() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping empty-catalog test");
        return;
    };
    let target = create_isolated_target(&base, "person_empty");
    target.apply_migrations();
    let mut client = target.connect();

    let result = resolve_named(&mut client, "Who is Han Duc Tran");
    let PersonResult::NoMatch {
        catalog_empty,
        query_name,
        ..
    } = result
    else {
        panic!("an empty catalog cannot produce a person");
    };
    assert!(catalog_empty);
    assert_eq!(query_name, "Han Duc Tran");

    drop(client);
    target.cleanup();
}
