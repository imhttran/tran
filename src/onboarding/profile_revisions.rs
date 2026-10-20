//! Owner drafts and independently reviewed profile revisions (USR-004B phase 1).
//!
//! `onboarding_profile_value` is the last approved value. A revision is an
//! owner-private proposal: it starts as a draft, becomes pending review on
//! submission, and ends in `approved`, `rejected`, or `needs_changes`. Only an
//! approval of a pending revision updates the approved value, and the reviewer
//! must be a different actor from the submitter.
//!
//! The database adds a second line of defense: submitted content cannot change,
//! terminal revisions cannot change, and the revision trigger rejects a
//! self-review. This module never writes to the RAG database.

use thiserror::Error;

use crate::onboarding::account_map::IdentityAccount;

/// Profile fields the owner may propose in this slice.
pub const ALLOWED_PROFILE_FIELDS: [&str; 3] = ["display_name", "preferred_name", "biography"];

/// Maximum length for a display/preferred name.
pub const NAME_MAX_CHARS: usize = 200;
/// Maximum length for a biography.
pub const BIOGRAPHY_MAX_CHARS: usize = 5_000;
/// Maximum length for a reviewer note.
pub const REVIEW_NOTE_MAX_CHARS: usize = 2_000;

/// The lifecycle state of one profile revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionStatus {
    Draft,
    PendingReview,
    Approved,
    Rejected,
    NeedsChanges,
}

impl RevisionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RevisionStatus::Draft => "draft",
            RevisionStatus::PendingReview => "pending_review",
            RevisionStatus::Approved => "approved",
            RevisionStatus::Rejected => "rejected",
            RevisionStatus::NeedsChanges => "needs_changes",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(RevisionStatus::Draft),
            "pending_review" => Some(RevisionStatus::PendingReview),
            "approved" => Some(RevisionStatus::Approved),
            "rejected" => Some(RevisionStatus::Rejected),
            "needs_changes" => Some(RevisionStatus::NeedsChanges),
            _ => None,
        }
    }

    fn is_terminal(self) -> bool {
        matches!(
            self,
            RevisionStatus::Approved | RevisionStatus::Rejected | RevisionStatus::NeedsChanges
        )
    }
}

/// Who may see an approved or proposed value once it is approved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileVisibility {
    OwnerPrivate,
    Family,
}

impl ProfileVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            ProfileVisibility::OwnerPrivate => "owner_private",
            ProfileVisibility::Family => "family",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "owner_private" => Some(ProfileVisibility::OwnerPrivate),
            "family" => Some(ProfileVisibility::Family),
            _ => None,
        }
    }
}

/// A reviewer's terminal decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Approve,
    Reject,
    NeedsChanges,
}

impl ReviewDecision {
    fn status(self) -> RevisionStatus {
        match self {
            ReviewDecision::Approve => RevisionStatus::Approved,
            ReviewDecision::Reject => RevisionStatus::Rejected,
            ReviewDecision::NeedsChanges => RevisionStatus::NeedsChanges,
        }
    }
}

/// Who is reading approved values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileAudience {
    /// The owner sees every approved value, including owner-private ones.
    Owner,
    /// Family-visible responses may see only values marked `family`.
    Family,
}

impl ProfileAudience {
    fn as_str(self) -> &'static str {
        match self {
            ProfileAudience::Owner => "owner",
            ProfileAudience::Family => "family",
        }
    }
}

/// One profile revision, including immutable provenance and current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRevision {
    pub revision_id: i64,
    pub person_id: i64,
    pub field_key: String,
    pub revision_number: i64,
    pub prior_value_text: Option<String>,
    pub prior_approved_revision_number: i64,
    pub new_value_text: String,
    pub visibility: ProfileVisibility,
    pub status: RevisionStatus,
    pub changed_by: String,
    pub changed_at: String,
    pub submitted_at: Option<String>,
    pub reviewed_at: Option<String>,
    pub reviewed_by: Option<String>,
    pub review_note: Option<String>,
}

