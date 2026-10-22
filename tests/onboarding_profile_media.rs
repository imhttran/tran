//! USR-004C private media tests.
//!
//! Synthetic files only. The test writes generated images/text to a temporary
//! `MEDIA_ROOT` and uses disposable identity/genealogy databases.

use std::fs;
use std::io::Cursor;
use std::str::FromStr;
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use image::{DynamicImage, ImageFormat};
use postgres::{Client, NoTls};
use rag_foundation::onboarding::{
    link_reviewed, read_authorized_media, read_review_media, resolve_account, review_revision,
    revoke_link, store_submission, withdraw_owned_media, IdentityAccount, LinkDecision, MediaError,
    MediaKind, MediaStorage, ProfileVisibility, ReviewDecision, RevisionStatus, DOCUMENT_MAX_BYTES,
    PHOTO_MAX_BYTES,
};
use tempfile::TempDir;

const IDENTITY_MIGRATIONS: [&str; 3] = [
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
];

const GENEALOGY_MIGRATIONS: [&str; 21] = [
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
];

const ISSUER: &str = "https://synthetic-issuer.example";

const PERSON_CHILD_TABLES: [&str; 9] = [
    "onboarding_profile_review_event",
    "onboarding_profile_revision",
    "onboarding_profile_value",
    "onboarding_media_withdrawal",
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
        eprintln!("DATABASE_URL or GENEALOGY_DATABASE_URL unset; skipping media test");
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
        "the media test must not target the identity database"
    );
    Some((identity, genealogy))
}

