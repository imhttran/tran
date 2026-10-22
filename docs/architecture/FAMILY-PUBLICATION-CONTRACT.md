# FAMILY-PUBLICATION-CONTRACT

Status: **PROPOSED — design and verification contract only.** This document adds
no publication store, no publishing endpoint, and no family-content indexing.
It defines the boundary that later implementation slices must satisfy.

Scope owner: family presentation, genealogy review, profile and story
publication contracts. Document storage, ingestion, workers, and retrieval
infrastructure are owned separately; this contract only consumes their
existing publication boundary.

## 1. Purpose

Family information is already collected under review: profile values, media,
genealogy assertions, and documents each have an approval workflow and a
visibility rule. What does not exist yet is a single, explicit answer to:

> Given a piece of family information, may it be shown to relatives, and may it
> be indexed for retrieval?

This document answers that question with one eligibility predicate, one
publication identity, and one lifecycle, so that presentation and indexing
cannot drift apart.

## 2. Vocabulary

- **Authoritative revision** — the approval record that currently owns a piece
  of content: `onboarding_profile_value.approved_revision_id`,
  `onboarding_media_metadata.revision_id`, a verified `assertion`, or
  `document_version.version_id`.
- **Effective visibility** — the scope that applies to the *current* approved
  content, not to a draft or a proposal: `owner_private` or `family` for
  profile content, `family` or `admin_only` for documents, and "verified shared
  fact" for genealogy.
- **Presentation** — an authenticated family-facing read (a profile page, a
  family tree panel, an authorized media download, a document preview).
- **Publication (indexing)** — writing content into the retrieval pipeline so
  `/api/query` can answer with it and cite it.
- **Publication item** — the pair of a publication identity and its content
  fingerprint. One item exists per approved version of one source.
- **Candidate** — an eligible item offered to the indexing adapter. A candidate
  is not published until the sink revalidates it.

## 3. Eligibility predicate (normative)

For content of type `C` from source `S`, observed at time `t`:

```
eligible(C, S, t) :=
      status(authoritative_revision(C, S, t)) == approved
  AND effective_visibility(C, S, t) == family
  AND not removed(C, S, t)
  AND not superseded(C, S, t)
  AND provenance_binding_valid(C, S, t)
  AND no_blocking_dispute(C, S, t)
  AND audience_authorized(requester, C, S, t)
```

Every clause is evaluated against the authoritative database, inside the same
transaction as the read or write it guards. No clause may be satisfied by
cached state, by an earlier check, or by client input.

`audience_authorized` is per-requester and is never a property of the item:
an item that is eligible for one relative is not thereby eligible for another.

### 3.1 Mapping per content type

| Content | `status` | `effective_visibility` | `not superseded` | `provenance_binding_valid` | `no_blocking_dispute` |
| --- | --- | --- | --- | --- | --- |
| Profile value | revision `approved` | `onboarding_profile_value.visibility` | `value.approved_revision_number == revision.revision_number` | `value.approved_revision_id == revision.revision_id` and the value matches the revision text | n/a |
| Profile media | revision `approved` | `onboarding_media_metadata.visibility` | revision is the current approved revision for that field | `onboarding_media_metadata.revision_id` joins a revision with the same visibility | n/a |
| Genealogy fact | `assertion.review_status = verified` | shared family fact | latest `assertion_review_event` is the current one | evidence is an original, non-derivative source with page ≥ 1 and a locator | `review_status` is not `disputed` or `needs_review` |
| Document | `review_status = approved` | `access_scope = family` | `processing_status = ready` and `approved_version_number` is current | `approved_sha256 = content_sha256`, `approved_scope = access_scope`, `approved_version_number = version_number`, `removed_at IS NULL` | n/a |

The document row matches the existing `approved_documents` view exactly
(`genealogy/migrations/009_document_lifecycle.sql`):

```sql
CREATE VIEW approved_documents AS
SELECT d.*
FROM documents d
JOIN document_version v ON v.version_id = d.document_version_id
WHERE v.review_status = 'approved'
  AND v.processing_status = 'ready'
  AND v.removed_at IS NULL
  AND v.access_scope = 'family'
  AND v.approved_sha256 = v.content_sha256
  AND v.approved_scope = v.access_scope
  AND v.approved_version_number = v.version_number;
```

This contract must not weaken or duplicate that predicate.

### 3.2 Default deny

Anything not provably satisfying §3 is excluded. This includes, without
exception:

- drafts and unsubmitted edits;
- pending, rejected, or needs-changes revisions and proposals;
- `owner_private` profile values and any contact field;
- private photographs and personal documents that were not approved as family
  content;
