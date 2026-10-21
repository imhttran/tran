//! Family document lifecycle: versions, private originals, review, processing
//! state, legacy quarantine, and safe audit.
//!
//! The module owns the contracts persisted by migration
//! `009_document_lifecycle.sql`. It never writes chunk rows directly; processing
//! uses the existing ingestion pipeline and then links the version. The approved
//! retrieval view is owned by the migration and excludes every version that is
//! not explicitly approved, bound to its fingerprint and scope, processed, and
//! unremoved.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use postgres::{Client, GenericClient};
use serde::Serialize;
use thiserror::Error;

/// Maximum original document size, matching the existing upload contract.
pub const MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;

/// Supported document extensions.
pub const DOCUMENT_EXTENSIONS: &[&str] = &["md", "markdown", "txt", "pdf"];

/// How a version came to exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Uploaded through the browser/API; never auto-approved.
    Submission,
    /// Written by the trusted local operator CLI.
    OperatorImport,
    /// Quarantined from pre-existing `documents` rows.
    LegacyImport,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Submission => "submission",
            Origin::OperatorImport => "operator_import",
            Origin::LegacyImport => "legacy_import",
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who may read a version. Legacy versions have no invented scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessScope {
    /// Family retrieval is allowed after approval and processing.
    Family,
    /// Review-only; never enters family retrieval.
    AdminOnly,
}

impl AccessScope {
    pub fn as_str(self) -> &'static str {
        match self {
            AccessScope::Family => "family",
            AccessScope::AdminOnly => "admin_only",
        }
    }
}

impl fmt::Display for AccessScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AccessScope {
    type Err = LifecycleError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "family" => Ok(AccessScope::Family),
            "admin_only" => Ok(AccessScope::AdminOnly),
            other => Err(LifecycleError::Invalid("access scope", other.to_string())),
        }
    }
}

/// Review state. Approval is always bound to a content fingerprint and scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewStatus {
    PendingReview,
    Approved,
    Rejected,
}

impl ReviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewStatus::PendingReview => "pending_review",
            ReviewStatus::Approved => "approved",
            ReviewStatus::Rejected => "rejected",
        }
    }

    pub fn is_approved(self) -> bool {
        self == ReviewStatus::Approved
    }
}

impl fmt::Display for ReviewStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Processing state. `ready` requires approval and a committed chunk set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingStatus {
    LegacyIndexed,
    Unprocessed,
    Queued,
    Processing,
    Ready,
    Failed,
}

impl ProcessingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ProcessingStatus::LegacyIndexed => "legacy_indexed",
            ProcessingStatus::Unprocessed => "unprocessed",
            ProcessingStatus::Queued => "queued",
            ProcessingStatus::Processing => "processing",
            ProcessingStatus::Ready => "ready",
            ProcessingStatus::Failed => "failed",
        }
    }

    pub fn is_ready(self) -> bool {
        self == ProcessingStatus::Ready
    }
}

impl fmt::Display for ProcessingStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A supported document category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Genealogy,
    Photograph,
    Letter,
    Certificate,
    Story,
    Other,
    Legacy,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Genealogy => "genealogy",
            Category::Photograph => "photograph",
            Category::Letter => "letter",
            Category::Certificate => "certificate",
            Category::Story => "story",
            Category::Other => "other",
            Category::Legacy => "legacy",
        }
    }
}

impl FromStr for Category {
    type Err = LifecycleError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "genealogy" => Ok(Category::Genealogy),
            "photograph" => Ok(Category::Photograph),
            "letter" => Ok(Category::Letter),
            "certificate" => Ok(Category::Certificate),
            "story" => Ok(Category::Story),
            "other" => Ok(Category::Other),
            "legacy" => Ok(Category::Legacy),
            other => Err(LifecycleError::Invalid("category", other.to_string())),
        }
    }
}

