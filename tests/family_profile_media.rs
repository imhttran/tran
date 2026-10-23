//! USR-006-C: approved-only family media presentation.
//!
//! Synthetic PostgreSQL and generated files only. Uses `DATABASE_URL`,
//! `GENEALOGY_DATABASE_URL`, and a temporary `MEDIA_ROOT`, applies every
//! genealogy migration, and cleans only the rows carrying its own prefix.
//!
//! Every test asserts the same invariant from two angles: what the family
//! profile *lists*, and what the family media route *serves*. A row must fail
//! both or succeed in both.

use std::fs;
use std::io::Cursor;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use image::{DynamicImage, ImageFormat};
use postgres::{Client, NoTls};
use rag_foundation::onboarding::{
    link_reviewed, read_authorized_media, read_family_media, read_family_profile, resolve_account,
    restrict_own_field, review_revision, store_submission, withdraw_owned_media, FamilyProfile,
    FamilyProfileError, IdentityAccount, LinkDecision, MediaAsset, MediaDownload, MediaError,
    MediaKind, MediaStorage, ProfileVisibility, ReviewDecision, RevisionError,
};
use tempfile::TempDir;

const IDENTITY_MIGRATIONS: [&str; 3] = [
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
];

const GENEALOGY_MIGRATIONS: [&str; 22] = [
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
    "genealogy/migrations/0017_profile_media_withdrawal_audit.sql",
    "genealogy/migrations/0018_onboarding_batch.sql",
    "genealogy/migrations/0019_branch_reviewer_grants.sql",
    "genealogy/migrations/0020_branch_review_proposals.sql",
    "genealogy/migrations/0021_profile_field_restriction.sql",
    "genealogy/migrations/0022_profile_revision_restriction_generation.sql",
];

const ISSUER: &str = "https://synthetic-issuer.example";

const PERSON_CHILD_TABLES: [&str; 10] = [
    "onboarding_profile_review_event",
    "onboarding_profile_revision",
    "onboarding_profile_value",
    "onboarding_profile_field_restriction_event",
    "onboarding_profile_field_restriction",
    "onboarding_media_withdrawal",
    "onboarding_media_metadata",
    "onboarding_link_audit",
    "onboarding_person_account_link",
    "onboarding_username_reservation",
];

static DB_LOCK: Mutex<()> = Mutex::new(());

fn dsns() -> Option<(String, String)> {
    let identity = std::env::var("DATABASE_URL").ok()?;
    let genealogy = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let identity = identity.trim().to_string();
    let genealogy = genealogy.trim().to_string();
    if identity.is_empty() || genealogy.is_empty() {
        eprintln!("DATABASE_URL or GENEALOGY_DATABASE_URL unset; skipping family media test");
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
        "the family media test must not target the identity database"
    );
    Some((identity, genealogy))
}

