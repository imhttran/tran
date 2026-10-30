//! Restricted draft contact, preparation, and explicit send for one person
//! (PIR-003, PIR-004).
//!
//! Preparation is a review step, never a send. The module offers these
//! operations:
//!
//! * [`read_invitation_state`] reads the family-admin decision, eligibility,
//!   invitation/delivery state, linked-account status, proposed username, and a
//!   plain-language blocker list;
//! * [`save_contact_draft`] stores a proposed address in the restricted
//!   `onboarding_invitation_contact_draft` table only;
//! * [`prepare_invitation`] rechecks authorization, eligibility, link state,
//!   duplicate invitation, and the draft contact, and returns a review payload;
//! * [`send_invitation`], [`resend_invitation`], [`revoke_invitation`], and
//!   [`read_invitation_status`] perform and report one explicit send.
//!
//! Only [`send_invitation`] and [`resend_invitation`] hand a message to a
//! configured [`MailAdapter`]. Every operation reports the adapter's actual
//! receipt: an unconfirmed send is never reported as delivered, and a failure
//! leaves the invitation recoverable without creating a second active link.
//! Only a masked address leaves the module in a response; the error text never
//! carries an address, and no return value carries a token.

use std::time::Duration;

use postgres::Client;
use thiserror::Error;

use crate::identity::VerifiedIdentity;
use crate::onboarding::account_map::IdentityAccount;
use crate::onboarding::candidate::{
    load_name_parts, mask_email, reclassify_person, CandidateError, Classification, Reason,
};
use crate::onboarding::delivery::{activation_link, DeliveryError, MailAdapter};
use crate::onboarding::family_admin::{
    authorize_person_invitation_in, FamilyAdminCheckError, FamilyAdminDecision,
};
use crate::onboarding::invitation::{create, revoke as revoke_row, InvitationError};
use crate::onboarding::username::candidate_username;

/// The invitation lifetime used by preparation and by an explicit send.
///
/// Preparation never sends; the row it reserves and the row a send delivers are
/// the same invitation, so both use one lifetime.
const INVITATION_TTL_SECS: u64 = 7 * 24 * 60 * 60;

/// The fixed review payload access level. Preparation grants no role: it only
/// offers the Member (`viewer`) level the operator would assign.
pub const PREPARE_ACCESS: &str = "viewer";

/// A safe, plain-language blocker for one person.
///
/// A blocker is a rule name in reader-facing words. It never carries a display
/// name, a date, a source excerpt, an address, or a private hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    /// The stable rule code for programmatic use.
    pub code: &'static str,
    /// The plain-language sentence for the operator.
    pub message: &'static str,
}

/// The current invitation and delivery state for one person.
///
/// The delivery state is read from the batch item, if any. A person with no
/// batch row, or whose batch row is not delivered, reports `not_sent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitationStateView {
    /// True when a pending, unexpired invitation exists.
    pub pending: bool,
    /// The pending invitation id, when one exists.
    pub invitation_id: Option<i64>,
    /// The pending invitation expiry, rendered as text.
    pub expires_at: Option<String>,
    /// The stored delivery state, or `not_sent` when none was recorded.
    pub delivery_state: &'static str,
}

/// The full prepared-state result of `read_invitation_state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitationState {
    /// The opaque person key this state describes.
    pub person_key: String,
    /// The family-admin allow code that authorized the read.
    pub authorization: &'static str,
    /// The eligibility decision (`eligible_for_invite`, `needs_review`, ...).
    pub eligibility: &'static str,
    /// The current invitation and delivery state.
    pub invitation: InvitationStateView,
    /// True when the person is linked to an account and is therefore not
    /// invitable.
    pub linked_account: bool,
    /// The proposed display username. A proposal only; no reservation is made.
    pub proposed_username: Option<String>,
    /// The masked draft contact, when a draft was saved.
    pub masked_contact: Option<String>,
    /// The plain-language blockers that stop invitation preparation.
    pub blockers: Vec<Blocker>,
}

/// The review payload returned by `prepare_invitation`.
///
/// It names the exact person, the masked contact, the proposed username, and the
/// Member (`viewer`) access level preparation would grant. It carries no
/// unmasked address and never a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareReview {
    /// The opaque person key under review.
    pub person_key: String,
    /// The masked draft contact. Never the stored address.
    pub masked_contact: String,
    /// The proposed username, when the verified name parts permit one.
    pub proposed_username: Option<String>,
    /// The access level offered at review time: always [`PREPARE_ACCESS`].
    pub access: &'static str,
    /// The pending invitation id, whether reused or newly issued.
    pub invitation_id: i64,
    /// True when an existing pending invitation was reused, not duplicated.
    pub invitation_reused: bool,
    /// True when the row was newly created by this preparation.
    pub invitation_created: bool,
}