/// One current approved value. This is the only profile content that may be
/// exposed to family-visible or retrieval paths in a later slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedProfileValue {
    pub field_key: String,
    pub value_text: String,
    pub visibility: ProfileVisibility,
    pub updated_by: String,
    pub updated_at: String,
    pub approved_revision_number: i64,
    pub approved_revision_id: Option<i64>,
}

/// The owner-visible profile: approved values and owner-private open revisions
/// are returned separately so callers cannot confuse pending content with the
/// last approved value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerProfile {
    pub person_key: String,
    pub display_name: String,
    pub approved: Vec<ApprovedProfileValue>,
    pub pending: Vec<ProfileRevision>,
}

/// Errors raised by profile revision operations.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RevisionError {
    #[error("unknown person")]
    UnknownPerson,
    #[error("unknown profile revision")]
    UnknownRevision,
    #[error("an open draft or pending revision already exists for this field")]
    OpenRevisionExists,
    #[error("the authenticated account has no reviewed person link")]
    Unlinked,
    #[error("only the revision owner may modify or submit it")]
    NotOwner,
    #[error("a submitter cannot review their own revision")]
    SelfReview,
    #[error("invalid revision transition")]
    InvalidTransition,
    #[error("the approved value changed since this draft was created")]
    VersionConflict,
    #[error("a rejection or needs-changes decision requires a review note")]
    ReviewNoteRequired,
    #[error("invalid profile field")]
    InvalidField,
    #[error("invalid profile value")]
    InvalidValue,
    #[error("invalid profile visibility")]
    InvalidVisibility,
    #[error("profile revision database operation failed: {0}")]
    Database(String),
}

impl From<postgres::Error> for RevisionError {
    fn from(err: postgres::Error) -> Self {
        RevisionError::Database(err.to_string())
    }
}

const REVISION_COLUMNS: &str = "revision_id, person_id, field_key, revision_number, \
     prior_value_text, prior_approved_revision_number, new_value_text, visibility, status, \
     changed_by, changed_at::text, submitted_at::text, reviewed_at::text, reviewed_by, \
     review_note";

fn row_to_revision(row: postgres::Row) -> Result<ProfileRevision, RevisionError> {
    let visibility = ProfileVisibility::parse(&row.get::<_, String>(7))
        .ok_or_else(|| RevisionError::Database("unknown profile visibility".into()))?;
    let status = RevisionStatus::parse(&row.get::<_, String>(8))
        .ok_or_else(|| RevisionError::Database("unknown profile revision status".into()))?;
    Ok(ProfileRevision {
        revision_id: row.get(0),
        person_id: row.get(1),
        field_key: row.get(2),
        revision_number: row.get(3),
        prior_value_text: row.get(4),
        prior_approved_revision_number: row.get(5),
        new_value_text: row.get(6),
        visibility,
        status,
        changed_by: row.get(9),
        changed_at: row.get(10),
        submitted_at: row.get(11),
        reviewed_at: row.get(12),
        reviewed_by: row.get(13),
        review_note: row.get(14),
    })
}

fn clean_actor(actor: &str) -> Result<String, RevisionError> {
    let actor = actor.trim();
    if actor.is_empty() {
        return Err(RevisionError::InvalidValue);
    }
    Ok(actor.to_string())
}

fn field_limit(field_key: &str) -> Option<usize> {
    match field_key {
        "display_name" | "preferred_name" => Some(NAME_MAX_CHARS),
        "biography" => Some(BIOGRAPHY_MAX_CHARS),
        _ => None,
    }
}

/// Reports whether a field key is accepted by this slice.
pub fn is_allowed_profile_field(field_key: &str) -> bool {
    field_limit(field_key).is_some()
}

fn clean_field(field_key: &str) -> Result<String, RevisionError> {
    let field_key = field_key.trim();
    if !is_allowed_profile_field(field_key) {
        return Err(RevisionError::InvalidField);
    }
    Ok(field_key.to_string())
}

fn clean_value(field_key: &str, value_text: &str) -> Result<String, RevisionError> {
    let limit = field_limit(field_key).ok_or(RevisionError::InvalidField)?;
    let value_text = value_text.trim();
    if value_text.is_empty() || value_text.chars().count() > limit {
        return Err(RevisionError::InvalidValue);
    }
    Ok(value_text.to_string())
}

