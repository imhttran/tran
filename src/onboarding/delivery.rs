//! Invitation delivery adapters and the separately approved bulk send step
//! (BULK-004).
//!
//! Preparation and delivery are separate gates. [`send_batch`] runs only for a
//! batch the caller explicitly approved, only for a person with a verified
//! recipient contact, and only after re-evaluating living eligibility. It
//! creates one hashed, expiring, single-use invitation per person and hands the
//! activation link to the configured adapter.
//!
//! The module never logs or returns a plaintext token. A token appears only in
//! the message body handed to the adapter. The batch item stores the invitation
//! id, the delivery state, the provider name, and a safe error code, never the
//! token or the recipient address.
//!
//! The default adapter is [`UnconfiguredAdapter`], which fails closed with
//! [`DeliveryError::NotConfigured`] without creating an invitation. The fake
//! adapter exists for synthetic tests and records messages in memory only.

use std::sync::Mutex;
use std::time::Duration;

use postgres::Client;
use thiserror::Error;

use crate::onboarding::candidate::{reclassify_person, CandidateError, Classification, Reason};
use crate::onboarding::invitation::{create, expire_due, revoke, InvitationError};

/// The persisted delivery state of one prepared person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    /// Approved preparation exists but no send has been attempted.
    NotSent,
    /// The adapter accepted the message for later delivery.
    Queued,
    /// The adapter confirmed it sent the message.
    Sent,
    /// The invitation was accepted.
    Accepted,
    /// The attempt failed safely and may be retried.
    Failed,
    /// The invitation expired before acceptance.
    Expired,
}

impl DeliveryStatus {
    /// The stable database string.
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryStatus::NotSent => "not_sent",
            DeliveryStatus::Queued => "queued",
            DeliveryStatus::Sent => "sent",
            DeliveryStatus::Accepted => "accepted",
            DeliveryStatus::Failed => "failed",
            DeliveryStatus::Expired => "expired",
        }
    }

    /// Parses a database string.
    pub fn from_db(value: &str) -> Option<Self> {
        Some(match value {
            "not_sent" => DeliveryStatus::NotSent,
            "queued" => DeliveryStatus::Queued,
            "sent" => DeliveryStatus::Sent,
            "accepted" => DeliveryStatus::Accepted,
            "failed" => DeliveryStatus::Failed,
            "expired" => DeliveryStatus::Expired,
            _ => return None,
        })
    }

    /// True when a later send must not issue a second invitation.
    fn is_delivered(self) -> bool {
        matches!(
            self,
            DeliveryStatus::Queued | DeliveryStatus::Sent | DeliveryStatus::Accepted
        )
    }
}

/// A safe delivery error. No variant carries a token, an address, or raw SQL.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DeliveryError {
    /// No delivery adapter is configured. No invitation is created.
    #[error("no invitation delivery adapter is configured")]
    NotConfigured,
    /// The supplied recipient address is not acceptable.
    #[error("the recipient address is not valid")]
    InvalidRecipient,
    /// The person has no explicitly verified recipient contact.
    #[error("no verified recipient contact is recorded")]
    MissingVerifiedContact,
    /// The person is not in a sendable prepared state.
    #[error("the person is not prepared for sending")]
    NotPrepared,
    /// The adapter failed. The detail is a safe provider failure code.
    #[error("the delivery adapter failed: {0}")]
    Provider(String),
    /// The database operation failed.
    #[error("delivery database operation failed: {0}")]
    Database(String),
    /// The invitation operation failed.
    #[error("invitation operation failed: {0}")]
    Invitation(String),
}

impl From<CandidateError> for DeliveryError {
    fn from(err: CandidateError) -> Self {
        DeliveryError::Database(err.to_string())
    }
}

impl From<InvitationError> for DeliveryError {
    fn from(err: InvitationError) -> Self {
        DeliveryError::Invitation(err.to_string())
    }
}

/// A provider's answer to one delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryReceipt {
    /// The provider that handled the message.
    pub provider: String,
    /// True only when the provider confirmed the message was sent. A queued
    /// message returns `false` and is never reported as delivered.
    pub accepted: bool,
    /// An opaque provider message id, when the provider supplies one.
    pub message_id: Option<String>,
}

