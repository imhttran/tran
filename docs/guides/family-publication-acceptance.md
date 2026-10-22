# Family publication acceptance matrix

This matrix is the acceptance plan for the publication boundary defined in
[`docs/architecture/FAMILY-PUBLICATION-CONTRACT.md`](../architecture/FAMILY-PUBLICATION-CONTRACT.md).

The test names and files cited below were checked against this branch's tree;
run the commands in the next section to reproduce them.

**Recorded runs.** USR-006-B executed the browser suites twice against one pair
of disposable databases: the full suite passed once (118/118), and the
deterministic set — every spec except the stochastic `rag.spec.ts` — passed
twice (113/113 each). The full suite's second attempt failed only in
`rag.spec.ts`, which is the unresolved PR #54 P1 recorded under row 13. Rows
marked PASS for browser behaviour are backed by that executed run, not by
inspection.

Status meanings:

- **PASS** — an existing, executed test proves the scenario for the content
  types that exist today.
- **PARTIAL** — the proven mechanism covers part of the scenario; the remaining
  part is named and has a specification below.
- **NOT IMPLEMENTED** — no publication or presentation path exists yet, so the
  scenario is a specification for a later slice. It is not claimed as passing.

Run the existing evidence with:

```sh
cargo test --locked
cargo test --locked --test document_lifecycle
cargo test --locked --test document_processing
cargo test --locked --test onboarding_profile_revisions
cargo test --locked --test onboarding_profile_submission
cargo test --locked --test onboarding_profile_media
cargo test --locked --test branch_review
make web-check
```

Database-backed tests take `DATABASE_URL` and `GENEALOGY_DATABASE_URL` and use
disposable databases only. No test in this matrix touches real family data.

Browser checks in `web/e2e/*` skip when the harness variables are absent; they
are executed through `scripts/web-e2e.sh`, which starts the synthetic OIDC
issuer and seeds the disposable genealogy database. A skipped browser file is
not evidence for a PASS row — only an executed `make web-e2e` run is.

## Matrix

