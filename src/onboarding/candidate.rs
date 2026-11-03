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
//!  3. `AlreadyPrepared` — a `prepared` row in `onboarding_batch_item`;
//!  4. `NeedsReview` — contradictory, disputed, uncited, ambiguous, or
//!     unresolved-duplicate vital evidence, ambiguous name segmentation, or a
//!     minor awaiting guardian review;
//!  5. `EligibleForInvite` — verified living evidence with attributed citations;
//!  6. `NotEligible` — no living evidence at all (for example deceased).
//!
//! Missing date of birth, a username collision, and a missing verified recipient
//! contact are recorded as warnings on an eligible candidate: they do not block
//! preparation, and a missing recipient blocks only sending.

use postgres::Client;
use thiserror::Error;

use crate::onboarding::username::NameParts;

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
    /// A batch already prepared this person.
    AlreadyPrepared,
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
            Classification::AlreadyPrepared => "already_prepared",
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
    /// A batch already prepared this person.
    AlreadyPrepared,
    /// No living evidence was found.
    NoLivingEvidence,
    /// Structured name parts segmented ambiguously.
    AmbiguousSegmentation,
    /// The candidate username collides with a taken reservation.
    UsernameCollision,
    /// The date of birth is missing, so age cannot be verified.
    MissingDob,
    /// A minor needs guardian review before onboarding.
    MinorGuardianReview,
    /// No verified contact email exists, so the candidate cannot be contacted.
    MissingVerifiedContact,
    /// The candidate is preparable but blocked from sending because no verified
    /// recipient address exists.
    BlockedMissingRecipient,
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
            Reason::AlreadyPrepared => "already_prepared",
            Reason::NoLivingEvidence => "no_living_evidence",
            Reason::AmbiguousSegmentation => "ambiguous_segmentation",
            Reason::UsernameCollision => "username_collision",
            Reason::MissingDob => "missing_dob",
            Reason::MinorGuardianReview => "minor_guardian_review",
            Reason::MissingVerifiedContact => "missing_verified_contact",
            Reason::BlockedMissingRecipient => "blocked_missing_recipient",
        }
    }
}

/// One classified candidate.
///
/// Carries the opaque person key, the display name, and a *masked* verified
/// contact email. It never carries a date or a source excerpt, and never an
/// unmasked email address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRow {
    /// The stable opaque catalog key.
    pub person_key: String,
    /// The person display name, used to label the preview row.
    pub display_name: String,
    /// The classification.
    pub classification: Classification,
    /// The ordered, deduplicated reason codes.
    pub reasons: Vec<Reason>,
    /// The proposed display username, when the verified name parts permit one.
    /// This is a proposal only; preparation resolves collisions transactionally.
    pub proposed_username: Option<String>,
    /// The verified contact email, masked for the wire. `None` when no verified
    /// contact exists. This never carries the unmasked address.
    pub contact_email_masked: Option<String>,
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
    /// Count of `already_prepared`.
    pub already_prepared: usize,
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
            already_prepared: preview.count(Classification::AlreadyPrepared),
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
    /// True when a batch item already prepared this person.
    already_prepared: bool,
    /// True when the structured name segments ambiguously.
    ambiguous_segmentation: bool,
    /// True when the candidate username collides with a taken reservation.
    username_collision: bool,
    /// True when the date of birth is missing.
    missing_dob: bool,
    /// True when the person is a minor or needs guardian review.
    minor_guardian_review: bool,
    /// True when no verified contact email is available.
    missing_verified_contact: bool,
    /// The verified contact email, masked for the wire. `None` when no verified
    /// contact exists.
    contact_email_masked: Option<String>,
    /// The person display name, read by the same evidence query.
    display_name: String,
}

/// Returns `true` when the text names a living status. Only explicit living
/// values qualify; a missing death year is never treated as living.
fn is_living_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "alive" | "living"
    )
}

