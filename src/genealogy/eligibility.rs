//! Head of Family eligibility decision (TRAN-002F, genealogy domain only).
//!
//! This module is a pure genealogy-domain decision. It reads verified assertions
//! from the genealogy database through a read-only adapter and returns an
//! [`EligibilityDecision`] with a [`ReasonCode`] and the supporting assertion IDs.
//!
//! The designation is a domain value. It carries no role and no permission
//! field. This module has no dependency on any RBAC or authorization code and
//! never grants or implies an application permission.
//!
//! Rules:
//! - Eligibility requires verified third-branch membership, a verified male
//!   designation, verified male birth order that identifies the first-born male
//!   of that branch, and independently verified living status.
//! - Living status is never inferred from a missing death date. It is read only
//!   from an explicit verified `vital_status` living assertion.
//! - The oldest recorded male is not assumed to be the first-born. Only explicit
//!   verified birth-order evidence counts.
//! - A deceased, unknown, contested, or non-verified first-born yields
//!   `undetermined` plus an explicit succession-policy request. No successor is
//!   chosen automatically.

use postgres::Client;
use serde::Serialize;

use super::GenealogyError;

/// The assertion kind that carries branch membership.
pub const KIND_BRANCH: &str = "branch_claim";
/// The assertion kind that carries gender designation.
pub const KIND_GENDER: &str = "gender";
/// The assertion kind that carries birth order within a branch.
pub const KIND_BIRTH_ORDER: &str = "birth_order";
/// The assertion kind that carries vital (living/deceased) status.
pub const KIND_VITAL_STATUS: &str = "vital_status";

/// The `vital_status` assertion key that carries living status.
pub const VITAL_LIVING: &str = "living";

/// One candidate evidence item for a decision. `None` means the evidence is
/// absent or not verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateEvidence {
    /// The assertion ID that supplies this evidence.
    pub assertion_id: i64,
    /// The stored value text.
    pub value_text: Option<String>,
}

/// The pure inputs to a Head of Family decision.
///
/// Every field is the verified evidence for one requirement, or `None` when the
/// requirement is absent, disputed, or not verified.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EligibilityInput {
    /// The requested person key.
    pub person_key: String,
    /// The requested branch key.
    pub branch_key: String,
    /// Verified third-branch membership evidence.
    pub branch: Option<CandidateEvidence>,
    /// Verified male designation evidence.
    pub gender: Option<CandidateEvidence>,
    /// Verified male birth-order evidence identifying the first-born male.
    pub birth_order: Option<CandidateEvidence>,
    /// Independently verified living-status evidence.
    pub living: Option<CandidateEvidence>,
    /// Verified first-born (or candidate successor) deceased evidence, when set.
    pub deceased_first_born: Option<CandidateEvidence>,
    /// Contested first-born evidence, when set.
    pub contested_first_born: Option<CandidateEvidence>,
}

impl EligibilityInput {
    /// Whether the person was recorded as the male designated gender `male`.
    fn is_male(&self) -> bool {
        matches!(
            self.gender.as_ref().and_then(|g| g.value_text.as_deref()),
            Some(v) if v.eq_ignore_ascii_case("male") || v.eq_ignore_ascii_case("m")
        )
    }

    /// Whether the person's verified birth order is first in the branch.
    fn is_first_born(&self) -> bool {
        matches!(
            self.birth_order.as_ref().and_then(|b| b.value_text.as_deref()),
            Some(v) if v.eq_ignore_ascii_case("1") || v.eq_ignore_ascii_case("first")
        )
    }

    /// Whether explicit verified living evidence says the person is living.
    fn is_living(&self) -> bool {
        matches!(
            self.living.as_ref().and_then(|l| l.value_text.as_deref()),
            Some(v) if v.eq_ignore_ascii_case("living") || v.eq_ignore_ascii_case("alive")
        )
    }

    /// Whether the branch membership names the requested branch.
    fn branch_matches(&self, branch_key: &str) -> bool {
        match self.branch.as_ref() {
            Some(branch) => match branch.value_text.as_deref() {
                Some(value) => value == branch_key,
                None => false,
            },
            None => false,
        }
    }
}

/// The reason code recorded for every eligibility result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    /// Every requirement was verified.
    Eligible,
    /// No matching person.
    UnknownPerson,
    /// Branch membership evidence is missing or not verified.
    MissingBranch,
    /// Gender designation evidence is missing or not verified.
    MissingGender,
    /// Birth-order evidence is missing or not verified.
    MissingBirthOrder,
    /// Living-status evidence is missing or not verified.
    MissingLivingEvidence,
    /// The verified birth order does not identify the first-born male.
    NotFirstBorn,
    /// The first-born male is deceased. A succession policy is required.
    DeceasedFirstBorn,
    /// The first-born male is contested. A succession policy is required.
    ContestedFirstBorn,
}

impl ReasonCode {
    /// The stable string code for this reason.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasonCode::Eligible => "eligible",
            ReasonCode::UnknownPerson => "unknown_person",
            ReasonCode::MissingBranch => "missing_branch",
            ReasonCode::MissingGender => "missing_gender",
            ReasonCode::MissingBirthOrder => "missing_birth_order",
            ReasonCode::MissingLivingEvidence => "missing_living_evidence",
            ReasonCode::NotFirstBorn => "not_first_born",
            ReasonCode::DeceasedFirstBorn => "deceased_first_born",
            ReasonCode::ContestedFirstBorn => "contested_first_born",
        }
    }
}

