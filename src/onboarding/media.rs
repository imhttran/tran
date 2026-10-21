//! Private owner media: one photo and one personal document per member.
//!
//! Binary content lives under `MEDIA_ROOT`; the database stores metadata only.
//! Each upload creates a normal `onboarding_profile_revision` in
//! `pending_review` state, so approval, rejection, version conflicts, and the
//! owner history use the existing lifecycle instead of a second review engine.
//!
//! Photos are decoded and re-encoded to PNG, which strips container metadata.
//! Documents are validated as PDF or UTF-8 text/Markdown and stored byte-for-byte.
//! Pending, rejected, and needs-changes media is owner-visible only; family
//! retrieval requires an approved revision with `family` visibility.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use image::{ImageFormat, ImageReader, Limits};
use ring::digest;
use thiserror::Error;

use crate::onboarding::account_map::IdentityAccount;
use crate::onboarding::profile_revisions::{ProfileVisibility, RevisionStatus};

/// Maximum accepted photo size (5 MiB, matching the family-tree plan).
pub const PHOTO_MAX_BYTES: usize = 5 * 1024 * 1024;
/// Maximum accepted personal-document size (10 MiB, matching the existing upload limit).
pub const DOCUMENT_MAX_BYTES: usize = 10 * 1024 * 1024;
/// Maximum decoded image dimension on either axis.
pub const PHOTO_MAX_DIMENSION: u32 = 4_096;
/// Hard decoder guard before the explicit dimension rejection.
const PHOTO_DECODE_MAX_DIMENSION: u32 = 16_384;
/// Maximum allocation during image decoding.
pub const PHOTO_MAX_ALLOC: u64 = 256 * 1024 * 1024;

pub const PHOTO_FIELD: &str = "profile_photo";
pub const DOCUMENT_FIELD: &str = "personal_document";

/// The two media slots owned by a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Photo,
    PersonalDocument,
}

impl MediaKind {
    pub fn field_key(self) -> &'static str {
        match self {
            MediaKind::Photo => PHOTO_FIELD,
            MediaKind::PersonalDocument => DOCUMENT_FIELD,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            PHOTO_FIELD => Some(MediaKind::Photo),
            DOCUMENT_FIELD => Some(MediaKind::PersonalDocument),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        self.field_key()
    }

    /// Maximum file bytes for this slot.
    pub fn max_bytes(self) -> usize {
        match self {
            MediaKind::Photo => PHOTO_MAX_BYTES,
            MediaKind::PersonalDocument => DOCUMENT_MAX_BYTES,
        }
    }

    /// Maximum multipart body size, including framing overhead.
    pub fn max_body_bytes(self) -> usize {
        self.max_bytes() + 64 * 1024
    }
}

/// Stored media metadata, including the review state of its revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaAsset {
    pub media_id: i64,
    pub person_id: i64,
    pub revision_id: i64,
    pub kind: MediaKind,
    pub mime_type: String,
    pub byte_size: i64,
    pub sha256: String,
    pub file_name: String,
    pub visibility: ProfileVisibility,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub status: RevisionStatus,
    pub created_at: String,
    pub storage_key: String,
}

/// An authorized media download. The caller writes these bytes to the response;
/// the storage path is never exposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDownload {
    pub asset: MediaAsset,
    pub bytes: Vec<u8>,
}

/// Errors raised by media validation or access.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MediaError {
    #[error("the authenticated account has no reviewed person link")]
    Unlinked,
    #[error("no such media")]
    NotFound,
    #[error("an open revision already exists for this media slot")]
    OpenRevisionExists,
    #[error("the uploaded file is too large")]
    TooLarge,
    #[error("the uploaded file type is not supported")]
    UnsupportedType,
    #[error("the uploaded image could not be decoded or re-encoded")]
    InvalidImage,
    #[error("the uploaded document could not be validated")]
    InvalidDocument,
    #[error("the media revision is not pending review")]
    InvalidTransition,
    #[error("the media file is missing from private storage")]
    MissingFile,
    #[error("media storage operation failed: {0}")]
    Io(String),
    #[error("media database operation failed: {0}")]
    Database(String),
}

impl From<postgres::Error> for MediaError {
    fn from(err: postgres::Error) -> Self {
        MediaError::Database(err.to_string())
    }
}

/// Private filesystem storage rooted at `MEDIA_ROOT`.
#[derive(Debug, Clone)]
pub struct MediaStorage {
    root: PathBuf,
}

