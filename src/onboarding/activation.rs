//! Invitation welcome and activation (PIR-006).
//!
//! An invitation link is a bearer credential. [`preview_invitation`] reads the
//! intended person and the invitation state for the welcome screen without
//! writing anything; [`accept_invitation`] completes activation for one signed-in
//! account.
//!
//! Activation never creates an account, never replaces an existing
//! person-account link, and never grants a role. It writes exactly one reviewed
//! link for the invited person and the caller's own account, then consumes the
//! one-time invitation. The account keeps whatever roles and grants it was
//! provisioned with, so acceptance can never grant admin or family-head
//! authority. No return value carries a token, a full address, or a credential.

use postgres::Client;
use thiserror::Error;

use crate::onboarding::account_map::IdentityAccount;
use crate::onboarding::invitation::{accept, hash_token, InvitationError};
use crate::onboarding::link::{
    link_for_account, link_reviewed, revoke_link, LinkDecision, LinkError,
};

/// The recorded lifecycle of an invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationState {
    /// Still usable: not accepted, not revoked, and not past its expiry.
    Pending,
    /// Past its expiry without acceptance.
    Expired,
    /// Revoked by a family administrator.
    Revoked,
    /// Already accepted.
    Accepted,
}

impl ActivationState {
    /// The stable string code for this state.
    pub fn as_str(self) -> &'static str {
        match self {
            ActivationState::Pending => "pending",
            ActivationState::Expired => "expired",
            ActivationState::Revoked => "revoked",
            ActivationState::Accepted => "accepted",
        }
    }
}

/// The welcome-screen view of one invitation link.
///
/// It carries the intended person's display name so the invitee can confirm they
/// are the right person (and use the "This is not me" path otherwise). Nothing
/// else about the person is exposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitationPreview {
    /// The opaque person key the invitation names.
    pub person_key: String,
    /// The person's display name, as recorded in the archive.
    pub display_name: String,
    /// The invitation lifecycle.
    pub state: ActivationState,
    /// True when the person already has an active account link. The welcome
    /// screen then explains that the account already exists instead of offering
    /// activation.
    pub person_linked: bool,
}

/// The result of one activation attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationOutcome {
    /// The opaque person key the invitation names.
    pub person_key: String,
    /// The person's display name.
    pub display_name: String,
    /// Always `accepted` after a successful activation.
    pub state: ActivationState,
    /// True when this account was already linked to the person before the call.
    pub already_linked: bool,
}

/// Errors raised by the welcome and activation operations.
///
/// No variant carries a token, a hash, an address, a name, or raw SQL text.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActivationError {
    /// The presented link does not match any invitation.
    #[error("this invitation link is not valid")]
    InvalidToken,
    /// The invitation expired before use.
    #[error("this invitation has expired")]
    Expired,
    /// A family administrator revoked the invitation.
    #[error("this invitation was revoked")]
    Revoked,
    /// The invitation was already accepted.
    #[error("this invitation was already accepted")]
    AlreadyAccepted,
    /// The person already has an active link to another account.
    #[error("this person is already linked to another account")]
    PersonAlreadyLinked,
    /// This account is already linked to a different person.
    #[error("this account is already linked to another person")]
    AccountLinkedElsewhere,
    /// The person row no longer exists.
    #[error("unknown person")]
    UnknownPerson,
    /// The reviewed-link operation failed.
    #[error("link operation failed")]
    Link(LinkError),
    /// The invitation operation failed.
    #[error("invitation operation failed")]
    Invitation(String),
    /// The database operation failed.
    #[error("activation database operation failed")]
    Database(String),
}

impl From<LinkError> for ActivationError {
    fn from(err: LinkError) -> Self {
        ActivationError::Link(err)
    }
}

impl From<InvitationError> for ActivationError {
    fn from(err: InvitationError) -> Self {
        ActivationError::Invitation(err.to_string())
    }
}

impl From<postgres::Error> for ActivationError {
    fn from(err: postgres::Error) -> Self {
        ActivationError::Database(err.to_string())
    }
}

