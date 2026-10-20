# Head of Family

The Head of Family is a **genealogy designation**, not an application
administrator role. It grants no permission to manage users, documents, roles,
or security settings by itself. A user may be an administrator without being the
Head of Family, and the Head of Family may hold no application role beyond what
the RBAC matrix grants separately.

## Eligibility rule

Only a person who satisfies **all** of the following may hold an active Head of
Family designation:

1. The cataloged person belongs to the **third family branch** by a verified
   branch identifier. Branch 3 is not the same as generation 3.
2. The family record classifies the person as male. This is never inferred from
   a name.
3. The person is documented as the **first-born male** of the applicable
   third-branch lineage. Parentage, birth order, and conflicting accounts must be
   resolved from cited sources.
4. The person has a **verified living status** backed by recent, attributed
   attestation or authoritative evidence. `unknown`, `unverified`, `deceased`,
   and disputed status are not eligible.
5. The branch mapping and evidence have been reviewed. Uncertain translations or
   model-generated answers are not authoritative facts.

At most one active approved Head of Family exists for branch 3. The database
enforces one current appointment per branch.

## Evidence requirements

Every conclusion and appointment links to cited evidence by identifier. Each
evidence record carries:

- original source identity;
- page or section;
- translated passage when available;
- assertion type;
- provenance;
- reviewer and review timestamp.

Evidence status is `verified`, `unverified`, or `needs_review`. The browser shows
`Unverified` for missing evidence and `Needs review` for conflicting evidence. It
never invents a family head from a model answer.

## Approval workflow

- Editors may draft a proposal with citations.
- Only an active administrator may approve, create, revoke, or replace a
  designation.
- The approval API re-evaluates eligibility inside the same transaction that
  assigns the designation; a role or request field cannot override a failed
  check.
- Concurrent branch-3 approvals are serialized or rejected so two active
  appointments cannot be created.
- The browser requires an explicit confirmation before submitting a nomination,
  approval, or revocation. Cancelling sends no request.

## Succession rule (unresolved family policy)

The succession rule after the first-born male is **not implemented as a software
default**. If the first-born male is deceased, disputed, or no longer verifiably
living:

- do not silently appoint the next male child;
- suspend or revoke the active designation by a documented policy;
- require an explicit family-policy decision before any new appointment.

The application records the suspension/revocation and does not infer a successor.

## Current implementation state

The eligibility function, cited-evidence model, appointment table, one-active-
per-branch constraint, and admin approval workflow are implemented and covered by
tests. **No real Head of Family has been appointed.** The designation remains
unset until a verified third-branch candidate with complete evidence is approved
by an administrator. No public or production deployment occurs without OIDC,
TLS, secure sessions, and an administrative bootstrap.