/// Masks an email address for display.
///
/// Keeps the first local-part character when the local part has more than one
/// character, replaces the rest of the local part with a fixed `•••`, and keeps
/// the domain. A one-character local part becomes `•••@domain`, and an input
/// with no `@` is masked in full.
pub fn mask_email(email: &str) -> String {
    let email = email.trim();
    let Some((local, domain)) = email.split_once('@') else {
        return "•••".to_string();
    };
    let mut chars = local.chars();
    let first = chars.next();
    let masked_local = match first {
        Some(first) if chars.next().is_some() => format!("{first}•••"),
        _ => "•••".to_string(),
    };
    format!("{masked_local}@{domain}")
}

/// Splits a verified display name into structured parts without inventing a
/// person: the first token is the family name, the middle tokens are the middle
/// name, and the last token is the given name. Explicit structured columns win.
///
/// A one-token display name cannot be segmented safely; callers mark it
/// ambiguous instead of guessing.
pub fn name_parts(
    family_name: Option<String>,
    given_name: Option<String>,
    display_name: &str,
) -> NameParts {
    let tokens: Vec<String> = display_name
        .split_whitespace()
        .map(|token| token.to_string())
        .collect();
    let family = family_name
        .filter(|value| !value.trim().is_empty())
        .or_else(|| tokens.first().cloned());
    let given = given_name
        .filter(|value| !value.trim().is_empty())
        .or_else(|| tokens.last().cloned());
    let middle = if tokens.len() >= 3 {
        Some(tokens[1..tokens.len() - 1].join(" "))
    } else {
        None
    };
    NameParts {
        family,
        middle,
        given,
    }
}

/// Returns `true` when the name cannot be segmented safely.
///
/// Explicit structured components win: a record with both a family and a given
/// name is unambiguous even when its display label is a single token.
pub fn segmentation_is_ambiguous(
    display_name: &str,
    family_name: Option<&str>,
    given_name: Option<&str>,
) -> bool {
    let has_family = family_name.is_some_and(|value| !value.trim().is_empty());
    let has_given = given_name.is_some_and(|value| !value.trim().is_empty());
    if has_family && has_given {
        return false;
    }
    display_name.split_whitespace().count() < 2
}

/// Reads the verified structured name parts for one person.
pub fn load_name_parts(client: &mut Client, person_id: i64) -> Result<NameParts, CandidateError> {
    let row = client
        .query_opt(
            "SELECT family_name, given_name, display_name FROM person WHERE person_id = $1",
            &[&person_id],
        )
        .map_err(|err| CandidateError::Database(err.to_string()))?
        .ok_or_else(|| CandidateError::Database("unknown person".to_string()))?;
    let display: String = row.get(2);
    Ok(name_parts(row.get(0), row.get(1), &display))
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
              WHERE person_id = $1 AND status = 'pending'
                AND (expires_at IS NULL OR expires_at > now())",
            &[&person_id],
        )
        .map_err(|err| CandidateError::Database(err.to_string()))?
        .get(0);

    let already_prepared: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_batch_item
              WHERE person_id = $1 AND status = 'prepared'",
            &[&person_id],
        )
        .map_err(|err| CandidateError::Database(err.to_string()))?
        .get(0);

    let contact_email: Option<String> = client
        .query_opt(
            "SELECT email FROM onboarding_recipient_contact
              WHERE person_id = $1 AND verified_at IS NOT NULL
              ORDER BY verified_at DESC LIMIT 1",
            &[&person_id],
        )
        .map_err(|err| CandidateError::Database(err.to_string()))?
        .map(|row| row.get(0));
    let missing_verified_contact = contact_email.is_none();
    let contact_email_masked = contact_email.as_deref().map(mask_email);

    let person = client
        .query_opt(
            "SELECT family_name, given_name, display_name, birth_year,
                    (birth_year IS NULL OR btrim(birth_year) = '') AS missing_dob,
                    CASE WHEN birth_year IS NULL OR btrim(birth_year) = ''
                         THEN false
                         WHEN birth_year ~ '^[0-9]{4}$'
                         THEN (EXTRACT(YEAR FROM now())::int - birth_year::int) < 18
                         ELSE true END AS minor
               FROM person WHERE person_id = $1",
            &[&person_id],
        )
        .map_err(|err| CandidateError::Database(err.to_string()))?
        .ok_or_else(|| CandidateError::Database("unknown person".to_string()))?;
    let display_name: String = person.get(2);
    let missing_dob: bool = person.get(4);
    let minor_guardian_review: bool = person.get(5);
    let ambiguous_segmentation = segmentation_is_ambiguous(
        &display_name,
        person.get::<_, Option<String>>(0).as_deref(),
        person.get::<_, Option<String>>(1).as_deref(),
    );

    let parts = name_parts(person.get(0), person.get(1), &display_name);
    let username_collision = crate::onboarding::username::candidate_username(&parts)
        .ok()
        .map(|base| {
            let key = crate::onboarding::username::key(&base);
            client
                .query_one(
                    "SELECT count(*) FROM onboarding_username_reservation
                      WHERE username_key = $1 AND status <> 'released'",
                    &[&key],
                )
                .map(|row| row.get::<_, i64>(0) > 0)
                .unwrap_or(false)
        })
        .unwrap_or(false);

    Ok(PersonEvidence {
        has_verified_living,
        missing_citation,
        contradictory,
        disputed,
        duplicate_key,
        already_linked: already_linked > 0,
        invited: invited > 0,
        already_prepared: already_prepared > 0,
        ambiguous_segmentation,
        username_collision,
        missing_dob,
        minor_guardian_review,
        missing_verified_contact,
        contact_email_masked,
        display_name,
    })
}

