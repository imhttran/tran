# Bulk onboard preview table — pull request report

Plan: `docs/plans/PLAN-TRAN-BULK-ONBOARD-TABLE.md`
Plan ID: `plan-tran-bulk-onboard-table`
Branch: `sop/tran-bulk-onboard-table`
Base: `main` (`438e524` at the time of implementation)

## What changed

### Server projection

- `src/onboarding/candidate.rs`
  - `CandidateRow` carries `display_name` and `contact_email_masked`.
  - The verified-contact query selects the latest `onboarding_recipient_contact`
    email and masks it with `mask_email`.
  - `mask_email` keeps the first local-part character when the local part has more
    than one character, replaces the rest with a fixed `•••`, and keeps the
    domain. A one-character local part becomes `•••@domain`.
  - Unit tests cover a normal address, a one-character local part, uppercase
    input, the fixed three-character mask, and no local-part leak.
- `src/onboarding/dry_run.rs`
  - `DryRunRow` carries the display name and the masked contact.
- `src/onboarding/mod.rs`
  - Exports `mask_email`.
- `src/api.rs`
  - `BulkPreviewRowBody` carries `display_name` and
    `contact: { type: "email", masked } | { type: "none" }`.
  - The full email never enters the response body.

### Web

- `web/src/lib/api.ts`
  - `BulkPreviewRow` adds `display_name` and an optional `contact` union.
- `web/src/app/admin/family-members/page.tsx`
  - Loads every preview page (200-row pages) until the summary total is reached.
  - Adds Person and Contact columns; the person key sits below the name.
  - Adds classification stat-card toggles, a "No contact" toggle, and an
    accent-insensitive search over name, key, and username.
  - Defaults to Eligible + Needs review; selecting none shows all rows.
  - Hides contact reason codes from the Warnings column.
  - Keeps "Select eligible" over every preparable row, including filtered rows.
  - Adds `data-person-key` to each preview row for stable browser selection.
- `web/src/app/globals.css`
  - Widens the bulk workspace to 1280px.
  - Adds stable desktop column widths, nowrap/ellipsis for the person key and
    username, warning chips, pressed stat-card styling, and a scoped disabled
    checkbox state.
  - Leaves the existing 560px card layout in place and resets the table layout
    below that breakpoint.

### Tests

- `tests/onboarding_admin_api.rs`
  - Seeds a disposable genealogy database with one person that has a verified
    `ada@example.test` contact and one without.
  - Asserts the display name, the masked contact, the no-contact state, and that
    the full address never appears in the preview projection.
- `src/api.rs` tests
  - Asserts the contact body serializes as `{"type":"email","masked":...}` and
    `{"type":"none"}`, never the full address.
- `web/e2e/bulk-onboarding.spec.ts`
  - Adds synthetic stubbed coverage for the default filter, stat-card toggles,
    "No contact", accent-insensitive search, Contact/Warnings columns, paging,
    and the mobile card layout.
  - Updates the existing real-API flow to enable the non-default classification
    toggles before asserting blocked fixture rows.

## Acceptance criteria and evidence

| Criterion | Evidence |
| --- | --- |
| Preview rows carry `display_name` and `contact` | `tests/onboarding_admin_api.rs::preview_rows_carry_the_name_and_only_a_masked_contact` PASS; `src/api.rs::bulk_contact_body_serializes_only_the_masked_email` PASS |
| Verified contact is masked; no contact is `none` | Same focused tests PASS |
| No unmasked email in the preview response | Focused test asserts the projection does not contain `ada@example.test`; serialization test asserts the same |
| Classification, reasons, and counts unchanged | `cargo test --locked` PASS; existing bulk integration and E2E flow PASS |
| Crate builds and unit tests pass | `sop validate`: BUILD, UNIT_TEST, LINT all PASS |
| Complete load past the 200-row limit | Stubbed two-page test (`the Warnings column hides contact codes and pages load later rows`) PASS; page waits for the full summary total |
| Default view shows Eligible + Needs review | Stubbed default-filter test PASS |
| Stat-card toggles change the visible rows | Stubbed stat-card test PASS |
| Accent-insensitive search | Stubbed search test PASS (NFD + combining-mark strip) |
| Contact codes absent from Warnings | Stubbed Warnings test PASS |
| Mobile labelled cards, no page overflow | Stubbed 390px card test PASS |
| TypeScript build | `make web-check` PASS (typecheck + production build) |
| Real browser suite | `make web-e2e`: **291 passed**, including every `bulk-onboarding.spec.ts` test. One unrelated RAG answer-wording test failed (see below); 5 dependent tests did not run |
| No merge | Not merged |

## Validation commands and results

- `sop validate` (disposable `rag_test` / `tran_genealogy_test`):
  - `cargo build --locked` — PASS
  - `cargo test --locked` — PASS
  - `cargo fmt --check` — PASS
  - `cargo clippy --locked --all-targets -- -D warnings` — PASS
  - Log: `.run/validation/bot-final.log`
- `make web-check` — PASS (typecheck + production build)
  - Log: `.run/validation/bot-final-web.log`
- `make web-e2e` (disposable databases, API on 8081, web on 3000) — **291 passed, 1 unrelated failure, 5 did not run**
  - Unrelated failure: `e2e/rag.spec.ts:156` — “no answer matching /page two|page 2/i cited /page 2/i after 3 attempts”. This is the RAG generation wording check, not the bulk preview.
  - All `e2e/bulk-onboarding.spec.ts` tests passed.
  - Log: `.run/web-e2e.log`
- `git diff --check` — PASS

## Privacy and data boundaries

- The preview privacy contract change was explicitly approved by the plan: an
  administrator may see the person display name and a server-masked verified
  email. The full email never leaves the server through this endpoint.
- No phone field exists in the archive; the preview does not invent one.
- No migration was added.
- Tests use synthetic names and `example.test` addresses only.
- No real family name, identifier, address, source excerpt, or private media is
  included in this report or the test fixtures.

## Limitations

- Email is the only stored contact channel. The UI shows `None` when no verified
  contact exists; it does not infer a channel from profile text.
- The preview loads all pages in sequence. The 100-page cap is a safety bound for
  very large catalogs; the current catalog is far below it.
- The full browser suite is not completely green because of the unrelated RAG
  answer-wording failure above.

## Status

Nothing in this report was merged. The branch is ready for review.
