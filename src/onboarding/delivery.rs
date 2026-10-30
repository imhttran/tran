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

/// A provider-backed adapter that posts one message to a configured HTTPS
/// endpoint.
///
/// The operator supplies the endpoint and an optional bearer credential. The
/// adapter reports `accepted` only when the provider answers with a 2xx status:
/// that is provider acceptance, never confirmation of inbox delivery. Redirects
/// are not followed, and no error carries the endpoint or the credential.
pub struct HttpMailAdapter {
    endpoint: String,
    bearer: Option<String>,
    timeout: Duration,
}

impl HttpMailAdapter {
    /// Builds an adapter for one provider endpoint.
    pub fn new(endpoint: impl Into<String>, bearer: Option<String>, timeout: Duration) -> Self {
        Self {
            endpoint: endpoint.into(),
            bearer,
            timeout,
        }
    }
}

impl MailAdapter for HttpMailAdapter {
    fn name(&self) -> &'static str {
        "http"
    }

    fn is_configured(&self) -> bool {
        !self.endpoint.trim().is_empty()
    }

    fn deliver(
        &self,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        if !self.is_configured() {
            return Err(DeliveryError::NotConfigured);
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| DeliveryError::Provider("client_unavailable".to_string()))?;
        let payload = serde_json::json!({ "to": to, "subject": subject, "body": body });
        let encoded = serde_json::to_vec(&payload)
            .map_err(|_| DeliveryError::Provider("payload_unavailable".to_string()))?;
        let mut request = client
            .post(&self.endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(encoded);
        if let Some(bearer) = self.bearer.as_deref() {
            request = request.bearer_auth(bearer);
        }
        let response = request
            .send()
            .map_err(|_| DeliveryError::Provider("provider_unreachable".to_string()))?;
        let status = response.status();
        if !status.is_success() {
            // The numeric status is safe and useful; the body may echo the
            // message, so it is never read into an error.
            return Err(DeliveryError::Provider(format!(
                "provider_status_{}",
                status.as_u16()
            )));
        }
        let message_id = response
            .text()
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| {
                value
                    .get("message_id")
                    .and_then(|id| id.as_str())
                    .map(str::to_string)
            });
        Ok(DeliveryReceipt {
            provider: "http".to_string(),
            accepted: true,
            message_id,
        })
    }
}

/// The explicit transport security mode for the SMTP adapter.
///
/// The mode is fixed by configuration. The adapter never retries with a lower
/// mode after a connection or TLS failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTlsMode {
    /// Plaintext. Permitted only for a loopback host through explicit local
    /// configuration.
    None,
    /// Implicit TLS (for example port 465).
    Implicit,
    /// Connect in plaintext, then upgrade with STARTTLS before credentials.
    StartTls,
}

impl SmtpTlsMode {
    /// Parses a configured mode. `None` when the value is unknown.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Some(SmtpTlsMode::None),
            "implicit" | "tls" => Some(SmtpTlsMode::Implicit),
            "starttls" | "start_tls" => Some(SmtpTlsMode::StartTls),
            _ => None,
        }
    }
}

/// The validated SMTP settings handed to the adapter.
///
/// Configuration code performs the loopback rule and the credential pairing
/// checks before it builds an adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpSettings {
    /// The SMTP server host name or address.
    pub host: String,
    /// The SMTP server port.
    pub port: u16,
    /// The explicit transport security mode.
    pub tls: SmtpTlsMode,
    /// The envelope sender address.
    pub from: String,
    /// The optional authentication user name. Present only together with
    /// `password`.
    pub username: Option<String>,
    /// The optional authentication password. Present only together with
    /// `username`.
    pub password: Option<String>,
    /// The bounded per-send timeout.
    pub timeout: Duration,
}

/// Returns true when `host` names a loopback interface.
///
/// Plaintext SMTP is permitted only for these hosts, and only through explicit
/// local configuration.
pub fn smtp_host_is_loopback(host: &str) -> bool {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1")
}

/// An SMTP adapter that sends one message per call over a fresh connection.
///
/// The adapter uses the configured transport security mode and never downgrades
/// it after a connection or TLS failure. It reports `accepted` only when the SMTP
/// server accepts the message after DATA: that is server acceptance, never a
/// claim of delivery to a real inbox. Error messages carry a safe provider code
/// only; they never carry the host, a credential, the token, or the body.
pub struct SmtpMailAdapter {
    settings: SmtpSettings,
}

impl SmtpMailAdapter {
    /// Builds an adapter from validated settings.
    pub fn new(settings: SmtpSettings) -> Self {
        Self { settings }
    }
}

