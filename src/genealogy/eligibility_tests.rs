//! Unit tests for the Head of Family eligibility decision (TRAN-002F).
//!
//! These tests are pure: they build synthetic inputs and never open a database.

use super::*;

fn evidence(id: i64, value: &str) -> CandidateEvidence {
    CandidateEvidence {
        assertion_id: id,
        value_text: Some(value.to_string()),
    }
}

fn eligible_input() -> EligibilityInput {
    EligibilityInput {
        person_key: "p-1".to_string(),
        branch_key: "branch-3".to_string(),
        branch: Some(evidence(11, "branch-3")),
        gender: Some(evidence(12, "male")),
        birth_order: Some(evidence(13, "1")),
        living: Some(evidence(14, "living")),
        deceased_first_born: None,
        contested_first_born: None,
    }
}

#[test]
fn fully_verified_case_is_eligible_with_supporting_ids() {
    let decision = decide(&eligible_input());
    assert_eq!(decision.decision, "eligible");
    assert!(decision.is_eligible());
    assert_eq!(decision.reason_code, ReasonCode::Eligible);
    assert_eq!(decision.supporting_assertion_ids, vec![11, 12, 13, 14]);
    assert!(!decision.requires_succession_policy);
}

#[test]
fn missing_branch_is_undetermined() {
    let mut input = eligible_input();
    input.branch = None;
    let decision = decide(&input);
    assert_eq!(decision.decision, "undetermined");
    assert_eq!(decision.reason_code, ReasonCode::MissingBranch);
}

#[test]
fn branch_mismatch_is_treated_as_missing() {
    let mut input = eligible_input();
    input.branch = Some(evidence(11, "branch-9"));
    let decision = decide(&input);
    assert_eq!(decision.reason_code, ReasonCode::MissingBranch);
}

#[test]
fn missing_gender_is_undetermined() {
    let mut input = eligible_input();
    input.gender = None;
    let decision = decide(&input);
    assert_eq!(decision.decision, "undetermined");
    assert_eq!(decision.reason_code, ReasonCode::MissingGender);
}

#[test]
fn non_male_designation_is_undetermined() {
    let mut input = eligible_input();
    input.gender = Some(evidence(12, "female"));
    let decision = decide(&input);
    assert_eq!(decision.reason_code, ReasonCode::MissingGender);
}

#[test]
fn missing_birth_order_is_undetermined() {
    let mut input = eligible_input();
    input.birth_order = None;
    let decision = decide(&input);
    assert_eq!(decision.decision, "undetermined");
    assert_eq!(decision.reason_code, ReasonCode::MissingBirthOrder);
}

#[test]
fn missing_living_evidence_is_undetermined() {
    let mut input = eligible_input();
    input.living = None;
    let decision = decide(&input);
    assert_eq!(decision.decision, "undetermined");
    assert_eq!(decision.reason_code, ReasonCode::MissingLivingEvidence);
}

#[test]
fn missing_death_date_never_implies_living() {
    // No living assertion exists. A missing death date (deceased evidence is
    // also absent) never yields eligible or living.
    let mut input = eligible_input();
    input.living = None;
    input.deceased_first_born = None;
    let decision = decide(&input);
    assert!(!decision.is_eligible());
    assert_eq!(decision.reason_code, ReasonCode::MissingLivingEvidence);
}

#[test]
fn not_first_born_is_undetermined() {
    let mut input = eligible_input();
    input.birth_order = Some(evidence(13, "2"));
    let decision = decide(&input);
    assert_eq!(decision.decision, "undetermined");
    assert_eq!(decision.reason_code, ReasonCode::NotFirstBorn);
}

#[test]
fn deceased_first_born_requests_succession_policy_without_successor() {
    let mut input = eligible_input();
    input.deceased_first_born = Some(evidence(15, "deceased"));
    let decision = decide(&input);
    assert_eq!(decision.decision, "undetermined");
    assert_eq!(decision.reason_code, ReasonCode::DeceasedFirstBorn);
    assert!(decision.requires_succession_policy);
    // No successor assertion id is selected beyond the supporting evidence.
    assert!(decision.supporting_assertion_ids.contains(&15));
}

#[test]
fn contested_first_born_requests_succession_policy_without_successor() {
    let mut input = eligible_input();
    input.contested_first_born = Some(evidence(16, "disputed"));
    let decision = decide(&input);
    assert_eq!(decision.decision, "undetermined");
    assert_eq!(decision.reason_code, ReasonCode::ContestedFirstBorn);
    assert!(decision.requires_succession_policy);
}

#[test]
fn unknown_person_is_undetermined() {
    let input = EligibilityInput::default();
    let decision = decide(&input);
    assert_eq!(decision.decision, "undetermined");
    assert_eq!(decision.reason_code, ReasonCode::UnknownPerson);
}

#[test]
fn decision_carries_no_permission_surface() {
    // The serialized decision has exactly these keys and no permission, role, or
    // capability field.
    let decision = decide(&eligible_input());
    let json = serde_json::to_value(&decision).unwrap();
    let object = json.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "decision",
            "reason_code",
            "requires_succession_policy",
            "supporting_assertion_ids"
        ]
    );
    for forbidden in ["permission", "permissions", "role", "roles", "capabilities"] {
        assert!(!object.contains_key(forbidden));
    }
}
