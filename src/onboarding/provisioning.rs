//! Audited member provisioning for an invited identity.
//!
//! Invitation acceptance must not require an operator to provision the account by
//! hand. [`provision_invited_member`] creates (or reactivates) the identity row
//! for the immutable `(issuer, subject)` pair and grants exactly the member
//! access an invitation implies: the `viewer` role and one archive corpus grant.
//!
//! It never grants `editor` or `admin`. Every write goes through the identity
//! store's audited, idempotent operations, so a retried activation neither
//! duplicates a role nor removes one, and each attempt leaves an audit event.

use crate::identity::Role;
use crate::identity_store::identity_client::Client as IdentityClient;
use crate::identity_store::IdentityStoreError;
use crate::identity_store_pg::PgIdentityClient;

/// Provisions member access for one invited identity.
///
/// Returns the opaque identity account id. Repeated calls are safe: the user row
/// is reused (or reactivated), the viewer role is granted only when absent, and
/// the corpus grant is added only when absent.
pub fn provision_invited_member(
    client: &mut PgIdentityClient,
    subject: &str,
    issuer: &str,
    corpus: &str,
    actor: &str,
) -> Result<i64, IdentityStoreError> {
    let user_id = client.provision_user(subject, issuer, actor)?;
    // The trait method writes the role and its audit row in one transaction.
    IdentityClient::change_role(client, subject, issuer, Role::Viewer, actor, true)?;
    client.grant_corpus(subject, issuer, corpus, actor)?;
    Ok(user_id)
}