/// One document version as exposed to the application. `storage_key` is
/// deliberately absent so a metadata response can never leak a private path.
#[derive(Debug, Clone, Serialize)]
pub struct DocumentVersion {
    pub version_id: i64,
    pub document_key: String,
    pub version_number: i32,
    pub corpus: String,
    pub source: String,
    pub title: String,
    pub description: Option<String>,
    pub category: String,
    pub origin: String,
    pub access_scope: Option<String>,
    pub review_status: String,
    pub processing_status: String,
    pub content_sha256: String,
    pub byte_size: i64,
    pub mime_type: String,
    pub original_filename: String,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<String>,
    pub review_note: Option<String>,
    pub removed_at: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

impl DocumentVersion {
    pub fn is_approved(&self) -> bool {
        self.review_status == "approved"
    }

    pub fn is_ready(&self) -> bool {
        self.processing_status == "ready"
    }

    pub fn is_removed(&self) -> bool {
        self.removed_at.is_some()
    }

    pub fn is_retrievable(&self) -> bool {
        self.review_status == "approved"
            && self.processing_status == "ready"
            && !self.is_removed()
            && self.access_scope.as_deref() == Some("family")
    }
}

/// A safe lifecycle error. No variant carries file bytes, a storage path, or a
/// private document excerpt.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LifecycleError {
    #[error("invalid {0}: {1}")]
    Invalid(&'static str, String),
    #[error("unknown document version")]
    NotFound,
    #[error("the document version is not in a state that permits this action")]
    InvalidTransition,
    #[error("the document version is not approved for processing")]
    NotApproved,
    #[error("the document original is missing")]
    MissingFile,
    #[error("document storage failed")]
    Storage,
    #[error("document database operation failed: {0}")]
    Database(String),
}

impl From<postgres::Error> for LifecycleError {
    fn from(err: postgres::Error) -> Self {
        LifecycleError::Database(err.to_string())
    }
}

/// Private content-addressed storage rooted at `DOCUMENT_ROOT`.
#[derive(Debug, Clone)]
pub struct DocumentStorage {
    root: PathBuf,
}

impl DocumentStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn safe_relative(&self, storage_key: &str) -> Result<PathBuf, LifecycleError> {
        let candidate = Path::new(storage_key);
        if candidate.is_absolute()
            || candidate
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(LifecycleError::Invalid(
                "storage key",
                storage_key.to_string(),
            ));
        }
        Ok(self.root.join(candidate))
    }

    /// Writes the original if absent. The path is content-addressed and never
    /// served by a static file handler.
    pub fn put(
        &self,
        sha256: &str,
        extension: &str,
        bytes: &[u8],
    ) -> Result<String, LifecycleError> {
        if !is_hex_sha256(sha256) {
            return Err(LifecycleError::Invalid("sha256", sha256.to_string()));
        }
        if !DOCUMENT_EXTENSIONS.contains(&extension) {
            return Err(LifecycleError::Invalid("extension", extension.to_string()));
        }
        let storage_key = format!("{}/{}.{}", &sha256[..2], sha256, extension);
        let path = self.safe_relative(&storage_key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| LifecycleError::Storage)?;
        }
        if !path.exists() {
            let temp = path.with_extension(format!("{extension}.tmp"));
            fs::write(&temp, bytes).map_err(|_| LifecycleError::Storage)?;
            fs::rename(&temp, &path).map_err(|_| LifecycleError::Storage)?;
        }
        Ok(storage_key)
    }

    /// Reads one stored original. Callers must authorize the version first.
    pub fn get(&self, storage_key: &str) -> Result<Vec<u8>, LifecycleError> {
        let path = self.safe_relative(storage_key)?;
        fs::read(path).map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                LifecycleError::MissingFile
            } else {
                LifecycleError::Storage
            }
        })
    }
}

/// Computes the lowercase hex SHA-256 of the original bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_hex_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn extension_for(filename: &str) -> Result<&'static str, LifecycleError> {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .ok_or_else(|| LifecycleError::Invalid("filename", filename.to_string()))?;
    match ext.as_str() {
        "md" => Ok("md"),
        "markdown" => Ok("markdown"),
        "txt" => Ok("txt"),
        "pdf" => Ok("pdf"),
        _ => Err(LifecycleError::Invalid("extension", ext)),
    }
}

fn mime_for(extension: &str) -> &'static str {
    match extension {
        "pdf" => "application/pdf",
        "md" | "markdown" => "text/markdown",
        _ => "text/plain",
    }
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "document".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