fn clean_note(note: Option<&str>) -> Result<Option<String>, RevisionError> {
    let Some(note) = note else {
        return Ok(None);
    };
    let note = note.trim();
    if note.chars().count() > REVIEW_NOTE_MAX_CHARS {
        return Err(RevisionError::InvalidValue);
    }
    Ok((!note.is_empty()).then(|| note.to_string()))
}

fn lock_field(
    tx: &mut postgres::Transaction<'_>,
    person_id: i64,
    field_key: &str,
) -> Result<(), RevisionError> {
    let key = format!("profile-revision:{person_id}:{field_key}");
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
        &[&key],
    )?;
    Ok(())
}

struct CurrentApprovedValue {
    text: Option<String>,
    revision_number: i64,
}

fn current_approved_value(
    tx: &mut postgres::Transaction<'_>,
    person_id: i64,
    field_key: &str,
) -> Result<CurrentApprovedValue, RevisionError> {
    let row = tx.query_opt(
        "SELECT value_text, approved_revision_number FROM onboarding_profile_value \
          WHERE person_id = $1 AND field_key = $2",
        &[&person_id, &field_key],
    )?;
    Ok(match row {
        Some(row) => CurrentApprovedValue {
            text: Some(row.get(0)),
            revision_number: row.get(1),
        },
        None => CurrentApprovedValue {
            text: None,
            revision_number: 0,
        },
    })
}

fn prior_matches(
    current: &CurrentApprovedValue,
    prior_text: Option<&str>,
    prior_revision_number: i64,
) -> bool {
    current.text.as_deref() == prior_text && current.revision_number == prior_revision_number
}

fn append_event(
    tx: &mut postgres::Transaction<'_>,
    revision: &ProfileRevision,
    event: RevisionStatus,
    actor: &str,
    note: Option<&str>,
) -> Result<(), RevisionError> {
    tx.execute(
        "INSERT INTO onboarding_profile_review_event \
             (revision_id, person_id, event, actor, note) \
         VALUES ($1, $2, $3, $4, $5)",
        &[
            &revision.revision_id,
            &revision.person_id,
            &event.as_str(),
            &actor,
            &note,
        ],
    )?;
    Ok(())
}

fn revision_by_id_in_tx(
    tx: &mut postgres::Transaction<'_>,
    revision_id: i64,
    for_update: bool,
) -> Result<ProfileRevision, RevisionError> {
    let suffix = if for_update { " FOR UPDATE" } else { "" };
    let sql = format!(
        "SELECT {REVISION_COLUMNS} FROM onboarding_profile_revision \
          WHERE revision_id = $1{suffix}"
    );
    tx.query_opt(&sql, &[&revision_id])?
        .map(row_to_revision)
        .transpose()?
        .ok_or(RevisionError::UnknownRevision)
}

/// Reads one revision without mutation.
pub fn revision_by_id(
    client: &mut postgres::Client,
    revision_id: i64,
) -> Result<ProfileRevision, RevisionError> {
    let sql = format!(
        "SELECT {REVISION_COLUMNS} FROM onboarding_profile_revision \
          WHERE revision_id = $1"
    );
    client
        .query_opt(&sql, &[&revision_id])?
        .map(row_to_revision)
        .transpose()?
        .ok_or(RevisionError::UnknownRevision)
}

fn owner_person_in_tx(
    tx: &mut postgres::Transaction<'_>,
    account: &IdentityAccount,
) -> Result<i64, RevisionError> {
    let row = tx.query_opt(
        "SELECT person_id FROM onboarding_person_account_link \
          WHERE account_id = $1 AND identity_fingerprint = $2 FOR SHARE",
        &[&account.account_id, &account.identity_fingerprint],
    )?;
    row.map(|row| row.get(0)).ok_or(RevisionError::Unlinked)
}

