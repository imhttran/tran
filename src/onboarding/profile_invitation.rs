//! Restricted draft contact and invitation preparation for one person (PIR-003).
//!
//! Preparation is a review step, never a send. The module offers three
//! operations:
//!
//! * [`read_invitation_state`] reads the family-admin decision, eligibility,
//!   invitation/delivery state, linked-account status, proposed username, and a
//!   plain-language blocker list;
//! * [`save_contact_draft`] stores a proposed address in the restricted
//!   `onboarding_invitation_contact_draft` table only;
//! * [`prepare_invitation`] rechecks authorization, eligibility, link state,
//!   duplicate invitation, and the draft contact, and returns a review payload.
//!
//! No operation here imports a delivery adapter, writes
//! `onboarding_recipient_contact`, or marks an invitation sent. An existing
//! pending invitation is reused through [`crate::onboarding::invitation::create`],
//! so a duplicate is reported, not duplicated. Only a masked address leaves the
//! module in a response; the error text never carries an address.

use postgres::Client;
use thiserror::Error;

use crate::identity::VerifiedIdentity;
use crate::onboarding::account_map::IdentityAccount;
use crate::onboarding::candidate::{
    load_name_parts, mask_email, reclassify_person, CandidateError, Classification, Reason,
};
use crate::onboarding::family_admin::{
    authorize_person_invitation_in, FamilyAdminCheckError, FamilyAdminDecision,
};
use crate::onboarding::invitation::{create, InvitationError};
use crate::onboarding::username::candidate_username;

/// The invitation lifetime used when preparation has to issue a placeholder
/// invitation row. Preparation never sends; the row only reserves the pending
/// slot the review payload names.
const PREPARE_INVITATION_TTL_SECS: u64 = 7 * 24 * 60 * 60;

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

/// Reads the pending invitation and delivery state for one person.
fn read_invitation_state_view(
    client: &mut Client,
    person_id: i64,
) -> Result<InvitationStateView, ProfileInvitationError> {
    let pending = client.query_opt(
        "SELECT invitation_id, expires_at::text
           FROM onboarding_invitation
          WHERE person_id = $1
            AND status = 'pending'
            AND expires_at IS NOT NULL
            AND expires_at > now()
          ORDER BY created_at DESC, invitation_id DESC
          LIMIT 1",
        &[&person_id],
    )?;
    let (invitation_id, expires_at) = match pending {
        Some(row) => (Some(row.get(0)), Some(row.get(1))),
        None => (None, None),
    };

    // The delivery state reflects a batch item when one exists. Preparation and
    // draft contact never write a delivery state, so a person without a batch
    // item reports `not_sent`.
    let delivery: Option<String> = client
        .query_opt(
            "SELECT delivery_status FROM onboarding_batch_item
              WHERE person_id = $1
              ORDER BY batch_id DESC LIMIT 1",
            &[&person_id],
        )?
        .map(|row| row.get(0));
    let delivery_state = match delivery.as_deref() {
        Some("queued") => "queued",
        Some("sent") => "sent",
        Some("accepted") => "accepted",
        Some("failed") => "failed",
        Some("expired") => "expired",
        _ => "not_sent",
    };

    Ok(InvitationStateView {
        pending: invitation_id.is_some(),
        invitation_id,
        expires_at,
        delivery_state,
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
        std::time::Duration::from_secs(PREPARE_INVITATION_TTL_SECS),
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
