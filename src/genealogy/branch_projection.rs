//! Deterministic projection of a person's recorded branch membership.
//!
//! This module is the single projection of recorded branch claims. It reads only
//! current `branch_claim` (canonical) and `branch` (legacy imported) assertions
//! whose `value_text` is non-blank, resolves each claim to a `family_branch` by
//! an exact, case-insensitive, trimmed match on `branch_key` or `branch_label`,
//! and classifies the result with a pure function.
//!
//! Resolution never uses a surname, a relationship, an appointment, or a role,
//! and it never partial-matches. A claim that does not exactly match a recorded
//! key or label resolves to no branch.
//!
//! The load functions take [`postgres::GenericClient`] so a transaction can call
//! the projection inside an authorization-sensitive write.

use std::collections::BTreeSet;

use postgres::GenericClient;
use serde::Serialize;

use super::current::CURRENT_ASSERTION;
use super::GenealogyError;

/// The state of a person's recorded branch membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchState {
    /// No current claims.
    Missing,
    /// The recorded claims contradict one another.
    Conflicting,
    /// Exactly one distinct resolved branch is verified.
    Verified,
    /// Claims exist, but the verified rule does not hold.
    Unverified,
}

impl BranchState {
    /// The stable text form of this state.
    pub fn as_str(self) -> &'static str {
        match self {
            BranchState::Missing => "missing",
            BranchState::Conflicting => "conflicting",
            BranchState::Verified => "verified",
            BranchState::Unverified => "unverified",
        }
    }
}

/// One recorded branch claim, projected with its resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchClaim {
    /// The recorded assertion kind: `branch_claim` or `branch`.
    pub assertion_kind: String,
    /// The recorded claim text.
    pub value_text: String,
    /// The review status: `draft`, `unverified`, or `verified`.
    pub review_status: String,
    /// True when the review status is `verified`.
    pub verified: bool,
    /// The matched `family_branch` key, when the value resolves.
    pub branch_key: Option<String>,
    /// The matched `family_branch` label, when the value resolves.
    pub branch_label: Option<String>,
}

impl BranchClaim {
    /// Builds a verified flag from a review status.
    fn is_verified(&self) -> bool {
        self.review_status.eq_ignore_ascii_case("verified")
    }
}

/// A full projection of one person's recorded branch membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchProjection {
    /// The classified state.
    pub state: BranchState,
    /// The single verified branch key, present only when `state` is verified.
    pub verified_branch_key: Option<String>,
    /// Every current claim, in projection order.
    pub claims: Vec<BranchClaim>,
}

/// One catalogued family branch used to resolve a claim text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyBranch {
    /// The recorded branch key.
    pub branch_key: String,
    /// The recorded branch label.
    pub branch_label: String,
}

/// Normalizes text for exact, case-insensitive, trimmed comparison.
fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

/// Resolves one recorded claim text to a `family_branch`.
///
/// The match is exact after trimming and case folding, against either the key or
/// the label. A partial match, a surname, a relationship, an appointment, or a
/// role never resolves. When several rows match, the branch with the smallest key
/// wins deterministically.
fn resolve<'a>(value_text: &str, catalog: &'a [FamilyBranch]) -> Option<&'a FamilyBranch> {
    let needle = normalize(value_text);
    if needle.is_empty() {
        return None;
    }
    catalog
        .iter()
        .filter(|branch| {
            normalize(&branch.branch_key) == needle || normalize(&branch.branch_label) == needle
        })
        .min_by(|left, right| left.branch_key.cmp(&right.branch_key))
}