/// One invitation row joined to its person.
struct TokenRow {
    invitation_id: i64,
    person_id: i64,
    status: String,
    expired: bool,
    person_key: String,
    display_name: String,
}

/// Reads the invitation and person for one presented token.
fn load_token(client: &mut Client, token: &str) -> Result<Option<TokenRow>, ActivationError> {
    let token_hash = hash_token(token);
    let row = client.query_opt(
        "SELECT invitation.invitation_id, invitation.person_id, invitation.status,
                (invitation.status = 'pending'
                 AND invitation.expires_at IS NOT NULL
                 AND invitation.expires_at <= now()),
                person.person_key, person.display_name
           FROM onboarding_invitation AS invitation
           JOIN person ON person.person_id = invitation.person_id
          WHERE invitation.token_hash = $1",
        &[&token_hash],
    )?;
    Ok(row.map(|row| TokenRow {
        invitation_id: row.get(0),
        person_id: row.get(1),
        status: row.get(2),
        expired: row.get(3),
        person_key: row.get(4),
        display_name: row.get(5),
    }))
}

/// Maps a stored invitation row to its effective state.
fn state_of(row: &TokenRow) -> ActivationState {
    match row.status.as_str() {
        "accepted" => ActivationState::Accepted,
        "revoked" => ActivationState::Revoked,
        "expired" => ActivationState::Expired,
        _ if row.expired => ActivationState::Expired,
        _ => ActivationState::Pending,
    }
}

/// Reports whether the person already has an active account link.
fn person_is_linked(client: &mut Client, person_id: i64) -> Result<bool, ActivationError> {
    let linked: bool = client
        .query_one(
            "SELECT EXISTS (
                 SELECT 1 FROM onboarding_person_account_link WHERE person_id = $1
             )",
            &[&person_id],
        )?
        .get(0);
    Ok(linked)
}

/// Marks one due pending invitation expired.
fn mark_expired(client: &mut Client, invitation_id: i64) -> Result<(), ActivationError> {
    client.execute(
        "UPDATE onboarding_invitation
            SET status = 'expired'
          WHERE invitation_id = $1 AND status = 'pending'",
        &[&invitation_id],
    )?;
    Ok(())
}

/// Reads the welcome-screen view of one invitation link.
///
/// The function issues SELECT statements only: it never consumes, expires, or
/// links anything. An invalid, unknown, or blank token is reported as
/// [`ActivationError::InvalidToken`] without revealing whether a person exists.
pub fn preview_invitation(
    client: &mut Client,
    token: &str,
) -> Result<InvitationPreview, ActivationError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(ActivationError::InvalidToken);
    }
    let Some(row) = load_token(client, token)? else {
        return Err(ActivationError::InvalidToken);
    };
    let state = state_of(&row);
    let person_linked = person_is_linked(client, row.person_id)?;
    Ok(InvitationPreview {
        person_key: row.person_key,
        display_name: row.display_name,
        state,
        person_linked,
    })
}

