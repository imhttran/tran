//! Synthetic PostgreSQL integration tests for the reviewed document-to-genealogy
//! workflow.
//!
//! Every test uses a disposable genealogy database created by
//! `tests/support/genealogy_isolation.rs`. No live family record is read or
//! written.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn, create_isolated_target};
use rag_foundation::genealogy::document_extraction::{
    apply, decide, detail, extract, DecisionInput, ExtractionError, ExtractionInput,
};
use rag_foundation::genealogy::person_search;

const CARD: &str = "\
Person: Minh An Tran
Also known as: minhan
Relationship: Son of Hoang Van Tran and Lan Thi Nguyen.
Instagram: https://example.test/minhan
Source: Synthetic card.
";

fn insert_person(client: &mut postgres::Client, key: &str, name: &str) {
    client
        .execute(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2)",
            &[&key, &name],
        )
        .expect("insert synthetic person");
}

fn count(
    client: &mut postgres::Client,
    sql: &str,
    params: &[&(dyn postgres::types::ToSql + Sync)],
) -> i64 {
    client.query_one(sql, params).expect("count").get(0)
}

fn proposal<'a>(
    detail: &'a rag_foundation::genealogy::document_extraction::ExtractionDetail,
    kind: &str,
    subject: &str,
) -> &'a rag_foundation::genealogy::document_extraction::ProposalRow {
    detail
        .proposals
        .iter()
        .find(|proposal| proposal.kind == kind && proposal.subject_name == subject)
        .unwrap_or_else(|| panic!("no {kind} proposal for {subject}"))
}