/// Builds a stable document key for a new submission. A short content suffix
/// prevents two different uploads with the same title from sharing a key.
fn new_document_key(title: &str, sha256: &str) -> String {
    format!("{}-{}", slug(title), &sha256[..8])
}

/// Input for a new browser/API submission.
pub struct NewSubmission<'a> {
    pub actor: &'a str,
    pub corpus: &'a str,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub category: Category,
    pub access_scope: AccessScope,
    pub owner_subject: Option<&'a str>,
    pub original_filename: &'a str,
    pub bytes: &'a [u8],
    /// Optional existing document key, used to create a new version.
    pub document_key: Option<&'a str>,
}

fn audit<C: GenericClient>(
    client: &mut C,
    version_id: Option<i64>,
    job_id: Option<i64>,
    action: &str,
    actor: &str,
    detail: serde_json::Value,
) -> Result<(), LifecycleError> {
    let detail = detail.to_string();
    client.execute(
        "INSERT INTO document_audit_event (version_id, job_id, action, actor, detail)
         VALUES ($1, $2, $3, $4, $5::text::jsonb)",
        &[&version_id, &job_id, &action, &actor, &detail],
    )?;
    Ok(())
}

/// Creates a pending submission and stores the private original. No chunk or
/// embedding is written.
pub fn create_submission(
    client: &mut Client,
    storage: &DocumentStorage,
    input: NewSubmission<'_>,
) -> Result<DocumentVersion, LifecycleError> {
    if input.bytes.is_empty() {
        return Err(LifecycleError::Invalid("file", "empty".to_string()));
    }
    if input.bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(LifecycleError::Invalid("file", "too large".to_string()));
    }
    let extension = extension_for(input.original_filename)?;
    let title = input.title.trim();
    if title.is_empty() {
        return Err(LifecycleError::Invalid("title", "empty".to_string()));
    }
    let sha256 = sha256_hex(input.bytes);
    let storage_key = storage.put(&sha256, extension, input.bytes)?;
    let document_key = input
        .document_key
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| new_document_key(title, &sha256));

    let mut tx = client.transaction()?;
    let version_number: i32 = tx
        .query_one(
            "SELECT coalesce(max(version_number), 0) + 1
               FROM document_version
              WHERE corpus = $1 AND document_key = $2",
            &[&input.corpus, &document_key],
        )?
        .get(0);
    // The chunk source keeps the original filename for citations while the
    // short document-key suffix keeps two same-named uploads distinct. A
    // version-specific suffix prevents a new version from replacing an older
    // version's chunks before it is approved and ready.
    let short_key: String = document_key
        .chars()
        .rev()
        .take(8)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let source = if version_number <= 1 {
        format!("{} ({short_key})", input.original_filename)
    } else {
        format!(
            "{} ({short_key}) (v{version_number})",
            input.original_filename
        )
    };
    let version_id: i64 = tx
        .query_one(
            "INSERT INTO document_version
                 (document_key, version_number, corpus, source, title, description,
                  category, origin, access_scope, owner_subject, content_sha256,
                  byte_size, mime_type, original_filename, storage_key,
                  review_status, processing_status, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'submission', $8, $9, $10,
                     $11, $12, $13, $14, 'pending_review', 'unprocessed', $15)
             RETURNING version_id",
            &[
                &document_key,
                &version_number,
                &input.corpus,
                &source,
                &title,
                &input.description,
                &input.category.as_str(),
                &input.access_scope.as_str(),
                &input.owner_subject,
                &sha256,
                &(input.bytes.len() as i64),
                &mime_for(extension),
                &input.original_filename,
                &storage_key,
                &input.actor,
            ],
        )?
        .get(0);
    audit(
        &mut tx,
        Some(version_id),
        None,
        "submitted",
        input.actor,
        serde_json::json!({
            "document_key": document_key,
            "version_number": version_number,
            "content_sha256": sha256,
        }),
    )?;
    let version = fetch_version(&mut tx, version_id)?;
    tx.commit()?;
    Ok(version)
}

