//! Audited identity provisioning and member access for an invitation.
//!
//! Two separate steps keep the archive closed until a person link is approved:
//!
//! * [`provision_identity_account`] creates (or reactivates) the identity row so
//!   the invitee can sign in. It grants no role and no corpus access.
//! * [`grant_member_access`] grants the viewer role and the archive corpus grant
//!   after an administrator approves the reviewed person link.
//!
//! Both are audited and idempotent. Neither grants `editor` or `admin`. An
//! unapproved invitation holder can therefore sign in and see their own account
//! and the pending-review state, but cannot read protected archive content.

use crate::identity::Role;
use crate::identity_store::identity_client::Client as IdentityClient;
use crate::identity_store::IdentityStoreError;
use crate::identity_store_pg::PgIdentityClient;

/// Creates or reactivates the identity row for one invited identity.
///
/// Returns the opaque identity account id. No role or corpus grant is written;
/// archive access stays closed until [`grant_member_access`] runs after review.
pub fn provision_identity_account(
    client: &mut PgIdentityClient,
    subject: &str,
    issuer: &str,
    actor: &str,
) -> Result<i64, IdentityStoreError> {
    client.provision_user(subject, issuer, actor)
}

/// Grants the member access an approved link implies, by opaque account id.
///
/// The genealogy database stores only the account id and a one-way fingerprint,
/// so the identity pair is read back here before the audited role and corpus
/// writes. A deactivated or unknown account is refused. Repeated calls are safe:
/// the viewer role and the corpus grant are added only when absent.
pub fn grant_member_access(
    client: &mut PgIdentityClient,
    account_id: i64,
    corpus: &str,
    actor: &str,
) -> Result<(), IdentityStoreError> {
    let Some((subject, issuer)) = client.active_identity_by_id(account_id)? else {
        return Err(IdentityStoreError::UnknownUser(format!(
            "account {account_id}"
        )));
    };
    IdentityClient::change_role(client, &subject, &issuer, Role::Viewer, actor, true)?;
    client.grant_corpus(&subject, &issuer, corpus, actor)?;
    Ok(())
}
