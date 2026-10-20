//! Deterministic living-person candidate query and classification (USR-002).
//!
//! [`preview_candidates`] runs a strictly read-only query against the private
//! genealogy database and classifies every cataloged person. It writes nothing:
//! no invitation, no link, no reservation, and no audit row. The same database
//! state always yields the same classification for the same person.
//!
//! Classification precedence (first match wins):
//!  1. `AlreadyLinked` — a row in `onboarding_person_account_link`;
//!  2. `Invited` — a pending row in `onboarding_invitation`;
//!  3. `NeedsReview` — contradictory, disputed, uncited, ambiguous, or
//!     unresolved-duplicate vital evidence;
//!  4. `EligibleForInvite` — verified living evidence with attributed citations;
//!  5. `NotEligible` — no living evidence at all (for example deceased).

use postgres::Client;
use thiserror::Error;

/// The classification of one person for the onboarding inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// Verified living evidence with attributed citations and no blockers.
    EligibleForInvite,
    /// Contradictory, disputed, uncited, ambiguous, or unresolved-duplicate.
    NeedsReview,
    /// The person already has a linked account.
    AlreadyLinked,
    /// An invitation is already pending.
    Invited,
    /// No living evidence (for example a confident deceased person).
    NotEligible,
}

impl Classification {
    /// The stable wire name. This never contains private data.
    pub fn as_str(self) -> &'static str {
        match self {
            Classification::EligibleForInvite => "eligible_for_invite",
            Classification::NeedsReview => "needs_review",
            Classification::AlreadyLinked => "already_linked",
            Classification::Invited => "invited",
            Classification::NotEligible => "not_eligible",
        }
    }
}

/// A privacy-safe reason code for a classification decision.
///
/// Reason codes name a rule, never a person, a date, a source excerpt, or a
/// private hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Verified living evidence with attributed citations.
    VerifiedLivingEvidence,
    /// Two vital-status assertions disagree (contradictory death evidence).
    ContradictoryVital,
    /// A reviewer disputed the vital evidence.
    Disputed,
    /// The living assertion has no attributed citation.
    MissingCitation,
    /// The identity is ambiguous (more than one cataloged person matches).
    Ambiguous,
    /// More than one person row shares the stable key (unresolved duplicate).
    UnresolvedDuplicate,
    /// The person already has a linked account.
    AlreadyLinked,
    /// An invitation is already pending.
    AlreadyInvited,
    /// No living evidence was found.
    NoLivingEvidence,
}

impl Reason {
    /// The stable reason code string.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::VerifiedLivingEvidence => "verified_living_evidence",
            Reason::ContradictoryVital => "contradictory_vital",
            Reason::Disputed => "disputed",
            Reason::MissingCitation => "missing_citation",
            Reason::Ambiguous => "ambiguous",
            Reason::UnresolvedDuplicate => "unresolved_duplicate",
            Reason::AlreadyLinked => "already_linked",
            Reason::AlreadyInvited => "already_invited",
            Reason::NoLivingEvidence => "no_living_evidence",
        }
    }
}

/// One classified candidate. Carries the opaque person key and reason codes
/// only; never a display name, a date, or a source excerpt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRow {
    /// The stable opaque catalog key.
    pub person_key: String,
    /// The classification.
    pub classification: Classification,
    /// The ordered, deduplicated reason codes.
    pub reasons: Vec<Reason>,
}

/// The aggregated preview for the admin endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidatePreview {
    /// The classified rows, ordered by person key.
    pub rows: Vec<CandidateRow>,
}

impl CandidatePreview {
    /// The count of rows per classification for one classification.
    pub fn count(&self, classification: Classification) -> usize {
        self.rows
            .iter()
            .filter(|row| row.classification == classification)
            .count()
    }
}

/// A per-classification tally. Used for summaries that never expose names.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreviewSummary {
    /// Count of `eligible_for_invite`.
    pub eligible_for_invite: usize,
    /// Count of `needs_review`.
    pub needs_review: usize,
    /// Count of `already_linked`.
    pub already_linked: usize,
    /// Count of `invited`.
    pub invited: usize,
    /// Count of `not_eligible`.
    pub not_eligible: usize,
}

