//! Invitation welcome and activation (PIR-006 follow-up).
//!
//! An invitation link is a bearer credential. [`preview_invitation`] reads the
//! intended person, the invitation state, and the link-request state for the
//! welcome screen without writing anything. [`accept_invitation`] consumes the
//! invitation and records a **request** to link the signed-in account to the
//! invited person.
//!
//! Possessing the token never approves a person link by itself. An administrator
//! makes the explicit reviewed-link decision through
//! [`decide_link_request`], which delegates to the existing [`link_reviewed`]
//! policy and its audit trail. An existing link is never replaced, and no role is
//! granted here (member provisioning lives in [`crate::onboarding::provisioning`]).
//!
//! No return value carries a token, a full address, or a credential.

use postgres::Client;
use thiserror::Error;

use crate::onboarding::account_map::IdentityAccount;
use crate::onboarding::invitation::{accept, hash_token, InvitationError};
use crate::onboarding::link::{
    link_for_account, link_reviewed, LinkDecision, LinkError, LinkRecord,
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

/// The reviewed-link request recorded for one invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRequestState {
    /// No request was recorded.
    None,
    /// An administrator has not decided yet.
    Pending,
    /// An administrator approved the reviewed link.
    Approved,
    /// An administrator rejected the reviewed link.
    Rejected,
}

impl LinkRequestState {
    /// The stable string code for this state.
    pub fn as_str(self) -> &'static str {
        match self {
            LinkRequestState::None => "none",
            LinkRequestState::Pending => "pending",
            LinkRequestState::Approved => "approved",
            LinkRequestState::Rejected => "rejected",
        }
    }

    fn from_db(value: &str) -> Self {
        match value {
            "pending" => LinkRequestState::Pending,
            "approved" => LinkRequestState::Approved,
            "rejected" => LinkRequestState::Rejected,
            _ => LinkRequestState::None,
        }
    }
}

/// The link outcome reported after an acceptance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// The account is linked to the invited person.
    Linked,
    /// The request is recorded and waits for an administrator's review.
    PendingReview,
}

impl LinkState {
    /// The stable string code for this state.
    pub fn as_str(self) -> &'static str {
        match self {
            LinkState::Linked => "linked",
            LinkState::PendingReview => "pending_review",
        }
    }
}

/// The welcome-screen view of one invitation link.
///
/// It carries the intended person's display name so the invitee can confirm they
/// are the right person, and the link-request state so a returning visitor sees
/// "pending review" or "linked" instead of an offer to accept again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitationPreview {
    /// The opaque person key the invitation names.
    pub person_key: String,
    /// The person's display name, as recorded in the archive.
    pub display_name: String,
    /// The invitation lifecycle.
    pub state: ActivationState,
    /// True when the person already has an active account link.
    pub person_linked: bool,
    /// The reviewed-link request recorded for this invitation.
    pub request_state: LinkRequestState,
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
    /// Whether the account is linked or the request waits for review.
    pub link_state: LinkState,
}

/// One pending reviewed-link request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRequestView {
    /// The request id.
    pub request_id: i64,
    /// The invitation that produced the request.
    pub invitation_id: i64,
    /// The opaque person key.
    pub person_key: String,
    /// The person's display name.
    pub display_name: String,
    /// The opaque identity account id.
    pub account_id: i64,
    /// The request time, rendered as text.
    pub requested_at: String,
}

/// The result of one administrative decision on a link request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRequestDecision {
    /// The decided request.
    pub request_id: i64,
    /// The opaque person key.
    pub person_key: String,
    /// The opaque identity account id the request belongs to.
    pub account_id: i64,
    /// True when the reviewed link now exists.
    pub linked: bool,
    /// True when the decision was already recorded with the same outcome.
    pub already_decided: bool,
}

