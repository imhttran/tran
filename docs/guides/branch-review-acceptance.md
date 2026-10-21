# Branch review acceptance matrix (USR-005D)

This matrix maps the sixteen branch-review security and end-to-end scenarios to
the evidence that proves them. Every row is executed against the integrated
implementation: the real Rust API, the real Next.js proxy, disposable
PostgreSQL databases, and synthetic identities, branches, people, and sources.
No real family record, credential, address, or document is used.

Run the suites with:

```sh
cargo test --locked                     # server-side scenarios
make web-check                         # typecheck and production build
make web-e2e                           # browser scenarios, real proxy + API
sop validate && sop review             # SOP gates
```

Browser scenarios need `scripts/web-e2e.sh`, which starts the synthetic OIDC
issuer, seeds the disposable genealogy database, and deletes only its own rows.

## Matrix

Status legend:

- **PASS** — the scenario is proven end to end by an executed test.
- **PARTIAL** — the part that exists is proven; a named policy or harness gap
  remains and is listed under _Gaps_ below.
- **NOT COVERED** — no evidence yet.

| # | Scenario | Evidence | Result |
| --- | --- | --- | --- |
| 1 | Administrator grants branch-review authority | `tests/onboarding_branch_grants.rs::grants_are_explicit_scoped_and_audited`; `web/e2e/branch-grants.spec.ts` (grant, list, revoke, re-grant) | PASS |
| 2 | Reviewer accesses only their granted branches | `tests/branch_review.rs::cross_branch_and_revoked_grants_are_denied`, `::proposals_are_scoped_to_the_person_branch_not_the_reviewer`; `web/e2e/branch-review.spec.ts` (branch filter refused); `branch-review-ui.spec.ts` (picker built from grants) | PASS |
| 3 | Reviewer approves an eligible source-backed correction | `tests/branch_review.rs::branch_reviewer_approves_an_in_scope_source_backed_proposal`; `web/e2e/branch-review.spec.ts`; `branch-review-ui.spec.ts` (approve through the console) | PASS |
| 4 | Reviewer rejects a proposal with a recorded reason | `tests/branch_review.rs::rejection_and_needs_changes_leave_the_verified_fact_unchanged`; `web/e2e/branch-review.spec.ts`; `branch-review-ui.spec.ts` (note required, fact unchanged) | PASS |
| 5 | Reviewer requests additional evidence | same Rust test (needs-changes path); `web/e2e/branch-review.spec.ts` | PASS |
| 6 | Reviewer cannot approve their own submission | `tests/branch_review.rs` self-review denial inside `branch_reviewer_approves_an_in_scope_source_backed_proposal`; `web/e2e/branch-review.spec.ts` (submitter decision 403); `branch-review-ui.spec.ts` | PASS |
| 7 | Reviewer cannot inspect another branch's restricted records | `tests/branch_review.rs::proposals_are_scoped_to_the_person_branch_not_the_reviewer`; `web/e2e/branch-review-security.spec.ts` (404 and no value in the body); `branch-review.spec.ts` (evidence 403) | PASS |
| 8 | Grant revocation immediately blocks subsequent access | `tests/branch_review.rs::cross_branch_and_revoked_grants_are_denied`; `web/e2e/branch-grants.spec.ts` (revoked link makes an active grant unusable); `branch-review.spec.ts` | PASS |
| 9 | Account deactivation blocks reviewer capabilities | `web/e2e/branch-review-security.spec.ts` (grant, approve, deactivate, then every read and write denied) | PASS |
| 10 | Account replacement does not inherit grants | `tests/branch_review.rs::linked_owner_and_replaced_account_cannot_use_a_grant`; `tests/onboarding_branch_grants.rs::account_replacement_does_not_inherit_grants` | PASS |
| 11 | Head of Family without an explicit grant has no reviewer access | `tests/branch_review.rs::head_of_family_without_a_grant_has_no_review_authority` (active appointment, still no grants, queue, detail, or decision) | PASS |
| 12 | Concurrent decisions cannot produce conflicting approved facts | `tests/branch_review.rs::concurrent_decisions_have_exactly_one_winner` (coordinated competing transactions, not sleeps); repeated decision is refused with 409 in `branch-review.spec.ts` | PASS |
| 13 | Disputed evidence cannot silently become verified | `tests/branch_review.rs::a_disputed_fact_is_not_verified_by_submission_alone` | PASS |
| 14 | Shared or cross-branch relationships require appropriate authority | `tests/branch_review.rs::ambiguous_branch_membership_blocks_proposals`, `::proposals_are_scoped_to_the_person_branch_not_the_reviewer` | **PARTIAL** — person-scope authority is proven; cross-branch relationship escalation does not exist yet (gap 1) |
| 15 | Administrator role and branch reviewer role remain independent | `web/e2e/branch-review-security.spec.ts` (reviewer denied `/api/admin/reviews`, `/api/admin/family-members`, grant administration, and admin navigation; admin without a grant sees no branch authority); `tests/branch_review.rs::a_grant_without_an_audited_appointment_confers_no_authority` | PASS |
| 16 | No private profile, owner-private media, or unapproved content leaks through UI, API, logs, or RAG | `tests/branch_review.rs::branch_review_reads_never_return_private_profile_values`, `::proposal_content_never_enters_the_rag_corpus`; `web/e2e/branch-review-security.spec.ts` (a seeded approved owner-private value is absent from the queue JSON, the detail JSON, and the rendered page; no `/api/profile` request; pending document submissions are review-only); `web/e2e/branch-review.spec.ts` | **PARTIAL** — UI, API, and RAG are proven with a real seeded value; log leakage is not asserted (gap 3) |