struct TestDb {
    identity: Client,
    genealogy: Client,
    storage: MediaStorage,
    prefix: String,
    _temp: TempDir,
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
        let temp = tempfile::tempdir().expect("temporary media root");
        let storage = MediaStorage::new(temp.path());
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        Some(Self {
            identity,
            genealogy,
            storage,
            prefix: format!("synthetic-family-media-{}-{nanos}", std::process::id()),
            _temp: temp,
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

    fn person(&mut self, label: &str) -> (i64, String) {
        let key = format!("{}-person-{label}", self.prefix);
        let id = self
            .genealogy
            .query_one(
                "INSERT INTO person (person_key, display_name) VALUES ($1, $2) RETURNING person_id",
                &[&key, &format!("Synthetic Family Media Person {label}")],
            )
            .expect("insert synthetic person")
            .get(0);
        (id, key)
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

    fn upload(
        &mut self,
        account: &IdentityAccount,
        kind: MediaKind,
        file_name: &str,
        bytes: &[u8],
        visibility: ProfileVisibility,
    ) -> MediaAsset {
        store_submission(
            &mut self.genealogy,
            &self.storage,
            account,
            kind,
            Some(file_name),
            bytes,
            visibility,
        )
        .expect("store synthetic media")
    }

    fn approve(&mut self, revision_id: i64) {
        review_revision(
            &mut self.genealogy,
            revision_id,
            "reviewer-a",
            ReviewDecision::Approve,
            Some("synthetic family media approval"),
        )
        .expect("approve media revision");
    }

    fn reject(&mut self, revision_id: i64) {
        review_revision(
            &mut self.genealogy,
            revision_id,
            "reviewer-a",
            ReviewDecision::Reject,
            Some("synthetic family media rejection"),
        )
        .expect("reject media revision");
    }

    fn family_profile(
        &mut self,
        requester: &IdentityAccount,
        person_key: &str,
    ) -> Result<FamilyProfile, FamilyProfileError> {
        read_family_profile(&mut self.genealogy, requester, person_key)
    }

    fn family_media(
        &mut self,
        requester: &IdentityAccount,
        person_key: &str,
        media_id: i64,
    ) -> Result<MediaDownload, MediaError> {
        read_family_media(
            &mut self.genealogy,
            &self.storage,
            requester,
            person_key,
            media_id,
        )
    }

    fn any_media(
        &mut self,
        requester: &IdentityAccount,
        media_id: i64,
    ) -> Result<MediaDownload, MediaError> {
        read_authorized_media(&mut self.genealogy, &self.storage, requester, media_id)
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

/// Removes child rows whose person row no longer exists.
///
/// This only ever deletes rows that already have no owning person, which happens
/// when another integration test resets the shared `person` table (see
/// `tests/genealogy_schema.rs`) while these child tables survive. It never touches
/// a row that belongs to a live person, and these databases are disposable.
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

fn png_bytes() -> Vec<u8> {
    let image = DynamicImage::new_rgb8(24, 24);
    let mut out = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .expect("encode synthetic png");
    out
}

fn document_bytes() -> Vec<u8> {
    b"Synthetic family document.\n\nShared for the family archive.\n".to_vec()
}

/// Owner, subject person, and a linked viewer who is not the owner.
fn scene(db: &mut TestDb, label: &str) -> (IdentityAccount, i64, String, IdentityAccount) {
    let owner = db.account(&format!("owner-{label}"));
    let (person, person_key) = db.person(label);
    db.link(&owner, person);
    let viewer = db.account(&format!("viewer-{label}"));
    let (viewer_person, _) = db.person(&format!("viewer-{label}"));
    db.link(&viewer, viewer_person);
    (owner, person, person_key, viewer)
}

#[test]
fn approved_family_photo_is_presented_and_readable() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, _person, person_key, viewer) = scene(&mut db, "photo");
    let photo = db.upload(
        &owner,
        MediaKind::Photo,
        "portrait.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );
    db.approve(photo.revision_id);

    let profile = db
        .family_profile(&viewer, &person_key)
        .expect("family profile");
    assert_eq!(profile.media.len(), 1, "one approved family photo");
    let item = &profile.media[0];
    assert_eq!(item.media_id, photo.media_id);
    assert_eq!(item.kind, "profile_photo");
    assert_eq!(item.mime_type, "image/png");
    assert_eq!(item.approved_revision_number, 1);
    assert!(item.width.is_some() && item.height.is_some());

    let family = db
        .family_media(&viewer, &person_key, photo.media_id)
        .expect("family media");
    let owner_read = db.any_media(&owner, photo.media_id).expect("owner media");
    assert_eq!(
        family.bytes, owner_read.bytes,
        "the family gets the approved bytes"
    );
    assert_eq!(family.asset.sha256, photo.sha256);
    assert_eq!(family.asset.person_id, photo.person_id);

    // The response never carries the storage key or the content hash.
    let json = format!("{:?}", profile.media);
    assert!(!json.contains(&photo.storage_key));
    assert!(!json.contains(&photo.sha256));
}

#[test]
fn pending_photo_is_neither_listed_nor_readable() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, _person, person_key, viewer) = scene(&mut db, "pending");
    let photo = db.upload(
        &owner,
        MediaKind::Photo,
        "pending.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );

    let profile = db
        .family_profile(&viewer, &person_key)
        .expect("family profile");
    assert!(
        profile.media.is_empty(),
        "a pending submission is not presented"
    );
    assert_eq!(
        db.family_media(&viewer, &person_key, photo.media_id),
        Err(MediaError::NotFound)
    );
    // The owner still sees their own pending upload.
    assert!(db.any_media(&owner, photo.media_id).is_ok());
}

#[test]
fn owner_private_photo_is_neither_listed_nor_readable() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, _person, person_key, viewer) = scene(&mut db, "private");
    let photo = db.upload(
        &owner,
        MediaKind::Photo,
        "private.png",
        &png_bytes(),
        ProfileVisibility::OwnerPrivate,
    );
    db.approve(photo.revision_id);

    let profile = db
        .family_profile(&viewer, &person_key)
        .expect("family profile");
    assert!(
        profile.media.is_empty(),
        "owner-private media is not presented"
    );
    assert_eq!(
        db.family_media(&viewer, &person_key, photo.media_id),
        Err(MediaError::NotFound)
    );
    assert_eq!(
        db.any_media(&viewer, photo.media_id),
        Err(MediaError::NotFound),
        "the generic asset path refuses a family member too"
    );
    assert!(db.any_media(&owner, photo.media_id).is_ok());
}

#[test]
fn rejected_photo_is_neither_listed_nor_readable() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, _person, person_key, viewer) = scene(&mut db, "rejected");
    let photo = db.upload(
        &owner,
        MediaKind::Photo,
        "rejected.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );
    db.reject(photo.revision_id);

    let profile = db
        .family_profile(&viewer, &person_key)
        .expect("family profile");
    assert!(
        profile.media.is_empty(),
        "a rejected submission is not presented"
    );
    assert_eq!(
        db.family_media(&viewer, &person_key, photo.media_id),
        Err(MediaError::NotFound)
    );
}

