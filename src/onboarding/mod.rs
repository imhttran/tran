//! Onboarding of living person candidates (USR-002).
//!
//! This module derives a deterministic inventory of people who may be invited
//! to claim an account. Eligibility comes only from the private genealogy
//! database and reuses the TRAN-002 conventions:
//!
//! * an [`assertion_kind`] of `vital_status`;
//! * `review_status = 'verified'`;
//! * a living value (`alive` / `living`);
//! * attributed citations through
//!   `assertion_evidence -> source_evidence -> source_document`.
//!
//! Living status is never inferred from a missing `death_year` and never from a
//! model answer. A person with contradictory death evidence, a disputed review,
//! missing citations, an ambiguous identity, or an unresolved duplicate is
//! classified [`Classification::NeedsReview`] and is never automatically
//! invitable.
//!
//! Operational output is privacy safe: logs, error bodies, and the preview
//! response carry classification reasons and opaque person keys only. They
//! never carry display names, dates, source excerpts, or private hashes.

pub mod candidate;
pub mod reservation;
pub mod username;

pub use candidate::{
    classify, preview_candidates, CandidateError, CandidatePreview, CandidateRow, Classification,
    PreviewSummary, Reason,
};
pub use reservation::{claim, release, reserve, Reservation, ReservationError, ReservationStatus};
pub use username::NameParts;
