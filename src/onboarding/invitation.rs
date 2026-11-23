//! Invitation tokens for onboarding (USR-003B).
//!
//! This module issues, accepts, revokes, and expires onboarding invitations.
//! A token is 32 cryptographically random bytes encoded as lowercase hex. The
//! plaintext token is returned to the caller exactly once, at creation. Only
//! the lowercase hex SHA-256 hash is stored, in `token_hash`. The database never
//! holds a plaintext token or a password.
//!
//! Operational output is privacy safe: no error variant carries a token, a hash,
//! a person name, or raw SQL text.

use std::fmt::Write as _;
use std::time::Duration;

use postgres::Client;
use ring::rand::SecureRandom;
use thiserror::Error;

use crate::onboarding::account_map::IdentityAccount;
use crate::onboarding::link::{link_reviewed_in, LinkDecision, LinkError, LinkRecord};

/// A created (or reused) invitation.
///
/// `token` is `Some` only when this call generated a new token. A reused
/// pending invitation returns `token: None`.
#[derive(Debug, Clone)]
pub struct IssuedInvitation {
    pub invitation_id: i64,
    pub person_id: i64,
    pub token: Option<String>,
    /// SQL-computed expiry rendered as text (`expires_at::text`).
    pub expires_at: String,
}

/// Failure modes of the invitation operations.
///
/// No variant carries a token, a hash, or raw database text.
#[derive(Debug, Error)]
pub enum InvitationError {
    #[error("unknown person")]
    UnknownPerson,
    #[error("invalid invitation token")]
    InvalidToken,
    #[error("invitation has expired")]
    Expired,
    #[error("invitation already accepted")]
    AlreadyAccepted,
    /// The invited person or the redeeming account already has an active link.
    #[error("person or account is already linked")]
    AlreadyLinked,
    /// The self-reported name is blank or too long.
    #[error("a name of 1 to 100 characters is required")]
    InvalidName,
    #[error("database error")]
    Database(String),
}

impl From<postgres::Error> for InvitationError {
    fn from(err: postgres::Error) -> Self {
        // The detail is kept for operators but never contains a token: the SQL
        // binds only hashes and ids.
        InvitationError::Database(err.to_string())
    }
}

/// Generate 32 random bytes as lowercase hex.
fn generate_token() -> Result<String, InvitationError> {
    let rng = ring::rand::SystemRandom::new();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes)
        .map_err(|_| InvitationError::Database("random source unavailable".to_string()))?;
    Ok(to_hex(&bytes))
}

/// Lowercase hex SHA-256 of the presented token.
pub fn hash_token(token: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, token.as_bytes());
    to_hex(digest.as_ref())
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing into a String cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Create an invitation for `person_id` valid for `ttl`.
///
/// Confirms the person exists inside the transaction. When a pending, unexpired
/// invitation already exists for the person, that row is returned with
/// `token: None` and no second row is inserted (idempotent create).
pub fn create(
    client: &mut Client,
    person_id: i64,
    ttl: Duration,
    actor: &str,
) -> Result<IssuedInvitation, InvitationError> {
    let mut tx = client.transaction()?;

    let exists: bool = tx
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM person WHERE person_id = $1)",
            &[&person_id],
        )?
        .get(0);
    if !exists {
        return Err(InvitationError::UnknownPerson);
    }

    // An expired pending row must not keep the person in an invited state.
    // Transition it inside the same transaction before looking for a live row.
    tx.execute(
        "UPDATE onboarding_invitation
            SET status = 'expired'
          WHERE person_id = $1
            AND status = 'pending'
            AND expires_at IS NOT NULL
            AND expires_at <= now()",
        &[&person_id],
    )?;

    let existing = tx.query_opt(
        "SELECT invitation_id, person_id, expires_at::text
           FROM onboarding_invitation
          WHERE person_id = $1
            AND status = 'pending'
            AND expires_at IS NOT NULL
            AND expires_at > now()
          ORDER BY created_at DESC, invitation_id DESC
          LIMIT 1",
        &[&person_id],
    )?;
    if let Some(row) = existing {
        let invitation = IssuedInvitation {
            invitation_id: row.get(0),
            person_id: row.get(1),
            token: None,
            expires_at: row.get(2),
        };
        tx.commit()?;
        return Ok(invitation);
    }

    let token = generate_token()?;
    let token_hash = hash_token(&token);

    let row = tx.query_one(
        "INSERT INTO onboarding_invitation
             (person_id, token_hash, expires_at, status, created_by)
         VALUES ($1, $2, now() + make_interval(secs => $3), 'pending', $4)
         RETURNING invitation_id, person_id, expires_at::text",
        &[&person_id, &token_hash, &(ttl.as_secs_f64()), &actor],
    )?;

    let invitation = IssuedInvitation {
        invitation_id: row.get(0),
        person_id: row.get(1),
        token: Some(token),
        expires_at: row.get(2),
    };
    tx.commit()?;
    Ok(invitation)
}

