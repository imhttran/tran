//! TRAN-002I: source-linked historical narratives.
//!
//! Synthetic fixtures only. Each suite gets a disposable genealogy database
//! derived from `GENEALOGY_DATABASE_URL`.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn, create_isolated_target, IsolatedTarget};
use postgres::Client;
use rag_foundation::genealogy::narrative::{
    self, FamilyPassage, ImportReport, PassageDecision, PassageVisibility,
};

const SOURCE: &str = "synthetic/narrative-vi.md";
const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

const TEXT: &str = "\
# Synthetic family history

Front matter.

## Trang PDF 8

### Đời thứ 12: SYNTHETIC ANCESTOR ONE (1877–1922)

First paragraph about the ancestor.

Second paragraph mentioning Synthetic Ancestor Two by name.

## Trang PDF 11

### Đời thứ 13: SYNTHETIC ANCESTOR TWO

Only paragraph for the second person.
";

const ENGLISH_TEXT: &str = "\
# Synthetic English family history

Front matter.

## PDF page 12

### SYNTHETIC ANCESTOR ONE

English paragraph with paragraph one.\n\nAnd paragraph two.
";

fn target() -> Option<IsolatedTarget> {
    let base = base_dsn()?;
    let target = create_isolated_target(&base, "narrative");
    target.apply_migrations();
    Some(target)
}

fn seed_people(client: &mut Client) -> (i64, i64) {
    let one: i64 = client
        .query_one(
            "INSERT INTO person (person_key, display_name)
             VALUES ('synthetic-one', 'Synthetic Ancestor One') RETURNING person_id",
            &[],
        )
        .expect("person one")
        .get(0);
    let two: i64 = client
        .query_one(
            "INSERT INTO person (person_key, display_name)
             VALUES ('synthetic-two', 'Synthetic Ancestor Two') RETURNING person_id",
            &[],
        )
        .expect("person two")
        .get(0);
    (one, two)
}

fn import(client: &mut Client, text: &str, sha: &str) -> ImportReport {
    let parse = narrative::parse_narrative(text, SOURCE);
    let people = narrative::linkable_people(client).expect("people");
    let mut tx = client.transaction().expect("tx");
    let report =
        narrative::import_narrative(&mut tx, SOURCE, sha, "translation", "vi", &parse, &people)
            .expect("import");
    tx.commit().expect("commit");
    report
}

fn history(client: &mut Client, person_id: i64) -> Vec<FamilyPassage> {
    let mut tx = client.transaction().expect("tx");
    let passages = narrative::family_history(&mut tx, person_id).expect("history");
    tx.commit().expect("commit");
    passages
}

fn passage_ids(client: &mut Client) -> Vec<(i64, i32, String)> {
    client
        .query(
            "SELECT passage_id, page_start, review_status
               FROM source_narrative_passage ORDER BY passage_id",
            &[],
        )
        .expect("passages")
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect()
}

#[test]
fn an_english_pdf_page_marker_imports_with_its_page_range() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping English narrative test");
        return;
    };
    let mut client = target.connect();
    seed_people(&mut client);

    let source = "synthetic/narrative-en.md";
    let parse = narrative::parse_narrative(ENGLISH_TEXT, source);
    assert_eq!(
        parse.passages.len(),
        1,
        "the English page marker starts a passage"
    );
    assert_eq!(parse.passages[0].page_start, 12);
    assert_eq!(parse.passages[0].page_end, 12);

    let people = narrative::linkable_people(&mut client).expect("people");
    let mut tx = client.transaction().expect("tx");
    let report =
        narrative::import_narrative(&mut tx, source, SHA, "translation", "en", &parse, &people)
            .expect("import");
    tx.commit().expect("commit");
    assert_eq!(report.inserted, 1);
    assert_eq!(
        report.links_inserted, 1,
        "the heading names exactly one catalog person"
    );

    let rows = client
        .query(
            "SELECT language, page_start, page_end FROM source_narrative_passage
              WHERE source_document_id = (
                SELECT source_document_id FROM source_document WHERE relative_path = $1
              )",
            &[&source],
        )
        .expect("English passage");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, String>(0), "en");
    assert_eq!(rows[0].get::<_, i32>(1), 12);
    assert_eq!(rows[0].get::<_, i32>(2), 12);
}

