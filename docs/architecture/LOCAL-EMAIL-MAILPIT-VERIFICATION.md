# Local email with Mailpit — verification evidence

Status: MAIL-005 verification note
Plan: `docs/plans/PLAN-TRAN-LOCAL-EMAIL-MAILPIT.md`

This note records the local verification of the Mailpit integration, the SMTP
invitation adapter, local Keycloak SMTP, and one captured-email invitation path.
All identities, addresses, and the person record are synthetic (`example.test`
and `SOP-MAILPIT-001`). No real invitation was sent and no raw invitation URL or
token is recorded here.

## Environment

- API on the host: `127.0.0.1:8080`.
- UI on the host: `127.0.0.1:3000`.
- PostgreSQL in Docker: `127.0.0.1:5435` for this local stack. The repository
  Compose file maps `5434:5432`; set `DATABASE_URL` and
  `GENEALOGY_DATABASE_URL` to the port the running stack publishes.
- Mailpit: `axllent/mailpit:v1.21`, `127.0.0.1:1025` (SMTP) and
  `127.0.0.1:8025` (browser/API), `MP_DATABASE=/data/mailpit.db` on the
  `mailpit_data` volume.
- Keycloak: `http://127.0.0.1:8180`, realm `tran`.

The API was started with:

```text
ONBOARDING_MAIL_ADAPTER=smtp
ONBOARDING_SMTP_HOST=127.0.0.1
ONBOARDING_SMTP_PORT=1025
ONBOARDING_SMTP_TLS=none
ONBOARDING_SMTP_FROM=tran@example.test
ONBOARDING_SMTP_TIMEOUT_SECS=10
ONBOARDING_ACTIVATION_URL=http://127.0.0.1:3000/activate
```

Plaintext SMTP is used only for the loopback Mailpit sink. The adapter applies
the configured TLS mode exactly and never falls back after a TLS failure.

## Deterministic checks

```text
cargo build --locked                                              PASS
cargo test --locked (disposable test DSNs)                       PASS
cargo fmt --check                                                PASS
cargo clippy --locked --all-targets -- -D warnings               PASS
```

The disposable DSNs were obtained from `scripts/test-db.sh env`:

```text
DATABASE_URL=postgres://.../rag_test?sslmode=disable
GENEALOGY_DATABASE_URL=postgres://.../tran_genealogy_test?sslmode=disable
```

The live database guard and the `tran_genealogy`/`rag` names are unchanged.
Applying the additive genealogy migrations 0033–0035 to the local development
database was required because the invitation contact, delivery, and link-request
tables did not yet exist there; no database was reset or recreated.

## Mailpit service

- `docker compose --profile mail config` validates.
- `docker compose --profile mail up -d mailpit` reports the container
  `Up (healthy)`.
- `GET http://127.0.0.1:8025/api/v1/info` returns HTTP 200 and
  `"Database": "/data/mailpit.db"`.
- A synthetic SMTP message sent to `127.0.0.1:1025` appears in
  `GET /api/v1/messages`.
- The message is still present after `stop`/`start` and after
  `rm -f`/`up -d`, proving the named-volume retention.
- The published ports are loopback-only; no relay or forwarding is configured.

## Keycloak

- Realm `tran` has `registrationAllowed=false` before and after the change.
- `scripts/keycloak-mailpit-smtp.sh --show` reports
  `smtpServer.host=host.docker.internal`, `port=1025`, `from=tran@example.test`,
  `auth=false`, `starttls=false`, `ssl=false`.
- A temporary Keycloak identity requested a password-reset action through the
  Admin API. Mailpit captured the message "Update Your Account" for the
  synthetic `mailpit-probe@example.test` address.
- The previous realm SMTP map was empty; the rollback baseline is
  `.run/keycloak-smtp-before.json`, and
  `scripts/keycloak-mailpit-smtp.sh --revert` restores it.

## Captured-email invitation flow

1. A synthetic living person `SOP-MAILPIT-001` was added to the development
   genealogy database with one verified, cited `vital_status=living`
   assertion.
2. A synthetic application administrator and a synthetic invitee were created in
   the `tran` realm. The invitee had no Tran link or archive grant.
3. The administrator saved the synthetic contact, prepared the invitation, and
   sent it through the SMTP adapter. The API reported
   `provider="smtp"`, `delivered=true`, `delivery_state="sent"`.
4. Mailpit captured a message with subject "Activate your family archive
   account" for the synthetic invitee address. The message body contained the
   exact path-form URL `http://127.0.0.1:3000/activate/<token>`.
5. The invitee opened that exact URL, signed in through Keycloak, and the
   invitation survived the OIDC round trip.
6. Before acceptance and before approval, `/api/family/photographs`,
   `/api/genealogy/person/SOP-MAILPIT-001`, and
   `/api/admin/family-members` all returned HTTP 403.
7. Accepting the invitation produced the "Pending review" page. Protected
   archive access still returned HTTP 403.
8. The administrator approved the reviewed link in the admin family-members
   panel (`Pending link reviews` → `Approve link`), which reported
   "Link approved." and left no pending review.
9. After approval, `/api/family/photographs` and the synthetic person profile
   returned HTTP 200 for the invitee. `/api/admin/family-members` still returned
   HTTP 403, so the viewer did not receive administrator access.

## Activation route confirmation

The path form is served by `web/src/app/activate/[token]/page.tsx`, which reads
the `[token]` segment and calls the public `GET /api/invitations/{token}` route.
The captured message's exact URL was opened in the browser and completed the
flow described above. The Rust unit test `activation_link_uses_the_path_form`
pins the builder output. No API route was changed by MAIL-003; only the link
shape changed, and the existing web route accepts it.

## Known limitations

- This is local verification only. Production readiness, public application and
  Keycloak URLs, real SMTP credentials, and the first real-person rollout remain
  separate work.
- Mailpit is a capture sink. A successful send means the SMTP server accepted the
  message; it never means a real inbox received it.
- The synthetic records and test identities are local development data. They are
  not part of the committed evidence, and the invitation token is single-use.
- The Keycloak helper assumes the local container keeps the bootstrap-admin
  environment. It refuses to print the password and fails closed when the
  container is absent.