/// The safe delivery state of one profile invitation.
///
/// The state is recorded from the mail adapter's actual receipt. `Queued` is
/// deliberately distinct from `Sent`: a provider that only accepted the message
/// for later delivery has not confirmed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileDeliveryState {
    /// No invitation row exists for this person.
    NotSent,
    /// An invitation row exists but no send was attempted.
    Pending,
    /// The provider accepted the message for later delivery. Not confirmed.
    Queued,
    /// The provider confirmed the message was sent.
    Sent,
    /// The last delivery attempt failed. The invitation stays recoverable.
    Failed,
    /// The invitation expired before acceptance.
    Expired,
    /// The invitation was accepted.
    Accepted,
    /// The invitation was revoked.
    Revoked,
}

impl ProfileDeliveryState {
    /// The stable string code for this state.
    pub fn as_str(self) -> &'static str {
        match self {
            ProfileDeliveryState::NotSent => "not_sent",
            ProfileDeliveryState::Pending => "pending",
            ProfileDeliveryState::Queued => "queued",
            ProfileDeliveryState::Sent => "sent",
            ProfileDeliveryState::Failed => "failed",
            ProfileDeliveryState::Expired => "expired",
            ProfileDeliveryState::Accepted => "accepted",
            ProfileDeliveryState::Revoked => "revoked",
        }
    }

    /// Reads a stored delivery state. An unrecognized value yields `None`, so a
    /// caller reports an unknown state rather than guessing.
    fn from_db(value: &str) -> Option<Self> {
        match value {
            "not_sent" => Some(ProfileDeliveryState::NotSent),
            "pending" => Some(ProfileDeliveryState::Pending),
            "queued" => Some(ProfileDeliveryState::Queued),
            "sent" => Some(ProfileDeliveryState::Sent),
            "failed" => Some(ProfileDeliveryState::Failed),
            "expired" => Some(ProfileDeliveryState::Expired),
            "accepted" => Some(ProfileDeliveryState::Accepted),
            "revoked" => Some(ProfileDeliveryState::Revoked),
            _ => None,
        }
    }

    /// True only when the provider confirmed the send.
    fn is_confirmed(self) -> bool {
        matches!(self, ProfileDeliveryState::Sent)
    }
}

/// The result of one explicit send or resend.
///
/// It names the invitation and the adapter's actual outcome. It carries the
/// masked contact only, never a token, an address, or a provider credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendInvitationOutcome {
    /// The opaque person key the send belongs to.
    pub person_key: String,
    /// The invitation this send attempted.
    pub invitation_id: i64,
    /// The recorded delivery state.
    pub delivery_state: ProfileDeliveryState,
    /// The adapter that handled the attempt, when one ran.
    pub provider: Option<String>,
    /// True only when the provider confirmed the message was sent.
    pub delivered: bool,
    /// The masked contact the message was addressed to.
    pub masked_contact: String,
}

/// The result of one revoke request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeInvitationOutcome {
    /// The opaque person key the revoke belongs to.
    pub person_key: String,
    /// The revoked invitation, when one was live.
    pub invitation_id: Option<i64>,
    /// The resulting delivery state.
    pub delivery_state: ProfileDeliveryState,
    /// True when a live pending invitation was revoked.
    pub revoked: bool,
    /// A safe reason code when nothing was revoked.
    pub reason: Option<&'static str>,
}

/// The safe invitation and delivery status of one person.
///
/// The view never carries a token, an address, or a provider credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitationStatusView {
    /// The opaque person key this status describes.
    pub person_key: String,
    /// The most recent invitation, when one exists.
    pub invitation_id: Option<i64>,
    /// The invitation lifecycle: `pending`, `accepted`, `revoked`, or `expired`.
    pub invitation_status: Option<&'static str>,
    /// The recorded delivery state.
    pub delivery_state: ProfileDeliveryState,
    /// The adapter that handled the last attempt.
    pub delivery_provider: Option<String>,
    /// A safe failure code such as `not_configured` or `provider_failed`.
    pub last_delivery_error: Option<String>,
    /// How many delivery attempts were recorded.
    pub delivery_attempts: i32,
    /// The invitation expiry, rendered as text.
    pub expires_at: Option<String>,
    /// The acceptance time, rendered as text.
    pub accepted_at: Option<String>,
}

