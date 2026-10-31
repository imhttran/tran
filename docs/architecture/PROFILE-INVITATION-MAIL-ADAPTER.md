# Profile Invitation — HTTP Mail Adapter Contract

The invitation flow can deliver through a generic HTTP endpoint. This document
defines exactly what that endpoint must accept, the delivery semantics the
application claims, and whether an existing configured provider in this
repository is compatible.

## Configuration

| Variable                       | Required | Meaning                                                                                      |
| ------------------------------ | -------- | -------------------------------------------------------------------------------------------- |
| `ONBOARDING_MAIL_ADAPTER`      | yes      | Must be `http` to enable the adapter.                                                        |
| `ONBOARDING_MAIL_URL`          | yes      | The provider endpoint. `https://` is required; plain `http://` is accepted only on loopback. |
| `ONBOARDING_MAIL_BEARER`       | no       | Sent as `Authorization: Bearer <value>`. Never logged or returned.                           |
| `ONBOARDING_MAIL_TIMEOUT_SECS` | no       | Request timeout in seconds. Default `10`.                                                    |
| `ONBOARDING_ACTIVATION_URL`    | no       | Base URL for the activation link inside the message body.                                    |

`fake` is synthetic and performs no network I/O. `unconfigured`, an unknown name,
or `http` with a missing URL all fail closed: no invitation is created and no
delivery is claimed.

## Request

```
POST <ONBOARDING_MAIL_URL>
Content-Type: application/json
Authorization: Bearer <ONBOARDING_MAIL_BEARER>   (only when configured)

{
  "to": "<verified recipient address>",
  "subject": "<message subject>",
  "body": "<plain-text body containing the one-time activation link>"
}
```

- Redirects are **not** followed. A 3xx is a provider failure.
- The endpoint must accept an arbitrary recipient address and a plain-text body.
  It must not require a template id, a provider-specific envelope, or a
  pre-registered recipient.
- The body is the complete message. The activation link is
  `{ONBOARDING_ACTIVATION_URL}/{token}` (trailing slashes on the base are
  trimmed), which is the path route `/activate/[token]`. The token is the only
  secret in it; it is never sent in a header, a URL query, or a log.

## Response

| Provider answer              | Recorded state                            | Meaning                                                                                                       |
| ---------------------------- | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Any `2xx`                    | `sent`                                    | The provider accepted the message. The optional JSON field `message_id` is stored as the provider message id. |
| Any non-`2xx`                | `failed`, code `provider_status_<number>` | The provider refused the message.                                                                             |
| Connection failure / timeout | `failed`, code `provider_unreachable`     | No answer.                                                                                                    |

A `2xx` means **provider acceptance only**. The application never claims inbox
delivery, a bounce result, or an open. A failure leaves the invitation pending and
recoverable; resending revokes the previous link and issues exactly one new one.

## Compatibility of existing providers

| Provider in this repository                   | Contract                                                                                | Compatible?                                                       |
| --------------------------------------------- | --------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| `PASSWORD_RESET_URL` / `PASSWORD_RESET_TOKEN` | Provider-managed reset: receives `{"issuer","subject"}` and resolves the address itself | **No.** It does not accept a recipient address or a message body. |
| `ONBOARDING_MAIL_URL`                         | Unset in this repository                                                                | **Not configured.**                                               |

No other outbound mail provider is configured (`.env` contains no SMTP, REST mail,
or webhook mail endpoint). A generic endpoint is not automatically compatible with
an email provider: SendGrid, Postmark, SES, Resend, and similar services use their
own JSON schemas and authentication. To use one, point `ONBOARDING_MAIL_URL` at a
small gateway that translates the request above into that provider's API, or add a
provider-specific adapter to `src/onboarding/delivery.rs`.

## Required configuration for real delivery

```sh
ONBOARDING_MAIL_ADAPTER=http
ONBOARDING_MAIL_URL=https://<provider-or-gateway>/<path>
ONBOARDING_MAIL_BEARER=<provider credential>   # optional
ONBOARDING_ACTIVATION_URL=https://<public-app-host>/activate
```

Until these are set, invitations can be prepared and their delivery state
inspected, but a send fails closed with `delivery_not_configured` and nothing is
created.
