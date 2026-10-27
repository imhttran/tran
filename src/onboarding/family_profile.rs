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

use std::collections::BTreeMap;

use postgres::Transaction;
use thiserror::Error;

use crate::genealogy::dates as genealogy_dates;
use crate::genealogy::narrative::{self, FamilyPassage};
use crate::genealogy::photograph_store::{PhotographStore, PhotographStoreError};

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

/// One person's published archive history.
///
/// `history` may be empty for an authorized member. It never contains pending,
/// rejected, admin-only, restricted, or superseded passages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyHistory {
    pub person_key: String,
    /// The shared genealogy display name, already visible in the family tree.
    pub display_name: String,
    pub history: Vec<FamilyPassage>,
}

/// One published archive photograph.
///
/// The record carries only metadata and the PDF page citation. The storage key,
/// content hash, and filesystem location stay server-side; the bytes are served
/// by the authorized photograph route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyArchivePhotograph {
    pub asset_id: i64,
    pub caption: Option<String>,
    pub mime_type: String,
    pub byte_size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub document_version_id: i64,
    pub corpus: String,
    pub page_number: i32,
    pub created_at: String,
}

/// One confirmed person on a gallery photograph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GalleryPerson {
    pub person_key: String,
    pub display_name: String,
}

/// One published photograph in the family gallery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyGalleryPhotograph {
    pub photograph: FamilyArchivePhotograph,
    /// Confirmed links only. A proposed or rejected link never appears here.
    pub people: Vec<GalleryPerson>,
}

/// One stored photograph's bytes and media type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePhotographDownload {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

/// One person link as the administrator review screen reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminArchivePhotographLink {
    pub person_id: i64,
    pub person_key: String,
    pub display_name: String,
    pub status: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    /// The PDF page whose text supports the proposal.
    pub source_page: Option<i32>,
    /// The bounded supporting text.
    pub supporting_text: Option<String>,
    /// `high`, `medium`, or `low`.
    pub confidence: Option<String>,
    /// The signals that set the confidence.
    pub confidence_basis: Option<String>,
    /// Why an ambiguous match needs an explicit choice.
    pub ambiguity_note: Option<String>,
    /// `exact`, `alias`, `reordered`, or `partial`.
    pub match_kind: Option<String>,
    /// 1-based strength order.
    pub candidate_rank: Option<i32>,
    /// `caption`, `layout`, or `manual`.
    pub proposal_origin: Option<String>,
}

/// One archive photograph as the administrator review screen reads it.
///
/// The read model intentionally omits the storage key and content hash. The
/// reviewer sees provenance and review state, not the private byte location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminArchivePhotograph {
    pub asset_id: i64,
    pub caption: Option<String>,
    pub publication_state: String,
    pub mime_type: String,
    pub byte_size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub object_id: String,
    pub classification: String,
    pub classification_rule: String,
    pub document_version_id: i64,
    pub corpus: String,
    pub page_numbers: Vec<i32>,
    pub created_at: String,
    pub links: Vec<AdminArchivePhotographLink>,
}

/// Errors raised by an archive photograph byte read.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ArchivePhotographError {
    /// The caller is not a linked family member.
    #[error("the account is not a linked family member")]
    NotFamilyMember,
    /// No published asset exists for the id, or its bytes are unknown.
    #[error("no such archive photograph")]
    NotFound,
    /// The private byte store failed.
    #[error("the archive photograph store is unavailable")]
    StoreUnavailable,
    /// The genealogy store failed.
    #[error("archive photograph database operation failed: {0}")]
    Database(String),
}

impl From<postgres::Error> for ArchivePhotographError {
    fn from(err: postgres::Error) -> Self {
        ArchivePhotographError::Database(err.to_string())
    }
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
    require_active_link(&mut tx, requester)?;

