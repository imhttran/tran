//! USR-004B phase 2: owner submission API service tests.
//!
//! Synthetic identity and genealogy databases only. These tests exercise the
//! owner-scoped service primitives the API calls: link resolution, ownership
//! rechecks inside write transactions, self-review denial, approved/pending
//! separation, visibility filtering, and RAG isolation.

use std::fs;
use std::str::FromStr;
use std::sync::{mpsc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::identity::{decide, Decision, Operation, Role, VerifiedIdentity};
use rag_foundation::onboarding::{
    cancel_owned_draft, create_owned_draft, identity_fingerprint, link_for_account, link_reviewed,
    read_approved_values, read_owner_profile, resolve_account, review_revision, revision_by_id,
    revoke_link, submit_owned_revision, update_owned_draft, IdentityAccount, LinkDecision,
    ProfileAudience, ProfileVisibility, ReviewDecision, RevisionError, RevisionStatus,
};

const IDENTITY_MIGRATIONS: [&str; 3] = [
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
];

const GENEALOGY_MIGRATIONS: [&str; 14] = [
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
];

const ISSUER: &str = "https://synthetic-issuer.example";

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

fn dsns() -> Option<(String, String)> {
    let identity = std::env::var("DATABASE_URL").ok()?;
    let genealogy = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let identity = identity.trim().to_string();
    let genealogy = genealogy.trim().to_string();
    if identity.is_empty() || genealogy.is_empty() {
        eprintln!("DATABASE_URL or GENEALOGY_DATABASE_URL unset; skipping submission test");
        return None;
    }
    let identity_config = postgres::Config::from_str(&identity).expect("DATABASE_URL parses");
    let genealogy_config =
        postgres::Config::from_str(&genealogy).expect("GENEALOGY_DATABASE_URL parses");
    let database = genealogy_config.get_dbname().unwrap_or_default();
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable genealogy database {database:?}"
    );
    assert_ne!(
        (
            genealogy_config.get_dbname(),
            genealogy_config.get_hosts(),
            genealogy_config.get_ports()
        ),
        (
            identity_config.get_dbname(),
            identity_config.get_hosts(),
            identity_config.get_ports()
        ),
        "the submission test must not target the identity database"
    );
    Some((identity, genealogy))
}

struct TestDb {
    identity: Client,
    genealogy: Client,
    genealogy_dsn: String,
    rag_dsn: Option<String>,
    prefix: String,
    _guard: MutexGuard<'static, ()>,
}

impl TestDb {
    fn new() -> Option<Self> {
        let (identity_dsn, genealogy_dsn) = dsns()?;
        let guard = DB_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let mut identity = Client::connect(&identity_dsn, NoTls).expect("identity database");
        let mut genealogy = Client::connect(&genealogy_dsn, NoTls).expect("genealogy database");
        apply_migrations(&mut identity, &IDENTITY_MIGRATIONS);
        apply_migrations(&mut genealogy, &GENEALOGY_MIGRATIONS);
        clear_orphan_child_rows(&mut genealogy);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        Some(Self {
            identity,
            genealogy,
            genealogy_dsn,
            rag_dsn: std::env::var("DATABASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            prefix: format!("synthetic-submission-{}-{nanos}", std::process::id()),
            _guard: guard,
        })
    }

    fn account(&mut self, label: &str) -> IdentityAccount {
        let subject = format!("{}-user-{label}", self.prefix);
        self.identity
            .execute(
                "INSERT INTO users (subject, issuer, display_name) VALUES ($1, $2, $3)",
                &[&subject, &ISSUER, &format!("Synthetic {label}")],
            )
            .expect("insert synthetic identity");
        resolve_account(&mut self.identity, ISSUER, &subject).expect("resolve synthetic identity")
    }

    fn person(&mut self, label: &str) -> i64 {
        let key = format!("{}-person-{label}", self.prefix);
        self.genealogy
            .query_one(
                "INSERT INTO person (person_key, display_name) VALUES ($1, $2) RETURNING person_id",
                &[&key, &format!("Synthetic Submission Person {label}")],
            )
            .expect("insert synthetic person")
            .get(0)
    }

    fn link(&mut self, account: &IdentityAccount, person_id: i64) {
        link_reviewed(
            &mut self.genealogy,
            person_id,
            account,
            LinkDecision::Approve,
            "synthetic-reviewer",
        )
        .expect("link synthetic account");
    }

    fn approved(
        &mut self,
        person_id: i64,
        field_key: &str,
        value: &str,
        visibility: ProfileVisibility,
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
                &[&person_id, &field_key, &value, &visibility.as_str()],
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
                &[
                    &person_id,
                    &field_key,
                    &value,
                    &visibility.as_str(),
                    &revision_id,
                ],
            )
            .expect("seed approved value");
    }

