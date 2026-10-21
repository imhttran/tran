//! Read-only dry-run selection and evaluation for bulk onboarding (BULK-002).
//!
//! [`preview_dry_run`] consumes the output of
//! [`crate::onboarding::candidate::preview_candidates`] and a selector, then
//! applies server-side eligibility filtering, reason classification, filters,
//! pagination, and missing-recipient blocking.
//!
//! The whole module is pure: it never issues SQL, never sends email, never
//! provisions an OIDC account, never issues a token, and never grants a role.
//! Eligibility is decided here, on the server, from the classification produced
//! by the candidate module. Client-supplied filters can only narrow the result;
//! they can never include an ineligible row.

use crate::onboarding::candidate::{CandidatePreview, CandidateRow, Classification, Reason};
use crate::onboarding::username::NameParts;

/// The selection the admin requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// Explicit opaque person keys. Eligibility is still decided here; a key
    /// that is not eligible is reported, not silently included.
    ExplicitSelection(Vec<String>),
    /// Every row that classifies as `eligible_for_invite` right now.
    AllCurrentlyEligible,
    /// Every row in the preview, including blocked and ineligible people. Used
    /// by the admin review table; it never makes a blocked row selectable.
    All,
}

/// Optional filters. A filter can only narrow the server-side result.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DryRunFilters {
    /// Keep only rows whose classification matches this value.
    pub classification: Option<Classification>,
    /// Keep only rows that carry this reason code.
    pub reason: Option<Reason>,
}

/// Pagination. Applied after server-side classification and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// Maximum number of rows to return.
    pub limit: usize,
    /// Number of rows to skip.
    pub offset: usize,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            limit: 50,
            offset: 0,
        }
    }
}

/// One dry-run row. Carries the opaque key, its classification, and reason
/// codes only. The `blocked_missing_recipient` reason plus `preparable` report
/// the missing-recipient state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunRow {
    /// The opaque person key.
    pub person_key: String,
    /// The classification. Decided server-side.
    pub classification: Classification,
    /// The ordered reason codes.
    pub reasons: Vec<Reason>,
    /// The proposed username, when the verified name parts permit one. A
    /// proposal is not a promise: preparation reserves it transactionally.
    pub proposed_username: Option<String>,
    /// True when the row could be prepared for an invitation; sending may still
    /// be blocked.
    pub preparable: bool,
    /// True when the row cannot be sent because a recipient is missing or
    /// no verified contact email exists.
    pub blocked_missing_recipient: bool,
    /// True only when the row may be sent (eligible, unblocked). Dry-run never
    /// sends; this flag is advisory.
    pub sendable: bool,
}

/// The aggregated dry-run report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunReport {
    /// True when the dry-run succeeded. An empty result is still `true`.
    pub ok: bool,
    /// The selected, filtered, paginated rows.
    pub rows: Vec<DryRunRow>,
    /// Per-classification counts over the selected set before pagination.
    pub eligible_for_invite: usize,
    /// Count of `needs_review`.
    pub needs_review: usize,
    /// Count of `already_linked`.
    pub already_linked: usize,
    /// Count of `already_prepared`.
    pub already_prepared: usize,
    /// Count of `invited`.
    pub invited: usize,
    /// Count of `not_eligible`.
    pub not_eligible: usize,
    /// Rows that are preparable but blocked from sending.
    pub blocked_missing_recipient: usize,
    /// A privacy-safe explanation when the result is empty. `Some` only when
    /// zero verified living candidates exist.
    pub review_needed: Option<String>,
}

/// The explanation for a zero-candidate dry-run. It names no person.
pub const REVIEW_NEEDED_EMPTY: &str = "No verified living candidates are available. Review the private catalog evidence before preparing any invitation.";

/// Builds the dry-run row for one classified candidate.
///
/// A row is preparable only when it is eligible. It is blocked from sending
/// when a verified contact email is missing; the row then carries
/// `Reason::BlockedMissingRecipient`.
fn build_row(row: &CandidateRow) -> DryRunRow {
    let eligible = row.classification == Classification::EligibleForInvite;
    let preparable = eligible;
    let blocked_missing_recipient = row.reasons.contains(&Reason::MissingVerifiedContact);
    let mut reasons = row.reasons.clone();
    if blocked_missing_recipient && !reasons.contains(&Reason::BlockedMissingRecipient) {
        reasons.push(Reason::BlockedMissingRecipient);
    }
    let sendable = eligible && !blocked_missing_recipient;
    DryRunRow {
        person_key: row.person_key.clone(),
        classification: row.classification,
        reasons,
        proposed_username: row.proposed_username.clone(),
        preparable,
        blocked_missing_recipient,
        sendable,
    }
}

