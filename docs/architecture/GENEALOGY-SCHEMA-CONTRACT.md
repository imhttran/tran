# Genealogy entity and evidence schema contract

Scope: TRAN-002. This contract defines the genealogy records and the evidence
rules that later migrations implement. It creates no migration and no table.
It states the persistence boundary and the terms that the schema must keep.

Source authority: `docs/tran/Family Genealogy.pdf`, `Tran-Family-Vietnamese.md`,
`Tran-Family-English.md`, and `Tran-Family-People-Catalog.md`. The PDF is the
original reference. The Vietnamese transcription is a derivative. The English
translation and the people catalog are interpretation aids, not independent
proof.

The model separates a _person_ from a _claim about a person_. A record that
links to a person is not a verified fact by itself. Only an assertion with
source evidence and a review decision can be `verified`.

## 1. Logical record categories

Seven logical record categories make the model. Each category is one table in
the persistence layer.

| Category              | Purpose                                                                             | Key identifier           | Evidence rule                                                   |
| --------------------- | ----------------------------------------------------------------------------------- | ------------------------ | --------------------------------------------------------------- |
| `source_document`     | A private local source file, its hash, language, and derivation lineage.            | `source_document_id`     | Holds the SHA-256 of the file bytes. Never committed.           |
| `source_evidence`     | One citation anchor: source, page, locator, optional private excerpt.               | `source_evidence_id`     | The only object that can justify `verified`.                    |
| `person`              | A distinct person, separate from any source ordinal.                                | `person_id`              | Carries identity and review state, not birth facts.             |
| `person_name`         | One name form of a person: original, normalized, script, alias type.                | `person_name_id`         | Links to `person_id`; keeps literal Vietnamese spelling.        |
| `family_branch`       | One family branch with an explicit number or label.                                 | `family_branch_id`       | Requires source evidence for the branch claim.                  |
| `genealogy_assertion` | One claim: subject, predicate, optional object or literal, status, review metadata. | `genealogy_assertion_id` | Requires `source_evidence_id` before `verified`.                |
| `catalog_entry`       | One stable key for one source catalog entry and its parse result.                   | `catalog_entry_key`      | Links to candidate person and assertion IDs, never proves them. |

### 1.1 `source_document`

Fields: `source_document_id`, `relative_path`, `sha256`, `language`,
`derived_from_source_document_id` (nullable reference to another
`source_document_id`), `source_kind` (`original`, `transcription`,
`translation`, `catalog`, or `derivative`).

Rule: the SHA-256 must equal the value in the private TRAN-001 manifest. The
four source files keep their hashes. A hash change reopens all assertions that
cite the document. Only a document with `source_kind = 'original'` and no
derivation parent is authoritative; transcriptions, translations, catalogs, and
other derivatives can be evidence but cannot independently justify `verified`.

### 1.2 `source_evidence`

Fields: `source_evidence_id`, `source_document_id`, `page_number` (1-based,
nullable), `locator`, `excerpt` (nullable, private), `char_offset` (nullable).

Rule: `page_number` must be in the valid page range of the source PDF. This
range is 1 to 56 for the current input. An out-of-range page number is rejected
at parse time and stays `unresolved`.

Review rule: a `verified` assertion requires at least one
`assertion_evidence` row linked through `source_evidence` to an `original`
source document, with a 1-based `page` and a non-empty `locator`. A null
`source_evidence_id`, evidence linked only to a derivative catalog or
transcription, and evidence without a valid page/locator all refuse
verification.

### 1.3 `person`

Fields: `person_id`, `identity_state`, `review_state`, `created_at`,
`updated_at`.

Rule: `person_id` is stable and opaque. It does not encode source position,
name, or branch.

### 1.4 `person_name`

Fields: `person_name_id`, `person_id`, `original_form`, `normalized_form`,
`script`, `alias_type` (`primary`, `alias`, `translation`).

Rule: keep the literal Vietnamese spelling in `original_form`. The normalized
form is for search only and never replaces the original.

### 1.5 `family_branch`

