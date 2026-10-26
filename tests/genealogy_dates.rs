//! TRAN-002H: explicit birth and death dates.
//!
//! These suites use a disposable genealogy database derived from
//! `GENEALOGY_DATABASE_URL`. Every fixture is synthetic: no real family name,
//! date, or source text appears here.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn, create_isolated_target, IsolatedTarget};
use postgres::Client;
use rag_foundation::genealogy::dates::{DatePrecision, DateQualifier, DateValue};
use rag_foundation::genealogy::query;
use rag_foundation::genealogy::reimport::{self, ImportAssertion, ImportDocument, ImportPerson};
use rag_foundation::genealogy::review::{self, ReviewError, ReviewRequest, ReviewStatus};
use rag_foundation::genealogy::tree;

const ORIGINAL: &str = "synthetic/original.pdf";
const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn isolated_target(label: &str) -> Option<IsolatedTarget> {
    let base = base_dsn()?;
    let target = create_isolated_target(&base, label);
    target.apply_migrations();
    Some(target)
}

fn date(
    start: &str,
    end: Option<&str>,
    precision: DatePrecision,
    qualifier: DateQualifier,
    raw: &str,
) -> DateValue {
    DateValue {
        start: start.to_string(),
        end: end.map(str::to_string),
        precision,
        qualifier,
        raw: raw.to_string(),
    }
}

fn birth_assertion(value: DateValue) -> ImportAssertion {
    ImportAssertion {
        person_key: "synthetic-date-person".to_string(),
        kind: "birth_date".to_string(),
        key: format!("birth_date:{}", value.start),
        value_text: Some(value.raw.clone()),
        object_key: None,
        evidence: vec![(ORIGINAL.to_string(), 7)],
        date: Some(value),
    }
}

fn death_assertion(value: DateValue) -> ImportAssertion {
    ImportAssertion {
        person_key: "synthetic-date-person".to_string(),
        kind: "death_date".to_string(),
        key: format!("death_date:{}", value.start),
        value_text: Some(value.raw.clone()),
        object_key: None,
        evidence: vec![(ORIGINAL.to_string(), 7)],
        date: Some(value),
    }
}

fn document(relative_path: &str, sha: &str, assertions: Vec<ImportAssertion>) -> ImportDocument {
    ImportDocument {
        relative_path: relative_path.to_string(),
        sha256: sha.to_string(),
        source_kind: "original".to_string(),
        persons: vec![ImportPerson {
            person_key: "synthetic-date-person".to_string(),
            display_name: "Synthetic Date Person".to_string(),
        }],
        assertions,
        links: Vec::new(),
    }
}

fn reimport(client: &mut Client, assertions: Vec<ImportAssertion>) -> reimport::ReimportReport {
    let document = document(ORIGINAL, SHA, assertions);
    reimport::reimport_document(client, &document).expect("reimport")
}

fn assertion_id(client: &mut Client, kind: &str, key: &str) -> i64 {
    client
        .query_one(
            "SELECT a.assertion_id
               FROM assertion a
               JOIN person p ON p.person_id = a.person_id
              WHERE p.person_key = 'synthetic-date-person'
                AND a.assertion_kind = $1 AND a.assertion_key = $2",
            &[&kind, &key],
        )
        .expect("assertion row")
        .get(0)
}

fn verify(client: &mut Client, assertion_id: i64) -> Result<(), ReviewError> {
    review::review_assertion(
        client,
        &ReviewRequest {
            assertion_id,
            new_status: ReviewStatus::Verified,
            reviewer: "synthetic-reviewer",
            evidence_ref: "synthetic page 7",
            note: None,
        },
    )
    .map(|_| ())
}

fn status_of(client: &mut Client, assertion_id: i64) -> String {
    client
        .query_one(
            "SELECT review_status FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .expect("status")
        .get(0)
}

fn count_dates(client: &mut Client) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM assertion
              WHERE person_id = (
                  SELECT person_id FROM person WHERE person_key = 'synthetic-date-person'
              )
                AND assertion_kind IN ('birth_date', 'death_date', 'date_unresolved')",
            &[],
        )
        .expect("count")
        .get(0)
}

