# Session expiry, renewal, and sign-in-again

This note explains the browser session lifetime, automatic renewal, what happens
when renewal is unavailable, and why an expired session ends in a clear
sign-in-again action.

## Token lifetime versus cookie lifetime

Tran stores the Keycloak ID token inside the HttpOnly `rag_session` cookie. Two
different clocks matter:

| Lifetime | Owner | Meaning |
| --- | --- | --- |
| ID token `exp` | Keycloak | When the bearer credential stops being accepted by the Rust API |
| `rag_session` cookie `maxAge` | Next.js | When the browser drops the cookie |
| `browser_session` idle / absolute | Rust session store | How long an inactive session may be renewed |

The cookie lifetime is derived from the token:

```
maxAge = min(token.exp - now, SESSION_MAX_SECONDS)
```

- `SESSION_MAX_SECONDS` is one hour.
- An already-expired token creates **no** session.
- The cookie can never outlive the token. Fractional seconds are truncated,
  which can only shorten the lifetime, never extend it.

## Automatic renewal (optional)

When the session store is configured, the Next.js callback also stores the
provider's refresh token in the Rust API and sets an opaque, HttpOnly `rag_sid`
cookie. The refresh token never reaches the browser and is never logged.

On each protected request and on `/api/auth/session`, Next.js checks the ID
token:

- more than `SESSION_REFRESH_WINDOW_SECONDS` (default 60) before expiry: the
  request continues with the current token;
- inside the window: the Rust API refreshes the token with the provider, rotates
  the refresh token, validates the new ID token, and returns only the new ID
  token and its lifetime.

A successful renewal replaces `rag_session`, refreshes the `rag_sid` lifetime up
to the absolute limit, and restores `rag_csrf` when the previous CSRF cookie has
expired. The new CSRF value is delivered to the browser so the next write passes
the double-submit check; a write is never replayed automatically.

### Concurrency

Renewal is coalesced in PostgreSQL:

- a claimant increments `refresh_generation` and takes a 45-second
  `renewal_lease_until`, long enough to cover the provider timeout and its
  bounded retries;
- only that generation may write the rotated credentials, and only that
  generation may revoke the session on `invalid_grant`, so a stale claim cannot
  undo a newer renewal;
- a second request gets `409 renewal_in_progress`, retries briefly, then uses the
  now-fresh stored token (the provider is called once).

Multiple tabs and Next.js workers therefore do not race the provider or
overwrite newer credentials.

### Limits

| Setting | Default | Meaning |
| --- | --- | --- |
| `SESSION_IDLE_SECONDS` | 1800 | An inactive session expires after 30 minutes (minimum 120) |
| `SESSION_ABSOLUTE_SECONDS` | 28800 | A session expires after 8 hours regardless of activity |
| `SESSION_REFRESH_WINDOW_SECONDS` | 60 | Renewal starts this many seconds before expiry |

The server enforces both limits; the browser cannot extend them. Idle time is
refreshed at most every 30 seconds per session.

## Configuration

Both the Rust API and the Next.js server read the same variables.

| Variable | Required for renewal | Meaning |
| --- | --- | --- |
| `WEB_SESSION_SECRET` | yes | Shared secret for the server-to-server session endpoints |
| `SESSION_STORE_KEY` | yes | AES-256-GCM key, 32 bytes, base64 or base64url |
| `OIDC_TOKEN_ENDPOINT` | yes | Provider token endpoint used for the refresh grant |
| `OIDC_CLIENT_ID` | yes | Registered client id (falls back to `AUTH_AUDIENCE`) |
| `OIDC_CLIENT_SECRET` | for confidential clients | Server-side only |
| `SESSION_IDLE_SECONDS` | no | Idle limit, default 1800 |
| `SESSION_ABSOLUTE_SECONDS` | no | Absolute limit, default 28800 |
| `SESSION_REFRESH_WINDOW_SECONDS` | no | Refresh window, default 60 |
| `OIDC_REVOCATION_ENDPOINT` | no | Provider token-revocation endpoint used on sign-out |

`SESSION_STORE_KEY` must decode to exactly 32 bytes; a malformed key fails
startup rather than silently storing tokens unencrypted.

### Keycloak prerequisites

- The browser client must use the standard authorization-code flow.
- **Use Refresh Tokens** must be enabled for the client (it is on by default).
- The token endpoint must return `refresh_token` for the authorization-code
  grant and for `grant_type=refresh_token`.
- A confidential client (`OIDC_CLIENT_SECRET`) is recommended.
- If the client cannot issue refresh tokens, leave the renewal variables unset.
  The app then keeps the cookie-only session from PR #65: it expires with the
  token and shows the sign-in-again action. There is no insecure fallback.

## Storage and cleanup

Migration `011_browser_sessions.sql` adds `browser_session` to the identity
database:

- `session_id_hash` — SHA-256 of the opaque cookie value; the cookie value itself
  is never stored;
- `id_token` / `refresh_token` — AES-256-GCM ciphertext with a random nonce and
  the session hash as additional authenticated data;
- `refresh_generation` / `renewal_lease_until` — renewal coalescing;
- `idle_expires_at` / `absolute_expires_at` / `revoked_at`.

Expired and long-revoked rows are removed opportunistically in bounded batches;
rows are retained for seven days after they become unusable. A database dump
contains ciphertext, not usable refresh credentials.

## What happens on expiry or revocation

1. If the refresh grant is rejected (`invalid_grant`), the session was revoked, or
   the account was deactivated, the Rust store revokes the row.
2. The proxy returns JSON `401`, clears `rag_session`, `rag_sid`, and `rag_csrf`,
   and the account menu shows **“Session expired. Sign in again.”**
3. Protected calls stop until sign-in; unsent form content is left in place and
   nothing is replayed.

## What does not expire a session

Only a definitive `401`/expired result does.

| Result | Meaning | Browser behavior |
| --- | --- | --- |
| `401` | Token or refresh grant rejected, revoked, or expired | Clear cookies; sign-in-again |
| `403` | Authenticated but not permitted | Keep the session; show the denial |
| `502`/`503` | Provider or API failure | Keep the session; show a service error |
| Network error | API unreachable | Keep the session; show the error |

Sign-out calls the store's revoke endpoint (and the provider's revocation
endpoint when configured) and clears all three cookies.

## Troubleshooting

**Repeated `api: token rejected: token expired`.** If renewal is configured,
check that the Keycloak client returns a refresh token and that
`OIDC_TOKEN_ENDPOINT`/`OIDC_CLIENT_ID` are correct. If renewal is not configured,
the session intentionally expires with the token; the menu should show the
sign-in-again action.

**Everyone is signed out after a deploy.** Confirm the same `SESSION_STORE_KEY`
and `WEB_SESSION_SECRET` are set for both processes. A key change makes stored
sessions undecryptable, which fails closed into sign-in-again.

**Renewal returns 503 during an identity-provider outage.** That is intentional:
the session is kept and the next request retries. A `503` is not a permission
denial and not an expiry.

**A session ends before the absolute limit.** Check `SESSION_IDLE_SECONDS`; an
inactive user is signed out after the idle limit.

## Limitations

- The refresh token is encrypted with a single application key. Key rotation is
  a deployment task (re-authenticate sessions after changing the key).
- `offline_access` is not requested; renewal requires the user's normal refresh
  token and cannot run after the provider session ends.
- Renewal happens on request boundaries. A completely idle browser is signed out
  at the next request after the idle limit.
- Access tokens are not stored or refreshed; only the ID token is forwarded.
