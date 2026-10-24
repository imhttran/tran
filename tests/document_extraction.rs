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
    apply, decide, detail, extract, withdraw_document_evidence, DecisionInput, ExtractionError,
    ExtractionInput,
};
use rag_foundation::genealogy::person_search;
use rag_foundation::genealogy::query::lookup_person;
use rag_foundation::genealogy::tree::load_tree;

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

    // A text document cites its line, not an invented PDF page.
    let stored = lookup_person(&mut client, &created_key)
        .expect("person")
        .expect("exists");
    let parent_assertion = stored
        .assertions
        .iter()
        .find(|assertion| assertion.kind == "parent_child")
        .expect("parent assertion");
    assert_eq!(parent_assertion.citations.len(), 1);
    let citation = &parent_assertion.citations[0];
    assert_eq!(citation.locator.as_deref(), Some("line 3"));
    assert!(citation.label().contains("line 3"));
    assert!(citation
        .excerpt
        .as_deref()
        .unwrap_or_default()
        .contains("Relationship:"));

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

#[test]
fn a_reordered_reference_needs_confirmation_and_becomes_an_alias() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping reordered reference test");
        return;
    };
    let target = create_isolated_target(&base, "doc_extract_reordered");
    target.apply_migrations();
    let mut client = target.connect();
    insert_person(&mut client, "P300", "Tran Van Binh");

    let sha = "e".repeat(64);
    let extracted = extract(
        &mut client,
        ExtractionInput {
            document_version_id: 12,
            corpus: "family-docs",
            document_source: "synthetic/english-order.md",
            content_sha256: &sha,
            text: "Person: Binh Van Tran\n",
            actor: "admin-subject",
        },
    )
    .expect("extract");
    let person = proposal(&extracted, "person", "Binh Van Tran");
    // The English name order is a candidate, not proof of identity.
    assert_eq!(person.resolution, "ambiguous");
    assert_eq!(person.candidates.len(), 1);
    assert_eq!(person.candidates[0].match_kind, "reordered");

    // Approving an unconfirmed reordered match is refused.
    assert!(matches!(
        decide(
            &mut client,
            DecisionInput {
                proposal_id: person.proposal_id,
                decision: "approve",
                person_key: None,
                create_person: false,
                value_text: None,
                actor: "reviewer",
            },
        ),
        Err(ExtractionError::Invalid(_))
    ));

    // The reviewer confirms the identity explicitly.
    decide(
        &mut client,
        DecisionInput {
            proposal_id: person.proposal_id,
            decision: "approve",
            person_key: Some("P300"),
            create_person: false,
            value_text: None,
            actor: "reviewer",
        },
    )
    .expect("confirm identity");
    let report = apply(&mut client, 12, &sha, "reviewer").expect("apply");
    assert_eq!(report.applied.len(), 1);
    assert_eq!(
        report.applied[0].result["alias_recorded"],
        serde_json::json!("Binh Van Tran")
    );

    // The confirmed spelling is now a recorded alias on the same person.
    assert_eq!(
        count(
            &mut client,
            "SELECT count(*) FROM assertion a JOIN person p ON p.person_id = a.person_id
              WHERE p.person_key = 'P300' AND a.assertion_kind = 'name_variant'
                AND a.value_text = 'Binh Van Tran'",
            &[]
        ),
        1
    );
    let catalog = person_search::load_catalog(&mut client)
        .expect("catalog")
        .people;
    let ranked = person_search::rank(&catalog, "Binh Van Tran");
    assert_eq!(
        ranked
            .first()
            .map(|candidate| candidate.person_key.as_str()),
        Some("P300")
    );
    // The tree read now exposes the same alias, so its search can match it.
    let graph = load_tree(&mut client).expect("tree");
    let node = graph
        .people
        .iter()
        .find(|person| person.person_key == "P300")
        .expect("P300 in the tree");
    assert!(node.aliases.iter().any(|alias| alias == "Binh Van Tran"));

    // Evidence written before version ids were recorded is still withdrawn by
    // its document reference.
    client
        .execute(
            "UPDATE assertion_evidence SET document_version_id = NULL
              WHERE document_version_id = 12",
            &[],
        )
        .expect("simulate legacy evidence");
    // Withdrawing the document takes its evidence out of current answers.
    let withdrawn = withdraw_document_evidence(&mut client, 12, "synthetic/english-order.md")
        .expect("withdraw");
    assert!(withdrawn >= 1);
    let stored = lookup_person(&mut client, "P300")
        .expect("person")
        .expect("exists");
    assert!(
        stored
            .assertions
            .iter()
            .all(|assertion| assertion.citations.is_empty()),
        "withdrawn evidence must not be cited"
    );

    drop(client);
    target.cleanup();
}

