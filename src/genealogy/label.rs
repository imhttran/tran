//! Review labels for callers (TRAN-002D).
//!
//! [`review_label`] maps an assertion's review status and evidence onto a
//! caller-facing [`ReviewLabel`]. Unknown facts stay `Unknown` and are never
//! relabeled as `Verified`. A disputed fact is surfaced as `Disputed` and is
//! never hidden. A rejected fact carries the reason from the latest event.
//!
//! A caller that reads a genealogy assertion should place this label next to
//! the asserted fact so the reader can see that it is not confirmed.

/// The caller-facing review label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewLabel {
    /// Reviewed, verified, and the evidence still matches the baseline.
    Verified,
    /// A reviewer disputes the claim.
    Disputed,
    /// The claim is rejected, with the reason from the latest event.
    Rejected { reason: String },
    /// The claim is unknown: unverified or needs_review with no evidence.
    Unknown,
}

/// The inputs needed to compute a label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelInput {
    /// The raw `review_status` text.
    pub review_status: String,
    /// Whether at least one `assertion_evidence` row exists.
    pub has_evidence: bool,
    /// Whether a stored baseline fingerprint matches the current one.
    pub baseline_matches: bool,
    /// The reason text from the latest event, used for `Rejected`.
    pub latest_reason: Option<String>,
}

/// Computes the caller-facing label.
pub fn review_label(input: &LabelInput) -> ReviewLabel {
    match input.review_status.as_str() {
        "verified" if input.has_evidence && input.baseline_matches => ReviewLabel::Verified,
        "disputed" => ReviewLabel::Disputed,
        "rejected" => ReviewLabel::Rejected {
            reason: input
                .latest_reason
                .clone()
                .unwrap_or_else(|| "no reason recorded".to_string()),
        },
        _ => ReviewLabel::Unknown,
    }
}