/// Accept an invitation by presenting its plaintext `token`.
///
/// Marks the matching pending, unexpired row accepted with `used_at = now()`
/// in one atomic update and returns its invitation id. A missing, used, or
/// revoked token returns [`InvitationError::InvalidToken`]; an expired pending
/// token is marked `expired` and returns [`InvitationError::Expired`].
pub fn accept(client: &mut Client, token: &str) -> Result<i64, InvitationError> {
    redeem_in(client.transaction()?, token, None)
}

/// Redeems a shared invitation link for a signed-in `account`.
///
/// Consumes the token exactly like [`accept`] and records the claiming account
/// (opaque id + fingerprint) and the member's self-reported name. A claim is
/// not a link: an administrator confirms or rejects it with [`decide_claim`].
/// The token is left untouched when the person or the account already has an
/// active link, so a mistaken redemption does not burn the invitation.
pub fn claim_invitation(
    client: &mut Client,
    token: &str,
    account: &IdentityAccount,
    claimed_name: &str,
) -> Result<i64, InvitationError> {
    let name = claimed_name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(InvitationError::InvalidName);
    }
    let mut tx = client.transaction()?;
    let linked: bool = tx
        .query_one(
            "SELECT EXISTS (
                 SELECT 1 FROM onboarding_person_account_link l
                  WHERE l.account_id = $2
                     OR l.person_id = (SELECT person_id FROM onboarding_invitation
                                        WHERE token_hash = $1))",
            &[&hash_token(token), &account.account_id],
        )?
        .get(0);
    if linked {
        return Err(InvitationError::AlreadyLinked);
    }
    redeem_in(tx, token, Some((account, name)))
}

/// Consumes a pending, unexpired token inside `tx` and commits. With a claim,
/// the claiming account and name are stored on the same row.
fn redeem_in(
    mut tx: postgres::Transaction<'_>,
    token: &str,
    claim: Option<(&IdentityAccount, &str)>,
) -> Result<i64, InvitationError> {
    let token_hash = hash_token(token);
    let account_id = claim.map(|(account, _)| account.account_id);
    let fingerprint = claim.map(|(account, _)| account.identity_fingerprint.as_str());
    let name = claim.map(|(_, name)| name);

    let accepted = tx.query_opt(
        "UPDATE onboarding_invitation
            SET status = 'accepted',
                used_at = now(),
                accepted_at = now(),
                account_id = COALESCE($2, account_id),
                claimed_fingerprint = COALESCE($3, claimed_fingerprint),
                claimed_name = COALESCE($4, claimed_name)
          WHERE token_hash = $1
            AND status = 'pending'
            AND expires_at IS NOT NULL
            AND expires_at > now()
          RETURNING invitation_id",
        &[&token_hash, &account_id, &fingerprint, &name],
    )?;
    if let Some(row) = accepted {
        let invitation_id: i64 = row.get(0);
        tx.commit()?;
        return Ok(invitation_id);
    }

    // The update matched nothing. Look at the pre-state to classify the
    // failure without leaking database detail.
    let prior = tx.query_opt(
        "SELECT status, (expires_at IS NOT NULL AND expires_at <= now())
           FROM onboarding_invitation
          WHERE token_hash = $1",
        &[&token_hash],
    )?;

    match prior {
        None => {
            tx.rollback()?;
            Err(InvitationError::InvalidToken)
        }
        Some(row) => {
            let status: String = row.get(0);
            let is_expired: bool = row.get(1);
            if status == "accepted" {
                tx.rollback()?;
                Err(InvitationError::AlreadyAccepted)
            } else if status == "pending" && is_expired {
                // Mark the expired pending row as expired before failing.
                tx.execute(
                    "UPDATE onboarding_invitation
                        SET status = 'expired'
                      WHERE token_hash = $1
                        AND status = 'pending'",
                    &[&token_hash],
                )?;
                tx.commit()?;
                Err(InvitationError::Expired)
            } else {
                tx.rollback()?;
                Err(InvitationError::InvalidToken)
            }
        }
    }
}