#[test]
fn a_verified_assertion_is_not_rewritten_by_a_later_apply() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping verified preservation test");
        return;
    };
    let target = create_isolated_target(&base, "doc_extract_verified");
    target.apply_migrations();
    let mut client = target.connect();
    insert_person(&mut client, "P100", "Hoang Van Tran");
    insert_person(&mut client, "P200", "Lan Thi Nguyen");

    let text = "Person: Minh An Tran\nRelationship: Son of Hoang Van Tran and Lan Thi Nguyen.\n";
    let approve_all =
        |client: &mut postgres::Client,
         detail: &rag_foundation::genealogy::document_extraction::ExtractionDetail| {
            for proposal in &detail.proposals {
                let person_key =
                    if proposal.kind == "person" && proposal.subject_name == "Minh An Tran" {
                        None
                    } else {
                        proposal.resolved_person_key.as_deref()
                    };
                decide(
                    client,
                    DecisionInput {
                        proposal_id: proposal.proposal_id,
                        decision: "approve",
                        person_key,
                        create_person: proposal.kind == "person"
                            && proposal.subject_name == "Minh An Tran",
                        value_text: None,
                        actor: "reviewer",
                    },
                )
                .expect("approve");
            }
        };

    let first_sha = "f".repeat(64);
    let first = extract(
        &mut client,
        ExtractionInput {
            document_version_id: 13,
            corpus: "family-docs",
            document_source: "synthetic/verified.md",
            content_sha256: &first_sha,
            text,
            actor: "admin-subject",
        },
    )
    .expect("extract");
    approve_all(&mut client, &first);
    apply(&mut client, 13, &first_sha, "reviewer").expect("apply");

    // A reviewer verifies one parent link, and it is stored with the raw key
    // (the shape a later repair would otherwise rewrite).
    let child_id: i64 = client
        .query_one(
            "SELECT person_id FROM person WHERE display_name = 'Minh An Tran'",
            &[],
        )
        .expect("child")
        .get(0);
    client
        .execute(
            "UPDATE assertion
                SET value_text = 'P100', review_status = 'verified',
                    reviewed_by = 'reviewer', reviewed_at = now()
              WHERE person_id = $1 AND assertion_kind = 'parent_child' AND value_text = 'Hoang Van Tran'",
            &[&child_id],
        )
        .expect("verify parent link");

    // A new extraction applies the same facts again.
    let second_sha = "g".repeat(64);
    let second = extract(
        &mut client,
        ExtractionInput {
            document_version_id: 14,
            corpus: "family-docs",
            document_source: "synthetic/verified.md",
            content_sha256: &second_sha,
            text,
            actor: "admin-subject",
        },
    )
    .expect("second extract");
    approve_all(&mut client, &second);
    apply(&mut client, 14, &second_sha, "reviewer").expect("second apply");

    let row = client
        .query_one(
            "SELECT value_text, review_status FROM assertion
              WHERE person_id = $1 AND assertion_kind = 'parent_child' AND value_text = 'P100'",
            &[&child_id],
        )
        .expect("verified parent link");
    assert_eq!(row.get::<_, String>(0), "P100");
    assert_eq!(row.get::<_, String>(1), "verified");

    drop(client);
    target.cleanup();
}