/// Selects and evaluates candidates for a dry-run.
///
/// `preview` comes from `preview_candidates`; this function adds no SQL. The
/// selector narrows the set; explicit keys that are unknown to the preview are
/// ignored (they cannot invent a candidate). Eligibility is always taken from
/// the server-side classification.
pub fn preview_dry_run(
    preview: &CandidatePreview,
    selector: &Selector,
    filters: &DryRunFilters,
    page: Page,
) -> DryRunReport {
    // 1. Selection against server-side classification.
    let selected: Vec<&CandidateRow> = match selector {
        Selector::AllCurrentlyEligible => preview
            .rows
            .iter()
            .filter(|row| row.classification == Classification::EligibleForInvite)
            .collect(),
        Selector::All => preview.rows.iter().collect(),
        Selector::ExplicitSelection(keys) => preview
            .rows
            .iter()
            .filter(|row| keys.iter().any(|key| key == &row.person_key))
            .collect(),
    };

    // 2. Server-side filter. Filters narrow; they cannot bypass classification.
    let filtered: Vec<DryRunRow> = selected
        .into_iter()
        .map(build_row)
        .filter(|row| {
            let by_class = filters
                .classification
                .map(|value| value == row.classification)
                .unwrap_or(true);
            let by_reason = filters
                .reason
                .map(|reason| row.reasons.contains(&reason))
                .unwrap_or(true);
            by_class && by_reason
        })
        .collect();

    // 3. Summary counts over the filtered set (before pagination).
    let eligible_for_invite = filtered
        .iter()
        .filter(|row| row.classification == Classification::EligibleForInvite)
        .count();
    let needs_review = filtered
        .iter()
        .filter(|row| row.classification == Classification::NeedsReview)
        .count();
    let already_linked = filtered
        .iter()
        .filter(|row| row.classification == Classification::AlreadyLinked)
        .count();
    let already_prepared = filtered
        .iter()
        .filter(|row| row.classification == Classification::AlreadyPrepared)
        .count();
    let invited = filtered
        .iter()
        .filter(|row| row.classification == Classification::Invited)
        .count();
    let not_eligible = filtered
        .iter()
        .filter(|row| row.classification == Classification::NotEligible)
        .count();
    let blocked_missing_recipient = filtered
        .iter()
        .filter(|row| row.blocked_missing_recipient)
        .count();

    // 4. Pagination after classification and filtering.
    let rows: Vec<DryRunRow> = filtered
        .into_iter()
        .skip(page.offset)
        .take(page.limit)
        .collect();

    // 5. Zero verified living candidates gets a review-needed explanation.
    let review_needed = if preview
        .rows
        .iter()
        .all(|row| row.classification != Classification::EligibleForInvite)
    {
        Some(REVIEW_NEEDED_EMPTY.to_string())
    } else {
        None
    };

    DryRunReport {
        ok: true,
        rows,
        eligible_for_invite,
        needs_review,
        already_linked,
        already_prepared,
        invited,
        not_eligible,
        blocked_missing_recipient,
        review_needed,
    }
}

/// Detects an ambiguous name segmentation from verified name parts.
///
/// A single-token name (only one of family/given present) cannot be segmented
/// into a safe username, so it is ambiguous.
pub fn name_is_ambiguous(parts: &NameParts) -> bool {
    let tokens = [&parts.family, &parts.middle, &parts.given]
        .iter()
        .filter(|part| {
            part.as_deref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
        })
        .count();
    tokens == 1
}

