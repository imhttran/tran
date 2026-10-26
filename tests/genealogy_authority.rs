//! Synthetic PostgreSQL tests for authoritative source decisions.
//!
//! Every test creates a disposable genealogy database through
//! `tests/support/genealogy_isolation.rs`. No live family record is read or
//! written, and no fixture is copied from the family corpus.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn, create_isolated_target, IsolatedTarget};
use rag_foundation::genealogy::authority::{
    self, ApplyInput, AuthorityError, ReverseInput, ScopeSpec,
};
use rag_foundation::genealogy::document_extraction::withdraw_document_evidence;
use rag_foundation::genealogy::eligibility::{self, KIND_BRANCH};
use rag_foundation::genealogy::person_search::{self, rank};
use rag_foundation::genealogy::query::{lookup_person, unresolved_assertions};
use rag_foundation::genealogy::review::{
    review_assertion, ReviewError, ReviewRequest, ReviewStatus,
};
use rag_foundation::genealogy::tree::load_tree;
use std::sync::mpsc;
use std::time::Duration;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn setup(label: &str) -> Option<(IsolatedTarget, postgres::Client)> {
    let base = base_dsn()?;
    let target = create_isolated_target(&base, label);
    target.apply_migrations();
    let client = target.connect();
    Some((target, client))
}

fn insert_person(client: &mut postgres::Client, key: &str, name: &str) -> i64 {
    client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2)
             RETURNING person_id",
            &[&key, &name],
        )
        .expect("insert synthetic person")
        .get(0)
}

/// Inserts the applied extraction row a document version needs.
///
/// A row with `status = 'applied'` must also carry `applied_by` and
/// `applied_at` (`document_extraction_applied_check`), so the fixture writes
/// them together.
fn insert_extraction(
    client: &mut postgres::Client,
    version_id: i64,
    document_source: &str,
    sha: &str,
) {
    client
        .execute(
            "INSERT INTO document_extraction
                 (document_version_id, corpus, document_source, content_sha256, created_by,
                  status, applied_by, applied_at)
             VALUES ($1, 'synthetic', $2, $3, 'fixture', 'applied', 'fixture', now())",
            &[&version_id, &document_source, &sha],
        )
        .expect("insert synthetic extraction");
}

/// Inserts one current assertion with evidence bound to a document version.
///
/// Evidence is added before a `verified` status is written, because a verified
/// assertion must already have evidence when its own statement commits.
#[allow(clippy::too_many_arguments)]
fn insert_assertion(
    client: &mut postgres::Client,
    person_id: i64,
    kind: &str,
    key: &str,
    value: &str,
    version_id: i64,
    document_ref: &str,
    line: i32,
    review_status: &str,
) -> i64 {
    let assertion_id: i64 = client
        .query_one(
            "INSERT INTO assertion
                 (person_id, assertion_kind, assertion_key, value_text, review_status)
             VALUES ($1, $2, $3, $4, 'unverified')
             RETURNING assertion_id",
            &[&person_id, &kind, &key, &value],
        )
        .expect("insert synthetic assertion")
        .get(0);
    client
        .execute(
            "INSERT INTO assertion_evidence
                 (assertion_id, document_ref, page, locator, excerpt, document_version_id)
             VALUES ($1, $2, 1, $3, $4, $5)",
            &[
                &assertion_id,
                &document_ref,
                &format!("line {line}"),
                &format!("{kind}: {value}"),
                &version_id,
            ],
        )
        .expect("insert synthetic evidence");
    if review_status == "verified" {
        client
            .execute(
                "UPDATE assertion
                    SET review_status = 'verified', reviewed_by = 'fixture', reviewed_at = now()
                  WHERE assertion_id = $1",
                &[&assertion_id],
            )
            .expect("verify synthetic assertion");
    }
    assertion_id
}