    // `FOR SHARE` serializes this read with an owner restriction or approval,
    // which lock the person row for update. A restriction cannot commit in the
    // middle of a family read that has already decided eligibility.
    let Some((person_id, display_name)) = family_person(&mut tx, person_key)? else {
        return Err(FamilyProfileError::UnknownPerson);
    };

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

/// Reads one person's published archive photographs for a linked family member.
///
/// Only published assets with a confirmed link to this person are returned. A
/// proposed or rejected link never presents a photograph, and an unpublished or
/// reviewer-restricted asset is excluded exactly like a missing row. The PDF
/// page citation is the photograph's first occurrence in the source document.
pub fn read_family_archive_photographs(
    client: &mut postgres::Client,
    requester: &IdentityAccount,
    person_key: &str,
) -> Result<Vec<FamilyArchivePhotograph>, FamilyProfileError> {
    let person_key = person_key.trim();
    if person_key.is_empty() {
        return Err(FamilyProfileError::UnknownPerson);
    }
    let mut tx = client.transaction()?;
    require_active_link(&mut tx, requester)?;
    let Some((person_id, _)) = family_person(&mut tx, person_key)? else {
        return Err(FamilyProfileError::UnknownPerson);
    };
    let photographs = published_archive_photographs(&mut tx, Some(person_id))?;
    tx.commit()?;
    Ok(photographs)
}

/// Reads the family gallery of published archive photographs.
///
/// The gallery is family-only, so the caller must hold an active family link.
/// Every published photograph appears, including one with no confirmed person
/// link; only confirmed people are attached to an item.
pub fn read_family_gallery(
    client: &mut postgres::Client,
    requester: &IdentityAccount,
) -> Result<Vec<FamilyGalleryPhotograph>, FamilyProfileError> {
    let mut tx = client.transaction()?;
    require_active_link(&mut tx, requester)?;
    let photographs = published_archive_photographs(&mut tx, None)?;
    let mut people: BTreeMap<i64, Vec<GalleryPerson>> = BTreeMap::new();
    if !photographs.is_empty() {
        let ids: Vec<i64> = photographs.iter().map(|photo| photo.asset_id).collect();
        let rows = tx.query(
            "SELECT l.asset_id, p.person_key, p.display_name \
               FROM archive_photograph_person_link l \
               JOIN person p ON p.person_id = l.person_id \
              WHERE l.asset_id = ANY($1) AND l.status = 'confirmed' \
              ORDER BY l.asset_id, p.display_name, p.person_key",
            &[&ids],
        )?;
        for row in rows {
            people.entry(row.get(0)).or_default().push(GalleryPerson {
                person_key: row.get(1),
                display_name: row.get(2),
            });
        }
    }
    tx.commit()?;
    Ok(photographs
        .into_iter()
        .map(|photograph| FamilyGalleryPhotograph {
            people: people.remove(&photograph.asset_id).unwrap_or_default(),
            photograph,
        })
        .collect())
}

/// Reads one published archive photograph's bytes for a linked family member.
///
/// An unpublished or reviewer-restricted asset, an unknown id, and an unknown
/// storage key are all [`ArchivePhotographError::NotFound`], so a family reader
/// cannot use thumbnail and original responses to probe publication state.
pub fn read_family_archive_photograph(
    client: &mut postgres::Client,
    store: &PhotographStore,
    requester: &IdentityAccount,
    asset_id: i64,
) -> Result<ArchivePhotographDownload, ArchivePhotographError> {
    let mut tx = client.transaction()?;
    match require_active_link(&mut tx, requester) {
        Ok(_) => {}
        Err(FamilyProfileError::NotFamilyMember) => {
            return Err(ArchivePhotographError::NotFamilyMember)
        }
        Err(FamilyProfileError::UnknownPerson) => return Err(ArchivePhotographError::NotFound),
        Err(FamilyProfileError::Database(message)) => {
            return Err(ArchivePhotographError::Database(message))
        }
    }
    let row = tx.query_opt(
        "SELECT storage_key, mime_type \
           FROM archive_photograph_asset \
          WHERE asset_id = $1 \
            AND publication_state = 'published' \
            AND storage_key IS NOT NULL \
            AND mime_type IS NOT NULL",
        &[&asset_id],
    )?;
    let Some(row) = row else {
        return Err(ArchivePhotographError::NotFound);
    };
    let key: String = row.get(0);
    let mime_type: String = row.get(1);
    tx.commit()?;
    read_stored_photograph(store, &key, mime_type)
}

/// Reads one archive photograph's bytes for an administrator review read.
///
/// The review read deliberately ignores publication state so a reviewer can
/// inspect a pending or restricted photograph before deciding. Authorization is
/// the caller's `DocumentReview` operation, enforced by the API handler.
pub fn read_admin_archive_photograph(
    client: &mut postgres::Client,
    store: &PhotographStore,
    asset_id: i64,
) -> Result<ArchivePhotographDownload, ArchivePhotographError> {
    let mut tx = client.transaction()?;
    let row = tx.query_opt(
        "SELECT storage_key, mime_type \
           FROM archive_photograph_asset \
          WHERE asset_id = $1 \
            AND storage_key IS NOT NULL \
            AND mime_type IS NOT NULL",
        &[&asset_id],
    )?;
    let Some(row) = row else {
        return Err(ArchivePhotographError::NotFound);
    };
    let key: String = row.get(0);
    let mime_type: String = row.get(1);
    tx.commit()?;
    read_stored_photograph(store, &key, mime_type)
}

/// Reads every archive photograph for the administrator review screen.
///
/// All publication states are included. The read model omits the storage key,
/// content hash, and every filesystem path.
pub fn read_admin_archive_photographs(
    client: &mut postgres::Client,
) -> Result<Vec<AdminArchivePhotograph>, FamilyProfileError> {
    let mut tx = client.transaction()?;
    let rows = tx.query(
        "SELECT a.asset_id, \
                COALESCE( \
                  (SELECT NULLIF(btrim(i.caption), '') \
                     FROM archive_photograph_inventory_image i \
                    WHERE i.inventory_id = a.inventory_id \
                      AND i.object_number = a.object_number \
                      AND i.object_generation = a.object_generation \
                      AND i.caption IS NOT NULL \
                      AND btrim(i.caption) <> '' \
                    ORDER BY i.page_number, i.occurrence_ordinal \
                    LIMIT 1), \
                  (SELECT NULLIF(btrim(p.caption), '') \
                     FROM archive_photograph_inventory_page p \
                    WHERE p.inventory_id = a.inventory_id \
                      AND p.page_number = ( \
                          SELECT o.page_number \
                            FROM archive_photograph_occurrence o \
                           WHERE o.asset_id = a.asset_id \
                           ORDER BY o.page_number, o.occurrence_ordinal \
                           LIMIT 1) \
                  ) \
                ) AS caption, \
                a.publication_state, a.mime_type, a.byte_size, \
                a.pixel_width, a.pixel_height, a.object_id, \
                a.classification, a.classification_rule, \
                inv.document_version_id, inv.corpus, \
                ARRAY( \
                  SELECT o.page_number \
                    FROM archive_photograph_occurrence o \
                   WHERE o.asset_id = a.asset_id \
                   ORDER BY o.page_number, o.occurrence_ordinal \
                ) AS page_numbers, \
                inv.created_at::text \
           FROM archive_photograph_asset a \
           JOIN archive_photograph_inventory inv \
             ON inv.inventory_id = a.inventory_id \
          WHERE a.storage_key IS NOT NULL \
            AND a.mime_type IS NOT NULL \
            AND a.byte_size IS NOT NULL \
          ORDER BY inv.created_at DESC, a.asset_id",
        &[],
    )?;
    let mut photographs: Vec<AdminArchivePhotograph> = rows
        .into_iter()
        .map(|row| AdminArchivePhotograph {
            asset_id: row.get(0),
            caption: row.get(1),
            publication_state: row.get(2),
            mime_type: row.get(3),
            byte_size: row.get(4),
            width: row.get(5),
            height: row.get(6),
            object_id: row.get(7),
            classification: row.get(8),
            classification_rule: row.get(9),
            document_version_id: row.get(10),
            corpus: row.get(11),
            page_numbers: row.get(12),
            created_at: row.get(13),
            links: Vec::new(),
        })
        .collect();
    if !photographs.is_empty() {
        let ids: Vec<i64> = photographs.iter().map(|photo| photo.asset_id).collect();
        let rows = tx.query(
            "SELECT l.asset_id, l.person_id, p.person_key, p.display_name, l.status, \
                    l.decided_by, l.decided_at::text, l.source_page, \
                    l.supporting_text, l.confidence, l.confidence_basis, \
                    l.ambiguity_note, l.match_kind, l.candidate_rank, \
                    l.proposal_origin \
               FROM archive_photograph_person_link l \
               JOIN person p ON p.person_id = l.person_id \
              WHERE l.asset_id = ANY($1) \
              ORDER BY l.asset_id, \
                       CASE l.status WHEN 'confirmed' THEN 0 \
                                     WHEN 'proposed' THEN 1 ELSE 2 END, \
                       COALESCE(l.candidate_rank, 999), \
                       p.display_name, p.person_key",
            &[&ids],
        )?;
        let mut links: BTreeMap<i64, Vec<AdminArchivePhotographLink>> = BTreeMap::new();
        for row in rows {
            links
                .entry(row.get(0))
                .or_default()
                .push(AdminArchivePhotographLink {
                    person_id: row.get(1),
                    person_key: row.get(2),
                    display_name: row.get(3),
                    status: row.get(4),
                    decided_by: row.get(5),
                    decided_at: row.get(6),
                    source_page: row.get(7),
                    supporting_text: row.get(8),
                    confidence: row.get(9),
                    confidence_basis: row.get(10),
                    ambiguity_note: row.get(11),
                    match_kind: row.get(12),
                    candidate_rank: row.get(13),
                    proposal_origin: row.get(14),
                });
        }
        for photograph in &mut photographs {
            photograph.links = links.remove(&photograph.asset_id).unwrap_or_default();
        }
    }
    tx.commit()?;
    Ok(photographs)
}

/// Reads the recorded caption for one archive photograph, or `None`.
///
/// Only a caption stated by the document (or written by a reviewer into the
/// inventory image) is returned. No name or page position is inferred.
pub fn archive_photograph_caption(
    client: &mut postgres::Client,
    asset_id: i64,
) -> Result<Option<String>, FamilyProfileError> {
    let row = client.query_opt(
        "SELECT COALESCE( \
                  (SELECT NULLIF(btrim(i.caption), '') \
                     FROM archive_photograph_inventory_image i \
                    WHERE i.inventory_id = a.inventory_id \
                      AND i.object_number = a.object_number \
                      AND i.object_generation = a.object_generation \
                      AND i.caption IS NOT NULL \
                      AND btrim(i.caption) <> '' \
                    ORDER BY i.page_number, i.occurrence_ordinal \
                    LIMIT 1), \
                  (SELECT NULLIF(btrim(p.caption), '') \
                     FROM archive_photograph_inventory_page p \
                    WHERE p.inventory_id = a.inventory_id \
                      AND p.page_number = ( \
                          SELECT o.page_number \
                            FROM archive_photograph_occurrence o \
                           WHERE o.asset_id = a.asset_id \
                           ORDER BY o.page_number, o.occurrence_ordinal \
                           LIMIT 1) \
                  ) \
                ) \
           FROM archive_photograph_asset a \
          WHERE a.asset_id = $1",
        &[&asset_id],
    )?;
    Ok(row.and_then(|row| row.get(0)))
}

/// The source document coordinates for one archive photograph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePhotographSource {
    pub document_version_id: i64,
    pub corpus: String,
}