impl MediaStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn person_dir(&self, person_id: i64) -> PathBuf {
        self.root.join(person_id.to_string())
    }

    fn path_for(
        &self,
        person_id: i64,
        storage_key: &str,
        extension: &str,
    ) -> Result<PathBuf, MediaError> {
        if !safe_component(storage_key) || !safe_component(extension) {
            return Err(MediaError::Io("unsafe media storage key".to_string()));
        }
        Ok(self
            .person_dir(person_id)
            .join(format!("{storage_key}.{extension}")))
    }

    fn write_temp(
        &self,
        person_id: i64,
        storage_key: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, MediaError> {
        let dir = self.person_dir(person_id);
        fs::create_dir_all(&self.root).map_err(|err| MediaError::Io(err.to_string()))?;
        fs::create_dir_all(&dir).map_err(|err| MediaError::Io(err.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700));
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        }
        let path = dir.join(format!(".{storage_key}.tmp"));
        if !safe_component(storage_key) {
            return Err(MediaError::Io("unsafe media storage key".to_string()));
        }
        fs::write(&path, bytes).map_err(|err| MediaError::Io(err.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        Ok(path)
    }

    fn finalize(
        &self,
        temp: &Path,
        person_id: i64,
        storage_key: &str,
        extension: &str,
    ) -> Result<(), MediaError> {
        let final_path = self.path_for(person_id, storage_key, extension)?;
        fs::rename(temp, final_path).map_err(|err| MediaError::Io(err.to_string()))
    }

    fn read(
        &self,
        person_id: i64,
        storage_key: &str,
        extension: &str,
    ) -> Result<Vec<u8>, MediaError> {
        let path = self.path_for(person_id, storage_key, extension)?;
        fs::read(path).map_err(|_| MediaError::MissingFile)
    }

    fn remove_final(&self, person_id: i64, storage_key: &str, extension: &str) {
        if let Ok(path) = self.path_for(person_id, storage_key, extension) {
            let _ = fs::remove_file(path);
        }
    }
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && !value.contains("..")
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

struct PreparedMedia {
    bytes: Vec<u8>,
    mime_type: &'static str,
    extension: &'static str,
    file_name: String,
    width: Option<i32>,
    height: Option<i32>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = digest::digest(&digest::SHA256, bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest.as_ref() {
        use std::fmt::Write as _;
        write!(hex, "{byte:02x}").expect("write to String");
    }
    hex
}

fn storage_key(person_id: i64, sha256: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let prefix = sha256.get(..16).unwrap_or(sha256);
    format!("{person_id}-{nanos}-{prefix}")
}

fn sanitize_file_name(name: Option<&str>, fallback: &str) -> String {
    let base = name
        .unwrap_or(fallback)
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(fallback);
    let cleaned: String = base
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    let mut result = if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    };
    result.truncate(120);
    result
}

fn prepare_photo(bytes: &[u8]) -> Result<PreparedMedia, MediaError> {
    if bytes.len() > PHOTO_MAX_BYTES {
        return Err(MediaError::TooLarge);
    }
    let format = image::guess_format(bytes).map_err(|_| MediaError::UnsupportedType)?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg) {
        return Err(MediaError::UnsupportedType);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| MediaError::InvalidImage)?;
    reader.set_format(format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(PHOTO_DECODE_MAX_DIMENSION);
    limits.max_image_height = Some(PHOTO_DECODE_MAX_DIMENSION);
    limits.max_alloc = Some(PHOTO_MAX_ALLOC);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| MediaError::InvalidImage)?;
    if image.width() > PHOTO_MAX_DIMENSION || image.height() > PHOTO_MAX_DIMENSION {
        return Err(MediaError::TooLarge);
    }
    let mut output = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut output), ImageFormat::Png)
        .map_err(|_| MediaError::InvalidImage)?;
    if output.len() > PHOTO_MAX_BYTES {
        return Err(MediaError::TooLarge);
    }
    Ok(PreparedMedia {
        bytes: output,
        mime_type: "image/png",
        extension: "png",
        file_name: "photo.png".to_string(),
        width: Some(image.width() as i32),
        height: Some(image.height() as i32),
    })
}

