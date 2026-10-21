//! USR-004B phase 1 service tests for owner drafts and independent review.
//!
//! Synthetic PostgreSQL only. The test uses `GENEALOGY_DATABASE_URL`, applies
//! migrations 0001-0012, and cleans its own rows by prefix.

use std::fs;
use std::str::FromStr;
use std::sync::mpsc;
use std::sync::{Barrier, Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::onboarding::{
    create_draft, list_open_revisions, read_approved_values, review_revision, submit_revision,
    update_draft, ProfileAudience, ProfileVisibility, ReviewDecision, RevisionError,
    RevisionStatus, REVIEW_NOTE_MAX_CHARS,
};

const MIGRATIONS: [&str; 16] = [
    "genealogy/migrations/0001_genealogy_schema_version.sql",
    "genealogy/migrations/0002_catalog_keys.sql",
    "genealogy/migrations/0003_assertions_evidence_and_links.sql",
    "genealogy/migrations/0004_review_workflow.sql",
    "genealogy/migrations/0005_source_kind.sql",
    "genealogy/migrations/0006_head_of_family.sql",
    "genealogy/migrations/0007_onboarding.sql",
    "genealogy/migrations/0008_onboarding_identity.sql",
    "genealogy/migrations/0009_onboarding_link_audit.sql",
    "genealogy/migrations/0010_onboarding_foreign_keys.sql",
    "genealogy/migrations/0011_onboarding_foreign_key_reconciliation.sql",
    "genealogy/migrations/0012_profile_revisions.sql",
    "genealogy/migrations/0013_profile_revision_actor_scope.sql",
    "genealogy/migrations/0014_profile_revision_draft_withdrawal.sql",
    "genealogy/migrations/0015_profile_media.sql",
    "genealogy/migrations/0016_profile_media_withdrawal.sql",
];

const PERSON_CHILD_TABLES: [&str; 8] = [
    "onboarding_profile_review_event",
    "onboarding_profile_revision",
    "onboarding_profile_value",
    "onboarding_media_metadata",
    "onboarding_link_audit",
    "onboarding_person_account_link",
    "onboarding_invitation",
    "onboarding_username_reservation",
];

static DB_LOCK: Mutex<()> = Mutex::new(());

fn dsn() -> Option<String> {
    let dsn = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    if dsn.is_empty() {
        eprintln!("GENEALOGY_DATABASE_URL unset; skipping profile revision service test");
        return None;
    }
    let config = postgres::Config::from_str(&dsn).expect("GENEALOGY_DATABASE_URL parses");
    let database = config.get_dbname().unwrap_or_default();
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable genealogy database {database:?}"
    );
    Some(dsn)
}

struct TestDb {
    genealogy: Client,
    dsn: String,
    prefix: String,
    _guard: MutexGuard<'static, ()>,
}

impl TestDb {
    fn new() -> Option<Self> {
        let dsn = dsn()?;
        let guard = DB_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let mut genealogy = Client::connect(&dsn, NoTls).expect("genealogy database");
        apply_migrations(&mut genealogy, &MIGRATIONS);
        clear_orphan_child_rows(&mut genealogy);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        Some(Self {
            genealogy,
            dsn,
            prefix: format!("synthetic-revision-{}-{nanos}", std::process::id()),
            _guard: guard,
        })
    }

    fn person(&mut self, label: &str) -> i64 {
        let key = format!("{}-person-{label}", self.prefix);
        self.genealogy
            .query_one(
                "INSERT INTO person (person_key, display_name) VALUES ($1, $2) RETURNING person_id",
                &[&key, &format!("Synthetic Revision Person {label}")],
            )
            .expect("insert synthetic person")
            .get(0)
    }

    fn approved(&mut self, person_id: i64, field_key: &str, value: &str) {
        self.approved_with_visibility(person_id, field_key, value, "owner_private");
    }