fn create_draft_in_tx(
    tx: &mut postgres::Transaction<'_>,
    person_id: i64,
    actor: &str,
    field_key: &str,
    value_text: &str,
    visibility: ProfileVisibility,
) -> Result<ProfileRevision, RevisionError> {
    let actor = clean_actor(actor)?;
    let field_key = clean_field(field_key)?;
    let value_text = clean_value(&field_key, value_text)?;

    lock_field(tx, person_id, &field_key)?;

    let person_exists: bool = tx
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM person WHERE person_id = $1)",
            &[&person_id],
        )?
        .get(0);
    if !person_exists {
        return Err(RevisionError::UnknownPerson);
    }

    let open: bool = tx
        .query_one(
            "SELECT EXISTS (\
                SELECT 1 FROM onboarding_profile_revision \
                 WHERE person_id = $1 AND field_key = $2 \
                   AND status IN ('draft', 'pending_review')\
             )",
            &[&person_id, &field_key],
        )?
        .get(0);
    if open {
        return Err(RevisionError::OpenRevisionExists);
    }

    let prior_value_text = current_approved_value(tx, person_id, &field_key)?;

    // The revision number is allocated inside the same statement as the insert,
    // while the per-field advisory lock is held. Concurrent creates for the same
    // field therefore serialize and cannot allocate the same version.
    let sql = format!(
        "INSERT INTO onboarding_profile_revision \
             (person_id, field_key, prior_value_text, prior_approved_revision_number, \
              new_value_text, changed_by, revision_number, status, visibility) \
         SELECT $1, $2, $3, $4, $5, $6, \
                COALESCE(max(revision_number), 0) + 1, 'draft', $7 \
           FROM onboarding_profile_revision \
          WHERE person_id = $1 AND field_key = $2 \
         RETURNING {REVISION_COLUMNS}"
    );
    let revision = tx
        .query_one(
            &sql,
            &[
                &person_id,
                &field_key,
                &prior_value_text.text,
                &prior_value_text.revision_number,
                &value_text,
                &actor,
                &visibility.as_str(),
            ],
        )
        .map_err(RevisionError::from)
        .and_then(row_to_revision)?;
    append_event(
        tx,
        &revision,
        RevisionStatus::Draft,
        &actor,
        Some("draft created"),
    )?;
    Ok(revision)
}

/// Creates an owner-private draft. The prior approved value is captured so a
/// later approval can detect a version conflict.
pub fn create_draft(
    client: &mut postgres::Client,
    person_id: i64,
    actor: &str,
    field_key: &str,
    value_text: &str,
    visibility: ProfileVisibility,
) -> Result<ProfileRevision, RevisionError> {
    let mut tx = client.transaction()?;
    let revision =
        create_draft_in_tx(&mut tx, person_id, actor, field_key, value_text, visibility)?;
    tx.commit()?;
    Ok(revision)
}

/// Creates a draft only for the person linked to `account`. The link is
/// rechecked and share-locked inside the same transaction as the write. The
/// actor is the issuer-scoped identity fingerprint, never a bare subject.
pub fn create_owned_draft(
    client: &mut postgres::Client,
    account: &IdentityAccount,
    field_key: &str,
    value_text: &str,
    visibility: ProfileVisibility,
) -> Result<ProfileRevision, RevisionError> {
    let mut tx = client.transaction()?;
    let person_id = owner_person_in_tx(&mut tx, account)?;
    let revision = create_draft_in_tx(
        &mut tx,
        person_id,
        &account.identity_fingerprint,
        field_key,
        value_text,
        visibility,
    )?;
    tx.commit()?;
    Ok(revision)
}

fn update_draft_in_tx(
    tx: &mut postgres::Transaction<'_>,
    revision_id: i64,
    actor: &str,
    value_text: &str,
    visibility: ProfileVisibility,
    required_person: Option<i64>,
) -> Result<ProfileRevision, RevisionError> {
    let actor = clean_actor(actor)?;
    let revision = revision_by_id_in_tx(tx, revision_id, true)?;
    if let Some(person_id) = required_person {
        if revision.person_id != person_id {
            return Err(RevisionError::NotOwner);
        }
    }
    if revision.changed_by != actor {
        return Err(RevisionError::NotOwner);
    }
    if revision.status != RevisionStatus::Draft {
        return Err(RevisionError::InvalidTransition);
    }
    let value_text = clean_value(&revision.field_key, value_text)?;
    lock_field(tx, revision.person_id, &revision.field_key)?;

    let sql = format!(
        "UPDATE onboarding_profile_revision \
            SET new_value_text = $2, visibility = $3, changed_at = now() \
          WHERE revision_id = $1 AND status = 'draft' \
          RETURNING {REVISION_COLUMNS}"
    );
    let revision = tx
        .query_opt(&sql, &[&revision_id, &value_text, &visibility.as_str()])?
        .map(row_to_revision)
        .transpose()?
        .ok_or(RevisionError::InvalidTransition)?;
    append_event(
        tx,
        &revision,
        RevisionStatus::Draft,
        &actor,
        Some("draft updated"),
    )?;
    Ok(revision)
}