/// Errors raised by the profile-invitation operations.
///
/// No variant carries an address, a token, a name, or raw SQL text.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProfileInvitationError {
    /// The caller was refused by the family-admin rule. The code is the stable
    /// snake_case refusal.
    #[error("the caller may not invite this person: {0}")]
    Refused(&'static str),
    /// The person key does not resolve to a person row.
    #[error("unknown person")]
    UnknownPerson,
    /// The supplied draft address is not acceptable.
    #[error("the contact address is not valid")]
    InvalidAddress,
    /// Preparation needs a saved draft contact and found none.
    #[error("a draft contact is required before preparation")]
    MissingContact,
    /// The person is linked to an account and is not invitable.
    #[error("the person is already linked to an account")]
    AlreadyLinked,
    /// The person's evidence needs review or is not eligible.
    #[error("the person is not eligible for an invitation")]
    NotEligible,
    /// No invitation exists for this person, so there is nothing to resend or
    /// revoke.
    #[error("no invitation exists for this person")]
    NoInvitation,
    /// The invitation is not in a state that permits the operation. The code is
    /// the invitation's current safe status.
    #[error("the invitation is not pending: {0}")]
    InvitationNotPending(&'static str),
    /// The mail adapter failed or is not configured. The invitation, when one
    /// was issued, stays recoverable.
    #[error("invitation delivery failed")]
    Delivery(DeliveryError),
    /// The invitation operation failed for a non-policy reason.
    #[error("invitation operation failed")]
    Invitation(String),
    /// The database operation failed.
    #[error("profile invitation database operation failed")]
    Database(String),
}

impl From<CandidateError> for ProfileInvitationError {
    fn from(err: CandidateError) -> Self {
        ProfileInvitationError::Database(err.to_string())
    }
}

impl From<InvitationError> for ProfileInvitationError {
    fn from(err: InvitationError) -> Self {
        match err {
            InvitationError::UnknownPerson => ProfileInvitationError::UnknownPerson,
            other => ProfileInvitationError::Invitation(other.to_string()),
        }
    }
}

impl From<DeliveryError> for ProfileInvitationError {
    fn from(err: DeliveryError) -> Self {
        ProfileInvitationError::Delivery(err)
    }
}

impl From<postgres::Error> for ProfileInvitationError {
    fn from(err: postgres::Error) -> Self {
        ProfileInvitationError::Database(err.to_string())
    }
}

impl From<FamilyAdminCheckError> for ProfileInvitationError {
    fn from(err: FamilyAdminCheckError) -> Self {
        match err {
            FamilyAdminCheckError::Refused(reason) => {
                ProfileInvitationError::Refused(reason.as_str())
            }
            FamilyAdminCheckError::Database(_) => {
                ProfileInvitationError::Database("family-admin read failed".to_string())
            }
        }
    }
}

/// Resolves a person key to its id inside the caller's client.
fn person_id_for_key(client: &mut Client, person_key: &str) -> Result<i64, ProfileInvitationError> {
    let row = client.query_opt(
        "SELECT person_id FROM person WHERE person_key = $1",
        &[&person_key],
    )?;
    row.map(|row| row.get(0))
        .ok_or(ProfileInvitationError::UnknownPerson)
}

/// Returns the stable allow code for a decision.
fn allow_code(decision: FamilyAdminDecision) -> &'static str {
    decision.as_str()
}

/// One invitation row with its safe delivery receipt.
struct InvitationRecord {
    invitation_id: i64,
    /// The stored lifecycle: `pending`, `accepted`, `revoked`, or `expired`.
    status: &'static str,
    /// True when the row is still `pending` but its expiry has passed.
    expired: bool,
    delivery_state: ProfileDeliveryState,
    delivery_provider: Option<String>,
    last_delivery_error: Option<String>,
    delivery_attempts: i32,
    expires_at: Option<String>,
    accepted_at: Option<String>,
}

/// Maps the stored invitation status to its stable code.
fn status_code(status: &str) -> &'static str {
    match status {
        "pending" => "pending",
        "accepted" => "accepted",
        "revoked" => "revoked",
        "expired" => "expired",
        _ => "unknown",
    }
}

/// Reads the most recent invitation row for one person with its delivery receipt.
///
/// The row is read as stored. When it is `pending` past its expiry, the returned
/// record reports the effective `expired` state without writing anything, so a
/// status read stays read-only.
fn latest_invitation(
    client: &mut Client,
    person_id: i64,
) -> Result<Option<InvitationRecord>, ProfileInvitationError> {
    let row = client.query_opt(
        "SELECT invitation_id, status,
                (status = 'pending' AND expires_at IS NOT NULL AND expires_at <= now()),
                delivery_state, delivery_provider, last_delivery_error,
                delivery_attempts, expires_at::text, accepted_at::text
           FROM onboarding_invitation
          WHERE person_id = $1
          ORDER BY created_at DESC, invitation_id DESC
          LIMIT 1",
        &[&person_id],
    )?;
    let Some(row) = row else {
        return Ok(None);
    };
    let stored_status: String = row.get(1);
    let expired: bool = row.get(2);
    let stored_delivery: String = row.get(3);
    let invitation_id: i64 = row.get(0);

    let status = match stored_status.as_str() {
        "accepted" => "accepted",
        "revoked" => "revoked",
        "expired" => "expired",
        // A pending row past its expiry is reported as expired.
        _ if expired => "expired",
        _ => "pending",
    };
    let delivery_state = match status {
        "accepted" => ProfileDeliveryState::Accepted,
        "revoked" => ProfileDeliveryState::Revoked,
        "expired" => ProfileDeliveryState::Expired,
        _ => {
            ProfileDeliveryState::from_db(&stored_delivery).unwrap_or(ProfileDeliveryState::Pending)
        }
    };

    Ok(Some(InvitationRecord {
        invitation_id,
        status,
        expired,
        delivery_state,
        delivery_provider: row.get(4),
        last_delivery_error: row.get(5),
        delivery_attempts: row.get(6),
        expires_at: row.get(7),
        accepted_at: row.get(8),
    }))
}

