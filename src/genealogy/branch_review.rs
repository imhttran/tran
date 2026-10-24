//! USR-005B: branch-scoped shared-genealogy proposals and review.
//!
//! Branch reviewers act only on a family branch for which they hold an active,
//! link-bound grant. A proposal stores a proposed fact, its prior approved
//! snapshot, and source evidence. Approval applies the value through the existing
//! assertion evidence/provenance validation and review-event workflow. Draft
//! profile content, contact fields, and private media are outside this module.
//!
//! A usable grant is an administrator appointment, not a genealogy status: it
//! must be active, bound to this exact verified identity, still backed by the
//! reviewed account-person link, and recorded by an append-only `granted` audit
//! event. A grant row created out of band therefore confers no authority.

use postgres::{Client, Transaction};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::genealogy::review::{review_in_transaction, ReviewError, ReviewRequest, ReviewStatus};
use crate::onboarding::IdentityAccount;

pub const PROPOSAL_LIMIT_MAX: i64 = 100;
pub const PROPOSAL_VALUE_MAX_CHARS: usize = 20_000;
pub const PROPOSAL_NOTE_MAX_CHARS: usize = 2_000;
pub const PROPOSAL_EVIDENCE_MAX: usize = 50;

/// One source evidence reference attached to a proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalEvidence {
    pub source_evidence_id: i64,
    pub document_ref: String,
    pub page: i32,
}

/// One branch a reviewer may review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveBranchGrant {
    pub branch_key: String,
    pub branch_label: Option<String>,
}

/// One source evidence reference available for a shared-fact proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonEvidenceRef {
    pub source_evidence_id: i64,
    pub document_ref: String,
    pub page: i32,
    pub locator: Option<String>,
    pub assertion_kind: String,
    pub assertion_key: String,
}

/// One queue row. It contains shared genealogy facts and evidence only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalSummary {
    pub proposal_id: i64,
    pub person_key: String,
    pub display_name: String,
    pub branch_key: String,
    pub assertion_kind: String,
    pub assertion_key: String,
    pub prior_value_text: Option<String>,
    pub prior_review_status: Option<String>,
    pub proposed_value_text: String,
    pub evidence: Vec<ProposalEvidence>,
    pub submitted_by: String,
    pub submitted_at: String,
    pub status: String,
    pub can_decide: bool,
    pub decision_block: Option<String>,
}

/// One append-only proposal event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalEvent {
    pub event: String,
    pub actor: String,
    pub note: Option<String>,
    pub created_at: String,
}

/// The current authoritative assertion for the proposal's key, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentAssertion {
    pub assertion_id: i64,
    pub value_text: Option<String>,
    pub review_status: String,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<String>,
}

/// One proposal with its current fact and event history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalDetail {
    pub summary: ProposalSummary,
    pub current_assertion: Option<CurrentAssertion>,
    pub events: Vec<ProposalEvent>,
}

/// A bounded queue page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalPage {
    pub submissions: Vec<ProposalSummary>,
    pub next_cursor: Option<i64>,
    pub status: String,
    pub limit: i64,
}

/// The outcome of one proposal decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalDecision {
    pub proposal_id: i64,
    pub status: String,
    pub applied_assertion_id: Option<i64>,
}

/// Errors raised by branch-scoped proposal review.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BranchReviewError {
    #[error("the account holds no active grant for this branch")]
    NotGranted,
    #[error("unknown or off-scope proposal")]
    UnknownProposal,
    #[error("a branch is required")]
    MissingBranch,
    #[error("the person has no single verified branch")]
    BranchAmbiguous,
    #[error("the person is not verified in the requested branch")]
    NotInBranch,
    #[error("the proposed fact is invalid")]
    InvalidField,
    #[error("at least one valid source evidence reference is required")]
    InvalidEvidence,
    #[error("a proposal is already pending for this fact")]
    OpenProposal,
    #[error("the proposal is not pending review")]
    InvalidTransition,
    #[error("a submitter cannot review their own proposal")]
    SelfReview,
    #[error("the reviewer is linked to this person")]
    OwnerReview,
    #[error("the proposal is stale against the current verified fact")]
    Stale,
    #[error("assertion review rejected the proposal: {0}")]
    Review(String),
    #[error("a non-empty reason is required")]
    ReviewNoteRequired,
    #[error("branch review database operation failed: {0}")]
    Database(String),
}