    fn read(
        &mut self,
        account: &IdentityAccount,
    ) -> Option<rag_foundation::onboarding::OwnerProfile> {
        read_owner_profile(&mut self.genealogy, account).expect("read owner profile")
    }

    fn extra_genealogy(&self) -> Client {
        Client::connect(&self.genealogy_dsn, NoTls).expect("second genealogy connection")
    }

    fn rag_document_count(&self) -> Option<i64> {
        let dsn = self.rag_dsn.as_ref()?;
        let mut client = Client::connect(dsn, NoTls).ok()?;
        client
            .query_one("SELECT count(*) FROM documents", &[])
            .ok()
            .map(|row| row.get(0))
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
        let _ = self
            .identity
            .execute("DELETE FROM users WHERE subject LIKE $1", &[&pattern]);
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
fn linked_owner_drafts_and_submissions_keep_approved_values_separate() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner");
    let person = db.person("owner");
    db.link(&owner, person);
    db.approved(
        person,
        "display_name",
        "Approved Name",
        ProfileVisibility::OwnerPrivate,
    );

    let before = db.read(&owner).expect("linked owner");
    assert_eq!(before.approved.len(), 1);
    assert_eq!(before.approved[0].value_text, "Approved Name");
    assert!(before.pending.is_empty());

    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "display_name",
        "Draft Name",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create owner draft");
    assert_eq!(draft.status, RevisionStatus::Draft);

    let after_draft = db.read(&owner).expect("linked owner");
    assert_eq!(after_draft.approved[0].value_text, "Approved Name");
    assert_eq!(after_draft.pending.len(), 1);
    assert_eq!(after_draft.pending[0].new_value_text, "Draft Name");

    let edited = update_owned_draft(
        &mut db.genealogy,
        &owner,
        draft.revision_id,
        "Edited Name",
        Some(ProfileVisibility::OwnerPrivate),
    )
    .expect("edit owner draft");
    assert_eq!(edited.new_value_text, "Edited Name");

    let submitted = submit_owned_revision(&mut db.genealogy, &owner, draft.revision_id)
        .expect("submit owner draft");
    assert_eq!(submitted.status, RevisionStatus::PendingReview);
    let pending = db.read(&owner).expect("linked owner");
    assert_eq!(pending.approved[0].value_text, "Approved Name");
    assert_eq!(pending.pending[0].status, RevisionStatus::PendingReview);

    let approved = review_revision(
        &mut db.genealogy,
        draft.revision_id,
        "reviewer-a",
        ReviewDecision::Approve,
        Some("independent approval"),
    )
    .expect("independent approval");
    assert_eq!(approved.status, RevisionStatus::Approved);

    let final_profile = db.read(&owner).expect("linked owner");
    assert_eq!(final_profile.approved[0].value_text, "Edited Name");
    assert!(final_profile.pending.is_empty());
}

#[test]
fn unlinked_and_cross_account_accounts_cannot_write() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let unlinked = db.account("unlinked");
    let person = db.person("cross-account-a");
    assert_eq!(
        create_owned_draft(
            &mut db.genealogy,
            &unlinked,
            "biography",
            "No link",
            ProfileVisibility::OwnerPrivate
        ),
        Err(RevisionError::Unlinked)
    );

    let owner_a = db.account("cross-account-a");
    let owner_b = db.account("cross-account-b");
    db.link(&owner_a, person);
    let person_b = db.person("cross-account-b");
    db.link(&owner_b, person_b);
    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner_a,
        "biography",
        "Owner A biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("owner A draft");

