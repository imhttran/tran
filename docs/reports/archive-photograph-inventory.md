# Archive photograph inventory (AP-101)

This report describes the deterministic per-page photograph inventory for a
document version original. The implementation is `src/genealogy/inventory.rs`.
The schema is migration `genealogy/migrations/0028_archive_photograph_inventory.sql`.

The inventory records document properties only. It stores no image bytes, no
stream content, no storage path, and no real family data.

## Method

`inventory_bytes` takes the original bytes of one document version and opens
them with `lopdf` in memory. It walks every page with `doc.get_pages()` and
returns exactly one `PageRecord` per page, in 1-based page order.

The walk keeps three guarantees:

1. **One record per page.** A page with no image still produces a record with an
   empty image list. The accounting covers every page exactly once.
2. **A failed page is recorded, not dropped.** When `doc.get_page_content`
   returns an error, the record has `readable: false` and the exact `lopdf`
   error text in `failure_text`. The walk continues with the next page.
3. **A container that will not open is an error.** `inventory_bytes` returns
   `InventoryError::Unreadable` with the exact `lopdf` text. Empty input returns
   `InventoryError::Empty`. No partial inventory is returned.

For each page the walk reads the page resources and scans the content stream
tokens. It tracks the current transformation matrix (`cm`) so a `Do` operator
resolves its placement rectangle in default user space. A `BI` inline image is
recorded with no object id, because an inline image has none.

The walk never decodes an inline image payload. The tokenizer skips from `ID` to
the next `EI` pair, so image bytes never enter the inventory.

## Recorded object fields

One `ImageObject` per drawn image object:

| Field | Meaning |
| --- | --- |
| `object_number`, `generation` | lopdf object id; `0 0` for an inline image |
| `pixel_width`, `pixel_height` | declared sample dimensions, `None` when unresolved |
| `color_space` | colour space name, `None` when unresolved |
| `filters` | filter chain, space separated, `None` when unfiltered |
| `bits_per_component` | declared bit depth, `None` when unresolved |
| `complete`, `completeness_reason` | whether the declared stream covers the sample set, and why not |
| `placement` | `x`, `y`, `width`, `height` when the operators resolve it |
| `object_name` | the resource name the content stream used |
| `tinted`, `tint_signal` | whether `SMask`, `Mask`, `Matte`, `Separation`, or `DeviceN` is present, and which |
| `occurrence_count` | how many times the page draws the object |
| `page_frequency` | how many pages of the document reach the same object |

Unresolved properties stay `None`. The inventory never fills them with a guess.

## Classification signals and named rules

`classify` is a pure function of the recorded properties. Every result carries a
label, a named rule, and a detail string with the values that fired the rule.
The rules are checked in a fixed order.

### watermark

Rule `watermark-repeated-faint`.

Fires when the object appears on at least 3 pages **and** either its bit depth is
4 or less, its colour space is `DeviceGray` or `CalGray`, or it is tinted.
Detail: page count, tint flag, pixel dimensions, colour space, filters, bits, and
placement.

### photograph

Rules `photograph-full-page-opaque` and `photograph-inset-opaque`.

Fires when the colour space is `DeviceRGB`, `CalRGB`, `ICCBased`, `RGB`, or
`DeviceCMYK`, and the bit depth is 8 or more. The full-page rule fires when the
placement covers at least 60% of both pixel dimensions. The inset rule fires
otherwise.

### diagram

Rule `diagram-low-colour-or-bilevel`.

Fires when the bit depth is 4 or less or the colour space is `Indexed`, and the
object is not tinted.

### decorative

Rules `decorative-small-placement` and `decorative-gray-or-mask`.

The small-placement rule fires when the placement is under 10% of both pixel
dimensions. The gray-or-mask rule fires when the object is tinted or its colour
space is `DeviceGray` or `CalGray`.

### unknown

Rule `unknown-no-signal`.

The classifier returns `unknown` when no rule above fires. The detail always
names a non-empty reason: which properties are unresolved, or that no rule
matched the recorded values. The schema enforces a non-empty
`classification_detail` for `unknown`, so an undecided object always carries a
reason.

## Persistence and idempotency

`persist_inventory` writes one run row, one page row per page, and one image row
per drawn occurrence.

- The run key is `(document_version_id, content_sha256)`. The insert uses
  `ON CONFLICT DO NOTHING` and returns nothing on a repeated key. A second
  inventory of the same version and content hash is a no-op and writes zero
  image rows.
- A new content hash inserts a fresh run. The old rows keep their own key, so a
  replaced original cannot inherit an old inventory.
- The page and image inserts also use `ON CONFLICT DO NOTHING` against their own
  unique keys, so a retry cannot duplicate a page or an occurrence.
- A page caption is stored only when the page dictionary states `Caption` or
  `T`. Otherwise `caption` is `NULL`.
- The writer stores no image bytes and no storage path. It stores object ids,
  dimensions, colour space, filters, placement, flags, classification, rule, and
detail only.
- Any insert failure aborts the run. The caller wraps the calls in one
  transaction.

## Failures the inventory can record

| Failure | Where it is recorded |
| --- | --- |
| The whole container is not a PDF | `InventoryError::Readable` text (function error) |
| Empty input | `InventoryError::Empty` |
| A page content stream cannot be read | `PageRecord.readable = false` and `failure_text` with the exact error |
| An image object has no declared dimensions | `pixel_width` / `pixel_height` are `None` |
| An image object has no colour space | `color_space` is `None` |
| An image stream is shorter than its sample set | `complete = false` with `completeness_reason` |
| The content operators never place the object | `placement` is `None` |
| The signals do not decide | `unknown` with a reason in `classification_detail` |
| A database write fails | `InventoryError::Database` with the server text |

The schema also enforces these invariants with CHECK constraints:

- a page that is not readable must have non-empty `failure_text`;
- an incomplete image must have non-empty `completeness_reason`;
- `classification` is one of the five labels;
- `classification_rule` is not blank;
- `unknown` carries a non-empty `classification_detail`.

## Migration

The migration stays additive. Every statement is `CREATE TABLE IF NOT EXISTS`,
`CREATE INDEX IF NOT EXISTS`, `CREATE UNIQUE INDEX IF NOT EXISTS`,
`CREATE OR REPLACE FUNCTION`, `DROP TRIGGER IF EXISTS` + `CREATE TRIGGER`, or
`COMMENT ON`. No existing object is dropped, renamed, or retyped. The migration
runner records an applied file in `genealogy_schema_version`, so a second run
applies zero files.

## Tests

Unit tests live in the module test submodule. They use in-memory bytes only.
They need no database and write no file. They cover:

- an empty input error and an unreadable input error with text;
- a two-page in-memory PDF where every page appears exactly once and pages with
  no image still have a record;
- each classification label with its named rule and fired values;
- an undecidable object that is `unknown` with a non-empty reason.