/// Updates a draft's proposed value. Only the owner may edit, and only before
/// submission. The revision number and immutable provenance are preserved.
pub fn update_draft(
    client: &mut postgres::Client,
    revision_id: i64,
    actor: &str,
    value_text: &str,
    visibility: ProfileVisibility,
) -> Result<ProfileRevision, RevisionError> {
    let mut tx = client.transaction()?;
    let revision = update_draft_in_tx(&mut tx, revision_id, actor, value_text, visibility, None)?;
    tx.commit()?;
    Ok(revision)
}

/// Updates a draft only when it belongs to the person linked to `account`.
pub fn update_owned_draft(
    client: &mut postgres::Client,
    account: &IdentityAccount,
    revision_id: i64,
    value_text: &str,
    visibility: ProfileVisibility,
) -> Result<ProfileRevision, RevisionError> {
    let mut tx = client.transaction()?;
    let person_id = owner_person_in_tx(&mut tx, account)?;
    let revision = update_draft_in_tx(
        &mut tx,
        revision_id,
        &account.identity_fingerprint,
        value_text,
        visibility,
        Some(person_id),
    )?;
    tx.commit()?;
    Ok(revision)
}

fn submit_revision_in_tx(
    tx: &mut postgres::Transaction<'_>,
    revision_id: i64,
    actor: &str,
    required_person: Option<i64>,
) -> Result<ProfileRevision, RevisionError> {
    let actor = clean_actor(actor)?;
    let revision = revision_by_id_in_tx(tx, revision_id, true)?;
    if let Some(person_id) = required_person {
        if revision.person_id != person_id {
            return Err(RevisionError::NotOwner);
        }
    }
    if revision.changed_by != actor {
        return Err(RevisionError::NotOwner);
    }
    if revision.status != RevisionStatus::Draft {
        return Err(RevisionError::InvalidTransition);
    }
    clean_value(&revision.field_key, &revision.new_value_text)?;
    lock_field(tx, revision.person_id, &revision.field_key)?;
    let current = current_approved_value(tx, revision.person_id, &revision.field_key)?;
    if !prior_matches(
        &current,
        revision.prior_value_text.as_deref(),
        revision.prior_approved_revision_number,
    ) {
        return Err(RevisionError::VersionConflict);
    }

    let sql = format!(
        "UPDATE onboarding_profile_revision \
            SET status = 'pending_review', submitted_at = now() \
          WHERE revision_id = $1 AND status = 'draft' \
          RETURNING {REVISION_COLUMNS}"
    );
    let revision = tx
        .query_opt(&sql, &[&revision_id])?
        .map(row_to_revision)
        .transpose()?
        .ok_or(RevisionError::InvalidTransition)?;
    append_event(tx, &revision, RevisionStatus::PendingReview, &actor, None)?;
    Ok(revision)
}

/// Submits an owner draft for independent review.
pub fn submit_revision(
    client: &mut postgres::Client,
    revision_id: i64,
    actor: &str,
) -> Result<ProfileRevision, RevisionError> {
    let mut tx = client.transaction()?;
    let revision = submit_revision_in_tx(&mut tx, revision_id, actor, None)?;
    tx.commit()?;
    Ok(revision)
}

/// Submits a draft only when it belongs to the person linked to `account`.
pub fn submit_owned_revision(
    client: &mut postgres::Client,
    account: &IdentityAccount,
    revision_id: i64,
) -> Result<ProfileRevision, RevisionError> {
    let mut tx = client.transaction()?;
    let person_id = owner_person_in_tx(&mut tx, account)?;
    let revision = submit_revision_in_tx(
        &mut tx,
        revision_id,
        &account.identity_fingerprint,
        Some(person_id),
    )?;
    tx.commit()?;
    Ok(revision)
}