    assert_eq!(
        update_owned_draft(
            &mut db.genealogy,
            &owner_b,
            draft.revision_id,
            "Cross-account edit",
            Some(ProfileVisibility::OwnerPrivate)
        ),
        Err(RevisionError::NotOwner)
    );
    assert_eq!(
        submit_owned_revision(&mut db.genealogy, &owner_b, draft.revision_id),
        Err(RevisionError::NotOwner)
    );
}

#[test]
fn owner_cannot_approve_own_revision() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("self-review");
    let person = db.person("self-review");
    db.link(&owner, person);
    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "biography",
        "Self review biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("owner draft");
    submit_owned_revision(&mut db.genealogy, &owner, draft.revision_id).expect("submit");

    assert_eq!(
        review_revision(
            &mut db.genealogy,
            draft.revision_id,
            &owner.identity_fingerprint,
            ReviewDecision::Approve,
            None
        ),
        Err(RevisionError::SelfReview)
    );
}

#[test]
fn reviewer_linked_to_the_person_cannot_approve_a_prior_owner_submission() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("prior-owner");
    let person = db.person("prior-owner");
    db.link(&owner, person);
    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "biography",
        "Prior owner biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("owner draft");
    submit_owned_revision(&mut db.genealogy, &owner, draft.revision_id).expect("submit");

    let owner_link = link_for_account(
        &mut db.genealogy,
        owner.account_id,
        &owner.identity_fingerprint,
    )
    .expect("find owner link")
    .expect("owner link exists");
    revoke_link(&mut db.genealogy, owner_link.link_id, "synthetic-reviewer")
        .expect("revoke owner link");

    let replacement_owner = db.account("replacement-owner");
    db.link(&replacement_owner, person);
    assert_eq!(
        review_revision(
            &mut db.genealogy,
            draft.revision_id,
            &replacement_owner.identity_fingerprint,
            ReviewDecision::Approve,
            Some("replacement owner approval")
        ),
        Err(RevisionError::SelfReview),
        "a reviewer linked to the same person is the owner for self-review purposes"
    );
}

#[test]
fn replacement_owner_does_not_see_or_inherit_former_drafts() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner_a = db.account("replacement-a");
    let person = db.person("replacement");
    db.link(&owner_a, person);
    let former = create_owned_draft(
        &mut db.genealogy,
        &owner_a,
        "biography",
        "Former owner draft",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("former owner draft");

    let owner_link = link_for_account(
        &mut db.genealogy,
        owner_a.account_id,
        &owner_a.identity_fingerprint,
    )
    .expect("find owner link")
    .expect("owner link exists");
    revoke_link(&mut db.genealogy, owner_link.link_id, "synthetic-reviewer")
        .expect("revoke former owner link");

    let owner_b = db.account("replacement-b");
    db.link(&owner_b, person);
    let profile_b = db.read(&owner_b).expect("replacement owner profile");
    assert!(
        profile_b.pending.is_empty(),
        "the replacement owner must not see the former owner's draft"
    );
    assert!(
        profile_b.history.is_empty(),
        "the replacement owner must not inherit the former owner's history"
    );

    let replacement = create_owned_draft(
        &mut db.genealogy,
        &owner_b,
        "biography",
        "Replacement owner draft",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("replacement owner can draft the same field");
    assert_eq!(replacement.changed_by, owner_b.identity_fingerprint);
    assert_ne!(replacement.revision_id, former.revision_id);

    assert!(
        db.read(&owner_a).is_none(),
        "the former owner is unlinked and receives no profile"
    );
}

#[test]
fn updating_without_visibility_preserves_the_draft_visibility() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("visibility");
    let person = db.person("visibility");
    db.link(&owner, person);
    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "biography",
        "Family draft",
        ProfileVisibility::Family,
    )
    .expect("family draft");

    let edited = update_owned_draft(
        &mut db.genealogy,
        &owner,
        draft.revision_id,
        "Edited family draft",
        None,
    )
    .expect("edit without visibility");
    assert_eq!(edited.visibility, ProfileVisibility::Family);
}