/// Records a trusted local operator import as an approved, ready version and
/// links the chunks already written by the CLI ingest. This is the separate
/// trusted operator boundary; browser/API uploads never call it.
#[allow(clippy::too_many_arguments)]
pub fn record_operator_import(
    client: &mut Client,
    storage: &DocumentStorage,
    actor: &str,
    corpus: &str,
    source: &str,
    title: &str,
    original_filename: &str,
    bytes: &[u8],
) -> Result<DocumentVersion, LifecycleError> {
    let extension = extension_for(original_filename)?;
    let sha256 = sha256_hex(bytes);
    let storage_key = storage.put(&sha256, extension, bytes)?;
    let document_key = slug(source);
    let mut tx = client.transaction()?;
    // Re-running the same trusted import with unchanged content is a no-op.
    let existing: Option<i64> = tx
        .query_opt(
            "SELECT version_id FROM document_version
              WHERE corpus = $1 AND source = $2 AND content_sha256 = $3
                AND origin = 'operator_import' AND review_status = 'approved'
                AND processing_status = 'ready'
              ORDER BY version_id DESC LIMIT 1",
            &[&corpus, &source, &sha256],
        )?
        .map(|row| row.get(0));
    if let Some(version_id) = existing {
        tx.execute(
            "UPDATE documents SET document_version_id = $1
              WHERE corpus = $2 AND source = $3",
            &[&version_id, &corpus, &source],
        )?;
        let version = fetch_version(&mut tx, version_id)?;
        tx.commit()?;
        return Ok(version);
    }
    let version_number: i32 = tx
        .query_one(
            "SELECT coalesce(max(version_number), 0) + 1
               FROM document_version
              WHERE corpus = $1 AND document_key = $2",
            &[&corpus, &document_key],
        )?
        .get(0);
    let version_id: i64 = tx
        .query_one(
            "INSERT INTO document_version
                 (document_key, version_number, corpus, source, title, category, origin,
                  access_scope, content_sha256, byte_size, mime_type, original_filename,
                  storage_key, review_status, processing_status, reviewed_by, reviewed_at,
                  approved_sha256, approved_scope, approved_version_number, created_by)
             VALUES ($1, $2, $3, $4, $5, 'other', 'operator_import', 'family', $6, $7,
                     $8, $9, $10, 'approved', 'ready', $11, now(), $6, 'family', $2, $11)
             RETURNING version_id",
            &[
                &document_key,
                &version_number,
                &corpus,
                &source,
                &title,
                &sha256,
                &(bytes.len() as i64),
                &mime_for(extension),
                &original_filename,
                &storage_key,
                &actor,
            ],
        )?
        .get(0);
    tx.execute(
        "UPDATE documents SET document_version_id = $1
          WHERE corpus = $2 AND source = $3",
        &[&version_id, &corpus, &source],
    )?;
    audit(
        &mut tx,
        Some(version_id),
        None,
        "operator_import",
        actor,
        serde_json::json!({
            "document_key": document_key,
            "version_number": version_number,
            "content_sha256": sha256,
        }),
    )?;
    let version = fetch_version(&mut tx, version_id)?;
    tx.commit()?;
    Ok(version)
}

fn version_columns() -> &'static str {
    "version_id, document_key, version_number, corpus, source, title, description, \
     category, origin, access_scope, review_status, processing_status, content_sha256, \
     byte_size, mime_type, original_filename, reviewed_by, reviewed_at::text, \
     review_note, removed_at::text, created_by, created_at::text, updated_at::text"
}

fn row_to_version(row: postgres::Row) -> DocumentVersion {
    DocumentVersion {
        version_id: row.get(0),
        document_key: row.get(1),
        version_number: row.get(2),
        corpus: row.get(3),
        source: row.get(4),
        title: row.get(5),
        description: row.get(6),
        category: row.get(7),
        origin: row.get(8),
        access_scope: row.get(9),
        review_status: row.get(10),
        processing_status: row.get(11),
        content_sha256: row.get(12),
        byte_size: row.get(13),
        mime_type: row.get(14),
        original_filename: row.get(15),
        reviewed_by: row.get(16),
        reviewed_at: row.get(17),
        review_note: row.get(18),
        removed_at: row.get(19),
        created_by: row.get(20),
        created_at: row.get(21),
        updated_at: row.get(22),
    }
}