## How the guarantees are enforced

- **Authority is an appointment.** A grant is usable only when it is active,
  bound to the caller's exact verified identity fingerprint, still backed by the
  reviewed account-person link, and recorded by the append-only `granted` audit
  event. Every protected request re-evaluates that predicate inside its own
  transaction; there is no session-level cache to revoke.
- **Scoping is per person.** Queue, detail, evidence, and decision paths all
  require the person's single verified branch to be one the caller holds.
  Off-branch identifiers return the same not-found answer as an unknown id.
- **Independence is checked at decision time.** A submitter cannot decide their
  own proposal, and a caller linked to the person cannot decide it either, even
  while holding the branch grant.
- **Approval is the existing reviewed path.** Approval writes the proposed value
  through the assertion evidence, provenance, validation, review-event, and
  baseline workflow. Rejection and change requests leave the authoritative fact
  untouched, and a stale prior snapshot is refused.
- **Nothing private is read.** The module only ever selects genealogy
  assertions, source evidence, and the proposal ledger. Private profile values,
  contact fields, drafts, and owner-private media are never joined, so they
  cannot appear in a response or the DOM. The browser fixture stores a
  distinctive approved owner-private value (`current_location`) for the person
  under review, and the acceptance run asserts that exact value is absent from
  the queue JSON, the detail JSON, and the rendered page — and fails the run if
  the fixture was not seeded, so the negative assertion cannot pass vacuously.
  The browser page never requests `/api/profile`.
- **Nothing unapproved is published.** Proposals live only in the genealogy
  database. A proposed value is never written to the retrieval corpus, and a
  pending document submission writes no chunks and is invisible to family
  viewers.

## Verification record (one recorded run)

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | PASS |
| `cargo test --locked` | PASS — 527 passed, 0 failed, 0 ignored |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS |
| `make web-check` | PASS |
| `make web-e2e` (complete suite, disposable databases) | PASS — 106 tests, 0 failed |
| `sop validate` | PASS |
| `sop review` | see the pull request |
| `sop security scan` | NOT_APPLICABLE (the add-on is disabled for this project) |

The table above records one full local run against freshly created disposable
databases; run the commands at the top of this page to reproduce it. Counts move
when tests are added, so treat the commands and the assertions as the contract,
not the numbers.

`make web-e2e` also covers `/documents`: an administrator submits a document,
sees it in the review list, and a family viewer is refused both the page and
the listing. The historical `admin-accounts` failure (a `<details>` panel that
closed after a mutation) was fixed in the same run by opening the panel
deterministically instead of assuming it stayed open.

## Gaps and explicit limits

1. **Cross-branch relationships are not an approval workflow** (scenario 14 is
   PARTIAL). A fact that
   spans two people (for example a parent-child claim between branches) is
   stored on the reviewed person and validated by the existing genealogy rules.
   There is no escalation path that requires authority over both branches. The
   plan assigns that policy to BR-004 and USR-006; this implementation does not
   guess it, and no shared lineage fact is silently promoted.
2. **Head of Family remains a genealogy status.** An active appointment confers
   no application permission, and a lapsed appointment is not re-derived here.
   The appointment-validity recheck belongs to BR-004.
3. **Logs are content-free by construction, not by test** (scenario 16 is
   PARTIAL). The branch review handlers log an error variant and never a value,
   note, evidence locator, or storage key. There is no log-capture harness in
   this repository, so the property is reviewed by inspection rather than
   asserted.
4. **The security add-on is not available.** `sop security scan` reports
   `NOT_APPLICABLE` for this project; this matrix is the substitute evidence.