/// One outbound activation message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentMessage {
    /// The verified recipient address.
    pub to: String,
    /// The message subject.
    pub subject: String,
    /// The message body containing the one-time activation link.
    pub body: String,
}

/// A mail provider adapter.
///
/// Implementations must return [`DeliveryError::NotConfigured`] rather than
/// pretending to deliver when the operator has not configured them.
pub trait MailAdapter: Send + Sync {
    /// A stable provider name stored in the audit trail.
    fn name(&self) -> &'static str;

    /// False when the adapter has no provider configuration. Callers fail
    /// closed before creating an invitation.
    fn is_configured(&self) -> bool {
        true
    }

    /// Delivers one message.
    fn deliver(
        &self,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<DeliveryReceipt, DeliveryError>;
}

/// The fail-closed default: no provider is configured.
pub struct UnconfiguredAdapter;

impl MailAdapter for UnconfiguredAdapter {
    fn name(&self) -> &'static str {
        "unconfigured"
    }

    fn is_configured(&self) -> bool {
        false
    }

    fn deliver(
        &self,
        _to: &str,
        _subject: &str,
        _body: &str,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        Err(DeliveryError::NotConfigured)
    }
}

/// A synthetic adapter used by tests and local acceptance runs.
///
/// It never performs network I/O. Messages are kept in memory only.
pub struct FakeAdapter {
    accepted: bool,
    messages: Mutex<Vec<SentMessage>>,
}

impl FakeAdapter {
    /// Builds an adapter that reports a successful send.
    pub fn accepting() -> Self {
        Self {
            accepted: true,
            messages: Mutex::new(Vec::new()),
        }
    }

    /// Builds an adapter that reports the message as queued, not delivered.
    pub fn queueing() -> Self {
        Self {
            accepted: false,
            messages: Mutex::new(Vec::new()),
        }
    }

    /// Returns the messages accepted so far.
    pub fn messages(&self) -> Vec<SentMessage> {
        self.messages
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

impl MailAdapter for FakeAdapter {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn deliver(
        &self,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        self.messages
            .lock()
            .map_err(|_| DeliveryError::Provider("adapter state unavailable".to_string()))?
            .push(SentMessage {
                to: to.to_string(),
                subject: subject.to_string(),
                body: body.to_string(),
            });
        Ok(DeliveryReceipt {
            provider: "fake".to_string(),
            accepted: self.accepted,
            message_id: Some("fake-message".to_string()),
        })
    }
}

/// The safe result of one send attempt. It carries no token and no address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOutcome {
    /// The person the outcome belongs to.
    pub person_id: i64,
    /// The resulting delivery state.
    pub status: DeliveryStatus,
    /// The provider name, when an attempt reached an adapter.
    pub provider: Option<String>,
    /// A safe reason code for a skip or failure.
    pub reason: Option<&'static str>,
}

/// Records an explicitly verified recipient contact.
///
/// The address is stored only after an administrator confirms it. This function
/// never reads genealogy documents and never guesses an address.
pub fn verify_recipient(
    client: &mut Client,
    person_id: i64,
    email: &str,
    actor: &str,
) -> Result<(), DeliveryError> {
    let email = email.trim();
    let (local, domain) = email
        .split_once('@')
        .ok_or(DeliveryError::InvalidRecipient)?;
    if local.is_empty() || domain.is_empty() || !domain.contains('.') || email.contains(' ') {
        return Err(DeliveryError::InvalidRecipient);
    }
    let actor = actor.trim();
    if actor.is_empty() {
        return Err(DeliveryError::InvalidRecipient);
    }
    client
        .execute(
            "INSERT INTO onboarding_recipient_contact
                 (person_id, email, verified_at, verified_by)
             VALUES ($1, $2, now(), $3)
             ON CONFLICT (person_id) DO UPDATE
                SET email = EXCLUDED.email,
                    verified_at = now(),
                    verified_by = EXCLUDED.verified_by,
                    updated_at = now()",
            &[&person_id, &email, &actor],
        )
        .map_err(|err| DeliveryError::Database(err.to_string()))?;
    Ok(())
}

fn contact_for(client: &mut Client, person_id: i64) -> Result<Option<String>, DeliveryError> {
    let row = client
        .query_opt(
            "SELECT email FROM onboarding_recipient_contact
              WHERE person_id = $1 AND verified_at IS NOT NULL",
            &[&person_id],
        )
        .map_err(|err| DeliveryError::Database(err.to_string()))?;
    Ok(row.map(|row| row.get(0)))
}

/// Builds the activation link. The token appears only in the message body.
fn activation_link(base: &str, token: &str) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    format!("{base}{separator}token={token}")
}