/// Errors raised by the welcome, activation, and review operations.
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
    /// The invitation was already accepted by another account.
    #[error("this invitation was already accepted")]
    AlreadyAccepted,
    /// The person already has an active link to another account.
    #[error("this person is already linked to another account")]
    PersonAlreadyLinked,
    /// This account is already linked to a different person.
    #[error("this account is already linked to another person")]
    AccountLinkedElsewhere,
    /// An administrator rejected the link request.
    #[error("the link request was rejected")]
    LinkRejected,
    /// The person row no longer exists.
    #[error("unknown person")]
    UnknownPerson,
    /// No link request exists for the id.
    #[error("unknown link request")]
    UnknownLinkRequest,
    /// The request was already decided with a different outcome.
    #[error("the link request was already decided")]
    LinkRequestAlreadyDecided,
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

/// Reads the link-request row for one invitation, including its owner.
fn request_for_invitation(
    client: &mut Client,
    invitation_id: i64,
) -> Result<Option<(LinkRequestState, i64, String)>, ActivationError> {
    let row = client.query_opt(
        "SELECT status, account_id, identity_fingerprint
           FROM onboarding_link_request WHERE invitation_id = $1",
        &[&invitation_id],
    )?;
    Ok(row.map(|row| {
        (
            LinkRequestState::from_db(&row.get::<_, String>(0)),
            row.get(1),
            row.get(2),
        )
    }))
}

/// Reads the link-request state for one invitation.
fn request_state(
    client: &mut Client,
    invitation_id: i64,
) -> Result<LinkRequestState, ActivationError> {
    let row = client.query_opt(
        "SELECT status FROM onboarding_link_request WHERE invitation_id = $1",
        &[&invitation_id],
    )?;
    Ok(row
        .map(|row| LinkRequestState::from_db(&row.get::<_, String>(0)))
        .unwrap_or(LinkRequestState::None))
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
    let request_state = request_state(client, row.invitation_id)?;
    Ok(InvitationPreview {
        person_key: row.person_key,
        display_name: row.display_name,
        state,
        person_linked,
        request_state,
    })
}

/// Inserts the reviewed-link request for one invitation, idempotently.
///
/// A second acceptance of the same invitation returns the request recorded by the
/// first one, so a retry never creates a duplicate.
fn record_link_request(
    client: &mut Client,
    invitation_id: i64,
    person_id: i64,
    account: &IdentityAccount,
) -> Result<Option<LinkRequestState>, ActivationError> {
    client.execute(
        "INSERT INTO onboarding_link_request
             (invitation_id, person_id, account_id, identity_fingerprint, status)
         VALUES ($1, $2, $3, $4, 'pending')
         ON CONFLICT (invitation_id) DO NOTHING",
        &[
            &invitation_id,
            &person_id,
            &account.account_id,
            &account.identity_fingerprint,
        ],
    )?;
    // The unique invitation key decides. A conflicting row from a racing
    // acceptance of the same link belongs to another account and yields `None`.
    let row = client.query_opt(
        "SELECT status FROM onboarding_link_request
          WHERE invitation_id = $1 AND account_id = $2 AND identity_fingerprint = $3",
        &[
            &invitation_id,
            &account.account_id,
            &account.identity_fingerprint,
        ],
    )?;
    Ok(row.map(|row| LinkRequestState::from_db(&row.get::<_, String>(0))))
}

/// Marks one pending request approved after the reviewed link exists.
fn mark_request_approved(
    client: &mut Client,
    invitation_id: i64,
    actor: &str,
) -> Result<(), ActivationError> {
    client.execute(
        "UPDATE onboarding_link_request
            SET status = 'approved', decided_at = now(), decided_by = $2
          WHERE invitation_id = $1 AND status = 'pending'",
        &[&invitation_id, &actor],
    )?;
    Ok(())
}

