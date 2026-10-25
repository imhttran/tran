//! Validation rules for candidate assertions (TRAN-002D).
//!
//! [`validate_candidate`] runs the blocking checks for one assertion:
//!  (a) self-parenting over `parent_child_link` and `parent_child` assertions;
//!  (b) transitive parent-child cycles;
//!  (c) duplicate spouse relations;
//!  (d) incompatible branch claims (`branch_claim` assertions);
//!  (e) contradictory vital-status assertions (`vital_status` assertions).
//!
//! The result is ordered, deduplicated, and each [`Violation`] reports whether
//! it blocks promotion to `verified`.

use postgres::Transaction;
use std::fmt;

use super::dates;
use super::GenealogyError;

/// A blocking or advisory validation finding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Violation {
    /// A person is their own parent.
    SelfParenting { person_id: i64 },
    /// A parent-child cycle was found.
    Cycle { person_id: i64 },
    /// A duplicate spouse relation exists.
    DuplicateSpouse { person_id: i64, spouse_id: i64 },
    /// Two distinct verified branch keys exist for one person.
    IncompatibleBranch {
        person_id: i64,
        first: String,
        second: String,
    },
    /// Two vital_status assertions disagree for one person.
    ContradictoryVital {
        person_id: i64,
        vital: String,
        first: String,
        second: String,
    },
    /// Two verified birth or death dates disagree for one person.
    ContradictoryDate {
        person_id: i64,
        kind: String,
        first: String,
        second: String,
    },
}

impl Violation {
    /// Whether this violation blocks promotion to `verified`.
    pub fn is_blocking(&self) -> bool {
        true
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::SelfParenting { person_id } => {
                write!(f, "self_parenting(person={person_id})")
            }
            Violation::Cycle { person_id } => write!(f, "parent_child_cycle(person={person_id})"),
            Violation::DuplicateSpouse { person_id, spouse_id } => {
                write!(f, "duplicate_spouse(person={person_id},spouse={spouse_id})")
            }
            Violation::IncompatibleBranch {
                person_id,
                first,
                second,
            } => write!(
                f,
                "incompatible_branch(person={person_id},first={first},second={second})"
            ),
            Violation::ContradictoryVital {
                person_id,
                vital,
                first,
                second,
            } => write!(
                f,
                "contradictory_vital(person={person_id},vital={vital},first={first},second={second})"
            ),
            Violation::ContradictoryDate {
                person_id,
                kind,
                first,
                second,
            } => write!(
                f,
                "contradictory_date(person={person_id},kind={kind},first={first},second={second})"
            ),
        }
    }
}

/// Runs every validation rule for one candidate assertion and returns the
/// ordered, deduplicated violations.
pub fn validate_candidate(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
) -> Result<Vec<Violation>, GenealogyError> {
    let row = tx
        .query_opt(
            "SELECT person_id, assertion_kind, assertion_key, value_text
               FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .map_err(GenealogyError::Query)?;
    let row = match row {
        Some(row) => row,
        None => return Ok(Vec::new()),
    };
    let person_id: i64 = row.get(0);
    let kind: String = row.get(1);
    let _key: String = row.get(2);
    let value: Option<String> = row.get(3);

    let mut found: Vec<Violation> = Vec::new();

    if kind == "parent_child" {
        check_self_parenting_link(tx, person_id, &mut found)?;
        check_cycle(tx, person_id, &mut found)?;
        // A self-referential parent_child assertion.
        if let Some(name) = &value {
            let is_self: Option<bool> = tx
                .query_opt(
                    "SELECT p.person_key = $2 FROM person p WHERE p.person_id = $1",
                    &[&person_id, name],
                )
                .map_err(GenealogyError::Query)?
                .map(|r| r.get(0));
            if is_self == Some(true) {
                found.push(Violation::SelfParenting { person_id });
            }
        }
    } else if kind == "spouse" {
        check_duplicate_spouse(tx, person_id, &mut found)?;
    } else if kind == "branch_claim" {
        check_incompatible_branch(tx, person_id, value.as_deref(), &mut found)?;
    } else if kind == "vital_status" {
        check_contradictory_vital(tx, person_id, &mut found)?;
    } else if kind == "birth_date" || kind == "death_date" {
        check_contradictory_date(tx, assertion_id, person_id, &kind, &mut found)?;
    }

    found.sort();
    found.dedup();
    Ok(found)
}

fn check_self_parenting_link(
    tx: &mut Transaction<'_>,
    person_id: i64,
    found: &mut Vec<Violation>,
) -> Result<(), GenealogyError> {
    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM parent_child_link WHERE parent_id = $1 AND child_id = $1",
            &[&person_id],
        )
        .map_err(GenealogyError::Query)?
        .get(0);
    if count > 0 {
        found.push(Violation::SelfParenting { person_id });
    }
    Ok(())
}

fn check_cycle(
    tx: &mut Transaction<'_>,
    person_id: i64,
    found: &mut Vec<Violation>,
) -> Result<(), GenealogyError> {
    // A cycle exists when the person is reachable from themselves through the
    // parent-child edge direction (parent -> child).
    let row = tx
        .query_one(
            "WITH RECURSIVE reach(node) AS (
                SELECT child_id FROM parent_child_link WHERE parent_id = $1
                UNION
                SELECT l.child_id FROM parent_child_link l JOIN reach r ON l.parent_id = r.node
             )
             SELECT EXISTS (SELECT 1 FROM reach WHERE node = $1)",
            &[&person_id],
        )
        .map_err(GenealogyError::Query)?;
    let cyclic: bool = row.get(0);
    if cyclic {
        found.push(Violation::Cycle { person_id });
    }
    Ok(())
}