| # | Scenario | Status | Evidence or specification |
| --- | --- | --- | --- |
| 1 | Draft profile values excluded | PASS | `tests/onboarding_profile_revisions.rs::unapproved_drafts_never_appear_in_approved_values`; `tests/onboarding_profile_submission.rs::pending_content_never_reaches_family_or_rag` |
| 2 | Pending revisions excluded | PASS | `tests/onboarding_profile_revisions.rs::draft_and_submission_keep_the_approved_value_until_independent_approval`; `pending_content_never_reaches_family_or_rag` |
| 3 | Rejected revisions excluded | PASS | `tests/onboarding_profile_revisions.rs::rejection_and_needs_changes_leave_the_approved_value_unchanged`; `pending_content_never_reaches_family_or_rag` |
| 4 | Approved owner-private values excluded | PASS | `tests/onboarding_profile_revisions.rs::approved_values_are_filtered_by_audience`, `::owner_private_and_unapproved_values_stay_out_of_family_audience` |
| 5 | Approved family-visible values eligible for presentation | PASS | Service: `tests/family_profile_presentation.rs` (including `inconsistent_or_missing_provenance_is_never_presented`). HTTP: `web/e2e/family-profile.spec.ts`. UI: `web/e2e/family-profile-ui.spec.ts` — a linked member opens `/family/{person_key}` from the family tree and reads the approved biography and narrative fields; see [family-profile-presentation.md](family-profile-presentation.md) |
| 6 | Private media excluded | PASS | `tests/onboarding_profile_media.rs::photo_is_private_until_approved_and_current`, `::document_is_private_until_approved_and_current`, `::cross_account_and_revoked_link_cannot_access_media` |
| 7 | Disputed genealogy facts excluded | PASS | `tests/branch_review.rs::a_disputed_fact_is_not_verified_by_submission_alone`; `src/genealogy/tree.rs` only marks verified facts as verified and returns citations only for verified edges. Publication of genealogy facts is out of scope (contract §4) |
| 8 | Superseded versions excluded | PASS | Documents: `tests/rag_publication.rs::superseded_cross_corpus_and_revalidation_are_exact` (superseded and removed versions do not resolve, cross-corpus exactness), `tests/document_lifecycle.rs::lifecycle_is_additive_quarantined_and_approved_only`, `tests/document_processing.rs::processing_is_durable_idempotent_and_approved_only`. Profile: `tests/onboarding_profile_revisions.rs::approved_revision_number_is_part_of_the_conflict_check` and the presentation join, which reads only the value bound to the current approved revision. Media: `tests/onboarding_profile_media.rs::replacement_owner_sees_only_current_approved_or_own_media` |
| 9 | Removal revokes presentation | PASS | `tests/document_lifecycle.rs::lifecycle_is_additive_quarantined_and_approved_only` (remove → retrieval empty); `tests/onboarding_profile_media.rs::owner_can_withdraw_pending_media_and_reupload` |
| 10 | Visibility changes revoke publication | PASS | Documents: `tests/document_lifecycle.rs::lifecycle_is_additive_quarantined_and_approved_only` (a scope change resets the version to review). Profile values: decision A2 is implemented — `tests/profile_restriction.rs` (immediate exclusion from the family read, owner view keeps the value, stale approval refused, post-restriction expansion lifts the restriction, concurrent approval/restriction serialize, and a restriction still waiting for its lock cannot be lifted by a submission created meanwhile) and `web/e2e/profile-privacy.spec.ts` (owner **Make private**, family API/page exclusion, reload persistence, independent re-approval restores). Ordering is the monotonic restriction generation, never a transaction-start timestamp; a submission from an earlier generation and a legacy submission with no generation both fail closed. See F2 below |
| 11 | Account/person removal prevents unauthorized visibility | PASS | `tests/onboarding_profile_media.rs::cross_account_and_revoked_link_cannot_access_media`; `tests/onboarding_profile_submission.rs::revoked_link_blocks_further_owner_writes`; `tests/branch_review.rs::linked_owner_and_replaced_account_cannot_use_a_grant` |
| 12 | Stale indexing cannot republish revoked content | PASS | `tests/document_processing.rs::processing_is_durable_idempotent_and_approved_only` — an approval withdrawn after queueing makes the job fail and writes no chunks |
| 13 | Citations resolve to the actual approved version | PARTIAL | Proven for the single-chunk case: `tests/rag_publication.rs::citations_carry_version_identity_and_never_mix_documents` (exact key resolves to one version; an ambiguous filename is rejected), `::superseded_cross_corpus_and_revalidation_are_exact`, and `src/answer/citation.rs` unit tests (same filename, different keys). **Unresolved P1 in PR #54 (Linux-owned):** when section expansion places several chunks of the *same* source, section, and page into the context — a normal multi-chunk section — the match count exceeds one and the citation is treated as ambiguous, so a valid citation can be stripped when validation is enabled or returned with an empty `sources` array when it is disabled. Citation resolution is therefore not universally complete until Linux deduplicates matches by publication/citation coordinates |
| 14 | Cross-branch access cannot reveal restricted information | PARTIAL | Person-scope PASS: `tests/branch_review.rs::proposals_are_scoped_to_the_person_branch_not_the_reviewer`, `web/e2e/branch-review-security.spec.ts`. Cross-branch relationship escalation is decision B and is not implemented |
| 15 | A user without family access cannot retrieve family content | PARTIAL | Family profiles require an **active reviewed account-person link**: unlinked accounts get a non-leaking 403 and deactivated accounts are refused, at the API and on the page (`web/e2e/family-profile.spec.ts`, `web/e2e/family-profile-ui.spec.ts`). This is stricter than the family tree, which remains readable by **any authenticated viewer** because there is no per-family membership model. The two audiences are intentionally different and are documented as such; per-family scoping is test F4 below |
| 16 | RAG responses cannot contain unpublished or revoked versions | PARTIAL | Documents: `tests/rag_publication.rs` (pending, rejected, unprocessed, admin-only, removed, superseded, and stale fingerprint/version content excluded from snapshot, vector, keyword, and section reads; a revocation committed during generation returns the refusal) and `tests/document_processing.rs` (a stale job writes nothing). Profiles: `tests/onboarding_profile_submission.rs::pending_content_never_reaches_family_or_rag`, and the family page never renders drafts or private values (`web/e2e/family-profile-ui.spec.ts`). **Limits:** a response already transmitted to a client cannot be recalled, so revocation is enforced at generation and retrieval time rather than retroactively; and profile/story indexing remains unimplemented — specified as test F5 below |

## Existing coverage summary

| Area | Tests |
| --- | --- |
| Profile revision lifecycle and audience | `tests/onboarding_profile_revisions.rs`, `tests/onboarding_profile_submission.rs` |
| Profile media privacy and withdrawal | `tests/onboarding_profile_media.rs` |
| Genealogy fact review and branch scope | `tests/branch_review.rs`, `tests/genealogy_schema.rs` |
| Document lifecycle and approved-only retrieval | `tests/document_lifecycle.rs` |
| Processing and stale-job refusal | `tests/document_processing.rs` |
| Family profile presentation | `tests/family_profile_presentation.rs`, `web/e2e/family-profile.spec.ts` |
| Owner privacy restriction (A2) | `tests/profile_restriction.rs`, `web/e2e/profile-privacy.spec.ts` |
| HTTP boundary | `acceptance/api_contract/tests/api_contract.rs`, `web/e2e/*` |