/// Reads the live pending invitation for one person, if any.
///
/// An expired pending row is not live, so it never counts as a duplicate or as a
/// reason to reuse a row a later send cannot deliver.
fn pending_invitation(
    client: &mut Client,
    person_id: i64,
) -> Result<Option<InvitationRecord>, ProfileInvitationError> {
    let latest = latest_invitation(client, person_id)?;
    Ok(latest.filter(|record| record.status == "pending" && !record.expired))
}

/// Reads the invitation and delivery state for one person.
fn read_invitation_state_view(
    client: &mut Client,
    person_id: i64,
) -> Result<InvitationStateView, ProfileInvitationError> {
    let Some(record) = latest_invitation(client, person_id)? else {
        return Ok(InvitationStateView {
            pending: false,
            invitation_id: None,
            expires_at: None,
            delivery_state: ProfileDeliveryState::NotSent.as_str(),
        });
    };
    let live = record.status == "pending" && !record.expired;
    Ok(InvitationStateView {
        pending: live,
        invitation_id: live.then_some(record.invitation_id),
        expires_at: live.then_some(record.expires_at).flatten(),
        delivery_state: record.delivery_state.as_str(),
    })
}

/// Reads the current masked draft contact for one person.
fn read_masked_draft(
    client: &mut Client,
    person_id: i64,
) -> Result<Option<String>, ProfileInvitationError> {
    let row = client.query_opt(
        "SELECT email FROM onboarding_invitation_contact_draft WHERE person_id = $1",
        &[&person_id],
    )?;
    Ok(row.map(|row| mask_email(&row.get::<_, String>(0))))
}

/// Reads the current linked-account status for one person.
fn linked_account(client: &mut Client, person_id: i64) -> Result<bool, ProfileInvitationError> {
    let count: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_person_account_link WHERE person_id = $1",
            &[&person_id],
        )?
        .get(0);
    Ok(count > 0)
}

/// Builds the plain-language blockers from classification reasons and the
/// draft-contact state.
///
/// Missing draft contact is reported last, because a reviewer first needs to
/// know whether the person is eligible at all.
fn build_blockers(reasons: &[Reason], has_draft: bool) -> Vec<Blocker> {
    let mut blockers: Vec<Blocker> = Vec::new();
    for reason in reasons {
        if let Some(blocker) = blocker_for_reason(*reason) {
            if !blockers
                .iter()
                .any(|existing| existing.code == blocker.code)
            {
                blockers.push(blocker);
            }
        }
    }
    if !has_draft {
        blockers.push(Blocker {
            code: "missing_contact",
            message: "a draft contact is required before preparation",
        });
    }
    blockers
}

/// Maps a classification reason to a plain-language blocker, when it blocks
/// preparation.
///
/// Warnings (missing DOB, username collision, missing verified recipient) do not
/// block preparation and yield `None`.
fn blocker_for_reason(reason: Reason) -> Option<Blocker> {
    let (code, message) = match reason {
        Reason::AlreadyLinked => (
            "already_linked",
            "the person is already linked to an account and cannot be invited",
        ),
        Reason::AlreadyInvited => (
            "already_invited",
            "a pending invitation already exists and will be reused at preparation",
        ),
        Reason::AlreadyPrepared => (
            "already_prepared",
            "the person was already prepared in a batch",
        ),
        Reason::ContradictoryVital => (
            "contradictory_vital",
            "the vital records disagree and need a review",
        ),
        Reason::Disputed => ("disputed", "a reviewer disputed this person's evidence"),
        Reason::MissingCitation => (
            "missing_citation",
            "the living evidence has no attributed source",
        ),
        Reason::Ambiguous => ("ambiguous", "more than one person matches this record"),
        Reason::UnresolvedDuplicate => (
            "unresolved_duplicate",
            "more than one record shares this person's key",
        ),
        Reason::NoLivingEvidence => (
            "no_living_evidence",
            "the archive records no living evidence for this person",
        ),
        Reason::AmbiguousSegmentation => (
            "ambiguous_segmentation",
            "the name cannot be split into structured parts",
        ),
        Reason::MinorGuardianReview => (
            "minor_guardian_review",
            "a minor needs guardian review before onboarding",
        ),
        Reason::VerifiedLivingEvidence
        | Reason::UsernameCollision
        | Reason::MissingDob
        | Reason::MissingVerifiedContact
        | Reason::BlockedMissingRecipient => return None,
    };
    Some(Blocker { code, message })
}