/// Re-evaluates one person and returns the classification and reasons.
///
/// This is the shared server-side revalidation used by preview, dry-run, and
/// batch preparation. It performs no writes.
pub fn reclassify_person(
    client: &mut Client,
    person_id: i64,
    person_key: &str,
) -> Result<(Classification, Vec<Reason>), CandidateError> {
    let evidence = load_person_evidence(client, person_id, person_key)?;
    Ok(classify(&evidence))
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
    let mut blockers: Vec<Reason> = Vec::new();
    if evidence.duplicate_key {
        blockers.push(Reason::UnresolvedDuplicate);
    }
    if evidence.contradictory {
        blockers.push(Reason::ContradictoryVital);
    }
    if evidence.disputed {
        blockers.push(Reason::Disputed);
    }
    if evidence.missing_citation {
        blockers.push(Reason::MissingCitation);
    }
    if evidence.ambiguous_segmentation {
        blockers.push(Reason::AmbiguousSegmentation);
    }
    if evidence.minor_guardian_review {
        blockers.push(Reason::MinorGuardianReview);
    }

    // Warnings do not block preparation. A username collision is resolved with
    // a deterministic suffix, a missing DOB does not invalidate a verified
    // living adult, and a missing recipient blocks only sending.
    let mut warnings: Vec<Reason> = Vec::new();
    if evidence.username_collision {
        warnings.push(Reason::UsernameCollision);
    }
    if evidence.missing_dob {
        warnings.push(Reason::MissingDob);
    }
    if evidence.missing_verified_contact {
        warnings.push(Reason::MissingVerifiedContact);
    }

    if !blockers.is_empty() {
        blockers.extend(warnings);
        return (Classification::NeedsReview, blockers);
    }

    if evidence.has_verified_living {
        if evidence.already_prepared {
            let mut reasons = vec![Reason::AlreadyPrepared];
            reasons.extend(warnings);
            (Classification::AlreadyPrepared, reasons)
        } else {
            let mut reasons = vec![Reason::VerifiedLivingEvidence];
            reasons.extend(warnings);
            (Classification::EligibleForInvite, reasons)
        }
    } else {
        let mut reasons = vec![Reason::NoLivingEvidence];
        reasons.extend(warnings);
        (Classification::NotEligible, reasons)
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
        let parts = load_name_parts(client, person_id)?;
        rows.push(CandidateRow {
            person_key,
            display_name: evidence.display_name.clone(),
            classification,
            reasons,
            proposed_username: crate::onboarding::username::candidate_username(&parts).ok(),
            contact_email_masked: evidence.contact_email_masked.clone(),
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
            already_prepared: false,
            ambiguous_segmentation: false,
            username_collision: false,
            missing_dob: false,
            minor_guardian_review: false,
            missing_verified_contact: false,
            contact_email_masked: None,
            display_name: String::new(),
        }
    }

    #[test]
    fn mask_email_masks_local_part_and_keeps_domain() {
        assert_eq!(mask_email("tran@gmail.com"), "t•••@gmail.com");
    }

    #[test]
    fn mask_email_handles_one_character_local_part() {
        assert_eq!(mask_email("a@example.com"), "•••@example.com");
    }

    #[test]
    fn mask_email_uses_a_fixed_three_character_mask() {
        assert_eq!(mask_email("VeryLongLocal@example.org"), "V•••@example.org");
    }

    #[test]
    fn mask_email_handles_uppercase_input() {
        assert_eq!(mask_email("Tran@Gmail.com"), "T•••@Gmail.com");
    }

    #[test]
    fn mask_email_never_leaks_the_local_part() {
        for (input, local) in [
            ("tran@gmail.com", "tran"),
            ("a@example.com", "a"),
            ("VeryLongLocal@example.org", "VeryLongLocal"),
        ] {
            let masked = mask_email(input);
            let remainder = &local[local
                .char_indices()
                .nth(1)
                .map(|(i, _)| i)
                .unwrap_or(local.len())..];
            if !remainder.is_empty() {
                assert!(!masked.contains(remainder));
            }
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

    #[test]
    fn structured_name_parts_resolve_a_single_token_display_name() {
        assert!(!segmentation_is_ambiguous(
            "Madonna",
            Some("Tran"),
            Some("Madonna")
        ));
        assert!(segmentation_is_ambiguous("Madonna", None, None));
        assert!(segmentation_is_ambiguous("Madonna", Some("Tran"), None));
        assert!(segmentation_is_ambiguous("Madonna", None, Some("Madonna")));
    }

    #[test]
    fn ambiguous_segmentation_is_needs_review() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.ambiguous_segmentation = true;
        let (classification, reasons) = classify(&e);
        assert_eq!(classification, Classification::NeedsReview);
        assert!(reasons.contains(&Reason::AmbiguousSegmentation));
    }

    #[test]
    fn username_collision_is_a_warning_not_a_blocker() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.username_collision = true;
        let (classification, reasons) = classify(&e);
        assert_eq!(classification, Classification::EligibleForInvite);
        assert!(reasons.contains(&Reason::UsernameCollision));
    }

    #[test]
    fn missing_dob_is_a_warning_not_a_blocker() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.missing_dob = true;
        let (classification, reasons) = classify(&e);
        assert_eq!(classification, Classification::EligibleForInvite);
        assert!(reasons.contains(&Reason::MissingDob));
    }

    #[test]
    fn minor_guardian_review_is_needs_review() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.minor_guardian_review = true;
        let (classification, reasons) = classify(&e);
        assert_eq!(classification, Classification::NeedsReview);
        assert!(reasons.contains(&Reason::MinorGuardianReview));
    }

    #[test]
    fn missing_verified_contact_is_preparable_but_not_sendable() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.missing_verified_contact = true;
        let (classification, reasons) = classify(&e);
        assert_eq!(classification, Classification::EligibleForInvite);
        assert!(reasons.contains(&Reason::MissingVerifiedContact));
    }

    #[test]
    fn revoked_evidence_blocks_an_already_prepared_person() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.disputed = true;
        e.already_prepared = true;
        assert_eq!(classify(&e).0, Classification::NeedsReview);
    }

    #[test]
    fn eligible_and_already_prepared_is_already_prepared() {
        let mut e = evidence();
        e.has_verified_living = true;
        e.already_prepared = true;
        assert_eq!(classify(&e).0, Classification::AlreadyPrepared);
    }

    #[test]
    fn missing_living_evidence_beats_already_prepared() {
        let mut e = evidence();
        e.already_prepared = true;
        assert_eq!(classify(&e).0, Classification::NotEligible);
    }
}