/// Reads the source document version and corpus for one photograph asset.
pub fn archive_photograph_source(
    client: &mut postgres::Client,
    asset_id: i64,
) -> Result<Option<ArchivePhotographSource>, FamilyProfileError> {
    let row = client.query_opt(
        "SELECT i.document_version_id, i.corpus \
           FROM archive_photograph_asset a \
           JOIN archive_photograph_inventory i ON i.inventory_id = a.inventory_id \
          WHERE a.asset_id = $1",
        &[&asset_id],
    )?;
    Ok(row.map(|row| ArchivePhotographSource {
        document_version_id: row.get(0),
        corpus: row.get(1),
    }))
}

/// Reads one person's published archive history for a family reader.
///
/// An administrator may read it without a family-person link; every other
/// caller still needs an active linked membership. The check runs in the same
/// transaction as the read, and the same error mapping is used, so an
/// unauthorized caller cannot distinguish a missing person from a forbidden
/// one. The returned list is the [`narrative::family_history`] projection:
/// approved, family-visible, accepted-link, newest-approved-revision rows only.
pub fn read_family_history(
    client: &mut postgres::Client,
    requester: &IdentityAccount,
    person_key: &str,
    is_admin: bool,
) -> Result<FamilyHistory, FamilyProfileError> {
    let person_key = person_key.trim();
    if person_key.is_empty() {
        return Err(FamilyProfileError::UnknownPerson);
    }
    let mut tx = client.transaction()?;
    require_history_access(&mut tx, requester, is_admin)?;
    let Some((person_id, display_name)) = family_person(&mut tx, person_key)? else {
        return Err(FamilyProfileError::UnknownPerson);
    };
    let history = narrative::family_history(&mut tx, person_id)
        .map_err(|err| FamilyProfileError::Database(err.to_string()))?;
    tx.commit()?;
    Ok(FamilyHistory {
        person_key: person_key.to_string(),
        display_name,
        history,
    })
}

