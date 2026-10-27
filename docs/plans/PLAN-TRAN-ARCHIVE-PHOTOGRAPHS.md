# Implementation Plan

## Project

Tran genealogy — photographs from the original family PDFs

## Summary

Extend the archive so a reviewed family PDF contributes its photographs, not only
its text. Photographs are inventoried page by page, extracted from the PDF's own
embedded images where possible, stored privately with source provenance and a
content hash, linked to people only from explicit captions or an administrator's
confirmation, and published only after review.

### How execution works in this repository

SOP's command policy classifies `cargo` and `psql` as `REQUIRES_APPROVAL`, and
that class cannot be loosened by project configuration. The agent therefore
cannot run a build, a test, or a migration, and must not spend its window trying
to. The SOP validation stage runs the configured commands (`cargo build`,
`cargo test`, `cargo fmt --check`, `cargo clippy --all-targets`, and the
integration command) after the edit, supplies the runtime evidence, and drives the
FIX cycle when they fail.

Consequences for every task below:

- Read only the files the task names. Do not re-read a file already read in the
  same task.
- Write the code and the report; do not attempt to compile, test, lint, migrate, or
  query a database.
- State in the closing message what was changed and what the validation stage must
  prove. Do not claim a command was run.
- Acceptance criteria describe **outcomes the validation stage verifies**. They are
  not instructions to run a command.

### Boundaries that apply to every task

### Boundaries that apply to every task

- **Migration number.** AP-101 shipped
  `genealogy/migrations/0028_archive_photograph_inventory.sql`; AP-102 added the
  asset/byte-store columns in `0031_archive_photograph_assets.sql`. The
  layout-aware proposal provenance lives in
  `genealogy/migrations/0032_archive_photograph_link_provenance.sql`. `0027` is
  reserved by the narrative branch (`0027_source_narrative.sql`, now on main).
  Do not renumber or modify `0027`, `0028`, or `0031`.
- **Private by default.** Extracted assets and detailed inventories are family
  data. They stay outside Git, PR text, and CI artifacts. `/data/` is already
  ignored; keep extracted files under that root (or a configurable private root)
  and never stage them.
- **Synthetic fixtures only.** Committed tests generate their images and their PDF
  at runtime. `fixtures/parity/inputs/` holds only text-layer PDFs, so a photograph
  fixture is built in the test with `lopdf` rather than added as a binary. No real
  family photograph, caption, name, or hash enters the repository.
- **Isolated databases only.** Tests use the disposable helper in
  `tests/support/genealogy_isolation.rs`. Never point a test at the operator's
  `tran_genealogy` database.
- **No publication, no bulk approval.** Imported photographs start unpublished.
  No task approves, publishes, or links a real photograph.
- **Bounded context.** The SOP fix prompt carries the task diff, so a task that
  touches many large files overflows the model window. Name the exact files a
  task may change, keep the diff under roughly 50 KB, and never add vendored
  assets or generated media. The vendored agent tooling under `.agents/` is now
  git-ignored for this reason and must stay out of every diff.
- **Reports on disk, summaries in context.** Write evidence to the named
  `docs/reports/` file and return a short summary. Never paste file contents,
  logs, or the diff into the response; a later task reads the summary, not the
  transcript.
- **Identity rules.** A person link may come only from an explicit caption in the
  document, the section heading or surrounding biography that identifies the
  photo's subject, or an administrator's explicit confirmation. A group photo may
  propose several people only when the source caption identifies those people; a
  nearby or biographical name never identifies everyone in the photo. Never use
  face recognition, and never merge namesakes. A proposal is not approval: proposed
  links stay separate from confirmed links, and a reviewer makes the final choice.

## AP-101 — Inventory photographs by PDF page

Implement a deterministic inventory over every page of one document version, and
persist it.

Files to read, at most once each: `src/genealogy/migrations`-style migration
`genealogy/migrations/0028_archive_photograph_inventory.sql` (already written; read
it and extend it), `src/genealogy/mod.rs`, `src/genealogy/migrate.rs`,
`src/loader/pdf.rs`, `src/document_lifecycle.rs`, `Cargo.toml`.