#[test]
fn extraction_proposes_reviewed_records_and_apply_is_idempotent() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping document extraction test");
        return;
    };
    let target = create_isolated_target(&base, "doc_extract");
    target.apply_migrations();
    let mut client = target.connect();

    insert_person(&mut client, "P100", "Hoang Van Tran");
    insert_person(&mut client, "P200", "Lan Thi Nguyen");
    // A namesake: the same recorded name, a different person.
    insert_person(&mut client, "P201", "Lan Thi Nguyen");

    let sha = "a".repeat(64);
    let extracted = extract(
        &mut client,
        ExtractionInput {
            document_version_id: 7,
            corpus: "family-docs",
            document_source: "synthetic/demo.md",
            content_sha256: &sha,
            text: CARD,
            actor: "admin-subject",
        },
    )
    .expect("extract");

    // The subject is unknown, one parent matches, the other is a namesake.
    let subject = proposal(&extracted, "person", "Minh An Tran");
    assert_eq!(subject.resolution, "unresolved");
    assert!(subject.resolved_person_key.is_none());
    assert!(subject.support_text.contains("Person: Minh An Tran"));

    let matched_parent = proposal(&extracted, "person", "Hoang Van Tran");
    assert_eq!(matched_parent.resolution, "matched");
    assert_eq!(matched_parent.resolved_person_key.as_deref(), Some("P100"));

    let ambiguous_parent = proposal(&extracted, "person", "Lan Thi Nguyen");
    assert_eq!(ambiguous_parent.resolution, "ambiguous");
    assert_eq!(ambiguous_parent.candidates.len(), 2);

    let links: Vec<&str> = extracted
        .proposals
        .iter()
        .filter(|proposal| proposal.kind == "parent_child")
        .map(|proposal| proposal.target_name.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(links.len(), 2);

    let alias = proposal(&extracted, "alias", "Minh An Tran");
    assert_eq!(alias.value_text.as_deref(), Some("minhan"));
    let social = proposal(&extracted, "social_link", "Minh An Tran");
    assert_eq!(
        social.value_text.as_deref(),
        Some("https://example.test/minhan")
    );
    // The alias is never turned into a link.
    assert_ne!(alias.value_text, social.value_text);

    // Approving an ambiguous reference without choosing a person is refused.
    let ambiguous_id = ambiguous_parent.proposal_id;
    let refused = decide(
        &mut client,
        DecisionInput {
            proposal_id: ambiguous_id,
            decision: "approve",
            person_key: None,
            create_person: false,
            value_text: None,
            actor: "reviewer",
        },
    );
    assert!(matches!(refused, Err(ExtractionError::Invalid(_))));

    // Approve everything a reviewer would approve.
    let approve = |client: &mut postgres::Client,
                   proposal_id: i64,
                   person_key: Option<&str>,
                   create: bool| {
        decide(
            client,
            DecisionInput {
                proposal_id,
                decision: "approve",
                person_key,
                create_person: create,
                value_text: None,
                actor: "reviewer",
            },
        )
        .expect("approve proposal")
    };
    approve(&mut client, subject.proposal_id, None, true);
    approve(&mut client, matched_parent.proposal_id, None, false);
    approve(&mut client, ambiguous_id, Some("P200"), false);
    let relationship_ids: Vec<i64> = extracted
        .proposals
        .iter()
        .filter(|proposal| proposal.kind == "parent_child")
        .map(|proposal| proposal.proposal_id)
        .collect();
    for id in &relationship_ids {
        approve(&mut client, *id, None, false);
    }
    // A reviewer may correct an alias before approving it.
    decide(
        &mut client,
        DecisionInput {
            proposal_id: alias.proposal_id,
            decision: "approve",
            person_key: None,
            create_person: false,
            value_text: Some("minh.an"),
            actor: "reviewer",
        },
    )
    .expect("approve corrected alias");
    approve(&mut client, social.proposal_id, None, false);

    let persons_before = count(&mut client, "SELECT count(*) FROM person", &[]);
    let report = apply(&mut client, 7, &sha, "reviewer").expect("apply");
    assert_eq!(report.applied.len(), 7);
    assert!(!report.already_applied);

    // One new person, no duplicate parents.
    let persons_after = count(&mut client, "SELECT count(*) FROM person", &[]);
    assert_eq!(persons_after, persons_before + 1);
    assert_eq!(
        count(
            &mut client,
            "SELECT count(*) FROM person WHERE display_name = 'Hoang Van Tran'",
            &[]
        ),
        1
    );

    let created_key: String = client
        .query_one(
            "SELECT person_key FROM person WHERE display_name = 'Minh An Tran'",
            &[],
        )
        .expect("created person")
        .get(0);
    let created_id: i64 = client
        .query_one(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&created_key],
        )
        .expect("created id")
        .get(0);

    assert_eq!(
        count(
            &mut client,
            "SELECT count(*) FROM parent_child_link WHERE child_id = $1",
            &[&created_id]
        ),
        2
    );
    // The applied parent link names the parent, not an internal key.
    let parent_values: Vec<String> = client
        .query(
            "SELECT value_text FROM assertion WHERE person_id = $1 AND assertion_kind = 'parent_child' ORDER BY value_text",
            &[&created_id],
        )
        .expect("parent assertions")
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(
        parent_values,
        vec!["Hoang Van Tran".to_string(), "Lan Thi Nguyen".to_string()]
    );

    assert_eq!(
        count(
            &mut client,
            "SELECT count(*) FROM assertion WHERE person_id = $1 AND assertion_kind = 'name_variant' AND value_text = 'minh.an'",
            &[&created_id]
        ),
        1
    );
    assert_eq!(
        count(
            &mut client,
            "SELECT count(*) FROM assertion WHERE person_id = $1 AND assertion_kind = 'social_link' AND value_text = 'https://example.test/minhan'",
            &[&created_id]
        ),
        1
    );
    // Extraction approval never verifies a fact.
    assert_eq!(
        count(
            &mut client,
            "SELECT count(*) FROM assertion WHERE person_id = $1 AND review_status <> 'unverified'",
            &[&created_id]
        ),
        0
    );
    // Every applied assertion keeps the document reference and the exact line.
    assert_eq!(
        count(
            &mut client,
            "SELECT count(*) FROM assertion_evidence WHERE document_ref = 'synthetic/demo.md' AND excerpt IS NOT NULL",
            &[]
        ),
        5
    );

    // The alias is searchable through the existing matcher.
    let catalog = person_search::load_catalog(&mut client)
        .expect("catalog")
        .people;
    let ranked = person_search::rank(&catalog, "minh.an");
    assert_eq!(
        ranked
            .first()
            .map(|candidate| candidate.person_key.as_str()),
        Some(created_key.as_str())
    );

    // Repeating extraction and apply changes nothing.
    let again = extract(
        &mut client,
        ExtractionInput {
            document_version_id: 7,
            corpus: "family-docs",
            document_source: "synthetic/demo.md",
            content_sha256: &sha,
            text: CARD,
            actor: "another-admin",
        },
    )
    .expect("re-extract");
    assert_eq!(
        again.extraction.extraction_id,
        extracted.extraction.extraction_id
    );
    assert_eq!(again.proposals.len(), extracted.proposals.len());

    let repeat = apply(&mut client, 7, &sha, "reviewer").expect("re-apply");
    assert!(repeat.already_applied);
    assert_eq!(repeat.applied.len(), 7);
    assert_eq!(
        count(&mut client, "SELECT count(*) FROM person", &[]),
        persons_after
    );
    assert_eq!(
        count(&mut client, "SELECT count(*) FROM parent_child_link", &[]),
        2
    );

    drop(client);
    target.cleanup();
}

