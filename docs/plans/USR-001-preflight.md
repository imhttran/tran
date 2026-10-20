# USR-001 Preflight Note — Identity Security Prerequisite

## Baseline

- Branch: `sop/plan-tran-user-onboarding`.
- Baseline commit at recording time: `dc5a4ab` (`docs: use SOP-authorized
  validation commands in the onboarding plan`).
- Toolchain: pinned by `rust-toolchain.toml`; acceptance uses `cargo test
  --locked` and `cargo fmt --check`.
- Migration runner: identity migrations are applied as ordered SQL files
  `migrations/006_identity.sql`, `007_user_lifecycle.sql`, and
  `008_identity_issuer_subject.sql`. Tests apply them to a disposable database
  derived from `DATABASE_URL`.

## Open work (recorded, not duplicated here)

- PR #16 (read-only family tree) is open.
- PR #17 (this user-onboarding plan) is open.
- No profile/asset engine has been merged. This prerequisite work must not
  duplicate the family tree (#16) or the onboarding plan (#17).

## Prerequisite identity fix

The execution baseline had `users.subject` globally unique and `provision_user`
rewrote `issuer` on conflict. That let a subject string from another issuer
inherit, reactivate, or take over an existing account. The fix makes identity
immutable on the pair `(issuer, subject)`:

1. **Unique `(issuer, subject)`** — migration
   `008_identity_issuer_subject.sql` drops the global `users_subject_key`
   constraint, adds `users_issuer_subject_key UNIQUE (issuer, subject)`, and
   replaces the active subject-only index with an active
   `(issuer, subject)` index. The migration is additive: it does not modify,
   delete, or truncate any `users`, `role_assignments`, `corpus_grants`, or
   `audit_events` row, and it drops no table.

2. **`provision_user` without issuer rewrite** — `provision_user` inserts with
   `ON CONFLICT (issuer, subject) DO UPDATE SET deactivated_at = NULL,
   updated_at = now()`. The `DO UPDATE` clause never assigns `issuer`, so a
   provision call for issuer B cannot reactivate or rewrite an account under
   issuer A. Only the matching `(issuer, subject)` row is reactivated.

3. **Audit disambiguation** — `audit_events.subject` stays a plain column, but
   every audit write records `detail.issuer` through
   `jsonb_build_object('issuer', $4::text)`. Two accounts that share a subject
   string are disambiguated by `detail.issuer`. Every state-changing path
   (`provision`, `role grant/revoke`, `corpus grant/revoke`, `deactivate`,
   `bootstrap`) resolves the user by `WHERE subject = $1 AND issuer = $2` and
   writes the audit row with the same issuer in the same transaction.

## Guards preserved

- `ensure_not_last_admin` still takes
  `pg_advisory_xact_lock(hashtext('role:admin')::bigint)` and returns
  `IdentityStoreError::LastAdmin` when at most one active administrator
  remains. Both `change_role` (admin revoke) and `deactivate_user` call it.
- Audit rows are still written in the same transaction as the change they
  describe.

## Synthetic tests

`tests/identity_cross_issuer.rs` covers, against a disposable database:

- provision for issuer B does not modify or reactivate issuer A's account;
- deactivation for issuer B fails with `UnknownUser` and leaves issuer A active;
- two accounts with the same subject string keep separate roles, corpus grants,
  audit history, and deactivation state;
- cross-issuer role and corpus grants fail with `UnknownUser`;
- the last-active-admin guard still holds across issuers.