fn fetch_version<C: GenericClient>(
    client: &mut C,
    version_id: i64,
) -> Result<DocumentVersion, LifecycleError> {
    let sql = format!(
        "SELECT {} FROM document_version WHERE version_id = $1",
        version_columns()
    );
    client
        .query_opt(&sql, &[&version_id])?
        .map(row_to_version)
        .ok_or(LifecycleError::NotFound)
}

pub fn get_version(
    client: &mut Client,
    version_id: i64,
) -> Result<DocumentVersion, LifecycleError> {
    fetch_version(client, version_id)
}

/// Lists submissions. An admin sees every version; an editor sees only versions
/// they created.
pub fn list_submissions(
    client: &mut Client,
    actor: &str,
    is_admin: bool,
) -> Result<Vec<DocumentVersion>, LifecycleError> {
    let sql = format!(
        "SELECT {} FROM document_version
          WHERE ($1 OR created_by = $2)
          ORDER BY created_at DESC, version_id DESC",
        version_columns()
    );
    let rows = client.query(&sql, &[&is_admin, &actor])?;
    Ok(rows.into_iter().map(row_to_version).collect())
}

/// Reads the private original. Callers enforce ownership/admin authorization.
pub fn read_original(
    client: &mut Client,
    storage: &DocumentStorage,
    version_id: i64,
) -> Result<(DocumentVersion, Vec<u8>), LifecycleError> {
    let version = fetch_version(client, version_id)?;
    if version.is_removed() {
        return Err(LifecycleError::NotFound);
    }
    let storage_key: Option<String> = client
        .query_one(
            "SELECT storage_key FROM document_version WHERE version_id = $1",
            &[&version_id],
        )?
        .get(0);
    let key = storage_key.ok_or(LifecycleError::MissingFile)?;
    let bytes = storage.get(&key)?;
    Ok((version, bytes))
}

/// Records an explicit review decision. Approval binds the exact fingerprint,
/// version, and scope; rejection clears any prior approval binding.
pub fn review(
    client: &mut Client,
    version_id: i64,
    actor: &str,
    approve: bool,
    note: Option<&str>,
    scope: Option<AccessScope>,
) -> Result<DocumentVersion, LifecycleError> {
    let mut tx = client.transaction()?;
    let row = tx
        .query_opt(
            "SELECT review_status, content_sha256, version_number, access_scope, (removed_at IS NOT NULL)
               FROM document_version WHERE version_id = $1 FOR UPDATE",
            &[&version_id],
        )?
        .ok_or(LifecycleError::NotFound)?;
    let current: String = row.get(0);
    if current == "rejected" && approve {
        return Err(LifecycleError::InvalidTransition);
    }
    if row.get::<_, bool>(4) {
        return Err(LifecycleError::InvalidTransition);
    }
    let actor_owned = actor.to_string();
    let note_owned = note.unwrap_or("").to_string();
    if approve {
        let effective_scope = scope
            .map(|value| value.as_str().to_string())
            .or_else(|| row.get::<_, Option<String>>(3))
            .ok_or_else(|| LifecycleError::Invalid("scope", "required for approval".to_string()))?;
        tx.execute(
            "UPDATE document_version
                SET review_status = 'approved', reviewed_by = $2, reviewed_at = now(),
                    review_note = $3, access_scope = $4, approved_sha256 = content_sha256,
                    approved_scope = $4, approved_version_number = version_number,
                    updated_at = now()
              WHERE version_id = $1",
            &[
                &version_id,
                &actor_owned,
                &note_owned,
                &effective_scope.as_str(),
            ],
        )?;
        audit(
            &mut tx,
            Some(version_id),
            None,
            "review_approved",
            actor,
            serde_json::json!({ "scope": effective_scope }),
        )?;
    } else {
        tx.execute(
            "UPDATE document_version
                SET review_status = 'rejected', reviewed_by = $2, reviewed_at = now(),
                    review_note = $3, approved_sha256 = NULL, approved_scope = NULL,
                    approved_version_number = NULL,
                    processing_status = CASE WHEN processing_status = 'ready'
                                             THEN 'unprocessed' ELSE processing_status END,
                    updated_at = now()
              WHERE version_id = $1",
            &[&version_id, &actor_owned, &note_owned],
        )?;
        audit(
            &mut tx,
            Some(version_id),
            None,
            "review_rejected",
            actor,
            serde_json::json!({ "note": note }),
        )?;
    }
    let version = fetch_version(&mut tx, version_id)?;
    tx.commit()?;
    Ok(version)
}