Then create `src/genealogy/inventory.rs` and declare it in
`src/genealogy/mod.rs`. The module must:

- take a document version's original bytes and walk every page with `lopdf`,
  returning one page record per page, 1-based, including pages with no image and
  pages that could not be read (with the exact error text);
- record every image object a page draws: object number and generation, pixel
  width and height, colour space, filters, whether the stream decodes to a
  complete image, placement when the content operators resolve it, and whether it
  is tinted or repeated;
- classify each image object as `photograph`, `decorative`, `watermark`,
  `diagram`, or `unknown` with a named rule and the property values that fired it.
  When the signals do not decide, classify `unknown` and record why. Never guess;
- keep an optional caption only when the document states one;
- write the rows idempotently: a second inventory of the same version and content
  hash is a no-op, and a replaced original cannot inherit an old inventory.

Also write `docs/reports/archive-photograph-inventory.md` describing the method,
the classification signals, and the failures the inventory can record. The report
contains no image bytes and no real family data.

### Dependencies

None

### Deliverables

- `src/genealogy/inventory.rs` and its declaration in `src/genealogy/mod.rs`.
- Any extension to `genealogy/migrations/0028_archive_photograph_inventory.sql` the
  writer needs.
- `docs/reports/archive-photograph-inventory.md`.
- Unit tests in the module for the classifier and the page accounting, using
  in-memory bytes; no database and no file writes.

### Acceptance Criteria

- Every page of a test PDF appears exactly once in the returned inventory,
  including pages with no image and pages that failed to read.
- Each classified object carries a named rule and the values that fired it, and an
  undecidable object is `unknown` with a reason, never a guess.
- The module compiles under the SOP validation stage and its unit tests pass.
- The migration remains additive and a second run applies zero files.
- No image bytes appear in the module, the report, or test output.

## AP-102 — Asset schema, hashing, and private storage

Extend `genealogy/migrations/0028_archive_photograph_inventory.sql` with the asset
and byte-store columns the later tasks need:

- a content hash on the asset, unique per asset, plus mime type, byte size, pixel
  dimensions, and a relative storage key;
- an occurrence row that keeps `(asset, document version, page, ordinal)`;
- the publication state, defaulting to unpublished;
- append-only review events.

Create `src/genealogy/photograph_store.rs` for the private byte store: a
configurable root (default under the already-ignored `data/`), a content-hash path
scheme, `put` returning the existing key when the bytes are unchanged, and a
`read` that returns bytes only for a known key. It stores no path outside the root
and never writes into the repository tree.

Files you may change: `genealogy/migrations/0028_archive_photograph_inventory.sql`,
`src/genealogy/photograph_store.rs` (new), `src/genealogy/mod.rs`.
Do not change `src/genealogy/inventory.rs`.

### Dependencies

- AP-101

### Deliverables

- The migration extensions.
- `src/genealogy/photograph_store.rs` with unit tests for hashing, deduplication,
  and a second `put` of identical bytes.

### Acceptance Criteria

- Two identical byte sequences produce one asset key; different bytes produce two.
- The store writes only under its root and returns no absolute path.
- The migration stays additive and applies on a clean disposable database.
- The validation stage reports the new unit tests passing.

## AP-103 — Extract embedded images from the inventory

Add `src/genealogy/photograph_extract.rs`. It takes the original bytes plus the
`ImageObject` records from `inventory::inventory_bytes`, and for each object whose
classification is `photograph` returns the storable bytes:

- use the embedded stream as-is when the filter chain is a complete JPEG or PNG;
- otherwise decode with the `image` crate and re-encode to PNG;
- return an explicit reason when an object cannot be decoded;
- never repack or rewrite the source PDF.

`page_crop` stays unimplemented: record `crop_unavailable` with the reason. Do not
add a rasterizer dependency.