#[test]
fn published_history_counts_agree_with_the_person_read() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping history count test");
        return;
    };
    let mut client = target.connect();
    let (one, two) = seed_people(&mut client);
    import(&mut client, TEXT, SHA);
    let first_id = passage_ids(&mut client)[0].0;

    narrative::review_passage(
        &mut client,
        first_id,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        None,
    )
    .expect("approve");

    let keys = vec!["synthetic-one".to_string(), "synthetic-two".to_string()];
    let counts = narrative::published_history_counts(&mut client, &keys).expect("counts");
    assert_eq!(counts.get("synthetic-one"), Some(&1));
    assert_eq!(counts.get("synthetic-two"), None, "no published passage");
    assert_eq!(history(&mut client, one).len() as i64, 1);
    assert_eq!(
        history(&mut client, two).len() as i64,
        counts.get("synthetic-two").copied().unwrap_or(0)
    );
}

#[test]
fn a_second_unchanged_import_creates_no_duplicates() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative import test");
        return;
    };
    let mut client = target.connect();
    seed_people(&mut client);

    let first = import(&mut client, TEXT, SHA);
    assert_eq!(first.inserted, 2, "one passage per page/section");
    assert_eq!(
        first.links_inserted, 3,
        "two heading links and one body mention"
    );
    assert_eq!(first.unchanged, 0);

    let second = import(&mut client, TEXT, SHA);
    assert_eq!(second.inserted, 0, "no duplicate passages");
    assert_eq!(second.updated, 0);
    assert_eq!(second.revisions, 0);
    assert_eq!(second.unchanged, 2);
    assert_eq!(second.links_inserted, 0, "no duplicate links");
    assert_eq!(second.links_kept, 3);

    assert_eq!(passage_ids(&mut client).len(), 2);
}

#[test]
fn a_section_heading_resolves_and_a_mention_waits_for_review() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative link test");
        return;
    };
    let mut client = target.connect();
    let (one, two) = seed_people(&mut client);
    import(&mut client, TEXT, SHA);

    let rows = client
        .query(
            "SELECT p.page_start, l.link_status, l.match_kind
               FROM source_narrative_link l
               JOIN source_narrative_passage p ON p.passage_id = l.passage_id
               JOIN person pe ON pe.person_id = l.person_id
              WHERE pe.person_id = $1 ORDER BY p.page_start",
            &[&one],
        )
        .expect("one links");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, i32>(0), 8);
    assert_eq!(rows[0].get::<_, String>(1), "resolved");
    assert_eq!(rows[0].get::<_, String>(2), "section_heading");

    let rows = client
        .query(
            "SELECT p.page_start, l.link_status, l.match_kind
               FROM source_narrative_link l
               JOIN source_narrative_passage p ON p.passage_id = l.passage_id
               JOIN person pe ON pe.person_id = l.person_id
              WHERE pe.person_id = $1 ORDER BY p.page_start",
            &[&two],
        )
        .expect("two links");
    assert_eq!(rows.len(), 2, "heading link and body mention");
    assert_eq!(
        rows[0].get::<_, String>(1),
        "ambiguous",
        "the page-8 mention"
    );
    assert_eq!(
        rows[1].get::<_, String>(1),
        "resolved",
        "the page-11 heading"
    );
}

#[test]
fn only_an_approved_family_passage_reaches_the_person() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative publication test");
        return;
    };
    let mut client = target.connect();
    let (one, _two) = seed_people(&mut client);
    import(&mut client, TEXT, SHA);
    let passages = passage_ids(&mut client);
    let first_id = passages[0].0;

    // Imported passages start unpublished.
    assert!(history(&mut client, one).is_empty());

    // Family visibility needs an accepted link; the heading link is resolved.
    narrative::review_passage(
        &mut client,
        first_id,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        Some("checked against the source"),
    )
    .expect("approve");

    let passages = history(&mut client, one);
    assert_eq!(passages.len(), 1);
    assert_eq!(passages[0].language, "vi");
    assert_eq!(passages[0].document_ref, SOURCE);
    assert_eq!(passages[0].page_start, 8);
    assert_eq!(passages[0].page_end, 8);
    assert!(passages[0]
        .body
        .contains("First paragraph about the ancestor."));
    assert!(
        passages[0].body.contains("\n\n"),
        "paragraph breaks survive"
    );

    // Approving the passage did not create any member profile contribution.
    let contributions: i64 = client
        .query_one("SELECT count(*) FROM onboarding_profile_value", &[])
        .expect("profile values")
        .get(0);
    assert_eq!(contributions, 0);
}