impl From<postgres::Error> for BranchReviewError {
    fn from(err: postgres::Error) -> Self {
        BranchReviewError::Database(err.to_string())
    }
}

fn review_error(err: ReviewError) -> BranchReviewError {
    match err {
        ReviewError::UnknownAssertion(_)
        | ReviewError::MissingReviewer
        | ReviewError::MissingEvidence
        | ReviewError::UnknownStatus(_)
        | ReviewError::InvalidTransition(_, _)
        | ReviewError::NoEvidence(_)
        | ReviewError::Unprovenanced(_, _)
        | ReviewError::Blocked(_, _) => BranchReviewError::Review(err.to_string()),
        ReviewError::Db(_) => BranchReviewError::Database(err.to_string()),
    }
}

/// Lists source evidence references already attached to a person's assertions.
/// The caller must be the linked owner or hold a grant for the person's single
/// verified branch.
pub fn person_evidence(
    client: &mut Client,
    account: &IdentityAccount,
    person_key: &str,
) -> Result<Vec<PersonEvidenceRef>, BranchReviewError> {
    let mut tx = client.transaction()?;
    let person_id: i64 = tx
        .query_opt(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&person_key.trim()],
        )?
        .ok_or(BranchReviewError::UnknownProposal)?
        .get(0);
    let branch = single_verified_branch(&mut tx, person_id)?;
    let is_owner = linked_person(&mut tx, &account.identity_fingerprint)? == Some(person_id);
    if !is_owner && !has_branch_grant(&mut tx, account, &branch)? {
        return Err(BranchReviewError::NotGranted);
    }
    let rows = tx.query(
        "SELECT DISTINCT se.source_evidence_id, ae.document_ref, ae.page, se.locator, \
                a.assertion_kind, a.assertion_key \
           FROM assertion a \
           JOIN assertion_evidence ae
             ON ae.assertion_id = a.assertion_id AND ae.withdrawn_at IS NULL \
           JOIN source_evidence se ON se.source_evidence_id = ae.source_evidence_id \
          WHERE a.person_id = $1 \
          ORDER BY se.source_evidence_id, a.assertion_kind, a.assertion_key",
        &[&person_id],
    )?;
    let refs = rows
        .into_iter()
        .map(|row| PersonEvidenceRef {
            source_evidence_id: row.get(0),
            document_ref: row.get(1),
            page: row.get(2),
            locator: row.get(3),
            assertion_kind: row.get(4),
            assertion_key: row.get(5),
        })
        .collect();
    tx.commit()?;
    Ok(refs)
}

/// Returns the branches this exact account identity may review.
pub fn effective_grants(
    client: &mut Client,
    account: &IdentityAccount,
) -> Result<Vec<EffectiveBranchGrant>, BranchReviewError> {
    let rows = client.query(
        "SELECT b.branch_key, b.branch_label \
           FROM onboarding_branch_reviewer_grant g \
           JOIN family_branch b ON b.family_branch_id = g.family_branch_id \
          WHERE g.account_id = $1 \
            AND g.identity_fingerprint = $2 \
            AND g.revoked_at IS NULL \
            AND EXISTS (SELECT 1 FROM onboarding_person_account_link l \
                         WHERE l.account_id = g.account_id \
                           AND l.identity_fingerprint = g.identity_fingerprint) \
            AND EXISTS (SELECT 1 FROM onboarding_branch_reviewer_grant_event e \
                         WHERE e.account_id = g.account_id \
                           AND e.family_branch_id = g.family_branch_id \
                           AND e.event = 'granted') \
          ORDER BY b.branch_key",
        &[&account.account_id, &account.identity_fingerprint],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| EffectiveBranchGrant {
            branch_key: row.get(0),
            branch_label: row.get(1),
        })
        .collect())
}