#[test]
fn a_stale_extraction_is_refused_and_unknown_references_block_apply() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping stale extraction test");
        return;
    };
    let target = create_isolated_target(&base, "doc_extract_stale");
    target.apply_migrations();
    let mut client = target.connect();
    insert_person(&mut client, "P100", "Hoang Van Tran");
    insert_person(&mut client, "P200", "Lan Thi Nguyen");

    let sha = "b".repeat(64);
    let extracted = extract(
        &mut client,
        ExtractionInput {
            document_version_id: 9,
            corpus: "family-docs",
            document_source: "synthetic/stale.md",
            content_sha256: &sha,
            text: CARD,
            actor: "admin-subject",
        },
    )
    .expect("extract");

    // Nothing is approved yet, so applying is refused rather than recording an
    // empty success.
    assert!(matches!(
        apply(&mut client, 9, &sha, "reviewer"),
        Err(ExtractionError::Invalid(_))
    ));

    // Approve the subject and one relationship, but reject the second parent's
    // person proposal: the relationship then has no person to point at.
    let subject = proposal(&extracted, "person", "Minh An Tran");
    decide(
        &mut client,
        DecisionInput {
            proposal_id: subject.proposal_id,
            decision: "approve",
            person_key: None,
            create_person: true,
            value_text: None,
            actor: "reviewer",
        },
    )
    .expect("approve subject");
    let target_parent = extracted
        .proposals
        .iter()
        .find(|proposal| proposal.kind == "person" && proposal.subject_name == "Lan Thi Nguyen")
        .expect("namesake proposal");
    decide(
        &mut client,
        DecisionInput {
            proposal_id: target_parent.proposal_id,
            decision: "reject",
            person_key: None,
            create_person: false,
            value_text: None,
            actor: "reviewer",
        },
    )
    .expect("reject namesake");
    let relationship = extracted
        .proposals
        .iter()
        .find(|proposal| {
            proposal.kind == "parent_child"
                && proposal.target_name.as_deref() == Some("Lan Thi Nguyen")
        })
        .expect("relationship");
    decide(
        &mut client,
        DecisionInput {
            proposal_id: relationship.proposal_id,
            decision: "approve",
            person_key: None,
            create_person: false,
            value_text: None,
            actor: "reviewer",
        },
    )
    .expect("approve relationship");

    assert!(matches!(
        apply(&mut client, 9, &sha, "reviewer"),
        Err(ExtractionError::UnresolvedPerson(name)) if name == "Lan Thi Nguyen"
    ));

    // A replaced document changes the hash, so the old extraction cannot apply.
    assert!(matches!(
        apply(&mut client, 9, &"c".repeat(64), "reviewer"),
        Err(ExtractionError::Stale)
    ));

    // The stored extraction is still visible to the review screen.
    let stored = detail(&mut client, 9).expect("detail").expect("extraction");
    assert_eq!(stored.extraction.status, "draft");

    drop(client);
    target.cleanup();
}

#[test]
fn a_later_approval_can_still_be_applied_without_duplicates() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping incremental apply test");
        return;
    };
    let target = create_isolated_target(&base, "doc_extract_incremental");
    target.apply_migrations();
    let mut client = target.connect();

    let sha = "d".repeat(64);
    let extracted = extract(
        &mut client,
        ExtractionInput {
            document_version_id: 11,
            corpus: "family-docs",
            document_source: "synthetic/incremental.md",
            content_sha256: &sha,
            text: "Person: Lan An Tran\nAlso known as: lanan\n",
            actor: "admin-subject",
        },
    )
    .expect("extract");
    let person = proposal(&extracted, "person", "Lan An Tran");
    let alias = proposal(&extracted, "alias", "Lan An Tran");

    // Approve only the person, then apply.
    decide(
        &mut client,
        DecisionInput {
            proposal_id: person.proposal_id,
            decision: "approve",
            person_key: None,
            create_person: true,
            value_text: None,
            actor: "reviewer",
        },
    )
    .expect("approve person");
    let first = apply(&mut client, 11, &sha, "reviewer").expect("first apply");
    assert_eq!(first.applied.len(), 1);
    assert!(!first.already_applied);

    // A later approval is applied on the next call, not lost.
    decide(
        &mut client,
        DecisionInput {
            proposal_id: alias.proposal_id,
            decision: "approve",
            person_key: None,
            create_person: false,
            value_text: None,
            actor: "reviewer",
        },
    )
    .expect("approve alias");
    let second = apply(&mut client, 11, &sha, "reviewer").expect("second apply");
    assert_eq!(second.applied.len(), 1);
    assert!(second.already_applied);
    assert_eq!(
        count(&mut client, "SELECT count(*) FROM person", &[]),
        1,
        "the person is created once"
    );
    assert_eq!(
        count(
            &mut client,
            "SELECT count(*) FROM assertion WHERE assertion_kind = 'name_variant'",
            &[]
        ),
        1
    );

    // Applying once more changes nothing.
    let third = apply(&mut client, 11, &sha, "reviewer").expect("third apply");
    assert!(third.already_applied);
    assert_eq!(third.applied.len(), 2);

    drop(client);
    target.cleanup();
}