    fn approved_with_visibility(
        &mut self,
        person_id: i64,
        field_key: &str,
        value: &str,
        visibility: &str,
    ) {
        let revision_id: i64 = self
            .genealogy
            .query_one(
                "INSERT INTO onboarding_profile_revision \
                     (person_id, field_key, prior_value_text, \
                      prior_approved_revision_number, new_value_text, changed_by, \
                      revision_number, status, visibility) \
                 VALUES ($1, $2, NULL, 0, $3, 'synthetic-seed', 1, 'draft', $4) \
                 RETURNING revision_id",
                &[&person_id, &field_key, &value, &visibility],
            )
            .expect("insert seed revision")
            .get(0);
        self.genealogy
            .execute(
                "UPDATE onboarding_profile_revision \
                    SET status = 'pending_review', submitted_at = now() \
                  WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("submit seed revision");
        self.genealogy
            .execute(
                "UPDATE onboarding_profile_revision \
                    SET status = 'approved', reviewed_at = now(), \
                        reviewed_by = 'synthetic-seed-reviewer', review_note = 'seed' \
                  WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("approve seed revision");
        self.genealogy
            .execute(
                "INSERT INTO onboarding_profile_value \
                     (person_id, field_key, value_text, visibility, updated_by, \
                      approved_revision_number, approved_revision_id) \
                 VALUES ($1, $2, $3, $4, 'synthetic-owner', 1, $5)",
                &[&person_id, &field_key, &value, &visibility, &revision_id],
            )
            .expect("seed approved value");
    }

    fn approved_value(&mut self, person_id: i64, field_key: &str) -> Option<String> {
        self.genealogy
            .query_opt(
                "SELECT value_text FROM onboarding_profile_value \
                  WHERE person_id = $1 AND field_key = $2",
                &[&person_id, &field_key],
            )
            .expect("read approved value")
            .map(|row| row.get(0))
    }

    fn event_count(&mut self, revision_id: i64) -> i64 {
        self.genealogy
            .query_one(
                "SELECT count(*) FROM onboarding_profile_review_event WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("count review events")
            .get(0)
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let pattern = format!("{}%", self.prefix);
        for table in PERSON_CHILD_TABLES {
            let sql = format!(
                "DELETE FROM {table} AS child USING person p \
                  WHERE child.person_id = p.person_id AND p.person_key LIKE $1"
            );
            let _ = self.genealogy.execute(&sql, &[&pattern]);
        }
        let _ = self
            .genealogy
            .execute("DELETE FROM person WHERE person_key LIKE $1", &[&pattern]);
    }
}

fn clear_orphan_child_rows(genealogy: &mut Client) {
    for table in PERSON_CHILD_TABLES {
        let sql = format!(
            "DELETE FROM {table} AS child \
              WHERE NOT EXISTS (SELECT 1 FROM person p WHERE p.person_id = child.person_id)"
        );
        let _ = genealogy.execute(&sql, &[]);
    }
}

fn apply_migrations(client: &mut Client, paths: &[&str]) {
    for path in paths {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

#[test]
fn draft_and_submission_keep_the_approved_value_until_independent_approval() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("approval");
    db.approved(person, "display_name", "Approved Name");

    let draft = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "display_name",
        "Proposed Name",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create draft");
    assert_eq!(draft.status, RevisionStatus::Draft);
    assert_eq!(draft.prior_value_text.as_deref(), Some("Approved Name"));
    assert_eq!(
        db.approved_value(person, "display_name").as_deref(),
        Some("Approved Name")
    );

    let pending =
        submit_revision(&mut db.genealogy, draft.revision_id, "owner-a").expect("submit draft");
    assert_eq!(pending.status, RevisionStatus::PendingReview);
    assert_eq!(
        db.approved_value(person, "display_name").as_deref(),
        Some("Approved Name"),
        "pending content must not replace the approved value"
    );

    let self_review = review_revision(
        &mut db.genealogy,
        draft.revision_id,
        "owner-a",
        ReviewDecision::Approve,
        None,
    );
    assert_eq!(self_review, Err(RevisionError::SelfReview));
    assert_eq!(
        db.approved_value(person, "display_name").as_deref(),
        Some("Approved Name")
    );

    let approved = review_revision(
        &mut db.genealogy,
        draft.revision_id,
        "reviewer-a",
        ReviewDecision::Approve,
        Some("looks correct"),
    )
    .expect("independent approval");
    assert_eq!(approved.status, RevisionStatus::Approved);
    assert_eq!(approved.reviewed_by.as_deref(), Some("reviewer-a"));
    assert_eq!(
        db.approved_value(person, "display_name").as_deref(),
        Some("Proposed Name"),
        "approval must update the approved value atomically"
    );
    let approved_row = db
        .genealogy
        .query_one(
            "SELECT updated_by, approved_revision_number, approved_revision_id \
               FROM onboarding_profile_value \
              WHERE person_id = $1 AND field_key = 'display_name'",
            &[&person],
        )
        .expect("read approved metadata");
    assert_eq!(approved_row.get::<_, String>(0), "reviewer-a");
    assert_eq!(approved_row.get::<_, i64>(1), draft.revision_number);
    assert_eq!(
        approved_row.get::<_, Option<i64>>(2),
        Some(draft.revision_id)
    );
    assert_eq!(db.event_count(draft.revision_id), 3);
}

#[test]
fn draft_edits_are_owner_only_and_submitted_content_is_immutable() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("immutable");
    let draft = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "biography",
        "First biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create draft");

    assert_eq!(
        update_draft(
            &mut db.genealogy,
            draft.revision_id,
            "owner-b",
            "Hijacked biography",
            Some(ProfileVisibility::OwnerPrivate)
        ),
        Err(RevisionError::NotOwner)
    );

    let edited = update_draft(
        &mut db.genealogy,
        draft.revision_id,
        "owner-a",
        "Edited biography",
        Some(ProfileVisibility::Family),
    )
    .expect("owner edits draft");
    assert_eq!(edited.new_value_text, "Edited biography");
    assert_eq!(edited.visibility, ProfileVisibility::Family);
    assert_eq!(edited.status, RevisionStatus::Draft);

    submit_revision(&mut db.genealogy, draft.revision_id, "owner-a").expect("submit");
    assert_eq!(
        update_draft(
            &mut db.genealogy,
            draft.revision_id,
            "owner-a",
            "Late edit",
            Some(ProfileVisibility::OwnerPrivate)
        ),
        Err(RevisionError::InvalidTransition)
    );

    let direct = db.genealogy.execute(
        "UPDATE onboarding_profile_revision SET new_value_text = 'direct edit' \
          WHERE revision_id = $1",
        &[&draft.revision_id],
    );
    assert!(
        direct.is_err(),
        "submitted content must be immutable in SQL"
    );
}

#[test]
fn updating_without_visibility_preserves_the_draft_visibility() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("preserve-visibility");
    let draft = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "biography",
        "Family-visible draft",
        ProfileVisibility::Family,
    )
    .expect("create family draft");