/// True when no authority decision superseded the assertion.
///
/// This is intentionally narrower than "current": an assertion can be
/// unsuperseded and still stop being current because its only source was
/// withdrawn. Tests assert both facts separately.
fn is_not_superseded(client: &mut postgres::Client, assertion_id: i64) -> bool {
    client
        .query_one(
            "SELECT superseded_at IS NULL FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .expect("read supersession")
        .get(0)
}

fn review_status(client: &mut postgres::Client, assertion_id: i64) -> String {
    client
        .query_one(
            "SELECT review_status FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .expect("read review status")
        .get(0)
}

/// Inserts a parent-child link plus the child's assertion naming the parent.
fn insert_parent_link(
    client: &mut postgres::Client,
    parent_id: i64,
    parent_name: &str,
    child_id: i64,
    version_id: i64,
    document_ref: &str,
    review_status: &str,
) -> i64 {
    client
        .execute(
            "INSERT INTO parent_child_link (parent_id, child_id) VALUES ($1, $2)
             ON CONFLICT (parent_id, child_id) DO NOTHING",
            &[&parent_id, &child_id],
        )
        .expect("insert synthetic parent link");
    let assertion_id = insert_assertion(
        client,
        child_id,
        "parent_child",
        &format!("parent_child:{parent_id}"),
        parent_name,
        version_id,
        document_ref,
        4,
        review_status,
    );
    client
        .execute(
            "UPDATE assertion SET object_person_id = $2 WHERE assertion_id = $1",
            &[&assertion_id, &parent_id],
        )
        .expect("bind the parent object");
    assertion_id
}

/// Sets the structured date columns the tree projection reads.
fn set_birth_date(client: &mut postgres::Client, assertion_id: i64, start: &str) {
    client
        .execute(
            "UPDATE assertion SET value_date_start = $2, value_date_precision = 'year',
                    value_date_qualifier = 'exact' WHERE assertion_id = $1",
            &[&assertion_id, &start],
        )
        .expect("set structured date");
}

fn withdraw(client: &mut postgres::Client, version_id: i64, document_ref: &str) -> u64 {
    withdraw_document_evidence(client, version_id, document_ref).expect("withdraw evidence")
}

#[test]
fn document_scope_replaces_the_current_value_and_preserves_history() {
    let Some((target, mut client)) = setup("authority_doc") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping authority test");
        return;
    };

    let person = insert_person(&mut client, "P001", "Trần Văn An");
    insert_extraction(&mut client, 1, "synthetic/doc-a.md", SHA_A);
    // The previously reviewed value, supported by an older document.
    let previous = insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:old",
        "Trần Văn An",
        900,
        "synthetic/catalog.md",
        3,
        "verified",
    );
    // The newer document spells the same name differently, so its assertion key
    // differs and nothing but an authority decision connects the two.
    let incoming = insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:new",
        "Tran Van An",
        1,
        "synthetic/doc-a.md",
        1,
        "unverified",
    );

    let spec = ScopeSpec::document();
    let preview = authority::preview(&mut client, 1, SHA_A, &spec).expect("preview");
    assert_eq!(preview.items.len(), 1);
    assert_eq!(preview.items[0].action, "replaced");
    assert_eq!(preview.items[0].replaces.len(), 1);
    assert_eq!(preview.items[0].replaces[0].assertion_id, previous);
    assert_eq!(preview.conflicts.len(), 1);
    assert_eq!(preview.conflicts[0].code, "replaces_verified");
    assert_eq!(preview.conflicts[0].assertion.assertion_id, previous);

    // A reviewed record may not lose authority without an explicit
    // acknowledgement.
    let refused = authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 1,
            content_sha256: SHA_A,
            spec: ScopeSpec::document(),
            reason: "Newer reviewed submission",
            actor: "reviewer-1",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: false,
        },
    );
    assert!(matches!(refused, Err(AuthorityError::Conflicts(1))));
    assert!(is_not_superseded(&mut client, previous));
    assert!(is_not_superseded(&mut client, incoming));

    let report = authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 1,
            content_sha256: SHA_A,
            spec: ScopeSpec::document(),
            reason: "Newer reviewed submission",
            actor: "reviewer-1",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("apply");
    assert_eq!(report.applied, 1);
    assert_eq!(report.superseded, 1);

    // The superseded row keeps its value, its review status, and its citation.
    assert!(!is_not_superseded(&mut client, previous));
    assert_eq!(review_status(&mut client, previous), "verified");
    let kept = client
        .query_one(
            "SELECT a.value_text, count(ae.assertion_evidence_id) AS evidence_count
               FROM assertion a
               LEFT JOIN assertion_evidence ae ON ae.assertion_id = a.assertion_id
              WHERE a.assertion_id = $1
              GROUP BY a.value_text",
            &[&previous],
        )
        .expect("read kept history");
    let kept_value: String = kept.get(0);
    let kept_evidence: i64 = kept.get(1);
    assert_eq!(kept_value, "Trần Văn An");
    assert_eq!(kept_evidence, 1);

    // Authoritative is not verified: the override never promotes the claim.
    assert!(is_not_superseded(&mut client, incoming));
    assert_eq!(review_status(&mut client, incoming), "unverified");

    // Current views read the new value only, and label it as authoritative.
    let detail = lookup_person(&mut client, "P001")
        .expect("read person")
        .expect("person exists");
    let names: Vec<&str> = detail
        .assertions
        .iter()
        .filter(|assertion| assertion.kind == "display_name")
        .map(|assertion| assertion.value_text.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(names, vec!["Tran Van An"]);
    let authority = detail
        .assertions
        .iter()
        .find(|assertion| assertion.assertion_id == incoming)
        .and_then(|assertion| assertion.authority.as_ref())
        .expect("authoritative assertion is annotated");
    assert_eq!(authority.decision_id, report.decision_id);
    assert_eq!(authority.actor, "reviewer-1");
    assert_eq!(authority.reason, "Newer reviewed submission");

    // The unresolved queue no longer offers the superseded row.
    let unresolved: Vec<i64> = unresolved_assertions(&mut client)
        .expect("unresolved")
        .into_iter()
        .map(|row| row.assertion_id)
        .collect();
    assert!(!unresolved.contains(&previous));

    target.cleanup();
}

#[test]
fn a_stale_preview_token_is_refused() {
    let Some((target, mut client)) = setup("authority_stale") else {
        return;
    };
    let person = insert_person(&mut client, "P010", "Lê Thị Mai");
    insert_extraction(&mut client, 2, "synthetic/doc-b.md", SHA_B);
    insert_assertion(
        &mut client,
        person,
        "name_variant",
        "variant:mai",
        "mai.le",
        2,
        "synthetic/doc-b.md",
        2,
        "unverified",
    );

    let spec = ScopeSpec::document();
    let preview = authority::preview(&mut client, 2, SHA_B, &spec).expect("preview");
    // Any concurrent change to a slot invalidates the confirmation token.
    let other = insert_assertion(
        &mut client,
        person,
        "name_variant",
        "variant:mai2",
        "mai",
        2,
        "synthetic/doc-b.md",
        3,
        "unverified",
    );
    let _ = other;
    client
        .execute(
            "UPDATE assertion SET superseded_at = now() WHERE assertion_id = $1",
            &[&other],
        )
        .expect("supersede a sibling");

    let refused = authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 2,
            content_sha256: SHA_B,
            spec,
            reason: "Should be refused",
            actor: "reviewer-1",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    );
    assert!(matches!(refused, Err(AuthorityError::Stale)));

    // A content-hash mismatch is stale too, before any row is written.
    assert!(matches!(
        authority::preview(&mut client, 2, SHA_C, &ScopeSpec::document()),
        Err(AuthorityError::Stale)
    ));
    target.cleanup();
}