struct TestDb {
    identity: Client,
    genealogy: Client,
    storage: MediaStorage,
    rag_dsn: Option<String>,
    prefix: String,
    genealogy_dsn: String,
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
            rag_dsn: std::env::var("DATABASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            prefix: format!("synthetic-media-{}-{nanos}", std::process::id()),
            genealogy_dsn,
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

    fn person(&mut self, label: &str) -> i64 {
        let key = format!("{}-person-{label}", self.prefix);
        self.genealogy
            .query_one(
                "INSERT INTO person (person_key, display_name) VALUES ($1, $2) RETURNING person_id",
                &[&key, &format!("Synthetic Media Person {label}")],
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

    fn upload(
        &mut self,
        account: &IdentityAccount,
        kind: MediaKind,
        file_name: &str,
        bytes: &[u8],
        visibility: ProfileVisibility,
    ) -> Result<rag_foundation::onboarding::MediaAsset, MediaError> {
        store_submission(
            &mut self.genealogy,
            &self.storage,
            account,
            kind,
            Some(file_name),
            bytes,
            visibility,
        )
    }

    fn download(
        &mut self,
        requester: &IdentityAccount,
        media_id: i64,
    ) -> Result<rag_foundation::onboarding::MediaDownload, MediaError> {
        read_authorized_media(&mut self.genealogy, &self.storage, requester, media_id)
    }

    fn approve(&mut self, revision_id: i64) {
        review_revision(
            &mut self.genealogy,
            revision_id,
            "reviewer-a",
            ReviewDecision::Approve,
            Some("synthetic media approval"),
        )
        .expect("approve media revision");
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

fn png_bytes() -> Vec<u8> {
    let image = DynamicImage::new_rgb8(24, 24);
    let mut out = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .expect("encode synthetic png");
    out
}

fn jpeg_with_metadata(marker: &[u8]) -> Vec<u8> {
    let image = DynamicImage::new_rgb8(24, 24);
    let mut base = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut base), ImageFormat::Jpeg)
        .expect("encode synthetic jpeg");
    let mut app1 = vec![0xFF, 0xE1];
    let length = (2 + 6 + marker.len()) as u16;
    app1.extend(length.to_be_bytes());
    app1.extend(b"Exif\0\0");
    app1.extend(marker);
    let mut out = base[..2].to_vec();
    out.extend(app1);
    out.extend(&base[2..]);
    out
}

#[test]
fn photo_is_private_until_approved_and_current() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("photo-owner");
    let person = db.person("photo");
    db.link(&owner, person);
    let family = db.account("photo-family");
    let family_person = db.person("photo-family");
    db.link(&family, family_person);

    let photo = db
        .upload(
            &owner,
            MediaKind::Photo,
            "photo.png",
            &png_bytes(),
            ProfileVisibility::Family,
        )
        .expect("upload photo");
    assert_eq!(photo.mime_type, "image/png");
    let owner_pending = db.download(&owner, photo.media_id).expect("owner pending");
    assert!(owner_pending.bytes.starts_with(b"\x89PNG"));
    assert_eq!(
        db.download(&family, photo.media_id),
        Err(MediaError::NotFound),
        "pending photos are owner-only"
    );

    db.approve(photo.revision_id);
    let family_download = db
        .download(&family, photo.media_id)
        .expect("approved family photo");
    assert_eq!(family_download.asset.mime_type, "image/png");
    assert!(family_download.bytes.starts_with(b"\x89PNG"));

    let replacement = db
        .upload(
            &owner,
            MediaKind::Photo,
            "replacement.png",
            &png_bytes(),
            ProfileVisibility::Family,
        )
        .expect("upload replacement");
    assert_eq!(
        db.download(&family, replacement.media_id),
        Err(MediaError::NotFound),
        "a pending replacement is not family-visible"
    );
    assert!(
        db.download(&family, photo.media_id).is_ok(),
        "the previous approved photo stays current until replacement approval"
    );
    db.approve(replacement.revision_id);
    assert_eq!(
        db.download(&family, photo.media_id),
        Err(MediaError::NotFound),
        "only the current approved photo is family-visible"
    );
    assert!(db.download(&family, replacement.media_id).is_ok());
}

#[test]
fn document_is_private_until_approved_and_current() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("document-owner");
    let person = db.person("document");
    db.link(&owner, person);
    let family = db.account("document-family");
    let family_person = db.person("document-family");
    db.link(&family, family_person);
    let rag_before = db.rag_document_count();

    let content = b"# Synthetic personal document\n\nNo real family data.\n";
    let document = db
        .upload(
            &owner,
            MediaKind::PersonalDocument,
            "family.md",
            content,
            ProfileVisibility::Family,
        )
        .expect("upload document");
    assert_eq!(document.mime_type, "text/plain");
    assert_eq!(
        db.download(&family, document.media_id),
        Err(MediaError::NotFound),
        "pending documents are owner-only"
    );

    db.approve(document.revision_id);
    let family_download = db
        .download(&family, document.media_id)
        .expect("approved document");
    assert_eq!(family_download.bytes, content);

    if let Some(before) = rag_before {
        assert_eq!(
            db.rag_document_count(),
            Some(before),
            "uploading or approving media must not ingest it into RAG"
        );
    }
}

#[test]
fn cross_account_and_revoked_link_cannot_access_media() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("access-owner");
    let person = db.person("access");
    db.link(&owner, person);
    let other = db.account("access-other");
    let other_person = db.person("access-other");
    db.link(&other, other_person);
    let photo = db
        .upload(
            &owner,
            MediaKind::Photo,
            "private.png",
            &png_bytes(),
            ProfileVisibility::OwnerPrivate,
        )
        .expect("upload photo");
    db.approve(photo.revision_id);
    assert_eq!(
        db.download(&other, photo.media_id),
        Err(MediaError::NotFound),
        "another account cannot read an owner-private approved photo"
    );
    assert_eq!(
        withdraw_owned_media(&mut db.genealogy, &db.storage, &other, photo.media_id),
        Err(MediaError::NotFound),
        "another account cannot withdraw the owner's media"
    );

    let link = rag_foundation::onboarding::link_for_account(
        &mut db.genealogy,
        owner.account_id,
        &owner.identity_fingerprint,
    )
    .expect("find owner link")
    .expect("owner link");
    revoke_link(&mut db.genealogy, link.link_id, "synthetic-reviewer").expect("revoke link");
    assert_eq!(
        db.download(&owner, photo.media_id),
        Err(MediaError::Unlinked),
        "a revoked account cannot read its former media"
    );
    assert_eq!(
        db.upload(
            &owner,
            MediaKind::Photo,
            "after-revoke.png",
            &png_bytes(),
            ProfileVisibility::OwnerPrivate,
        ),
        Err(MediaError::Unlinked)
    );
}

