//! Read-only administrator review queue and detail (USR-004E).
//!
//! This module reads the existing revision, approved-value, review-event, and
//! media-metadata tables. It does not introduce another review lifecycle: the
//! decision itself still goes through
//! [`crate::onboarding::profile_revisions::review_revision`], which rechecks
//! status, ownership, independence, notes, and version conflicts inside the
//! decision transaction.
//!
//! Submitted content is admin-visible for review. Drafts are never returned, and
//! media metadata never includes a storage key or raw bytes.

use postgres::{Client, Transaction};

use crate::onboarding::profile_revisions::{
    row_to_revision, ApprovedProfileValue, ProfileRevision, ProfileVisibility, RevisionError,
    RevisionStatus, REVISION_COLUMNS,
};

/// Metadata for one submitted photo or personal document. It never carries a
/// storage path, hash, or content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewMedia {
    pub media_id: i64,
    pub kind: String,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub byte_size: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

/// One append-only review-lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewEvent {
    pub event: String,
    pub actor: String,
    pub note: Option<String>,
    pub created_at: String,
}

/// One queue row with the information needed to compare the proposal against
/// the last approved value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewEntry {
    pub revision: ProfileRevision,
    pub display_name: String,
    pub current_approved: Option<ApprovedProfileValue>,
    pub current_media: Option<ReviewMedia>,
    pub media: Option<ReviewMedia>,
    pub can_decide: bool,
    pub decision_block: Option<String>,
}

/// One bounded page of review entries, newest revision first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewQueue {
    pub entries: Vec<ReviewEntry>,
    pub next_cursor: Option<i64>,
    pub status: String,
    pub limit: i64,
}

/// A queue entry plus its immutable events and same-field revision history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewDetail {
    pub entry: ReviewEntry,
    pub events: Vec<ReviewEvent>,
    pub history: Vec<ProfileRevision>,
}

const REVISION_COLUMNS_QUALIFIED: &str = "r.revision_id, r.person_id, r.field_key, \
     r.revision_number, r.prior_value_text, r.prior_approved_revision_number, \
     r.new_value_text, r.visibility, r.status, r.changed_by, r.changed_at::text, \
     r.submitted_at::text, r.reviewed_at::text, r.reviewed_by, r.review_note";

/// Reads a bounded queue of submitted revisions.
///
/// `statuses` must not contain `draft`; drafts are owner-private and are never
/// exposed through the review console. `cursor` is the last revision id from the
/// previous page, so the ordering is stable even when timestamps collide.
pub fn read_review_queue(
    client: &mut Client,
    reviewer_fingerprint: &str,
    statuses: &[RevisionStatus],
    cursor: Option<i64>,
    limit: i64,
) -> Result<ReviewQueue, RevisionError> {
    if statuses.is_empty() || statuses.contains(&RevisionStatus::Draft) {
        return Err(RevisionError::InvalidValue);
    }
    let limit = limit.clamp(1, 100);
    let status_values: Vec<String> = statuses
        .iter()
        .map(|status| status.as_str().to_string())
        .collect();
    let sql = format!(
        "SELECT {REVISION_COLUMNS_QUALIFIED}, p.display_name, \
                v.value_text, v.visibility, v.updated_by, v.updated_at::text, \
                v.approved_revision_number, v.approved_revision_id, \
                m.media_id, m.media_kind, m.mime_type, m.file_name, m.byte_size, \
                m.width, m.height, \
                cm.media_id, cm.media_kind, cm.mime_type, cm.file_name, cm.byte_size, \
                cm.width, cm.height, \
                EXISTS (SELECT 1 FROM onboarding_person_account_link l \
                         WHERE l.person_id = r.person_id \
                           AND l.identity_fingerprint = $3) AS reviewer_owns_person, \
                (SELECT fr.generation \
                   FROM onboarding_profile_field_restriction fr \
                  WHERE fr.person_id = r.person_id AND fr.field_key = r.field_key \
                    AND fr.lifted_at IS NULL) AS current_restriction_generation \
           FROM onboarding_profile_revision r \
           JOIN person p ON p.person_id = r.person_id \
           LEFT JOIN onboarding_profile_value v \
             ON v.person_id = r.person_id AND v.field_key = r.field_key \
           LEFT JOIN onboarding_media_metadata m ON m.revision_id = r.revision_id \
           LEFT JOIN onboarding_media_metadata cm ON cm.revision_id = v.approved_revision_id \
          WHERE r.status = ANY($1) \
            AND ($2::bigint IS NULL OR r.revision_id < $2) \
          ORDER BY r.revision_id DESC \
          LIMIT $4"
    );
    let rows = client
        .query(
            &sql,
            &[&status_values, &cursor, &reviewer_fingerprint, &(limit + 1)],
        )
        .map_err(|err| RevisionError::Database(err.to_string()))?;
    let mut entries = rows
        .into_iter()
        .map(|row| entry_from_row(row, reviewer_fingerprint))
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if entries.len() as i64 > limit {
        entries.truncate(limit as usize);
        entries.last().map(|entry| entry.revision.revision_id)
    } else {
        None
    };
    Ok(ReviewQueue {
        entries,
        next_cursor,
        status: status_values.join(","),
        limit,
    })
}