#[test]
fn reimport_of_dates_is_idempotent_and_keeps_precision() {
    let Some(target) = isolated_target("dates_idempotent") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping date reimport test");
        return;
    };
    let mut client = target.connect();

    let value = date(
        "1880-03-15",
        None,
        DatePrecision::Day,
        DateQualifier::Exact,
        "15/03/1880",
    );
    let first = reimport(
        &mut client,
        vec![
            birth_assertion(value.clone()),
            death_assertion(date(
                "1950",
                None,
                DatePrecision::Year,
                DateQualifier::Circa,
                "khoảng 1950",
            )),
        ],
    );
    assert_eq!(first.inserted, 2, "two new date candidates");

    let second = reimport(
        &mut client,
        vec![
            birth_assertion(value),
            death_assertion(date(
                "1950",
                None,
                DatePrecision::Year,
                DateQualifier::Circa,
                "khoảng 1950",
            )),
        ],
    );
    assert_eq!(second.inserted, 0, "a repeat import inserts nothing");
    assert_eq!(second.unchanged, 2);
    assert_eq!(count_dates(&mut client), 2, "no duplicate date rows");

    let row = client
        .query_one(
            "SELECT value_date_start, value_date_end, value_date_precision,
                    value_date_qualifier, value_text
               FROM assertion
              WHERE assertion_kind = 'birth_date'",
            &[],
        )
        .expect("birth row");
    assert_eq!(row.get::<_, String>(0), "1880-03-15");
    assert_eq!(row.get::<_, Option<String>>(1), None);
    assert_eq!(row.get::<_, String>(2), "day");
    assert_eq!(row.get::<_, String>(3), "exact");
    assert_eq!(
        row.get::<_, Option<String>>(4).as_deref(),
        Some("15/03/1880"),
        "the original wording is preserved"
    );
}

#[test]
fn a_corrected_date_adds_a_claim_and_never_replaces_the_reviewed_one() {
    let Some(target) = isolated_target("dates_conflict") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping date conflict test");
        return;
    };
    let mut client = target.connect();

    let first = birth_assertion(date(
        "1877",
        None,
        DatePrecision::Year,
        DateQualifier::Exact,
        "born 1877",
    ));
    reimport(&mut client, vec![first.clone()]);
    let original_id = assertion_id(&mut client, "birth_date", &first.key);
    verify(&mut client, original_id).expect("the original date verifies");

    // The catalog is corrected: the same person now states another year.
    let corrected = birth_assertion(date(
        "1878",
        None,
        DatePrecision::Year,
        DateQualifier::Exact,
        "born 1878",
    ));
    reimport(&mut client, vec![corrected.clone()]);

    assert_eq!(count_dates(&mut client), 2, "both claims are preserved");
    assert_eq!(
        status_of(&mut client, original_id),
        "verified",
        "re-import never rewrites a reviewed value"
    );

    let corrected_id = assertion_id(&mut client, "birth_date", &corrected.key);
    let blocked = verify(&mut client, corrected_id);
    assert!(
        matches!(blocked, Err(ReviewError::Blocked(_, _))),
        "a conflicting date cannot be silently verified: {blocked:?}"
    );
    assert_eq!(
        status_of(&mut client, corrected_id),
        "unverified",
        "the refused claim keeps its prior status"
    );
    let blocked_events: i64 = client
        .query_one(
            "SELECT count(*) FROM assertion_review_event
              WHERE assertion_id = $1 AND to_status = 'needs_review'",
            &[&corrected_id],
        )
        .expect("blocked event")
        .get(0);
    assert_eq!(blocked_events, 1, "the refusal is audited for review");
}

#[test]
fn verified_dates_reach_the_tree_and_the_person_query() {
    let Some(target) = isolated_target("dates_projection") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping date projection test");
        return;
    };
    let mut client = target.connect();

    let birth = birth_assertion(date(
        "1877",
        None,
        DatePrecision::Year,
        DateQualifier::Exact,
        "born 1877",
    ));
    let death = death_assertion(date(
        "1922",
        None,
        DatePrecision::Year,
        DateQualifier::Exact,
        "died 1922",
    ));
    reimport(&mut client, vec![birth.clone(), death.clone()]);

    // Nothing is verified yet, so the tree carries no lifespan at all.
    let graph = tree::load_tree(&mut client, false).expect("tree");
    let person = graph
        .people
        .iter()
        .find(|person| person.person_key == "synthetic-date-person")
        .expect("person in tree");
    assert!(
        person.birth.is_none(),
        "an unverified date is not a lifespan"
    );
    assert!(person.death.is_none());

    let birth_id = assertion_id(&mut client, "birth_date", &birth.key);
    let death_id = assertion_id(&mut client, "death_date", &death.key);
    verify(&mut client, birth_id).expect("verify birth");
    verify(&mut client, death_id).expect("verify death");

    let graph = tree::load_tree(&mut client, false).expect("tree after review");
    let person = graph
        .people
        .iter()
        .find(|person| person.person_key == "synthetic-date-person")
        .expect("person in tree");
    let birth_date = person.birth.as_ref().expect("verified birth");
    assert_eq!(birth_date.start, "1877");
    assert_eq!(birth_date.precision, DatePrecision::Year);
    let death_date = person.death.as_ref().expect("verified death");
    assert_eq!(death_date.start, "1922");

    let detail = query::lookup_person(&mut client, "synthetic-date-person")
        .expect("query")
        .expect("person");
    let birth_row = detail
        .assertions
        .iter()
        .find(|assertion| assertion.kind == "birth_date")
        .expect("birth assertion in the API projection");
    let stored = birth_row.date.as_ref().expect("structured date");
    assert_eq!(stored.start, "1877");
    assert_eq!(stored.precision, DatePrecision::Year);
    assert_eq!(stored.qualifier, DateQualifier::Exact);
    assert_eq!(birth_row.review_status, "verified");
    assert!(!birth_row.citations.is_empty(), "citations are preserved");
}

