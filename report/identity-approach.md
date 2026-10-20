# Phase 0 Application Identity Approach

The application uses standards-based OIDC. The Rust side verifies each token. No
client or browser decides identity.

## OIDC verification on the Rust side

The Rust API verifies every bearer token before a handler reads data. Verification
checks all of the following:

- **Issuer**: the token `iss` claim equals the configured OIDC issuer.
- **Audience**: the token `aud` claim contains the configured audience.
- **Signature**: the signature verifies against the issuer's published keys.
- **JWKS**: keys come from the issuer's JWKS endpoint and are cached with
  refresh on unknown `kid`.
- **Expiry**: the token is not expired (`exp`) and is not used before `nbf`.
  Clock skew is bounded.
- **Subject**: the token carries a non-empty, stable `sub` claim.

A token that fails any check is rejected. There is no partial-trust path.

## Account binding by immutable provider subject

Accounts bind to the immutable provider subject (`sub`), not to email or display
name. Email and name can change; `sub` cannot. A first login creates the local
account keyed by `(issuer, sub)`. Later logins resolve the same account from the
same `(issuer, sub)` pair.

## Isolated, cryptographically valid test identities

Tests do not use a production identity provider. The test suite mints tokens
signed by an isolated test key and serves a matching JWKS. The tokens are
cryptographically valid, so they pass the same verification code path as
production tokens. Test identities are isolated to the test environment and are
not production secrets.

## No auth-bypass switch

There is no configuration flag, header, or query parameter that disables
authentication. Verification always runs. Operators supply production OIDC
issuer, audience, and TLS settings through their environment; these settings are
not committed to the repository, and no public deploy occurs without them.

## Status

OIDC settings are not yet configured in the repository. Phase 2 implements the
Rust-side OIDC adapter against configured issuer, audience, and JWKS. Production
OIDC settings and TLS remain an operator-supplied gate.