/// Changes the access scope. A scope change on an approved version invalidates
/// that approval and requires a new review.
pub fn change_scope(
    client: &mut Client,
    version_id: i64,
    actor: &str,
    scope: AccessScope,
) -> Result<DocumentVersion, LifecycleError> {
    let mut tx = client.transaction()?;
    let row = tx
        .query_opt(
            "SELECT access_scope, review_status FROM document_version
              WHERE version_id = $1 FOR UPDATE",
            &[&version_id],
        )?
        .ok_or(LifecycleError::NotFound)?;
    let old_scope: Option<String> = row.get(0);
    let old_review: String = row.get(1);
    let changed = old_scope.as_deref() != Some(scope.as_str());
    if changed && old_review == "approved" {
        tx.execute(
            "UPDATE document_version
                SET access_scope = $2, review_status = 'pending_review',
                    approved_sha256 = NULL, approved_scope = NULL,
                    approved_version_number = NULL, reviewed_by = NULL,
                    reviewed_at = NULL,
                    processing_status = CASE WHEN processing_status = 'ready'
                                             THEN 'unprocessed' ELSE processing_status END,
                    updated_at = now()
              WHERE version_id = $1",
            &[&version_id, &scope.as_str()],
        )?;
    } else {
        tx.execute(
            "UPDATE document_version SET access_scope = $2, updated_at = now()
              WHERE version_id = $1",
            &[&version_id, &scope.as_str()],
        )?;
    }
    audit(
        &mut tx,
        Some(version_id),
        None,
        "scope_changed",
        actor,
        serde_json::json!({ "scope": scope.as_str(), "approval_invalidated": changed && old_review == "approved" }),
    )?;
    let version = fetch_version(&mut tx, version_id)?;
    tx.commit()?;
    Ok(version)
}

/// Removes a version from effective access. Chunks and originals are retained.
pub fn remove(
    client: &mut Client,
    version_id: i64,
    actor: &str,
) -> Result<DocumentVersion, LifecycleError> {
    let mut tx = client.transaction()?;
    tx.execute(
        "UPDATE document_version
            SET removed_at = now(), removed_by = $2, updated_at = now()
          WHERE version_id = $1 AND removed_at IS NULL",
        &[&version_id, &actor],
    )?;
    tx.execute(
        "UPDATE document_processing_job
            SET status = 'failed', last_error = 'version removed', updated_at = now(),
                finished_at = now()
          WHERE version_id = $1 AND status IN ('queued', 'processing')",
        &[&version_id],
    )?;
    audit(
        &mut tx,
        Some(version_id),
        None,
        "removed",
        actor,
        serde_json::json!({}),
    )?;
    let version = fetch_version(&mut tx, version_id)?;
    tx.commit()?;
    Ok(version)
}