/// Classifies a set of raw claim records against a catalog of family branches.
///
/// This function is pure: it performs no I/O and is unit-testable without a
/// database.
///
/// The rules:
/// - `missing`: no current claims;
/// - `conflicting`: more than one distinct resolved `branch_key`, or no resolved
///   key and more than one distinct recorded text, or more than one distinct
///   verified text that does not resolve to one key;
/// - `verified`: exactly one distinct resolved `branch_key` among verified
///   claims, no conflicting state, and every verified claim resolves;
/// - `unverified`: claims exist but the verified rule does not hold.
pub fn classify(raw: &[(String, String, String)], catalog: &[FamilyBranch]) -> BranchProjection {
    let claims: Vec<BranchClaim> = raw
        .iter()
        .map(|(assertion_kind, value_text, review_status)| {
            let matched = resolve(value_text, catalog);
            let mut claim = BranchClaim {
                assertion_kind: assertion_kind.clone(),
                value_text: value_text.clone(),
                review_status: review_status.clone(),
                verified: false,
                branch_key: matched.map(|branch| branch.branch_key.clone()),
                branch_label: matched.map(|branch| branch.branch_label.clone()),
            };
            claim.verified = claim.is_verified();
            claim
        })
        .collect();

    let state = classify_state(&claims);
    let verified_branch_key = if state == BranchState::Verified {
        verified_key(&claims)
    } else {
        None
    };

    BranchProjection {
        state,
        verified_branch_key,
        claims,
    }
}

/// The single distinct resolved key among verified claims, if exactly one.
fn verified_key(claims: &[BranchClaim]) -> Option<String> {
    let keys: BTreeSet<&String> = claims
        .iter()
        .filter(|claim| claim.verified)
        .filter_map(|claim| claim.branch_key.as_ref())
        .collect();
    if keys.len() == 1 {
        keys.into_iter().next().cloned()
    } else {
        None
    }
}

/// Derives the state from resolved claims.
fn classify_state(claims: &[BranchClaim]) -> BranchState {
    if claims.is_empty() {
        return BranchState::Missing;
    }

    let resolved_keys: BTreeSet<&String> = claims
        .iter()
        .filter_map(|claim| claim.branch_key.as_ref())
        .collect();
    if resolved_keys.len() > 1 {
        return BranchState::Conflicting;
    }

    if resolved_keys.is_empty() {
        // No claim resolves: conflicting only when the recorded text differs.
        let texts: BTreeSet<String> = claims
            .iter()
            .map(|claim| normalize(&claim.value_text))
            .collect();
        if texts.len() > 1 {
            return BranchState::Conflicting;
        }
        return BranchState::Unverified;
    }

    let verified_claims: Vec<&BranchClaim> = claims.iter().filter(|claim| claim.verified).collect();
    if verified_claims.is_empty() {
        return BranchState::Unverified;
    }

    // Every verified claim must resolve; a verified claim that does not resolve
    // means the record is not a usable single branch.
    if verified_claims
        .iter()
        .any(|claim| claim.branch_key.is_none())
    {
        let verified_texts: BTreeSet<String> = verified_claims
            .iter()
            .map(|claim| normalize(&claim.value_text))
            .collect();
        if verified_texts.len() > 1 {
            return BranchState::Conflicting;
        }
        return BranchState::Unverified;
    }

    let verified_keys: BTreeSet<&String> = verified_claims
        .iter()
        .filter_map(|claim| claim.branch_key.as_ref())
        .collect();
    match verified_keys.len() {
        1 => BranchState::Verified,
        0 => BranchState::Unverified,
        _ => BranchState::Conflicting,
    }
}

/// Loads the current `family_branch` catalog ordered by key.
fn load_catalog(client: &mut impl GenericClient) -> Result<Vec<FamilyBranch>, GenealogyError> {
    let rows = client
        .query(
            "SELECT branch_key, branch_label FROM family_branch ORDER BY branch_key",
            &[],
        )
        .map_err(GenealogyError::Query)?;
    Ok(rows
        .iter()
        .map(|row| FamilyBranch {
            branch_key: row.get(0),
            branch_label: row.get(1),
        })
        .collect())
}

