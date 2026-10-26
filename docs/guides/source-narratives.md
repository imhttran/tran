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

| Table                           | Purpose                                                                                                                    |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `source_narrative_passage`      | One passage revision: slot key, revision key, language, section, page range, body, content hash, review status, visibility |
| `source_narrative_link`         | One `(passage, person)` link with `resolved`, `ambiguous`, `confirmed`, or `rejected`                                      |
| `source_narrative_review_event` | Append-only audit of passage and link decisions                                                                            |

Migration: `genealogy/migrations/0027_source_narrative.sql`.

A slot is one `(document, page, section)` position. A slot can hold several
revisions: the first uses the slot key, and changed text gets a revision key
derived from its content hash. An approved revision is never overwritten.

## 2. Extraction

The parser recognises:

- page boundaries: `## Trang PDF N`, `## PDF page N`, `## Page N`, `## Trang N`,
  or a line that is only `[p. N]`; the prefixed forms are matched
  case-insensitively;
- section headings: `###` or deeper, kept as the section title;
- paragraphs: runs of non-empty lines, joined with a blank line.

A section that continues onto the next page keeps its identity and its page range
grows (`page_start`…`page_end`). Text before the first page marker is counted as
an unpaged section and is not imported, because it cannot be cited.

## 3. Person linking

Links come from three signals, in this order:

| Signal                                                                                               | Result                         |
| ---------------------------------------------------------------------------------------------------- | ------------------------------ |
| An explicit person key in the text or heading (for example `ce-0007-…`, matched as a token sequence) | `resolved` / `explicit_id`     |
| A section heading naming exactly one catalog person                                                  | `resolved` / `section_heading` |
| A full-name mention in the body                                                                      | `ambiguous` / `name_mention`   |

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

### Web review screen

Administrators can review passages at `/admin/narrative-review`. The route is in
the authenticated admin area, and every list, detail, link, and passage request
re-checks the administrator review permission in Rust. An ordinary viewer or
editor cannot list or decide passages.

The queue defaults to pending passages. Reviewers can combine these filters
(with AND on the server before pagination):

- **Person** — search display names, recorded aliases, and person keys. Search
  ignores case and Vietnamese accents, including `đ` → `d`, and preserves the
  original names. Results show the person key and any available lifespan or
  parent context so same-named people remain distinguishable. Typing only
  searches; the queue is filtered by the exact person key after a result is
  selected. Text is never used to infer identity.
- **Search passage text** — a separate accent-insensitive search over section
  titles and passage bodies only.
- **Review status** and **language**.
- **Person links** — `All`, `Needs confirmation` (at least one ambiguous link),
  `No accepted links` (no resolved or confirmed link), `Has accepted links` (at
  least one resolved or confirmed link), or `No candidates` (no person links).
  These definitions can overlap; each is stated explicitly.
- **Visibility**, **source document**, and an inclusive **PDF page range**
  matched by overlap. Page numbers must be positive and the end page must be at
  least the start page.
- Clear filters restores the pending default and the first page.

Accent normalization is provided by
`genealogy/migrations/0029_narrative_review_search.sql`. It normalizes only the
search input and stored text at read time; it never changes the original names or
passage wording.

Each row shows the passage id, heading, language, source document, PDF pages,
review status, and one of four link summaries: `confirmed`, `resolved`,
`ambiguous`, or `unmatched`. Ambiguous rows list every candidate.

The queue uses bounded pagination with stable ordering and reports the total
matching passage count and current page range. Changing filters resets to page
one. Opening a passage keeps the active filters and page; returning to another
passage or the list does not reset the queue.

Text and person searches are debounced. A sequence guard discards older
responses so a slow request cannot replace a newer result. Loading, error, and
empty states are announced; the empty state offers clear filters.

The review detail shows the complete verbatim passage, source document, page
range, section, review status, visibility, and the genealogy context for every
candidate (person key, aliases, verified lifespan, and recorded parents, spouses,
and children). It states that only citation metadata is available; the original
document is not served. The existing source-details control shows the citation.

The screen keeps the two decisions separate:

1. **Person link** — select one candidate explicitly and choose **Confirm selected
   person** or **Reject selected person**. An ambiguous match has no default
   selection. Confirming a link never approves the passage.