#[test]
fn replacement_owner_sees_only_current_approved_or_own_media() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let former = db.account("media-replacement-former");
    let person = db.person("media-replacement");
    db.link(&former, person);
    let pending = db
        .upload(
            &former,
            MediaKind::Photo,
            "pending.png",
            &png_bytes(),
            ProfileVisibility::OwnerPrivate,
        )
        .expect("pending photo");

    let former_link = rag_foundation::onboarding::link_for_account(
        &mut db.genealogy,
        former.account_id,
        &former.identity_fingerprint,
    )
    .expect("find former link")
    .expect("former link");
    revoke_link(&mut db.genealogy, former_link.link_id, "synthetic-reviewer")
        .expect("revoke former link");
    let replacement = db.account("media-replacement-new");
    db.link(&replacement, person);

    assert_eq!(
        db.download(&replacement, pending.media_id),
        Err(MediaError::NotFound),
        "a replacement owner must not read the former owner's pending media"
    );

    db.approve(pending.revision_id);
    assert!(
        db.download(&replacement, pending.media_id).is_ok(),
        "the current approved media is person-level and readable by the owner"
    );

    let replacement_pending = db
        .upload(
            &replacement,
            MediaKind::Photo,
            "replacement-pending.png",
            &png_bytes(),
            ProfileVisibility::OwnerPrivate,
        )
        .expect("replacement pending photo");
    assert_eq!(
        db.download(&replacement, replacement_pending.media_id)
            .expect("own pending media")
            .asset
            .media_id,
        replacement_pending.media_id
    );
}

#[test]
fn malformed_and_oversized_media_are_rejected() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("bad-media");
    let person = db.person("bad-media");
    db.link(&owner, person);

    assert!(matches!(
        db.upload(
            &owner,
            MediaKind::Photo,
            "bad.png",
            b"not an image",
            ProfileVisibility::OwnerPrivate
        ),
        Err(MediaError::UnsupportedType | MediaError::InvalidImage)
    ));
    assert_eq!(
        db.upload(
            &owner,
            MediaKind::Photo,
            "large.png",
            &vec![0u8; PHOTO_MAX_BYTES + 1],
            ProfileVisibility::OwnerPrivate
        ),
        Err(MediaError::TooLarge)
    );
    assert_eq!(
        db.upload(
            &owner,
            MediaKind::PersonalDocument,
            "large.txt",
            &vec![b'a'; DOCUMENT_MAX_BYTES + 1],
            ProfileVisibility::OwnerPrivate
        ),
        Err(MediaError::TooLarge)
    );
    assert_eq!(
        db.upload(
            &owner,
            MediaKind::PersonalDocument,
            "broken.pdf",
            b"%PDF-1.4\nthis is not a complete pdf",
            ProfileVisibility::OwnerPrivate
        ),
        Err(MediaError::InvalidDocument)
    );
    assert_eq!(
        db.upload(
            &owner,
            MediaKind::PersonalDocument,
            "evil.exe",
            b"plain text with a dangerous extension",
            ProfileVisibility::OwnerPrivate
        ),
        Err(MediaError::UnsupportedType)
    );

    let mut wide = Vec::new();
    DynamicImage::new_rgb8(4_097, 8)
        .write_to(&mut Cursor::new(&mut wide), ImageFormat::Png)
        .expect("encode wide png");
    assert_eq!(
        db.upload(
            &owner,
            MediaKind::Photo,
            "wide.png",
            &wide,
            ProfileVisibility::OwnerPrivate
        ),
        Err(MediaError::TooLarge),
        "images above the dimension limit are rejected, not silently resized"
    );
}

#[test]
fn photo_metadata_is_stripped_by_reencoding() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("metadata-owner");
    let person = db.person("metadata");
    db.link(&owner, person);
    let marker = b"SECRET_METADATA_MARKER";
    let photo = db
        .upload(
            &owner,
            MediaKind::Photo,
            "metadata.jpg",
            &jpeg_with_metadata(marker),
            ProfileVisibility::OwnerPrivate,
        )
        .expect("upload jpeg");
    let download = db.download(&owner, photo.media_id).expect("download photo");
    assert_eq!(download.asset.mime_type, "image/png");
    assert!(
        !download
            .bytes
            .windows(marker.len())
            .any(|window| window == marker),
        "re-encoded media must not retain original metadata"
    );
}