    let edited = update_draft(
        &mut db.genealogy,
        draft.revision_id,
        "owner-a",
        "Edited family draft",
        None,
    )
    .expect("edit without visibility");
    assert_eq!(edited.visibility, ProfileVisibility::Family);
}

#[test]
fn rejection_and_needs_changes_leave_the_approved_value_unchanged() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("decisions");
    db.approved(person, "biography", "Approved biography");

    let first = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "biography",
        "Rejected biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("first draft");
    submit_revision(&mut db.genealogy, first.revision_id, "owner-a").expect("submit first");
    let rejected = review_revision(
        &mut db.genealogy,
        first.revision_id,
        "reviewer-a",
        ReviewDecision::Reject,
        Some("not enough detail"),
    )
    .expect("reject");
    assert_eq!(rejected.status, RevisionStatus::Rejected);
    assert_eq!(
        db.approved_value(person, "biography").as_deref(),
        Some("Approved biography")
    );

    let second = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "biography",
        "Needs changes biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("second draft after terminal decision");
    assert_eq!(second.revision_number, 3);
    submit_revision(&mut db.genealogy, second.revision_id, "owner-a").expect("submit second");
    let needs_changes = review_revision(
        &mut db.genealogy,
        second.revision_id,
        "reviewer-a",
        ReviewDecision::NeedsChanges,
        Some("please add dates"),
    )
    .expect("needs changes");
    assert_eq!(needs_changes.status, RevisionStatus::NeedsChanges);
    assert_eq!(
        db.approved_value(person, "biography").as_deref(),
        Some("Approved biography")
    );
}

#[test]
fn version_conflicts_block_submission_and_approval() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("conflict");
    db.approved(person, "display_name", "Base Name");

    let draft = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "display_name",
        "Proposed Name",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create draft");

    // Simulate a privileged external replacement that bypassed the guard. The
    // service must still detect the changed text/version.
    db.genealogy
        .execute("SET session_replication_role = replica", &[])
        .expect("disable guards for synthetic external change");
    db.genealogy
        .execute(
            "UPDATE onboarding_profile_value \
                SET value_text = 'Changed Elsewhere', approved_revision_number = 2 \
              WHERE person_id = $1 AND field_key = 'display_name'",
            &[&person],
        )
        .expect("change approved value externally");
    db.genealogy
        .execute("SET session_replication_role = origin", &[])
        .expect("restore guards");

    assert_eq!(
        submit_revision(&mut db.genealogy, draft.revision_id, "owner-a"),
        Err(RevisionError::VersionConflict)
    );
    assert_eq!(
        db.approved_value(person, "display_name").as_deref(),
        Some("Changed Elsewhere")
    );
}