Fields: `family_branch_id`, `branch_number` (nullable), `label`,
`source_evidence_id`.

Rule: a branch membership is an assertion, not a person attribute. Do not infer
branch from a surname or a missing field.

### 1.6 `genealogy_assertion`

Fields: `genealogy_assertion_id`, `subject_person_id`, `predicate`,
`object_person_id` (nullable), `literal_value` (nullable), `source_evidence_id`
(nullable only before review), `assertion_status`, `review`.

Predicates include `parent_child`, `spouse_partner`, `branch_membership`,
`sex_gender`, `birth_date`, `death_date`, `living_status`, and `birth_order`.

Rule: exactly one of `object_person_id` or `literal_value` is set. Facts such as
sex, dates, living status, and birth order are assertions, never default guesses.

### 1.7 `catalog_entry`

Fields: `catalog_entry_key`, `source_document_id`, `raw_text_reference`
(private), `parse_result` (`parsed`, `ambiguous`, `rejected`), `reason`
(nullable), `candidate_person_ids`, `candidate_assertion_ids`.

Rule: `catalog_entry_key` is stable across parses of unchanged input. Every
entry is accounted for exactly once.

## 2. Entity identifiers

- Every identifier is an opaque string. Use a UUIDv7-style string for
  `person_id`, `family_branch_id`, and assertion IDs. Never reuse an ID.
- `catalog_entry_key` is a deterministic key: source document ID plus the
  source-relative entry location. The same input always yields the same key.
- `source_document_id` is deterministic from `relative_path`. A file rename
  creates a new `source_document_id` and reopens its assertions.
- Identifiers never contain a name, date, branch number, or source ordinal.

## 3. Citation format

Every displayed claim carries one citation per supporting evidence row. The
rendered form is:

```
<source-file>:p<page>#<locator>
```

Examples:

- `Family Genealogy.pdf:p12#section-3`
- `Tran-Family-Vietnamese.md:p0#entry-042` (use `p0` when no page exists)

Rules:

- `page` is the 1-based PDF page, or `p0` when the source has no page.
- `locator` is the section or entry anchor inside the source.
- A rendered citation never includes a private excerpt.
- A claim with no evidence renders `unverified, no citation`.

## 4. Assertion states

`assertion_status` has four explicit values.

| State       | Meaning                                                    | Can display as a relative? |
| ----------- | ---------------------------------------------------------- | -------------------------- |
| `candidate` | Extracted or proposed, not reviewed.                       | No.                        |
| `verified`  | Reviewed and backed by valid source evidence.              | Yes, with citation.        |
| `disputed`  | Reviewed and contradicted by other evidence or a reviewer. | No; show as disputed.      |
| `rejected`  | Reviewed and found false or not in the source.             | No.                        |

Allowed transitions:

- `candidate` -> `verified`
- `candidate` -> `disputed`
- `candidate` -> `rejected`
- `disputed` -> `verified` (only with new resolving evidence)
- `disputed` -> `rejected`
- `verified` -> `disputed` (only when cited evidence changes or is withdrawn)
- `verified` -> `candidate` (only when cited evidence changes; reopens review)

No other transition is allowed. `rejected` is terminal for its evidence set; a
new evidence set creates a new assertion.

Promotion rules:

- No assertion becomes `verified` without a `source_evidence_id`.
- No parser, model, or translation output sets `verified`.
- A cyclic or self-referential `parent_child` assertion cannot become `verified`.
- A changed source hash reopens `verified` assertions to `candidate`.

## 5. Uncertainty and identity states

`identity_state` has three values:

| State         | Meaning                                              |
| ------------- | ---------------------------------------------------- |
| `resolved`    | One person with reviewed, unique evidence.           |
| `provisional` | One person with candidate evidence only.             |
| `unresolved`  | Two or more candidate people may be the same person. |

Unknown facts stay unknown. A missing death date never implies `living`. A
missing birth order never implies first-born. A shared surname never implies
kinship.

## 6. Identity-resolution rules

- Shared names never merge automatically. Each catalog entry becomes its own
  candidate `person` until evidence supports a merge.
