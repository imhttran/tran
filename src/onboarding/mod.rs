//! Onboarding: candidate classification, username reservation, and invitation
//! tokens.
//!
//! Operational output is privacy safe: logs, error bodies, and the preview
//! response carry classification reasons and opaque person keys only. They
//! never carry display names, dates, source excerpts, or private hashes.
//!
//! Invitation tokens (USR-003B) are random, expiring, and single use: only the
//! SHA-256 hash of a token is stored, and the plaintext is returned once.

pub mod candidate;
pub mod invitation;
pub mod reservation;
pub mod username;

pub use candidate::{
    classify, preview_candidates, CandidateError, CandidatePreview, CandidateRow, Classification,
    PreviewSummary, Reason,
};
pub use invitation::{accept, create, expire_due, revoke, InvitationError, IssuedInvitation};
pub use reservation::{claim, release, reserve, Reservation, ReservationError, ReservationStatus};
pub use username::NameParts;
