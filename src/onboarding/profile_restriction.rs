//! USR-006-P: owner-initiated privacy restriction (policy decision A2).
//!
//! An owner may immediately stop an approved, family-visible profile value from
//! being presented to the family. The historical approved revision is never
//! rewritten: a restriction is an additive, audited record that the family read
//! must honour.
//!
//! Lifting a restriction is deliberately not an owner action. It happens only
//! when an independent reviewer approves a *new* revision created after the
//! restriction (see [`super::profile_revisions::review_revision`]), so a stale
//! pending submission from before the restriction cannot silently republish the
//! field.

use postgres::Transaction;
use thiserror::Error;

use super::account_map::IdentityAccount;
use super::profile_revisions::{ProfileVisibility, ALLOWED_PROFILE_FIELDS};

/// The result of an owner restriction request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestrictionOutcome {
    pub field_key: String,
    pub generation: i64,
    /// True when an active restriction already covered this field. No new
    /// audit event is written in that case.
    pub already_restricted: bool,
}

/// Errors raised by owner restrictions.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RestrictionError {
    /// The caller holds no active reviewed account-person link.
    #[error("the authentication account has no reviewed person link")]
    Unlinked,
    /// The field is not a text profile field this feature can restrict.
    #[error("invalid profile field")]
    InvalidField,
    /// No approved value exists for the field.
    #[error("no approved value exists for this field")]
    NoApprovedValue,
    /// The approved value is not currently family-visible.
    #[error("only a family-visible approved value can be restricted")]
    NotFamilyVisible,
    /// The store failed.
    #[error("profile restriction database operation failed: {0}")]
    Database(String),
}

impl From<postgres::Error> for RestrictionError {
    fn from(err: postgres::Error) -> Self {
        RestrictionError::Database(err.to_string())
    }
}

/// Restricts one of the caller's own approved family-visible fields.
///
/// The restriction, its audit event, and the visibility decision commit
/// together. A repeated call for a field that is already restricted is an
/// idempotent success and does not write a second audit event.
///
/// The transaction takes the same per-field advisory lock the profile review
/// transaction uses, and locks the person row so it serializes with the family
/// read (which holds the person row `FOR SHARE`). A restriction therefore cannot
/// commit in the middle of a family read that was already deciding eligibility.
pub fn restrict_own_field(
    client: &mut postgres::Client,
    account: &IdentityAccount,
    field_key: &str,
) -> Result<RestrictionOutcome, RestrictionError> {
    let field_key = field_key.trim();
    if field_key.is_empty() || !ALLOWED_PROFILE_FIELDS.contains(&field_key) {
        return Err(RestrictionError::InvalidField);
    }
    let mut tx = client.transaction()?;

    // The reviewed link is the ownership proof; it is locked so a concurrent
    // revocation cannot land between this check and the write.
    let linked = tx.query_opt(
        "SELECT person_id FROM onboarding_person_account_link \
          WHERE account_id = $1 AND identity_fingerprint = $2 FOR SHARE",
        &[&account.account_id, &account.identity_fingerprint],
    )?;
    let Some(linked) = linked else {
        return Err(RestrictionError::Unlinked);
    };
    let person_id: i64 = linked.get(0);

    // Lock order matters: the profile review transaction locks the person row
    // first and then the per-field advisory lock, and the family read holds the
    // person row `FOR SHARE`. Take the person row first here too, so a
    // concurrent approval and this restriction cannot deadlock and a family read
    // cannot straddle the commit.
    tx.query_one(
        "SELECT 1 FROM person WHERE person_id = $1 FOR UPDATE",
        &[&person_id],
    )?;
    lock_field(&mut tx, person_id, field_key)?;

    // An active restriction is an idempotent success.
    let existing = tx.query_opt(
        "SELECT generation FROM onboarding_profile_field_restriction \
          WHERE person_id = $1 AND field_key = $2 AND lifted_at IS NULL FOR UPDATE",
        &[&person_id, &field_key],
    )?;
    if let Some(row) = existing {
        let generation: i64 = row.get(0);
        tx.commit()?;
        return Ok(RestrictionOutcome {
            field_key: field_key.to_string(),
            generation,
            already_restricted: true,
        });
    }

    let value = tx.query_opt(
        "SELECT visibility, approved_revision_id, approved_revision_number \
           FROM onboarding_profile_value WHERE person_id = $1 AND field_key = $2",
        &[&person_id, &field_key],
    )?;
    let Some(value) = value else {
        return Err(RestrictionError::NoApprovedValue);
    };
    let visibility: String = value.get(0);
    let approved_revision_id: Option<i64> = value.get(1);
    let approved_revision_number: i64 = value.get(2);
    if ProfileVisibility::parse(&visibility) != Some(ProfileVisibility::Family) {
        return Err(RestrictionError::NotFamilyVisible);
    }
    // Fail closed when the approval binding is incomplete: there is nothing
    // provable to restrict.
    let Some(approved_revision_id) = approved_revision_id else {
        return Err(RestrictionError::NoApprovedValue);
    };

    let generation: i64 = tx
        .query_one(
            "SELECT COALESCE(MAX(generation), 0) + 1 \
           FROM onboarding_profile_field_restriction \
          WHERE person_id = $1 AND field_key = $2",
            &[&person_id, &field_key],
        )?
        .get(0);

    let restriction_id: i64 = tx
        .query_one(
            "INSERT INTO onboarding_profile_field_restriction \
                 (person_id, field_key, generation, approved_revision_id, \
                  approved_revision_number, prior_visibility, new_visibility, \
                  restricted_by) \
             VALUES ($1, $2, $3, $4, $5, 'family', 'owner_private', $6) \
             RETURNING restriction_id",
            &[
                &person_id,
                &field_key,
                &generation,
                &approved_revision_id,
                &approved_revision_number,
                &account.identity_fingerprint,
            ],
        )?
        .get(0);

    tx.execute(
        "INSERT INTO onboarding_profile_field_restriction_event \
             (restriction_id, person_id, field_key, generation, event, actor, \
              approved_revision_id, approved_revision_number, prior_visibility, \
              new_visibility) \
         VALUES ($1, $2, $3, $4, 'restricted', $5, $6, $7, 'family', 'owner_private')",
        &[
            &restriction_id,
            &person_id,
            &field_key,
            &generation,
            &account.identity_fingerprint,
            &approved_revision_id,
            &approved_revision_number,
        ],
    )?;

    tx.commit()?;
    Ok(RestrictionOutcome {
        field_key: field_key.to_string(),
        generation,
        already_restricted: false,
    })
}

/// The same per-field serialization key the profile review transaction uses.
fn lock_field(
    tx: &mut Transaction<'_>,
    person_id: i64,
    field_key: &str,
) -> Result<(), RestrictionError> {
    let key = format!("profile-revision:{person_id}:{field_key}");
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
        &[&key],
    )?;
    Ok(())
}