#[test]
fn owner_can_withdraw_pending_media_and_reupload() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("withdraw-owner");
    let person = db.person("withdraw");
    db.link(&owner, person);
    let photo = db
        .upload(
            &owner,
            MediaKind::Photo,
            "withdraw.png",
            &png_bytes(),
            ProfileVisibility::OwnerPrivate,
        )
        .expect("upload photo");

    withdraw_owned_media(&mut db.genealogy, &db.storage, &owner, photo.media_id)
        .expect("withdraw pending photo");
    assert_eq!(
        db.download(&owner, photo.media_id),
        Err(MediaError::NotFound),
        "withdrawn media is no longer readable"
    );
    let replacement = db
        .upload(
            &owner,
            MediaKind::Photo,
            "replacement.png",
            &png_bytes(),
            ProfileVisibility::OwnerPrivate,
        )
        .expect("reupload after withdrawal");
    assert!(db.download(&owner, replacement.media_id).is_ok());
}

#[test]
fn withdrawal_audit_retains_references_after_revision_deletion() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("audit-owner");
    let person = db.person("audit");
    db.link(&owner, person);
    let document = db
        .upload(
            &owner,
            MediaKind::PersonalDocument,
            "audit.txt",
            b"synthetic family document for the withdrawal audit test",
            ProfileVisibility::Family,
        )
        .expect("upload document");

    let events_before: i64 = db
        .genealogy
        .query_one(
            "SELECT count(*) FROM onboarding_profile_review_event WHERE revision_id = $1",
            &[&document.revision_id],
        )
        .expect("count review events")
        .get(0);
    assert!(events_before > 0, "a submission records a review event");

    withdraw_owned_media(&mut db.genealogy, &db.storage, &owner, document.media_id)
        .expect("withdraw pending document");

    let revision_exists: bool = db
        .genealogy
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM onboarding_profile_revision WHERE revision_id = $1)",
            &[&document.revision_id],
        )
        .expect("revision lookup")
        .get(0);
    assert!(!revision_exists, "withdrawal removes the pending revision");

    let audit = db
        .genealogy
        .query_one(
            "SELECT media_id, revision_id, person_id, media_kind, action, actor, \
                    created_at::text, \
                    (SELECT count(*) FROM pg_attribute \
                      WHERE attrelid = 'onboarding_media_withdrawal'::regclass \
                        AND attname IN ('storage_key', 'file_name', 'sha256', 'content')) \
               FROM onboarding_media_withdrawal WHERE media_id = $1",
            &[&document.media_id],
        )
        .expect("withdrawal audit row survives revision deletion");
    assert_eq!(audit.get::<_, i64>(0), document.media_id);
    assert_eq!(audit.get::<_, i64>(1), document.revision_id);
    assert_eq!(audit.get::<_, i64>(2), person);
    assert_eq!(audit.get::<_, String>(3), "personal_document");
    assert_eq!(audit.get::<_, String>(4), "withdraw");
    assert_eq!(audit.get::<_, String>(5), owner.identity_fingerprint);
    assert!(!audit.get::<_, String>(6).trim().is_empty());
    assert_eq!(
        audit.get::<_, i64>(7),
        0,
        "the audit ledger must not retain private payload columns"
    );
}

#[test]
fn unauthorized_or_repeated_withdrawal_writes_no_audit() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("audit-negative-owner");
    let other = db.account("audit-negative-other");
    let person = db.person("audit-negative");
    let other_person = db.person("audit-negative-other");
    db.link(&owner, person);
    db.link(&other, other_person);
    let photo = db
        .upload(
            &owner,
            MediaKind::Photo,
            "negative.png",
            &png_bytes(),
            ProfileVisibility::OwnerPrivate,
        )
        .expect("upload photo");

    assert_eq!(
        withdraw_owned_media(&mut db.genealogy, &db.storage, &other, photo.media_id),
        Err(MediaError::NotFound),
        "another account cannot withdraw the owner's media"
    );
    let audit_count = |db: &mut TestDb| -> i64 {
        db.genealogy
            .query_one(
                "SELECT count(*) FROM onboarding_media_withdrawal WHERE media_id = $1",
                &[&photo.media_id],
            )
            .expect("count audit rows")
            .get(0)
    };
    assert_eq!(
        audit_count(&mut db),
        0,
        "a denied withdrawal is not audited"
    );

    withdraw_owned_media(&mut db.genealogy, &db.storage, &owner, photo.media_id)
        .expect("owner withdraws pending photo");
    assert_eq!(audit_count(&mut db), 1);
    assert_eq!(
        withdraw_owned_media(&mut db.genealogy, &db.storage, &owner, photo.media_id),
        Err(MediaError::NotFound),
        "a withdrawn media submission cannot be withdrawn twice"
    );
    assert_eq!(
        audit_count(&mut db),
        1,
        "repeat withdrawal adds no audit row"
    );
}

