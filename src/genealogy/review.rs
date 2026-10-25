//! Evidence-backed review transitions (TRAN-002D).
//!
//! [`review_assertion`] moves exactly one assertion from its current status to
//! a new status. It always writes one `assertion_review_event` row with the
//! reviewer, the timestamp, the from/to status, and the supporting evidence
//! reference. Mass approval is unsupported by construction: every review takes
//! a single `assertion_id` and an evidence reference.
//!
//! Before any transition to `verified`, the assertion is validated. A blocking
//! violation (cycle, self-parenting, duplicate, incompatible branch, vital
//! contradiction) refuses the transition, keeps the prior status, writes one
//! `needs_review` event that describes the blocking violations, and returns
//! [`ReviewError::Blocked`].

use postgres::{Client, Transaction};
use thiserror::Error;

use super::validation::{self, Violation};

/// The review statuses understood by the workflow. `unverified` is the
/// candidate state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewStatus {
    /// A candidate that no reviewer has accepted (the database default).
    Unverified,
    /// Previously verified, reopened for review.
    NeedsReview,
    /// Accepted by a reviewer with evidence.
    Verified,
    /// A reviewer disputes the claim.
    Disputed,
    /// A reviewer rejected the claim.
    Rejected,
}

impl ReviewStatus {
    /// The database text value.
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewStatus::Unverified => "unverified",
            ReviewStatus::NeedsReview => "needs_review",
            ReviewStatus::Verified => "verified",
            ReviewStatus::Disputed => "disputed",
            ReviewStatus::Rejected => "rejected",
        }
    }

    /// Parses a database text value.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "unverified" => ReviewStatus::Unverified,
            "needs_review" => ReviewStatus::NeedsReview,
            "verified" => ReviewStatus::Verified,
            "disputed" => ReviewStatus::Disputed,
            "rejected" => ReviewStatus::Rejected,
            _ => return None,
        })
    }
}

/// A refusal to perform a review.
#[derive(Debug, Error, PartialEq)]
pub enum ReviewError {
    /// The assertion ID is missing or unknown.
    #[error("unknown or missing assertion: {0}")]
    UnknownAssertion(String),
    /// The reviewer is empty.
    #[error("reviewer is required")]
    MissingReviewer,
    /// The evidence reference is empty.
    #[error("supporting evidence reference is required")]
    MissingEvidence,
    /// The requested status is not a known status.
    #[error("unknown review status: {0}")]
    UnknownStatus(String),
    /// The requested transition is not allowed from the current status.
    #[error("invalid review transition: {0} -> {1}")]
    InvalidTransition(String, String),
    /// A verified transition has no evidence rows.
    #[error("cannot verify assertion {0}: it has no supporting evidence")]
    NoEvidence(i64),
    /// The evidence is not linked to an authoritative original source with a
    /// valid locator.
    #[error("cannot verify assertion {0}: {1}")]
    Unprovenanced(i64, String),
    /// The assertion is blocked by a blocking validation violation.
    #[error("cannot verify assertion {0}: {1:?}")]
    Blocked(i64, Vec<Violation>),
    /// The database operation failed.
    #[error("review database error: {0}")]
    Db(String),
}

/// A request to review exactly one assertion.
#[derive(Debug, Clone)]
pub struct ReviewRequest<'a> {
    /// The single chosen assertion. Required: there is no bulk form.
    pub assertion_id: i64,
    /// The target status.
    pub new_status: ReviewStatus,
    /// The reviewer identity. Required and non-empty.
    pub reviewer: &'a str,
    /// The supporting evidence reference. Required and non-empty.
    pub evidence_ref: &'a str,
    /// An optional note.
    pub note: Option<&'a str>,
}

/// The result of an accepted review transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewOutcome {
    /// The assertion that was reviewed.
    pub assertion_id: i64,
    /// The prior status text.
    pub from_status: String,
    /// The new status text.
    pub to_status: String,
    /// The evidence fingerprint recorded on a verified transition.
    pub baseline: Option<String>,
}

