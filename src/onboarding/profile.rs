//! Owner-scoped, read-only member profiles.
//!
//! A profile is read only through the reviewed link that binds one immutable
//! OIDC identity to one genealogy person. The caller never supplies a person
//! key, account id, or reviewer: the service derives ownership from the verified
//! identity and refuses to guess.
//!
//! USR-004A intentionally reuses the additive tables created by migration 0007.
//! `onboarding_profile_value` holds the current value for one `(person, field)`
//! pair, and `onboarding_profile_revision` preserves history. No profile or
//! media table is introduced here.
//!
//! The response contains private family data, so an absent link, a revoked
//! link, or a fingerprint mismatch returns `None` rather than an empty profile.

use thiserror::Error;

use crate::onboarding::account_map::IdentityAccount;
use crate::onboarding::link::{link_for_account, LinkError};

/// One current private profile field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileField {
    /// The stable field name, for example `biography`.
    pub field_key: String,
    /// The current value. History lives in `onboarding_profile_revision`.
    pub value: String,
    /// The database timestamp rendered as text, so this module needs no
    /// additional date/time dependency.
    pub updated_at: Option<String>,
}

/// The private profile owned by one linked account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileView {
    /// The stable genealogy person key. It is opaque outside the service.
    pub person_key: String,
    /// The display name stored on the linked person row.
    pub display_name: String,
    /// Current values, ordered by field key. Empty when none are recorded.
    pub fields: Vec<ProfileField>,
}

/// Errors raised while reading a profile.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProfileError {
    /// The genealogy database operation failed.
    #[error("profile database operation failed: {0}")]
    Database(String),
}

impl From<LinkError> for ProfileError {
    fn from(err: LinkError) -> Self {
        ProfileError::Database(err.to_string())
    }
}

/// Reads the profile owned by `account`.
///
/// `account` must come from [`crate::onboarding::resolve_account`], which rejects
/// a deactivated identity before this function is reached. The lookup is
/// anchored on both the opaque account id and the one-way identity fingerprint.
/// A missing link returns `Ok(None)` so the API can say "unlinked" without
/// returning any private field. A revoked link is deleted by
/// [`crate::onboarding::revoke_link`], so it also returns `Ok(None)`.
pub fn read_own_profile(
    client: &mut postgres::Client,
    account: &IdentityAccount,
) -> Result<Option<ProfileView>, ProfileError> {
    let Some(link) = link_for_account(client, account.account_id, &account.identity_fingerprint)?
    else {
        return Ok(None);
    };

    let rows = client
        .query(
            "SELECT p.person_key, p.display_name, v.field_key, v.value_text, \
                    v.updated_at::text \
               FROM person p \
               LEFT JOIN onboarding_profile_value v ON v.person_id = p.person_id \
              WHERE p.person_id = $1 \
              ORDER BY v.field_key",
            &[&link.person_id],
        )
        .map_err(|err| ProfileError::Database(err.to_string()))?;

    let mut profile: Option<ProfileView> = None;
    for row in rows {
        if profile.is_none() {
            profile = Some(ProfileView {
                person_key: row.get(0),
                display_name: row.get(1),
                fields: Vec::new(),
            });
        }
        let field_key: Option<String> = row.get(2);
        if let Some(field_key) = field_key {
            if let Some(profile) = profile.as_mut() {
                profile.fields.push(ProfileField {
                    field_key,
                    value: row.get(3),
                    updated_at: row.get(4),
                });
            }
        }
    }
    Ok(profile)
}