Files you may change: `src/genealogy/photograph_extract.rs` (new),
`src/genealogy/mod.rs`. Read `src/genealogy/inventory.rs` once; do not edit it.

### Dependencies

- AP-102

### Deliverables

- `src/genealogy/photograph_extract.rs` with unit tests over in-memory synthetic
  image bytes, including a decode failure and an unsupported filter.

### Acceptance Criteria

- A complete embedded JPEG or PNG is returned unchanged, byte for byte.
- An undecodable object yields a recorded failure, not a panic and not a
  placeholder.
- No source PDF byte is modified.
- The validation stage reports the new unit tests passing.

## AP-104 — Idempotent photograph import

Add `src/genealogy/photograph_import.rs` that composes inventory, extraction,
storage, and the schema into one operation:

`import_photographs(version_id, corpus, content_sha256, bytes, actor)`.

It writes one asset per content hash, one occurrence per page, and an inventory
summary; a second call for the same version and hash writes nothing and preserves
every existing review decision. A new content hash belongs to a replaced original
and gets its own rows.

Files you may change: `src/genealogy/photograph_import.rs` (new),
`src/genealogy/mod.rs`.

### Dependencies

- AP-103

### Deliverables

- `src/genealogy/photograph_import.rs`.
- An integration test in `tests/photograph_import.rs` using the disposable
  database helper, with a synthetic PDF generated in the test.

### Acceptance Criteria

- The first import creates assets and occurrences; the second creates none and
  reports them as unchanged.
- A replaced original does not inherit the previous review state.
- The test creates and drops only its own disposable database.

### Acceptance Checks

- `cargo test --locked`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1

## AP-105 — Person links from explicit captions

Implement link proposals and publication state in
`src/genealogy/photograph_review.rs`:

- a new photograph is `unpublished`;
- a link is proposed only from an explicit caption naming the person, reusing
  `person_search` folding, alias, and name-order matching;
- an exact or alias match may be proposed; several candidates or a weak match stay
  `ambiguous` and require a reviewer choice; a caption with no name yields no link;
- confirm, reject, edit-caption, and set-visibility operations, each appending a
  review event with the reviewer, the time, and the prior value.

Files you may change: `src/genealogy/photograph_review.rs` (new),
`src/genealogy/mod.rs`, `genealogy/migrations/0028_archive_photograph_inventory.sql`.

### Dependencies

- AP-104

### Deliverables

- `src/genealogy/photograph_review.rs`.
- Tests for single-person, group, ambiguous-namesake, and unnamed captions.

### Acceptance Criteria

- An unnamed caption produces no link; no link derives from page position,
  ordering, or a facial comparison.
- Two people with the same recorded name require an explicit choice.
- Repeating a decision is a no-op and adds no duplicate event.

### Acceptance Checks

- `cargo test --locked`

### Acceptance Mapping

- criterion 1: check 1
- criterion 2: check 1
- criterion 3: check 1

## AP-106 — Authorized image routes, gallery, and profile API

Add the read surfaces to `src/api.rs`:

- a gallery listing approved photographs, including those without a link;
- thumbnails and originals through the API only, never a storage path;
- an archive-photograph section on the family profile read, captioned with the PDF
  page citation and kept separate from `profile_media`;
- `Operation::GenealogyRead` for family reads and the document-review operation for
  administrative reads; unpublished or restricted photographs return not-found for
  both thumbnail and original.

Operator update (2026-10-27): the administrator review path must also propose
people from the PDF layout and nearby text, not only from an explicit caption.
The proposal engine lives in `src/genealogy/photograph_context.rs` and uses:

- an explicit caption, a section heading, text above or below the image, a unique
  biography subject, or a biography continued on an adjacent page;
- dates and recorded relationships to distinguish namesakes;
- a group caption only when the source identifies each named person;
- a bounded source page, supporting text, confidence level, confidence basis,
  ambiguity note, and strength order stored on each proposed link;
- proposed links refreshed without touching confirmed or rejected links.

