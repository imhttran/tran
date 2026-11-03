//! Server-side family-admin authorization rule (PIR-002).
//!
//! The rule is independent of HTTP handlers so every invitation operation can
//! reuse it. It answers one question: may this verified account invite this
//! target person?
//!
//! Trust model:
//!
//! * A system administrator (`admin` role) may always invite, after the target
//!   person key is confirmed to exist.
//! * Otherwise, the account is authorized only when it owns a reviewed
//!   `onboarding_person_account_link` for the exact `(account_id,
//!   identity_fingerprint)` pair, holds a current `head_of_family_appointment`
//!   with `status = 'active'` and `current_eligibility = Eligible`, and the
//!   target person either is the caller's own linked person or carries one
//!   current verified `branch_claim` equal to the appointment `branch_key`.
//! * A title, name, label, nomination text, request body, or client-supplied
//!   role is never authority.
//! * The rule never grants a role and never writes a link or a role. It issues
//!   SELECT statements only.
//!
//! Read shapes consumed (and only these):
//!
//! * [`crate::onboarding::link::link_for_account`] — the reviewed
//!   `(account_id, identity_fingerprint)` link row. A revoked link was deleted,
//!   so the `onboarding_link_audit` `revoked` row is read to tell a revoked
//!   link apart from a link that never existed.
//! * [`crate::genealogy::appointments::list_active_appointments`] row shape —
//!   the caller's `head_of_family_appointment` columns (`person_key`,
//!   `branch_key`, `status`) plus the re-evaluated `current_eligibility`.
//! * The `branch_claim` predicate used by
//!   `crate::genealogy::appointments::search_nomination_people` — current
//!   verified branch labels for the target person.
//!
//! Allow codes: `system_admin`, `family_head_branch`, `self_link`.
//! Deny codes: `missing_link`, `revoked_link`, `ineligible_appointment`,
//! `revoked_appointment`, `ineligible_holder`, `unknown_target_person`,
//! `wrong_branch`, `ambiguous_branch`, `unrelated_person`.
//! Database failures are reported as [`FamilyAdminError::Database`], never as a
//! policy refusal.
//!
//! The link and appointment checks run in the same read transaction as the
//! caller's operation: [`authorize_person_invitation_in`] reuses a caller
//! transaction, while [`authorize_person_invitation`] opens one itself.

use postgres::GenericClient;
use thiserror::Error;

use crate::genealogy::appointments::AppointmentRow;
use crate::genealogy::current::CURRENT_ASSERTION;
use crate::genealogy::eligibility::ReasonCode;
use crate::genealogy::GenealogyError;
use crate::identity::{Role, VerifiedIdentity};
use crate::onboarding::account_map::IdentityAccount;
use crate::onboarding::link::{LinkError, LinkRecord};

/// The result of a successful family-admin authorization check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilyAdminDecision {
    /// The caller may invite the target person.
    Allow(FamilyAdminAllow),
}

impl FamilyAdminDecision {
    /// The stable string code for this decision.
    pub fn as_str(self) -> &'static str {
        match self {
            FamilyAdminDecision::Allow(reason) => reason.as_str(),
        }
    }
}

/// Why the rule allowed the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilyAdminAllow {
    /// The caller holds the system `admin` role.
    SystemAdmin,
    /// The caller's active Head of Family appointment covers the target's
    /// branch. A caller may invite themselves only through this branch scope,
    /// never through a separate self permission.
    FamilyHeadBranch,
}

impl FamilyAdminAllow {
    /// The stable string code for the allow reason.
    pub fn as_str(self) -> &'static str {
        match self {
            FamilyAdminAllow::SystemAdmin => "system_admin",
            FamilyAdminAllow::FamilyHeadBranch => "family_head_branch",
        }
    }
}

