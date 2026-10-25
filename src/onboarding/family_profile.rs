//! USR-006-A: approved-only family profile presentation.
//!
//! Reads the currently approved, family-visible profile values of another
//! member. The caller must be a linked, active family member, and the
//! membership check runs in the same transaction as the read, so a concurrent
//! link revocation cannot slip between authorization and data access.
//!
//! Only values whose effective visibility is `family` are returned. Drafts,
//! pending or rejected revisions, owner-private values, contact details the
//! owner kept private, and private media are never returned, so they cannot
//! appear in a response, an error body, or a log line. Media is listed only when
//! an approved, family-visible asset is the person's current approved value for
//! that field and no owner restriction is active; the bytes themselves are only
//! served by the family media route.

use postgres::Transaction;
use thiserror::Error;

use crate::genealogy::dates as genealogy_dates;
use crate::genealogy::narrative::{self, FamilyPassage};

use super::account_map::IdentityAccount;
use super::media::{FAMILY_VISIBLE_MEDIA_JOIN, FAMILY_VISIBLE_MEDIA_PREDICATE};
use super::profile_revisions::MEDIA_PROFILE_FIELDS;

/// One approved family-visible profile value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyProfileField {
    pub field_key: String,
    pub value: String,
    pub visibility: String,
    /// Provenance: the approved revision this value came from.
    pub approved_revision_number: i64,
    /// Provenance: when the approved value was last written.
    pub updated_at: String,
}

/// One verified birth or death date in a family profile.
///
/// Only `verified` genealogy assertions are returned, so an unreviewed or
/// disputed date never reaches the family audience. `value_text` keeps the
/// source wording; the structured components carry the precision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyProfileDate {
    /// `birth_date` or `death_date`.
    pub kind: String,
    /// The original source wording.
    pub value_text: Option<String>,
    /// Partial ISO start value.
    pub start: String,
    /// Partial ISO end value, for a range.
    pub end: Option<String>,
    /// `day`, `month`, `year`, or `range`.
    pub precision: String,
    /// `exact`, `circa`, `before`, or `after`.
    pub qualifier: String,
}

/// One approved, family-visible media item.
///
/// `media_id` addresses the asset through the family media route; the storage
/// key, the content hash, and the owning person id never leave the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyProfileMedia {
    pub media_id: i64,
    pub kind: String,
    pub mime_type: String,
    pub byte_size: i64,
    pub file_name: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Provenance: the approved revision this asset came from.
    pub approved_revision_number: i64,
    pub created_at: String,
}

/// One person's family-visible profile.
///
/// `fields` and `media` may both be empty: an authorized member with nothing
/// approved for the family audience still gets a profile, but no private
/// information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyProfile {
    pub person_key: String,
    /// The shared genealogy display name, already visible in the family tree.
    pub display_name: String,
    pub fields: Vec<FamilyProfileField>,
    pub media: Vec<FamilyProfileMedia>,
    /// Verified birth and death dates. Empty when nothing is verified yet.
    pub dates: Vec<FamilyProfileDate>,
    /// Approved archive passages linked to this person. Separate from the
    /// member's own `biography`, `my_story`, and `family_memories` fields.
    pub history: Vec<FamilyPassage>,
}

/// Errors raised by family profile presentation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FamilyProfileError {
    /// The caller holds no active reviewed account-person link.
    #[error("the account is not a linked family member")]
    NotFamilyMember,
    /// The requested person key does not exist.
    #[error("no such family person")]
    UnknownPerson,
    /// The genealogy store failed.
    #[error("family profile database operation failed: {0}")]
    Database(String),
}

impl From<postgres::Error> for FamilyProfileError {
    fn from(err: postgres::Error) -> Self {
        FamilyProfileError::Database(err.to_string())
    }
}