#[test]
fn concurrent_link_creation_is_serialized_with_review() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("review-link-owner");
    let person = db.person("review-link");
    db.link(&owner, person);
    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "biography",
        "Review link biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("owner draft");
    submit_owned_revision(&mut db.genealogy, &owner, draft.revision_id).expect("submit");

    let owner_link = link_for_account(
        &mut db.genealogy,
        owner.account_id,
        &owner.identity_fingerprint,
    )
    .expect("find owner link")
    .expect("owner link exists");
    revoke_link(&mut db.genealogy, owner_link.link_id, "synthetic-reviewer")
        .expect("revoke owner link");

    let reviewer = db.account("concurrent-reviewer");
    let (ready_tx, ready_rx) = mpsc::channel();
    let dsn = db.genealogy_dsn.clone();
    let reviewer_account_id = reviewer.account_id;
    let reviewer_fingerprint = reviewer.identity_fingerprint.clone();
    let linker = thread::spawn(move || {
        let mut client = Client::connect(&dsn, NoTls).expect("linker connection");
        let mut tx = client.transaction().expect("begin link transaction");
        tx.query_one(
            "SELECT 1 FROM person WHERE person_id = $1 FOR UPDATE",
            &[&person],
        )
        .expect("lock person");
        tx.execute(
            "INSERT INTO onboarding_person_account_link \
                 (person_id, account_id, identity_fingerprint, linked_by) \
             VALUES ($1, $2, $3, 'synthetic-reviewer')",
            &[&person, &reviewer_account_id, &reviewer_fingerprint],
        )
        .expect("insert reviewer link");
        ready_tx.send(()).expect("signal link");
        thread::sleep(Duration::from_millis(200));
        tx.commit().expect("commit reviewer link");
    });
    ready_rx.recv().expect("linker started");

    let result = review_revision(
        &mut db.genealogy,
        draft.revision_id,
        &reviewer.identity_fingerprint,
        ReviewDecision::Approve,
        Some("concurrent reviewer approval"),
    );
    linker.join().expect("join linker");
    assert_eq!(
        result,
        Err(RevisionError::SelfReview),
        "review must observe the linked reviewer after serialization"
    );
}

#[test]
fn stale_pending_revision_can_still_be_rejected() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("stale-pending");
    let person = db.person("stale-pending");
    db.link(&owner, person);
    db.approved(
        person,
        "biography",
        "Base biography",
        ProfileVisibility::OwnerPrivate,
    );
    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "biography",
        "Stale pending biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("draft");
    submit_owned_revision(&mut db.genealogy, &owner, draft.revision_id).expect("submit");

    db.genealogy
        .execute("SET session_replication_role = replica", &[])
        .expect("disable guards for synthetic external change");
    db.genealogy
        .execute(
            "UPDATE onboarding_profile_value \
                SET value_text = 'Newer approved value', approved_revision_number = 2 \
              WHERE person_id = $1 AND field_key = 'biography'",
            &[&person],
        )
        .expect("advance approved value externally");
    db.genealogy
        .execute("SET session_replication_role = origin", &[])
        .expect("restore guards");

    let rejected = review_revision(
        &mut db.genealogy,
        draft.revision_id,
        "reviewer-a",
        ReviewDecision::Reject,
        Some("superseded by newer value"),
    )
    .expect("a stale pending revision can still be rejected");
    assert_eq!(rejected.status, RevisionStatus::Rejected);
}

#[test]
fn owner_can_cancel_a_stale_draft_and_open_a_new_one() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("cancel");
    let person = db.person("cancel");
    db.link(&owner, person);
    db.approved(
        person,
        "biography",
        "Base biography",
        ProfileVisibility::OwnerPrivate,
    );
    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "biography",
        "Stale draft",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("draft");

    db.genealogy
        .execute("SET session_replication_role = replica", &[])
        .expect("disable guards for synthetic external change");
    db.genealogy
        .execute(
            "UPDATE onboarding_profile_value \
                SET value_text = 'Newer approved value', approved_revision_number = 2 \
              WHERE person_id = $1 AND field_key = 'biography'",
            &[&person],
        )
        .expect("advance approved value externally");
    db.genealogy
        .execute("SET session_replication_role = origin", &[])
        .expect("restore guards");

    let cancelled = cancel_owned_draft(&mut db.genealogy, &owner, draft.revision_id)
        .expect("cancel stale draft");
    assert_eq!(cancelled.revision_id, draft.revision_id);
    assert_eq!(
        revision_by_id(&mut db.genealogy, draft.revision_id),
        Err(RevisionError::UnknownRevision),
        "withdrawing a draft removes the unsubmitted row"
    );
    let replacement = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "biography",
        "Replacement draft after cancel",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("new draft after cancel");
    assert_ne!(replacement.revision_id, draft.revision_id);
}