#[test]
fn person_and_assertion_scopes_limit_the_effect() {
    let Some((target, mut client)) = setup("authority_scopes") else {
        return;
    };
    let first = insert_person(&mut client, "P100", "Nguyễn Văn An");
    let second = insert_person(&mut client, "P200", "Trần Văn Bình");
    insert_extraction(&mut client, 10, "synthetic/doc-c.md", SHA_C);

    let old_first = insert_assertion(
        &mut client,
        first,
        "birth_date",
        "date:1900",
        "1900",
        800,
        "synthetic/old-catalog.md",
        4,
        "unverified",
    );
    let old_second = insert_assertion(
        &mut client,
        second,
        "birth_date",
        "date:1905",
        "1905",
        800,
        "synthetic/old-catalog.md",
        5,
        "unverified",
    );
    let new_first = insert_assertion(
        &mut client,
        first,
        "birth_date",
        "date:1901",
        "1901",
        10,
        "synthetic/doc-c.md",
        2,
        "unverified",
    );
    let new_second = insert_assertion(
        &mut client,
        second,
        "birth_date",
        "date:1906",
        "1906",
        10,
        "synthetic/doc-c.md",
        3,
        "unverified",
    );

    // A person scope only touches that person.
    let spec = ScopeSpec {
        scope: "person",
        person_key: Some("P100"),
        assertion_ids: &[],
        remove_assertion_ids: &[],
    };
    let preview = authority::preview(&mut client, 10, SHA_C, &spec).expect("person preview");
    assert_eq!(preview.person_key.as_deref(), Some("P100"));
    assert_eq!(preview.items.len(), 1);
    assert_eq!(preview.items[0].action, "replaced");

    let report = authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 10,
            content_sha256: SHA_C,
            spec,
            reason: "Person-scoped correction",
            actor: "reviewer-2",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("person apply");
    assert_eq!(report.applied, 1);
    assert!(!is_not_superseded(&mut client, old_first));
    assert!(is_not_superseded(&mut client, new_first));
    // The other person is untouched by a person-scoped decision.
    assert!(is_not_superseded(&mut client, old_second));
    assert!(is_not_superseded(&mut client, new_second));

    // An assertion scope selects exactly the named assertions.
    let selected = [new_second];
    let spec = ScopeSpec {
        scope: "assertions",
        person_key: None,
        assertion_ids: &selected,
        remove_assertion_ids: &[],
    };
    let preview = authority::preview(&mut client, 10, SHA_C, &spec).expect("assertion preview");
    assert_eq!(preview.items.len(), 1);
    assert_eq!(preview.items[0].action, "replaced");
    authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 10,
            content_sha256: SHA_C,
            spec,
            reason: "Only this assertion",
            actor: "reviewer-2",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("assertion apply");
    assert!(!is_not_superseded(&mut client, old_second));
    assert!(is_not_superseded(&mut client, new_second));

    // Namesakes stay separate: the two people keep their own values.
    assert_eq!(
        lookup_person(&mut client, "P100")
            .expect("read first")
            .expect("first exists")
            .assertions
            .iter()
            .filter(|assertion| assertion.kind == "birth_date")
            .map(|assertion| assertion.value_text.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["1901".to_string()]
    );
    assert_eq!(
        lookup_person(&mut client, "P200")
            .expect("read second")
            .expect("second exists")
            .assertions
            .iter()
            .filter(|assertion| assertion.kind == "birth_date")
            .map(|assertion| assertion.value_text.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["1906".to_string()]
    );
    target.cleanup();
}

#[test]
fn omission_is_not_deletion_unless_the_reviewer_confirms_it() {
    let Some((target, mut client)) = setup("authority_omission") else {
        return;
    };
    let person = insert_person(&mut client, "P300", "Phạm Thị Lan");
    insert_extraction(&mut client, 20, "synthetic/doc-d.md", SHA_A);
    let first_alias = insert_assertion(
        &mut client,
        person,
        "name_variant",
        "variant:one",
        "lan",
        700,
        "synthetic/old.md",
        2,
        "unverified",
    );
    let second_alias = insert_assertion(
        &mut client,
        person,
        "name_variant",
        "variant:two",
        "lan.pham",
        700,
        "synthetic/old.md",
        3,
        "unverified",
    );
    let incoming = insert_assertion(
        &mut client,
        person,
        "name_variant",
        "variant:three",
        "lannie",
        20,
        "synthetic/doc-d.md",
        2,
        "unverified",
    );

    let preview = authority::preview(&mut client, 20, SHA_A, &ScopeSpec::document())
        .expect("preview omissions");
    let omitted: Vec<i64> = preview
        .omissions
        .iter()
        .map(|view| view.assertion_id)
        .collect();
    assert_eq!(omitted.len(), 2);
    assert!(omitted.contains(&first_alias));
    assert!(omitted.contains(&second_alias));

    // Applying without naming a removal leaves both current.
    authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 20,
            content_sha256: SHA_A,
            spec: ScopeSpec::document(),
            reason: "Add the new alias",
            actor: "reviewer-3",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("apply without removal");
    assert!(is_not_superseded(&mut client, first_alias));
    assert!(is_not_superseded(&mut client, second_alias));
    assert!(is_not_superseded(&mut client, incoming));

    // Naming one removal turns it into an explicit removal.
    let remove = [first_alias];
    let spec = ScopeSpec {
        scope: "document",
        person_key: None,
        assertion_ids: &[],
        remove_assertion_ids: &remove,
    };
    let preview = authority::preview(&mut client, 20, SHA_A, &spec).expect("removal preview");
    assert_eq!(preview.items.len(), 2);
    assert!(preview
        .items
        .iter()
        .any(|item| item.action == "removed" && item.incoming.is_none()));
    assert!(preview
        .omissions
        .iter()
        .all(|view| view.assertion_id != first_alias));
    authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 20,
            content_sha256: SHA_A,
            spec,
            reason: "Remove the obsolete alias",
            actor: "reviewer-3",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("apply removal");
    assert!(!is_not_superseded(&mut client, first_alias));
    assert!(is_not_superseded(&mut client, second_alias));

    // The removed row is still readable in the authorized history.
    let decisions = authority::list_decisions(&mut client, Some("P300")).expect("history");
    assert!(!decisions.is_empty());
    let removed_item = decisions
        .iter()
        .flat_map(|decision| decision.items.iter())
        .find(|item| item.action == "removed")
        .expect("removal is recorded");
    assert_eq!(removed_item.replaces[0].value_text.as_deref(), Some("lan"));

    // Removing an assertion that the document already replaces is refused.
    let redundant = ScopeSpec {
        scope: "document",
        person_key: None,
        assertion_ids: &[],
        remove_assertion_ids: &[incoming],
    };
    assert!(matches!(
        authority::preview(&mut client, 20, SHA_A, &redundant),
        Err(AuthorityError::Invalid(_))
    ));
    target.cleanup();
}

#[test]
fn reversal_restores_the_previous_version_and_keeps_both_records() {
    let Some((target, mut client)) = setup("authority_reverse") else {
        return;
    };
    let person = insert_person(&mut client, "P400", "Đỗ Văn Hải");
    insert_extraction(&mut client, 30, "synthetic/doc-e.md", SHA_B);
    let previous = insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:old",
        "Đỗ Văn Hải",
        600,
        "synthetic/old.md",
        1,
        "unverified",
    );
    let incoming = insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:new",
        "Do Van Hai",
        30,
        "synthetic/doc-e.md",
        1,
        "unverified",
    );

    let preview =
        authority::preview(&mut client, 30, SHA_B, &ScopeSpec::document()).expect("preview");
    let applied = authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 30,
            content_sha256: SHA_B,
            spec: ScopeSpec::document(),
            reason: "Newer submission",
            actor: "reviewer-4",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("apply");
    assert!(!is_not_superseded(&mut client, previous));
    assert!(is_not_superseded(&mut client, incoming));

    let reversal = authority::reverse(
        &mut client,
        ReverseInput {
            decision_id: applied.decision_id,
            reason: "The earlier value was correct",
            actor: "reviewer-4",
        },
    )
    .expect("reverse");
    assert!(is_not_superseded(&mut client, previous));
    assert!(!is_not_superseded(&mut client, incoming));

    // Nothing was deleted: both decisions and both assertions remain readable.
    let history = authority::list_decisions(&mut client, Some("P400")).expect("history");
    let reversed = history
        .iter()
        .find(|decision| decision.decision_id == applied.decision_id)
        .expect("original decision is kept");
    assert_eq!(reversed.status, "reversed");
    let reversal_row = history
        .iter()
        .find(|decision| decision.decision_id == reversal.decision_id)
        .expect("reversal decision is kept");
    assert_eq!(reversal_row.kind, "reversal");
    assert_eq!(reversal_row.reverses_decision_id, Some(applied.decision_id));

    // Only the active decision may be reversed.
    assert!(matches!(
        authority::reverse(
            &mut client,
            ReverseInput {
                decision_id: applied.decision_id,
                reason: "again",
                actor: "reviewer-4",
            },
        ),
        Err(AuthorityError::Invalid(_))
    ));
    // A reversal needs a reason.
    assert!(matches!(
        authority::reverse(
            &mut client,
            ReverseInput {
                decision_id: reversal.decision_id,
                reason: "   ",
                actor: "reviewer-4",
            },
        ),
        Err(AuthorityError::Invalid(_))
    ));
    target.cleanup();
}

