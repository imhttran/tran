# Current source of truth

How an authorized reviewer designates one approved document version as
authoritative, and how to inspect or reverse that decision.

This surface changes **which stored version is current**. It does not verify a
claim, and it does not delete anything.

## When to use it

Use it when two reviewed sources record the same fact differently and the family
needs one current answer: a corrected submission, a re-transcribed name, a
revised birth year, or a replaced parent link.

Do not use it to settle an identity. Two people who share a name stay two people;
choosing which record is authoritative for one person never merges them.

## What you see before applying

The preview lists four things and nothing is written until you confirm:

| Group | Meaning |
| --- | --- |
| Additions | The document supports an assertion that no current record occupies |
| Replacements | A current record in the same slot is superseded by this one |
| Conflicts | A record losing authority is `verified` or `disputed` |
| Omissions | A current record the document does not mention |

Conflicts require a second acknowledgement. Omissions are **not** deletions: they
stay current unless you tick them for removal, and a checked omission becomes an
explicit, audited removal rather than a side effect of upload.

A *slot* is one person plus one assertion kind plus one key. `display_name`,
`birth_date`, and `death_date` are single-valued, so a newer spelling replaces the
previous one. Aliases, branches, parent links, spouse links, and social links are
multi-valued, so two different entries can both be current.

## Scopes

| Scope | Governs |
| --- | --- |
| Whole document | Every assertion the version supports, for every person in it |
| One person | Only that person's assertions from the version |
| Selected records | Only the records you tick |

The person and selected-record scopes exist so a document that also mentions
someone else cannot silently change that person's facts.

## The confirmation token

The preview carries a hash of the exact effect. The apply request must echo it.
If any slot, omission, conflict, or the document content hash changed in between,
the apply is refused with `authority_stale` and nothing is written. That is the
stale-review and concurrent-reviewer guard: re-preview and confirm again.

## What the decision records

- reviewer, timestamp, reason, scope, document version, and content hash;
- which assertions became current and which ones they replaced;
- the superseded rows, unchanged, with their values, citations, and review
  events;
- an append-only event trail (`applied`, `reversed`).

## Current views and answers

After an apply, the family tree, person search, the family profile, and archive
person answers read the current version. A superseded record is still readable
through the decision history and keeps its citations.

The person answer and the assertion list label a current version as
**Authoritative** and name the decision and its reason. That label is separate
from the review status: an authoritative assertion may still be `Unverified`.

## Reversing a decision

Only the active decision can be reversed. Reversal is a *new* decision: it
supersedes what the reversed decision made current and restores what it replaced,
recording the reviewer and the reason. Both decisions and every assertion stay in
the database.

## Operator commands

The console is at `/admin/source-of-truth` for administrators. The same operations
exist in the local CLI, which only ever talks to the guarded
`GENEALOGY_DATABASE_URL`:

```sh
# Inspect the effect first. The document content hash is read from the stored
# extraction, so it does not have to be copied by hand.
cargo run --locked --bin genealogy -- authority-preview 42

# One person only, and confirm the exact effect.
cargo run --locked --bin genealogy -- authority-preview 42 --scope person --person P013

# Selected records, with one explicit removal.
cargo run --locked --bin genealogy -- authority-preview 42 \
  --scope assertions --assertions 101,104 --remove 88

# Apply the preview you reviewed.
cargo run --locked --bin genealogy -- authority-apply 42 \
  --scope document \
  --reason "Supersedes the 2024 transcription" \
  --actor admin-subject \
  --preview-hash 0123456789abcdef \
  --confirm --acknowledge-conflicts

# Inspect the decision trail, optionally for one person.
cargo run --locked --bin genealogy -- authority-history
cargo run --locked --bin genealogy -- authority-history P013

# Restore the previous version through a new decision.
cargo run --locked --bin genealogy -- authority-reverse 7 \
  --reason "The earlier value was correct" --actor admin-subject
```

`authority-apply` refuses without `--confirm`, without a reason, with a stale
preview hash, or with an unacknowledged conflict. `authority-reverse` refuses a
decision that is not active.

## API routes

| Route | Purpose |
| --- | --- |
| `POST /api/genealogy/documents/{version}/authority/preview` | preview a scope |
| `POST /api/genealogy/documents/{version}/authority/apply` | apply a confirmed preview |
| `GET /api/genealogy/authority[?person_key=…]` | decision history |
| `POST /api/genealogy/authority/{id}/reverse` | reverse the active decision |

All four require the `DocumentReview` operation (administrator). The handlers
re-read the stored document version and its content hash before any write, so a
replaced or withdrawn document cannot be designated.

## Boundaries

- The genealogy database is separate; nothing here reads or writes RAG tables
  directly. The RAG document version is revalidated first.
- No account link, Head of Family eligibility, namesake identity, or privacy
  setting changes because of a decision.
- A decision never writes `review_status`.
- Superseded rows are never deleted, and no migration rewrites history.