/// Computes the deterministic evidence fingerprint of an assertion.
///
/// The fingerprint is the sorted natural key set plus the `sha256` of each
/// referenced source document. A source document `sha256` change therefore
/// changes the fingerprint, and re-import can detect the change.
pub fn evidence_fingerprint(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
) -> Result<String, ReviewError> {
    let rows = tx
        .query(
            "SELECT ae.document_ref, ae.page, COALESCE(sd.sha256, '')
               FROM assertion_evidence ae
               LEFT JOIN source_evidence se ON se.source_evidence_id = ae.source_evidence_id
               LEFT JOIN source_document sd ON sd.source_document_id = se.source_document_id
              WHERE ae.assertion_id = $1",
            &[&assertion_id],
        )
        .map_err(|err| ReviewError::Db(err.to_string()))?;
    let mut keys: Vec<String> = rows
        .iter()
        .map(|row| {
            let document_ref: String = row.get(0);
            let page: i32 = row.get(1);
            let sha: String = row.get(2);
            format!("{document_ref}|{page}|{sha}")
        })
        .collect();
    keys.sort();
    Ok(fnv1a64_hex(keys.join("\n").as_bytes()))
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Reports whether one review transition is allowed.
///
/// A candidate or reopened assertion may be verified, disputed, or rejected.
/// A verified or disputed assertion may be reopened or rejected. A rejected
/// assertion must be reopened before it can be reviewed again; it cannot jump
/// directly back to verified or disputed.
pub fn allowed_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("unverified", "verified" | "disputed" | "rejected")
            | ("needs_review", "verified" | "disputed" | "rejected")
            | ("verified", "needs_review" | "disputed" | "rejected")
            | ("disputed", "needs_review" | "verified" | "rejected")
            | ("rejected", "needs_review")
    )
}

/// Performs one evidence-backed review transition on exactly one assertion.
///
/// The assertion status row, the review event, and (on verify) the evidence
/// baseline are written in one transaction. The prior event rows are never
/// updated or deleted.
pub fn review_assertion(
    conn: &mut Client,
    request: &ReviewRequest<'_>,
) -> Result<ReviewOutcome, ReviewError> {
    if request.reviewer.trim().is_empty() {
        return Err(ReviewError::MissingReviewer);
    }
    if request.evidence_ref.trim().is_empty() {
        return Err(ReviewError::MissingEvidence);
    }
    if request.assertion_id <= 0 {
        return Err(ReviewError::UnknownAssertion(
            request.assertion_id.to_string(),
        ));
    }

    let mut tx = conn
        .transaction()
        .map_err(|err| ReviewError::Db(err.to_string()))?;
    let outcome = review_in_transaction(&mut tx, request);
    match outcome {
        Ok(result) => {
            tx.commit()
                .map_err(|err| ReviewError::Db(err.to_string()))?;
            Ok(result)
        }
        Err(ReviewError::Blocked(id, violations)) => {
            // Record the blocking violations as a needs_review event, then
            // commit so the audit trail survives. The status stays unchanged.
            let from_status = current_status(&mut tx, id)?;
            insert_event(
                &mut tx,
                id,
                request.reviewer,
                &from_status,
                ReviewStatus::NeedsReview.as_str(),
                request.evidence_ref,
                Some(&format_blocked(&violations)),
            )?;
            tx.commit()
                .map_err(|err| ReviewError::Db(err.to_string()))?;
            Err(ReviewError::Blocked(id, violations))
        }
        Err(other) => Err(other),
    }
}

fn format_blocked(violations: &[Violation]) -> String {
    let parts: Vec<String> = violations.iter().map(|v| v.to_string()).collect();
    format!("blocked: {}", parts.join("; "))
}

/// Returns why the assertion lacks authoritative, locator-backed evidence.
///
/// A verified claim needs at least one evidence row joined through
/// `source_evidence` to an `original` source document whose derivation lineage
/// is empty and whose page/locator is explicit. A null link, a derivative
/// catalog/transcription, or a missing locator never satisfies the rule.
///
/// Withdrawn evidence is ignored: removing the source document removes its
/// support, so a claim cannot be verified on a document the archive no longer
/// publishes.
fn provenance_gap(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
) -> Result<Option<String>, ReviewError> {
    let rows = tx
        .query(
            "SELECT ae.source_evidence_id, se.page, se.locator,
                    sd.source_kind, sd.derived_from_source_document_id
               FROM assertion_evidence ae
               LEFT JOIN source_evidence se ON se.source_evidence_id = ae.source_evidence_id
               LEFT JOIN source_document sd ON sd.source_document_id = se.source_document_id
              WHERE ae.assertion_id = $1 AND ae.withdrawn_at IS NULL",
            &[&assertion_id],
        )
        .map_err(|err| ReviewError::Db(err.to_string()))?;

    let mut null_link = false;
    let mut derivative = false;
    let mut bad_locator = false;
    for row in rows {
        let source_evidence_id: Option<i64> = row.get(0);
        let page: Option<i32> = row.get(1);
        let locator: Option<String> = row.get(2);
        let source_kind: Option<String> = row.get(3);
        let derived_from: Option<i64> = row.get(4);

        if source_evidence_id.is_none() || source_kind.is_none() {
            null_link = true;
            continue;
        }
        if source_kind.as_deref() != Some("original") || derived_from.is_some() {
            derivative = true;
            continue;
        }
        let locator_ok = locator
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let page_ok = page.is_some_and(|value| value >= 1);
        if !locator_ok || !page_ok {
            bad_locator = true;
            continue;
        }
        return Ok(None);
    }

    let reason = if null_link {
        "evidence is not linked to source_evidence".to_string()
    } else if derivative {
        "evidence is linked only to a derivative source document".to_string()
    } else if bad_locator {
        "evidence lacks a valid page and locator".to_string()
    } else {
        "no authoritative evidence".to_string()
    };
    Ok(Some(reason))
}