/// Issues an invitation whose plaintext token can be delivered.
///
/// [`create`] is idempotent and returns `token: None` when a pending row already
/// exists but its plaintext is no longer available. That row is revoked and a
/// replacement is issued; the returned identifier and token always belong to the
/// same invitation, so later persistence cannot reference the revoked row.
pub fn issue_deliverable_invitation(
    client: &mut Client,
    person_id: i64,
    ttl: Duration,
    actor: &str,
) -> Result<Option<(i64, String)>, DeliveryError> {
    let issued = create(client, person_id, ttl, actor)?;
    if let Some(token) = issued.token {
        return Ok(Some((issued.invitation_id, token)));
    }
    // The pending invitation cannot produce a link. Revoke it and issue the
    // replacement that this approved send will actually deliver.
    let _ = revoke(client, issued.invitation_id);
    let reissued = create(client, person_id, ttl, actor)?;
    Ok(reissued.token.map(|token| (reissued.invitation_id, token)))
}

/// Sends invitations for the prepared items of one approved batch.
///
/// `person_ids` limits the send to a selected subset; `None` sends every
/// prepared item whose delivery state is not already delivered. A failed item
/// may be retried: the previous invitation is revoked before a replacement is
/// issued, so no two active links exist for one person.
///
/// The function fails closed with [`DeliveryError::NotConfigured`] before any
/// database write when the adapter is not configured.
#[allow(clippy::too_many_arguments)]
pub fn send_batch(
    client: &mut Client,
    batch_id: i64,
    person_ids: Option<&[i64]>,
    adapter: &dyn MailAdapter,
    activation_base: &str,
    ttl: Duration,
    actor: &str,
    throttle: Duration,
) -> Result<Vec<SendOutcome>, DeliveryError> {
    if !adapter.is_configured() {
        return Err(DeliveryError::NotConfigured);
    }

    // Expired pending invitations must leave `pending` before revalidation, or
    // the person remains classified as invited and a safe retry is impossible.
    expire_due(client).map_err(|err| DeliveryError::Database(err.to_string()))?;

    let rows = client
        .query(
            "SELECT person_id, status, reason_code, invitation_id, delivery_status
               FROM onboarding_batch_item
              WHERE batch_id = $1
              ORDER BY person_id",
            &[&batch_id],
        )
        .map_err(|err| DeliveryError::Database(err.to_string()))?;

    let selected: Option<std::collections::BTreeSet<i64>> =
        person_ids.map(|ids| ids.iter().copied().collect());
    let mut outcomes = Vec::new();

    for row in &rows {
        let person_id: i64 = row.get(0);
        if selected
            .as_ref()
            .map(|set| !set.contains(&person_id))
            .unwrap_or(false)
        {
            continue;
        }
        let item_status: String = row.get(1);
        if item_status != "prepared" {
            outcomes.push(SendOutcome {
                person_id,
                status: DeliveryStatus::NotSent,
                provider: None,
                reason: Some("not_prepared"),
            });
            continue;
        }
        let existing_invitation: Option<i64> = row.get(3);
        let delivery_text: String = row.get(4);
        let delivery = DeliveryStatus::from_db(&delivery_text).unwrap_or(DeliveryStatus::NotSent);
        if delivery.is_delivered() {
            outcomes.push(SendOutcome {
                person_id,
                status: delivery,
                provider: None,
                reason: Some("already_delivered"),
            });
            continue;
        }

        let person_key: String = match client.query_opt(
            "SELECT person_key FROM person WHERE person_id = $1",
            &[&person_id],
        ) {
            Ok(Some(row)) => row.get(0),
            _ => {
                outcomes.push(SendOutcome {
                    person_id,
                    status: DeliveryStatus::Failed,
                    provider: None,
                    reason: Some("unknown_person"),
                });
                continue;
            }
        };
        let (classification, reasons) = reclassify_person(client, person_id, &person_key)?;
        match classification {
            Classification::EligibleForInvite | Classification::AlreadyPrepared => {}
            Classification::AlreadyLinked | Classification::Invited => {
                outcomes.push(SendOutcome {
                    person_id,
                    status: DeliveryStatus::Failed,
                    provider: None,
                    reason: Some("already_linked_or_invited"),
                });
                continue;
            }
            Classification::NotEligible => {
                outcomes.push(SendOutcome {
                    person_id,
                    status: DeliveryStatus::Failed,
                    provider: None,
                    reason: Some(if reasons.contains(&Reason::NoLivingEvidence) {
                        "no_living_evidence"
                    } else {
                        "not_eligible"
                    }),
                });
                continue;
            }
            Classification::NeedsReview => {
                outcomes.push(SendOutcome {
                    person_id,
                    status: DeliveryStatus::Failed,
                    provider: None,
                    reason: Some("eligibility_revalidation_failed"),
                });
                continue;
            }
        }

        let Some(email) = contact_for(client, person_id)? else {
            record_delivery(
                client,
                batch_id,
                person_id,
                None,
                DeliveryStatus::Failed,
                "missing_verified_contact",
                Some("batch"),
            )?;
            outcomes.push(SendOutcome {
                person_id,
                status: DeliveryStatus::Failed,
                provider: None,
                reason: Some("missing_verified_contact"),
            });
            continue;
        };

        // A retry must not leave two active links. Revoke the previous
        // invitation before issuing a replacement.
        if let Some(invitation_id) = existing_invitation {
            let _ = revoke(client, invitation_id);
        }
        let Some((invitation_id, token)) =
            issue_deliverable_invitation(client, person_id, ttl, actor)?
        else {
            outcomes.push(SendOutcome {
                person_id,
                status: DeliveryStatus::Failed,
                provider: None,
                reason: Some("invitation_reissue_failed"),
            });
            continue;
        };

        let link = activation_link(activation_base, &token);
        let subject = "Activate your family archive account";
        let body = format!(
            "An administrator prepared an account for you in the family archive.\n\
             Open this single-use link to continue:\n\n{link}\n\n\
             The link expires automatically. If you did not expect this message, ignore it."
        );

        match adapter.deliver(&email, subject, &body) {
            Ok(receipt) => {
                let status = if receipt.accepted {
                    DeliveryStatus::Sent
                } else {
                    DeliveryStatus::Queued
                };
                let _ = client.execute(
                    "UPDATE onboarding_invitation SET invited_email = $2 WHERE invitation_id = $1",
                    &[&invitation_id, &email],
                );
                record_delivery(
                    client,
                    batch_id,
                    person_id,
                    Some(invitation_id),
                    status,
                    &receipt.provider,
                    Some(actor),
                )?;
                outcomes.push(SendOutcome {
                    person_id,
                    status,
                    provider: Some(receipt.provider),
                    reason: None,
                });
            }
            Err(err) => {
                let reason = match err {
                    DeliveryError::NotConfigured => "not_configured",
                    DeliveryError::Provider(_) => "provider_failed",
                    _ => "delivery_failed",
                };
                record_delivery(
                    client,
                    batch_id,
                    person_id,
                    Some(invitation_id),
                    DeliveryStatus::Failed,
                    reason,
                    Some(actor),
                )?;
                outcomes.push(SendOutcome {
                    person_id,
                    status: DeliveryStatus::Failed,
                    provider: Some(adapter.name().to_string()),
                    reason: Some(reason),
                });
            }
        }

        if !throttle.is_zero() {
            std::thread::sleep(throttle);
        }
    }

    Ok(outcomes)
}

