# Document publication integrity

This guide records the approved-only retrieval, citation-provenance, and
revocation behavior implemented for FAM-RAG-01. It complements
[`docs/architecture/FAMILY-PUBLICATION-CONTRACT.md`](../architecture/FAMILY-PUBLICATION-CONTRACT.md);
the contract's document row in §3.1 is unchanged.

## Authoritative retrieval predicate

Family retrieval reads only the existing `approved_documents` view
(`migrations/009_document_lifecycle.sql`):

```sql
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

Every user-facing path selects from this view and filters on `corpus`:

| Path | Read |
| --- | --- |
| `PgStore::retrieval_snapshot` | provenance snapshot used by `/api/query` |
| `PgStore::rows_for_corpus` | full-corpus reads and evaluation |
| `PgStore::chunks_for_corpus` | expansion corpus |
| `PgStore::vector_search` | pgvector similarity |
| `PgStore::keyword_search` | PostgreSQL full-text |
| `PgStore::section_chunks` | section expansion |

Reads that intentionally use the raw `documents` table are not family
retrieval: ingestion (`chunks_for_source`, `chunks_by_hash`), status/inventory,
mutation (`delete_by_source`), and `find_by_id` (used only by the storage
interop test). They never supply answer context.

Eligibility is enforced in SQL, before any content reaches the model. There is
no post-hoc filtering and no second retrieval store.

## Publication identity

A retrieved passage carries an authoritative identity observed at query time:

```
identity := (corpus, chunk_id, document_version_id, content_hash)
```

- `chunk_id` is `documents.id`.
- `document_version_id` is the version the chunk was published for.
- `content_hash` is the fingerprint bound to the approval.
- The exact `source` string (version-specific, composed by the lifecycle) and
  the display `section`/`page` remain available for rendering.

The identity travels from `approved_documents` through `Candidate.provenance`
and `ContextPassage.identity` into `Answer.citations` and the `/api/query`
`sources` objects, which now include `corpus`, `document_version_id`,
`chunk_id`, and `content_hash` alongside the existing
`source`/`section`/`page`/`text`.

## Citation resolution

`citation::resolve` accepts a model citation only when it names exactly one
retrieved passage:

1. **Exact source.** The stored source must equal the citation source, with the
   same section (normalized) and page.
2. **Normalized fallback.** Only when no exact match exists, the lifecycle
   suffix is stripped from both sides. This form is accepted only when exactly
   one passage matches.
3. **One publication, not one chunk.** The prompt exposes source, section, and
   page, so several chunks of the same source, section, and page are one
   citeable publication; the first matching chunk represents it and the
   citation resolves normally. This is the normal multi-chunk section case.
4. **Ambiguity fails closed.** Matches that differ in exact source, section, or
   page name more than one publication and return no resolution. The citation
   is stripped by repair, never attributed to the first passage.

Consequences:

- Two documents with the same filename but different document keys stay
  distinct; a keyed citation resolves to the intended version.
- A human-readable filename that names two different documents is rejected.
- A citation to a removed, restricted, or superseded row never resolves,
  because that row is absent from the retrieval snapshot.
- Repair removes unresolved citations and never substitutes another document.

The public citation format `[source - section]` / `[source - section - p.N]` is
unchanged.

## Revocation and supersession

| Event | Effect on the next authorized read |
| --- | --- |
| Removal (`removed_at`) | The version leaves `approved_documents`; snapshot, vector, keyword, and section reads exclude it immediately after commit. |
| Scope `family → admin_only` | The view requires `access_scope = 'family'`; the version leaves retrieval immediately. |
| Rejection / approval withdrawal | `review_status` leaves `approved`; retrieval excludes it and any queued job fails closed. |
| Supersession | The prior version is marked removed; only the current approved version's chunks remain in the view. Historical rows and chunks are preserved. |
| Stale processing | `publish_ready` re-checks approval, fingerprint, scope, version, and removal, and requires the current claim holder, so a stale or reclaimed job cannot republish. |
| Cross-corpus | Every read filters on the configured corpus; the same filename in another corpus is a different publication. |

No cache is trusted for authorization. There is no background cleanup
dependency: the authoritative row already fails the predicate.

## Final validation boundary

Retrieval takes a snapshot, then the model runs. `AnswerService` performs one
final revalidation after generation and citation repair:

- `PgStore::revalidate_publication(corpus, identities)` reads
  `approved_documents` once for the cited chunk ids and checks the version id
  too.
- If any passage is no longer published, the answer is replaced by the
  insufficient-evidence refusal with no text and no sources.
- An identity without a chunk id cannot be confirmed and fails closed.

No transaction or lock is held across the model call. A response already written
to the network cannot be recalled; this check is the last point at which the
server can still fail closed, and it is the documented limit of the guarantee.

## Test matrix

`tests/rag_publication.rs` executes the matrix with synthetic documents against
a disposable `*_test` database. Each test seeds a positive control and asserts it
is retrievable before asserting exclusions.

| ID | Scenario | Where |
| --- | --- | --- |
| RAG-01 | Pending submission excluded | `approved_documents_is_the_only_retrieval_boundary` |
| RAG-02 | Rejected submission excluded | same |
| RAG-03 | Approved but unprocessed excluded | same |
| RAG-04 | Approved ready family included (control) | same |
| RAG-05 | Approved `admin_only` excluded | same |
| RAG-06 | Removed document excluded | same |
| RAG-07 | `family → admin_only` excluded | `superseded_cross_corpus_and_revalidation_are_exact` |
| RAG-08 | Superseded version excluded | same |
| RAG-09 | Stale fingerprint / approval binding excluded | `approved_documents_is_the_only_retrieval_boundary` |
| RAG-10 | Same filename, different identities distinct | `citations_carry_version_identity_and_never_mix_documents` |
| RAG-11 | Same filename in two corpora never crosses | `superseded_cross_corpus_and_revalidation_are_exact` |
| RAG-12 | Ambiguous citation rejected | `citation::tests::ambiguous_normalized_citation_is_rejected`, `citations_carry_version_identity_and_never_mix_documents` |
| RAG-13 | Citation to a removed version invalid | `superseded_cross_corpus_and_revalidation_are_exact` |
| RAG-14 | Citation to the current approved version valid | `citations_carry_version_identity_and_never_mix_documents` |
| RAG-15 | Revocation during generation refuses the answer | `revocation_during_generation_refuses_the_answer` |
| RAG-16 | Stale/reclaimed job cannot republish | `tests/document_processing.rs::processing_is_durable_idempotent_and_approved_only` |
| RAG-17 | Vector, keyword, and section paths share restrictions | `approved_documents_is_the_only_retrieval_boundary` |
| RAG-18 | Query after restriction returns nothing | `superseded_cross_corpus_and_revalidation_are_exact` |

Run:

```sh
cargo test --locked --test rag_publication
cargo test --locked --test document_lifecycle
cargo test --locked --test document_processing
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
make web-check
```

Database-backed tests require `DATABASE_URL` pointing at a `*_test` database.
No test touches real family data, and none requires a live model.

## Known limitations and deferred work

- **Response already sent.** A revocation committed after the HTTP response is
  written cannot be recalled. The final revalidation window is bounded to the
  time between the last retrieval and the response.
- **Processing lease renewal.** A run longer than the worker lease can be
  reclaimed and reprocessed. Publication remains safe (`claimed_by`), but the
  work is repeated. Lease heartbeat is FAM-RAG-02.
- **Normalized fallback.** The human-readable filename fallback exists for
  model-generated citations. It is safe because it is accepted only when unique;
  it can still fail to resolve a citation whose filename is shared.
- **Single corpus per deployment.** `corpus` scoping is enforced on every read,
  but the running API serves one configured corpus. Multi-family tenancy remains
  out of scope.
