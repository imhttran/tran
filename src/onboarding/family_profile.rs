//! USR-006-A: approved-only family profile presentation.
//!
//! Reads the currently approved, family-visible profile values of another
//! member. The caller must be a linked, active family member, and the
//! membership check runs in the same transaction as the read, so a concurrent
//! link revocation cannot slip between authorization and data access.
//!
//! Only values whose effective visibility is `family` are returned. Drafts,
//! pending or rejected revisions, owner-private values, contact details the
//! owner kept private, and the profile media references are never read, so they
//! cannot appear in a response, an error body, or a log line.

use postgres::Transaction;
use thiserror::Error;

use super::account_map::IdentityAccount;
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

/// One person's family-visible profile.
///
/// `fields` may be empty: an authorized member with nothing approved for the
/// family audience still gets a profile, but no private information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyProfile {
    pub person_key: String,
    /// The shared genealogy display name, already visible in the family tree.
    pub display_name: String,
    pub fields: Vec<FamilyProfileField>,
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

    // Family membership: an active, reviewed link bound to this exact identity.
    let linked = tx.query_opt(
        "SELECT person_id FROM onboarding_person_account_link \
          WHERE account_id = $1 AND identity_fingerprint = $2 FOR SHARE",
        &[&requester.account_id, &requester.identity_fingerprint],
    )?;
    if linked.is_none() {
        return Err(FamilyProfileError::NotFamilyMember);
    }

    let target = tx.query_opt(
        "SELECT person_id, display_name FROM person WHERE person_key = $1",
        &[&person_key],
    )?;
    let Some(target) = target else {
        return Err(FamilyProfileError::UnknownPerson);
    };
    let person_id: i64 = target.get(0);
    let display_name: String = target.get(1);

    let fields = family_visible_values(&mut tx, person_id)?;
    tx.commit()?;
    Ok(FamilyProfile {
        person_key: person_key.to_string(),
        display_name,
        fields,
    })
}

/// Reads the approved values whose effective visibility is `family`.
///
/// The media reference fields are excluded because media presentation has its
/// own authorization rule; a profile text response must never imply that a
/// photograph or personal document is available.
fn family_visible_values(
    tx: &mut Transaction<'_>,
    person_id: i64,
) -> Result<Vec<FamilyProfileField>, FamilyProfileError> {
    let media_fields: Vec<String> = MEDIA_PROFILE_FIELDS
        .iter()
        .map(|field| field.to_string())
        .collect();
    let rows = tx.query(
        "SELECT field_key, value_text, visibility, approved_revision_number, \
                updated_at::text \
           FROM onboarding_profile_value \
          WHERE person_id = $1 AND visibility = 'family' \
            AND field_key <> ALL($2) \
          ORDER BY field_key",
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