#[test]
fn pending_rejected_admin_only_and_restricted_passages_stay_private() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative privacy test");
        return;
    };
    let mut client = target.connect();
    let (one, _two) = seed_people(&mut client);
    import(&mut client, TEXT, SHA);
    let passages = passage_ids(&mut client);
    let first_id = passages[0].0;

    // Pending.
    assert!(history(&mut client, one).is_empty());

    // Approved but admin-only.
    narrative::review_passage(
        &mut client,
        first_id,
        PassageDecision::Approved,
        Some(PassageVisibility::AdminOnly),
        "synthetic-reviewer",
        None,
    )
    .expect("approve admin only");
    assert!(history(&mut client, one).is_empty());

    // Rejected.
    narrative::review_passage(
        &mut client,
        first_id,
        PassageDecision::Rejected,
        None,
        "synthetic-reviewer",
        None,
    )
    .expect("reject");
    assert!(history(&mut client, one).is_empty());

    // Approved for the family, then restricted back to administrators.
    narrative::review_passage(
        &mut client,
        first_id,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        None,
    )
    .expect("approve family");
    assert_eq!(history(&mut client, one).len(), 1);
    narrative::review_passage(
        &mut client,
        first_id,
        PassageDecision::Approved,
        Some(PassageVisibility::AdminOnly),
        "synthetic-reviewer",
        Some("owner asked to restrict"),
    )
    .expect("restrict");
    assert!(history(&mut client, one).is_empty());
}

#[test]
fn an_ambiguous_link_publishes_only_after_confirmation() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative link review test");
        return;
    };
    let mut client = target.connect();
    let (_one, two) = seed_people(&mut client);
    import(&mut client, TEXT, SHA);
    let passages = passage_ids(&mut client);
    // Page 8 mentions Ancestor Two in the body only.
    let first_id = passages[0].0;

    narrative::review_passage(
        &mut client,
        first_id,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        None,
    )
    .expect("approve");

    // The ambiguous mention is not enough on its own.
    assert!(history(&mut client, two).is_empty());

    narrative::review_link(
        &mut client,
        first_id,
        "synthetic-two",
        true,
        "synthetic-reviewer",
        Some("the paragraph names him"),
    )
    .expect("confirm link");
    assert_eq!(history(&mut client, two).len(), 1);
}

#[test]
fn family_visibility_requires_an_accepted_link() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative guard test");
        return;
    };
    let mut client = target.connect();
    seed_people(&mut client);
    import(&mut client, TEXT, SHA);
    let second_id = passage_ids(&mut client)[1].0;

    // Reject every link on the second passage, then try to publish it.
    client
        .execute(
            "UPDATE source_narrative_link SET link_status = 'rejected',
                    reviewed_by = 'synthetic-reviewer', reviewed_at = now()
              WHERE passage_id = $1",
            &[&second_id],
        )
        .expect("reject links");

    let refused = narrative::review_passage(
        &mut client,
        second_id,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        None,
    );
    assert!(
        refused.is_err(),
        "publishing without an accepted link is refused"
    );
}