#[test]
fn one_open_revision_per_field_is_enforced() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("open");
    let first = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "preferred_name",
        "First",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("first draft");

    assert_eq!(
        create_draft(
            &mut db.genealogy,
            person,
            "owner-a",
            "preferred_name",
            "Second",
            ProfileVisibility::OwnerPrivate
        ),
        Err(RevisionError::OpenRevisionExists)
    );

    submit_revision(&mut db.genealogy, first.revision_id, "owner-a").expect("submit first");
    review_revision(
        &mut db.genealogy,
        first.revision_id,
        "reviewer-a",
        ReviewDecision::Reject,
        Some("synthetic rejection"),
    )
    .expect("reject first");
    let second = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "preferred_name",
        "Second",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("new draft after terminal decision");
    assert_eq!(second.revision_number, 2);
}

#[test]
fn invalid_fields_and_values_are_rejected() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("invalid");
    assert_eq!(
        create_draft(
            &mut db.genealogy,
            person,
            "owner-a",
            "email",
            "not-allowed@example.test",
            ProfileVisibility::OwnerPrivate
        ),
        Err(RevisionError::InvalidField)
    );
    assert_eq!(
        create_draft(
            &mut db.genealogy,
            person,
            "owner-a",
            "biography",
            "   ",
            ProfileVisibility::OwnerPrivate
        ),
        Err(RevisionError::InvalidValue)
    );
    assert_eq!(
        create_draft(
            &mut db.genealogy,
            person,
            "",
            "biography",
            "valid",
            ProfileVisibility::OwnerPrivate
        ),
        Err(RevisionError::InvalidValue)
    );
}

#[test]
fn unapproved_drafts_never_appear_in_approved_values() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("separation");
    let draft = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "biography",
        "Owner-private draft",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create draft");

    let approved = read_approved_values(&mut db.genealogy, person, ProfileAudience::Owner)
        .expect("read approved");
    assert!(approved.is_empty(), "a draft is not an approved value");

    submit_revision(&mut db.genealogy, draft.revision_id, "owner-a").expect("submit");
    let approved = read_approved_values(&mut db.genealogy, person, ProfileAudience::Owner)
        .expect("read approved");
    assert!(approved.is_empty(), "pending content is not approved");
    let open = list_open_revisions(&mut db.genealogy, person).expect("list open");
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].status, RevisionStatus::PendingReview);
}

#[test]
fn approved_revision_number_is_part_of_the_conflict_check() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("version-number");
    db.approved(person, "display_name", "Base Name");

    let draft = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "display_name",
        "Proposed Name",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create draft");
    assert_eq!(draft.prior_approved_revision_number, 1);

    // Same text, different approved version: this is still a conflict. A
    // privileged external writer bypassed the ordinary guard.
    db.genealogy
        .execute("SET session_replication_role = replica", &[])
        .expect("disable guards for synthetic external change");
    db.genealogy
        .execute(
            "UPDATE onboarding_profile_value \
                SET approved_revision_number = 2 \
              WHERE person_id = $1 AND field_key = 'display_name'",
            &[&person],
        )
        .expect("advance approved version externally");
    db.genealogy
        .execute("SET session_replication_role = origin", &[])
        .expect("restore guards");
    assert_eq!(
        submit_revision(&mut db.genealogy, draft.revision_id, "owner-a"),
        Err(RevisionError::VersionConflict)
    );
}

#[test]
fn approved_values_are_filtered_by_audience() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("audience");
    db.approved_with_visibility(person, "biography", "owner only", "owner_private");
    db.approved_with_visibility(person, "display_name", "family visible", "family");

    let owner = read_approved_values(&mut db.genealogy, person, ProfileAudience::Owner)
        .expect("owner read");
    assert_eq!(owner.len(), 2);
    let family = read_approved_values(&mut db.genealogy, person, ProfileAudience::Family)
        .expect("family read");
    assert_eq!(family.len(), 1);
    assert_eq!(family[0].field_key, "display_name");
    assert_eq!(family[0].value_text, "family visible");
    assert_eq!(family[0].visibility, ProfileVisibility::Family);
}

#[test]
fn review_notes_are_bounded() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("review-note");
    let draft = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "biography",
        "Note test biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create draft");
    submit_revision(&mut db.genealogy, draft.revision_id, "owner-a").expect("submit");

    let too_long = "x".repeat(REVIEW_NOTE_MAX_CHARS + 1);
    assert_eq!(
        review_revision(
            &mut db.genealogy,
            draft.revision_id,
            "reviewer-a",
            ReviewDecision::Approve,
            Some(&too_long)
        ),
        Err(RevisionError::InvalidValue)
    );
    let revision = rag_foundation::onboarding::revision_by_id(&mut db.genealogy, draft.revision_id)
        .expect("read revision");
    assert_eq!(revision.status, RevisionStatus::PendingReview);
}