/// Reads the invitation state for one person.
///
/// The caller must pass the family-admin rule. The function issues SELECT
/// statements only, never sends, and never creates or updates a row. The
/// returned state carries only the masked draft contact.
pub fn read_invitation_state(
    client: &mut Client,
    identity: &VerifiedIdentity,
    account: &IdentityAccount,
    person_key: &str,
) -> Result<InvitationState, ProfileInvitationError> {
    let person_key = person_key.trim();
    let decision = authorize_person_invitation_in(client, identity, account, person_key)?;
    let authorization = allow_code(decision);
    let person_id = person_id_for_key(client, person_key)?;

    let (classification, reasons) = reclassify_person(client, person_id, person_key)?;
    let invitation = read_invitation_state_view(client, person_id)?;
    let linked =
        linked_account(client, person_id)? || classification == Classification::AlreadyLinked;
    let masked_contact = read_masked_draft(client, person_id)?;
    let proposed_username = load_name_parts(client, person_id)
        .ok()
        .and_then(|parts| candidate_username(&parts).ok());

    let mut blockers = build_blockers(&reasons, masked_contact.is_some());
    if linked
        && !blockers
            .iter()
            .any(|blocker| blocker.code == "already_linked")
    {
        blockers.insert(
            0,
            Blocker {
                code: "already_linked",
                message: "the person is already linked to an account and cannot be invited",
            },
        );
    }

    Ok(InvitationState {
        person_key: person_key.to_string(),
        authorization,
        eligibility: classification.as_str(),
        invitation,
        linked_account: linked,
        proposed_username,
        masked_contact,
        blockers,
    })
}

/// Validates a draft address shape.
///
/// The shape rules match the verified-recipient rule: one `@`, a non-empty local
/// part and domain, a dot in the domain, and no whitespace. The function returns
/// a trimmed address or [`ProfileInvitationError::InvalidAddress`]; the error
/// text never carries the input.
fn validate_draft_address(email: &str) -> Result<String, ProfileInvitationError> {
    let email = email.trim();
    let (local, domain) = email
        .split_once('@')
        .ok_or(ProfileInvitationError::InvalidAddress)?;
    if local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || email.chars().any(char::is_whitespace)
        || email.matches('@').count() != 1
    {
        return Err(ProfileInvitationError::InvalidAddress);
    }
    Ok(email.to_string())
}

/// Saves a proposed draft contact for one person.
///
/// The caller must pass the family-admin rule. The address is stored only in
/// the restricted `onboarding_invitation_contact_draft` table. The function does
/// not create an invitation, does not write `onboarding_recipient_contact`, and
/// imports no delivery adapter. The returned value is the masked contact.
pub fn save_contact_draft(
    client: &mut Client,
    identity: &VerifiedIdentity,
    account: &IdentityAccount,
    person_key: &str,
    email: &str,
) -> Result<String, ProfileInvitationError> {
    let person_key = person_key.trim();
    let address = validate_draft_address(email)?;
    // Authorization runs before any write, so a refused caller writes no row.
    authorize_person_invitation_in(client, identity, account, person_key)?;
    let person_id = person_id_for_key(client, person_key)?;
    let actor = format!("{}/{}", account.account_id, account.identity_fingerprint);
    client.execute(
        "INSERT INTO onboarding_invitation_contact_draft
             (person_id, email, created_by, updated_by)
         VALUES ($1, $2, $3, $3)
         ON CONFLICT (person_id) DO UPDATE
            SET email = EXCLUDED.email,
                updated_by = EXCLUDED.updated_by,
                updated_at = now()",
        &[&person_id, &address, &actor],
    )?;
    Ok(mask_email(&address))
}

/// Prepares one person for invitation review.
///
/// The function rechecks the family-admin rule, eligibility, the reviewed link
/// state, a duplicate pending invitation, and the draft contact. It refuses a
/// linked person and a person whose evidence needs review or is not eligible. A
/// duplicate pending invitation is reused through `invitation::create`, so no
/// second row is inserted. The function imports no delivery adapter and never
/// marks an invitation sent. The payload carries the masked contact only.
pub fn prepare_invitation(
    client: &mut Client,
    identity: &VerifiedIdentity,
    account: &IdentityAccount,
    person_key: &str,
) -> Result<PrepareReview, ProfileInvitationError> {
    let person_key = person_key.trim();
    authorize_person_invitation_in(client, identity, account, person_key)?;
    let person_id = person_id_for_key(client, person_key)?;

    if linked_account(client, person_id)? {
        return Err(ProfileInvitationError::AlreadyLinked);
    }
    let (classification, _reasons) = reclassify_person(client, person_id, person_key)?;
    match classification {
        Classification::AlreadyLinked => return Err(ProfileInvitationError::AlreadyLinked),
        Classification::NeedsReview | Classification::NotEligible => {
            return Err(ProfileInvitationError::NotEligible)
        }
        Classification::EligibleForInvite
        | Classification::Invited
        | Classification::AlreadyPrepared => {}
    }

    let draft = client.query_opt(
        "SELECT email FROM onboarding_invitation_contact_draft WHERE person_id = $1",
        &[&person_id],
    )?;
    let Some(draft) = draft else {
        return Err(ProfileInvitationError::MissingContact);
    };
    let stored: String = draft.get(0);
    let masked_contact = mask_email(&stored);

    let proposed_username = load_name_parts(client, person_id)
        .ok()
        .and_then(|parts| candidate_username(&parts).ok());

    // A duplicate pending invitation is reused: `create` returns the live row
    // with `token: None` instead of inserting a second one.
    let before = client.query_opt(
        "SELECT invitation_id FROM onboarding_invitation
          WHERE person_id = $1 AND status = 'pending'
            AND expires_at IS NOT NULL AND expires_at > now()
          ORDER BY created_at DESC, invitation_id DESC LIMIT 1",
        &[&person_id],
    )?;
    let existing_id: Option<i64> = before.map(|row| row.get(0));

    let actor = format!("{}/{}", account.account_id, account.identity_fingerprint);
    let issued = create(
        client,
        person_id,
        std::time::Duration::from_secs(INVITATION_TTL_SECS),
        &actor,
    )?;

    let invitation_reused = existing_id == Some(issued.invitation_id) && issued.token.is_none();
    let invitation_created = issued.token.is_some();

    Ok(PrepareReview {
        person_key: person_key.to_string(),
        masked_contact,
        proposed_username,
        access: PREPARE_ACCESS,
        invitation_id: issued.invitation_id,
        invitation_reused,
        invitation_created,
    })
}

