# Family publication acceptance matrix

This matrix is the acceptance plan for the publication boundary defined in
[`docs/architecture/FAMILY-PUBLICATION-CONTRACT.md`](../architecture/FAMILY-PUBLICATION-CONTRACT.md).

The test names and files cited below were checked against this branch's tree;
run the commands in the next section to reproduce them.

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
| 5 | Approved family-visible values eligible for presentation | PASS | `tests/family_profile_presentation.rs` (service) and `web/e2e/family-profile.spec.ts` (HTTP through the proxy) exercise `GET /api/family/persons/{person_key}/profile`; see [family-profile-presentation.md](family-profile-presentation.md) |
| 6 | Private media excluded | PASS | `tests/onboarding_profile_media.rs::photo_is_private_until_approved_and_current`, `::document_is_private_until_approved_and_current`, `::cross_account_and_revoked_link_cannot_access_media` |
| 7 | Disputed genealogy facts excluded | PASS | `tests/branch_review.rs::a_disputed_fact_is_not_verified_by_submission_alone`; `src/genealogy/tree.rs` only marks verified facts as verified and returns citations only for verified edges. Publication of genealogy facts is out of scope (contract §4) |
| 8 | Superseded versions excluded | PASS | `tests/document_lifecycle.rs::lifecycle_is_additive_quarantined_and_approved_only` (approved version binding); `tests/document_processing.rs::processing_is_durable_idempotent_and_approved_only`; profile side: `tests/onboarding_profile_revisions.rs::approved_revision_number_is_part_of_the_conflict_check`; media side: `onboarding_profile_media.rs::replacement_owner_sees_only_current_approved_or_own_media` |
| 9 | Removal revokes presentation | PASS | `tests/document_lifecycle.rs::lifecycle_is_additive_quarantined_and_approved_only` (remove → retrieval empty); `tests/onboarding_profile_media.rs::owner_can_withdraw_pending_media_and_reupload` |
| 10 | Visibility changes revoke publication | PARTIAL | Documents PASS: `tests/document_lifecycle.rs::lifecycle_is_additive_quarantined_and_approved_only` (scope change resets to pending review). Profile values: a change requires a new approved revision, so the previous value stays presentable until that commits — decision A in the contract; specified as test F2 below |
| 11 | Account/person removal prevents unauthorized visibility | PASS | `tests/onboarding_profile_media.rs::cross_account_and_revoked_link_cannot_access_media`; `tests/onboarding_profile_submission.rs::revoked_link_blocks_further_owner_writes`; `tests/branch_review.rs::linked_owner_and_replaced_account_cannot_use_a_grant` |
| 12 | Stale indexing cannot republish revoked content | PASS | `tests/document_processing.rs::processing_is_durable_idempotent_and_approved_only` — an approval withdrawn after queueing makes the job fail and writes no chunks |
| 13 | Citations resolve to the actual approved version | PARTIAL | Retrieval reads only `approved_documents`, so citations necessarily come from approved chunks. No test asserts the negative case explicitly — specified as test F3 below |
| 14 | Cross-branch access cannot reveal restricted information | PARTIAL | Person-scope PASS: `tests/branch_review.rs::proposals_are_scoped_to_the_person_branch_not_the_reviewer`, `web/e2e/branch-review-security.spec.ts`. Cross-branch relationship escalation is decision B and is not implemented |
| 15 | A user without family access cannot retrieve family content | PARTIAL | Profile presentation now requires an active reviewed account-person link: unlinked accounts get 403 and deactivated accounts are refused (`web/e2e/family-profile.spec.ts`). The shared family tree remains readable by any authenticated viewer, and there is still no per-family membership model — specified as test F4 below |
| 16 | RAG responses cannot contain unpublished or revoked versions | PASS for existing content; NOT IMPLEMENTED for profiles/stories | Documents: `tests/document_lifecycle.rs::lifecycle_is_additive_quarantined_and_approved_only` (pending and legacy content absent from retrieval; approved present; removed absent) and `document_processing.rs` (stale job writes nothing). Profiles: `tests/onboarding_profile_submission.rs::pending_content_never_reaches_family_or_rag` (profile content creates no retrieval rows). There is no profile/story publication path to test yet — specified as test F5 below |

## Existing coverage summary

| Area | Tests |
| --- | --- |
| Profile revision lifecycle and audience | `tests/onboarding_profile_revisions.rs`, `tests/onboarding_profile_submission.rs` |
| Profile media privacy and withdrawal | `tests/onboarding_profile_media.rs` |
| Genealogy fact review and branch scope | `tests/branch_review.rs`, `tests/genealogy_schema.rs` |
| Document lifecycle and approved-only retrieval | `tests/document_lifecycle.rs` |
| Processing and stale-job refusal | `tests/document_processing.rs` |
| Family profile presentation | `tests/family_profile_presentation.rs`, `web/e2e/family-profile.spec.ts` |
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

### F2 — Profile visibility downgrade (slice: presentation API, after decision A)

*Given* an approved family-visible biography, *when* the owner requests
owner-private visibility, *then* — if decision A2 is chosen — the family read
stops returning it in the same transaction and any published identity is
removed; if A1 is chosen, the family read continues to return it until the new
revision is approved, and the response states that a change is pending.

### F3 — Citations never resolve to a superseded version (slice: indexing adapter)

*Given* document version 1 approved and indexed, *when* version 2 is approved
and indexed, *then* every citation returned by `/api/query` resolves to the
version-2 rows, and no citation resolves to a version-1 row.

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

## Policy gates

The following are **not** implemented and must not be inferred from this
document:

1. **BR-004 cross-branch relationships** — options and a recommended default
   (hold and escalate) are recorded in the contract; activation needs owner
   approval.
2. **Head of Family validity** — a read-time validation and suspension proposal
   is recorded in the contract; no succession or automatic replacement is
   implemented.
3. **Immediate owner restriction of profile values** — decision A; the current
   behavior is review-gated.
4. **Indexing allow-list** — decision D; the default is "documents only".