## Specifications for later slices

Each specification names the slice that should implement it. None of these may
be marked PASS until executed.

### F1 — Family-visible profile presentation — IMPLEMENTED (USR-006-A)

*Given* a person with one approved `family` value and one approved
`owner_private` value, *when* an authorized family member reads the person's
profile, *then* the response contains the family value and the owner-private
field is absent from the body, the headers, and any error text. *And* the same
request as the owner returns both.

Implemented by `GET /api/family/persons/{person_key}/profile` and proven by
`tests/family_profile_presentation.rs` plus `web/e2e/family-profile.spec.ts`.
The owner keeps both values through the unchanged `GET /api/profile`.

### F2 — Profile visibility downgrade — IMPLEMENTED (USR-006-P, decision A2)

*Given* an approved family-visible biography, *when* the owner restricts it,
*then* the family read stops returning it immediately, the family page no longer
renders it, the owner still reads it privately with `restricted_from_family`
set, and the historical approved revision is unchanged. A pending submission
created before the restriction is refused at approval, and only a new
independently approved family-visible revision created under the active
restriction generation restores presentation.

Ordering is the monotonic restriction `generation` recorded on the revision at
creation, compared for exact equality at approval. It is deliberately never
derived from `changed_at`/`created_at`, because `now()` is fixed at transaction
start and two overlapping transactions can record timestamps that do not reflect
commit order. The deterministic regression holds the person row so the
restriction waits for its lock, submits a family-visible revision through the
product path during that wait, releases the lock, and asserts the approval is
refused with `RestrictionConflict`, no `lifted` event is written, the restriction
stays active, and the family read returns nothing.

The recorded generation is immutable after insert: a `BEFORE UPDATE` guard in
migration `0022` rejects any change, including NULL to value, while ordinary
draft edits, submission, and reviewer transitions still succeed.

Proven by `tests/profile_restriction.rs` and `web/e2e/profile-privacy.spec.ts`.

### F3 — Citations never resolve to a superseded version (slice: indexing adapter)

*Given* document version 1 approved and indexed, *when* version 2 is approved
and indexed, *then* every citation returned by `/api/query` resolves to the
version-2 rows, and no citation resolves to a version-1 row.

**Implemented for documents** by FAM-RAG-01: `tests/rag_publication.rs` asserts
exact, version-bearing citation identity and that superseded and removed rows do
not resolve. See [`document-publication-integrity.md`](document-publication-integrity.md).

### F4 — Audience model (slice: presentation API)

*Given* an authenticated account with no link to any genealogy person, *when*
it requests family content, *then* the request succeeds only where the existing
authorization model permits it, and the response never includes owner-private
fields or media. If a per-family membership model is introduced, a non-member
account must be refused, not merely filtered.

### F5 — Profile/story publication (slice: indexing adapter)

*Given* an approved family-visible biography and no publication record, *when*
`/api/query` is asked a question it answers, *then* no draft, owner-private,
pending, rejected, or superseded text appears in the answer or in the sources.
*And* after the biography is made private or removed, a subsequent query in the
same process returns neither the text nor a citation.

## Presentation field policy (USR-006-B)

The family profile page presents an explicit allow-list of narrative fields:
`display_name`, `preferred_name`, `biography`, `my_story`, and
`family_memories`. It deliberately does **not** render `current_location`,
`contact_preferences`, `facebook_url`, `linkedin_url`, `snapchat`, or
`whatsapp`, even if an owner had marked such a value `family`-visible, because
the contract lists contact details and addresses as private by product rule and
the field-level policy is not settled. Those values remain available to the
owner through `GET /api/profile`; they are simply not part of this family
history page. Resolving the policy is recorded under the open decisions.

The page also ignores the API's `state` for its own emptiness decision: it
computes emptiness over the allow-listed fields only, so the presence of
excluded private fields is never revealed.

## Policy gates

The following are **not** implemented and must not be inferred from this
document:

1. **BR-004 cross-branch relationships** — options and a recommended default
   (hold and escalate) are recorded in the contract; activation needs owner
   approval.
2. **Head of Family validity** — a read-time validation and suspension proposal
   is recorded in the contract; no succession or automatic replacement is
   implemented.
3. **Immediate owner restriction of profile values** — **decided (A2) and
   implemented**; listed here only to record that the previous review-gated
   behaviour no longer applies.
4. **Indexing allow-list** — decision D; the default is "documents only".