#[test]
fn a_second_decision_on_the_same_target_supersedes_the_first() {
    let Some((target, mut client)) = setup("authority_chain") else {
        return;
    };
    let person = insert_person(&mut client, "P700", "Hoàng Văn Tú");
    insert_extraction(&mut client, 60, "synthetic/doc-h.md", SHA_A);
    insert_extraction(&mut client, 61, "synthetic/doc-i.md", SHA_B);
    let first = insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:first",
        "Hoàng Văn Tú",
        60,
        "synthetic/doc-h.md",
        1,
        "unverified",
    );
    let second = insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:second",
        "Hoang Van Tu",
        61,
        "synthetic/doc-i.md",
        1,
        "unverified",
    );

    let person_scope = ScopeSpec {
        scope: "person",
        person_key: Some("P700"),
        assertion_ids: &[],
        remove_assertion_ids: &[],
    };
    let first_preview =
        authority::preview(&mut client, 60, SHA_A, &person_scope).expect("first preview");
    let first_decision = authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 60,
            content_sha256: SHA_A,
            spec: ScopeSpec {
                scope: "person",
                person_key: Some("P700"),
                assertion_ids: &[],
                remove_assertion_ids: &[],
            },
            reason: "First reviewed source",
            actor: "reviewer-7",
            preview_hash: &first_preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("first apply");
    assert!(is_not_superseded(&mut client, first));

    let preview =
        authority::preview(&mut client, 61, SHA_B, &person_scope).expect("second preview");
    assert_eq!(preview.items.len(), 1);
    assert_eq!(preview.items[0].action, "replaced");
    let second_decision = authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 61,
            content_sha256: SHA_B,
            spec: ScopeSpec {
                scope: "person",
                person_key: Some("P700"),
                assertion_ids: &[],
                remove_assertion_ids: &[],
            },
            reason: "Correction from the second source",
            actor: "reviewer-8",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("second apply");
    assert!(!is_not_superseded(&mut client, first));
    assert!(is_not_superseded(&mut client, second));

    let history = authority::list_decisions(&mut client, Some("P700")).expect("history");
    let first_row = history
        .iter()
        .find(|row| row.decision_id == first_decision.decision_id)
        .expect("first decision is kept");
    assert_eq!(first_row.status, "superseded");
    let second_row = history
        .iter()
        .find(|row| row.decision_id == second_decision.decision_id)
        .expect("second decision is kept");
    assert_eq!(second_row.status, "active");
    assert_eq!(
        second_row.supersedes_decision_id,
        Some(first_decision.decision_id)
    );

    // Reversing the second decision brings the first source back without
    // deleting either decision.
    authority::reverse(
        &mut client,
        ReverseInput {
            decision_id: second_decision.decision_id,
            reason: "The first source was right",
            actor: "reviewer-8",
        },
    )
    .expect("reverse the second decision");
    assert!(is_not_superseded(&mut client, first));
    assert!(!is_not_superseded(&mut client, second));
    target.cleanup();
}