/// A distinct refusal for every family-admin denial case.
///
/// Each variant maps to a stable snake_case code so the API can return a safe,
/// plain-language reason without leaking the underlying record.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum FamilyAdminRefusal {
    /// No reviewed link exists for this account and none was revoked.
    #[error("no reviewed person link for this account")]
    MissingLink,
    /// The account's link was revoked, so it grants no authority.
    #[error("the person link for this account was revoked")]
    RevokedLink,
    /// The caller has no Head of Family appointment at all.
    #[error("no head-of-family appointment")]
    IneligibleAppointment,
    /// The caller's appointment is no longer active (revoked or suspended).
    #[error("the head-of-family appointment is no longer active")]
    RevokedAppointment,
    /// The caller's active appointment holder is not currently eligible.
    #[error("the head-of-family appointment holder is not eligible")]
    IneligibleHolder,
    /// The target person does not exist.
    #[error("unknown target person")]
    UnknownTargetPerson,
    /// The target person is outside the caller's branch and is not the caller.
    #[error("the target person is outside the caller's family branch")]
    WrongBranch,
    /// The target person has more than one current verified branch claim.
    #[error("the target person's family branch is ambiguous")]
    AmbiguousBranch,
    /// The target person has no current verified branch claim and is not the caller.
    #[error("the target person is not related to the caller's branch")]
    UnrelatedPerson,
}

impl FamilyAdminRefusal {
    /// A stable snake_case code for the refusal.
    pub fn as_str(&self) -> &'static str {
        match self {
            FamilyAdminRefusal::MissingLink => "missing_link",
            FamilyAdminRefusal::RevokedLink => "revoked_link",
            FamilyAdminRefusal::IneligibleAppointment => "ineligible_appointment",
            FamilyAdminRefusal::RevokedAppointment => "revoked_appointment",
            FamilyAdminRefusal::IneligibleHolder => "ineligible_holder",
            FamilyAdminRefusal::UnknownTargetPerson => "unknown_target_person",
            FamilyAdminRefusal::WrongBranch => "wrong_branch",
            FamilyAdminRefusal::AmbiguousBranch => "ambiguous_branch",
            FamilyAdminRefusal::UnrelatedPerson => "unrelated_person",
        }
    }
}

/// Errors raised by [`authorize_person_invitation`].
///
/// A database failure is reported here and never converted into a policy
/// refusal, so an operational fault cannot be mistaken for a legitimate denial.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FamilyAdminError {
    /// A database read failed.
    #[error("family-admin database read failed: {0}")]
    Database(String),
}

impl From<LinkError> for FamilyAdminError {
    fn from(err: LinkError) -> Self {
        match err {
            LinkError::Database(message) => FamilyAdminError::Database(message),
            other => FamilyAdminError::Database(other.to_string()),
        }
    }
}

impl From<GenealogyError> for FamilyAdminError {
    fn from(err: GenealogyError) -> Self {
        FamilyAdminError::Database(err.to_string())
    }
}

impl From<postgres::Error> for FamilyAdminError {
    fn from(err: postgres::Error) -> Self {
        FamilyAdminError::Database(err.to_string())
    }
}

/// The checked rule: `Ok(Allow)` may invite, `Ok` never refuses.
///
/// A policy refusal is `Err(FamilyAdminRefusal)`; an operational failure is
/// `Err(FamilyAdminError)`.
pub type FamilyAdminOutcome = Result<FamilyAdminDecision, FamilyAdminCheckError>;

/// The single error type of the checked rule: either a distinct refusal or a
/// database failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FamilyAdminCheckError {
    /// The rule denied the request with a safe, distinct reason.
    #[error("{0}")]
    Refused(FamilyAdminRefusal),
    /// A database read failed.
    #[error("{0}")]
    Database(FamilyAdminError),
}

impl From<FamilyAdminRefusal> for FamilyAdminCheckError {
    fn from(reason: FamilyAdminRefusal) -> Self {
        FamilyAdminCheckError::Refused(reason)
    }
}

impl From<FamilyAdminError> for FamilyAdminCheckError {
    fn from(err: FamilyAdminError) -> Self {
        FamilyAdminCheckError::Database(err)
    }
}

impl From<postgres::Error> for FamilyAdminCheckError {
    fn from(err: postgres::Error) -> Self {
        FamilyAdminCheckError::Database(FamilyAdminError::from(err))
    }
}

impl From<GenealogyError> for FamilyAdminCheckError {
    fn from(err: GenealogyError) -> Self {
        FamilyAdminCheckError::Database(FamilyAdminError::from(err))
    }
}

impl From<LinkError> for FamilyAdminCheckError {
    fn from(err: LinkError) -> Self {
        FamilyAdminCheckError::Database(FamilyAdminError::from(err))
    }
}