/// Creates a pending shared-fact proposal for an authorized branch or the
/// caller's own linked person. The prior verified snapshot is captured here.
#[allow(clippy::too_many_arguments)]
pub fn propose_fact(
    client: &mut Client,
    account: &IdentityAccount,
    person_key: &str,
    branch_key: &str,
    assertion_kind: &str,
    assertion_key: &str,
    proposed_value: &str,
    evidence: &[ProposalEvidence],
    note: Option<&str>,
) -> Result<i64, BranchReviewError> {
    let person_key = person_key.trim();
    let branch_key = branch_key.trim();
    let assertion_kind = assertion_kind.trim();
    let assertion_key = assertion_key.trim();
    let proposed_value = proposed_value.trim();
    if person_key.is_empty()
        || branch_key.is_empty()
        || assertion_kind.is_empty()
        || assertion_key.is_empty()
        || proposed_value.is_empty()
        || proposed_value.chars().count() > PROPOSAL_VALUE_MAX_CHARS
    {
        return Err(BranchReviewError::InvalidField);
    }
    let note = note.map(str::trim).filter(|value| !value.is_empty());
    if note.is_some_and(|value| value.chars().count() > PROPOSAL_NOTE_MAX_CHARS) {
        return Err(BranchReviewError::InvalidField);
    }
    if evidence.is_empty() || evidence.len() > PROPOSAL_EVIDENCE_MAX {
        return Err(BranchReviewError::InvalidEvidence);
    }

    let mut tx = client.transaction()?;
    let person_id: i64 = tx
        .query_opt(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&person_key],
        )?
        .ok_or(BranchReviewError::InvalidField)?
        .get(0);
    let branch_exists: bool = tx
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM family_branch WHERE branch_key = $1)",
            &[&branch_key],
        )?
        .get(0);
    if !branch_exists {
        return Err(BranchReviewError::MissingBranch);
    }
    require_single_verified_branch(&mut tx, person_id, branch_key)?;

    let linked_person = linked_person(&mut tx, account.identity_fingerprint.as_str())?;
    let is_owner = linked_person == Some(person_id);
    if !is_owner && !has_branch_grant(&mut tx, account, branch_key)? {
        return Err(BranchReviewError::NotGranted);
    }
    validate_evidence(&mut tx, evidence)?;

    let prior = tx.query_opt(
        "SELECT value_text, review_status FROM assertion \
          WHERE person_id = $1 AND assertion_kind = $2 AND assertion_key = $3",
        &[&person_id, &assertion_kind, &assertion_key],
    )?;
    let (prior_value, prior_status) = match prior {
        Some(row) => (row.get::<_, Option<String>>(0), row.get::<_, String>(1)),
        None => (None, String::new()),
    };
    let prior_status = if prior_status.is_empty() {
        None
    } else {
        Some(prior_status)
    };
    let evidence_json = serde_json::to_value(evidence)
        .map_err(|err| BranchReviewError::Database(err.to_string()))?;
    let proposal_id: i64 = tx
        .query_one(
            "INSERT INTO genealogy_fact_proposal \
                 (person_id, branch_key, assertion_kind, assertion_key, \
                  prior_value_text, prior_review_status, proposed_value_text, \
                  evidence_json, note, submitted_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::jsonb, $9, $10) \
             RETURNING proposal_id",
            &[
                &person_id,
                &branch_key,
                &assertion_kind,
                &assertion_key,
                &prior_value,
                &prior_status,
                &proposed_value,
                &evidence_json.to_string(),
                &note,
                &account.identity_fingerprint,
            ],
        )
        .map_err(|err| {
            if err.code() == Some(&postgres::error::SqlState::UNIQUE_VIOLATION) {
                BranchReviewError::OpenProposal
            } else {
                BranchReviewError::Database(err.to_string())
            }
        })?
        .get(0);
    insert_event(
        &mut tx,
        proposal_id,
        person_id,
        branch_key,
        "submitted",
        &account.identity_fingerprint,
        note,
    )?;
    tx.commit()?;
    Ok(proposal_id)
}