#[test]
fn a_changed_source_needs_review_before_it_replaces_published_text() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative revision test");
        return;
    };
    let mut client = target.connect();
    let (one, _two) = seed_people(&mut client);
    import(&mut client, TEXT, SHA);
    let first_id = passage_ids(&mut client)[0].0;
    narrative::review_passage(
        &mut client,
        first_id,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        None,
    )
    .expect("approve");

    // The source is corrected; the document hash and the paragraph both change.
    let changed = TEXT.replace(
        "First paragraph about the ancestor.",
        "Corrected first paragraph about the ancestor.",
    );
    let changed_sha = "b".repeat(64);
    let report = import(&mut client, &changed, &changed_sha);
    assert_eq!(report.revisions, 1, "a pending revision is added");
    assert_eq!(report.updated, 0, "the approved row is untouched");

    // The published text is still the reviewed one.
    let published = history(&mut client, one);
    assert_eq!(published.len(), 1);
    assert!(published[0]
        .body
        .contains("First paragraph about the ancestor."));
    assert!(!published[0].body.contains("Corrected"));

    // Approving the revision supersedes the old text without deleting it.
    let revision_id: i64 = client
        .query_one(
            "SELECT passage_id FROM source_narrative_passage
              WHERE review_status = 'pending' ORDER BY passage_id DESC LIMIT 1",
            &[],
        )
        .expect("revision")
        .get(0);
    narrative::review_passage(
        &mut client,
        revision_id,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        Some("source correction"),
    )
    .expect("approve revision");

    let published = history(&mut client, one);
    assert_eq!(published.len(), 1, "one current passage per slot");
    assert!(published[0].body.contains("Corrected first paragraph"));
    let all: i64 = client
        .query_one("SELECT count(*) FROM source_narrative_passage", &[])
        .expect("count")
        .get(0);
    assert_eq!(all, 3, "the superseded row is kept for history");
}

#[test]
fn review_events_are_append_only() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative audit test");
        return;
    };
    let mut client = target.connect();
    seed_people(&mut client);
    import(&mut client, TEXT, SHA);
    let first_id = passage_ids(&mut client)[0].0;
    narrative::review_passage(
        &mut client,
        first_id,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        None,
    )
    .expect("approve");

    let events: i64 = client
        .query_one(
            "SELECT count(*) FROM source_narrative_review_event WHERE passage_id = $1",
            &[&first_id],
        )
        .expect("events")
        .get(0);
    assert_eq!(events, 1);

    let tampered = client.execute(
        "UPDATE source_narrative_review_event SET actor = 'someone-else'",
        &[],
    );
    assert!(tampered.is_err(), "review events cannot be rewritten");
}

#[test]
fn refreshing_pending_text_re_derives_person_links() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative relink test");
        return;
    };
    let mut client = target.connect();
    let (_one, _two) = seed_people(&mut client);
    import(&mut client, TEXT, SHA);
    let first_id = passage_ids(&mut client)[0].0;

    // A reviewer confirms the body mention while the passage is pending.
    narrative::review_link(
        &mut client,
        first_id,
        "synthetic-two",
        true,
        "synthetic-reviewer",
        None,
    )
    .expect("confirm link");

    // The source is corrected and no longer names that person.
    let corrected = TEXT.replace(
        "Second paragraph mentioning Synthetic Ancestor Two by name.",
        "Second paragraph with no additional names.",
    );
    let report = import(&mut client, &corrected, &"c".repeat(64));
    assert_eq!(
        report.updated, 1,
        "the pending passage is refreshed in place"
    );

    let surviving: i64 = client
        .query_one(
            "SELECT count(*) FROM source_narrative_link l
               JOIN person pe ON pe.person_id = l.person_id
              WHERE l.passage_id = $1 AND pe.person_key = 'synthetic-two'",
            &[&first_id],
        )
        .expect("links")
        .get(0);
    assert_eq!(
        surviving, 0,
        "a decision about the old wording does not survive the new text"
    );
}

#[test]
fn a_pending_revision_key_follows_its_body() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative revision key test");
        return;
    };
    let mut client = target.connect();
    let (_one, _two) = seed_people(&mut client);

    // A is published, then the source passes through B and C.
    import(&mut client, TEXT, SHA);
    let first_id = passage_ids(&mut client)[0].0;
    narrative::review_passage(
        &mut client,
        first_id,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        None,
    )
    .expect("approve A");

    let b = TEXT.replace("First paragraph about the ancestor.", "Body B.");
    import(&mut client, &b, &"b".repeat(64));
    let c = TEXT.replace("First paragraph about the ancestor.", "Body C.");
    import(&mut client, &c, &"c".repeat(64));

    let pending: i64 = client
        .query_one(
            "SELECT passage_id FROM source_narrative_passage
              WHERE review_status = 'pending' LIMIT 1",
            &[],
        )
        .expect("pending revision")
        .get(0);
    narrative::review_passage(
        &mut client,
        pending,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        None,
    )
    .expect("approve C");

    // Importing B again must not collide with a stale B-derived key.
    let report = import(&mut client, &b, &"b".repeat(64));
    assert_eq!(report.revisions, 1, "B becomes a new pending revision");
    let total: i64 = client
        .query_one("SELECT count(*) FROM source_narrative_passage", &[])
        .expect("count")
        .get(0);
    // The page-8 slot holds A, C, and the new B revision; the page-11 slot
    // still holds its single row.
    assert_eq!(total, 4, "three page-8 revisions and one page-11 passage");
}