- A merge requires at least one evidence row that names both candidate people,
  or a reviewer decision with a stated reason.
- A merge keeps both `person_id` values as history. Do not delete a candidate.
- Conflicting parentage keeps both assertions `candidate` or `disputed`.
- Re-import never auto-merges, auto-promotes, or erases a reviewer decision.
- `unresolved` identity blocks any `verified` relationship that depends on it.

## 7. Review transitions

`review_state` has four values: `unreviewed`, `in_review`, `approved`,
`rejected`.

Allowed transitions:

- `unreviewed` -> `in_review`
- `in_review` -> `approved`
- `in_review` -> `rejected`
- `approved` -> `in_review` (only when cited evidence changed)

Every transition records the reviewer, the timestamp, the prior state, the new
state, and a reason. The review audit trail is append-only. A review acts on one
chosen assertion ID. There is no mass approval.

`approved` review plus valid evidence is the only path to `assertion_status =
verified`.

## 8. Privacy boundaries

- All real genealogy records, source files, private excerpts, private reports,
  and database exports stay local. Never commit them.
- `docs/tran/` and `docs/tran/audit/` are private and untracked. Generated audit
  output is excluded from the source inventory.
- Committed fixtures are synthetic only. Use no living-person names, real family
  dates, or real hashes in tracked files and CI logs.
- Real data uses an explicit `GENEALOGY_DATABASE_URL`. Import and migration stop
  when the destination is unset, nonlocal, or equal to `DATABASE_URL`.
- Automated integration tests use disposable databases only.
- No route exposes family records before RBAC exists. The `Head of Family`
  designation never grants or implies an application permission.

## 9. Current-version authority

The schema stores every version of a claim. Exactly one version per slot is
_current_; the rest are history.

- `assertion.superseded_at` marks a row that an authority decision replaced. A
  current-state read (`person`, tree, person search, archive person answers,
  Head of Family eligibility) filters `superseded_at IS NULL`.
- A superseded row keeps its `value_text`, `value_date_*`, `review_status`,
  review events, and evidence rows. Supersession never deletes and never changes
  the review status.
- `authority_decision` records one decision: scope (`document`, `person`, or
  `assertions`), the exact document version and its content hash, the reviewer,
  the reason, the confirmation token, and the lifecycle state. At most one
  decision is `active` per scope target; the application enforces it inside a
  transaction-scoped advisory lock and retires overlapping decisions by
  recording `retired_by_decision_id`.
- `authority_decision_item` records each assertion a decision made current and
  the assertion it replaced. The table is append-only.
- `authority_decision_event` is the append-only decision trail (`applied`,
  `reversed`).
- A _slot_ is `(person, kind, key)`, except for `display_name`, `birth_date`, and
  `death_date`, which are single-valued and use `(person, kind)`. Two aliases,
  branches, parents, spouses, or links can coexist; two spellings of one name
  cannot both be the current display name.
- Omission is not deletion. A current assertion the document does not mention is
  superseded only when the reviewer names it in the apply request.
- A reversal inserts a new decision of kind `reversal`, supersedes what the
  reversed decision made current, and restores what it replaced. Neither decision
  nor any assertion is deleted.
- **Authoritative is not verified.** The authority tables never write
  `review_status`. Verification still requires its own evidence review and the
  provenance rule in §7.

## 10. Persistence boundary

This contract is implemented by additive, versioned SQL in `genealogy/migrations/`
(TRAN-002B). The existing `migrations/*.sql` files and the RAG `documents` table
stay unchanged, as they do in `src/storage.rs` and `src/storage_pg.rs`.

The storage conventions that apply:

- One migration file per version, applied in order, idempotent on a second run.
- Foreign keys enforce evidence references.
- A check constraint restricts `assertion_status` and `review_state` to the
  listed values.
- A unique constraint protects `catalog_entry_key`.
- A transaction covers each import, so a failed import leaves no partial graph.

No migration is created by TRAN-002A. Authority decisions are added by
`0026_authority_decisions.sql`.