#[test]
fn scope_validation_refuses_ambiguous_or_out_of_scope_requests() {
    let Some((target, mut client)) = setup("authority_validation") else {
        return;
    };
    let person = insert_person(&mut client, "P500", "Vũ Thị Hoa");
    insert_extraction(&mut client, 40, "synthetic/doc-f.md", SHA_C);
    insert_assertion(
        &mut client,
        person,
        "name_variant",
        "variant:hoa",
        "hoa",
        40,
        "synthetic/doc-f.md",
        1,
        "unverified",
    );

    assert!(authority::preview(&mut client, 40, SHA_C, &ScopeSpec::document()).is_ok());
    assert!(matches!(
        authority::preview(
            &mut client,
            40,
            SHA_C,
            &ScopeSpec {
                scope: "everything",
                ..ScopeSpec::document()
            }
        ),
        Err(AuthorityError::Invalid(_))
    ));
    assert!(matches!(
        authority::preview(
            &mut client,
            40,
            SHA_C,
            &ScopeSpec {
                scope: "person",
                person_key: None,
                assertion_ids: &[],
                remove_assertion_ids: &[],
            }
        ),
        Err(AuthorityError::Invalid(_))
    ));
    assert!(matches!(
        authority::preview(
            &mut client,
            40,
            SHA_C,
            &ScopeSpec {
                scope: "person",
                person_key: Some("P999"),
                assertion_ids: &[],
                remove_assertion_ids: &[],
            }
        ),
        Err(AuthorityError::Invalid(_))
    ));
    assert!(matches!(
        authority::preview(
            &mut client,
            40,
            SHA_C,
            &ScopeSpec {
                scope: "assertions",
                person_key: None,
                assertion_ids: &[],
                remove_assertion_ids: &[],
            }
        ),
        Err(AuthorityError::Invalid(_))
    ));
    assert!(matches!(
        authority::preview(
            &mut client,
            40,
            SHA_C,
            &ScopeSpec {
                scope: "assertions",
                person_key: None,
                assertion_ids: &[999_999],
                remove_assertion_ids: &[],
            }
        ),
        Err(AuthorityError::Invalid(_))
    ));
    // An unknown document version has no reviewed extraction.
    assert!(matches!(
        authority::preview(&mut client, 77, SHA_C, &ScopeSpec::document()),
        Err(AuthorityError::NotFound)
    ));
    // A decision needs explicit confirmation and a reason.
    assert!(matches!(
        authority::apply(
            &mut client,
            ApplyInput {
                document_version_id: 40,
                content_sha256: SHA_C,
                spec: ScopeSpec::document(),
                reason: "",
                actor: "reviewer-5",
                preview_hash: "deadbeef",
                confirm: true,
                acknowledge_conflicts: false,
            }
        ),
        Err(AuthorityError::Invalid(_))
    ));
    target.cleanup();
}

#[test]
fn the_tree_read_uses_the_authoritative_value() {
    let Some((target, mut client)) = setup("authority_tree") else {
        return;
    };
    let person = insert_person(&mut client, "P600", "Bùi Văn Nam");
    insert_extraction(&mut client, 50, "synthetic/doc-g.md", SHA_A);
    let previous = insert_assertion(
        &mut client,
        person,
        "birth_date",
        "date:1880",
        "1880",
        500,
        "synthetic/old.md",
        2,
        "verified",
    );
    let incoming = insert_assertion(
        &mut client,
        person,
        "birth_date",
        "date:1881",
        "1881",
        50,
        "synthetic/doc-g.md",
        2,
        "verified",
    );
    // Give the incoming date the structured columns the tree projection reads.
    client
        .execute(
            "UPDATE assertion SET value_date_start = '1881', value_date_precision = 'year',
                    value_date_qualifier = 'exact' WHERE assertion_id = $1",
            &[&incoming],
        )
        .expect("set structured date");
    client
        .execute(
            "UPDATE assertion SET value_date_start = '1880', value_date_precision = 'year',
                    value_date_qualifier = 'exact' WHERE assertion_id = $1",
            &[&previous],
        )
        .expect("set structured date");

    let preview =
        authority::preview(&mut client, 50, SHA_A, &ScopeSpec::document()).expect("preview");
    authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 50,
            content_sha256: SHA_A,
            spec: ScopeSpec::document(),
            reason: "Correct the birth year",
            actor: "reviewer-6",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("apply");

    let tree = load_tree(&mut client, false).expect("tree");
    let node = tree
        .people
        .iter()
        .find(|node| node.person_key == "P600")
        .expect("person is in the tree");
    assert_eq!(
        node.birth.as_ref().map(|date| date.start.as_str()),
        Some("1881")
    );
    target.cleanup();
}

#[test]
fn a_withdrawn_source_cannot_verify_an_assertion() {
    let Some((target, mut client)) = setup("authority_withdraw_verify") else {
        return;
    };
    let person = insert_person(&mut client, "P850", "Phan Văn Hùng");
    insert_extraction(&mut client, 95, "synthetic/doc-o.md", SHA_C);
    let withdrawn_source = insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:o",
        "Phan Văn Hùng",
        95,
        "synthetic/doc-o.md",
        1,
        "unverified",
    );
    let live_source = insert_assertion(
        &mut client,
        person,
        "name_variant",
        "variant:hung",
        "hung.phan",
        900,
        "synthetic/catalog.md",
        2,
        "unverified",
    );
    assert_eq!(withdraw(&mut client, 95, "synthetic/doc-o.md"), 1);

    // The withdrawn document no longer counts as support, so the verification
    // gate refuses rather than accepting a claim the archive stopped publishing.
    let refused = review_assertion(
        &mut client,
        &ReviewRequest {
            assertion_id: withdrawn_source,
            new_status: ReviewStatus::Verified,
            reviewer: "reviewer-14",
            evidence_ref: "synthetic/doc-o.md",
            note: None,
        },
    );
    assert!(matches!(refused, Err(ReviewError::NoEvidence(id)) if id == withdrawn_source));
    assert_eq!(review_status(&mut client, withdrawn_source), "unverified");
    // The row is untouched: the reviewer's attempt wrote no review event.
    let events: i64 = client
        .query_one(
            "SELECT count(*) FROM assertion_review_event WHERE assertion_id = $1",
            &[&withdrawn_source],
        )
        .expect("count review events")
        .get(0);
    assert_eq!(events, 0);
    // The unrelated assertion is not affected by another source's withdrawal.
    assert_eq!(review_status(&mut client, live_source), "unverified");
    target.cleanup();
}