impl FamilyAdminCheckError {
    /// Returns the distinct policy refusal, if this error is a refusal.
    pub fn refusal(&self) -> Option<FamilyAdminRefusal> {
        match self {
            FamilyAdminCheckError::Refused(reason) => Some(*reason),
            FamilyAdminCheckError::Database(_) => None,
        }
    }

    /// A stable snake_case code for the refusal, or `database_error`.
    pub fn as_str(&self) -> &'static str {
        match self {
            FamilyAdminCheckError::Refused(reason) => reason.as_str(),
            FamilyAdminCheckError::Database(_) => "database_error",
        }
    }
}

/// The caller's reviewed link state, as read from the genealogy database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerLink {
    /// A reviewed link row exists.
    Reviewed(LinkRecord),
    /// No link row exists and no revocation was recorded.
    Missing,
    /// A link existed and was revoked.
    Revoked,
}

/// The caller's Head of Family appointment state, as read from the genealogy
/// database. `None` means the caller holds no appointment at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerAppointment {
    /// The exact appointment branch key.
    pub branch_key: String,
    /// The stored status, for example `active`, `suspended`, or `revoked`.
    pub status: String,
    /// The holder's eligibility re-evaluated from current verified evidence.
    pub current_eligibility: ReasonCode,
}

impl CallerAppointment {
    fn from_row(row: &AppointmentRow) -> Self {
        CallerAppointment {
            branch_key: row.branch_key.clone(),
            status: row.status.clone(),
            current_eligibility: row.current_eligibility,
        }
    }
}

/// The complete set of facts the pure decision consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyAdminInput {
    /// Whether the verified caller holds the system `admin` role.
    pub caller_is_system_admin: bool,
    /// The caller's reviewed link state.
    pub link: CallerLink,
    /// The caller's appointment, if the caller holds one.
    pub appointment: Option<CallerAppointment>,
    /// Whether the target person key resolves to a person row.
    pub target_exists: bool,
    /// The target person's current verified branch labels.
    pub target_branches: Vec<String>,
}

impl FamilyAdminInput {
    /// Builds the input for a system administrator: no other fact is needed.
    pub fn system_admin() -> Self {
        FamilyAdminInput {
            caller_is_system_admin: true,
            link: CallerLink::Missing,
            appointment: None,
            target_exists: true,
            target_branches: Vec::new(),
        }
    }
}

/// Runs the pure family-admin decision over already-read facts.
///
/// The order of checks is fixed so the result is deterministic and every
/// refusal maps to exactly one reason. This function never reads a database,
/// never writes, and never constructs a role value.
pub fn decide(input: &FamilyAdminInput) -> FamilyAdminOutcome {
    // 1. The target person must exist before any allow is granted, so the admin
    //    path cannot authorize an invitation to an unknown person.
    if !input.target_exists {
        return Err(FamilyAdminRefusal::UnknownTargetPerson.into());
    }

    // 2. System admin is allowed, with no link or appointment facts.
    if input.caller_is_system_admin {
        return Ok(FamilyAdminDecision::Allow(FamilyAdminAllow::SystemAdmin));
    }

    // 3. A reviewed link must exist, and it must not be revoked.
    match &input.link {
        CallerLink::Reviewed(_) => {}
        CallerLink::Revoked => return Err(FamilyAdminRefusal::RevokedLink.into()),
        CallerLink::Missing => return Err(FamilyAdminRefusal::MissingLink.into()),
    };

    // 4. The caller must hold an active, eligible Head of Family appointment.
    //    A reviewed link alone never authorizes an invitation.
    let appointment = match &input.appointment {
        Some(appointment) => appointment,
        None => return Err(FamilyAdminRefusal::IneligibleAppointment.into()),
    };
    if appointment.status != "active" {
        return Err(FamilyAdminRefusal::RevokedAppointment.into());
    }
    if appointment.current_eligibility != ReasonCode::Eligible {
        return Err(FamilyAdminRefusal::IneligibleHolder.into());
    }

    // 5. The target must carry exactly one current verified branch claim.
    //    Deduplicate defensively so two distinct labels cannot be silently
    //    picked.
    let mut labels = input.target_branches.clone();
    labels.sort();
    labels.dedup();
    match labels.len() {
        0 => return Err(FamilyAdminRefusal::UnrelatedPerson.into()),
        1 => {}
        _ => return Err(FamilyAdminRefusal::AmbiguousBranch.into()),
    }

    // 6. The appointment branch must equal the target's single verified claim.
    //    This applies to every target, including the caller's own person: there
    //    is no separate self-invitation permission.
    if labels[0] != appointment.branch_key {
        return Err(FamilyAdminRefusal::WrongBranch.into());
    }

    Ok(FamilyAdminDecision::Allow(
        FamilyAdminAllow::FamilyHeadBranch,
    ))
}