The caption-only engine in `photograph_review` remains the pure fallback used by
AP-105's tests. The review path uses the layout engine.

Files you may change: `src/api.rs`, `src/onboarding/family_profile.rs`,
`src/genealogy/photograph_review.rs`, `src/genealogy/photograph_context.rs` (new),
`src/genealogy/mod.rs`,
`genealogy/migrations/0032_archive_photograph_link_provenance.sql` (new), and
`tests/photograph_acceptance.rs`.

### Dependencies

- AP-105

### Deliverables

- The routes and handlers.
- Integration tests for allowed and refused access on thumbnail and original.
- `src/genealogy/photograph_context.rs` with layout extraction and the
  strongest-first proposal engine.
- `genealogy/migrations/0032_archive_photograph_link_provenance.sql` with the
  proposal provenance columns and review ordering index.
- `src/genealogy/photograph_review.rs` persistence that refreshes only
  `proposed` links and preserves confirmed or rejected decisions.
- `tests/photograph_acceptance.rs` coverage for layout proposals, namesake
  ordering, adjacent pages, and approved-link preservation.

### Acceptance Criteria

- A family viewer cannot list or fetch a pending, restricted, or unpublished
  photograph.
- An approved photograph without a link appears in the gallery.
- No response carries a storage key, filesystem path, or credential.
- A layout proposal stores its source page, supporting text, confidence, basis,
  and strength order; the strongest candidate is first.
- A confirmed or rejected link is never overwritten by a later proposal run.

## AP-107 — Web surfaces

Add the gallery page, the profile section, and the administrator review screen.

Files you may change: `web/src/lib/api.ts`, `web/src/app/gallery/page.tsx` (new),
`web/src/app/admin/photographs/page.tsx` (new),
`web/src/components/FamilyProfileView.tsx`, `web/src/components/ExperienceShell.tsx`,
`web/e2e/photographs.spec.ts` (new).

### Dependencies

- AP-106

### Deliverables

- The three surfaces and their types.
- Playwright coverage with stubbed API responses.

### Acceptance Criteria

- The review screen exposes caption, link, and visibility controls.
- The profile section is visually and structurally separate from uploaded photos.
- The review screen shows confidence, source page, supporting text, and an
  ambiguity explanation, strongest candidate first.
- The e2e spec passes against stubbed responses and leaks no path or credential.

## AP-108 — Acceptance verification

Prove the acceptance cases with synthetic fixtures on a disposable database.

Files you may change: `tests/photograph_acceptance.rs` (new) and
`tests/support/genealogy_isolation.rs` only if a helper is genuinely missing.

Cover: a single-person photograph, a group photograph, an unidentified photograph,
a pending photograph invisible to a family viewer, a restricted photograph
invisible for thumbnail and original, a repeated import with no duplicates, every
page accounted for, and an extraction failure recorded. Also cover layout-aware
proposals: an explicit caption, a group caption, a namesake distinguished by a
stated year, an adjacent-page biography fallback, and preservation of a confirmed
link across a repeated proposal run.

### Dependencies

- AP-107

### Deliverables

- `tests/photograph_acceptance.rs`.

### Acceptance Criteria

- Each acceptance case has a test that fails if the behaviour regresses.
- The suite creates and drops only its own disposable database.
- No real photograph, caption, or name appears in any committed file.

## AP-109 — Prepare the pull request

Write the pull-request description at
`docs/reports/archive-photograph-pr.md` from the validation evidence already on
disk: design, why `0028` (and that `0027` belongs to the narrative branch), the
authorization model, the verification results, and the limitations. Use `NOT RUN`
for anything the evidence does not contain. State that no real photograph was
published, no operator database was migrated, and nothing was bulk-approved.

### Dependencies

- AP-108

### Deliverables

- `docs/reports/archive-photograph-pr.md`.

### Acceptance Criteria

- Every acceptance case from AP-108 is listed with its test.
- No real family image, caption, credential, or storage path appears.
- Nothing is merged.