#[test]
fn restricted_photo_is_withheld_from_the_family_but_kept_for_the_owner() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, _person, person_key, viewer) = scene(&mut db, "restricted");
    let photo = db.upload(
        &owner,
        MediaKind::Photo,
        "restricted.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );
    db.approve(photo.revision_id);
    assert_eq!(
        db.family_profile(&viewer, &person_key)
            .expect("family profile")
            .media
            .len(),
        1
    );

    restrict_own_field(&mut db.genealogy, &owner, "profile_photo").expect("restrict photo");

    let profile = db
        .family_profile(&viewer, &person_key)
        .expect("family profile");
    assert!(
        profile.media.is_empty(),
        "a restricted asset is not presented"
    );
    assert_eq!(
        db.family_media(&viewer, &person_key, photo.media_id),
        Err(MediaError::NotFound)
    );
    assert_eq!(
        db.any_media(&viewer, photo.media_id),
        Err(MediaError::NotFound),
        "the direct asset path honours the restriction"
    );
    // The owner keeps their own asset, and the historical revision is intact.
    assert!(db.any_media(&owner, photo.media_id).is_ok());
    let status: String = db
        .genealogy
        .query_one(
            "SELECT status FROM onboarding_profile_revision WHERE revision_id = $1",
            &[&photo.revision_id],
        )
        .expect("revision status")
        .get(0);
    assert_eq!(status, "approved");
}

#[test]
fn unlinked_account_cannot_read_family_media() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, _person, person_key, _viewer) = scene(&mut db, "unlinked");
    let photo = db.upload(
        &owner,
        MediaKind::Photo,
        "unlinked.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );
    db.approve(photo.revision_id);

    let stranger = db.account("stranger");
    assert_eq!(
        db.family_media(&stranger, &person_key, photo.media_id),
        Err(MediaError::Unlinked)
    );
    assert_eq!(
        db.family_profile(&stranger, &person_key),
        Err(FamilyProfileError::NotFamilyMember)
    );
}

#[test]
fn another_persons_media_is_not_reachable_through_this_person() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, _person, person_key, viewer) = scene(&mut db, "owner-a");
    let photo = db.upload(
        &owner,
        MediaKind::Photo,
        "a.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );
    db.approve(photo.revision_id);
    let (_other_person, other_key) = db.person("owner-b");

    // The viewer is a legitimate family member, but the asset belongs to
    // another person, so the person-scoped route reports it as missing.
    assert_eq!(
        db.family_media(&viewer, &other_key, photo.media_id),
        Err(MediaError::NotFound)
    );
    let other = db
        .family_profile(&viewer, &other_key)
        .expect("other profile");
    assert!(other.media.is_empty());
    // And the original person still works.
    assert!(db
        .family_media(&viewer, &person_key, photo.media_id)
        .is_ok());
}

