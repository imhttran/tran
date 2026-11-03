# Implementation Plan

## Project

Tran genealogy — Bulk onboard preview table: names, contact, filters, width

## Summary

The Bulk onboard preview lists every catalog person (281 today) in a table
squeezed into the 880px app shell. Admins see only an opaque person key, cannot
tell who can be reached, and must scroll past 267 not-eligible rows to find the
11 that need action. The page also requests at most 200 rows, so part of the
catalog never appears at all.

This plan adds the person's name and a masked contact to each preview row, loads
the complete preview, adds status / contact / text filters that default to the
actionable rows, and gives the page a wider layout with stable column widths.

Approved decision: the admin-only preview row may now carry the person's
display name and a server-masked verified email. This replaces the previous
"opaque key and reason codes only" contract for this endpoint. The full email
address never leaves the server through the preview.

### How execution works in this repository

SOP's command policy classifies `cargo` and `psql` as `REQUIRES_APPROVAL`, and
that class cannot be loosened by project configuration. The agent therefore
cannot run a build, a test, or a migration, and must not spend its window trying
to. The SOP validation stage runs the configured commands after the edit,
supplies the runtime evidence, and drives the FIX cycle when they fail.

Consequences for every task below:

- Read only the files the task names. Do not re-read a file already read in the
  same task.
- Write the code and the report; do not attempt to compile, test, lint, migrate,
  or query a database.
- State in the closing message what was changed and what the validation stage
  must prove. Do not claim a command was run.
- Acceptance criteria describe outcomes the validation stage verifies.

### Boundaries that apply to every task

- **No migration.** The data already exists: `person.display_name` and
  `onboarding_recipient_contact.email` (verified by construction). Do not add or
  change a migration.
- **Email only.** The only stored contact channel is a verified email. Do not
  invent phone or other contact types. A person without a row in
  `onboarding_recipient_contact` has contact `none`.
- **Mask on the server.** The preview carries a masked address only, for example
  `t•••@gmail.com` (first character of the local part, a fixed `•••`, then `@`
  and the full domain). A one-character local part masks to `•••@domain`. The
  unmasked email must not appear in the preview response, logs, or test output.
- **Classification unchanged.** Eligibility, reasons, preparable, sendable, and
  the prepare / send flows keep their current behaviour.
- **Synthetic fixtures only.** Tests use made-up names and `example.test`
  addresses. No real family name or address enters a committed file.
- **Isolated databases only.** Integration tests use
  `tests/support/genealogy_isolation.rs`, never the operator database.
- **Bounded context.** Change only the files each task names and keep each
  task's diff small. `.agents/` stays out of every diff.

## BOT-101 — Name and masked contact in the preview row

Extend the server-side preview so each row carries the person's display name and
a masked verified email.

Files you may change: `src/onboarding/candidate.rs`, `src/onboarding/dry_run.rs`,
`src/api.rs` (the `BulkPreviewRowBody` struct and the `bulk_preview` mapping
only).

- In `candidate.rs`, add `display_name: String` and
  `contact_email_masked: Option<String>` to `CandidateRow`. The display name is
  already read by the evidence query; carry it through. Replace the
  `SELECT EXISTS (...)` contact query with one that selects the verified email,
  derive `missing_verified_contact` from its absence, and mask it with a new
  pure function `mask_email(&str) -> String`.
- In `dry_run.rs`, carry both fields through `DryRunRow` and `build_row`, and
  update the doc comment that says the row carries the key and codes only.
- In `api.rs`, add `display_name` and `contact` to `BulkPreviewRowBody`, where
  `contact` is `{ "type": "email", "masked": "t•••@gmail.com" }` or
  `{ "type": "none" }`.

### Dependencies

None

### Deliverables

- The struct and query changes above.
- Unit tests for `mask_email`: normal address, one-character local part,
  uppercase input, and that the output never contains the original local part
  beyond its first character.
- Updated unit-test fixtures in `candidate.rs` and `dry_run.rs` so they compile
  with the new fields.

### Acceptance Criteria

- Each preview row in the JSON response has `display_name` and `contact`.
- A person with a verified contact has `contact.type == "email"` and a masked
  address; a person without one has `contact.type == "none"`.
- No unmasked email appears in the preview response.
- Classification, reasons, and counts are identical to before for the same data.
- The crate compiles and the unit tests pass under the SOP validation stage.

## BOT-102 — Admin API coverage for the new fields

Prove the new fields end to end against a disposable database.

Files you may change: `tests/onboarding_admin_api.rs`.

### Dependencies

- BOT-101

### Deliverables

- A test that seeds one person with a verified contact (`ada@example.test`) and
  one without, calls the preview endpoint as an admin, and asserts the name,
  `contact.type`, and the masked value, and that the string `ada@example.test`
  does not occur anywhere in the response body.
- A test that a non-admin still receives the existing denial.

### Acceptance Criteria