/// Reads one submitted revision with its current approved value, events, and
/// same-field revision history. A draft id returns `UnknownRevision` so the
/// console cannot be used to inspect unsubmitted content.
pub fn read_review_detail(
    client: &mut Client,
    reviewer_fingerprint: &str,
    revision_id: i64,
) -> Result<ReviewDetail, RevisionError> {
    let mut tx = client.transaction()?;
    let sql = format!(
        "SELECT {REVISION_COLUMNS_QUALIFIED}, p.display_name, \
                v.value_text, v.visibility, v.updated_by, v.updated_at::text, \
                v.approved_revision_number, v.approved_revision_id, \
                m.media_id, m.media_kind, m.mime_type, m.file_name, m.byte_size, \
                m.width, m.height, \
                cm.media_id, cm.media_kind, cm.mime_type, cm.file_name, cm.byte_size, \
                cm.width, cm.height, \
                EXISTS (SELECT 1 FROM onboarding_person_account_link l \
                         WHERE l.person_id = r.person_id \
                           AND l.identity_fingerprint = $2) AS reviewer_owns_person, \
                (SELECT fr.generation \
                   FROM onboarding_profile_field_restriction fr \
                  WHERE fr.person_id = r.person_id AND fr.field_key = r.field_key \
                    AND fr.lifted_at IS NULL) AS current_restriction_generation \
           FROM onboarding_profile_revision r \
           JOIN person p ON p.person_id = r.person_id \
           LEFT JOIN onboarding_profile_value v \
             ON v.person_id = r.person_id AND v.field_key = r.field_key \
           LEFT JOIN onboarding_media_metadata m ON m.revision_id = r.revision_id \
           LEFT JOIN onboarding_media_metadata cm ON cm.revision_id = v.approved_revision_id \
          WHERE r.revision_id = $1 AND r.status <> 'draft'"
    );
    let row = tx
        .query_opt(&sql, &[&revision_id, &reviewer_fingerprint])
        .map_err(|err| RevisionError::Database(err.to_string()))?
        .ok_or(RevisionError::UnknownRevision)?;
    let entry = entry_from_row(row, reviewer_fingerprint)?;
    let events = read_events(&mut tx, revision_id)?;
    let history = read_history(&mut tx, &entry.revision)?;
    tx.commit()?;
    Ok(ReviewDetail {
        entry,
        events,
        history,
    })
}