/// Consumes the invitation when it is still pending.
fn consume_invitation(client: &mut Client, token: &str) -> Result<(), ActivationError> {
    match accept(client, token) {
        Ok(_) | Err(InvitationError::AlreadyAccepted) => Ok(()),
        Err(InvitationError::Expired) => Err(ActivationError::Expired),
        Err(InvitationError::InvalidToken) => Err(ActivationError::InvalidToken),
        Err(other) => Err(ActivationError::Invitation(other.to_string())),
    }
}

/// Accepts one invitation for the signed-in account.
///
/// The invitation is consumed and a reviewed-link **request** is recorded. The
/// link itself is written only by [`decide_link_request`]; possessing the token
/// never approves it. An account already linked to a different person is refused,
/// and an existing link is never replaced.
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
    let outcome = |link_state: LinkState| ActivationOutcome {
        person_key: row.person_key.clone(),
        display_name: row.display_name.clone(),
        state: ActivationState::Accepted,
        link_state,
    };

    // A link owned by this account decides before anything else: a second
    // person must never be attached to one account.
    if let Some(existing) =
        link_for_account(client, account.account_id, &account.identity_fingerprint)?
    {
        if existing.person_id != row.person_id {
            return Err(ActivationError::AccountLinkedElsewhere);
        }
        consume_invitation(client, token)?;
        mark_request_approved(client, row.invitation_id, "invitation")?;
        return Ok(outcome(LinkState::Linked));
    }

    match state_of(&row) {
        ActivationState::Revoked => return Err(ActivationError::Revoked),
        ActivationState::Expired => {
            mark_expired(client, row.invitation_id)?;
            return Err(ActivationError::Expired);
        }
        // A consuming retry reports the recorded request instead of failing, but
        // only for the account that recorded it.
        ActivationState::Accepted => {
            let Some((state, owner_id, owner_fingerprint)) =
                request_for_invitation(client, row.invitation_id)?
            else {
                return Err(ActivationError::AlreadyAccepted);
            };
            if owner_id != account.account_id || owner_fingerprint != account.identity_fingerprint {
                return Err(ActivationError::AlreadyAccepted);
            }
            return match state {
                LinkRequestState::Pending => Ok(outcome(LinkState::PendingReview)),
                LinkRequestState::Approved => match link_for_account(
                    client,
                    account.account_id,
                    &account.identity_fingerprint,
                )? {
                    Some(_) => Ok(outcome(LinkState::Linked)),
                    None => Err(ActivationError::PersonAlreadyLinked),
                },
                LinkRequestState::Rejected => Err(ActivationError::LinkRejected),
                LinkRequestState::None => Err(ActivationError::AlreadyAccepted),
            };
        }
        ActivationState::Pending => {}
    }

    if person_is_linked(client, row.person_id)? {
        // A reviewed link or a previous acceptance owns this person; never
        // replace it with this caller's account.
        return Err(ActivationError::PersonAlreadyLinked);
    }

    let recorded = record_link_request(client, row.invitation_id, row.person_id, account)?;
    match recorded {
        None => return Err(ActivationError::AlreadyAccepted),
        Some(LinkRequestState::Rejected) => return Err(ActivationError::LinkRejected),
        Some(LinkRequestState::Approved) => {
            consume_invitation(client, token)?;
            return Ok(outcome(LinkState::Linked));
        }
        Some(LinkRequestState::Pending | LinkRequestState::None) => {}
    }

    consume_invitation(client, token)?;
    Ok(outcome(LinkState::PendingReview))
}

/// Lists pending reviewed-link requests for an administrator.
pub fn pending_link_requests(client: &mut Client) -> Result<Vec<LinkRequestView>, ActivationError> {
    let rows = client.query(
        "SELECT request.request_id, request.invitation_id,
                person.person_key, person.display_name,
                request.account_id, request.requested_at::text
           FROM onboarding_link_request AS request
           JOIN person ON person.person_id = request.person_id
          WHERE request.status = 'pending'
          ORDER BY request.requested_at ASC, request.request_id ASC",
        &[],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| LinkRequestView {
            request_id: row.get(0),
            invitation_id: row.get(1),
            person_key: row.get(2),
            display_name: row.get(3),
            account_id: row.get(4),
            requested_at: row.get(5),
        })
        .collect())
}