/// Lists bounded proposals for the caller's granted branches.
pub fn list_proposals(
    client: &mut Client,
    account: &IdentityAccount,
    branch: Option<&str>,
    status: Option<&str>,
    cursor: Option<i64>,
    limit: i64,
) -> Result<ProposalPage, BranchReviewError> {
    let limit = limit.clamp(1, PROPOSAL_LIMIT_MAX);
    let (statuses, status_label) = parse_statuses(status)?;
    let branch = branch.map(str::trim).filter(|value| !value.is_empty());
    if let Some(branch) = branch {
        if !has_branch_grant(client, account, branch)? {
            return Err(BranchReviewError::NotGranted);
        }
    }
    let rows = client.query(
        "SELECT p.proposal_id, p.person_id, p.branch_key, p.assertion_kind, p.assertion_key, \
                p.prior_value_text, p.prior_review_status, p.proposed_value_text, \
                p.evidence_json::text, p.submitted_by, p.submitted_at::text, p.status, \
                per.person_key, per.display_name, \
                EXISTS (SELECT 1 FROM onboarding_person_account_link l \
                         WHERE l.person_id = p.person_id \
                           AND l.identity_fingerprint = $2) AS reviewer_owns_person \
           FROM genealogy_fact_proposal p \
           JOIN person per ON per.person_id = p.person_id \
          WHERE p.status = ANY($3) \
            AND ($1::text IS NULL OR p.branch_key = $1) \
            AND ($4::bigint IS NULL OR p.proposal_id < $4) \
            AND EXISTS (SELECT 1 \
                          FROM onboarding_branch_reviewer_grant g \
                          JOIN family_branch b ON b.family_branch_id = g.family_branch_id \
                          JOIN onboarding_person_account_link l \
                            ON l.account_id = g.account_id \
                           AND l.identity_fingerprint = g.identity_fingerprint \
                         WHERE g.account_id = $6 \
                           AND g.identity_fingerprint = $2 \
                           AND g.revoked_at IS NULL \
                           AND b.branch_key = p.branch_key \
                           AND EXISTS (SELECT 1 \
                                         FROM onboarding_branch_reviewer_grant_event e \
                                        WHERE e.account_id = g.account_id \
                                          AND e.family_branch_id = g.family_branch_id \
                                          AND e.event = 'granted')) \
          ORDER BY p.proposal_id DESC \
          LIMIT $5",
        &[
            &branch,
            &account.identity_fingerprint,
            &statuses,
            &cursor,
            &(limit + 1),
            &account.account_id,
        ],
    )?;
    let mut submissions = rows
        .into_iter()
        .map(|row| summary_from_row(row, &account.identity_fingerprint))
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if submissions.len() as i64 > limit {
        submissions.truncate(limit as usize);
        submissions.last().map(|item| item.proposal_id)
    } else {
        None
    };
    Ok(ProposalPage {
        submissions,
        next_cursor,
        status: status_label,
        limit,
    })
}

/// Reads one in-scope proposal with its current fact and event history.
pub fn read_proposal(
    client: &mut Client,
    account: &IdentityAccount,
    proposal_id: i64,
) -> Result<ProposalDetail, BranchReviewError> {
    let mut tx = client.transaction()?;
    let row = tx
        .query_opt(
            "SELECT p.proposal_id, p.person_id, p.branch_key, p.assertion_kind, p.assertion_key, \
                    p.prior_value_text, p.prior_review_status, p.proposed_value_text, \
                    p.evidence_json::text, p.submitted_by, p.submitted_at::text, p.status, \
                    per.person_key, per.display_name, \
                    EXISTS (SELECT 1 FROM onboarding_person_account_link l \
                             WHERE l.person_id = p.person_id \
                               AND l.identity_fingerprint = $3) AS reviewer_owns_person \
               FROM genealogy_fact_proposal p \
               JOIN person per ON per.person_id = p.person_id \
              WHERE p.proposal_id = $1 \
                AND EXISTS (SELECT 1 \
                              FROM onboarding_branch_reviewer_grant g \
                              JOIN family_branch b ON b.family_branch_id = g.family_branch_id \
                              JOIN onboarding_person_account_link l \
                                ON l.account_id = g.account_id \
                               AND l.identity_fingerprint = g.identity_fingerprint \
                             WHERE g.account_id = $2 \
                               AND g.identity_fingerprint = $3 \
                               AND g.revoked_at IS NULL \
                               AND b.branch_key = p.branch_key \
                               AND EXISTS (SELECT 1 \
                                             FROM onboarding_branch_reviewer_grant_event e \
                                            WHERE e.account_id = g.account_id \
                                              AND e.family_branch_id = g.family_branch_id \
                                              AND e.event = 'granted'))",
            &[
                &proposal_id,
                &account.account_id,
                &account.identity_fingerprint,
            ],
        )?
        .ok_or(BranchReviewError::UnknownProposal)?;
    let person_id: i64 = row.get(1);
    let summary = summary_from_row(row, &account.identity_fingerprint)?;
    let current = tx.query_opt(
        "SELECT assertion_id, value_text, review_status, reviewed_by, reviewed_at::text \
           FROM assertion WHERE person_id = $1 AND assertion_kind = $2 AND assertion_key = $3",
        &[&person_id, &summary.assertion_kind, &summary.assertion_key],
    )?;
    let current_assertion = current.map(|row| CurrentAssertion {
        assertion_id: row.get(0),
        value_text: row.get(1),
        review_status: row.get(2),
        reviewed_by: row.get(3),
        reviewed_at: row.get(4),
    });
    let events = read_events(&mut tx, proposal_id)?;
    tx.commit()?;
    Ok(ProposalDetail {
        summary,
        current_assertion,
        events,
    })
}