/// Splits a caller's active appointments into the one that backs the rule, if
/// any. Only an `active` row is considered; the row's `current_eligibility` is
/// carried forward so the pure decision can refuse an ineligible holder.
pub fn caller_appointment(caller_key: &str, rows: &[AppointmentRow]) -> Option<CallerAppointment> {
    rows.iter()
        .find(|row| row.person_key == caller_key && row.status == "active")
        .map(CallerAppointment::from_row)
}

/// Returns the current verified branch labels for one target person key.
///
/// The SQL mirrors the `branch_claim` predicate used by
/// `appointments::search_nomination_people`. It issues a SELECT only.
pub fn target_branch_claims<C: GenericClient>(
    client: &mut C,
    target_person_key: &str,
) -> Result<Vec<String>, FamilyAdminError> {
    let current = CURRENT_ASSERTION;
    let sql = format!(
        "SELECT DISTINCT COALESCE(b.branch_label, a.value_text) AS label \
           FROM assertion a \
           JOIN person p ON p.person_id = a.person_id \
           LEFT JOIN family_branch b ON b.branch_key = a.value_text \
          WHERE p.person_key = $1 \
            AND a.assertion_kind = 'branch_claim' \
            AND a.review_status = 'verified' \
            AND ({current}) \
          ORDER BY label"
    );
    let rows = client.query(&sql, &[&target_person_key])?;
    Ok(rows
        .iter()
        .filter_map(|row| row.get::<_, Option<String>>(0))
        .collect())
}

/// Reads the reviewed link owned by one verified identity on any client or
/// transaction, and reports whether it was revoked.
///
/// This mirrors [`crate::onboarding::link::link_for_account`] so the link check
/// can share the caller's open transaction. When no active row exists, it reads
/// the `revoked` audit row so a revoked link yields [`FamilyAdminRefusal::RevokedLink`]
/// instead of a missing link. It issues SELECT statements only.
pub fn caller_link<C: GenericClient>(
    client: &mut C,
    account_id: i64,
    identity_fingerprint: &str,
) -> Result<CallerLink, FamilyAdminError> {
    let sql = "SELECT link_id, person_id, account_id, identity_fingerprint, linked_by \
                 FROM onboarding_person_account_link \
                WHERE account_id = $1 AND identity_fingerprint = $2";
    let row = client.query_opt(sql, &[&account_id, &identity_fingerprint])?;
    if let Some(row) = row {
        return Ok(CallerLink::Reviewed(LinkRecord {
            link_id: row.get(0),
            person_id: row.get(1),
            account_id: row.get(2),
            identity_fingerprint: row.get(3),
            linked_by: row.get(4),
        }));
    }
    let revoked = client.query_opt(
        "SELECT 1 FROM onboarding_link_audit \
          WHERE account_id = $1 AND identity_fingerprint = $2 AND decision = 'revoked' \
          LIMIT 1",
        &[&account_id, &identity_fingerprint],
    )?;
    Ok(if revoked.is_some() {
        CallerLink::Revoked
    } else {
        CallerLink::Missing
    })
}

/// Reports whether a person row exists for `person_key`.
pub fn target_person_exists<C: GenericClient>(
    client: &mut C,
    person_key: &str,
) -> Result<bool, FamilyAdminError> {
    let row = client.query_opt("SELECT 1 FROM person WHERE person_key = $1", &[&person_key])?;
    Ok(row.is_some())
}

/// Returns the `person_key` of a linked person id, if any.
pub fn linked_person_key<C: GenericClient>(
    client: &mut C,
    person_id: i64,
) -> Result<Option<String>, FamilyAdminError> {
    let row = client.query_opt(
        "SELECT person_key FROM person WHERE person_id = $1",
        &[&person_id],
    )?;
    Ok(row.map(|row| row.get(0)))
}