/// Reads the stored draft contact for one person.
fn draft_contact(
    client: &mut Client,
    person_id: i64,
) -> Result<Option<String>, ProfileInvitationError> {
    let row = client.query_opt(
        "SELECT email FROM onboarding_invitation_contact_draft WHERE person_id = $1",
        &[&person_id],
    )?;
    Ok(row.map(|row| row.get(0)))
}

/// The audit actor string for a caller.
fn actor_for(account: &IdentityAccount) -> String {
    format!("{}/{}", account.account_id, account.identity_fingerprint)
}

/// Rechecks the authority, link, and eligibility a send requires.
///
/// The family-admin rule runs before any row is read, so a refused caller learns
/// nothing about the person.
fn check_invitable(
    client: &mut Client,
    identity: &VerifiedIdentity,
    account: &IdentityAccount,
    person_key: &str,
) -> Result<i64, ProfileInvitationError> {
    authorize_person_invitation_in(client, identity, account, person_key)?;
    let person_id = person_id_for_key(client, person_key)?;
    if linked_account(client, person_id)? {
        return Err(ProfileInvitationError::AlreadyLinked);
    }
    let (classification, _reasons) = reclassify_person(client, person_id, person_key)?;
    match classification {
        Classification::AlreadyLinked => Err(ProfileInvitationError::AlreadyLinked),
        Classification::NeedsReview | Classification::NotEligible => {
            Err(ProfileInvitationError::NotEligible)
        }
        Classification::EligibleForInvite
        | Classification::Invited
        | Classification::AlreadyPrepared => Ok(person_id),
    }
}

/// The resolved inputs for one delivery attempt.
struct DeliveryRequest<'a> {
    person_key: &'a str,
    person_id: i64,
    email: &'a str,
    activation_base: &'a str,
    actor: &'a str,
}

/// Issues at most one active invitation and hands one message to the adapter.
///
/// Any pending row is revoked first, so a resend never leaves two live links for
/// one person. The recorded state is the adapter's actual receipt: an unconfirmed
/// message is stored as `queued`, and a failure is stored as `failed` with a safe
/// code while the row stays recoverable. No return value carries the token.
fn deliver_invitation(
    client: &mut Client,
    request: DeliveryRequest<'_>,
    adapter: &dyn MailAdapter,
) -> Result<SendInvitationOutcome, ProfileInvitationError> {
    let DeliveryRequest {
        person_key,
        person_id,
        email,
        activation_base,
        actor,
    } = request;
    let masked_contact = mask_email(email);

    if let Some(pending) = pending_invitation(client, person_id)? {
        let _ = revoke_row(client, pending.invitation_id);
        let _ = client.execute(
            "UPDATE onboarding_invitation
                SET delivery_state = 'revoked', last_delivery_error = 'revoked'
              WHERE invitation_id = $1",
            &[&pending.invitation_id],
        );
    }

    let issued = create(
        client,
        person_id,
        Duration::from_secs(INVITATION_TTL_SECS),
        actor,
    )?;
    let Some(token) = issued.token else {
        return Err(ProfileInvitationError::Invitation(
            "no deliverable invitation was issued".to_string(),
        ));
    };

    let link = activation_link(activation_base, &token);
    let subject = "Activate your family archive account";
    let body = format!(
        "A family administrator prepared an account for you in the family archive.\n\
         Open this single-use link to continue:\n\n{link}\n\n\
         The link expires automatically. If you did not expect this message, ignore it."
    );

    match adapter.deliver(email, subject, &body) {
        Ok(receipt) => {
            let state = if receipt.accepted {
                ProfileDeliveryState::Sent
            } else {
                ProfileDeliveryState::Queued
            };
            client.execute(
                "UPDATE onboarding_invitation
                    SET invited_email = $2,
                        delivery_state = $3,
                        delivery_provider = $4,
                        delivery_attempts = delivery_attempts + 1,
                        last_delivery_error = NULL
                  WHERE invitation_id = $1",
                &[
                    &issued.invitation_id,
                    &email,
                    &state.as_str(),
                    &receipt.provider,
                ],
            )?;
            Ok(SendInvitationOutcome {
                person_key: person_key.to_string(),
                invitation_id: issued.invitation_id,
                delivery_state: state,
                provider: Some(receipt.provider),
                delivered: state.is_confirmed(),
                masked_contact: masked_contact.clone(),
            })
        }
        Err(err) => {
            let code = match &err {
                DeliveryError::NotConfigured => "not_configured",
                DeliveryError::Provider(_) => "provider_failed",
                DeliveryError::InvalidRecipient => "invalid_recipient",
                DeliveryError::MissingVerifiedContact => "missing_verified_contact",
                DeliveryError::NotPrepared => "not_prepared",
                DeliveryError::Database(_) | DeliveryError::Invitation(_) => "delivery_failed",
            };
            let _ = client.execute(
                "UPDATE onboarding_invitation
                    SET delivery_state = 'failed',
                        delivery_provider = $2,
                        delivery_attempts = delivery_attempts + 1,
                        last_delivery_error = $3
                  WHERE invitation_id = $1",
                &[&issued.invitation_id, &adapter.name(), &code],
            );
            Err(ProfileInvitationError::Delivery(err))
        }
    }
}