impl PreviewSummary {
    /// Builds the summary from a preview.
    pub fn from_preview(preview: &CandidatePreview) -> Self {
        Self {
            eligible_for_invite: preview.count(Classification::EligibleForInvite),
            needs_review: preview.count(Classification::NeedsReview),
            already_linked: preview.count(Classification::AlreadyLinked),
            invited: preview.count(Classification::Invited),
            not_eligible: preview.count(Classification::NotEligible),
        }
    }
}

/// Errors raised by the onboarding candidate query.
///
/// Error messages carry rule names and opaque keys only. They never carry a
/// display name, a date, a source excerpt, or a private hash.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CandidateError {
    /// The database query failed.
    #[error("onboarding candidate query failed")]
    Database(String),
}

/// The evidence state gathered for one person from the private genealogy DB.
pub struct PersonEvidence {
    /// True when a `vital_status` assertion with a living value is verified and
    /// cited.
    has_verified_living: bool,
    /// True when any living `vital_status` assertion lacks an attributed
    /// citation chain.
    missing_citation: bool,
    /// True when two `vital_status` assertions share a key with different
    /// values.
    contradictory: bool,
    /// True when a vital-status assertion is disputed.
    disputed: bool,
    /// True when the catalog has more than one row for the same stable key.
    duplicate_key: bool,
    /// True when a person-account link exists.
    already_linked: bool,
    /// True when a pending invitation exists.
    invited: bool,
}

/// Returns `true` when the text names a living status. Only explicit living
/// values qualify; a missing death year is never treated as living.
fn is_living_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "alive" | "living"
    )
}

/// Loads the evidence state for one person id.
fn load_person_evidence(
    client: &mut Client,
    person_id: i64,
    person_key: &str,
) -> Result<PersonEvidence, CandidateError> {
    let rows = client
        .query(
            "SELECT a.assertion_key, a.value_text, a.review_status,
                    (SELECT count(*) FROM assertion_evidence ae
                       JOIN source_evidence se
                         ON se.source_evidence_id = ae.source_evidence_id
                       JOIN source_document sd
                         ON sd.source_document_id = se.source_document_id
                      WHERE ae.assertion_id = a.assertion_id) AS citation_count
               FROM assertion a
              WHERE a.person_id = $1 AND a.assertion_kind = 'vital_status'",
            &[&person_id],
        )
        .map_err(|err| CandidateError::Database(err.to_string()))?;

    let mut vital_values: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    let mut has_verified_living = false;
    let mut missing_citation = false;
    let mut disputed = false;

    for row in &rows {
        let key: String = row.get(0);
        let value: Option<String> = row.get(1);
        let status: String = row.get(2);
        let citations: i64 = row.get(3);

        if status == "disputed" {
            disputed = true;
        }

        let Some(value) = value else { continue };
        vital_values
            .entry(key)
            .or_default()
            .insert(value.trim().to_ascii_lowercase());

        // An unverified living claim is not enough, but it is also not a
        // contradiction. It stays not eligible on its own.
        if is_living_value(&value) && status == "verified" {
            if citations > 0 {
                has_verified_living = true;
            } else {
                missing_citation = true;
            }
        }
    }

    let contradictory = vital_values.values().any(|values| values.len() >= 2);

    // Duplicate catalog key: more than one person row shares the key.
    let duplicate_count: i64 = client
        .query_one(
            "SELECT count(*) FROM person WHERE person_key = $1",
            &[&person_key],
        )
        .map_err(|err| CandidateError::Database(err.to_string()))?
        .get(0);
    let duplicate_key = duplicate_count > 1;

    let already_linked: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_person_account_link WHERE person_id = $1",
            &[&person_id],
        )
        .map_err(|err| CandidateError::Database(err.to_string()))?
        .get(0);

    let invited: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_invitation
              WHERE person_id = $1 AND status = 'pending'",
            &[&person_id],
        )
        .map_err(|err| CandidateError::Database(err.to_string()))?
        .get(0);

    Ok(PersonEvidence {
        has_verified_living,
        missing_citation,
        contradictory,
        disputed,
        duplicate_key,
        already_linked: already_linked > 0,
        invited: invited > 0,
    })
}

