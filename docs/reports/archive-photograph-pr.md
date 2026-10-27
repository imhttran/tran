# Archive photographs — pull request evidence (AP-106–AP-109)

This document is the pull-request description prepared from the evidence on
disk. It contains no real family photograph, caption, name, hash, credential,
database URL, or storage path. All fixtures are synthetic and generated in the
test at runtime.

## Scope

AP-106 through AP-109 complete the archive-photograph plan:

- authorized family gallery and profile reads, with thumbnail and original
  routes that never expose a storage key;
- an administrator review surface for caption, person-link, and visibility
  decisions;
- layout-aware person proposals using the PDF caption, section heading, nearby
  text, surrounding biography, and adjacent pages;
- acceptance verification and this evidence document.

The operator update of 2026-10-27 extended the original caption-only proposal
rule: the review path now also uses PDF layout and nearby text. The caption-only
engine in `src/genealogy/photograph_review.rs` remains as the pure fallback and
keeps its AP-105 tests. A proposal is still not approval; confirmed links are
never overwritten by a later proposal run.

## Design

### Inventory, extraction, import

AP-101 inventories every page and every drawn image object with `lopdf`:
page number, readability and failure text, object number and generation, pixel
dimensions, colour space, filters, completeness, placement, tint, repetition,
classification rule, and a caption only when the document states one.

AP-102 adds the private content-addressed byte store under the already-ignored
`data/` root. AP-103 extracts an embedded JPEG or PNG unchanged, or re-encodes a
decodable stream to PNG; an unsupported filter or decode failure is recorded
with a reason. AP-104 imports one asset per content hash and one occurrence per
page draw, idempotently.

### Layout-aware proposals (AP-106 extension)

`src/genealogy/photograph_context.rs` reads the PDF text layout from content
operators (`Tm`, `Td`, `TD`, `T*`, `Tj`, `TJ`) and groups text by baseline. It
then proposes people using, in descending strength:

1. an explicit caption (or a caption-like line within the image's text band);
2. an explicit group caption that identifies each named person;
3. a section heading that names the biography subject;
4. short text immediately above or below the image;
5. a biography page that names exactly one recorded person;
6. a biography continued on the previous or next page (low confidence only).

A nearby name never links every person on the page. A biography that mentions
several relatives without a caption that identifies them yields no link. A group
caption may propose several people only when the source names them together.

Each proposal records:

- `source_page` — the PDF page holding the supporting text;
- `supporting_text` — a bounded excerpt;
- `confidence` — `high`, `medium`, or `low`;
- `confidence_basis` — the source signal plus any date or relationship signal;
- `ambiguity_note` — why a namesake needs an explicit reviewer choice;
- `match_kind` — `exact`, `alias`, `reordered`, or `partial`;
- `candidate_rank` — strongest first;
- `proposal_origin` — `caption`, `layout`, or `manual`.

Namesakes are separated by the same folding, alias, and name-order rules used by
`person_search`. A four-digit year in the source that matches a recorded birth or
death date, or a named recorded parent or spouse, distinguishes the stronger
candidate; a tie stays ambiguous and is shown to the reviewer. Face recognition
is not used.

### Proposal persistence and approval

`archive_photograph_person_link` keeps `proposed`, `confirmed`, and `rejected`
states. A layout proposal run:

- deletes only `proposed` rows that are no longer supported;
- updates only `proposed` rows for candidates that remain supported;
- never updates a `confirmed` or `rejected` row;
- appends one append-only `reviewed` event with operation
  `layout_proposals`;
- returns the candidates strongest first and reports how many existing
  decisions were preserved.

The reviewer confirms or rejects one link at a time through the existing
`confirm_db` / `reject_db` workflow. Confirming one namesake does not merge it
with another; rejection stays separate.

### Read surfaces and authorization

Family reads:

- `GET /api/family/photographs` — published-only gallery, including a
  photograph with no confirmed person link;
- `GET /api/family/photographs/{id}/thumbnail` and `/original` — the same
  publication and membership predicate;
- `GET /api/family/persons/{person_key}/profile` — the
  `archive_photographs` section, separate from `profile_media`, with the PDF
  page citation.

They use `Operation::GenealogyRead` and additionally require an active family
link in the service. An unpublished, reviewer-restricted, or unknown photograph
is `404` for both thumbnail and original. An unlinked caller is `403`. The
responses carry metadata and API URLs only: no storage key, filesystem path,
content hash, or credential.

Administrator reads and writes use `Operation::DocumentReview`:

- `GET /api/admin/photographs` — every publication state, with provenance and
  ordered links;
- `GET /api/admin/photographs/{id}/thumbnail` and `/original`;
- `POST /api/admin/photographs/{id}/caption`;
- `POST /api/admin/photographs/{id}/visibility`;
- `POST /api/admin/photographs/{id}/propose`;
- `POST /api/admin/photographs/{id}/links/{person_id}`.

The propose route reads the original PDF from the private document store and
runs the layout engine. It never writes the PDF back or repacks it.

### Web surfaces

- `/gallery` lists published photographs, captions, PDF page citations, and
  confirmed people. An unlinked photograph still appears.
- The family profile has a separate “Archive photographs” section and count,
  distinct from uploaded profile photographs.
- `/admin/photographs` exposes caption, visibility, and link controls, plus
  confidence, source page, supporting text, and the ambiguity explanation.
  Candidates are rendered in the API's strongest-first order.

## Why the migration numbers are what they are

`0028_archive_photograph_inventory.sql` is AP-101's inventory migration and is
already on `main`. It cannot be edited after it has been applied: the migration
runner decides what to apply by file name, so an edited `0028` is skipped on a
database that already recorded it.

`0031_archive_photograph_assets.sql` added the asset byte-store columns and
publication state without editing `0028`. This extension therefore adds
`0032_archive_photograph_link_provenance.sql` for proposal provenance. It is
additive (`ADD COLUMN IF NOT EXISTS`, guarded constraints, `CREATE INDEX IF NOT
EXISTS`) and preserves existing rows.

`0027_source_narrative.sql` belongs to the source-narrative branch. It is not
renumbered, modified, or reused by this work.

## Acceptance mapping (AP-108)

The suite is `tests/photograph_acceptance.rs`. Every test creates its own
disposable database through `tests/support/genealogy_isolation.rs`, builds its
synthetic PDF in memory, and drops only its own database.

| Acceptance case | Test |
| --- | --- |
| A single-person photograph links only the caption-named person. | `a_single_person_caption_links_only_the_named_person` |
| A group caption links each explicitly named person. | `a_group_caption_links_each_explicitly_named_person` |
| An unidentified photograph appears in the gallery but on no profile. | `an_unidentified_photograph_appears_in_the_gallery_but_on_no_profile` |
| A pending photograph is invisible to a family viewer. | `a_pending_photograph_is_invisible_to_a_family_viewer` |
| A reviewer-restricted photograph is invisible for thumbnail and original. | `a_reviewer_restricted_photograph_is_invisible_for_thumbnail_and_original` |
| A repeated import creates no duplicates and preserves review state. | `a_repeated_import_creates_no_duplicates_and_preserves_review` |
| Every page of a two-page PDF is accounted for. | `every_page_of_a_two_page_pdf_is_accounted_for` |
| An extraction failure is recorded without creating an asset. | `an_extraction_failure_is_recorded_without_an_asset` |
| Thumbnail/original access is allowed for a linked member and refused for an unlinked caller. | `thumbnail_and_original_access_is_enforced_for_linked_and_unlinked_callers` |
| A layout proposal stores provenance and preserves a confirmed link. | `layout_proposals_persist_provenance_and_preserve_approved_links` |
| Namesakes are ordered by a stated year and the weaker candidate is marked ambiguous. | `layout_proposals_order_namesakes_and_record_ambiguity` |
| A biography continued on the next page is a low-confidence fallback. | `layout_proposals_use_a_continuing_biography_on_the_next_page` |

Pure layout-engine coverage is in
`src/genealogy/photograph_context.rs`: caption strength, explicit group,
namesake disambiguation, heading-only biography subject, unsupported photo,
multi-name biography, adjacent-page fallback, and separate nearby labels that do
not become a group link.

Browser coverage is `web/e2e/photographs.spec.ts`, with stubbed API responses:
the gallery, the separate profile section, the administrator controls, and the
non-administrator denial state. It asserts that no storage key, filesystem path,
or content hash appears in the rendered page.

## Verification results

Run from the repository root. The validation gate was run with the project's
disposable PostgreSQL test DSNs:

| Check | Result |
| --- | --- |
| `./scripts/test-db.sh exec -- sop validate` | **PASS** — `cargo build --locked`, `cargo test --locked`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` |
| `./scripts/test-db.sh exec -- cargo test --locked` | **PASS** — full Rust suite, including 12 acceptance tests |
| `./scripts/test-db.sh exec -- cargo test --locked --test genealogy_schema -- --nocapture` | **PASS** — configured integration gate |
| `cargo test --locked --lib genealogy::photograph_context` | **PASS** — 9 layout-engine tests |
| `cargo test --locked --test photograph_import --test photograph_review` | **PASS** — 4 import tests, 5 review tests |
| `cargo build --locked` | **PASS** |
| `cargo fmt --check` | **PASS** |
| `cargo clippy --locked --all-targets -- -D warnings` | **PASS** |
| `cd web && npm run typecheck` | **PASS** |
| `./scripts/web-control.sh build` | **PASS** — Next.js production build, including `/gallery` and `/admin/photographs` |
| `cd web && npx playwright test e2e/photographs.spec.ts` | **PASS** — 4 tests |
| `git diff --check` | **PASS** (run before commit) |

A bare `sop validate` with no database environment fails in the existing
`branch_review` suite because that suite needs a configured genealogy database.
The same gate with the repository's disposable test DSNs passes; the test
database is never the operator's application database.

`NOT RUN`: no operator/staging database migration, no production deployment, no
real-photograph import, and no provider/model call for this evidence document.

## Safety and data handling

- No real family photograph, caption, name, hash, or storage path is in the
  repository or this document. The test PDFs and images are generated at
  runtime.
- No operator database was migrated. Every test used a disposable
  `*_test` database.
- No real photograph was published, approved, or linked. Confirmed decisions in
  tests are synthetic.
- Nothing was bulk-approved. Imported assets still default to `unpublished`.
- The byte store stays under the ignored `data/` root; extracted bytes are never
  committed.
- This pull request is **not merged**.

## Limitations

- `page_crop` remains unavailable: no rasterizer is present and none was added.
- The thumbnail route serves the stored photograph rather than a derived
  thumbnail. It applies the same authorization and publication predicate as the
  original route.
- OCR is not present. A scanned, image-only PDF has no text context and yields no
  layout proposal; the inventory and extraction still work.
- Layout extraction reads text operators and font encodings. A document with an
  unusual embedded encoding can fall back to `extract_text` line order without
  positions; namesake disambiguation is then weaker.
- Adjacent-page proposals are deliberately low confidence and are used only when
  the target page names nobody.
- The `propose` admin action reads the original from the private document store;
  if the original is missing or outside the configured corpus, the action fails
  closed instead of inventing context.