fn current_status(tx: &mut Transaction<'_>, assertion_id: i64) -> Result<String, ReviewError> {
    let row = tx
        .query_opt(
            "SELECT review_status FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .map_err(|err| ReviewError::Db(err.to_string()))?
        .ok_or_else(|| ReviewError::UnknownAssertion(assertion_id.to_string()))?;
    Ok(row.get(0))
}

pub(crate) fn review_in_transaction(
    tx: &mut Transaction<'_>,
    request: &ReviewRequest<'_>,
) -> Result<ReviewOutcome, ReviewError> {
    let from_status = current_status(tx, request.assertion_id)?;
    if !allowed_transition(&from_status, request.new_status.as_str()) {
        return Err(ReviewError::InvalidTransition(
            from_status,
            request.new_status.as_str().to_string(),
        ));
    }

    let evidence_count: i64 = tx
        .query_one(
            "SELECT count(*) FROM assertion_evidence
              WHERE assertion_id = $1 AND withdrawn_at IS NULL",
            &[&request.assertion_id],
        )
        .map_err(|err| ReviewError::Db(err.to_string()))?
        .get(0);

    if request.new_status == ReviewStatus::Verified {
        if evidence_count == 0 {
            return Err(ReviewError::NoEvidence(request.assertion_id));
        }
        if let Some(reason) = provenance_gap(tx, request.assertion_id)? {
            return Err(ReviewError::Unprovenanced(request.assertion_id, reason));
        }
        let violations = validation::validate_candidate(tx, request.assertion_id)
            .map_err(|err| ReviewError::Db(err.to_string()))?;
        let blocking: Vec<Violation> = violations
            .into_iter()
            .filter(Violation::is_blocking)
            .collect();
        if !blocking.is_empty() {
            return Err(ReviewError::Blocked(request.assertion_id, blocking));
        }
    }

    tx.execute(
        "UPDATE assertion
            SET review_status = $1, reviewed_by = $2, reviewed_at = now(), review_note = $3
          WHERE assertion_id = $4",
        &[
            &request.new_status.as_str(),
            &request.reviewer,
            &request.note,
            &request.assertion_id,
        ],
    )
    .map_err(|err| ReviewError::Db(err.to_string()))?;

    insert_event(
        tx,
        request.assertion_id,
        request.reviewer,
        &from_status,
        request.new_status.as_str(),
        request.evidence_ref,
        request.note,
    )?;

    let mut baseline = None;
    if request.new_status == ReviewStatus::Verified {
        let fingerprint = evidence_fingerprint(tx, request.assertion_id)?;
        tx.execute(
            "INSERT INTO assertion_review_baseline (assertion_id, evidence_fingerprint)
             VALUES ($1, $2)
             ON CONFLICT (assertion_id) DO UPDATE
                SET evidence_fingerprint = EXCLUDED.evidence_fingerprint, recorded_at = now()",
            &[&request.assertion_id, &fingerprint],
        )
        .map_err(|err| ReviewError::Db(err.to_string()))?;
        baseline = Some(fingerprint);
    }

    Ok(ReviewOutcome {
        assertion_id: request.assertion_id,
        from_status,
        to_status: request.new_status.as_str().to_string(),
        baseline,
    })
}

fn insert_event(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
    reviewer: &str,
    from_status: &str,
    to_status: &str,
    evidence_ref: &str,
    note: Option<&str>,
) -> Result<(), ReviewError> {
    tx.execute(
        "INSERT INTO assertion_review_event
            (assertion_id, reviewer, from_status, to_status, evidence_ref, note)
         VALUES ($1, $2, $3, $4, $5, $6)",
        &[
            &assertion_id,
            &reviewer,
            &from_status,
            &to_status,
            &evidence_ref,
            &note,
        ],
    )
    .map_err(|err| ReviewError::Db(err.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_can_be_verified_disputed_or_rejected() {
        for to in ["verified", "disputed", "rejected"] {
            assert!(allowed_transition("unverified", to));
            assert!(allowed_transition("needs_review", to));
        }
    }

    #[test]
    fn terminal_and_noop_transitions_are_rejected() {
        assert!(!allowed_transition("verified", "verified"));
        assert!(!allowed_transition("unverified", "unverified"));
        assert!(!allowed_transition("rejected", "verified"));
        assert!(!allowed_transition("rejected", "disputed"));
        assert!(allowed_transition("rejected", "needs_review"));
        assert!(allowed_transition("verified", "disputed"));
        assert!(allowed_transition("disputed", "verified"));
    }
}