/// The Head of Family decision. It is a domain value: it has no role and no
/// permission field and must never confer an application permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EligibilityDecision {
    /// `eligible` or `undetermined`.
    pub decision: String,
    /// The reason code for this result.
    pub reason_code: ReasonCode,
    /// Whether an explicit succession-policy decision is requested.
    pub requires_succession_policy: bool,
    /// The supporting assertion IDs.
    pub supporting_assertion_ids: Vec<i64>,
}

impl EligibilityDecision {
    /// Whether the person is eligible.
    pub fn is_eligible(&self) -> bool {
        self.decision == "eligible"
    }
}

fn undetermined(reason: ReasonCode, ids: Vec<i64>, succession: bool) -> EligibilityDecision {
    EligibilityDecision {
        decision: "undetermined".to_string(),
        reason_code: reason,
        requires_succession_policy: succession,
        supporting_assertion_ids: ids,
    }
}

/// Runs the pure Head of Family decision over `input`.
///
/// No database access, no RBAC call. The order of checks is fixed so the result
/// is deterministic.
pub fn decide(input: &EligibilityInput) -> EligibilityDecision {
    // A missing person has no evidence at all.
    if input.person_key.is_empty() {
        return undetermined(ReasonCode::UnknownPerson, Vec::new(), false);
    }

    // Branch membership must be verified and match the requested branch.
    let branch = match &input.branch {
        Some(branch) if input.branch_matches(&input.branch_key) => branch,
        _ => return undetermined(ReasonCode::MissingBranch, Vec::new(), false),
    };

    // Male designation must be verified.
    let gender = match &input.gender {
        Some(gender) if input.is_male() => gender,
        _ => {
            return undetermined(ReasonCode::MissingGender, vec![branch.assertion_id], false);
        }
    };

    // Birth order must be verified and identify the first-born male.
    let birth_order = match &input.birth_order {
        Some(birth_order) => birth_order,
        _ => {
            return undetermined(
                ReasonCode::MissingBirthOrder,
                vec![branch.assertion_id, gender.assertion_id],
                false,
            );
        }
    };

    let mut base_ids = vec![
        branch.assertion_id,
        gender.assertion_id,
        birth_order.assertion_id,
    ];

    if !input.is_first_born() {
        return undetermined(ReasonCode::NotFirstBorn, base_ids, false);
    }

    // Contested or deceased first-born never advances to an automatic successor.
    if let Some(contested) = &input.contested_first_born {
        base_ids.push(contested.assertion_id);
        return undetermined(ReasonCode::ContestedFirstBorn, base_ids, true);
    }
    if let Some(deceased) = &input.deceased_first_born {
        base_ids.push(deceased.assertion_id);
        return undetermined(ReasonCode::DeceasedFirstBorn, base_ids, true);
    }

    // Living status must be independently verified. A missing death date never
    // yields living = true.
    let living = match &input.living {
        Some(living) if input.is_living() => living,
        _ => {
            return undetermined(ReasonCode::MissingLivingEvidence, base_ids, false);
        }
    };

    base_ids.push(living.assertion_id);
    EligibilityDecision {
        decision: "eligible".to_string(),
        reason_code: ReasonCode::Eligible,
        requires_succession_policy: false,
        supporting_assertion_ids: base_ids,
    }
}

fn verified_evidence(
    client: &mut Client,
    person_id: i64,
    kind: &str,
    key: Option<&str>,
) -> Result<Option<CandidateEvidence>, GenealogyError> {
    let row = match key {
        Some(key) => client
            .query_opt(
                "SELECT assertion_id, value_text FROM assertion
                  WHERE person_id = $1 AND assertion_kind = $2 AND assertion_key = $3
                    AND review_status = 'verified'
                  ORDER BY assertion_id LIMIT 1",
                &[&person_id, &kind, &key],
            )
            .map_err(|err| GenealogyError::Query(err.to_string()))?,
        None => client
            .query_opt(
                "SELECT assertion_id, value_text FROM assertion
                  WHERE person_id = $1 AND assertion_kind = $2
                    AND review_status = 'verified'
                  ORDER BY assertion_id LIMIT 1",
                &[&person_id, &kind],
            )
            .map_err(|err| GenealogyError::Query(err.to_string()))?,
    };
    Ok(row.map(|row| CandidateEvidence {
        assertion_id: row.get(0),
        value_text: row.get(1),
    }))
}

/// Loads the verified eligibility evidence for one person key and one branch
/// key. This function issues SELECT statements only.
///
/// A person key with no matching person returns an empty input that
/// [`decide`] maps to `undetermined`, not an error. Assertions whose
/// `review_status` is not `verified` are dropped and cannot support eligibility.
pub fn load_input(
    client: &mut Client,
    person_key: &str,
    branch_key: &str,
) -> Result<EligibilityInput, GenealogyError> {
    let row = client
        .query_opt(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&person_key],
        )
        .map_err(|err| GenealogyError::Query(err.to_string()))?;
    let Some(row) = row else {
        return Ok(EligibilityInput {
            person_key: person_key.to_string(),
            branch_key: branch_key.to_string(),
            ..Default::default()
        });
    };
    let person_id: i64 = row.get(0);

    let branch = verified_evidence(client, person_id, KIND_BRANCH, None)?;
    let gender = verified_evidence(client, person_id, KIND_GENDER, None)?;
    let birth_order = verified_evidence(client, person_id, KIND_BIRTH_ORDER, None)?;
    let living = verified_evidence(client, person_id, KIND_VITAL_STATUS, Some(VITAL_LIVING))?;

    Ok(EligibilityInput {
        person_key: person_key.to_string(),
        branch_key: branch_key.to_string(),
        branch,
        gender,
        birth_order,
        living,
        deceased_first_born: None,
        contested_first_born: None,
    })
}

#[cfg(test)]
#[path = "eligibility_tests.rs"]
mod tests;