// --- document withdrawal ---------------------------------------------------
//
// A removed document stops supporting answers and badges. The authority
// decision, its items, and every assertion row stay; nothing is deleted and no
// review status changes. These tests check the four withdrawal positions:
// before an apply, during an apply, after an apply, and around a reversal.

#[test]
fn withdrawal_before_apply_is_refused_and_writes_nothing() {
    let Some((target, mut client)) = setup("authority_withdraw_before") else {
        return;
    };
    let person = insert_person(&mut client, "P800", "Ngô Thị Bích");
    insert_extraction(&mut client, 90, "synthetic/doc-j.md", SHA_A);
    insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:j",
        "Ngô Thị Bích",
        90,
        "synthetic/doc-j.md",
        1,
        "unverified",
    );

    let spec = ScopeSpec::document();
    let preview = authority::preview(&mut client, 90, SHA_A, &spec).expect("preview");
    assert_eq!(preview.items.len(), 1);

    // The reviewer removes the source between the preview and the apply.
    assert_eq!(withdraw(&mut client, 90, "synthetic/doc-j.md"), 1);

    assert!(matches!(
        authority::preview(&mut client, 90, SHA_A, &ScopeSpec::document()),
        Err(AuthorityError::Withdrawn)
    ));
    let refused = authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 90,
            content_sha256: SHA_A,
            spec: ScopeSpec::document(),
            reason: "Should never be written",
            actor: "reviewer-9",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    );
    assert!(matches!(refused, Err(AuthorityError::Withdrawn)));

    // Nothing was written: no decision, no item, no event.
    let decisions: i64 = client
        .query_one("SELECT count(*) FROM authority_decision", &[])
        .expect("count decisions")
        .get(0);
    assert_eq!(decisions, 0);
    target.cleanup();
}