fn prepare_document(file_name: Option<&str>, bytes: &[u8]) -> Result<PreparedMedia, MediaError> {
    if bytes.len() > DOCUMENT_MAX_BYTES {
        return Err(MediaError::TooLarge);
    }
    let safe_name = sanitize_file_name(file_name, "document.txt");
    if bytes.starts_with(b"%PDF-") {
        lopdf::Document::load_mem(bytes).map_err(|_| MediaError::InvalidDocument)?;
        return Ok(PreparedMedia {
            bytes: bytes.to_vec(),
            mime_type: "application/pdf",
            extension: "pdf",
            file_name: with_extension(&safe_name, "pdf"),
            width: None,
            height: None,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MediaError::UnsupportedType)?;
    let lower = safe_name.to_ascii_lowercase();
    if !(lower.ends_with(".txt") || lower.ends_with(".md") || lower.ends_with(".markdown")) {
        return Err(MediaError::UnsupportedType);
    }
    if text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(MediaError::InvalidDocument);
    }
    Ok(PreparedMedia {
        bytes: bytes.to_vec(),
        mime_type: "text/plain",
        extension: "txt",
        file_name: with_extension(&safe_name, "txt"),
        width: None,
        height: None,
    })
}

fn with_extension(name: &str, extension: &str) -> String {
    let stem = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name);
    format!("{stem}.{extension}")
}

fn owner_person_in_tx(
    tx: &mut postgres::Transaction<'_>,
    account: &IdentityAccount,
) -> Result<i64, MediaError> {
    let row = tx.query_opt(
        "SELECT person_id FROM onboarding_person_account_link \
          WHERE account_id = $1 AND identity_fingerprint = $2 FOR SHARE",
        &[&account.account_id, &account.identity_fingerprint],
    )?;
    row.map(|row| row.get(0)).ok_or(MediaError::Unlinked)
}

fn lock_field(
    tx: &mut postgres::Transaction<'_>,
    person_id: i64,
    field_key: &str,
) -> Result<(), MediaError> {
    let key = format!("profile-revision:{person_id}:{field_key}");
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
        &[&key],
    )?;
    Ok(())
}

fn row_to_asset(row: postgres::Row) -> Result<MediaAsset, MediaError> {
    let kind_value: String = row.get(3);
    let kind = MediaKind::parse(&kind_value)
        .ok_or_else(|| MediaError::Database(format!("unknown media kind {kind_value:?}")))?;
    let visibility_value: String = row.get(8);
    let visibility = match visibility_value.as_str() {
        "owner_private" => ProfileVisibility::OwnerPrivate,
        "family" => ProfileVisibility::Family,
        other => {
            return Err(MediaError::Database(format!(
                "unknown visibility {other:?}"
            )))
        }
    };
    let status_value: String = row.get(12);
    let status = match status_value.as_str() {
        "draft" => RevisionStatus::Draft,
        "pending_review" => RevisionStatus::PendingReview,
        "approved" => RevisionStatus::Approved,
        "rejected" => RevisionStatus::Rejected,
        "needs_changes" => RevisionStatus::NeedsChanges,
        other => {
            return Err(MediaError::Database(format!(
                "unknown revision status {other:?}"
            )))
        }
    };
    Ok(MediaAsset {
        media_id: row.get(0),
        person_id: row.get(1),
        revision_id: row.get(2),
        kind,
        mime_type: row.get(4),
        byte_size: row.get(5),
        sha256: row.get(6),
        file_name: row.get(7),
        visibility,
        width: row.get(9),
        height: row.get(10),
        created_at: row.get(11),
        status,
        storage_key: row.get(13),
    })
}

const MEDIA_COLUMNS: &str = "m.media_id, m.person_id, m.revision_id, m.media_kind, \
     m.mime_type, m.byte_size, m.sha256, m.file_name, m.visibility, m.width, m.height, \
     m.created_at::text, r.status, m.storage_key";

fn extension_for(asset: &MediaAsset) -> &'static str {
    extension_for_mime(&asset.mime_type)
}

fn extension_for_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/png" => "png",
        "application/pdf" => "pdf",
        "text/markdown" => "md",
        _ => "txt",
    }
}