fn entry_from_row(
    row: postgres::Row,
    reviewer_fingerprint: &str,
) -> Result<ReviewEntry, RevisionError> {
    let reviewer_owns_person: bool = row.get(36);
    let display_name: String = row.get(15);
    let current_value: Option<String> = row.get(16);
    let current_visibility: Option<String> = row.get(17);
    let current_updated_by: Option<String> = row.get(18);
    let current_updated_at: Option<String> = row.get(19);
    let current_revision_number: Option<i64> = row.get(20);
    let current_revision_id: Option<i64> = row.get(21);
    let current_restriction_generation: Option<i64> = row.get(37);
    let media_id: Option<i64> = row.get(22);
    let media_kind: Option<String> = row.get(23);
    let media_mime: Option<String> = row.get(24);
    let media_name: Option<String> = row.get(25);
    let media_bytes: Option<i64> = row.get(26);
    let media_width: Option<i32> = row.get(27);
    let media_height: Option<i32> = row.get(28);
    let current_media_id: Option<i64> = row.get(29);
    let current_media_kind: Option<String> = row.get(30);
    let current_media_mime: Option<String> = row.get(31);
    let current_media_name: Option<String> = row.get(32);
    let current_media_bytes: Option<i64> = row.get(33);
    let current_media_width: Option<i32> = row.get(34);
    let current_media_height: Option<i32> = row.get(35);

    let revision = row_to_revision(row)?;
    let current_approved = match current_value {
        Some(value_text) => Some(ApprovedProfileValue {
            field_key: revision.field_key.clone(),
            value_text,
            visibility: ProfileVisibility::parse(&current_visibility.unwrap_or_default())
                .ok_or_else(|| RevisionError::Database("unknown profile visibility".into()))?,
            updated_by: current_updated_by.unwrap_or_default(),
            updated_at: current_updated_at.unwrap_or_default(),
            approved_revision_number: current_revision_number.unwrap_or_default(),
            approved_revision_id: current_revision_id,
            restricted_from_family: current_restriction_generation.is_some(),
            restriction_generation: current_restriction_generation,
        }),
        None => None,
    };
    let media = media_id.map(|media_id| ReviewMedia {
        media_id,
        kind: media_kind.unwrap_or_default(),
        mime_type: media_mime,
        file_name: media_name,
        byte_size: media_bytes,
        width: media_width,
        height: media_height,
    });
    let current_media = current_media_id.map(|media_id| ReviewMedia {
        media_id,
        kind: current_media_kind.unwrap_or_default(),
        mime_type: current_media_mime,
        file_name: current_media_name,
        byte_size: current_media_bytes,
        width: current_media_width,
        height: current_media_height,
    });
    let decision_block = match revision.status {
        RevisionStatus::PendingReview if revision.changed_by == reviewer_fingerprint => {
            Some("self_submitted".to_string())
        }
        RevisionStatus::PendingReview if reviewer_owns_person => {
            Some("reviewer_owns_person".to_string())
        }
        RevisionStatus::PendingReview => None,
        _ => Some("not_pending".to_string()),
    };
    Ok(ReviewEntry {
        can_decide: decision_block.is_none(),
        decision_block,
        revision,
        display_name,
        current_approved,
        current_media,
        media,
    })
}

fn read_events(
    tx: &mut Transaction<'_>,
    revision_id: i64,
) -> Result<Vec<ReviewEvent>, RevisionError> {
    let rows = tx.query(
        "SELECT event, actor, note, created_at::text \
           FROM onboarding_profile_review_event \
          WHERE revision_id = $1 \
          ORDER BY event_id",
        &[&revision_id],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| ReviewEvent {
            event: row.get(0),
            actor: row.get(1),
            note: row.get(2),
            created_at: row.get(3),
        })
        .collect())
}

fn read_history(
    tx: &mut Transaction<'_>,
    revision: &ProfileRevision,
) -> Result<Vec<ProfileRevision>, RevisionError> {
    let sql = format!(
        "SELECT {REVISION_COLUMNS} FROM onboarding_profile_revision \
          WHERE person_id = $1 AND field_key = $2 AND revision_id <> $3 \
            AND status <> 'draft' \
          ORDER BY revision_number DESC \
          LIMIT 20"
    );
    let rows = tx.query(
        &sql,
        &[
            &revision.person_id,
            &revision.field_key,
            &revision.revision_id,
        ],
    )?;
    rows.into_iter().map(row_to_revision).collect()
}
