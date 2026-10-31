# Keycloak sign-ins for invitations

Realm self-registration stays off. With `ONBOARDING_KEYCLOAK_LOGINS` on, the API
creates each invitee's Keycloak login when an administrator sends the
invitation, so the invitee never needs an operator to make one by hand.

## What the invitee gets

| Situation when you send | What happens | Email the person receives |
| --- | --- | --- |
| No Keycloak login for the address | The API creates one: the reserved username, the verified address, `emailVerified=true`, no password | One email from Keycloak: "choose your password". After they set it, Keycloak sends them to their `/activate/<token>` link, already signed in. |
| A login exists with the reserved username and the same address, no password yet (an earlier send) | Reused | The same Keycloak email again |
| A login holds the address under another username | Reused. They sign in with that username | Same as above if it has no password; otherwise the ordinary invitation email |
| A login with a password already exists | Reused. The password is never reset | The ordinary invitation email through `ONBOARDING_MAIL_ADAPTER` |

After that the flow is unchanged: the first sign-in creates the application
account, accepting the invitation asks for a person link, and an administrator
approves the link before the archive opens.

The API never sets, generates, reads, or logs a password. It never shows one to
you either.

## Refusals

A send that can't prepare a sign-in fails for that person only, records a safe
code, and can be retried once the cause is fixed.

| Code | Meaning | Fix |
| --- | --- | --- |
| `login_username_taken` | The reserved username belongs to a login with a different address | Change the username before preparing, or correct the login in Keycloak |
| `login_ambiguous` | Two or more logins use the address | Remove the duplicate in Keycloak |
| `login_disabled` | The matching login is disabled | Enable it in Keycloak |
| `login_username_unavailable` | No username could be reserved | Check the person's verified name parts |
| `login_redirect_rejected` | Keycloak refused the activation URL | Add it to the web client's valid redirect URIs (step 3) |
| `login_email_failed` | Keycloak's own SMTP failed | Fix realm email settings (step 4) |
| `login_provider_unauthorized` | Token request or admin call refused | Check the client secret and service-account roles (steps 1–2) |
| `login_provider_unavailable` / `login_provider_failed` | Keycloak unreachable or errored | Check `KEYCLOAK_URL` and Keycloak health |

## Keycloak setup (realm `tran`)

1. **Service-account client.** Clients → Create client. Client ID
   `tran-invitations`, Client authentication **on**, Service accounts roles
   **on**, Standard flow **off**, Direct access grants **off**. Copy the secret
   from the Credentials tab.
2. **Least-privilege roles.** On that client, Service account roles → Assign role
   → filter by client `realm-management` → assign `manage-users`, `view-users`,
   and `query-users` only. Don't assign `realm-admin`.
3. **Redirect back to the archive.** On the browser client (`tran-web`), add the
   activation URL pattern to Valid redirect URIs, for example
   `https://archive.example.test/activate/*`.
4. **Realm email.** Realm settings → Email: set From and the SMTP host. Locally,
   point it at Mailpit (`127.0.0.1`, port `1025`, no TLS). Keycloak sends the
   password email itself, so this is separate from the app's `ONBOARDING_SMTP_*`.
5. Optional: Realm settings → Tokens → *Email action lifespan* is overridden per
   message by `KEYCLOAK_ACTIONS_LIFESPAN_SECS`.

## API configuration

| Variable | Purpose |
| --- | --- |
| `ONBOARDING_KEYCLOAK_LOGINS` | `true` to create sign-ins on send. Default off. |
| `KEYCLOAK_URL` | Keycloak **base** URL. `https://`, or `http://` on loopback. Never the realm URL. |
| `KEYCLOAK_REALM` | Realm. `AUTH_ISSUER` must equal `KEYCLOAK_URL/realms/KEYCLOAK_REALM`, or the API refuses to start. |
| `KEYCLOAK_LOGIN_CLIENT_ID` | The service-account client from step 1. |
| `KEYCLOAK_LOGIN_CLIENT_SECRET` | Its secret. Server-side only; never logged. |
| `KEYCLOAK_ACTIONS_CLIENT_ID` | The browser client from step 3 (`tran-web`). |
| `KEYCLOAK_ACTIONS_LIFESPAN_SECS` | Optional. Password-link lifetime; default 7 days, the invitation's own lifetime. |
| `ONBOARDING_ACTIVATION_URL` | Required and absolute when this is on, for example `https://archive.example.test/activate`. |

`ONBOARDING_MAIL_ADAPTER` still delivers the ordinary invitation to people who
already have a password. Leave it `unconfigured` if every invitee is new; such a
send then fails closed only for a person with an existing password.

## Verification

- Unit tests in `src/onboarding/keycloak_logins.rs` drive a scripted Keycloak
  stand-in through creation, reuse, refusals, and error mapping.
- `tests/profile_invitation.rs` and `tests/onboarding_bulk.rs` prove both send
  paths hand the adapter the reserved username and link, and that a plain mail
  adapter reserves nothing.
- Not yet run against a live Keycloak. Before enabling it for real family
  members, run one synthetic invitation end to end against the `tran` realm and
  Mailpit.