/// Transition a pending invitation to `revoked`.
///
/// Any non-pending state fails safely with [`InvitationError::InvalidToken`]
/// and no other row is touched.
pub fn revoke(client: &mut Client, invitation_id: i64) -> Result<(), InvitationError> {
    let affected = client.execute(
        "UPDATE onboarding_invitation
            SET status = 'revoked'
          WHERE invitation_id = $1
            AND status = 'pending'",
        &[&invitation_id],
    )?;
    if affected == 1 {
        Ok(())
    } else {
        Err(InvitationError::InvalidToken)
    }
}

/// One redeemed invitation waiting for an administrator's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitationClaim {
    pub invitation_id: i64,
    pub person_key: String,
    pub person_label: String,
    /// The opaque identity account that redeemed the link.
    pub account_id: i64,
    /// The member's self-reported name. It is not verified.
    pub claimed_name: String,
    pub claimed_at: String,
}

/// Lists redeemed invitations that no administrator has decided yet, oldest
/// first.
pub fn pending_claims(client: &mut Client) -> Result<Vec<InvitationClaim>, InvitationError> {
    let rows = client.query(
        "SELECT i.invitation_id, p.person_key, p.display_name, i.account_id,
                COALESCE(i.claimed_name, ''), i.accepted_at::text
           FROM onboarding_invitation i
           JOIN person p ON p.person_id = i.person_id
          WHERE i.status = 'accepted'
            AND i.account_id IS NOT NULL
            AND i.claim_decision IS NULL
          ORDER BY i.accepted_at, i.invitation_id",
        &[],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| InvitationClaim {
            invitation_id: row.get(0),
            person_key: row.get(1),
            person_label: row.get(2),
            account_id: row.get(3),
            claimed_name: row.get(4),
            claimed_at: row.get(5),
        })
        .collect())
}

/// Confirms or rejects one open claim.
///
/// `Approve` writes the reviewed account-to-person link and its audit row;
/// `Reject` writes only a `rejected` audit row. Either way the claim is marked
/// decided in the same transaction, so a claim is decided at most once and a
/// failed link (for example, an account linked meanwhile) leaves it open. An
/// unknown or already decided claim returns [`LinkError::UnknownLink`].
pub fn decide_claim(
    client: &mut Client,
    invitation_id: i64,
    decision: LinkDecision,
    reviewer: &str,
) -> Result<Option<LinkRecord>, LinkError> {
    let db = |err: postgres::Error| LinkError::Database(err.to_string());
    let mut tx = client.transaction().map_err(db)?;
    let Some(row) = tx
        .query_opt(
            "SELECT person_id, account_id, claimed_fingerprint
               FROM onboarding_invitation
              WHERE invitation_id = $1
                AND status = 'accepted'
                AND account_id IS NOT NULL
                AND claimed_fingerprint IS NOT NULL
                AND claim_decision IS NULL
              FOR UPDATE",
            &[&invitation_id],
        )
        .map_err(db)?
    else {
        return Err(LinkError::UnknownLink);
    };
    let person_id: i64 = row.get(0);
    let account = IdentityAccount {
        account_id: row.get(1),
        identity_fingerprint: row.get(2),
    };
    let record = link_reviewed_in(&mut tx, person_id, &account, decision, reviewer)?;
    tx.execute(
        "UPDATE onboarding_invitation
            SET claim_decision = $2, claim_decided_by = $3, claim_decided_at = now()
          WHERE invitation_id = $1",
        &[&invitation_id, &decision.as_str(), &reviewer.trim()],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(record)
}

/// Mark pending invitations whose `expires_at <= now()` as `expired`.
///
/// Returns the number of rows updated.
pub fn expire_due(client: &mut Client) -> Result<u64, InvitationError> {
    let affected = client.execute(
        "UPDATE onboarding_invitation
            SET status = 'expired'
          WHERE status = 'pending'
            AND expires_at IS NOT NULL
            AND expires_at <= now()",
        &[],
    )?;
    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::{hash_token, to_hex};

    #[test]
    fn token_is_lowercase_hex_of_expected_length() {
        let hex = to_hex(&[0x00, 0x0f, 0xa5, 0xff]);
        assert_eq!(hex, "000fa5ff");
    }

    #[test]
    fn hash_is_lowercase_hex_and_differs_from_input() {
        let token = "deadbeef";
        let hash = hash_token(token);
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash, hash.to_lowercase());
        assert_ne!(hash, token);
    }
}