/// Authorizes one family-member invitation in the caller's read transaction.
///
/// Every check runs on `client`. When the caller already holds a transaction,
/// pass that transaction's client so the link, appointment, and branch reads
/// observe one consistent snapshot. The function issues SELECT statements only
/// and never grants a role. A database failure returns
/// [`FamilyAdminCheckError::Database`], never a policy refusal.
pub fn authorize_person_invitation_in<C: GenericClient>(
    client: &mut C,
    identity: &VerifiedIdentity,
    account: &IdentityAccount,
    target_person_key: &str,
) -> FamilyAdminOutcome {
    // The target must exist before any allow is granted.
    let target_exists = target_person_exists(client, target_person_key)?;
    if !target_exists {
        return Err(FamilyAdminRefusal::UnknownTargetPerson.into());
    }

    // A system admin is allowed after the target existence check and before any
    // link or appointment read.
    if identity.has_role(Role::Admin) {
        return Ok(FamilyAdminDecision::Allow(FamilyAdminAllow::SystemAdmin));
    }

    let link = caller_link(client, account.account_id, &account.identity_fingerprint)?;
    let caller_key = match &link {
        CallerLink::Reviewed(record) => match linked_person_key(client, record.person_id)? {
            Some(key) => key,
            // The link points at a person row that no longer exists.
            None => return Err(FamilyAdminRefusal::UnknownTargetPerson.into()),
        },
        CallerLink::Revoked => return Err(FamilyAdminRefusal::RevokedLink.into()),
        CallerLink::Missing => return Err(FamilyAdminRefusal::MissingLink.into()),
    };

    let appointment = caller_appointment_for(client, &caller_key)?;

    let target_branches = target_branch_claims(client, target_person_key)?;

    decide(&FamilyAdminInput {
        caller_is_system_admin: false,
        link,
        appointment,
        target_exists,
        target_branches,
    })
}

/// Resolves the caller's appointment for the pure decision.
///
/// An active appointment is returned as read. When no active appointment exists,
/// the latest row is inspected so a revoked or suspended appointment yields
/// [`FamilyAdminRefusal::RevokedAppointment`] rather than a missing one.
fn caller_appointment_for<C: GenericClient>(
    client: &mut C,
    caller_key: &str,
) -> Result<Option<CallerAppointment>, FamilyAdminCheckError> {
    let active = active_appointments_for(client, caller_key)?;
    if let Some(appointment) = caller_appointment(caller_key, &active) {
        return Ok(Some(appointment));
    }
    // No active row: report a revoked appointment if one is recorded.
    let row = client.query_opt(
        "SELECT branch_key, status \
           FROM head_of_family_appointment \
          WHERE person_key = $1 ORDER BY appointment_id DESC LIMIT 1",
        &[&caller_key],
    )?;
    match row {
        Some(row) => {
            let branch_key: String = row.get(0);
            let status: String = row.get(1);
            let input = crate::genealogy::eligibility::load_input(client, caller_key, &branch_key)?;
            Ok(Some(CallerAppointment {
                branch_key,
                status,
                current_eligibility: crate::genealogy::eligibility::decide(&input).reason_code,
            }))
        }
        None => Ok(None),
    }
}

/// Reads the caller's active appointment rows with the same columns and
/// eligibility recomputation as `appointments::list_active_appointments`, but
/// filters on one person key so the caller sees only their own authority.
fn active_appointments_for<C: GenericClient>(
    client: &mut C,
    caller_key: &str,
) -> Result<Vec<AppointmentRow>, FamilyAdminCheckError> {
    let rows = client.query(
        "SELECT a.appointment_id, a.person_key, p.display_name, a.branch_key, \
                a.approved_by, a.evidence_assertion_ids::text, a.status, a.appointed_at::text \
           FROM head_of_family_appointment a \
           JOIN person p ON p.person_key = a.person_key \
          WHERE a.status = 'active' AND a.person_key = $1",
        &[&caller_key],
    )?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let evidence: String = row.get(5);
        let person_key: String = row.get(1);
        let branch_key: String = row.get(3);
        let input = crate::genealogy::eligibility::load_input(client, &person_key, &branch_key)?;
        out.push(AppointmentRow {
            appointment_id: row.get(0),
            person_key,
            display_name: row.get(2),
            branch_key,
            approved_by: row.get(4),
            evidence_assertion_ids: serde_json::from_str(&evidence).unwrap_or_default(),
            status: row.get(6),
            appointed_at: row.get(7),
            current_eligibility: crate::genealogy::eligibility::decide(&input).reason_code,
        });
    }
    Ok(out)
}

