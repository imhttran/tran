# My Profile

The authenticated `/profile` page lets a verified family member contribute a
personal profile without creating a second profile, media, or review system. The
browser renders only what the owner-scoped Rust API returns, and every write is
re-authorized server-side.

## What the page renders

- **Profile header** — the linked person's display name, the current photo (or a
  placeholder), profile-completion guidance, and the current review status. A
  family branch is never inferred by the browser; the page says so explicitly.
- **About me** — display name, preferred name, biography, current location,
  contact preferences, and social/contact links.
- **My story** — one long narrative field for upbringing, education, career, life
  events, traditions, lessons, and advice.
- **Family memories** — recollections about parents, grandparents, traditions,
  and stories passed down. These remain member-submitted narratives and never
  create or modify genealogy relationships.
- **Photo and personal document** — private media upload, pending/approved
  status, authorized viewing or download, and withdrawal of a pending
  submission.
- **Review history** — pending, approved, rejected, and needs-changes decisions
  with reviewer feedback.

## Supported fields and limits

The browser renders exactly the fields the API accepts. Adding a field definition
to `web/src/lib/profile-fields.ts` is not enough: the Rust allowlist in
`src/onboarding/profile_revisions.rs` must accept it first.

| Field | Limit | Default visibility |
| --- | --- | --- |
| `display_name`, `preferred_name`, `current_location` | 200 characters | owner-private |
| `contact_preferences`, `facebook_url`, `linkedin_url`, `snapchat`, `whatsapp` | 500 characters | owner-private |
| `biography` | 5,000 characters | owner-private |
| `my_story`, `family_memories` | 20,000 characters | owner-private |

Every new value starts owner-private on the server. A member may explicitly
choose `family` visibility, which affects the last **approved** value only.

| Media | Accepted types | Limit |
| --- | --- | --- |
| Profile photo | JPEG, PNG | 5 MiB |
| Personal document | PDF, text, Markdown | 10 MiB |

Photos are decoded and re-encoded to PNG, which strips container metadata.
Documents are validated as PDF or UTF-8 text/Markdown and stored byte-for-byte.
Pending, rejected, and needs-changes media is owner-visible only.

## Review lifecycle

1. The owner saves a draft (`POST /api/profile/drafts` or
   `PATCH /api/profile/drafts/{id}`).
2. The owner submits it (`POST /api/profile/drafts/{id}/submit`).
3. An independent reviewer decides (`POST /api/profile/revisions/{id}/review`).
   The database rejects self-approval.
4. Rejected and needs-changes revisions keep their feedback and can be revised
   into a new draft.

The approved value in `onboarding_profile_value` remains visible and unchanged
while a newer revision is pending. Unapproved content is never returned as an
approved value, never rendered to another family member, and never sent to RAG
retrieval.

## Private media withdrawal audit

Withdrawing a pending media submission removes the pending revision, so the
revision's review events cascade with it. Migration
`0017_profile_media_withdrawal_audit.sql` adds an append-only
`onboarding_media_withdrawal` ledger written in the same transaction as the
withdrawal. It preserves the media id, revision id, person id, media kind, actor,
action, and timestamp — and stores no file content, file name, hash, or storage
path.

## Authorization boundaries

- A verified OIDC identity is required. The browser never supplies an account id,
  person id, or reviewer.
- Ownership comes from the reviewed `onboarding_person_account_link` row
  (`account_id` + `identity_fingerprint`).
- An unlinked, revoked, deactivated, or unprovisioned identity receives no
  private field. The page shows a safe sign-in or unlinked explanation.
- Cross-account reads, drafts, and media return `403`/`404`; the Rust API is the
  only authorization authority.
- State-changing requests go through the same-origin proxy, which requires the
  double-submit CSRF token.
- Private storage paths, file hashes, tokens, and contact fields are never placed
  in the DOM.

## Verification

- Rust: `tests/onboarding_profile_media.rs`,
  `tests/onboarding_profile_revisions.rs`, and the profile submission suites cover
  owner scoping, version conflicts, cross-account isolation, revoked links,
  malformed/oversized media, and the withdrawal audit.
- Browser: `web/e2e/my-profile.spec.ts` runs against the real Next.js proxy,
  Axum API, PostgreSQL schema, and synthetic OIDC issuer. It covers linked,
  unlinked, revoked-link, deactivated, expired-session, draft, submit, review,
  approved-during-review, upload/download, cross-account media, validation,
  network failure, mobile layout, and keyboard activation.
- The browser harness migrates genealogy `0001`–`0017` and seeds only synthetic
  members and links. It never uses real family data.

## Make private (owner restriction)

On an approved **Authorized family** field, **Make private** immediately stops
the value appearing on family profile pages. The approved revision and its
history are not rewritten; the owner keeps the value privately and the profile
shows that they restricted it. There is no toggle back: sharing it again is a
new family-visible revision that an independent reviewer must approve. A
submission created before the restriction cannot republish it.