/// Decides one in-scope proposal. Approval applies the proposed fact through the
/// existing assertion review transaction; rejection and needs-changes leave the
/// authoritative fact unchanged.
pub fn decide_proposal(
    client: &mut Client,
    account: &IdentityAccount,
    proposal_id: i64,
    decision: &str,
    note: Option<&str>,
) -> Result<ProposalDecision, BranchReviewError> {
    let decision = decision.trim().to_ascii_lowercase();
    if !matches!(decision.as_str(), "approve" | "reject" | "needs_changes") {
        return Err(BranchReviewError::InvalidField);
    }
    let note = note.map(str::trim).filter(|value| !value.is_empty());
    if decision != "approve" && note.is_none() {
        return Err(BranchReviewError::ReviewNoteRequired);
    }
    if note.is_some_and(|value| value.chars().count() > PROPOSAL_NOTE_MAX_CHARS) {
        return Err(BranchReviewError::InvalidField);
    }

    let mut tx = client.transaction()?;
    let row = tx
        .query_opt(
            "SELECT person_id, branch_key, assertion_kind, assertion_key, prior_value_text, \
                    prior_review_status, proposed_value_text, evidence_json::text, submitted_by, status \
               FROM genealogy_fact_proposal WHERE proposal_id = $1 FOR UPDATE",
            &[&proposal_id],
        )?
        .ok_or(BranchReviewError::UnknownProposal)?;
    let person_id: i64 = row.get(0);
    let branch_key: String = row.get(1);
    let assertion_kind: String = row.get(2);
    let assertion_key: String = row.get(3);
    let prior_value: Option<String> = row.get(4);
    let prior_status: Option<String> = row.get(5);
    let proposed_value: String = row.get(6);
    let evidence_json: String = row.get(7);
    let submitted_by: String = row.get(8);
    let status: String = row.get(9);
    if status != "pending_review" {
        return Err(BranchReviewError::InvalidTransition);
    }
    if !has_branch_grant(&mut tx, account, &branch_key)? {
        return Err(BranchReviewError::NotGranted);
    }
    if submitted_by == account.identity_fingerprint {
        return Err(BranchReviewError::SelfReview);
    }
    if linked_person(&mut tx, &account.identity_fingerprint)? == Some(person_id) {
        return Err(BranchReviewError::OwnerReview);
    }
    let evidence: Vec<ProposalEvidence> = serde_json::from_str(&evidence_json)
        .map_err(|err| BranchReviewError::Database(err.to_string()))?;

    let applied_assertion_id = if decision == "approve" {
        let current = tx.query_opt(
            "SELECT value_text, review_status FROM assertion \
              WHERE person_id = $1 AND assertion_kind = $2 AND assertion_key = $3",
            &[&person_id, &assertion_kind, &assertion_key],
        )?;
        let matches = match (&prior_status, &current) {
            (None, None) => true,
            (Some(prior_status), Some(row)) => {
                row.get::<_, Option<String>>(0) == prior_value
                    && row.get::<_, String>(1) == *prior_status
            }
            _ => false,
        };
        if !matches {
            return Err(BranchReviewError::Stale);
        }
        let assertion_id: i64 = tx
            .query_one(
                "INSERT INTO assertion \
                     (person_id, assertion_kind, assertion_key, value_text, review_status) \
                 VALUES ($1, $2, $3, $4, 'unverified') \
                 ON CONFLICT (person_id, assertion_kind, assertion_key) DO UPDATE \
                    SET value_text = EXCLUDED.value_text, \
                        review_status = 'unverified', \
                        reviewed_by = NULL, reviewed_at = NULL, review_note = NULL \
                 RETURNING assertion_id",
                &[&person_id, &assertion_kind, &assertion_key, &proposed_value],
            )?
            .get(0);
        for item in &evidence {
            tx.execute(
                "INSERT INTO assertion_evidence \
                     (assertion_id, source_evidence_id, document_ref, page) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (assertion_id, document_ref, page) DO NOTHING",
                &[
                    &assertion_id,
                    &item.source_evidence_id,
                    &item.document_ref,
                    &item.page,
                ],
            )?;
        }
        let first = evidence.first().ok_or(BranchReviewError::InvalidEvidence)?;
        let evidence_ref = format!("{}:{}", first.document_ref, first.page);
        review_in_transaction(
            &mut tx,
            &ReviewRequest {
                assertion_id,
                new_status: ReviewStatus::Verified,
                reviewer: &account.identity_fingerprint,
                evidence_ref: &evidence_ref,
                note,
            },
        )
        .map_err(review_error)?;
        Some(assertion_id)
    } else {
        None
    };

    let status = if decision == "approve" {
        "approved"
    } else if decision == "reject" {
        "rejected"
    } else {
        "needs_changes"
    };
    tx.execute(
        "UPDATE genealogy_fact_proposal \
            SET status = $2, reviewed_by = $3, reviewed_at = now(), review_note = $4, \
                applied_assertion_id = $5 \
          WHERE proposal_id = $1 AND status = 'pending_review'",
        &[
            &proposal_id,
            &status,
            &account.identity_fingerprint,
            &note,
            &applied_assertion_id,
        ],
    )?;
    insert_event(
        &mut tx,
        proposal_id,
        person_id,
        &branch_key,
        status,
        &account.identity_fingerprint,
        note,
    )?;
    tx.commit()?;
    Ok(ProposalDecision {
        proposal_id,
        status: status.to_string(),
        applied_assertion_id,
    })
}

