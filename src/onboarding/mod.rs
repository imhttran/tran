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
pub mod batch;
pub mod branch_review;
pub mod candidate;
pub mod delivery;
pub mod dry_run;
pub mod family_profile;
pub mod invitation;
pub mod link;
pub mod media;
pub mod profile;
pub mod profile_restriction;
pub mod profile_revisions;
pub mod reservation;
pub mod review_queue;
pub mod username;

pub use account_map::{identity_fingerprint, resolve_account, AccountMapError, IdentityAccount};
pub use batch::{
    confirm_batch, run_batch, BatchError, BatchItem, BatchKeys, BatchReport, ItemStatus, CHUNK_SIZE,
};
pub use branch_review::{
    active_branch_grants, grant_branches, has_active_branch_grant, list_branches,
    read_grant_snapshot, revoke_grant, BranchGrantError, BranchGrantEvent, BranchReviewerGrant,
    BranchSummary, GRANT_REASON_MAX_CHARS,
};
pub use candidate::{
    classify, load_name_parts, name_parts, preview_candidates, reclassify_person,
    segmentation_is_ambiguous, CandidateError, CandidatePreview, CandidateRow, Classification,
    PersonEvidence, PreviewSummary, Reason,
};
pub use delivery::{
    issue_deliverable_invitation, send_batch, sync_invitation_status, verify_recipient,
    DeliveryError, DeliveryReceipt, DeliveryStatus, FakeAdapter, MailAdapter, SendOutcome,
    SentMessage, UnconfiguredAdapter,
};
pub use dry_run::{
    name_is_ambiguous, preview_dry_run, username_collides, DryRunFilters, DryRunReport, DryRunRow,
    Page, Selector, REVIEW_NEEDED_EMPTY,
};
pub use family_profile::{
    read_family_profile, FamilyProfile, FamilyProfileError, FamilyProfileField, FamilyProfileMedia,
};
pub use invitation::{accept, create, expire_due, revoke, InvitationError, IssuedInvitation};
pub use link::{
    link_for_account, link_for_person, link_reviewed, revoke_link, LinkDecision, LinkError,
    LinkRecord,
};
pub use media::{
    read_authorized_media, read_family_media, read_owner_media, read_review_media,
    store_submission, withdraw_owned_media, MediaAsset, MediaDownload, MediaError, MediaKind,
    MediaStorage, OwnerMedia, DOCUMENT_FIELD, DOCUMENT_MAX_BYTES, PHOTO_FIELD, PHOTO_MAX_BYTES,
};
pub use profile::{read_own_profile, ProfileError, ProfileField, ProfileView};
pub use profile_restriction::{restrict_own_field, RestrictionError, RestrictionOutcome};
pub use profile_revisions::{
    cancel_owned_draft, create_draft, create_owned_draft, is_allowed_profile_field, is_media_field,
    is_terminal, is_text_profile_field, list_open_revisions, list_revision_history,
    read_approved_values, read_owner_profile, review_revision, revision_by_id,
    submit_owned_revision, submit_revision, update_draft, update_owned_draft, ApprovedProfileValue,
    OwnerProfile, ProfileAudience, ProfileRevision, ProfileVisibility, ReviewDecision,
    RevisionError, RevisionStatus, ALLOWED_PROFILE_FIELDS, BIOGRAPHY_MAX_CHARS,
    MEDIA_PROFILE_FIELDS, NAME_MAX_CHARS, NARRATIVE_MAX_CHARS, REVIEW_NOTE_MAX_CHARS,
};
pub use reservation::{claim, release, reserve, Reservation, ReservationError, ReservationStatus};
pub use review_queue::{
    read_review_detail, read_review_queue, ReviewDetail, ReviewEntry, ReviewEvent, ReviewMedia,
    ReviewQueue,
};
pub use username::NameParts;