/// Queues one approved version for processing. The job key makes repeated
/// requests idempotent.
pub fn queue_processing(
    client: &mut Client,
    version_id: i64,
    actor: &str,
) -> Result<i64, LifecycleError> {
    let mut tx = client.transaction()?;
    let row = tx
        .query_opt(
            "SELECT review_status, processing_status, (removed_at IS NOT NULL), content_sha256,
                    approved_sha256, approved_scope, access_scope, approved_version_number,
                    version_number, origin
               FROM document_version WHERE version_id = $1 FOR UPDATE",
            &[&version_id],
        )?
        .ok_or(LifecycleError::NotFound)?;
    if row.get::<_, bool>(2)
        || row.get::<_, String>(0) != "approved"
        || row.get::<_, Option<String>>(4).as_deref() != Some(row.get::<_, String>(3).as_str())
        || row.get::<_, Option<String>>(5) != row.get::<_, Option<String>>(6)
        || row.get::<_, Option<i32>>(7) != Some(row.get::<_, i32>(8))
    {
        return Err(LifecycleError::NotApproved);
    }
    let status: String = row.get(1);
    let job_key = format!("{version_id}:{}", row.get::<_, String>(3));
    // Repeated queue requests are idempotent: return the existing job for a
    // queued, processing, or ready version.
    if matches!(status.as_str(), "queued" | "processing" | "ready") {
        let job_id: i64 = tx
            .query_one(
                "SELECT job_id FROM document_processing_job WHERE job_key = $1",
                &[&job_key],
            )?
            .get(0);
        tx.commit()?;
        return Ok(job_id);
    }
    if !matches!(status.as_str(), "unprocessed" | "failed" | "legacy_indexed") {
        return Err(LifecycleError::InvalidTransition);
    }
    let job_id: i64 = tx
        .query_one(
            "INSERT INTO document_processing_job (version_id, job_key, status, created_by)
             VALUES ($1, $2, 'queued', $3)
             ON CONFLICT (job_key) DO UPDATE
                SET status = 'queued', last_error = NULL, updated_at = now(),
                    finished_at = NULL
              WHERE document_processing_job.status = 'failed'
             RETURNING job_id",
            &[&version_id, &job_key, &actor],
        )?
        .get(0);
    tx.execute(
        "UPDATE document_version
            SET processing_status = 'queued', updated_at = now()
          WHERE version_id = $1",
        &[&version_id],
    )?;
    audit(
        &mut tx,
        Some(version_id),
        Some(job_id),
        "queued",
        actor,
        serde_json::json!({ "job_key": job_key }),
    )?;
    tx.commit()?;
    Ok(job_id)
}

/// Read-only legacy inventory grouped by corpus/source. It emits counts and
/// fingerprints only.
#[derive(Debug, Clone, Serialize)]
pub struct LegacyInventoryRow {
    pub corpus: String,
    pub source: String,
    pub version_id: i64,
    pub chunks: i64,
    pub review_status: String,
    pub processing_status: String,
    pub fingerprint: String,
}

pub fn legacy_inventory(client: &mut Client) -> Result<Vec<LegacyInventoryRow>, LifecycleError> {
    let rows = client.query(
        "SELECT v.corpus, v.source, v.version_id, count(d.id)::bigint,
                v.review_status, v.processing_status, v.content_sha256
           FROM document_version v
           LEFT JOIN documents d ON d.document_version_id = v.version_id
          WHERE v.origin = 'legacy_import'
          GROUP BY v.corpus, v.source, v.version_id, v.review_status,
                   v.processing_status, v.content_sha256
          ORDER BY v.corpus, v.source",
        &[],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| LegacyInventoryRow {
            corpus: row.get(0),
            source: row.get(1),
            version_id: row.get(2),
            chunks: row.get(3),
            review_status: row.get(4),
            processing_status: row.get(5),
            fingerprint: row.get(6),
        })
        .collect())
}

/// Promotes an approved legacy version by marking it ready. The preserved
/// chunks are already linked by the migration; they are never re-embedded.
pub fn promote_legacy(
    client: &mut Client,
    version_id: i64,
    actor: &str,
) -> Result<DocumentVersion, LifecycleError> {
    let mut tx = client.transaction()?;
    let row = tx
        .query_opt(
            "SELECT origin, review_status, approved_sha256, content_sha256,
                    approved_scope, access_scope, (removed_at IS NOT NULL)
               FROM document_version WHERE version_id = $1 FOR UPDATE",
            &[&version_id],
        )?
        .ok_or(LifecycleError::NotFound)?;
    if row.get::<_, String>(0) != "legacy_import"
        || row.get::<_, String>(1) != "approved"
        || row.get::<_, bool>(6)
        || row.get::<_, Option<String>>(2).as_deref() != Some(row.get::<_, String>(3).as_str())
        || row.get::<_, Option<String>>(4) != row.get::<_, Option<String>>(5)
    {
        return Err(LifecycleError::NotApproved);
    }
    tx.execute(
        "UPDATE document_version
            SET processing_status = 'ready', updated_at = now()
          WHERE version_id = $1",
        &[&version_id],
    )?;
    audit(
        &mut tx,
        Some(version_id),
        None,
        "processing_ready",
        actor,
        serde_json::json!({ "origin": "legacy_import" }),
    )?;
    let version = fetch_version(&mut tx, version_id)?;
    tx.commit()?;
    Ok(version)
}