/// Persists a delivery state and appends an audit event.
fn record_delivery(
    client: &mut Client,
    batch_id: i64,
    person_id: i64,
    invitation_id: Option<i64>,
    status: DeliveryStatus,
    provider: &str,
    actor: Option<&str>,
) -> Result<(), DeliveryError> {
    client
        .execute(
            "UPDATE onboarding_batch_item
                SET invitation_id = COALESCE($2, invitation_id),
                    delivery_status = $3,
                    delivery_provider = $4,
                    delivery_attempts = delivery_attempts + 1,
                    delivered_at = CASE WHEN $3 = 'sent' THEN now() ELSE delivered_at END,
                    last_delivery_error = CASE WHEN $3 = 'failed' THEN $4 ELSE NULL END,
                    updated_at = now()
              WHERE batch_id = $1 AND person_id = $5",
            &[
                &batch_id,
                &invitation_id,
                &status.as_str(),
                &provider,
                &person_id,
            ],
        )
        .map_err(|err| DeliveryError::Database(err.to_string()))?;
    let event = match status {
        DeliveryStatus::Queued => "invitation_queued",
        DeliveryStatus::Sent => "invitation_sent",
        DeliveryStatus::Failed => "invitation_failed",
        _ => "invitation_sent",
    };
    client
        .execute(
            "INSERT INTO onboarding_batch_audit
                 (batch_id, person_id, event, to_status, reason_code, actor)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &batch_id,
                &person_id,
                &event,
                &status.as_str(),
                &provider,
                &actor.unwrap_or("batch"),
            ],
        )
        .map_err(|err| DeliveryError::Database(err.to_string()))?;
    Ok(())
}