/// Applies an independent reviewer's decision. Approval atomically replaces the
/// current approved value; rejection and needs_changes leave it untouched.
pub fn review_revision(
    client: &mut postgres::Client,
    revision_id: i64,
    reviewer: &str,
    decision: ReviewDecision,
    note: Option<&str>,
) -> Result<ProfileRevision, RevisionError> {
    let reviewer = clean_actor(reviewer)?;
    let mut tx = client.transaction()?;
    let revision = revision_by_id_in_tx(&mut tx, revision_id, true)?;
    if revision.status != RevisionStatus::PendingReview {
        return Err(RevisionError::InvalidTransition);
    }
    if revision.changed_by == reviewer {
        return Err(RevisionError::SelfReview);
    }
    let reviewer_owns_person: bool = tx
        .query_one(
            "SELECT EXISTS (\
                SELECT 1 FROM onboarding_person_account_link \
                 WHERE person_id = $1 AND identity_fingerprint = $2\
             )",
            &[&revision.person_id, &reviewer],
        )?
        .get(0);
    if reviewer_owns_person {
        return Err(RevisionError::SelfReview);
    }
    let note = clean_note(note)?;
    if decision != ReviewDecision::Approve && note.is_none() {
        return Err(RevisionError::ReviewNoteRequired);
    }
    lock_field(&mut tx, revision.person_id, &revision.field_key)?;
    let current = current_approved_value(&mut tx, revision.person_id, &revision.field_key)?;
    if !prior_matches(
        &current,
        revision.prior_value_text.as_deref(),
        revision.prior_approved_revision_number,
    ) {
        return Err(RevisionError::VersionConflict);
    }

    let status = decision.status();

    // Move the revision to its terminal state first. The approved-value guard
    // requires the referenced revision to be approved, and the whole decision
    // still commits or rolls back as one transaction.
    let sql = format!(
        "UPDATE onboarding_profile_revision \
            SET status = $2, reviewed_at = now(), reviewed_by = $3, review_note = $4 \
          WHERE revision_id = $1 AND status = 'pending_review' \
          RETURNING {REVISION_COLUMNS}"
    );
    let revision = tx
        .query_opt(&sql, &[&revision_id, &status.as_str(), &reviewer, &note])?
        .map(row_to_revision)
        .transpose()?
        .ok_or(RevisionError::InvalidTransition)?;

    if decision == ReviewDecision::Approve {
        let value_text = clean_value(&revision.field_key, &revision.new_value_text)?;
        tx.execute(
            "INSERT INTO onboarding_profile_value \
                 (person_id, field_key, value_text, visibility, updated_by, \
                  approved_revision_number, approved_revision_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (person_id, field_key) DO UPDATE \
                SET value_text = EXCLUDED.value_text, \
                    visibility = EXCLUDED.visibility, \
                    updated_by = EXCLUDED.updated_by, \
                    approved_revision_number = EXCLUDED.approved_revision_number, \
                    approved_revision_id = EXCLUDED.approved_revision_id, \
                    updated_at = now()",
            &[
                &revision.person_id,
                &revision.field_key,
                &value_text,
                &revision.visibility.as_str(),
                &reviewer,
                &revision.revision_number,
                &revision.revision_id,
            ],
        )?;
    }

    append_event(&mut tx, &revision, status, &reviewer, note.as_deref())?;
    tx.commit()?;
    Ok(revision)
}

fn open_revisions_in_tx(
    tx: &mut postgres::Transaction<'_>,
    person_id: i64,
) -> Result<Vec<ProfileRevision>, RevisionError> {
    let sql = format!(
        "SELECT {REVISION_COLUMNS} FROM onboarding_profile_revision \
          WHERE person_id = $1 AND status IN ('draft', 'pending_review') \
          ORDER BY field_key, revision_number"
    );
    let rows = tx.query(&sql, &[&person_id])?;
    rows.into_iter()
        .map(row_to_revision)
        .collect::<Result<Vec<_>, _>>()
}