/// Sends one reviewed profile invitation.
///
/// The family-admin rule, link state, and eligibility are rechecked before any
/// write. An adapter that is not configured fails closed before an invitation is
/// created. An invitation already confirmed by the provider is never duplicated
/// or silently re-sent; use [`resend_invitation`] for an explicit repeat. The
/// returned state is the adapter's actual receipt.
pub fn send_invitation(
    client: &mut Client,
    identity: &VerifiedIdentity,
    account: &IdentityAccount,
    person_key: &str,
    adapter: &dyn MailAdapter,
    activation_base: &str,
) -> Result<SendInvitationOutcome, ProfileInvitationError> {
    if !adapter.is_configured() {
        return Err(ProfileInvitationError::Delivery(
            DeliveryError::NotConfigured,
        ));
    }
    let person_key = person_key.trim();
    let person_id = check_invitable(client, identity, account, person_key)?;

    // A delivered or queued invitation is reported as it stands. Creating a
    // second row or re-sending without an explicit resend would duplicate the
    // link and misstate the outcome.
    if let Some(pending) = pending_invitation(client, person_id)? {
        if matches!(
            pending.delivery_state,
            ProfileDeliveryState::Sent | ProfileDeliveryState::Queued
        ) {
            let masked_contact = draft_contact(client, person_id)?
                .map(|email| mask_email(&email))
                .unwrap_or_default();
            return Ok(SendInvitationOutcome {
                person_key: person_key.to_string(),
                invitation_id: pending.invitation_id,
                delivery_state: pending.delivery_state,
                provider: pending.delivery_provider,
                delivered: pending.delivery_state.is_confirmed(),
                masked_contact,
            });
        }
    }

    let email = draft_contact(client, person_id)?.ok_or(ProfileInvitationError::MissingContact)?;
    let actor = actor_for(account);
    deliver_invitation(
        client,
        DeliveryRequest {
            person_key,
            person_id,
            email: &email,
            activation_base,
            actor: &actor,
        },
        adapter,
    )
}

/// Resends one profile invitation.
///
/// The same family-admin rule decides authority. The current invitation is
/// revoked before a replacement is issued, so the previous link stops working
/// and only one invitation stays live. An accepted invitation cannot be
/// resent.
pub fn resend_invitation(
    client: &mut Client,
    identity: &VerifiedIdentity,
    account: &IdentityAccount,
    person_key: &str,
    adapter: &dyn MailAdapter,
    activation_base: &str,
) -> Result<SendInvitationOutcome, ProfileInvitationError> {
    if !adapter.is_configured() {
        return Err(ProfileInvitationError::Delivery(
            DeliveryError::NotConfigured,
        ));
    }
    let person_key = person_key.trim();
    let person_id = check_invitable(client, identity, account, person_key)?;

    let Some(latest) = latest_invitation(client, person_id)? else {
        return Err(ProfileInvitationError::NoInvitation);
    };
    if latest.status == "accepted" {
        return Err(ProfileInvitationError::InvitationNotPending("accepted"));
    }

    let email = draft_contact(client, person_id)?.ok_or(ProfileInvitationError::MissingContact)?;
    let actor = actor_for(account);
    deliver_invitation(
        client,
        DeliveryRequest {
            person_key,
            person_id,
            email: &email,
            activation_base,
            actor: &actor,
        },
        adapter,
    )
}