#[test]
fn withdrawal_and_apply_share_one_source_lock() {
    let Some((target, mut client)) = setup("authority_withdraw_lock") else {
        return;
    };
    let person = insert_person(&mut client, "P810", "Lý Văn Tài");
    insert_extraction(&mut client, 91, "synthetic/doc-k.md", SHA_B);
    insert_assertion(
        &mut client,
        person,
        "name_variant",
        "variant:tai",
        "tai.ly",
        91,
        "synthetic/doc-k.md",
        2,
        "unverified",
    );
    let preview =
        authority::preview(&mut client, 91, SHA_B, &ScopeSpec::document()).expect("preview");

    // Hold the source lock the way an in-flight apply or withdrawal does.
    let mut gate =
        postgres::Client::connect(&target.dsn, postgres::NoTls).expect("second connection");
    gate.batch_execute("BEGIN")
        .expect("begin the gate transaction");
    gate.batch_execute("SELECT pg_advisory_xact_lock(hashtext('authority-source:91'))")
        .expect("take the source lock");

    // An apply on the same source must wait for that lock instead of reading a
    // half-changed state.
    let (tx, rx) = mpsc::channel();
    let dsn = target.dsn.clone();
    let hash = preview.preview_hash.clone();
    let handle = std::thread::spawn(move || {
        let mut worker =
            postgres::Client::connect(&dsn, postgres::NoTls).expect("apply connection");
        let result = authority::apply(
            &mut worker,
            ApplyInput {
                document_version_id: 91,
                content_sha256: SHA_B,
                spec: ScopeSpec::document(),
                reason: "Applied after the lock is released",
                actor: "reviewer-10",
                preview_hash: &hash,
                confirm: true,
                acknowledge_conflicts: true,
            },
        );
        let _ = tx.send(result.map(|report| report.decision_id));
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        rx.try_recv().is_err(),
        "apply must block while another transaction holds the source lock"
    );
    gate.batch_execute("COMMIT")
        .expect("release the source lock");
    let applied = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("apply completes after the lock is released")
        .expect("apply succeeds");
    handle.join().expect("apply thread");
    assert!(applied > 0);

    // The withdrawal takes the same lock, so it cannot land in the middle of an
    // apply either.
    gate.batch_execute("BEGIN")
        .expect("begin the second gate transaction");
    gate.batch_execute("SELECT pg_advisory_xact_lock(hashtext('authority-source:91'))")
        .expect("take the source lock again");
    let (wtx, wrx) = mpsc::channel();
    let wdsn = target.dsn.clone();
    let withdrawal = std::thread::spawn(move || {
        let mut worker =
            postgres::Client::connect(&wdsn, postgres::NoTls).expect("withdrawal connection");
        let result = withdraw_document_evidence(&mut worker, 91, "synthetic/doc-k.md");
        let _ = wtx.send(result);
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        wrx.try_recv().is_err(),
        "withdrawal must block while another transaction holds the source lock"
    );
    gate.batch_execute("COMMIT")
        .expect("release the source lock");
    let withdrawn = wrx
        .recv_timeout(Duration::from_secs(10))
        .expect("withdrawal completes after the lock is released")
        .expect("withdrawal succeeds");
    withdrawal.join().expect("withdrawal thread");
    assert_eq!(withdrawn, 1);

    target.cleanup();
}

#[test]
fn withdrawal_after_apply_stops_every_current_view_and_flags_the_decision() {
    let Some((target, mut client)) = setup("authority_withdraw_after") else {
        return;
    };
    let person = insert_person(&mut client, "P820", "Đặng Thị Hoa");
    let parent = insert_person(&mut client, "P821", "Đặng Văn Lộc");
    insert_extraction(&mut client, 92, "synthetic/doc-l.md", SHA_C);

    // A previously reviewed display name from the catalog, not from this upload.
    let catalog_name = insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:catalog",
        "Đặng Thị Hoa",
        900,
        "synthetic/catalog.md",
        1,
        "verified",
    );
    // Everything the uploaded document supports.
    let alias = insert_assertion(
        &mut client,
        person,
        "name_variant",
        "variant:hoa",
        "hoaarchive",
        92,
        "synthetic/doc-l.md",
        2,
        "unverified",
    );
    let birth = insert_assertion(
        &mut client,
        person,
        "birth_date",
        "date:1899",
        "1899",
        92,
        "synthetic/doc-l.md",
        3,
        "verified",
    );
    set_birth_date(&mut client, birth, "1899");
    let branch_claim = insert_assertion(
        &mut client,
        person,
        KIND_BRANCH,
        "branch:three",
        "third",
        92,
        "synthetic/doc-l.md",
        5,
        "verified",
    );
    let link = insert_parent_link(
        &mut client,
        parent,
        "Đặng Văn Lộc",
        person,
        92,
        "synthetic/doc-l.md",
        "verified",
    );
    // The document also replaces the display name.
    let display = insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:upload",
        "Dang Thi Hoa",
        92,
        "synthetic/doc-l.md",
        1,
        "unverified",
    );

    let preview =
        authority::preview(&mut client, 92, SHA_C, &ScopeSpec::document()).expect("preview");
    let decision = authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 92,
            content_sha256: SHA_C,
            spec: ScopeSpec::document(),
            reason: "The uploaded document is current",
            actor: "reviewer-11",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("apply");
    assert!(authority::is_current(&mut client, alias).expect("current check"));
    assert!(!authority::is_current(&mut client, catalog_name).expect("current check"));
    assert_eq!(review_status(&mut client, catalog_name), "verified");

    // Before the withdrawal every current path agrees the document is live.
    let tree = load_tree(&mut client, false).expect("tree before");
    let node = tree
        .people
        .iter()
        .find(|node| node.person_key == "P820")
        .expect("person in tree");
    assert!(node.aliases.contains(&"hoaarchive".to_string()));
    assert_eq!(
        node.birth.as_ref().map(|date| date.start.as_str()),
        Some("1899")
    );
    assert!(tree
        .links
        .iter()
        .any(|edge| edge.from_key == "P821" && edge.to_key == "P820"));
    let catalog = person_search::load_catalog(&mut client).expect("catalog before");
    assert!(!rank(&catalog.people, "hoaarchive").is_empty());
    assert!(eligibility::load_input(&mut client, "P820", "three")
        .expect("eligibility before")
        .branch
        .is_some());

    // The source document is removed.
    assert_eq!(withdraw(&mut client, 92, "synthetic/doc-l.md"), 5);

    // Person detail: none of the uploaded assertions survive as current.
    let detail = lookup_person(&mut client, "P820")
        .expect("person detail")
        .expect("person exists");
    let kinds: Vec<&str> = detail
        .assertions
        .iter()
        .map(|assertion| assertion.kind.as_str())
        .collect();
    assert!(!kinds.contains(&"name_variant"));
    assert!(!kinds.contains(&"birth_date"));
    assert!(!kinds.contains(&"parent_child"));
    assert!(!kinds.contains(&KIND_BRANCH));
    assert!(!kinds.contains(&"display_name"));

    // Tree: alias, date, and edge all disappear.
    let tree = load_tree(&mut client, false).expect("tree after");
    let node = tree
        .people
        .iter()
        .find(|node| node.person_key == "P820")
        .expect("person in tree");
    assert!(node.aliases.is_empty());
    assert!(node.birth.is_none());
    assert!(
        !tree
            .links
            .iter()
            .any(|edge| edge.from_key == "P821" && edge.to_key == "P820"),
        "an edge without live evidence must not be drawn"
    );
    assert_eq!(node.evidence_status, "unverified");

    // Search: the alias is no longer a way to find the person.
    let catalog = person_search::load_catalog(&mut client).expect("catalog after");
    assert!(rank(&catalog.people, "hoaarchive").is_empty());

    // Eligibility: the withdrawn branch claim is not eligible evidence.
    assert!(eligibility::load_input(&mut client, "P820", "three")
        .expect("eligibility after")
        .branch
        .is_none());

    // Review status is unchanged and nothing was restored or deleted.
    assert_eq!(review_status(&mut client, alias), "unverified");
    assert_eq!(review_status(&mut client, birth), "verified");
    assert_eq!(review_status(&mut client, link), "verified");
    assert_eq!(review_status(&mut client, branch_claim), "verified");
    assert!(!is_not_superseded(&mut client, catalog_name));
    let still_stored: i64 = client
        .query_one(
            "SELECT count(*) FROM assertion_evidence
              WHERE assertion_id = $1 AND withdrawn_at IS NOT NULL",
            &[&alias],
        )
        .expect("count withdrawn evidence")
        .get(0);
    assert_eq!(still_stored, 1);

    // The decision is preserved, still active, and reported as needing
    // attention with the assertions that lost their support.
    let decisions = authority::list_decisions(&mut client, None).expect("history");
    let row = decisions
        .iter()
        .find(|row| row.decision_id == decision.decision_id)
        .expect("decision is kept");
    assert_eq!(row.status, "active");
    assert!(row.needs_attention);
    let flagged: Vec<i64> = row
        .unsupported_assertions
        .iter()
        .map(|assertion| assertion.assertion_id)
        .collect();
    assert!(flagged.contains(&alias));
    assert!(flagged.contains(&display));
    // The history view labels the withdrawn citation instead of hiding it.
    let alias_view = row
        .unsupported_assertions
        .iter()
        .find(|assertion| assertion.assertion_id == alias)
        .expect("alias is flagged");
    assert!(!alias_view.citations.is_empty());
    assert!(alias_view
        .citations
        .iter()
        .all(|citation| citation.withdrawn));

    target.cleanup();
}