/// Lists the owner's open drafts and pending submissions.
pub fn list_open_revisions(
    client: &mut postgres::Client,
    person_id: i64,
) -> Result<Vec<ProfileRevision>, RevisionError> {
    let mut tx = client.transaction()?;
    let rows = open_revisions_in_tx(&mut tx, person_id)?;
    tx.commit()?;
    Ok(rows)
}

/// Lists all revisions for one person, newest first. This is owner-private
/// history: a family audience receives no revision history.
pub fn list_revision_history(
    client: &mut postgres::Client,
    person_id: i64,
    audience: ProfileAudience,
) -> Result<Vec<ProfileRevision>, RevisionError> {
    if audience == ProfileAudience::Family {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {REVISION_COLUMNS} FROM onboarding_profile_revision \
          WHERE person_id = $1 \
          ORDER BY changed_at DESC, revision_id DESC"
    );
    let rows = client.query(&sql, &[&person_id])?;
    rows.into_iter()
        .map(row_to_revision)
        .collect::<Result<Vec<_>, _>>()
}

fn approved_values_in_tx(
    tx: &mut postgres::Transaction<'_>,
    person_id: i64,
    audience: ProfileAudience,
) -> Result<Vec<ApprovedProfileValue>, RevisionError> {
    let rows = tx.query(
        "SELECT field_key, value_text, visibility, updated_by, updated_at::text, \
                approved_revision_number, approved_revision_id \
           FROM onboarding_profile_value \
          WHERE person_id = $1 AND ($2 = 'owner' OR visibility = 'family') \
          ORDER BY field_key",
        &[&person_id, &audience.as_str()],
    )?;
    rows.into_iter()
        .map(|row| {
            let visibility = ProfileVisibility::parse(&row.get::<_, String>(2))
                .ok_or_else(|| RevisionError::Database("unknown profile visibility".into()))?;
            Ok(ApprovedProfileValue {
                field_key: row.get(0),
                value_text: row.get(1),
                visibility,
                updated_by: row.get(3),
                updated_at: row.get(4),
                approved_revision_number: row.get(5),
                approved_revision_id: row.get(6),
            })
        })
        .collect()
}

/// Reads the current approved values for the requested audience. Family
/// responses never include owner-private values; the owner sees both.
pub fn read_approved_values(
    client: &mut postgres::Client,
    person_id: i64,
    audience: ProfileAudience,
) -> Result<Vec<ApprovedProfileValue>, RevisionError> {
    let mut tx = client.transaction()?;
    let rows = approved_values_in_tx(&mut tx, person_id, audience)?;
    tx.commit()?;
    Ok(rows)
}

/// Reads the authenticated owner's profile in one transaction. The reviewed
/// link row is share-locked before the data read, so a concurrent revoke or
/// replacement cannot land between the ownership check and the approved or
/// pending data. Approved values and owner-private pending revisions are
/// returned separately.
pub fn read_owner_profile(
    client: &mut postgres::Client,
    account: &IdentityAccount,
) -> Result<Option<OwnerProfile>, RevisionError> {
    let mut tx = client.transaction()?;
    let Some(row) = tx.query_opt(
        "SELECT person_id FROM onboarding_person_account_link \
          WHERE account_id = $1 AND identity_fingerprint = $2 FOR SHARE",
        &[&account.account_id, &account.identity_fingerprint],
    )?
    else {
        return Ok(None);
    };
    let person_id: i64 = row.get(0);
    let Some(person) = tx.query_opt(
        "SELECT person_key, display_name FROM person WHERE person_id = $1",
        &[&person_id],
    )?
    else {
        return Ok(None);
    };
    let approved = approved_values_in_tx(&mut tx, person_id, ProfileAudience::Owner)?;
    let pending = open_revisions_in_tx(&mut tx, person_id)?;
    tx.commit()?;
    Ok(Some(OwnerProfile {
        person_key: person.get(0),
        display_name: person.get(1),
        approved,
        pending,
    }))
}

/// Reports whether a revision is in a terminal state. Kept public so callers do
/// not have to duplicate the state machine.
pub fn is_terminal(status: RevisionStatus) -> bool {
    status.is_terminal()
}