/// Revokes the live pending invitation for one person.
///
/// The same family-admin rule decides authority. Revoking an already revoked,
/// expired, or accepted invitation changes nothing and reports the current
/// state instead of an error.
pub fn revoke_invitation(
    client: &mut Client,
    identity: &VerifiedIdentity,
    account: &IdentityAccount,
    person_key: &str,
) -> Result<RevokeInvitationOutcome, ProfileInvitationError> {
    let person_key = person_key.trim();
    authorize_person_invitation_in(client, identity, account, person_key)?;
    let person_id = person_id_for_key(client, person_key)?;

    let Some(pending) = pending_invitation(client, person_id)? else {
        let current = latest_invitation(client, person_id)?;
        return Ok(RevokeInvitationOutcome {
            person_key: person_key.to_string(),
            invitation_id: None,
            delivery_state: current
                .map(|record| record.delivery_state)
                .unwrap_or(ProfileDeliveryState::NotSent),
            revoked: false,
            reason: Some("no_pending_invitation"),
        });
    };

    revoke_row(client, pending.invitation_id)?;
    client.execute(
        "UPDATE onboarding_invitation
            SET delivery_state = 'revoked', last_delivery_error = 'revoked'
          WHERE invitation_id = $1",
        &[&pending.invitation_id],
    )?;

    Ok(RevokeInvitationOutcome {
        person_key: person_key.to_string(),
        invitation_id: Some(pending.invitation_id),
        delivery_state: ProfileDeliveryState::Revoked,
        revoked: true,
        reason: None,
    })
}

/// Reads the safe invitation and delivery status for one person.
///
/// The caller must pass the family-admin rule. The view reports the lifecycle and
/// the recorded attempt, and never a token, an address, or a credential. A
/// pending row past its expiry is reported as `expired` without a write.
pub fn read_invitation_status(
    client: &mut Client,
    identity: &VerifiedIdentity,
    account: &IdentityAccount,
    person_key: &str,
) -> Result<InvitationStatusView, ProfileInvitationError> {
    let person_key = person_key.trim();
    authorize_person_invitation_in(client, identity, account, person_key)?;
    let person_id = person_id_for_key(client, person_key)?;

    let Some(record) = latest_invitation(client, person_id)? else {
        return Ok(InvitationStatusView {
            person_key: person_key.to_string(),
            invitation_id: None,
            invitation_status: None,
            delivery_state: ProfileDeliveryState::NotSent,
            delivery_provider: None,
            last_delivery_error: None,
            delivery_attempts: 0,
            expires_at: None,
            accepted_at: None,
        });
    };

    Ok(InvitationStatusView {
        person_key: person_key.to_string(),
        invitation_id: Some(record.invitation_id),
        invitation_status: Some(status_code(record.status)),
        delivery_state: record.delivery_state,
        delivery_provider: record.delivery_provider,
        last_delivery_error: record.last_delivery_error,
        delivery_attempts: record.delivery_attempts,
        expires_at: record.expires_at,
        accepted_at: record.accepted_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_is_viewer() {
        assert_eq!(PREPARE_ACCESS, "viewer");
    }

    #[test]
    fn invalid_address_error_carries_no_address() {
        let cases = ["plain", "a@", "@b.test", "a b@c.test", "a@b@c.test", ""];

        for case in cases {
            let err = validate_draft_address(case).unwrap_err();
            let display = err.to_string();
            if !case.is_empty() {
                assert!(!display.contains(case), "the error text leaked the input");
            }
            assert_eq!(err, ProfileInvitationError::InvalidAddress);
        }
    }

    #[test]
    fn valid_address_is_accepted_and_trimmed() {
        assert_eq!(
            validate_draft_address("  person@example.test ").unwrap(),
            "person@example.test"
        );
    }

    #[test]
    fn masked_contact_never_carries_the_local_part() {
        let masked = mask_email("candidate@example.test");
        assert_eq!(masked, "c•••@example.test");
        assert!(!masked.contains("andidate"));
    }

    #[test]
    fn missing_draft_is_a_blocker() {
        let blockers = build_blockers(&[Reason::VerifiedLivingEvidence], false);
        assert!(blockers
            .iter()
            .any(|blocker| blocker.code == "missing_contact"));
    }

    #[test]
    fn warnings_do_not_block() {
        let blockers = build_blockers(
            &[
                Reason::VerifiedLivingEvidence,
                Reason::MissingDob,
                Reason::UsernameCollision,
                Reason::MissingVerifiedContact,
            ],
            true,
        );
        assert!(blockers.is_empty(), "{blockers:?}");
    }

    #[test]
    fn linked_reason_is_a_blocker() {
        let blockers = build_blockers(&[Reason::AlreadyLinked], true);
        assert!(blockers
            .iter()
            .any(|blocker| blocker.code == "already_linked"));
    }

    #[test]
    fn blockers_are_deduplicated() {
        let blockers = build_blockers(&[Reason::Disputed, Reason::Disputed], true);
        assert_eq!(blockers.len(), 1);
    }

    #[test]
    fn refusal_error_is_safe() {
        let err = ProfileInvitationError::Refused("wrong_branch");
        assert!(err.to_string().contains("wrong_branch"));
    }

    #[test]
    fn database_error_text_is_generic() {
        let err = ProfileInvitationError::Database("boom".into());
        assert_eq!(
            err.to_string(),
            "profile invitation database operation failed"
        );
    }
}