/// Whether this caller may read published family history.
///
/// The rule is the same for the tree history count and the person-scoped read:
/// an administrator may read it without a person link, and every other caller
/// needs an active linked membership. A false result is fail-closed: the tree
/// still renders, without history links.
pub fn can_read_family_history(
    client: &mut postgres::Client,
    requester: &IdentityAccount,
    is_admin: bool,
) -> Result<bool, FamilyProfileError> {
    let mut tx = client.transaction()?;
    let allowed = history_access(&mut tx, requester, is_admin)?;
    tx.commit()?;
    Ok(allowed)
}

fn require_history_access(
    tx: &mut Transaction<'_>,
    requester: &IdentityAccount,
    is_admin: bool,
) -> Result<(), FamilyProfileError> {
    if history_access(tx, requester, is_admin)? {
        Ok(())
    } else {
        Err(FamilyProfileError::NotFamilyMember)
    }
}

/// The one history-audience rule: an administrator, or an active linked member.
fn history_access(
    tx: &mut Transaction<'_>,
    requester: &IdentityAccount,
    is_admin: bool,
) -> Result<bool, FamilyProfileError> {
    if is_admin {
        return Ok(true);
    }
    match require_active_link(tx, requester) {
        Ok(_) => Ok(true),
        Err(FamilyProfileError::NotFamilyMember) => Ok(false),
        Err(err) => Err(err),
    }
}