/// Reads one person's approved, family-visible profile for a linked member.
///
/// Authorization and data access are one transaction: the caller's link row is
/// locked `FOR SHARE` and the values are read before the transaction commits.
/// A missing link is reported before the person is looked up, so an unlinked
/// caller cannot learn whether a person key exists.
pub fn read_family_profile(
    client: &mut postgres::Client,
    requester: &IdentityAccount,
    person_key: &str,
) -> Result<FamilyProfile, FamilyProfileError> {
    let person_key = person_key.trim();
    if person_key.is_empty() {
        return Err(FamilyProfileError::UnknownPerson);
    }
    let mut tx = client.transaction()?;

    // Family membership: a reviewed link bound to this exact identity. A link
    // has no status column by design: revocation deletes the row, so "the row
    // exists with this fingerprint" is exactly "the membership is active".
    let linked = tx.query_opt(
        "SELECT person_id FROM onboarding_person_account_link \
          WHERE account_id = $1 AND identity_fingerprint = $2 FOR SHARE",
        &[&requester.account_id, &requester.identity_fingerprint],
    )?;
    if linked.is_none() {
        return Err(FamilyProfileError::NotFamilyMember);
    }

    // `FOR SHARE` serializes this read with an owner restriction or approval,
    // which lock the person row for update. A restriction cannot commit in the
    // middle of a family read that has already decided eligibility.
    let target = tx.query_opt(
        "SELECT person_id, display_name FROM person WHERE person_key = $1 FOR SHARE",
        &[&person_key],
    )?;
    let Some(target) = target else {
        return Err(FamilyProfileError::UnknownPerson);
    };
    let person_id: i64 = target.get(0);
    let display_name: String = target.get(1);

    let fields = family_visible_values(&mut tx, person_id)?;
    let media = family_visible_media(&mut tx, person_id)?;
    let dates = verified_dates(&mut tx, person_id)?;
    let history = narrative::family_history(&mut tx, person_id)
        .map_err(|err| FamilyProfileError::Database(err.to_string()))?;
    tx.commit()?;
    Ok(FamilyProfile {
        person_key: person_key.to_string(),
        display_name,
        fields,
        media,
        dates,
        history,
    })
}

/// Reads the approved values whose effective visibility is `family`.
///
/// The media reference fields are excluded because media presentation has its
/// own authorization rule; a profile text response must never imply that a
/// photograph or personal document is available.
///
/// The read also re-establishes the provenance binding instead of trusting it:
/// the value must join an `approved` revision for the same person and field
/// whose number, text, and visibility match. The write-time guard
/// (`onboarding_profile_value_guard`, migration 0012) enforces the same rule for
/// new and updated rows, but 0012 does not backfill `approved_revision_id` for
/// values that existed before it and the column is nullable, so a legacy or
/// out-of-band row could otherwise be presented without provable approval. Such
/// a row fails the join and is silently excluded, which is the fail-closed
/// behavior the publication contract requires.
fn family_visible_values(
    tx: &mut Transaction<'_>,
    person_id: i64,
) -> Result<Vec<FamilyProfileField>, FamilyProfileError> {
    let media_fields: Vec<String> = MEDIA_PROFILE_FIELDS
        .iter()
        .map(|field| field.to_string())
        .collect();
    let rows = tx.query(
        "SELECT v.field_key, v.value_text, v.visibility, v.approved_revision_number, \
                v.updated_at::text \
           FROM onboarding_profile_value v \
           JOIN onboarding_profile_revision r \
             ON r.revision_id = v.approved_revision_id \
            AND r.person_id = v.person_id \
            AND r.field_key = v.field_key \
            AND r.revision_number = v.approved_revision_number \
            AND r.status = 'approved' \
            AND r.visibility = v.visibility \
            AND r.new_value_text = v.value_text \
          WHERE v.person_id = $1 AND v.visibility = 'family' \
            AND v.field_key <> ALL($2) \
            AND NOT EXISTS ( \
                SELECT 1 FROM onboarding_profile_field_restriction fr \
                 WHERE fr.person_id = v.person_id \
                   AND fr.field_key = v.field_key \
                   AND fr.lifted_at IS NULL \
            ) \
          ORDER BY v.field_key",
        &[&person_id, &media_fields],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| FamilyProfileField {
            field_key: row.get(0),
            value: row.get(1),
            visibility: row.get(2),
            approved_revision_number: row.get(3),
            updated_at: row.get(4),
        })
        .collect())
}