#[test]
fn admin_review_queue_and_detail_show_candidates_and_context() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative review read test");
        return;
    };
    let mut client = target.connect();
    seed_people(&mut client);
    import(&mut client, TEXT, SHA);
    let first_id = passage_ids(&mut client)[0].0;

    let queue = narrative::review_queue(
        &mut client,
        &narrative::ReviewFilters {
            status: "pending".to_string(),
            language: Some("vi".to_string()),
            person_key: Some("synthetic-one".to_string()),
            unmatched_only: false,
            limit: 100,
        },
    )
    .expect("review queue");
    assert_eq!(queue.entries.len(), 1);
    assert!(!queue.truncated);
    let entry = &queue.entries[0];
    assert_eq!(entry.passage.passage_id, first_id);
    assert_eq!(entry.passage.document_ref, SOURCE);
    assert_eq!(entry.passage.page_start, 8);
    assert_eq!(entry.passage.page_end, 8);
    assert!(entry
        .passage
        .body
        .contains("First paragraph about the ancestor."));
    assert_eq!(entry.links.len(), 2);
    assert!(entry
        .links
        .iter()
        .any(|link| link.person_key == "synthetic-one" && link.link_status == "resolved"));
    assert!(entry
        .links
        .iter()
        .any(|link| link.person_key == "synthetic-two" && link.link_status == "ambiguous"));

    let detail = narrative::review_detail(&mut client, first_id)
        .expect("review detail")
        .expect("passage");
    assert_eq!(detail.passage.review_status, "pending");
    assert_eq!(detail.links.len(), 2);
    assert_eq!(detail.people.len(), 2);
    assert!(detail
        .people
        .iter()
        .any(|person| person.person_key == "synthetic-one"
            && person.display_name == "Synthetic Ancestor One"));
    assert!(
        detail.events.is_empty(),
        "import alone writes no review event"
    );

    let english = narrative::review_queue(
        &mut client,
        &narrative::ReviewFilters {
            status: "pending".to_string(),
            language: Some("en".to_string()),
            person_key: None,
            unmatched_only: false,
            limit: 100,
        },
    )
    .expect("English filter");
    assert!(english.entries.is_empty());

    let no_person = narrative::review_queue(
        &mut client,
        &narrative::ReviewFilters {
            status: "pending".to_string(),
            language: None,
            person_key: Some("synthetic-unknown".to_string()),
            unmatched_only: false,
            limit: 100,
        },
    )
    .expect("unknown person filter");
    assert!(no_person.entries.is_empty());

    let unmatched_source = "## Trang PDF 20\n\n### No known person here\n\nUnmatched paragraph.\n";
    let unmatched_parse =
        narrative::parse_narrative(unmatched_source, "synthetic/narrative-unmatched.md");
    let people = narrative::linkable_people(&mut client).expect("people");
    let mut tx = client.transaction().expect("tx");
    narrative::import_narrative(
        &mut tx,
        "synthetic/narrative-unmatched.md",
        &"d".repeat(64),
        "translation",
        "vi",
        &unmatched_parse,
        &people,
    )
    .expect("unmatched import");
    tx.commit().expect("commit");

    let unmatched = narrative::review_queue(
        &mut client,
        &narrative::ReviewFilters {
            status: "pending".to_string(),
            language: None,
            person_key: None,
            unmatched_only: true,
            limit: 100,
        },
    )
    .expect("unmatched filter");
    assert_eq!(unmatched.entries.len(), 1);
    assert!(unmatched.entries[0].links.is_empty());
}