- disputed, needs-review, unverified, or evidence-poor genealogy facts;
- removed, withdrawn, cancelled, or superseded content;
- content whose audience can no longer be established.

## 4. Presentation and indexing are different gates

Eligibility for presentation does **not** imply eligibility for indexing.

```
presentable(C, S, t) := eligible(C, S, t)
publishable(C, S, t) := presentable(C, S, t)
                      AND type_policy_permits_indexing(C)
                      AND the indexing pipeline completed for exactly this identity
```

`type_policy_permits_indexing` is an explicit allow-list. Its default is **no**:
a content type is not indexable because it is approved, because it is
family-visible, or because a similar type is indexable.

Recommended initial allow-list, for owner approval:

| Content type | Indexable | Rationale |
| --- | --- | --- |
| Approved document with `access_scope = family` | yes | already the `approved_documents` boundary |
| Approved family-visible biography or story | proposed | useful for `/ask`; must exclude contact fields |
| Approved family-visible display name | proposed | low risk, high utility for person search |
| Contact fields, address, phone, social links | **never** | private by product rule even when family-visible |
| Photographs and personal documents | **never in this slice** | media presentation is separate; no content extraction is approved |
| Genealogy facts | **not in this slice** | requires a separate provenance/citation policy, see §9 |

## 5. Publication identity

```
identity := (content_type, source_id, version, content_sha256)
```

| Content | `content_type` | `source_id` | `version` | `content_sha256` |
| --- | --- | --- | --- | --- |
| Profile value | `profile_value` | `person:{person_id}:field:{field_key}` | `approved_revision_number` | SHA-256 of the published text (UTF-8) |
| Profile media | `profile_media` | `media:{media_id}` | revision number | stored content hash |
| Genealogy fact | `genealogy_fact` | `assertion:{assertion_id}` | latest `assertion_review_event.event_id` | SHA-256 of the canonical rendered fact |
| Document | `document` | `document:{document_key}` | `approved_version_number` | `approved_sha256` |

Rules:

- The identity is computed **inside** the transaction that observes the
  authoritative state. A candidate built from a prior read is not trusted.
- One identity maps to at most one published item. An upsert keyed by identity
  is idempotent; a retry after a crash must converge, not duplicate.
- A new version for the same `source_id` supersedes the older identity. The
  older version must leave retrieval; it may not remain reachable by citation.
- `content_sha256` is the binding between what was approved and what is
  published. Content that changes without a new approval cannot keep an
  existing identity.

## 6. Lifecycle and state transitions

| Event | Presentation effect | Indexing effect | Audit |
| --- | --- | --- | --- |
| First approval | becomes presentable in the same transaction | candidate created; published only after the sink revalidates | approval event already recorded by the owning workflow |
| Approved replacement | new version presentable; prior version no longer presentable | upsert new identity, delete superseded identity | the owning workflow records the decision; publication records the identity it published |
| Visibility change `family → owner_private` / `admin_only` | **immediate** on commit of the change | delete the identity; block re-indexing | visibility change plus removal |
| Withdrawal of a pending submission | nothing was presentable | nothing was published | existing withdrawal audit |
| Removal / `removed_at` | immediate | delete the identity; a queued job for it must fail closed | removal event |
| Person or account deactivation | presentation follows the authorization check at read time | deletion is required if the content is no longer authorized for any audience | deactivation audit + publication removal |
| Person deletion | cascades authoritative rows; the item stops existing | deletion by identity; stale jobs must not resurrect it | cascade plus publication removal |
| Stale indexing attempt | no effect | rejected before any write; the job is marked failed, never "ready" | job failure with the observed state |
| Reapproval after a visibility change | presentable again | a new candidate is required; prior published identity is not reused | new approval; new publication record |

Two asymmetries are deliberate and must be stated in any implementation:

1. **Documents** can restrict scope on the existing version; the owning
   workflow resets the version to `pending_review` and clears the approval
   binding, so the `approved_documents` predicate fails immediately.
2. **Profile values** cannot restrict scope on an existing value; a visibility
   change is a *new* revision that must be approved. Until that approval
   commits, the previous family-visible value remains presentable. Whether an
   owner may downgrade visibility immediately without review is an open
   decision (§10, decision A).

## 7. Consistency, concurrency, and revocation

**Revocation fails closed.** After the API acknowledges a revocation, no
authorized family read and no retrieval may return the revoked version —
including reads served from a cache, a replica, or an in-flight answer.

Required properties:

- **Single source of truth.** The eligibility predicate reads the authoritative
  row. A cached copy may accelerate a read but may not decide it.
- **No trust in an earlier check.** `publish` re-evaluates §3 inside its own
  transaction; if any clause fails, it writes nothing and reports
  `NotEligible`. This closes the time-of-check/time-of-use gap between
  "approved and queued" and "processed".
- **Idempotent identity.** Publication and deletion are keyed by identity;
  retries converge. A deletion is complete only when a follow-up read confirms
  the identity is absent.
- **Bounded, explicit caches.** Any cache is keyed by identity and invalidated
  on the revocation path, with a maximum staleness that is documented and
  tested. An asynchronous queue is never the only mechanism: the authoritative
  state must already exclude the item before the queue runs.
- **Serialized conflicting writes.** Approval, visibility change, and removal
  take a row lock and re-check state, matching the existing profile-review,
  branch-review, and document-processing patterns.
- **Stale work is cancelled, not published.** Jobs carry the identity they were
  created for and revalidate it; a mismatch marks the job failed.
- **No partial publication.** A crash between "wrote chunks" and "recorded the
  identity" must be recoverable and must not leave an unapproved item
  retrievable.

## 8. Reuse map

This slice reuses the existing approval systems and adds no parallel workflow.

| Concern | Existing system | Contract use |
| --- | --- | --- |
| Profile value approval and visibility | `onboarding_profile_revision` / `onboarding_profile_value`, `read_approved_values(audience)` | authoritative revision, effective visibility, audience filtering |
| Profile media approval and access | `onboarding_media_metadata` + `read_authorized_media` | per-requester media presentation rule |
| Genealogy fact review | `assertion`, `assertion_evidence`, `assertion_review_event`, `src/genealogy/review.rs` | verified status, provenance, disputes |
| Branch-scoped corrections | `genealogy_fact_proposal` + branch review service | approved corrections land in `assertion` through the reviewed path |
| Document approval and processing | `document_version`, `document_processing_job`, `approved_documents` | publication identity and retrieval boundary |
| Retrieval | `src/storage_pg.rs` reads only `approved_documents` | the destination of an indexed candidate |
| Multi-family seam | `corpus` column and `corpus_grants` | candidates are corpus-scoped; no cross-corpus publication |

**Do not** add a second review workflow, a second visibility flag, or a new
publication database in the follow-up slices. A publication record, if one is
needed, is an index of identities and outcomes, not a source of truth.

## 9. Candidate adapter contract (design only)

The adapter turns eligible family content into candidates for the existing
pipeline. It is described as types so implementations can follow it without
inventing a new service:

```rust
pub enum ContentType { ProfileValue, ProfileMedia, GenealogyFact, Document }

pub struct PublicationIdentity {
    pub content_type: ContentType,
    pub source_id: String,
    pub version: i64,
    pub content_sha256: String,
}

pub struct EligibilitySnapshot {
    pub status: String,
    pub visibility: String,
    pub removed: bool,
    pub superseded: bool,
    pub observed_at: String,
}

pub struct PublicationCandidate {
    pub identity: PublicationIdentity,
    pub corpus: String,
    pub text: String,             // documents may carry a storage key instead
    pub citations: Vec<CitationRef>,
    pub eligibility: EligibilitySnapshot,
    pub actor: String,
}

pub trait CandidateSource {
    /// Runs in one transaction with the snapshot it returns.
    fn collect(&self, scope: &CorpusScope) -> Result<Vec<PublicationCandidate>, Error>;
}

pub trait PublicationSink {
    /// Revalidates the candidate inside its own transaction.
    fn publish(&self, candidate: &PublicationCandidate) -> Result<PublishOutcome, Error>;
}

pub enum PublishOutcome { Published, Superseded, NotEligible }
```

Obligations:

- `collect` never queries across corpora and never returns content whose
  audience is unproven.
- `publish` re-reads §3, rejects `NotEligible` without writing, and is
  idempotent by identity.
- The sink is the existing ingestion/retrieval boundary; it is not a new
  pipeline.
- Citation references are part of the candidate, so a published item can never
  cite evidence that was not part of the approved revision.
- Future multi-family isolation maps one family to one corpus. Nothing in this
  contract requires global identifiers; nothing may join across corpora.

## 9a. Implemented so far (USR-006-A)

The presentation gate for profile values is implemented:

- `GET /api/family/persons/{person_key}/profile` returns only approved,
  `family`-visible, non-media values. Membership (an active reviewed
  account-person link) is checked in the same transaction as the read.