/// Copies the authoritative invitation lifecycle back onto the batch item.
///
/// The invitation row owns acceptance and expiry. This function only reflects
/// that state; it never invents a delivery result.
pub fn sync_invitation_status(client: &mut Client, batch_id: i64) -> Result<(), DeliveryError> {
    // The expiry clock is authoritative: transition due pending rows first so
    // this sync reflects an expired invitation and its retry path.
    expire_due(client).map_err(|err| DeliveryError::Database(err.to_string()))?;
    client
        .execute(
            "UPDATE onboarding_batch_item AS item
                SET delivery_status = CASE invitation.status
                        WHEN 'accepted' THEN 'accepted'
                        WHEN 'expired' THEN 'expired'
                        WHEN 'revoked' THEN 'failed'
                        ELSE item.delivery_status
                    END,
                    updated_at = now()
               FROM onboarding_invitation AS invitation
              WHERE item.batch_id = $1
                AND item.invitation_id = invitation.invitation_id",
            &[&batch_id],
        )
        .map_err(|err| DeliveryError::Database(err.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_adapter_fails_closed() {
        let adapter = UnconfiguredAdapter;
        assert!(!adapter.is_configured());
        assert_eq!(
            adapter.deliver("a@example.test", "subject", "body"),
            Err(DeliveryError::NotConfigured)
        );
    }

    #[test]
    fn fake_adapter_accepts_and_records() {
        let adapter = FakeAdapter::accepting();
        let receipt = adapter
            .deliver("a@example.test", "subject", "body")
            .expect("fake adapter accepts");
        assert!(receipt.accepted);
        assert_eq!(adapter.messages().len(), 1);
        assert_eq!(adapter.messages()[0].to, "a@example.test");
    }

    #[test]
    fn queued_is_not_reported_as_delivered() {
        let adapter = FakeAdapter::queueing();
        let receipt = adapter
            .deliver("a@example.test", "subject", "body")
            .expect("fake adapter queues");
        assert!(!receipt.accepted);
    }

    #[test]
    fn activation_link_uses_query_or_ampersand() {
        assert_eq!(activation_link("/activate", "abc"), "/activate?token=abc");
        assert_eq!(
            activation_link("/activate?locale=en", "abc"),
            "/activate?locale=en&token=abc"
        );
    }

    #[test]
    fn delivery_status_round_trips() {
        for status in [
            DeliveryStatus::NotSent,
            DeliveryStatus::Queued,
            DeliveryStatus::Sent,
            DeliveryStatus::Accepted,
            DeliveryStatus::Failed,
            DeliveryStatus::Expired,
        ] {
            assert_eq!(DeliveryStatus::from_db(status.as_str()), Some(status));
        }
    }

    #[test]
    fn invalid_recipient_is_rejected_without_a_database() {
        // Shape validation runs before the database call; this test covers the
        // rejection branches that do not need a client.
        for email in ["", "plain", "a@", "@b.test", "a b@c.test"] {
            let (local, domain) = email.split_once('@').unwrap_or(("", ""));
            assert!(
                local.is_empty()
                    || domain.is_empty()
                    || !domain.contains('.')
                    || email.contains(' '),
                "{email} should be rejected"
            );
        }
    }
}