/// Reads the approved, family-visible media assets for one person.
///
/// The predicate is shared with the family asset route
/// ([`FAMILY_VISIBLE_MEDIA_PREDICATE`]) so a listed asset and a downloaded asset
/// are authorized by exactly the same rule: approved revision, matching
/// provenance, current approved value, `family` visibility, and no active owner
/// restriction. Pending, rejected, owner-private, superseded, restricted, and
/// unprovenanced rows are silently excluded rather than reported.
fn family_visible_media(
    tx: &mut Transaction<'_>,
    person_id: i64,
) -> Result<Vec<FamilyProfileMedia>, FamilyProfileError> {
    let sql = format!(
        "SELECT m.media_id, m.media_kind, m.mime_type, m.byte_size, m.file_name, \
                m.width, m.height, r.revision_number, m.created_at::text \
           FROM onboarding_media_metadata m \
           {FAMILY_VISIBLE_MEDIA_JOIN} \
          WHERE m.person_id = $1 AND {FAMILY_VISIBLE_MEDIA_PREDICATE} \
          ORDER BY m.media_kind, m.media_id"
    );
    let rows = tx.query(&sql, &[&person_id])?;
    Ok(rows
        .into_iter()
        .map(|row| FamilyProfileMedia {
            media_id: row.get(0),
            kind: row.get(1),
            mime_type: row.get(2),
            byte_size: row.get(3),
            file_name: row.get(4),
            width: row.get(5),
            height: row.get(6),
            approved_revision_number: row.get(7),
            created_at: row.get(8),
        })
        .collect())
}

/// Reads one verified birth and one verified death date, if they exist.
///
/// A date is presentation-worthy only when a reviewer verified it. Unverified,
/// needs-review, disputed, and date-unresolved assertions are excluded, so the
/// family audience never sees a claim the archive has not accepted. When two
/// verified values of the same kind exist (which the verification rule blocks),
/// the first in stable order is used and no value is invented.
fn verified_dates(
    tx: &mut Transaction<'_>,
    person_id: i64,
) -> Result<Vec<FamilyProfileDate>, FamilyProfileError> {
    let current = crate::genealogy::current::CURRENT_ASSERTION;
    let sql = format!(
        "SELECT a.assertion_kind, a.value_text, a.value_date_start, a.value_date_end,
                a.value_date_precision, a.value_date_qualifier
           FROM assertion a
          WHERE a.person_id = $1
            AND a.assertion_kind IN ('birth_date', 'death_date')
            AND a.review_status = 'verified'
            AND ({current})
            AND a.value_date_start IS NOT NULL
          ORDER BY a.assertion_kind, a.value_date_start, a.assertion_id"
    );
    let rows = tx.query(&sql, &[&person_id])?;

    let mut by_kind: std::collections::BTreeMap<String, Vec<FamilyProfileDate>> =
        std::collections::BTreeMap::new();
    for row in rows {
        let kind: String = row.get(0);
        // A verified row without readable components is omitted rather than
        // presented with a guessed precision.
        let (Some(precision), Some(qualifier)) = (
            row.get::<_, Option<String>>(4),
            row.get::<_, Option<String>>(5),
        ) else {
            continue;
        };
        let date = FamilyProfileDate {
            kind: kind.clone(),
            value_text: row.get(1),
            start: row.get(2),
            end: row.get(3),
            precision,
            qualifier,
        };
        let group = by_kind.entry(kind).or_default();
        if !group.contains(&date) {
            group.push(date);
        }
    }
    // Overlapping verified claims are refinements, so the more precise one is
    // shown. Disjoint claims are an unsettled disagreement, so the page shows
    // neither rather than picking one.
    Ok(by_kind.into_values().filter_map(representative).collect())
}

/// Chooses the date to present from the verified claims of one kind.
///
/// Overlapping claims are refinements, so the more precise one is shown. Disjoint
/// claims are an unsettled disagreement, so the page shows neither.
fn representative(group: Vec<FamilyProfileDate>) -> Option<FamilyProfileDate> {
    let claims: Vec<(String, Option<String>, genealogy_dates::DatePrecision)> = group
        .iter()
        .map(|date| {
            let precision = genealogy_dates::DatePrecision::parse(&date.precision)
                .unwrap_or(genealogy_dates::DatePrecision::Year);
            (date.start.clone(), date.end.clone(), precision)
        })
        .collect();
    genealogy_dates::representative_claim(&claims).map(|index| group[index].clone())
}