- Proven by `tests/family_profile_presentation.rs` and
  `web/e2e/family-profile.spec.ts`.
- The endpoint contract, response states, and error mapping are documented in
  [guides/family-profile-presentation.md](../guides/family-profile-presentation.md).

Nothing in §4's indexing gate is implemented: no candidate adapter, no
publication record, and no retrieval write exist yet. Decisions **A** and **D**
below are still open; the endpoint does not implement either recommendation.

## 10. Decisions required from the owner

These are policy, not implementation detail. Until each is decided, the
fail-closed default below applies.

### A. Immediate owner-initiated restriction of a profile value

*Question:* may an owner make an approved family-visible value owner-private
immediately, without a new review?

- Option A1 — review required (current model). Simple and consistent, but a
  member who wants to withdraw a family-visible value must wait for review
  while it stays visible.
- Option A2 — restriction is immediate, expansion requires review. Matches the
  document rule and the principle that restriction never needs approval.
- **Default until decided:** keep A1 (no immediate change), and state the delay
  in the interface. Recommended: A2.

### B. BR-004 — cross-branch relationships

*Question:* a relationship touching two branches may require more than one
reviewer. What is the rule?

- Option B1 — hold and escalate. The proposal is accepted but cannot be
  approved by a single branch reviewer; an administrator decides. No automatic
  cross-branch grant.
- Option B2 — require an explicit grant for every affected branch. Strongest,
  but needs a reviewer who holds both.
- Option B3 — single-reviewer approval with a cross-branch flag. Fastest, but
  lets one branch decide a fact that belongs to another.
- **Recommended default:** B1. Security implication: a stuck queue is better
  than an unauthorized lineage change; the escalation path is explicit and
  audited. **Do not activate any option without owner approval.**

### C. Head of Family validity

*Question:* an appointment can outlive the evidence that justified it.

- Recommendation: validate eligibility at read time from current evidence; if
  any required fact is no longer `verified` (branch, living status, first-born),
  report the appointment as **review required** and suspend its *display* as an
  authoritative title, without revoking the record and without appointing a
  successor.
- Security implication: a lapsed appointment must never be usable as evidence
  for authority. Head of Family remains a genealogical status and confers no
  RBAC permission in any case.
- **Do not implement succession, automatic suspension, or replacement without
  owner approval.**

### D. Which content types may be indexed

Defined in §4. The allow-list is a decision, not a default: expanding it
requires an explicit change to this document and its tests.

## 11. Implementation status

This contract describes the whole boundary. Two related pieces of it are
implemented; nothing that publishes or indexes content is.

### Implemented (USR-006-A, USR-006-B)

- **Presentation of profile values**, read-only:
  `GET /api/family/persons/{person_key}/profile` returns approved,
  `family`-visible, non-media values to a caller holding an active reviewed
  account-person link. Membership and the value read share one transaction, and
  the read re-establishes the §3.1 provenance binding rather than trusting it,
  so a value whose approved revision is missing, unapproved, or inconsistent is
  excluded.
- **The family profile page**, `/family/{person_key}`. It renders that response
  for one person: approved display/preferred name, biography, personal story,
  and family memories, with provenance. It applies a narrower field allow-list
  than the API (contact and location fields are omitted until their policy is
  settled), keys the view by person so a same-route change cannot retain another
  person's data, and renders fixed text for every failure instead of
  server-supplied messages.
- Both are documented in
  [guides/family-profile-presentation.md](../guides/family-profile-presentation.md).

### Not implemented

- **No publication (indexing) endpoint, candidate adapter, publication record,
  or background job.** §4's second gate and §9's adapter are design only.
- **No content enters the retrieval corpus** through this contract.
  `approved_documents` remains the only retrieval source; decision **D**
  (indexing allow-list) is unresolved.
- **No indexing of profile values, biographies, stories, or media.**
- **No media presentation.** Photographs and personal documents remain
  owner/reviewer-only; there is no family-facing media surface.
- **No standalone story collections.** There is no browse-by-theme, timeline, or
  multi-person narrative view; only one person's approved fields are shown at a
  time.
- **No genealogy publication.** The verified family tree is unchanged and is not
  part of this contract's presentation surface.
- **No privacy mutation:** decision **A** (immediate owner restriction) is not
  implemented, and the BR-004 and Head of Family policies in §10 are untouched.
- **No change to ingestion, storage, workers, or retrieval.** Those remain
  Linux-owned.

A follow-up slice that adds indexing must satisfy §3–§7 and §9 in full before
any content becomes retrievable.