fn parse_statuses(value: Option<&str>) -> Result<(Vec<String>, String), BranchReviewError> {
    let raw = value
        .unwrap_or("pending_review")
        .trim()
        .to_ascii_lowercase();
    // The returned label is the canonical filter, never the raw client input.
    let (statuses, label) = match raw.as_str() {
        "" | "pending" | "pending_review" => (vec!["pending_review".to_string()], "pending_review"),
        "approved" => (vec!["approved".to_string()], "approved"),
        "rejected" => (vec!["rejected".to_string()], "rejected"),
        "needs_changes" | "needs-changes" => (vec!["needs_changes".to_string()], "needs_changes"),
        "all" => (
            vec![
                "pending_review".to_string(),
                "approved".to_string(),
                "rejected".to_string(),
                "needs_changes".to_string(),
            ],
            "all",
        ),
        _ => return Err(BranchReviewError::InvalidField),
    };
    Ok((statuses, label.to_string()))
}

fn require_single_verified_branch(
    tx: &mut Transaction<'_>,
    person_id: i64,
    branch_key: &str,
) -> Result<(), BranchReviewError> {
    let actual = single_verified_branch(tx, person_id)?;
    if actual != branch_key {
        return Err(BranchReviewError::NotInBranch);
    }
    Ok(())
}

fn single_verified_branch(
    tx: &mut Transaction<'_>,
    person_id: i64,
) -> Result<String, BranchReviewError> {
    let rows = tx.query(
        "SELECT DISTINCT value_text FROM assertion \
          WHERE person_id = $1 AND assertion_kind = 'branch_claim' \
            AND review_status = 'verified' \
            AND value_text IS NOT NULL AND btrim(value_text) <> ''",
        &[&person_id],
    )?;
    let branches: Vec<String> = rows.into_iter().map(|row| row.get(0)).collect();
    if branches.len() != 1 {
        return Err(BranchReviewError::BranchAmbiguous);
    }
    Ok(branches.into_iter().next().expect("one branch"))
}

fn has_branch_grant<C: postgres::GenericClient>(
    tx: &mut C,
    account: &IdentityAccount,
    branch_key: &str,
) -> Result<bool, BranchReviewError> {
    let exists: bool = tx
        .query_one(
            "SELECT EXISTS (SELECT 1 \
                              FROM onboarding_branch_reviewer_grant g \
                              JOIN family_branch b ON b.family_branch_id = g.family_branch_id \
                              JOIN onboarding_person_account_link l \
                                ON l.account_id = g.account_id \
                               AND l.identity_fingerprint = g.identity_fingerprint \
                             WHERE g.account_id = $1 \
                               AND g.identity_fingerprint = $2 \
                               AND g.revoked_at IS NULL \
                               AND b.branch_key = $3 \
                               AND EXISTS (SELECT 1 \
                                             FROM onboarding_branch_reviewer_grant_event e \
                                            WHERE e.account_id = g.account_id \
                                              AND e.family_branch_id = g.family_branch_id \
                                              AND e.event = 'granted'))",
            &[
                &account.account_id,
                &account.identity_fingerprint,
                &branch_key,
            ],
        )?
        .get(0);
    Ok(exists)
}

