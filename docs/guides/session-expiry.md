# Session expiry and sign-in-again

This note explains the browser session lifetime, what happens when the ID token
expires before the cookie would, and why Tran asks you to sign in again instead
of refreshing the token automatically.

## Token lifetime versus cookie lifetime

Tran stores the Keycloak ID token inside the HttpOnly `rag_session` cookie. Two
different clocks matter:

| Lifetime | Owner | Meaning |
| --- | --- | --- |
| ID token `exp` | Keycloak | When the bearer credential stops being accepted by the Rust API |
| `rag_session` cookie `maxAge` | Next.js | When the browser drops the cookie |

The cookie lifetime is now derived from the token:

```
maxAge = min(token.exp - now, SESSION_MAX_SECONDS)
```

- `SESSION_MAX_SECONDS` is one hour.
- An already-expired token creates **no** session at all: the callback rejects it
  and redirects to `/signin?error=invalid_id_token`.
- The cookie is never allowed to outlive the token. A long-lived token cannot
  produce a session longer than the one-hour maximum. Fractional seconds are
  truncated, which can only shorten the lifetime, never extend it.

There is no refresh-token storage or automatic renewal in this change. When the
token expires, the session expires too.

## What happens on expiry

1. A protected request reaches the Next.js proxy and the Rust API answers `401`
   (`api: token rejected: token expired`).
2. The proxy returns that `401` unchanged and clears the stale `rag_session` and
   `rag_csrf` cookies on the same response.
3. `/api/auth/session` does the same when it resolves an expired session.
4. In the browser, the first `401` marks the session expired once. The account
   menu replaces the signed-in state with **“Session expired. Sign in again.”**
5. Further protected requests in that page stop being sent until you sign in.
   Concurrent failures produce one transition, not a storm of reloads.

Unsent form content is left on the page. Nothing is replayed automatically after
sign-in.

## What does not expire a session

Only `401` is an authentication result.

| Result | Meaning | Browser behavior |
| --- | --- | --- |
| `401` | Token missing, expired, or rejected | Clear cookies; show sign-in-again |
| `403` | Authenticated but not permitted | Keep the session; show the denial |
| `5xx` | Provider or service failure | Keep the session; show the error |
| Network error | API unreachable | Keep the session; show the error |

Bearer-token API clients that do not use the browser cookie still receive the
API's own status codes; the proxy only clears cookies when the request carried a
session cookie.

## Troubleshooting

**Repeated `api: token rejected: token expired` in the API log.** The browser
kept forwarding an expired token. Confirm the session menu shows the
sign-in-again action; if it does, the stale cookie has been cleared and the
message should stop. If it repeats while the menu still shows a signed-in
account, check that the browser is actually receiving the `Set-Cookie` headers
from `/api/auth/session` or `/api/*` (a proxy in front of Next.js must not strip
them).

**Signed out unexpectedly.** Check whether the failure was `401` or something
else. `403`, `5xx`, and network errors keep the session. If a fresh login does
not restore access, the Rust API may be rejecting the token for a different
reason (issuer, audience, signature, or account provisioning), which is outside
this flow.

**Cookie lifetime looks shorter than expected.** That is correct: the cookie
lives only as long as the token's remaining lifetime, capped at one hour.

## Limitations

- Automatic token refresh (refresh-token storage and renewal) is outside this
  change. Expiry always ends in a sign-in-again action.
- The client keeps unsent form input where the page keeps it in component state,
  but a full sign-in navigation may still clear a page; writes are never replayed
  automatically.