/// Stores one validated media submission and its pending review revision.
///
/// The owner link is rechecked and share-locked in the same transaction as the
/// write. The file is written before commit and removed if commit fails.
pub fn store_submission(
    client: &mut postgres::Client,
    storage: &MediaStorage,
    account: &IdentityAccount,
    kind: MediaKind,
    file_name: Option<&str>,
    bytes: &[u8],
    visibility: ProfileVisibility,
) -> Result<MediaAsset, MediaError> {
    let prepared = match kind {
        MediaKind::Photo => prepare_photo(bytes)?,
        MediaKind::PersonalDocument => prepare_document(file_name, bytes)?,
    };
    let sha256 = sha256_hex(&prepared.bytes);
    let storage_key = storage_key(account.account_id, &sha256);
    let mut tx = client.transaction()?;
    let person_id = owner_person_in_tx(&mut tx, account)?;
    let field_key = kind.field_key();
    lock_field(&mut tx, person_id, field_key)?;

    let open: bool = tx
        .query_one(
            "SELECT EXISTS (\
                SELECT 1 FROM onboarding_profile_revision \
                 WHERE person_id = $1 AND field_key = $2 AND changed_by = $3 \
                   AND status IN ('draft', 'pending_review')\
             )",
            &[&person_id, &field_key, &account.identity_fingerprint],
        )?
        .get(0);
    if open {
        return Err(MediaError::OpenRevisionExists);
    }

    let current = tx.query_opt(
        "SELECT value_text, approved_revision_number FROM onboarding_profile_value \
          WHERE person_id = $1 AND field_key = $2",
        &[&person_id, &field_key],
    )?;
    let (prior_value, prior_revision_number) = match current {
        Some(row) => (Some(row.get::<_, String>(0)), row.get::<_, i64>(1)),
        None => (None, 0),
    };
    let revision_number: i64 = tx
        .query_one(
            "SELECT COALESCE(max(revision_number), 0) + 1 \
               FROM onboarding_profile_revision \
              WHERE person_id = $1 AND field_key = $2",
            &[&person_id, &field_key],
        )?
        .get(0);

    let revision_id: i64 = tx
        .query_one(
            "INSERT INTO onboarding_profile_revision \
                 (person_id, field_key, prior_value_text, prior_approved_revision_number, \
                  new_value_text, changed_by, revision_number, status, visibility) \
             VALUES ($1, $2, $3, $4, '0', $5, $6, 'draft', $7) \
             RETURNING revision_id",
            &[
                &person_id,
                &field_key,
                &prior_value,
                &prior_revision_number,
                &account.identity_fingerprint,
                &revision_number,
                &visibility.as_str(),
            ],
        )?
        .get(0);

    let media_row = tx.query_one(
        "INSERT INTO onboarding_media_metadata \
                 (person_id, revision_id, media_kind, mime_type, byte_size, sha256, \
                  storage_key, file_name, visibility, width, height) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
             RETURNING media_id, created_at::text",
        &[
            &person_id,
            &revision_id,
            &field_key,
            &prepared.mime_type,
            &(prepared.bytes.len() as i64),
            &sha256,
            &storage_key,
            &prepared.file_name,
            &visibility.as_str(),
            &prepared.width,
            &prepared.height,
        ],
    )?;
    let media_id: i64 = media_row.get(0);
    let created_at: String = media_row.get(1);
    tx.execute(
        "UPDATE onboarding_profile_revision \
            SET new_value_text = $2, status = 'pending_review', submitted_at = now() \
          WHERE revision_id = $1 AND status = 'draft'",
        &[&revision_id, &media_id.to_string()],
    )?;

    let temp = storage.write_temp(person_id, &storage_key, &prepared.bytes)?;
    if let Err(err) = storage.finalize(&temp, person_id, &storage_key, prepared.extension) {
        let _ = fs::remove_file(&temp);
        return Err(err);
    }
    if let Err(err) = tx.commit() {
        storage.remove_final(person_id, &storage_key, prepared.extension);
        return Err(MediaError::Database(err.to_string()));
    }

    Ok(MediaAsset {
        media_id,
        person_id,
        revision_id,
        kind,
        mime_type: prepared.mime_type.to_string(),
        byte_size: prepared.bytes.len() as i64,
        sha256,
        file_name: prepared.file_name,
        visibility,
        width: prepared.width,
        height: prepared.height,
        status: RevisionStatus::PendingReview,
        created_at,
        storage_key,
    })
}