/// Reports whether a generated username collides with the taken set.
pub fn username_collides(base: &str, taken: &[String]) -> bool {
    let base_key = crate::onboarding::username::key(base);
    taken
        .iter()
        .any(|item| crate::onboarding::username::key(item) == base_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, classification: Classification, reasons: Vec<Reason>) -> CandidateRow {
        CandidateRow {
            person_key: key.to_string(),
            classification,
            reasons,
            proposed_username: None,
        }
    }

    fn preview(rows: Vec<CandidateRow>) -> CandidatePreview {
        CandidatePreview { rows }
    }

    #[test]
    fn all_currently_eligible_selects_only_eligible() {
        let p = preview(vec![
            row(
                "a",
                Classification::EligibleForInvite,
                vec![Reason::VerifiedLivingEvidence],
            ),
            row("b", Classification::NeedsReview, vec![Reason::Disputed]),
            row(
                "c",
                Classification::NotEligible,
                vec![Reason::NoLivingEvidence],
            ),
        ]);
        let report = preview_dry_run(
            &p,
            &Selector::AllCurrentlyEligible,
            &DryRunFilters::default(),
            Page::default(),
        );
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].person_key, "a");
    }

    #[test]
    fn explicit_selection_cannot_include_ineligible() {
        let p = preview(vec![
            row(
                "a",
                Classification::EligibleForInvite,
                vec![Reason::VerifiedLivingEvidence],
            ),
            row(
                "b",
                Classification::NotEligible,
                vec![Reason::NoLivingEvidence],
            ),
        ]);
        let selector = Selector::ExplicitSelection(vec!["b".to_string(), "unknown".to_string()]);
        let report = preview_dry_run(&p, &selector, &DryRunFilters::default(), Page::default());
        // b is returned as not_eligible but never sendable; unknown is ignored.
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].person_key, "b");
        assert!(!report.rows[0].sendable);
    }

    #[test]
    fn pagination_applies_after_classification() {
        let p = preview(vec![
            row(
                "a",
                Classification::EligibleForInvite,
                vec![Reason::VerifiedLivingEvidence],
            ),
            row(
                "b",
                Classification::EligibleForInvite,
                vec![Reason::VerifiedLivingEvidence],
            ),
            row(
                "c",
                Classification::EligibleForInvite,
                vec![Reason::VerifiedLivingEvidence],
            ),
        ]);
        let page = Page {
            limit: 1,
            offset: 1,
        };
        let report = preview_dry_run(
            &p,
            &Selector::AllCurrentlyEligible,
            &DryRunFilters::default(),
            page,
        );
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].person_key, "b");
    }

    #[test]
    fn missing_verified_contact_is_blocked_not_sendable() {
        let p = preview(vec![row(
            "a",
            Classification::EligibleForInvite,
            vec![
                Reason::VerifiedLivingEvidence,
                Reason::MissingVerifiedContact,
            ],
        )]);
        let report = preview_dry_run(
            &p,
            &Selector::ExplicitSelection(vec!["a".to_string()]),
            &DryRunFilters::default(),
            Page::default(),
        );
        assert_eq!(report.rows.len(), 1);
        let r = &report.rows[0];
        assert!(r.preparable);
        assert!(r.blocked_missing_recipient);
        assert!(!r.sendable);
        assert!(r.reasons.contains(&Reason::BlockedMissingRecipient));
    }

    #[test]
    fn zero_candidates_returns_empty_with_review_needed() {
        let p = preview(vec![]);
        let report = preview_dry_run(
            &p,
            &Selector::AllCurrentlyEligible,
            &DryRunFilters::default(),
            Page::default(),
        );
        assert!(report.ok);
        assert!(report.rows.is_empty());
        assert_eq!(report.review_needed.as_deref(), Some(REVIEW_NEEDED_EMPTY));
    }

    #[test]
    fn filter_cannot_bypass_classification() {
        let p = preview(vec![row(
            "a",
            Classification::NotEligible,
            vec![Reason::NoLivingEvidence],
        )]);
        let filters = DryRunFilters {
            classification: Some(Classification::EligibleForInvite),
            reason: None,
        };
        let report = preview_dry_run(
            &p,
            &Selector::ExplicitSelection(vec!["a".to_string()]),
            &filters,
            Page::default(),
        );
        assert!(report.rows.is_empty());
    }

    #[test]
    fn ambiguous_single_token_name_is_detected() {
        let parts = NameParts {
            family: None,
            middle: None,
            given: Some("An".to_string()),
        };
        assert!(name_is_ambiguous(&parts));
        let full = NameParts {
            family: Some("Nguyen".to_string()),
            middle: None,
            given: Some("An".to_string()),
        };
        assert!(!name_is_ambiguous(&full));
    }

    #[test]
    fn username_collision_is_detected() {
        assert!(username_collides("NVAn", &["nvan".to_string()]));
        assert!(!username_collides("NVAn", &["other".to_string()]));
    }

    #[test]
    fn dry_run_is_deterministic() {
        let p = preview(vec![row(
            "a",
            Classification::EligibleForInvite,
            vec![Reason::VerifiedLivingEvidence],
        )]);
        let r1 = preview_dry_run(
            &p,
            &Selector::AllCurrentlyEligible,
            &DryRunFilters::default(),
            Page::default(),
        );
        let r2 = preview_dry_run(
            &p,
            &Selector::AllCurrentlyEligible,
            &DryRunFilters::default(),
            Page::default(),
        );
        assert_eq!(r1, r2);
    }
}