#[test]
fn an_unresolved_date_is_stored_without_a_structured_value() {
    let Some(target) = isolated_target("dates_unresolved") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping unresolved date test");
        return;
    };
    let mut client = target.connect();

    let unresolved = ImportAssertion {
        person_key: "synthetic-date-person".to_string(),
        kind: "date_unresolved".to_string(),
        key: "date_unresolved:synthetic".to_string(),
        value_text: Some("1914".to_string()),
        object_key: None,
        evidence: vec![(ORIGINAL.to_string(), 9)],
        date: None,
    };
    reimport(&mut client, vec![unresolved]);

    let detail = query::lookup_person(&mut client, "synthetic-date-person")
        .expect("query")
        .expect("person");
    let row = detail
        .assertions
        .iter()
        .find(|assertion| assertion.kind == "date_unresolved")
        .expect("unresolved row");
    assert!(row.date.is_none(), "no structured value is invented");
    assert_eq!(row.value_text.as_deref(), Some("1914"));
    assert_eq!(row.review_status, "unverified");
}

#[test]
fn changing_the_source_reopens_a_verified_date_without_losing_history() {
    let Some(target) = isolated_target("dates_reopen") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping date reopen test");
        return;
    };
    let mut client = target.connect();

    let birth = birth_assertion(date(
        "1877",
        None,
        DatePrecision::Year,
        DateQualifier::Exact,
        "born 1877",
    ));
    reimport(&mut client, vec![birth.clone()]);
    let id = assertion_id(&mut client, "birth_date", &birth.key);
    verify(&mut client, id).expect("verify");

    // A new edition of the source document changes its hash.
    let changed_sha = "b".repeat(64);
    reimport::reimport_document(
        &mut client,
        &document(ORIGINAL, &changed_sha, vec![birth.clone()]),
    )
    .expect("reimport changed source");

    assert_eq!(
        status_of(&mut client, id),
        "needs_review",
        "changed evidence reopens the claim instead of keeping it verified"
    );
    let events: i64 = client
        .query_one(
            "SELECT count(*) FROM assertion_review_event WHERE assertion_id = $1",
            &[&id],
        )
        .expect("events")
        .get(0);
    assert!(events >= 2, "the review history is preserved: {events}");
}

#[test]
fn the_schema_refuses_an_inconsistent_date() {
    let Some(target) = isolated_target("dates_schema") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping date schema test");
        return;
    };
    let mut client = target.connect();
    reimport(
        &mut client,
        vec![birth_assertion(date(
            "1877",
            None,
            DatePrecision::Year,
            DateQualifier::Exact,
            "born 1877",
        ))],
    );
    let id = assertion_id(&mut client, "birth_date", "birth_date:1877");

    // A range precision without an end value is not a representable date.
    let invalid = client.execute(
        "UPDATE assertion SET value_date_precision = 'range' WHERE assertion_id = $1",
        &[&id],
    );
    assert!(invalid.is_err(), "the pair constraint must reject the row");

    // A non-ISO value is rejected too, so a caller cannot bypass the parser.
    let invalid_format = client.execute(
        "UPDATE assertion SET value_date_start = '15/03/1880' WHERE assertion_id = $1",
        &[&id],
    );
    assert!(
        invalid_format.is_err(),
        "the format constraint must reject the row"
    );
}

#[test]
fn overlapping_verified_claims_keep_the_more_precise_date() {
    let Some(target) = isolated_target("dates_refinement") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping date refinement test");
        return;
    };
    let mut client = target.connect();

    let coarse = birth_assertion(date(
        "1877",
        None,
        DatePrecision::Year,
        DateQualifier::Exact,
        "born 1877",
    ));
    let precise = birth_assertion(date(
        "1877-03-15",
        None,
        DatePrecision::Day,
        DateQualifier::Exact,
        "born 15/03/1877",
    ));
    reimport(&mut client, vec![coarse.clone(), precise.clone()]);

    let coarse_id = assertion_id(&mut client, "birth_date", &coarse.key);
    let precise_id = assertion_id(&mut client, "birth_date", &precise.key);
    verify(&mut client, coarse_id).expect("the year claim verifies");
    // A refinement is not a contradiction: both claims can be true.
    verify(&mut client, precise_id).expect("the day claim refines the year");

    let graph = tree::load_tree(&mut client, false).expect("tree");
    let person = graph
        .people
        .iter()
        .find(|person| person.person_key == "synthetic-date-person")
        .expect("person in tree");
    let birth = person.birth.as_ref().expect("a birth date is presented");
    assert_eq!(
        birth.start, "1877-03-15",
        "the more precise of two overlapping claims is shown"
    );
    assert_eq!(birth.precision, DatePrecision::Day);

    // Both claims stay stored and reviewable.
    assert_eq!(count_dates(&mut client), 2);
    assert_eq!(status_of(&mut client, coarse_id), "verified");
}