#[test]
fn reject_and_needs_changes_require_a_review_note() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("note-required");
    let draft = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "biography",
        "Rejected without a reason",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create draft");
    submit_revision(&mut db.genealogy, draft.revision_id, "owner-a").expect("submit");

    assert_eq!(
        review_revision(
            &mut db.genealogy,
            draft.revision_id,
            "reviewer-a",
            ReviewDecision::Reject,
            None
        ),
        Err(RevisionError::ReviewNoteRequired)
    );
    let revision = rag_foundation::onboarding::revision_by_id(&mut db.genealogy, draft.revision_id)
        .expect("read revision");
    assert_eq!(revision.status, RevisionStatus::PendingReview);
}

#[test]
fn concurrent_drafts_for_one_field_allocate_one_version() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("concurrent-create");
    let dsn = db.dsn.clone();

    let barrier = Barrier::new(2);
    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..2 {
            let dsn = dsn.clone();
            let barrier = &barrier;
            let tx = tx.clone();
            scope.spawn(move || {
                let mut client = Client::connect(&dsn, NoTls).expect("draft connection");
                barrier.wait();
                let result = create_draft(
                    &mut client,
                    person,
                    "owner-a",
                    "biography",
                    "Concurrent draft",
                    ProfileVisibility::OwnerPrivate,
                );
                tx.send(result).expect("send draft result");
            });
        }
    });

    let results: Vec<_> = {
        drop(tx);
        rx.iter().collect()
    };
    let successes = results
        .iter()
        .filter(|result| matches!(result, Ok(revision) if revision.revision_number == 1))
        .count();
    assert_eq!(successes, 1, "exactly one concurrent draft must win");
    assert!(
        results
            .iter()
            .any(|result| matches!(result, Err(RevisionError::OpenRevisionExists))),
        "the losing draft must observe the open-revision guard"
    );
}

#[test]
fn concurrent_create_and_submit_leave_at_most_one_open_revision() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("concurrent-create-submit");
    let draft = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "biography",
        "Existing draft",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create draft");
    let dsn = db.dsn.clone();

    let barrier = Barrier::new(2);
    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        for action in 0..2 {
            let dsn = dsn.clone();
            let barrier = &barrier;
            let tx = tx.clone();
            scope.spawn(move || {
                let mut client = Client::connect(&dsn, NoTls).expect("connection");
                barrier.wait();
                if action == 0 {
                    let result = create_draft(
                        &mut client,
                        person,
                        "owner-a",
                        "biography",
                        "Competing draft",
                        ProfileVisibility::OwnerPrivate,
                    );
                    tx.send(result.map(|revision| revision.status))
                        .expect("send create result");
                } else {
                    let result = submit_revision(&mut client, draft.revision_id, "owner-a");
                    tx.send(result.map(|revision| revision.status))
                        .expect("send submit result");
                }
            });
        }
    });

    let results: Vec<_> = {
        drop(tx);
        rx.iter().collect()
    };
    let successes = results.iter().filter(|result| result.is_ok()).count();
    assert_eq!(successes, 1, "only one operation may hold the field open");
    let open: i64 = db
        .genealogy
        .query_one(
            "SELECT count(*) FROM onboarding_profile_revision \
              WHERE person_id = $1 AND status IN ('draft', 'pending_review')",
            &[&person],
        )
        .expect("count open revisions")
        .get(0);
    assert_eq!(open, 1);
}

#[test]
fn concurrent_submissions_have_exactly_one_winner() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let person = db.person("concurrent-submit");
    let draft = create_draft(
        &mut db.genealogy,
        person,
        "owner-a",
        "biography",
        "Concurrent biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create draft");
    let dsn = db.dsn.clone();

    let barrier = Barrier::new(2);
    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..2 {
            let dsn = dsn.clone();
            let barrier = &barrier;
            let tx = tx.clone();
            scope.spawn(move || {
                let mut client = Client::connect(&dsn, NoTls).expect("submission connection");
                barrier.wait();
                let result = submit_revision(&mut client, draft.revision_id, "owner-a");
                tx.send(result).expect("send submission result");
            });
        }
    });

    let results: Vec<_> = {
        drop(tx);
        rx.iter().collect()
    };
    let successes = results
        .iter()
        .filter(|result| {
            matches!(
                result,
                Ok(revision) if revision.status == RevisionStatus::PendingReview
            )
        })
        .count();
    assert_eq!(successes, 1, "exactly one concurrent submission must win");
    assert!(
        results
            .iter()
            .any(|result| matches!(result, Err(RevisionError::InvalidTransition))),
        "the losing submission must observe the pending state"
    );
}