#[test]
fn cancel_requires_a_linked_owner() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("cancel-auth-owner");
    let person = db.person("cancel-auth");
    db.link(&owner, person);
    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "biography",
        "Cancel authorization draft",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("draft");

    let unlinked = db.account("cancel-auth-unlinked");
    assert_eq!(
        cancel_owned_draft(&mut db.genealogy, &unlinked, draft.revision_id),
        Err(RevisionError::Unlinked)
    );

    let other = db.account("cancel-auth-other");
    let other_person = db.person("cancel-auth-other");
    db.link(&other, other_person);
    assert_eq!(
        cancel_owned_draft(&mut db.genealogy, &other, draft.revision_id),
        Err(RevisionError::NotOwner)
    );
    assert_eq!(
        revision_by_id(&mut db.genealogy, draft.revision_id)
            .expect("draft still exists")
            .status,
        RevisionStatus::Draft
    );
}

#[test]
fn revoked_link_blocks_further_owner_writes() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("revoked-owner");
    let person = db.person("revoked-owner");
    db.link(&owner, person);
    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "biography",
        "Revoked owner biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("owner draft");

    let link = link_for_account(
        &mut db.genealogy,
        owner.account_id,
        &owner.identity_fingerprint,
    )
    .expect("find link")
    .expect("link exists");
    revoke_link(&mut db.genealogy, link.link_id, "synthetic-reviewer").expect("revoke link");

    assert_eq!(
        update_owned_draft(
            &mut db.genealogy,
            &owner,
            draft.revision_id,
            "After revoke",
            Some(ProfileVisibility::OwnerPrivate)
        ),
        Err(RevisionError::Unlinked)
    );
    assert_eq!(
        submit_owned_revision(&mut db.genealogy, &owner, draft.revision_id),
        Err(RevisionError::Unlinked)
    );
}

#[test]
fn concurrent_revoke_is_rechecked_before_a_write_commits() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("concurrent-owner");
    let person = db.person("concurrent-owner");
    db.link(&owner, person);
    let link = link_for_account(
        &mut db.genealogy,
        owner.account_id,
        &owner.identity_fingerprint,
    )
    .expect("find link")
    .expect("link exists");

    let (ready_tx, ready_rx) = mpsc::channel();
    let dsn = db.genealogy_dsn.clone();
    let revoker = thread::spawn(move || {
        let mut client = Client::connect(&dsn, NoTls).expect("revoker connection");
        let mut tx = client.transaction().expect("begin revoke");
        tx.execute(
            "DELETE FROM onboarding_person_account_link WHERE link_id = $1",
            &[&link.link_id],
        )
        .expect("delete link");
        ready_tx.send(()).expect("signal revoke");
        thread::sleep(Duration::from_millis(200));
        tx.commit().expect("commit revoke");
    });
    ready_rx.recv().expect("revoker started");

    let mut writer = db.extra_genealogy();
    let result = create_owned_draft(
        &mut writer,
        &owner,
        "biography",
        "Concurrent write",
        ProfileVisibility::OwnerPrivate,
    );
    revoker.join().expect("join revoker");
    assert_eq!(
        result,
        Err(RevisionError::Unlinked),
        "the write must recheck ownership after the revoke commits"
    );
}