/// The shared membership check: the caller's exact identity must hold a link.
fn require_active_link(
    tx: &mut Transaction<'_>,
    requester: &IdentityAccount,
) -> Result<i64, FamilyProfileError> {
    let linked = tx.query_opt(
        "SELECT person_id FROM onboarding_person_account_link \
          WHERE account_id = $1 AND identity_fingerprint = $2 FOR SHARE",
        &[&requester.account_id, &requester.identity_fingerprint],
    )?;
    linked
        .map(|row| row.get(0))
        .ok_or(FamilyProfileError::NotFamilyMember)
}

/// Looks up the person a profile or history read targets, locking the row so a
/// concurrent restriction cannot commit between authorization and the read.
fn family_person(
    tx: &mut Transaction<'_>,
    person_key: &str,
) -> Result<Option<(i64, String)>, FamilyProfileError> {
    let target = tx.query_opt(
        "SELECT person_id, display_name FROM person WHERE person_key = $1 FOR SHARE",
        &[&person_key],
    )?;
    Ok(target.map(|row| (row.get(0), row.get(1))))
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
fn published_archive_photographs(
    tx: &mut Transaction<'_>,
    person_id: Option<i64>,
) -> Result<Vec<FamilyArchivePhotograph>, FamilyProfileError> {
    let rows = tx.query(
        "SELECT a.asset_id, \
                COALESCE( \
                  (SELECT NULLIF(btrim(i.caption), '') \
                     FROM archive_photograph_inventory_image i \
                    WHERE i.inventory_id = a.inventory_id \
                      AND i.object_number = a.object_number \
                      AND i.object_generation = a.object_generation \
                      AND i.caption IS NOT NULL \
                      AND btrim(i.caption) <> '' \
                    ORDER BY i.page_number, i.occurrence_ordinal \
                    LIMIT 1), \
                  (SELECT NULLIF(btrim(p.caption), '') \
                     FROM archive_photograph_inventory_page p \
                    WHERE p.inventory_id = a.inventory_id \
                      AND p.page_number = occ.page_number) \
                ) AS caption, \
                a.mime_type, a.byte_size, a.pixel_width, a.pixel_height, \
                inv.document_version_id, inv.corpus, occ.page_number, \
                inv.created_at::text \
           FROM archive_photograph_asset a \
           JOIN archive_photograph_inventory inv \
             ON inv.inventory_id = a.inventory_id \
           JOIN LATERAL ( \
                SELECT o.page_number \
                  FROM archive_photograph_occurrence o \
                 WHERE o.asset_id = a.asset_id \
                 ORDER BY o.page_number, o.occurrence_ordinal \
                 LIMIT 1 \
           ) occ ON TRUE \
          WHERE a.publication_state = 'published' \
            AND a.storage_key IS NOT NULL \
            AND a.mime_type IS NOT NULL \
            AND a.byte_size IS NOT NULL \
            AND ($1::bigint IS NULL OR EXISTS ( \
                 SELECT 1 FROM archive_photograph_person_link l \
                  WHERE l.asset_id = a.asset_id \
                    AND l.person_id = $1 \
                    AND l.status = 'confirmed' \
            )) \
          ORDER BY inv.created_at DESC, a.asset_id",
        &[&person_id],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| FamilyArchivePhotograph {
            asset_id: row.get(0),
            caption: row.get(1),
            mime_type: row.get(2),
            byte_size: row.get(3),
            width: row.get(4),
            height: row.get(5),
            document_version_id: row.get(6),
            corpus: row.get(7),
            page_number: row.get(8),
            created_at: row.get(9),
        })
        .collect())
}

fn read_stored_photograph(
    store: &PhotographStore,
    key: &str,
    mime_type: String,
) -> Result<ArchivePhotographDownload, ArchivePhotographError> {
    match store.read(key) {
        Ok(bytes) => Ok(ArchivePhotographDownload { bytes, mime_type }),
        Err(PhotographStoreError::UnknownKey(_)) => Err(ArchivePhotographError::NotFound),
        Err(_) => Err(ArchivePhotographError::StoreUnavailable),
    }
}

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