#[test]
fn reversal_after_withdrawal_restores_the_previous_version() {
    let Some((target, mut client)) = setup("authority_withdraw_reverse") else {
        return;
    };
    let person = insert_person(&mut client, "P830", "Trịnh Văn Quang");
    insert_extraction(&mut client, 93, "synthetic/doc-m.md", SHA_A);
    let previous = insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:old",
        "Trịnh Văn Quang",
        901,
        "synthetic/older.md",
        1,
        "verified",
    );
    insert_assertion(
        &mut client,
        person,
        "display_name",
        "display:new",
        "Trinh Van Quang",
        93,
        "synthetic/doc-m.md",
        1,
        "unverified",
    );

    let preview =
        authority::preview(&mut client, 93, SHA_A, &ScopeSpec::document()).expect("preview");
    let decision = authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 93,
            content_sha256: SHA_A,
            spec: ScopeSpec::document(),
            reason: "Newer source",
            actor: "reviewer-12",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("apply");
    assert_eq!(withdraw(&mut client, 93, "synthetic/doc-m.md"), 1);
    assert!(!authority::is_current(&mut client, previous).expect("current check"));

    let decided = authority::list_decisions(&mut client, None).expect("history");
    assert!(
        decided
            .iter()
            .find(|row| row.decision_id == decision.decision_id)
            .expect("decision kept")
            .needs_attention
    );

    let reversal = authority::reverse(
        &mut client,
        ReverseInput {
            decision_id: decision.decision_id,
            reason: "The newer source was withdrawn",
            actor: "reviewer-12",
        },
    )
    .expect("reverse");
    // The previous version had its own live evidence, so it is current again.
    assert!(authority::is_current(&mut client, previous).expect("current check"));
    assert_eq!(review_status(&mut client, previous), "verified");
    let detail = lookup_person(&mut client, "P830")
        .expect("person detail")
        .expect("person exists");
    assert_eq!(
        detail
            .assertions
            .iter()
            .filter(|assertion| assertion.kind == "display_name")
            .map(|assertion| assertion.value_text.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["Trịnh Văn Quang".to_string()]
    );
    // The reversal is the active decision and it does not need attention, while
    // the reversed decision keeps its history.
    let history = authority::list_decisions(&mut client, None).expect("history");
    let reversal_row = history
        .iter()
        .find(|row| row.decision_id == reversal.decision_id)
        .expect("reversal kept");
    assert_eq!(reversal_row.status, "active");
    assert!(!reversal_row.needs_attention);
    let reversed_row = history
        .iter()
        .find(|row| row.decision_id == decision.decision_id)
        .expect("reversed decision kept");
    assert_eq!(reversed_row.status, "reversed");
    target.cleanup();
}

#[test]
fn current_views_agree_on_aliases_dates_and_relationships_after_withdrawal() {
    let Some((target, mut client)) = setup("authority_withdraw_agree") else {
        return;
    };
    let child = insert_person(&mut client, "P840", "Mai Thị Lan");
    let parent = insert_person(&mut client, "P841", "Mai Văn Sơn");
    insert_extraction(&mut client, 94, "synthetic/doc-n.md", SHA_B);
    let alias = insert_assertion(
        &mut client,
        child,
        "name_variant",
        "variant:lan",
        "lanarchive",
        94,
        "synthetic/doc-n.md",
        1,
        "unverified",
    );
    let birth = insert_assertion(
        &mut client,
        child,
        "birth_date",
        "date:1902",
        "1902",
        94,
        "synthetic/doc-n.md",
        2,
        "verified",
    );
    set_birth_date(&mut client, birth, "1902");
    insert_parent_link(
        &mut client,
        parent,
        "Mai Văn Sơn",
        child,
        94,
        "synthetic/doc-n.md",
        "verified",
    );

    let preview =
        authority::preview(&mut client, 94, SHA_B, &ScopeSpec::document()).expect("preview");
    authority::apply(
        &mut client,
        ApplyInput {
            document_version_id: 94,
            content_sha256: SHA_B,
            spec: ScopeSpec::document(),
            reason: "Current source",
            actor: "reviewer-13",
            preview_hash: &preview.preview_hash,
            confirm: true,
            acknowledge_conflicts: true,
        },
    )
    .expect("apply");
    assert_eq!(withdraw(&mut client, 94, "synthetic/doc-n.md"), 3);

    // One snapshot of every current path.
    let detail = lookup_person(&mut client, "P840")
        .expect("person detail")
        .expect("person exists");
    let tree = load_tree(&mut client, false).expect("tree");
    let node = tree
        .people
        .iter()
        .find(|node| node.person_key == "P840")
        .expect("person in tree");
    let catalog = person_search::load_catalog(&mut client).expect("catalog");
    let person_names = catalog
        .people
        .iter()
        .find(|person| person.person_key == "P840")
        .expect("person in catalog");
    let parents = person_search::recorded_parents(&mut client, &["P840".to_string()])
        .expect("recorded parents");
    let input = eligibility::load_input(&mut client, "P840", "missing").expect("eligibility");

    let detail_kinds: Vec<&str> = detail
        .assertions
        .iter()
        .map(|assertion| assertion.kind.as_str())
        .collect();
    assert!(!detail_kinds.contains(&"name_variant"));
    assert!(!detail_kinds.contains(&"birth_date"));
    assert!(!detail_kinds.contains(&"parent_child"));

    assert!(node.aliases.is_empty());
    assert!(node.birth.is_none());
    assert!(person_names.aliases.is_empty());
    assert!(rank(&catalog.people, "lanarchive").is_empty());
    assert!(!parents.contains_key("P840"));
    assert!(tree
        .links
        .iter()
        .all(|edge| edge.from_key != "P841" && edge.to_key != "P841"));
    // Nothing in the withdrawn snapshot is offered as verified evidence.
    assert!(input.branch.is_none() && input.gender.is_none() && input.birth_order.is_none());

    // The review queue still shows the candidate, as an authorized review view.
    let unresolved: Vec<i64> = unresolved_assertions(&mut client)
        .expect("review queue")
        .into_iter()
        .map(|row| row.assertion_id)
        .collect();
    assert!(unresolved.contains(&alias));
    target.cleanup();
}