impl MailAdapter for SmtpMailAdapter {
    fn name(&self) -> &'static str {
        "smtp"
    }

    fn is_configured(&self) -> bool {
        !self.settings.host.trim().is_empty() && !self.settings.from.trim().is_empty()
    }

    fn deliver(
        &self,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        if !self.is_configured() {
            return Err(DeliveryError::NotConfigured);
        }
        // The mode comes from configuration and is never lowered here.
        crate::onboarding::smtp_delivery::send(&self.settings, to, subject, body)
            .map_err(|code| DeliveryError::Provider(code.to_string()))?;
        // SMTP acceptance only. This is server acceptance of the message, not a
        // claim that it reached a real inbox.
        Ok(DeliveryReceipt {
            provider: "smtp".to_string(),
            accepted: true,
            message_id: None,
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

/// Builds the activation link in the path-based `/activate/<token>` form.
///
/// The token appears only in the message body. Both `send_batch` (bulk) and
/// `deliver_invitation` (individual send and resend) call this one builder, so
/// every flow produces the same link shape.
pub(crate) fn activation_link(base: &str, token: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), token)
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
    // Keep the invitation row's own delivery receipt in step with the batch item,
    // so a per-person status read reports the same attempt for both flows.
    if let Some(invitation_id) = invitation_id {
        client
            .execute(
                "UPDATE onboarding_invitation
                    SET delivery_state = $2,
                        delivery_provider = $3,
                        delivery_attempts = delivery_attempts + 1,
                        last_delivery_error = CASE WHEN $2 = 'failed' THEN $3 ELSE NULL END
                  WHERE invitation_id = $1",
                &[&invitation_id, &status.as_str(), &provider],
            )
            .map_err(|err| DeliveryError::Database(err.to_string()))?;
    }
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
    fn activation_link_uses_the_path_form() {
        assert_eq!(activation_link("/activate", "abc"), "/activate/abc");
        assert_eq!(
            activation_link("http://localhost:3000/activate/", "abc"),
            "http://localhost:3000/activate/abc"
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

    /// Serves exactly one canned HTTP response, then closes.
    fn one_shot_server(
        status_line: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept one request");
            let mut buffer = [0u8; 8192];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let response = format!(
                "{status_line}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            request
        });
        (format!("http://{address}/deliver"), handle)
    }

    #[test]
    fn http_adapter_reports_provider_acceptance() {
        let (endpoint, handle) =
            one_shot_server("HTTP/1.1 202 Accepted", "{\"message_id\":\"m-1\"}");
        let adapter = HttpMailAdapter::new(
            endpoint,
            Some("synthetic-bearer".to_string()),
            Duration::from_secs(5),
        );
        assert!(adapter.is_configured());
        let receipt = adapter
            .deliver("synthetic@example.test", "Activate", "Link body")
            .expect("provider acceptance");
        assert!(receipt.accepted);
        assert_eq!(receipt.provider, "http");
        assert_eq!(receipt.message_id.as_deref(), Some("m-1"));
        let request = handle.join().expect("server thread");
        assert!(request.contains("POST /deliver"));
        assert!(request.contains("authorization: Bearer synthetic-bearer"));
        assert!(request.contains("synthetic@example.test"));
    }

    #[test]
    fn http_adapter_never_claims_success_on_a_provider_error() {
        let (endpoint, handle) = one_shot_server("HTTP/1.1 500 Internal Server Error", "no");
        let adapter = HttpMailAdapter::new(endpoint, None, Duration::from_secs(5));
        let error = adapter
            .deliver("synthetic@example.test", "Activate", "Link body")
            .expect_err("provider rejection");
        assert_eq!(
            error,
            DeliveryError::Provider("provider_status_500".to_string())
        );
        let rendered = format!("{error:?}");
        assert!(
            !rendered.contains("127.0.0.1"),
            "the error leaked the endpoint"
        );
        assert!(
            !rendered.contains("example.test"),
            "the error leaked the address"
        );
        handle.join().expect("server thread");
    }

    #[test]
    fn http_adapter_fails_closed_without_an_endpoint() {
        let adapter = HttpMailAdapter::new("   ", None, Duration::from_secs(1));
        assert!(!adapter.is_configured());
        assert_eq!(
            adapter.deliver("synthetic@example.test", "s", "b"),
            Err(DeliveryError::NotConfigured)
        );
    }

    fn smtp_settings(tls: SmtpTlsMode) -> SmtpSettings {
        SmtpSettings {
            host: "127.0.0.1".to_string(),
            port: 1025,
            tls,
            from: "tran@example.test".to_string(),
            username: None,
            password: None,
            timeout: Duration::from_secs(2),
        }
    }

    /// A one-shot plaintext SMTP server that answers with the given final reply.
    fn smtp_server(final_reply: &'static str) -> (u16, std::thread::JoinHandle<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind smtp stub");
        let port = listener.local_addr().expect("smtp address").port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept smtp client");
            let _ = stream.write_all(b"220 stub ESMTP\r\n");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut transcript = String::new();
            let mut in_data = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                transcript.push_str(&line);
                if in_data {
                    // Body lines (including one starting with a dot) are not
                    // commands; the client sends exactly one reply for the
                    // terminating dot. Replying to each body line would leave
                    // extra replies in the client's buffer.
                    if line.trim_end() == "." {
                        let _ = stream.write_all(final_reply.as_bytes());
                        let _ = stream.flush();
                        break;
                    }
                    continue;
                }
                let upper = line.trim().to_ascii_uppercase();
                let reply = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                    "250 stub\r\n"
                } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
                    "250 ok\r\n"
                } else if upper.starts_with("DATA") {
                    in_data = true;
                    "354 send data\r\n"
                } else if upper.starts_with("QUIT") {
                    "221 bye\r\n"
                } else {
                    "250 ok\r\n"
                };
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.flush();
            }
            transcript
        });
        (port, handle)
    }

    #[test]
    fn smtp_adapter_reports_acceptance_only_on_a_2xx_final_reply() {
        let (port, handle) = smtp_server("250 2.0.0 accepted\r\n");
        let mut settings = smtp_settings(SmtpTlsMode::None);
        settings.port = port;
        let adapter = SmtpMailAdapter::new(settings);
        assert!(adapter.is_configured());
        let receipt = adapter
            .deliver(
                "synthetic@example.test",
                "Activate",
                "/activate/secret-token",
            )
            .expect("smtp acceptance");
        assert!(receipt.accepted);
        assert_eq!(receipt.provider, "smtp");
        let transcript = handle.join().expect("smtp thread");
        assert!(transcript.contains("MAIL FROM:<tran@example.test>"));
        assert!(transcript.contains("RCPT TO:<synthetic@example.test>"));
    }

    #[test]
    fn smtp_adapter_reports_a_rejection_as_failure_without_leaking() {
        let (port, handle) = smtp_server("550 5.7.1 rejected\r\n");
        let mut settings = smtp_settings(SmtpTlsMode::None);
        settings.port = port;
        let adapter = SmtpMailAdapter::new(settings);
        let error = adapter
            .deliver(
                "synthetic@example.test",
                "Activate",
                "/activate/secret-token",
            )
            .expect_err("smtp rejection");
        let rendered = format!("{error:?}");
        assert!(!rendered.contains("secret-token"), "leaked a token");
        assert!(
            !rendered.contains("synthetic@example.test"),
            "leaked an address"
        );
        assert!(!rendered.contains("tran@example.test"), "leaked the sender");
        handle.join().expect("smtp thread");
    }

    #[test]
    fn smtp_adapter_does_not_downgrade_a_tls_mode() {
        use std::io::Write as _;

        // A plaintext stub that would happily accept MAIL FROM/RCPT TO makes a
        // downgrade observable: a TLS mode must fail during the handshake and
        // never send the envelope in plaintext.
        for tls in [SmtpTlsMode::Implicit, SmtpTlsMode::StartTls] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub");
            let port = listener.local_addr().expect("address").port();
            let handle = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept");
                let _ = stream.write_all(b"220 plaintext stub\r\n");
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buffer = [0u8; 4096];
                let read = std::io::Read::read(&mut stream, &mut buffer).unwrap_or(0);
                String::from_utf8_lossy(&buffer[..read]).into_owned()
            });
            let mut settings = smtp_settings(tls);
            settings.port = port;
            settings.timeout = Duration::from_secs(1);
            let adapter = SmtpMailAdapter::new(settings);
            let error = adapter
                .deliver("synthetic@example.test", "s", "b")
                .expect_err("a TLS mode must not fall back to plaintext");
            assert!(
                matches!(error, DeliveryError::Provider(_)),
                "unexpected error: {error:?}"
            );
            let transcript = handle.join().expect("stub thread");
            assert!(
                !transcript.contains("MAIL FROM"),
                "the adapter downgraded TLS and sent the envelope in plaintext"
            );
        }
    }

    #[test]
    fn smtp_adapter_times_out_on_a_silent_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind silent stub");
        let port = listener.local_addr().expect("address").port();
        let handle = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("accept");
            std::thread::sleep(Duration::from_secs(3));
        });
        let mut settings = smtp_settings(SmtpTlsMode::None);
        settings.port = port;
        settings.timeout = Duration::from_millis(300);
        let adapter = SmtpMailAdapter::new(settings);
        let started = std::time::Instant::now();
        let error = adapter
            .deliver("synthetic@example.test", "s", "b")
            .expect_err("timeout");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            matches!(error, DeliveryError::Provider(ref code) if code == "smtp_timeout"),
            "unexpected timeout error: {error:?}"
        );
        let _ = handle.join();
    }

    #[test]
    fn smtp_adapter_fails_closed_when_unconfigured() {
        let mut settings = smtp_settings(SmtpTlsMode::None);
        settings.host = "  ".to_string();
        let adapter = SmtpMailAdapter::new(settings);
        assert!(!adapter.is_configured());
        assert_eq!(
            adapter.deliver("a@example.test", "s", "b"),
            Err(DeliveryError::NotConfigured)
        );
    }

    #[test]
    fn smtp_loopback_detection() {
        assert!(smtp_host_is_loopback("127.0.0.1"));
        assert!(smtp_host_is_loopback("localhost"));
        assert!(smtp_host_is_loopback("[::1]"));
        assert!(!smtp_host_is_loopback("mail.example.test"));
    }
}