/// Authorizes one family-member invitation in a new read transaction.
///
/// The link, appointment, and branch reads run inside one transaction so they
/// observe a consistent snapshot. Callers that already hold a transaction must
/// use [`authorize_person_invitation_in`] instead. A transaction start failure
/// is reported as [`FamilyAdminCheckError::Database`], not a policy refusal.
pub fn authorize_person_invitation(
    client: &mut postgres::Client,
    identity: &VerifiedIdentity,
    account: &IdentityAccount,
    target_person_key: &str,
) -> FamilyAdminOutcome {
    let mut tx = client.transaction().map_err(|err| {
        FamilyAdminCheckError::Database(FamilyAdminError::Database(err.to_string()))
    })?;
    let outcome = authorize_person_invitation_in(&mut tx, identity, account, target_person_key);
    // Read-only: always roll back so the check cannot mutate any row.
    let _ = tx.rollback();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> LinkRecord {
        LinkRecord {
            link_id: 1,
            person_id: 42,
            account_id: 7,
            identity_fingerprint: "fp".to_string(),
            linked_by: "reviewer".to_string(),
        }
    }

    fn appointment(branch_key: &str, status: &str, eligible: bool) -> CallerAppointment {
        CallerAppointment {
            branch_key: branch_key.to_string(),
            status: status.to_string(),
            current_eligibility: if eligible {
                ReasonCode::Eligible
            } else {
                ReasonCode::MissingLivingEvidence
            },
        }
    }

    fn base_input() -> FamilyAdminInput {
        FamilyAdminInput {
            caller_is_system_admin: false,
            link: CallerLink::Reviewed(link()),
            appointment: Some(appointment("branch-a", "active", true)),
            target_exists: true,
            target_branches: vec!["branch-a".to_string()],
        }
    }

    #[test]
    fn system_admin_is_allowed_without_any_facts() {
        let input = FamilyAdminInput::system_admin();
        let outcome = decide(&input);
        assert_eq!(
            outcome,
            Ok(FamilyAdminDecision::Allow(FamilyAdminAllow::SystemAdmin))
        );
        assert_eq!(outcome.unwrap().as_str(), "system_admin");
    }

    #[test]
    fn family_head_in_same_branch_is_allowed() {
        let outcome = decide(&base_input());
        assert_eq!(
            outcome,
            Ok(FamilyAdminDecision::Allow(
                FamilyAdminAllow::FamilyHeadBranch
            ))
        );
        assert_eq!(outcome.unwrap().as_str(), "family_head_branch");
    }

    #[test]
    fn a_family_head_may_invite_their_own_person_only_within_their_branch() {
        // The caller's own person is not a separate permission: the branch rule
        // is the only way a linked parent may invite themselves.
        let input = base_input();
        let outcome = decide(&input);
        assert_eq!(
            outcome,
            Ok(FamilyAdminDecision::Allow(
                FamilyAdminAllow::FamilyHeadBranch
            ))
        );
    }

    #[test]
    fn a_linked_person_with_an_appointment_in_another_branch_is_refused() {
        let mut input = base_input();
        input.target_branches = vec!["branch-z".to_string()];
        let outcome = decide(&input);
        assert_eq!(outcome, Err(FamilyAdminRefusal::WrongBranch.into()));
    }

    #[test]
    fn a_linked_person_without_an_appointment_cannot_invite_themselves() {
        // A reviewed link alone is no longer an invitation permission.
        let mut input = base_input();
        input.appointment = None;
        let outcome = decide(&input);
        assert_eq!(outcome.unwrap_err().as_str(), "ineligible_appointment");
    }

    #[test]
    fn a_linked_person_with_a_revoked_appointment_cannot_invite_themselves() {
        let mut input = base_input();
        input.appointment = Some(appointment("branch-a", "revoked", true));
        let outcome = decide(&input);
        assert_eq!(outcome.unwrap_err().as_str(), "revoked_appointment");
    }

    #[test]
    fn wrong_branch_is_refused() {
        let mut input = base_input();
        input.target_branches = vec!["branch-b".to_string()];
        let outcome = decide(&input);
        assert_eq!(outcome, Err(FamilyAdminRefusal::WrongBranch.into()));
        assert_eq!(outcome.unwrap_err().as_str(), "wrong_branch");
    }

    #[test]
    fn unrelated_person_is_refused() {
        let mut input = base_input();
        input.target_branches = Vec::new();
        let outcome = decide(&input);
        assert_eq!(outcome, Err(FamilyAdminRefusal::UnrelatedPerson.into()));
        assert_eq!(outcome.unwrap_err().as_str(), "unrelated_person");
    }

    #[test]
    fn ambiguous_branch_is_refused() {
        let mut input = base_input();
        input.target_branches = vec!["branch-a".to_string(), "branch-b".to_string()];
        let outcome = decide(&input);
        assert_eq!(outcome, Err(FamilyAdminRefusal::AmbiguousBranch.into()));
        assert_eq!(outcome.unwrap_err().as_str(), "ambiguous_branch");
    }

    #[test]
    fn revoked_appointment_is_refused() {
        let mut input = base_input();
        input.appointment = Some(appointment("branch-a", "revoked", true));
        let outcome = decide(&input);
        assert_eq!(outcome, Err(FamilyAdminRefusal::RevokedAppointment.into()));
        assert_eq!(outcome.unwrap_err().as_str(), "revoked_appointment");
    }

    #[test]
    fn suspended_appointment_is_refused() {
        let mut input = base_input();
        input.appointment = Some(appointment("branch-a", "suspended", true));
        let outcome = decide(&input);
        assert_eq!(outcome.unwrap_err().as_str(), "revoked_appointment");
    }

    #[test]
    fn ineligible_appointment_is_refused() {
        let mut input = base_input();
        input.appointment = Some(appointment("branch-a", "active", false));
        let outcome = decide(&input);
        assert_eq!(outcome, Err(FamilyAdminRefusal::IneligibleHolder.into()));
        assert_eq!(outcome.unwrap_err().as_str(), "ineligible_holder");
    }

    #[test]
    fn missing_appointment_is_refused_distinctly() {
        let mut input = base_input();
        input.appointment = None;
        let outcome = decide(&input);
        assert_eq!(
            outcome,
            Err(FamilyAdminRefusal::IneligibleAppointment.into())
        );
        assert_eq!(outcome.unwrap_err().as_str(), "ineligible_appointment");
    }

    #[test]
    fn missing_link_is_refused() {
        let mut input = base_input();
        input.link = CallerLink::Missing;
        let outcome = decide(&input);
        assert_eq!(outcome, Err(FamilyAdminRefusal::MissingLink.into()));
        assert_eq!(outcome.unwrap_err().as_str(), "missing_link");
    }

    #[test]
    fn revoked_link_is_refused_distinctly() {
        let mut input = base_input();
        input.link = CallerLink::Revoked;
        let outcome = decide(&input);
        assert_eq!(outcome, Err(FamilyAdminRefusal::RevokedLink.into()));
        assert_eq!(outcome.unwrap_err().as_str(), "revoked_link");
    }

    #[test]
    fn unknown_target_person_is_refused_even_for_admin() {
        let mut input = FamilyAdminInput::system_admin();
        input.target_exists = false;
        let outcome = decide(&input);
        assert_eq!(outcome.unwrap_err().as_str(), "unknown_target_person");
    }

    #[test]
    fn database_error_is_not_a_policy_refusal() {
        let err = FamilyAdminCheckError::Database(FamilyAdminError::Database("boom".into()));
        assert_eq!(err.refusal(), None);
        assert_eq!(err.as_str(), "database_error");
    }

    #[test]
    fn refused_codes_are_distinct() {
        let codes = [
            FamilyAdminRefusal::MissingLink,
            FamilyAdminRefusal::RevokedLink,
            FamilyAdminRefusal::IneligibleAppointment,
            FamilyAdminRefusal::RevokedAppointment,
            FamilyAdminRefusal::IneligibleHolder,
            FamilyAdminRefusal::UnknownTargetPerson,
            FamilyAdminRefusal::WrongBranch,
            FamilyAdminRefusal::AmbiguousBranch,
            FamilyAdminRefusal::UnrelatedPerson,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for code in codes {
            assert!(seen.insert(code.as_str()), "duplicate code for {code:?}");
        }
    }
}
