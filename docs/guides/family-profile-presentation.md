# Family profile presentation (USR-006-A)

`GET /api/family/persons/{person_key}/profile` returns another member's
**currently approved, family-visible** profile values.

It is the first implementation of the presentation half of
[`docs/architecture/FAMILY-PUBLICATION-CONTRACT.md`](../architecture/FAMILY-PUBLICATION-CONTRACT.md).
It adds no storage, no approval workflow, and no indexing.

## Authorization

The Rust API is the only authority.

1. A verified OIDC identity is required. Anonymous requests get `401`.
2. The account must be active. A deactivated identity is refused even while its
   person link still exists.
3. The caller must hold an **active reviewed account-person link**. Membership
   is the family-audience permission; a genealogy relationship, branch
   membership, or Head of Family title grants nothing by itself.
4. The link check and the value read run in **one transaction** with the link
   row locked `FOR SHARE`, so a concurrent revocation serializes with the read.
5. No client-supplied identity, role, visibility, or authorization flag is read.

The operation (`Operation::FamilyProfileRead`) has no role gate, matching the
existing self-service reads: a newly linked member may not hold a role yet. The
reviewed link is the authorization rule.

### Corpus grants

Corpus grants govern RAG retrieval. This endpoint reads the genealogy database
and never touches retrieval, so no corpus grant applies. The only authorization
inputs are the verified identity, the active account, and the reviewed link.

## Response

`200` with one of two states:

```json
{
  "ok": true,
  "state": "published",
  "person": { "person_key": "…", "display_name": "…" },
  "fields": [
    {
      "field_key": "biography",
      "value": "…",
      "visibility": "family",
      "approved_revision_number": 3,
      "updated_at": "2026-10-22T12:00:00Z"
    }
  ]
}
```

- `state: "empty"` means the caller is authorized and the person exists, but
  nothing is approved for the family audience. `fields` is `[]`. The response
  does not reveal whether owner-private content exists.
- `person.display_name` is the shared genealogy record name, already visible in
  the family tree; it is not a private profile value.
- `approved_revision_number` and `updated_at` are provenance. Reviewer
  identities and internal fingerprints are never returned.

### Errors

| Status | Body `error` | Meaning |
| --- | --- | --- |
| 401 | `unauthenticated` | No verified identity |
| 403 | `family_profile_forbidden` | No active reviewed link. Returned **before** the person is looked up, so it cannot be used to probe which keys exist |
| 404 | `family_profile_not_found` | Linked member, unknown person key |
| 503 | `family_profile_unavailable` | Genealogy store unavailable |

Error bodies are fixed strings: they never echo a person key, a value, or a
private marker.

## What is excluded

Always, without exception:

- drafts, pending revisions, rejected or needs-changes revisions;
- `owner_private` values, including contact details the owner kept private;
- the `profile_photo` and `personal_document` reference fields — media has its
  own authorization rule and is not served here;
- superseded values (only the value row bound to the current approved revision
  is read);
- any content the caller is not authorized to see.

## What is not in this slice

- No write path, no visibility change, no withdrawal. Policy decision **A**
  (immediate owner restriction) remains open.
- No media presentation and no genealogy fact publication.
- No indexing. Policy decision **D** (indexing allow-list) remains open; this
  endpoint creates no retrieval rows.
- No family profile UI page. The response is designed so the family tree and
  profile pages can consume it later.

## Verification

- `tests/family_profile_presentation.rs` — six service tests: family-only
  values, owner-private and media exclusion, drafts excluded, unknown person,
  revoked link, authorized empty profile, owner-private view unchanged, and no
  retrieval rows created.
- `web/e2e/family-profile.spec.ts` — three browser tests through the real proxy:
  published profile, empty/unknown/unauthorized distinctions, and immediate
  refusal after link revocation and account deactivation.
- `docs/guides/family-publication-acceptance.md` — the matrix this slice
  satisfies row 5 of, and F1.