/// Classifies one person's gathered evidence into a deterministic outcome.
///
/// This function has no database access; it is pure so the precedence rules are
/// unit tested directly.
pub fn classify(evidence: &PersonEvidence) -> (Classification, Vec<Reason>) {
    if evidence.already_linked {
        return (Classification::AlreadyLinked, vec![Reason::AlreadyLinked]);
    }
    if evidence.invited {
        return (Classification::Invited, vec![Reason::AlreadyInvited]);
    }

    let mut reasons: Vec<Reason> = Vec::new();
    if evidence.duplicate_key {
        reasons.push(Reason::UnresolvedDuplicate);
    }
    if evidence.contradictory {
        reasons.push(Reason::ContradictoryVital);
    }
    if evidence.disputed {
        reasons.push(Reason::Disputed);
    }
    if evidence.missing_citation {
        reasons.push(Reason::MissingCitation);
    }
    if !reasons.is_empty() {
        return (Classification::NeedsReview, reasons);
    }

    if evidence.has_verified_living {
        (
            Classification::EligibleForInvite,
            vec![Reason::VerifiedLivingEvidence],
        )
    } else {
        (Classification::NotEligible, vec![Reason::NoLivingEvidence])
    }
}

/// Runs the read-only candidate query and classification.
///
/// This performs no writes. It is the dry-run body: no invitation, link,
/// reservation, or audit row is created.
pub fn preview_candidates(client: &mut Client) -> Result<CandidatePreview, CandidateError> {
    let people = client
        .query(
            "SELECT person_id, person_key FROM person ORDER BY person_key, person_id",
            &[],
        )
        .map_err(|err| CandidateError::Database(err.to_string()))?;

    let mut rows = Vec::with_capacity(people.len());
    for person in &people {
        let person_id: i64 = person.get(0);
        let person_key: String = person.get(1);
        let evidence = load_person_evidence(client, person_id, &person_key)?;
        let (classification, reasons) = classify(&evidence);
        rows.push(CandidateRow {
            person_key,
            classification,
            reasons,
        });
    }
    Ok(CandidatePreview { rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence() -> PersonEvidence {
        PersonEvidence {
            has_verified_living: false,
            missing_citation: false,
            contradictory: false,
            disputed: false,
            duplicate_key: false,
            already_linked: false,
            invited: false,
        }
    }

    #[test]
    fn verified_living_with_citation_is_eligible() {
        let mut e = evidence();
        e.has_verified_living = true;
        assert_eq!(classify(&e).0, Classification::EligibleForInvite);
        assert_eq!(classify(&e).1, vec![Reason::VerifiedLivingEvidence]);
    }

    #[test]
    fn deceased_without_living_evidence_is_not_eligible() {
        let e = evidence();
        assert_eq!(classify(&e).0, Classification::NotEligible);
    }

    #[test]
    fn contradictory_vital_is_needs_review() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.contradictory = true;
        let (classification, reasons) = classify(&e);
        assert_eq!(classification, Classification::NeedsReview);
        assert!(reasons.contains(&Reason::ContradictoryVital));
    }

    #[test]
    fn disputed_is_needs_review() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.disputed = true;
        assert_eq!(classify(&e).0, Classification::NeedsReview);
    }

    #[test]
    fn missing_citation_is_needs_review() {
        let mut e = evidence();
        e.missing_citation = true;
        let (classification, reasons) = classify(&e);
        assert_eq!(classification, Classification::NeedsReview);
        assert!(reasons.contains(&Reason::MissingCitation));
    }

    #[test]
    fn unresolved_duplicate_is_needs_review() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.duplicate_key = true;
        let (classification, reasons) = classify(&e);
        assert_eq!(classification, Classification::NeedsReview);
        assert!(reasons.contains(&Reason::UnresolvedDuplicate));
    }

    #[test]
    fn already_linked_takes_precedence() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.already_linked = true;
        assert_eq!(classify(&e).0, Classification::AlreadyLinked);
    }

    #[test]
    fn invited_takes_precedence_over_review() {
        let mut e = evidence();
        e.disputed = true;
        e.invited = true;
        assert_eq!(classify(&e).0, Classification::Invited);
    }

    #[test]
    fn classification_is_deterministic() {
        let mut e = evidence();
        e.has_verified_living = true;
        assert_eq!(classify(&e), classify(&e));
    }

    #[test]
    fn living_value_requires_explicit_value() {
        assert!(is_living_value("alive"));
        assert!(is_living_value("Living"));
        assert!(!is_living_value("deceased"));
        assert!(!is_living_value(""));
    }
}