#[test]
fn only_the_current_approved_photo_is_presented() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, _person, person_key, viewer) = scene(&mut db, "superseded");
    let first = db.upload(
        &owner,
        MediaKind::Photo,
        "first.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );
    db.approve(first.revision_id);
    let second = db.upload(
        &owner,
        MediaKind::Photo,
        "second.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );
    db.approve(second.revision_id);

    let profile = db
        .family_profile(&viewer, &person_key)
        .expect("family profile");
    assert_eq!(profile.media.len(), 1, "only the current approved asset");
    assert_eq!(profile.media[0].media_id, second.media_id);
    assert_eq!(profile.media[0].approved_revision_number, 2);

    assert_eq!(
        db.family_media(&viewer, &person_key, first.media_id),
        Err(MediaError::NotFound),
        "a superseded asset is not served"
    );
    assert!(db
        .family_media(&viewer, &person_key, second.media_id)
        .is_ok());
}

#[test]
fn approved_family_document_is_presented_and_readable() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, _person, person_key, viewer) = scene(&mut db, "document");
    let document = db.upload(
        &owner,
        MediaKind::PersonalDocument,
        "family-letter.txt",
        &document_bytes(),
        ProfileVisibility::Family,
    );
    db.approve(document.revision_id);

    let profile = db
        .family_profile(&viewer, &person_key)
        .expect("family profile");
    assert_eq!(profile.media.len(), 1);
    let item = &profile.media[0];
    assert_eq!(item.kind, "personal_document");
    assert_eq!(item.mime_type, "text/plain");
    assert_eq!(item.file_name, "family-letter.txt");

    let download = db
        .family_media(&viewer, &person_key, document.media_id)
        .expect("family document");
    assert_eq!(download.bytes, document_bytes());
}

#[test]
fn a_pre_restriction_media_submission_cannot_lift_the_restriction() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, _person, person_key, viewer) = scene(&mut db, "media-stale");

    // A current approved family photo becomes the value the restriction protects.
    let current = db.upload(
        &owner,
        MediaKind::Photo,
        "current.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );
    db.approve(current.revision_id);
    assert_eq!(
        db.family_profile(&viewer, &person_key)
            .expect("family profile")
            .media
            .len(),
        1
    );

    // A second family submission is created *before* the restriction, so it
    // records no restriction generation.
    let stale = db.upload(
        &owner,
        MediaKind::Photo,
        "stale.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );

    restrict_own_field(&mut db.genealogy, &owner, "profile_photo").expect("restrict photo");

    // Media approval goes through the same decision path as text, so the
    // pre-restriction submission is refused and no lift is recorded.
    assert_eq!(
        review_revision(
            &mut db.genealogy,
            stale.revision_id,
            "reviewer-a",
            ReviewDecision::Approve,
            Some("synthetic pre-restriction approval attempt"),
        ),
        Err(RevisionError::RestrictionConflict)
    );
    assert!(db
        .family_profile(&viewer, &person_key)
        .expect("family profile")
        .media
        .is_empty());
    assert_eq!(
        db.family_media(&viewer, &person_key, stale.media_id),
        Err(MediaError::NotFound)
    );

    // A refused approval leaves the submission pending, so the owner withdraws it
    // before sharing a new one — the same open-slot rule as any media upload.
    withdraw_owned_media(&mut db.genealogy, &db.storage, &owner, stale.media_id)
        .expect("withdraw the refused submission");

    // A submission created *under* the restriction, independently approved,
    // restores family visibility.
    let restored = db.upload(
        &owner,
        MediaKind::Photo,
        "restored.png",
        &png_bytes(),
        ProfileVisibility::Family,
    );
    db.approve(restored.revision_id);
    let listed = db
        .family_profile(&viewer, &person_key)
        .expect("family profile")
        .media;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].media_id, restored.media_id);
}