- Both tests fail if the field is removed or the email is returned unmasked.
- The suite uses only its own disposable database.

## BOT-103 — Complete load, new columns, and filters on the page

Update the web types and the Bulk onboard page.

Files you may change: `web/src/lib/api.ts`,
`web/src/app/admin/family-members/page.tsx`.

- Types: add `display_name: string` and
  `contact: { type: "email"; masked: string } | { type: "none" }` to
  `BulkPreviewRow`.
- Complete load: `load()` currently requests `limit: 200` once. Page through the
  preview with `offset` until the number of rows equals the sum of the summary
  counts (or a page returns fewer rows than the limit), so all people appear.
- Columns, in order: Select, Person (display name in bold; the person key below
  it in smaller muted text), Contact ("Email" plus the masked address, or
  "None"), Status, Proposed username, Warnings.
- Warnings: show the reason codes as small chips. Do not show
  `missing_verified_contact` or `blocked_missing_recipient` there, because the
  Contact column now carries that information. Keep the "recipient missing"
  hint out of the Status cell for the same reason.
- Filters, all client-side over the loaded rows:
  - The summary stat cards become toggle buttons (`aria-pressed`) that filter by
    classification. The default after a preview is Eligible and Needs review
    selected. Selecting none shows all rows.
  - A "No contact" toggle that keeps only rows with `contact.type == "none"`.
  - A search box matching display name, person key, and proposed username,
    case-insensitive and accent-insensitive (normalize with NFD and strip
    combining marks, so "tran" matches "Trần").
  - A line above the table: "Showing N of M people".
- "Select eligible" keeps selecting every preparable row, including rows hidden
  by a filter, and the prepare confirmation count is unchanged.
- Rows that are not preparable keep the disabled checkbox.
- Every `data-label` matches its column header so the mobile card layout keeps
  working.

### Dependencies

- BOT-101

### Deliverables

- The type, load, table, and filter changes above.

### Acceptance Criteria

- With 281 people in the summary, the page holds 281 rows after a preview.
- The default view shows only eligible and needs-review rows.
- Clicking a stat card toggles its classification in the filter.
- Search for an unaccented name finds the accented row.
- Contact-related codes no longer appear in the Warnings column.
- The TypeScript build passes.

## BOT-104 — Wider page and stable column widths

Give the Bulk onboard page room and keep columns from wrapping awkwardly.

Files you may change: `web/src/app/globals.css` (the bulk onboarding and
`fm-` sections only), and `web/src/app/admin/family-members/page.tsx` only to add
a `<colgroup>` or class names.

- Add `.app-shell:has(.fm-workspace) { max-width: 1280px; }`, matching the review
  workspace.
- Column widths on desktop: Select 48px, Person about 24%, Contact about 18%,
  Status about 14%, Proposed username about 14%, Warnings the remainder.
- Keep the person key and proposed username on one line (`white-space: nowrap`)
  with ellipsis on overflow; let Warnings chips wrap.
- Style the stat-card toggles with a clear pressed state, the filter row, and
  the warning chips using existing tokens (`--line`, `--card`, `--accent`,
  `--accent-soft`, `--muted`).
- Make disabled checkboxes visibly disabled (reduced opacity, not-allowed
  cursor).
- Do not change the existing max-width 560px card layout except where the new
  Person and Contact cells need it.

### Dependencies

- BOT-103

### Deliverables

- The CSS changes above.

### Acceptance Criteria

- At 1440px viewport width the table is wider than 1100px and no person key
  wraps onto two lines.
- At 390px viewport width the rows render as labelled cards with no horizontal
  page scroll.
- No selector outside the bulk onboarding and `fm-` sections changes.

## BOT-105 — End-to-end coverage

Update the Playwright spec for the new columns and filters.

Files you may change: `web/e2e/bulk-onboarding.spec.ts`.

### Dependencies

- BOT-104

### Deliverables

- Stubbed preview responses that include `display_name` and `contact`, with more
  rows than one page so the paging loop is exercised.
- Tests for: default filter shows only eligible and needs-review rows; clicking a
  stat card changes the visible rows; "No contact" filter; accent-insensitive
  search; Contact column shows masked email or "None"; Warnings omits the
  contact codes; mobile viewport renders labelled cards.

### Acceptance Criteria

- The spec passes against stubbed responses.
- No real family name or address appears in the spec.

## BOT-106 — Prepare the pull request

Write `docs/reports/bulk-onboard-table-pr.md` from the validation evidence on
disk: what changed, the approved change to the preview privacy contract and the
server-side masking, the fix for rows past the 200 limit, the verification
results, and limitations (email is the only contact channel). Use `NOT RUN` for
anything the evidence does not contain.

### Dependencies

- BOT-105

### Deliverables

- `docs/reports/bulk-onboard-table-pr.md`.

### Acceptance Criteria

- Every acceptance criterion above is listed with its evidence or `NOT RUN`.
- No real name, email, or credential appears.
- Nothing is merged.
