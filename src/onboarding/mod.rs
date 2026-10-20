//! Onboarding: candidate classification, username reservation, invitation
//! tokens, and reviewed identity linking.
//!
//! Operational output is privacy safe: logs, error bodies, and the preview
//! response carry classification reasons and opaque person keys only. They
//! never carry display names, dates, source excerpts, or private hashes.
//!
//! Invitation tokens (USR-003B) are random, expiring, and single use: only the
//! SHA-256 hash of a token is stored, and the plaintext is returned once.
//!
//! Links (USR-003C) resolve the immutable `(issuer, subject)` identity before a
//! link is written. The genealogy database stores only the opaque account id and
//! a one-way fingerprint.

pub mod account_map;
pub mod candidate;
pub mod invitation;
pub mod link;
pub mod reservation;
pub mod username;

pub use account_map::{identity_fingerprint, resolve_account, AccountMapError, IdentityAccount};
pub use candidate::{
    classify, preview_candidates, CandidateError, CandidatePreview, CandidateRow, Classification,
    PreviewSummary, Reason,
};
pub use invitation::{accept, create, expire_due, revoke, InvitationError, IssuedInvitation};
pub use link::{link_for_person, link_reviewed, revoke_link, LinkDecision, LinkError, LinkRecord};
pub use reservation::{claim, release, reserve, Reservation, ReservationError, ReservationStatus};
pub use username::NameParts;