/// Reads one media item for an authenticated requester.
///
/// The owner sees their own pending, rejected, or approved media. Any other
/// account sees only an approved revision whose visibility is `family`.
pub fn read_authorized_media(
    client: &mut postgres::Client,
    storage: &MediaStorage,
    requester: &IdentityAccount,
    media_id: i64,
) -> Result<MediaDownload, MediaError> {
    let sql = format!(
        "SELECT {MEDIA_COLUMNS} \
           FROM onboarding_media_metadata m \
           JOIN onboarding_profile_revision r \
             ON r.revision_id = m.revision_id AND r.visibility = m.visibility \
          WHERE m.media_id = $1"
    );
    let row = client
        .query_opt(&sql, &[&media_id])?
        .ok_or(MediaError::NotFound)?;
    let asset = row_to_asset(row)?;
    let requester_person: i64 = client
        .query_opt(
            "SELECT person_id FROM onboarding_person_account_link \
              WHERE account_id = $1 AND identity_fingerprint = $2",
            &[&requester.account_id, &requester.identity_fingerprint],
        )?
        .map(|row| row.get(0))
        .ok_or(MediaError::Unlinked)?;
    let owner = requester_person == asset.person_id;
    let current_approved: bool = client
        .query_one(
            "SELECT EXISTS (\
                SELECT 1 FROM onboarding_profile_revision r2 \
                 WHERE r2.person_id = $1 AND r2.field_key = $2 \
                   AND r2.revision_id = $3 AND r2.status = 'approved' \
                   AND EXISTS (\
                       SELECT 1 FROM onboarding_profile_value v \
                        WHERE v.person_id = r2.person_id \
                          AND v.field_key = r2.field_key \
                          AND v.approved_revision_id = r2.revision_id\
                   ) \
                   AND r2.revision_number = (\
                       SELECT max(r3.revision_number) \
                         FROM onboarding_profile_revision r3 \
                        WHERE r3.person_id = $1 AND r3.field_key = $2 \
                          AND r3.status = 'approved'\
                   )\
             )",
            &[
                &asset.person_id,
                &asset.kind.field_key(),
                &asset.revision_id,
            ],
        )?
        .get(0);
    let family_visible = !owner
        && asset.status == RevisionStatus::Approved
        && asset.visibility == ProfileVisibility::Family
        && current_approved;
    if !owner && !family_visible {
        return Err(MediaError::NotFound);
    }
    let bytes = storage.read(asset.person_id, &asset.storage_key, extension_for(&asset))?;
    Ok(MediaDownload { asset, bytes })
}

/// Withdraws an owner's pending media submission. The metadata row, pending
/// revision, and private file are removed; approved/rejected history cannot be
/// withdrawn this way.
pub fn withdraw_owned_media(
    client: &mut postgres::Client,
    storage: &MediaStorage,
    account: &IdentityAccount,
    media_id: i64,
) -> Result<(), MediaError> {
    let mut tx = client.transaction()?;
    let row = tx
        .query_opt(
            "SELECT m.person_id, m.storage_key, m.mime_type, m.revision_id, \
                    r.status, r.changed_by \
               FROM onboarding_media_metadata m \
               JOIN onboarding_profile_revision r ON r.revision_id = m.revision_id \
              WHERE m.media_id = $1 \
              FOR UPDATE OF r",
            &[&media_id],
        )?
        .ok_or(MediaError::NotFound)?;
    let person_id: i64 = row.get(0);
    let storage_key: String = row.get(1);
    let mime_type: String = row.get(2);
    let revision_id: i64 = row.get(3);
    let status: String = row.get(4);
    let changed_by: String = row.get(5);
    let owner_person = owner_person_in_tx(&mut tx, account)?;
    if owner_person != person_id || changed_by != account.identity_fingerprint {
        return Err(MediaError::NotFound);
    }
    if status != "pending_review" {
        return Err(MediaError::InvalidTransition);
    }
    tx.execute(
        "DELETE FROM onboarding_media_metadata WHERE media_id = $1",
        &[&media_id],
    )?;
    tx.execute(
        "DELETE FROM onboarding_profile_revision \
          WHERE revision_id = $1 AND status = 'pending_review'",
        &[&revision_id],
    )?;
    tx.commit()?;
    storage.remove_final(person_id, &storage_key, extension_for_mime(&mime_type));
    Ok(())
}

/// Lists the media visible to the owner: their own submissions plus the current
/// approved photo/document for the person.
pub fn read_owner_media(
    client: &mut postgres::Client,
    account: &IdentityAccount,
) -> Result<Vec<MediaAsset>, MediaError> {
    let person_id: i64 = client
        .query_opt(
            "SELECT person_id FROM onboarding_person_account_link \
              WHERE account_id = $1 AND identity_fingerprint = $2",
            &[&account.account_id, &account.identity_fingerprint],
        )?
        .map(|row| row.get(0))
        .ok_or(MediaError::Unlinked)?;
    let sql = format!(
        "SELECT {MEDIA_COLUMNS} \
           FROM onboarding_media_metadata m \
           JOIN onboarding_profile_revision r \
             ON r.revision_id = m.revision_id AND r.visibility = m.visibility \
          WHERE m.person_id = $1 \
            AND (r.changed_by = $2 OR EXISTS (\
                SELECT 1 FROM onboarding_profile_value v \
                 WHERE v.person_id = m.person_id \
                   AND v.field_key = m.media_kind \
                   AND v.approved_revision_id = m.revision_id\
            )) \
          ORDER BY m.created_at DESC, m.media_id DESC"
    );
    let rows = client.query(&sql, &[&person_id, &account.identity_fingerprint])?;
    rows.into_iter().map(row_to_asset).collect()
}
