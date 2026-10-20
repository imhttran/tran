# Onboarding identity mapping

USR-003C links an authenticated OIDC identity to exactly one genealogy person. The
link spans two databases, so the mapping is explicit and one-directional.

## Two authorities

| Database | Env var | Owns |
| --- | --- | --- |
| Identity | `DATABASE_URL` | `users.id`, immutable `(issuer, subject)`, roles, grants, deactivation, audit |
| Genealogy | `GENEALOGY_DATABASE_URL` | `person`, assertions, evidence, approvals, and `onboarding_*` tables |

There is **no cross-database foreign key**. The genealogy link stores:

- `account_id` — the opaque identity `users.id`;
- `identity_fingerprint` — lowercase hex SHA-256 of `issuer + "\n" + subject`.

It never stores a raw issuer, subject, email, token, or display name. The fingerprint is
one-way and only disambiguates audit records; it is not an authentication credential.

## Resolution order

1. The API already verified the OIDC token and loaded roles from `(issuer, subject)`.
2. `onboarding::resolve_account` reads the identity database by the **exact pair** and
   rejects:
   - blank subject (`InvalidIdentity`);
   - no row (`UnknownIdentity`);
   - `deactivated_at IS NOT NULL` (`DeactivatedIdentity`);
   - a subject present under more than one issuer when the pair does not resolve
     (`AmbiguousIdentity`). A subject is never matched on its own.
3. Only then does `onboarding::link_reviewed` write the genealogy link in one transaction:
   the link, the `linked` audit row, or a `rejected` audit row.

A failure between step 2 and step 3 leaves no link and is safe to retry. Because the link
carries the opaque account id and fingerprint, the identity database remains the only place
that can resolve a person back to an authentication identity.

## Cardinality and review

- One active link per person and one per account (schema uniqueness).
- A conflict returns `AlreadyLinked`; records are never silently merged or replaced.
- A link requires an explicit reviewer decision. Rejection writes only an audit row.
- Revocation deletes the link and appends a `revoked` audit row in the same transaction.
- A genealogical Head of Family title grants no link or admin permission; it is separate
  from application authorization.

## Privacy

The link audit table (`onboarding_link_audit`) stores `account_id`, the fingerprint, the
decision, the reviewer, and a timestamp. No row contains a raw identity claim, token, source
excerpt, or display name. Logs and API errors name a rule, never a person.