#[test]
fn pending_content_never_reaches_family_or_rag() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("privacy");
    let person = db.person("privacy");
    db.link(&owner, person);
    db.approved(
        person,
        "display_name",
        "Family Name",
        ProfileVisibility::Family,
    );
    db.approved(
        person,
        "biography",
        "Owner Private Biography",
        ProfileVisibility::OwnerPrivate,
    );
    let rag_before = db.rag_document_count();

    let draft = create_owned_draft(
        &mut db.genealogy,
        &owner,
        "biography",
        "Pending Biography",
        ProfileVisibility::OwnerPrivate,
    )
    .expect("create pending draft");
    submit_owned_revision(&mut db.genealogy, &owner, draft.revision_id)
        .expect("submit pending draft");
    let pending_profile = db.read(&owner).expect("owner profile while pending");
    assert_eq!(pending_profile.approved.len(), 2);
    assert_eq!(pending_profile.pending.len(), 1);
    assert_eq!(
        pending_profile.pending[0].status,
        RevisionStatus::PendingReview
    );
    assert_eq!(
        pending_profile.pending[0].new_value_text,
        "Pending Biography"
    );

    review_revision(
        &mut db.genealogy,
        draft.revision_id,
        "reviewer-a",
        ReviewDecision::NeedsChanges,
        Some("needs a source"),
    )
    .expect("request changes");

    let owner_profile = db.read(&owner).expect("owner profile");
    assert_eq!(owner_profile.approved.len(), 2);
    assert!(
        owner_profile.pending.is_empty(),
        "a terminal needs_changes revision is not an open pending revision"
    );
    assert_eq!(owner_profile.history.len(), 1);
    assert_eq!(
        owner_profile.history[0].status,
        RevisionStatus::NeedsChanges
    );
    assert_eq!(
        owner_profile.history[0].review_note.as_deref(),
        Some("needs a source"),
        "the submitting owner must receive the terminal review decision"
    );

    let family = read_approved_values(&mut db.genealogy, person, ProfileAudience::Family)
        .expect("family approved values");
    assert_eq!(family.len(), 1);
    assert_eq!(family[0].value_text, "Family Name");
    assert!(family
        .iter()
        .all(|value| value.value_text != "Owner Private Biography"));
    assert!(family
        .iter()
        .all(|value| value.value_text != "Pending Biography"));

    if let Some(before) = rag_before {
        assert_eq!(
            db.rag_document_count(),
            Some(before),
            "profile submissions must not create retrieval documents"
        );
    }
}

#[test]
fn profile_actors_are_issuer_scoped() {
    let first = identity_fingerprint("https://issuer-a", "same-subject");
    let second = identity_fingerprint("https://issuer-b", "same-subject");
    assert_ne!(
        first, second,
        "the same subject under another issuer must not share a profile actor"
    );
}

#[test]
fn profile_write_and_review_operations_are_scoped() {
    for operation in [Operation::ProfileWrite, Operation::ProfileReview] {
        assert_eq!(decide(None, operation), Decision::Unauthenticated);
    }
    let viewer = VerifiedIdentity::new("subject", ISSUER, vec![Role::Viewer], vec![]);
    let admin = VerifiedIdentity::new("subject", ISSUER, vec![Role::Admin], vec![]);
    assert_eq!(
        decide(Some(&viewer), Operation::ProfileWrite),
        Decision::Allow
    );
    assert_eq!(
        decide(Some(&viewer), Operation::ProfileReview),
        Decision::Forbidden
    );
    assert_eq!(
        decide(Some(&admin), Operation::ProfileReview),
        Decision::Allow
    );
}

#[test]
fn submission_routes_are_registered() {
    let source = include_str!("../src/api.rs");
    for route in [
        "/api/profile/drafts",
        "/api/profile/drafts/{id}",
        "/api/profile/drafts/{id}/submit",
        "/api/profile/drafts/{id}/cancel",
        "/api/profile/revisions/{id}/review",
    ] {
        assert!(source.contains(route), "missing route {route}");
    }
    assert!(source.contains("Operation::ProfileWrite"));
    assert!(source.contains("Operation::ProfileReview"));
    assert!(source.contains("create_owned_draft"));
    assert!(source.contains("update_owned_draft"));
    assert!(source.contains("submit_owned_revision"));
    assert!(source.contains("cancel_owned_draft"));
}
