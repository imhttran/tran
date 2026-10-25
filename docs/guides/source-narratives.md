# Source-linked historical narratives

This guide covers the archive narratives imported from the family history
transcription: how they are extracted, linked to people, reviewed, and published
on a family profile.

It covers TRAN-002I. Narrative passages are separate from structured genealogy
facts and from member contributions.

## 1. What a passage is

A passage is a verbatim block of a source document:

- the original wording, with paragraph breaks preserved;
- its language (`vi` or `en`);
- its source document (relative path and SHA-256);
- its PDF page range;
- the section heading it sits under;
- a review status and a visibility (`admin_only` or `family`).

The importer never translates, never regenerates OCR, and never rewrites the
wording. It only decides where a block starts and ends.

### Storage

| Table | Purpose |
| --- | --- |
| `source_narrative_passage` | One passage revision: slot key, revision key, language, section, page range, body, content hash, review status, visibility |
| `source_narrative_link` | One `(passage, person)` link with `resolved`, `ambiguous`, `confirmed`, or `rejected` |
| `source_narrative_review_event` | Append-only audit of passage and link decisions |

Migration: `genealogy/migrations/0027_source_narrative.sql`.

A slot is one `(document, page, section)` position. A slot can hold several
revisions: the first uses the slot key, and changed text gets a revision key
derived from its content hash. An approved revision is never overwritten.

## 2. Extraction

The parser recognises:

- page boundaries: `## Trang PDF N`, `## Page N`, `## Trang N`, or a line that is
  only `[p. N]`;
- section headings: `###` or deeper, kept as the section title;
- paragraphs: runs of non-empty lines, joined with a blank line.

A section that continues onto the next page keeps its identity and its page range
grows (`page_start`…`page_end`). Text before the first page marker is counted as
an unpaged section and is not imported, because it cannot be cited.

## 3. Person linking

Links come from three signals, in this order:

| Signal | Result |
| --- | --- |
| An explicit person key in the text or heading (for example `ce-0007-…`, matched as a token sequence) | `resolved` / `explicit_id` |
| A section heading naming exactly one catalog person | `resolved` / `section_heading` |
| A full-name mention in the body | `ambiguous` / `name_mention` |

Rules:

- A one-word name is never matched; given names repeat across generations.
- A heading or mention matching several catalog people is `ambiguous`, never
  resolved by ordering.
- An ambiguous link must be confirmed by a reviewer before the passage can reach
  a family profile.
- Passages and links do not require the person to have a user account; deceased
  ancestors and people without accounts work the same way.

## 4. Dry run

The dry run reads the source and the existing catalog. It writes no rows and
prints counts (or a local Markdown report with titles, pages, and link counts —
never passage bodies).

```sh
cargo run --locked --bin genealogy -- narrative dry-run \
  docs/tran/Tran-Family-Vietnamese.md --language vi --report /tmp/narrative-dry-run.md
```

Counts reported: passages, distinct pages, unpaged sections, resolved links,
ambiguous links, and section headings with no person match.

The report is local. Do not commit it: it names private people and pages.

## 5. Import

```sh
cargo run --locked --bin genealogy -- narrative import \
  docs/tran/Tran-Family-Vietnamese.md --language vi
```

- Imported passages start `pending`, visibility `admin_only`.
- A second unchanged import creates no passages and no links.
- A changed paragraph in a `pending` passage updates that row in place, moves its
  revision key to the new content, and **re-derives its person links**. A link
  decision applies to the wording it was made against, so it does not survive a
  text change; the audit events that recorded it remain.
- A slot holds at most one pending revision (partial unique index).
- A changed paragraph in an `approved` or `rejected` passage adds a new
  `pending` revision and leaves the reviewed wording untouched.

`--kind` sets the source-document kind (default `translation`).

## 6. Review

Passage text, audience, and links are reviewed separately.

```sh
# Text and audience
cargo run --locked --bin genealogy -- narrative review <passage-id> approved \
  --reviewer <name> --visibility family [--note "…"]

# A name mention
cargo run --locked --bin genealogy -- narrative link <passage-id> <person-key> \
  confirm --reviewer <name> [--note "…"]

# What is linked to one person, with page and status
cargo run --locked --bin genealogy -- narrative passages --person <person-key>
```

Rules enforced by the tool:

- publishing for the family (`approved` + `visibility family`) requires at least
  one `resolved` or `confirmed` person link;
- setting visibility back to `admin_only` is the restriction mechanism and hides
  the passage immediately;
- every decision appends a review event; events cannot be edited or deleted.

Imported passages are never bulk-approved.

## 7. Family profile

`GET /api/family/persons/{person_key}/profile` adds a `history` array beside
`fields`, `media`, and `dates`:

```json
{
  "history": [
    {
      "passage_id": 12,
      "language": "vi",
      "section_title": "…",
      "page_start": 11,
      "page_end": 12,
      "body": "…"
    }
  ]
}
```

The server returns a passage only when all of these hold:

- the passage is `approved`;
- its visibility is `family`;
- the person link is `resolved` or `confirmed`;
- it is the newest approved revision in its slot.

`pending`, `rejected`, `admin_only`, restricted, and superseded passages never
appear. The caller must also be a linked family member; the existing
authorization runs in the same transaction as the read.

The profile page shows this under **History from the family archive**, with the
language and the PDF page citation. It is a separate section from the member's own
`biography`, `my_story`, and `family_memories`, which the importer never writes.

## 8. Verification

```sh
cargo test --locked --lib genealogy::narrative
cargo test --locked --test source_narrative
cargo test --locked --test family_profile_presentation
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
make web-check
make web-e2e
```

`tests/source_narrative.rs` covers idempotent import, heading versus mention
links, publication, privacy, link confirmation, the publish guard, source-change
revisions, and append-only audit.

`web/e2e/family-history.spec.ts` covers the profile section, the language and page
citation, the absence of pending and admin-only text, separation from member
fields, and the unauthorized response.

## 9. Known limitations

- **No English Markdown source is present in this checkout.** Only the PDF, the
  Vietnamese transcription, and the people catalog are available locally. The
  importer is language-neutral (`--language en`), but the English narrative was
  not exercised.
- **No OCR.** Passages come from the Markdown transcription. The PDF text layer is
  not re-read.
- **Admin review is command-line only.** There is no web review screen for
  passages in this change; the existing review console covers member profile
  revisions.
- **Explicit identifiers match the person key.** The people-catalog row labels
  (`P007`) are not carried into `person_key`, so an identifier in a narrative is
  only useful when it matches the stored key (for example `ce-0007-…`). A
  hyphenated key is matched as a token sequence in the body or the section
  heading.
- **Mentions are numerous.** A long section can mention many relatives; the dry
  run reports the candidate count so the reviewer can judge the workload.
- **No RAG indexing.** Passages are not embedded and are not available to Ask the
  archive.
