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
pub mod media;
pub mod profile;
pub mod profile_revisions;
pub mod reservation;
pub mod username;

pub use account_map::{identity_fingerprint, resolve_account, AccountMapError, IdentityAccount};
pub use candidate::{
    classify, preview_candidates, CandidateError, CandidatePreview, CandidateRow, Classification,
    PreviewSummary, Reason,
};
pub use invitation::{accept, create, expire_due, revoke, InvitationError, IssuedInvitation};
pub use link::{
    link_for_account, link_for_person, link_reviewed, revoke_link, LinkDecision, LinkError,
    LinkRecord,
};
pub use media::{
    read_authorized_media, read_owner_media, store_submission, withdraw_owned_media, MediaAsset,
    MediaDownload, MediaError, MediaKind, MediaStorage, DOCUMENT_FIELD, DOCUMENT_MAX_BYTES,
    PHOTO_FIELD, PHOTO_MAX_BYTES,
};
pub use profile::{read_own_profile, ProfileError, ProfileField, ProfileView};
pub use profile_revisions::{
    cancel_owned_draft, create_draft, create_owned_draft, is_allowed_profile_field, is_media_field,
    is_terminal, is_text_profile_field, list_open_revisions, list_revision_history,
    read_approved_values, read_owner_profile, review_revision, revision_by_id,
    submit_owned_revision, submit_revision, update_draft, update_owned_draft, ApprovedProfileValue,
    OwnerProfile, ProfileAudience, ProfileRevision, ProfileVisibility, ReviewDecision,
    RevisionError, RevisionStatus, ALLOWED_PROFILE_FIELDS, BIOGRAPHY_MAX_CHARS,
    MEDIA_PROFILE_FIELDS, NAME_MAX_CHARS, REVIEW_NOTE_MAX_CHARS,
};
pub use reservation::{claim, release, reserve, Reservation, ReservationError, ReservationStatus};
pub use username::NameParts;