2. **Passage** — choose **Approve for family**, **Approve for admins only**,
   **Reject passage**, or **Return to pending**. **Approve for family** is available
   only after a `resolved` or `confirmed` link exists; the server enforces the same
   rule. Opening a passage or confirming a link never approves it.

The detail includes the append-only review events. Link and passage decisions are
written as separate events with the reviewer fingerprint, status transition,
visibility, note, and timestamp. If another reviewer changes the link status or
the passage version while the screen is open, the next decision returns `409` and
the screen reloads the latest state instead of overwriting it.

## 7. Family profile

`GET /api/family/persons/{person_key}/profile` adds a `history` array beside
`fields`, `media`, and `dates`:

```json
{
  "history": [
    {
      "passage_id": 12,
      "language": "vi",
      "document_ref": "docs/tran/Tran-Family-Vietnamese.md",
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

### Person-scoped history endpoint

The family tree fetches history only when a node's **Personal history** link is
opened:

```text
GET /api/family/persons/{person_key}/history
```

It applies the same authorization, the same membership check in the same
transaction, and the same published-passage predicate as the profile. The
response is `{ person: { person_key, display_name }, history: [...] }`, where each
passage has the same shape as the profile's `history[]`, including
`document_ref`.

The tree response adds `history_passages` to each person. The server counts only
when the authenticated caller is an active linked family member; identity or link
resolution fails closed to zero without failing the tree. The counts and the
dialog use one shared SQL predicate, so a link is never shown for a person whose
passages the dialog would not return.

On the tree, **Personal history** appears inside a node only when
`history_passages > 0`. It is a separate focusable control, not part of the card's
select/drag button, so opening it does not select the node, start a drag, open the
person popup, or navigate. The dialog shows every available passage with its
language, source document, PDF page range, section title, and verbatim
paragraphs. **View source details** opens the existing source-details control with
the document reference and page; it never invents an excerpt. Escape and the
visible **Close** button both close the dialog, and focus returns to the link.

The tree also shows no link for a person whose passages are pending, rejected,
admin-only, restricted, superseded, or linked only ambiguously.

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
revisions, append-only audit, the English `## PDF page N` marker, the document
reference, the count helper agreeing with the per-person read, the admin review
queue and detail, separate link and passage decisions, stale-state conflicts,
combined filters, accent-insensitive text and person search, page-overlap and
pagination behavior, and the `no_candidates` link state.

`tests/family_profile_presentation.rs` covers the person-scoped history read: an
unlinked caller is refused, and an approved passage carries its document
reference.

`web/e2e/family-history.spec.ts` covers the profile section, the language and page
citation, the absence of pending and admin-only text, separation from member
fields, the unauthorized response, and the tree link opening the dialog for an
eligible node only.

`web/e2e/family-tree.spec.ts` covers the link as its own control: it appears only
on a node with published history, opens the dialog without opening the person
popup, lists every passage with language, document, and pages, closes on Escape
and on the Close button, reuses the source-details control, and returns focus.

`web/e2e/narrative-review.spec.ts` covers the admin review screen with synthetic
data: viewer denial, pre-approval hiding, combined filters, accent-insensitive
text and person search, same-name distinction, page overlap, pagination, the
`no_candidates` state, filter retention when opening a detail, clear filters,
explicit ambiguous selection, separate link confirmation, family approval, the
audit trail, and the resulting tree link and dialog.

## 9. Known limitations

- **The private sources are not committed.** The PDF and both Markdown
  transcriptions are local, ignored inputs. The English `## PDF page N` marker is
  covered by a synthetic test; no real English passage is committed.
- **No OCR.** Passages come from the Markdown transcription. The PDF text layer is
  not re-read.
- **Reviewer access is administrator-only.** The web review screen uses the
  existing administrator review permission; there is no separate editorial role.
- **Explicit identifiers match the person key.** The people-catalog row labels
  (`P007`) are not carried into `person_key`, so an identifier in a narrative is
  only useful when it matches the stored key (for example `ce-0007-…`). A
  hyphenated key is matched as a token sequence in the body or the section
  heading.
- **Mentions are numerous.** A long section can mention many relatives; the dry
  run reports the candidate count so the reviewer can judge the workload. An
  ambiguous link stays unlinked until a reviewer confirms it, so it produces no
  tree link.
- **No original-file viewer exists.** Source details are citation metadata only:
  the document reference and page, not the PDF itself.
- **No RAG indexing.** Passages are not embedded and are not available to Ask the
  archive.