/// Accepts one invitation for the signed-in account.
///
/// The link is written for the invited person and this account only, and only
/// when no link exists yet. An existing link to a different person is refused as
/// [`ActivationError::AccountLinkedElsewhere`]; a person already linked to another
/// account is refused as [`ActivationError::PersonAlreadyLinked`]. Nothing is
/// replaced, merged, or deleted, and no role is granted.
///
/// When the invitation cannot be consumed after the link was written (a
/// concurrent use of the same link), exactly that new link is revoked, so a
/// failed activation never leaves an account attached to a person it did not
/// activate.
pub fn accept_invitation(
    client: &mut Client,
    token: &str,
    account: &IdentityAccount,
) -> Result<ActivationOutcome, ActivationError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(ActivationError::InvalidToken);
    }
    let Some(row) = load_token(client, token)? else {
        return Err(ActivationError::InvalidToken);
    };

    let outcome = |already_linked: bool| ActivationOutcome {
        person_key: row.person_key.clone(),
        display_name: row.display_name.clone(),
        state: ActivationState::Accepted,
        already_linked,
    };

    // An account already linked to another person can never accept this
    // invitation: a silent re-link would move a person between accounts.
    if let Some(existing) =
        link_for_account(client, account.account_id, &account.identity_fingerprint)?
    {
        if existing.person_id != row.person_id {
            return Err(ActivationError::AccountLinkedElsewhere);
        }
        // Already linked to this person: consume the invitation when it is still
        // usable and report an idempotent success.
        match state_of(&row) {
            ActivationState::Accepted => return Ok(outcome(true)),
            ActivationState::Pending => match accept(client, token) {
                Ok(_) | Err(InvitationError::AlreadyAccepted) => return Ok(outcome(true)),
                Err(InvitationError::Expired) => return Err(ActivationError::Expired),
                Err(InvitationError::InvalidToken) => return Err(ActivationError::InvalidToken),
                Err(err) => return Err(ActivationError::Invitation(err.to_string())),
            },
            ActivationState::Expired => return Err(ActivationError::Expired),
            ActivationState::Revoked => return Err(ActivationError::Revoked),
        }
    }

    match state_of(&row) {
        ActivationState::Accepted => return Err(ActivationError::AlreadyAccepted),
        ActivationState::Revoked => return Err(ActivationError::Revoked),
        ActivationState::Expired => {
            mark_expired(client, row.invitation_id)?;
            return Err(ActivationError::Expired);
        }
        ActivationState::Pending => {}
    }

    if person_is_linked(client, row.person_id)? {
        // A reviewed link or a previous acceptance owns this person; never replace
        // it with this caller's account.
        return Err(ActivationError::PersonAlreadyLinked);
    }

    let record = match link_reviewed(
        client,
        row.person_id,
        account,
        LinkDecision::Approve,
        "invitation",
    ) {
        Ok(Some(record)) => record,
        Ok(None) => return Err(ActivationError::Link(LinkError::Rejected)),
        Err(LinkError::AlreadyLinked) => return Err(ActivationError::PersonAlreadyLinked),
        Err(err) => return Err(ActivationError::Link(err)),
    };

    match accept(client, token) {
        Ok(_) => Ok(outcome(false)),
        Err(err) => {
            // The link was written for this call, so revoke exactly it. The
            // invitation stays in whatever state the failed consume left it.
            let _ = revoke_link(client, record.link_id, "invitation");
            Err(match err {
                InvitationError::Expired => ActivationError::Expired,
                InvitationError::AlreadyAccepted => ActivationError::AlreadyAccepted,
                InvitationError::InvalidToken => ActivationError::InvalidToken,
                other => ActivationError::Invitation(other.to_string()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{state_of, ActivationState, TokenRow};

    fn row(status: &str, expired: bool) -> TokenRow {
        TokenRow {
            invitation_id: 1,
            person_id: 1,
            status: status.to_string(),
            expired,
            person_key: "synthetic-person".to_string(),
            display_name: "Synthetic Person".to_string(),
        }
    }

    #[test]
    fn stored_status_maps_to_the_effective_state() {
        assert_eq!(state_of(&row("pending", false)), ActivationState::Pending);
        assert_eq!(state_of(&row("accepted", false)), ActivationState::Accepted);
        assert_eq!(state_of(&row("revoked", false)), ActivationState::Revoked);
        assert_eq!(state_of(&row("expired", false)), ActivationState::Expired);
        // A pending row past its expiry is reported as expired.
        assert_eq!(state_of(&row("pending", true)), ActivationState::Expired);
    }

    #[test]
    fn state_codes_are_stable() {
        assert_eq!(ActivationState::Pending.as_str(), "pending");
        assert_eq!(ActivationState::Expired.as_str(), "expired");
        assert_eq!(ActivationState::Revoked.as_str(), "revoked");
        assert_eq!(ActivationState::Accepted.as_str(), "accepted");
    }
}