/// Loads raw current branch claims for one person id.
fn load_raw_for_person_id(
    client: &mut impl GenericClient,
    person_id: i64,
) -> Result<Vec<(String, String, String)>, GenealogyError> {
    let sql = format!(
        "SELECT a.assertion_kind, a.value_text, a.review_status \
           FROM assertion a \
          WHERE a.person_id = $1 \
            AND a.assertion_kind IN ('branch_claim', 'branch') \
            AND a.value_text IS NOT NULL \
            AND btrim(a.value_text) <> '' \
            AND ({CURRENT_ASSERTION}) \
          ORDER BY a.assertion_id"
    );
    let rows = client
        .query(&sql, &[&person_id])
        .map_err(GenealogyError::Query)?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

/// Projects one person by `person_key`.
pub fn project(
    client: &mut impl GenericClient,
    person_key: &str,
) -> Result<BranchProjection, GenealogyError> {
    let person_id = person_id_for_key(client, person_key)?;
    match person_id {
        Some(id) => project_for_person_id(client, id),
        None => Ok(BranchProjection {
            state: BranchState::Missing,
            verified_branch_key: None,
            claims: Vec::new(),
        }),
    }
}

/// Projects one person by `person_id`.
pub fn project_for_person_id(
    client: &mut impl GenericClient,
    person_id: i64,
) -> Result<BranchProjection, GenealogyError> {
    let raw = load_raw_for_person_id(client, person_id)?;
    let catalog = load_catalog(client)?;
    Ok(classify(&raw, &catalog))
}

/// Projects many people by `person_key`, keyed by the requested key.
///
/// A requested key that does not resolve to a person projects as missing.
pub fn project_many(
    client: &mut impl GenericClient,
    person_keys: &[String],
) -> Result<Vec<(String, BranchProjection)>, GenealogyError> {
    let catalog = load_catalog(client)?;
    let mut out = Vec::with_capacity(person_keys.len());
    for person_key in person_keys {
        let projection = match person_id_for_key(client, person_key)? {
            Some(id) => {
                let raw = load_raw_for_person_id(client, id)?;
                classify(&raw, &catalog)
            }
            None => BranchProjection {
                state: BranchState::Missing,
                verified_branch_key: None,
                claims: Vec::new(),
            },
        };
        out.push((person_key.clone(), projection));
    }
    Ok(out)
}

/// Returns the distinct verified branch labels for one person, ordered ascending.
///
/// This is the reusable helper for the verified-claim reads. A verified label is
/// the resolved `family_branch` label; a verified claim that does not resolve to a
/// branch still reports its recorded text so behavior matches the prior search.
pub fn verified_branch_labels(
    client: &mut impl GenericClient,
    person_key: &str,
) -> Result<Vec<String>, GenealogyError> {
    let Some(person_id) = person_id_for_key(client, person_key)? else {
        return Ok(Vec::new());
    };
    let catalog = load_catalog(client)?;
    let raw = load_raw_for_person_id(client, person_id)?;
    let mut labels: Vec<String> = raw
        .iter()
        .filter(|(_, _, review_status)| review_status.eq_ignore_ascii_case("verified"))
        .map(|(_, value_text, _)| match resolve(value_text, &catalog) {
            Some(branch) => branch.branch_label.clone(),
            None => value_text.clone(),
        })
        .collect();
    labels.sort();
    labels.dedup();
    Ok(labels)
}

/// Looks up a person id from a person key.
fn person_id_for_key(
    client: &mut impl GenericClient,
    person_key: &str,
) -> Result<Option<i64>, GenealogyError> {
    let row = client
        .query_opt(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&person_key],
        )
        .map_err(GenealogyError::Query)?;
    Ok(row.map(|row| row.get(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<FamilyBranch> {
        vec![
            FamilyBranch {
                branch_key: "branch-1".to_string(),
                branch_label: "First Branch".to_string(),
            },
            FamilyBranch {
                branch_key: "branch-2".to_string(),
                branch_label: "Second Branch".to_string(),
            },
        ]
    }

    fn claim(kind: &str, text: &str, status: &str) -> (String, String, String) {
        (kind.to_string(), text.to_string(), status.to_string())
    }

    #[test]
    fn no_claim_is_missing() {
        let projection = classify(&[], &catalog());
        assert_eq!(projection.state, BranchState::Missing);
        assert!(projection.verified_branch_key.is_none());
        assert!(projection.claims.is_empty());
    }

    #[test]
    fn one_unverified_claim_is_unverified() {
        let raw = [claim("branch_claim", "branch-1", "unverified")];
        let projection = classify(&raw, &catalog());
        assert_eq!(projection.state, BranchState::Unverified);
        assert!(projection.verified_branch_key.is_none());
        assert_eq!(projection.claims.len(), 1);
        assert_eq!(projection.claims[0].branch_key.as_deref(), Some("branch-1"));
        assert!(!projection.claims[0].verified);
    }

    #[test]
    fn one_verified_claim_is_verified_with_its_key() {
        let raw = [claim("branch_claim", "branch-1", "verified")];
        let projection = classify(&raw, &catalog());
        assert_eq!(projection.state, BranchState::Verified);
        assert_eq!(projection.verified_branch_key.as_deref(), Some("branch-1"));
    }

    #[test]
    fn two_verified_claims_same_branch_by_key_and_label_are_verified() {
        let raw = [
            claim("branch_claim", "branch-1", "verified"),
            claim("branch_claim", "First Branch", "verified"),
        ];
        let projection = classify(&raw, &catalog());
        assert_eq!(projection.state, BranchState::Verified);
        assert_eq!(projection.verified_branch_key.as_deref(), Some("branch-1"));
    }

    #[test]
    fn two_verified_claims_different_branches_are_conflicting() {
        let raw = [
            claim("branch_claim", "branch-1", "verified"),
            claim("branch_claim", "branch-2", "verified"),
        ];
        let projection = classify(&raw, &catalog());
        assert_eq!(projection.state, BranchState::Conflicting);
        assert!(projection.verified_branch_key.is_none());
    }

    #[test]
    fn an_unverified_claim_beside_a_verified_claim_lists_both() {
        let raw = [
            claim("branch_claim", "branch-1", "verified"),
            claim("branch", "branch-2", "unverified"),
        ];
        let projection = classify(&raw, &catalog());
        // Two distinct resolved keys make the record conflicting.
        assert_eq!(projection.state, BranchState::Conflicting);
        assert_eq!(projection.claims.len(), 2);
        assert!(projection.verified_branch_key.is_none());
    }

    #[test]
    fn an_unverified_claim_of_the_same_branch_beside_a_verified_claim_is_verified() {
        let raw = [
            claim("branch_claim", "branch-1", "verified"),
            claim("branch", "branch-1", "unverified"),
        ];
        let projection = classify(&raw, &catalog());
        assert_eq!(projection.state, BranchState::Verified);
        assert_eq!(projection.verified_branch_key.as_deref(), Some("branch-1"));
        assert_eq!(projection.claims.len(), 2);
    }

    #[test]
    fn an_unrecognized_descriptive_value_never_resolves() {
        let raw = [claim(
            "branch_claim",
            "the northern family line",
            "verified",
        )];
        let projection = classify(&raw, &catalog());
        assert_eq!(projection.state, BranchState::Unverified);
        assert!(projection.verified_branch_key.is_none());
        assert_eq!(projection.claims.len(), 1);
        assert!(projection.claims[0].branch_key.is_none());
        assert!(projection.claims[0].branch_label.is_none());
    }

    #[test]
    fn a_legacy_branch_claim_stays_visible_and_unverified_without_a_branch_label() {
        let raw = [claim("branch", "some imported description", "unverified")];
        let projection = classify(&raw, &catalog());
        assert_eq!(projection.state, BranchState::Unverified);
        assert!(projection.verified_branch_key.is_none());
        assert_eq!(projection.claims.len(), 1);
        assert_eq!(projection.claims[0].assertion_kind, "branch");
        assert!(!projection.claims[0].verified);
        assert!(projection.claims[0].branch_key.is_none());
        assert!(projection.claims[0].branch_label.is_none());
    }

    #[test]
    fn a_legacy_branch_claim_resolves_only_on_an_exact_key_or_label_match() {
        // A trimmed, case-folded exact label match resolves.
        let raw = [claim("branch", "  first branch  ", "unverified")];
        let projection = classify(&raw, &catalog());
        assert_eq!(projection.state, BranchState::Unverified);
        assert_eq!(projection.claims[0].branch_key.as_deref(), Some("branch-1"));
        assert_eq!(
            projection.claims[0].branch_label.as_deref(),
            Some("First Branch")
        );

        // A partial match does not resolve.
        let raw = [claim("branch", "First", "unverified")];
        let projection = classify(&raw, &catalog());
        assert!(projection.claims[0].branch_key.is_none());
        assert!(projection.claims[0].branch_label.is_none());
    }
}