#[test]
fn media_routes_are_registered() {
    let source = include_str!("../src/api.rs");
    for route in [
        "/api/profile/media/photo",
        "/api/profile/media/personal-document",
        "/api/profile/media/{id}",
        "/api/admin/reviews",
        "/api/admin/reviews/media/{id}",
        "/api/admin/reviews/{id}",
    ] {
        assert!(source.contains(route), "missing route {route}");
    }
    assert!(source.contains("store_submission"));
    assert!(source.contains("read_authorized_media"));
    assert!(source.contains("withdraw_owned_media"));
    assert!(source.contains("MediaStorage"));
}

#[test]
fn review_media_is_pending_only_and_requires_an_independent_reviewer() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("review-media-owner");
    let person = db.person("review-media");
    db.link(&owner, person);
    let photo = db
        .upload(
            &owner,
            MediaKind::Photo,
            "review-media.png",
            &png_bytes(),
            ProfileVisibility::Family,
        )
        .expect("upload pending photo");

    let reviewer = "reviewer-media-fingerprint";
    let download = read_review_media(&mut db.genealogy, &db.storage, reviewer, photo.media_id)
        .expect("independent reviewer reads pending media");
    assert_eq!(download.asset.status, RevisionStatus::PendingReview);
    assert!(!download.bytes.is_empty());

    assert_eq!(
        read_review_media(
            &mut db.genealogy,
            &db.storage,
            &owner.identity_fingerprint,
            photo.media_id,
        ),
        Err(MediaError::NotFound),
        "the submitter cannot use the review media path"
    );

    // Replace the account holder while the submission stays pending. The new
    // owner must still be denied because they own the linked person.
    let replacement_owner = db.account("review-media-replacement");
    let link_id: i64 = db
        .genealogy
        .query_one(
            "SELECT link_id FROM onboarding_person_account_link WHERE person_id = $1",
            &[&person],
        )
        .expect("read link")
        .get(0);
    revoke_link(&mut db.genealogy, link_id, "synthetic-reviewer").expect("revoke link");
    db.link(&replacement_owner, person);
    assert_eq!(
        read_review_media(
            &mut db.genealogy,
            &db.storage,
            &replacement_owner.identity_fingerprint,
            photo.media_id,
        ),
        Err(MediaError::NotFound),
        "the linked owner cannot review their own media submission"
    );

    db.approve(photo.revision_id);
    assert_eq!(
        read_review_media(&mut db.genealogy, &db.storage, reviewer, photo.media_id),
        Err(MediaError::NotFound),
        "the review media path is limited to pending submissions"
    );
}

fn wait_until_blocked(control: &mut Client, pid: i32) {
    for _ in 0..50_000 {
        let blocked: bool = control
            .query_one("SELECT cardinality(pg_blocking_pids($1)) > 0", &[&pid])
            .expect("read blocking pids")
            .get(0);
        if blocked {
            return;
        }
        std::thread::yield_now();
    }
    panic!("backend {pid} never blocked on the expected row lock");
}

#[test]
fn review_media_waits_for_decision_lock_then_denies_terminal_revision() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("race-decision-owner");
    let person = db.person("race-decision");
    db.link(&owner, person);
    let photo = db
        .upload(
            &owner,
            MediaKind::Photo,
            "race-decision.png",
            &png_bytes(),
            ProfileVisibility::Family,
        )
        .expect("upload pending photo");
    let reviewer = "reviewer-race-decision";

    let mut locker = Client::connect(&db.genealogy_dsn, NoTls).expect("decision lock connection");
    let mut tx = locker.transaction().expect("decision transaction");
    tx.query_one(
        "SELECT 1 FROM onboarding_profile_revision WHERE revision_id = $1 FOR UPDATE",
        &[&photo.revision_id],
    )
    .expect("lock revision like a decision");

    let storage = db.storage.clone();
    let dsn = db.genealogy_dsn.clone();
    let media_id = photo.media_id;
    let (pid_tx, pid_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut client = Client::connect(&dsn, NoTls).expect("media reader connection");
        let pid: i32 = client
            .query_one("SELECT pg_backend_pid()", &[])
            .expect("reader pid")
            .get(0);
        pid_tx.send(pid).expect("send reader pid");
        read_review_media(&mut client, &storage, reviewer, media_id)
    });
    let reader_pid = pid_rx.recv().expect("reader pid received");
    wait_until_blocked(&mut db.genealogy, reader_pid);

    tx.execute(
        "UPDATE onboarding_profile_revision
            SET status = 'rejected', reviewed_at = now(),
                reviewed_by = 'reviewer-race-decision', review_note = 'race rejection'
          WHERE revision_id = $1",
        &[&photo.revision_id],
    )
    .expect("terminal decision update");
    tx.commit().expect("commit terminal decision");

    assert_eq!(
        reader.join().expect("reader joined"),
        Err(MediaError::NotFound),
        "a read that waited on the decision lock must observe the terminal state"
    );
}

