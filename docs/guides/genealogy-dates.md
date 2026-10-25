# Genealogy birth and death dates

This guide describes how explicitly stated birth and death dates move from the
private family catalog into evidence-backed genealogy assertions, and how they
reach the authorized API and the family-facing pages.

It covers TRAN-002H. It does not change the approval, publication, RBAC, or
Head of Family rules.

## 1. Model

A date is a **claim**, not a person attribute. It is stored on `assertion`
(migration `0025_assertion_dates.sql`) and keeps the same review workflow as
every other assertion:

| Column | Meaning |
| --- | --- |
| `assertion_kind` | `birth_date`, `death_date`, or `date_unresolved` |
| `value_text` | The source wording, for example `khoảng 1880` or `1877–1922` |
| `value_date_start` | Partial ISO start: `YYYY`, `YYYY-MM`, or `YYYY-MM-DD` |
| `value_date_end` | Partial ISO end, present exactly when the precision is `range` |
| `value_date_precision` | `day`, `month`, `year`, or `range` |
| `value_date_qualifier` | `exact`, `circa`, `before`, or `after` |
| `review_status` | The existing `unverified` → `verified` / `disputed` / `rejected` workflow |

`person.birth_year` and `person.death_year` are **not** written by the catalog
import. Those columns drive onboarding age and minor detection, so filling them
from unreviewed catalog text would change eligibility without a reviewer.
Dates therefore reach the UI only through assertions.

A `date_unresolved` assertion keeps the wording and its evidence but leaves the
structured columns `NULL`: the claim stays visible for review without pretending
the source stated a birthday.

### Why partial ISO values

`1877` cannot silently become `1877-01-01`. Text sorting is chronological for
these values, so range queries stay correct, and the precision travels with the
value.

## 2. Extraction rules

The parser reads two catalog shapes: the Markdown entity table (one row per
person, facts in the description cell separated by `;`) and the older
`Label: value` heading shape.

Accepted forms:

| Source text | Stored |
| --- | --- |
| `1877` | `1877`, `year`, `exact` |
| `1842-03` / `5/1842` | `1842-03`, `month`, `exact` |
| `1842-03-15` | `1842-03-15`, `day`, `exact` |
| `15/03/1842` or `03/15/1842` | `1842-03-15`, `day`, `exact` (only when one order is impossible) |
| `c. 1877`, `khoảng 1877`, `~1877`, `1877?` | `1877`, `year`, `circa` |
| `before 1877`, `after 1877` | `1877`, `year`, `before` / `after` |
| `1877–1922`, `1877-1922`, `1877 to 1922`, `1877 đến 1922` | `1877`…`1922`, `range` |
| `dates as printed: 1877–1922`, `(1877–1922)` | birth `1877` + death `1922` |
| `born 1877`, `sinh 1877`, `died 1922`, `mất 1922` | birth / death for that side only |
| `dates as printed: 1914` (a lone, unlabelled year) | `date_unresolved` |

Refusals, by design:

- **Ambiguous day/month order** (`03/04/1842`) → `date_unresolved`. The parser
  does not guess a locale.
- **Impossible calendar days** (`31/02/1842`, `29/02/1843`) → `date_unresolved`.
- **Years outside 1000–2100** → `date_unresolved`.
- **A range whose end precedes its start** → `date_unresolved`.
- **Prose without a date** (`died young`, `Died at age 14`, `death age 46`) →
  nothing. Age statements are not dates and are never converted.
- **A spouse's dates** (`Spouse: X (1878–1940)`) are never attributed to the
  entry itself.
- `birth order` and `birth place` are not dates.

The parser never emits a `verified` assertion.

## 3. Re-import and review

`genealogy import --apply` is idempotent:

- the assertion key is derived from the normalized start, end, and precision, so
  re-importing the same text inserts nothing and keeps the review status;
- a **corrected** date (for example `1877` → `1878`) creates a *second*
  assertion. The old claim is never overwritten, so conflicting claims stay
  visible for the reviewer;
- changing the source document hash reopens a verified date to `needs_review`
  through the existing evidence-fingerprint rule, and the review history is
  preserved;
- verifying a date that disagrees with an already-verified date of the same kind
  is blocked with `contradictory_date` and audited as a `needs_review` event.
  The reviewer rejects one claim rather than the importer picking a winner.

## 4. API surface

All reads are authorized by the existing operations; no new route is added.

| Endpoint | Date content |
| --- | --- |
| `GET /api/genealogy/person/{key}` | Every date assertion with `review_status`, `evidence`, and a structured `date` object (or `null` for `date_unresolved`). |
| `GET /api/genealogy/tree` | `birth` and `death` per person, **only** from verified assertions. |
| `GET /api/family/persons/{key}/profile` | A `dates` array, verified only, with `value` (source wording), `start`, `end`, `precision`, and `qualifier`. |

`dates` is additive: an older response without it is still valid.

## 5. Interface

- **Tree cards** show a concise lifespan beside the evidence status when it fits
  the fixed card width (for example `1877–1922` or `1877–c. 1922`). A full-day
  range is omitted there rather than colliding with the status text.
- **Person popup** always shows the full lifespan.
- **Family profile** has a "Dates" section with the readable value, the
  precision ("Year only", "Exact day", …) and the original wording.
- A missing death date is never rendered as "living".

## 6. Operator refresh

After deploying this change, apply the migration and re-import the catalog:

```sh
make genealogy-migrate
make genealogy-import
```

`make app-update` does both. The re-import adds date candidates; it never
verifies them and never rewrites a reviewed value.

Review them through the existing per-assertion workflow, for example:

```sh
cargo run --locked --bin genealogy person <person-key>
cargo run --locked --bin genealogy review <assertion-id> verified \
  --reviewer <name> --evidence "<source and page>" [--note "<reason>"]
```

## 7. Verification

```sh
cargo test --locked --lib genealogy::dates
cargo test --locked --lib genealogy::parser
cargo test --locked --test genealogy_dates
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
make web-check
make web-e2e
```

`tests/genealogy_dates.rs` covers idempotent re-import, corrected and
conflicting dates, reopen-on-source-change, the verified-only tree projection,
the structured person query, unresolved dates, and the schema constraints.

`web/e2e/family-dates.spec.ts` covers precision and qualifier rendering, the
card/popup lifespan, the profile "Dates" section, and the no-dates state.

## 8. Known limitations

- **Catalog facts cannot be verified on their own.** The review rule requires
  evidence linked to an `original` source document. The catalog import is stored
  as `catalog` (a derivative), so a date imported from it remains a candidate
  until original-source evidence is recorded. This is the same rule that
  already applies to names and relationships; it is not specific to dates.
- **No OCR or date extraction from the PDF text layer.** Dates must be present
  in the catalog text the importer reads.
- **No locale inference.** Dates written day-first with both components ≤ 12 stay
  unresolved by design.
- **No date arithmetic.** The importer never computes an age, a lifespan span, or
  a death date from an age statement.
- **Local time zone and calendar.** Only the Gregorian calendar is understood,
  and no time of day is stored.