fn linked_person<C: postgres::GenericClient>(
    tx: &mut C,
    identity_fingerprint: &str,
) -> Result<Option<i64>, BranchReviewError> {
    Ok(tx
        .query_opt(
            "SELECT person_id FROM onboarding_person_account_link \
              WHERE identity_fingerprint = $1",
            &[&identity_fingerprint],
        )?
        .map(|row| row.get(0)))
}

fn validate_evidence(
    tx: &mut Transaction<'_>,
    evidence: &[ProposalEvidence],
) -> Result<(), BranchReviewError> {
    for item in evidence {
        let document_ref = item.document_ref.trim();
        if item.source_evidence_id <= 0 || document_ref.is_empty() || item.page < 1 {
            return Err(BranchReviewError::InvalidEvidence);
        }
        // The cited document must be the one the evidence row actually belongs
        // to, so an approval cannot record a mismatched citation.
        let relative_path: Option<String> = tx
            .query_opt(
                "SELECT sd.relative_path \
                   FROM source_evidence se \
                   JOIN source_document sd \
                     ON sd.source_document_id = se.source_document_id \
                  WHERE se.source_evidence_id = $1 \
                    AND se.page = $2 \
                    AND se.locator IS NOT NULL \
                    AND btrim(se.locator) <> '' \
                    AND sd.source_kind = 'original' \
                    AND sd.derived_from_source_document_id IS NULL",
                &[&item.source_evidence_id, &item.page],
            )?
            .map(|row| row.get(0));
        if relative_path.as_deref() != Some(document_ref) {
            return Err(BranchReviewError::InvalidEvidence);
        }
    }
    Ok(())
}

fn summary_from_row(
    row: postgres::Row,
    reviewer_fingerprint: &str,
) -> Result<ProposalSummary, BranchReviewError> {
    let evidence_json: String = row.get(8);
    let evidence: Vec<ProposalEvidence> = serde_json::from_str(&evidence_json)
        .map_err(|err| BranchReviewError::Database(err.to_string()))?;
    let status: String = row.get(11);
    let submitted_by: String = row.get(9);
    let reviewer_owns_person: bool = row.get(14);
    let decision_block = if status != "pending_review" {
        Some("not_pending".to_string())
    } else if submitted_by == reviewer_fingerprint {
        Some("self_submitted".to_string())
    } else if reviewer_owns_person {
        Some("reviewer_owns_person".to_string())
    } else {
        None
    };
    Ok(ProposalSummary {
        proposal_id: row.get(0),
        person_key: row.get(12),
        display_name: row.get(13),
        branch_key: row.get(2),
        assertion_kind: row.get(3),
        assertion_key: row.get(4),
        prior_value_text: row.get(5),
        prior_review_status: row.get(6),
        proposed_value_text: row.get(7),
        evidence,
        submitted_by,
        submitted_at: row.get(10),
        status,
        can_decide: decision_block.is_none(),
        decision_block,
    })
}

fn read_events(
    tx: &mut Transaction<'_>,
    proposal_id: i64,
) -> Result<Vec<ProposalEvent>, BranchReviewError> {
    let rows = tx.query(
        "SELECT event, actor, note, created_at::text \
           FROM genealogy_fact_proposal_event \
          WHERE proposal_id = $1 ORDER BY event_id",
        &[&proposal_id],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| ProposalEvent {
            event: row.get(0),
            actor: row.get(1),
            note: row.get(2),
            created_at: row.get(3),
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn insert_event(
    tx: &mut Transaction<'_>,
    proposal_id: i64,
    person_id: i64,
    branch_key: &str,
    event: &str,
    actor: &str,
    note: Option<&str>,
) -> Result<(), BranchReviewError> {
    tx.execute(
        "INSERT INTO genealogy_fact_proposal_event \
             (proposal_id, person_id, branch_key, event, actor, note) \
         VALUES ($1, $2, $3, $4, $5, $6)",
        &[&proposal_id, &person_id, &branch_key, &event, &actor, &note],
    )?;
    Ok(())
}