#[test]
fn review_media_waits_for_account_replacement_lock_then_denies_new_owner() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("race-link-owner");
    let person = db.person("race-link");
    db.link(&owner, person);
    let photo = db
        .upload(
            &owner,
            MediaKind::Photo,
            "race-link.png",
            &png_bytes(),
            ProfileVisibility::Family,
        )
        .expect("upload pending photo");
    let reviewer = "reviewer-race-owner";

    let mut locker = Client::connect(&db.genealogy_dsn, NoTls).expect("link lock connection");
    let mut tx = locker.transaction().expect("link transaction");
    tx.query_one(
        "SELECT 1 FROM person WHERE person_id = $1 FOR UPDATE",
        &[&person],
    )
    .expect("lock person like a link change");
    tx.execute(
        "DELETE FROM onboarding_person_account_link WHERE person_id = $1",
        &[&person],
    )
    .expect("remove previous account link");
    tx.execute(
        "INSERT INTO onboarding_person_account_link
             (person_id, account_id, linked_by, identity_fingerprint)
         VALUES ($1, 9911, 'synthetic-reviewer', $2)",
        &[&person, &reviewer],
    )
    .expect("replace with reviewer link");

    let storage = db.storage.clone();
    let dsn = db.genealogy_dsn.clone();
    let media_id = photo.media_id;
    let (pid_tx, pid_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut client = Client::connect(&dsn, NoTls).expect("media reader connection");
        let pid: i32 = client
            .query_one("SELECT pg_backend_pid()", &[])
            .expect("reader pid")
            .get(0);
        pid_tx.send(pid).expect("send reader pid");
        read_review_media(&mut client, &storage, reviewer, media_id)
    });
    let reader_pid = pid_rx.recv().expect("reader pid received");
    wait_until_blocked(&mut db.genealogy, reader_pid);
    tx.commit().expect("commit link replacement");

    assert_eq!(
        reader.join().expect("reader joined"),
        Err(MediaError::NotFound),
        "a read that waited on a link change must observe the new owner"
    );
}

#[test]
fn review_media_is_closed_after_rejection_and_withdrawal() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("race-closure-owner");
    let person = db.person("race-closure");
    db.link(&owner, person);
    let reviewer = "reviewer-race-closure";

    let rejected = db
        .upload(
            &owner,
            MediaKind::PersonalDocument,
            "rejected.txt",
            b"synthetic rejected document",
            ProfileVisibility::Family,
        )
        .expect("upload rejected document");
    review_revision(
        &mut db.genealogy,
        rejected.revision_id,
        reviewer,
        ReviewDecision::Reject,
        Some("synthetic rejection"),
    )
    .expect("reject document");
    assert_eq!(
        read_review_media(&mut db.genealogy, &db.storage, reviewer, rejected.media_id,),
        Err(MediaError::NotFound)
    );

    let withdrawn = db
        .upload(
            &owner,
            MediaKind::Photo,
            "withdrawn.png",
            &png_bytes(),
            ProfileVisibility::Family,
        )
        .expect("upload withdrawn photo");
    withdraw_owned_media(&mut db.genealogy, &db.storage, &owner, withdrawn.media_id)
        .expect("withdraw pending photo");
    assert_eq!(
        read_review_media(&mut db.genealogy, &db.storage, reviewer, withdrawn.media_id,),
        Err(MediaError::NotFound)
    );
}