/// Applies one administrative decision to a reviewed-link request.
///
/// `Approve` delegates to the existing [`link_reviewed`] policy, so the same
/// audit row is written as any other reviewed link. `Reject` records the decision
/// without a link. An existing link is never replaced: a person linked to another
/// account is refused, and a repeat decision with the same outcome is reported as
/// already decided.
pub fn decide_link_request(
    client: &mut Client,
    request_id: i64,
    decision: LinkDecision,
    actor: &str,
) -> Result<LinkRequestDecision, ActivationError> {
    let actor = actor.trim();
    if actor.is_empty() {
        return Err(ActivationError::Database("a reviewer is required".into()));
    }
    let row = client
        .query_opt(
            "SELECT request.invitation_id, request.person_id, request.account_id,
                    request.identity_fingerprint, request.status, person.person_key
               FROM onboarding_link_request AS request
               JOIN person ON person.person_id = request.person_id
              WHERE request.request_id = $1",
            &[&request_id],
        )?
        .ok_or(ActivationError::UnknownLinkRequest)?;
    let invitation_id: i64 = row.get(0);
    let person_id: i64 = row.get(1);
    let account = IdentityAccount {
        account_id: row.get(2),
        identity_fingerprint: row.get(3),
    };
    let stored_status: String = row.get(4);
    let person_key: String = row.get(5);

    match decision {
        LinkDecision::Reject => {
            let affected = client.execute(
                "UPDATE onboarding_link_request
                    SET status = 'rejected', decided_at = now(), decided_by = $2
                  WHERE request_id = $1 AND status = 'pending'",
                &[&request_id, &actor],
            )?;
            if affected == 0 {
                return if stored_status == "rejected" {
                    Ok(LinkRequestDecision {
                        request_id,
                        person_key,
                        account_id: account.account_id,
                        linked: false,
                        already_decided: true,
                    })
                } else {
                    Err(ActivationError::LinkRequestAlreadyDecided)
                };
            }
            Ok(LinkRequestDecision {
                request_id,
                person_key,
                account_id: account.account_id,
                linked: false,
                already_decided: false,
            })
        }
        LinkDecision::Approve => {
            if stored_status == "approved" {
                return Ok(LinkRequestDecision {
                    request_id,
                    person_key,
                    account_id: account.account_id,
                    linked: true,
                    already_decided: true,
                });
            }
            if stored_status == "rejected" {
                return Err(ActivationError::LinkRequestAlreadyDecided);
            }
            let link: LinkRecord =
                match link_reviewed(client, person_id, &account, LinkDecision::Approve, actor) {
                    Ok(Some(record)) => record,
                    Ok(None) => return Err(ActivationError::Link(LinkError::Rejected)),
                    Err(LinkError::AlreadyLinked) => {
                        // Idempotent only when this same account already owns the
                        // person; anything else must not be reported as approved.
                        match link_for_account(
                            client,
                            account.account_id,
                            &account.identity_fingerprint,
                        )? {
                            Some(existing) if existing.person_id == person_id => existing,
                            _ => return Err(ActivationError::PersonAlreadyLinked),
                        }
                    }
                    Err(err) => return Err(ActivationError::Link(err)),
                };
            mark_request_approved(client, invitation_id, actor)?;
            Ok(LinkRequestDecision {
                request_id,
                person_key,
                account_id: account.account_id,
                linked: link.person_id == person_id,
                already_decided: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{state_of, ActivationState, LinkRequestState, TokenRow};

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
        assert_eq!(LinkRequestState::None.as_str(), "none");
        assert_eq!(LinkRequestState::Pending.as_str(), "pending");
        assert_eq!(LinkRequestState::Approved.as_str(), "approved");
        assert_eq!(LinkRequestState::Rejected.as_str(), "rejected");
    }
}