fn check_duplicate_spouse(
    tx: &mut Transaction<'_>,
    person_id: i64,
    found: &mut Vec<Violation>,
) -> Result<(), GenealogyError> {
    let rows = tx
        .query(
            "SELECT person_id, spouse_id FROM spouse_link
              WHERE person_id = $1 OR spouse_id = $1",
            &[&person_id],
        )
        .map_err(GenealogyError::Query)?;
    let mut seen: Vec<(i64, i64)> = Vec::new();
    for row in rows {
        let raw: (i64, i64) = (row.get(0), row.get(1));
        // Normalize unordered pairs so a reciprocal link is a duplicate.
        let pair = if raw.0 <= raw.1 { raw } else { (raw.1, raw.0) };
        if seen.contains(&pair) {
            found.push(Violation::DuplicateSpouse {
                person_id: pair.0,
                spouse_id: pair.1,
            });
        } else {
            seen.push(pair);
        }
    }
    Ok(())
}

fn check_incompatible_branch(
    tx: &mut Transaction<'_>,
    person_id: i64,
    value: Option<&str>,
    found: &mut Vec<Violation>,
) -> Result<(), GenealogyError> {
    let rows = tx
        .query(
            "SELECT value_text FROM assertion
              WHERE person_id = $1 AND assertion_kind = 'branch_claim'
                AND review_status IN ('verified', 'needs_review')
                AND value_text IS NOT NULL
                AND superseded_at IS NULL",
            &[&person_id],
        )
        .map_err(GenealogyError::Query)?;
    let mut keys: Vec<String> = rows.iter().map(|r| r.get::<_, String>(0)).collect();
    if let Some(value) = value {
        keys.push(value.to_string());
    }
    keys.sort();
    keys.dedup();
    if let Some(first) = keys.first() {
        if let Some(second) = keys.get(1) {
            found.push(Violation::IncompatibleBranch {
                person_id,
                first: first.clone(),
                second: second.clone(),
            });
        }
    }
    Ok(())
}

fn check_contradictory_vital(
    tx: &mut Transaction<'_>,
    person_id: i64,
    found: &mut Vec<Violation>,
) -> Result<(), GenealogyError> {
    let rows = tx
        .query(
            "SELECT assertion_key, value_text FROM assertion
              WHERE person_id = $1 AND assertion_kind = 'vital_status'
                AND review_status IN ('verified', 'needs_review', 'unverified', 'disputed')
                AND value_text IS NOT NULL
                AND superseded_at IS NULL",
            &[&person_id],
        )
        .map_err(GenealogyError::Query)?;
    let mut grouped: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for row in rows {
        let key: String = row.get(0);
        let value: String = row.get(1);
        grouped.entry(key).or_default().push(value);
    }
    for (vital, mut values) in grouped {
        values.sort();
        values.dedup();
        if values.len() >= 2 {
            found.push(Violation::ContradictoryVital {
                person_id,
                vital: vital.clone(),
                first: values[0].clone(),
                second: values[1].clone(),
            });
        }
    }
    Ok(())
}

/// Refuses to verify a date that cannot be true alongside an already-verified
/// date of the same kind.
///
/// The comparison uses the precision the source supports: `1877` and `1877-03`
/// overlap and may both be true, while `1877` and `1878` cannot. Both claims stay
/// stored either way; the reviewer decides which one the evidence supports, or
/// rejects the other. Nothing is overwritten, so a corrected import never
/// silently replaces a reviewed date.
fn check_contradictory_date(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
    person_id: i64,
    kind: &str,
    found: &mut Vec<Violation>,
) -> Result<(), GenealogyError> {
    let candidate = match tx
        .query_opt(
            "SELECT value_date_start, value_date_end
               FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .map_err(GenealogyError::Query)?
    {
        Some(row) => row,
        None => return Ok(()),
    };
    let Some(candidate_start) = candidate.try_get::<_, Option<String>>(0)? else {
        return Ok(());
    };
    let candidate_end: Option<String> = candidate.try_get(1)?;
    // A claim without a machine-readable value cannot be compared semantically;
    // the stored wording is not a date, so there is nothing to contradict.
    let Some(candidate_bounds) = dates::claim_bounds(&candidate_start, candidate_end.as_deref())
    else {
        return Ok(());
    };

    let rows = tx
        .query(
            "SELECT value_date_start, value_date_end
               FROM assertion
              WHERE person_id = $1
                AND assertion_kind = $2
                AND assertion_id <> $3
                AND review_status = 'verified'
                AND superseded_at IS NULL
                AND value_date_start IS NOT NULL
              ORDER BY value_date_start, assertion_id",
            &[&person_id, &kind, &assertion_id],
        )
        .map_err(GenealogyError::Query)?;
    for row in rows {
        let other_start: String = row.try_get(0)?;
        let other_end: Option<String> = row.try_get(1)?;
        let Some(other_bounds) = dates::claim_bounds(&other_start, other_end.as_deref()) else {
            continue;
        };
        if !dates::intervals_overlap(candidate_bounds, other_bounds) {
            found.push(Violation::ContradictoryDate {
                person_id,
                kind: kind.to_string(),
                first: candidate_start,
                second: other_start,
            });
            break;
        }
    }
    Ok(())
}