#[test]
fn admin_review_actions_are_separate_and_reject_stale_state() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative review action test");
        return;
    };
    let mut client = target.connect();
    seed_people(&mut client);
    import(&mut client, TEXT, SHA);
    let first_id = passage_ids(&mut client)[0].0;
    let before = narrative::review_detail(&mut client, first_id)
        .expect("detail")
        .expect("passage");
    let old_version = before.passage.updated_at.clone();

    narrative::review_link_checked(
        &mut client,
        first_id,
        "synthetic-two",
        true,
        "synthetic-reviewer",
        Some("confirm the ambiguous mention"),
        Some("ambiguous"),
    )
    .expect("confirm link");
    let after_link = narrative::review_detail(&mut client, first_id)
        .expect("detail")
        .expect("passage");
    assert!(after_link
        .links
        .iter()
        .any(|link| link.person_key == "synthetic-two" && link.link_status == "confirmed"));
    assert_eq!(
        after_link.passage.review_status, "pending",
        "confirming a link must not approve the passage"
    );

    assert!(
        narrative::review_link_checked(
            &mut client,
            first_id,
            "synthetic-two",
            false,
            "synthetic-reviewer",
            None,
            Some("ambiguous"),
        )
        .is_err(),
        "a stale link decision must not overwrite the confirmed link"
    );

    narrative::review_passage_checked(
        &mut client,
        first_id,
        PassageDecision::Approved,
        Some(PassageVisibility::Family),
        "synthetic-reviewer",
        Some("approved after source review"),
        Some("pending"),
        Some(&old_version),
    )
    .expect("approve passage");
    let after_review = narrative::review_detail(&mut client, first_id)
        .expect("detail")
        .expect("passage");
    assert_eq!(after_review.passage.review_status, "approved");
    assert_eq!(after_review.passage.visibility, "family");
    assert_eq!(after_review.events.len(), 2);
    assert!(after_review
        .events
        .iter()
        .any(|event| event.action == "link_review"));
    assert!(after_review
        .events
        .iter()
        .any(|event| event.action == "passage_review"));

    assert!(
        narrative::review_passage_checked(
            &mut client,
            first_id,
            PassageDecision::Rejected,
            Some(PassageVisibility::AdminOnly),
            "synthetic-reviewer",
            None,
            Some("pending"),
            Some(&old_version),
        )
        .is_err(),
        "a stale passage decision must not overwrite the approved passage"
    );

    let approved = narrative::review_queue(
        &mut client,
        &narrative::ReviewFilters {
            status: "approved".to_string(),
            language: None,
            person_key: Some("synthetic-one".to_string()),
            unmatched_only: false,
            limit: 100,
        },
    )
    .expect("approved queue");
    assert_eq!(approved.entries.len(), 1);
    let pending = narrative::review_queue(
        &mut client,
        &narrative::ReviewFilters {
            status: "pending".to_string(),
            language: None,
            person_key: Some("synthetic-one".to_string()),
            unmatched_only: false,
            limit: 100,
        },
    )
    .expect("pending queue");
    assert!(pending.entries.is_empty());
}

#[test]
fn checked_family_approval_still_requires_an_accepted_link() {
    let Some(target) = target() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping narrative checked publish guard test");
        return;
    };
    let mut client = target.connect();
    seed_people(&mut client);
    let source = "synthetic/narrative-unlinked.md";
    let text = "## Trang PDF 21\n\n### No catalog match\n\nUnlinked paragraph.\n";
    let parse = narrative::parse_narrative(text, source);
    let people = narrative::linkable_people(&mut client).expect("people");
    let mut tx = client.transaction().expect("tx");
    narrative::import_narrative(
        &mut tx,
        source,
        &"e".repeat(64),
        "translation",
        "vi",
        &parse,
        &people,
    )
    .expect("import");
    tx.commit().expect("commit");
    let passage_id: i64 = client
        .query_one(
            "SELECT passage_id FROM source_narrative_passage LIMIT 1",
            &[],
        )
        .expect("passage")
        .get(0);
    let detail = narrative::review_detail(&mut client, passage_id)
        .expect("detail")
        .expect("passage");
    assert!(
        narrative::review_passage_checked(
            &mut client,
            passage_id,
            PassageDecision::Approved,
            Some(PassageVisibility::Family),
            "synthetic-reviewer",
            None,
            Some("pending"),
            Some(&detail.passage.updated_at),
        )
        .is_err(),
        "family approval without an accepted link must fail"
    );
    let after = narrative::review_detail(&mut client, passage_id)
        .expect("detail")
        .expect("passage");
    assert_eq!(after.passage.review_status, "pending");
    assert!(after.events.is_empty(), "a rejected guard writes no event");
}
