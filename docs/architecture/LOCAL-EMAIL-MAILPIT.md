# Local email with Mailpit

Status: MAIL-002 implementation note
Plan: `docs/plans/PLAN-TRAN-LOCAL-EMAIL-MAILPIT.md`
Related: `docs/architecture/LOCAL-EMAIL-MAILPIT-PREREQUISITES.md` (MAIL-001)

Mailpit runs as a local test inbox only. It never sends mail to real
recipients. No relay, forwarding, or public port binding is enabled.

## Service definition

The root `docker-compose.yml` defines one optional service, `mailpit`, behind
the Compose profile `mail`. The default `docker compose up -d` and `make db-up`
keep their current behavior. Mailpit starts only on explicit request.

| Item | Value |
| --- | --- |
| Image | `axllent/mailpit:v1.21` (pinned version tag) |
| Profile | `mail` |
| SMTP publication | `127.0.0.1:1025:1025` (loopback only) |
| Web inbox publication | `127.0.0.1:8025:8025` (loopback only) |
| Readiness check | `["CMD", "/mailpit", "readyz"]` |

## Start and stop commands

Start Mailpit:

```bash
docker compose --profile mail up -d mailpit
```

Stop Mailpit:

```bash
docker compose --profile mail stop mailpit
```

Remove the container while keeping captured messages:

```bash
docker compose --profile mail rm mailpit
```

The `-d` flag detaches the service. The `--profile mail` flag names the profile
that enables the service, so the command works as written against this
`docker-compose.yml`.

## Addresses

| Caller | SMTP address |
| --- | --- |
| API or Keycloak on the same Compose network | `mailpit:1025` |
| API running on the host | `127.0.0.1:1025` |
| Browser viewing the local inbox | `http://127.0.0.1:8025` |

Container-to-container traffic uses the Compose service name `mailpit`.
Host-based applications use the loopback publication.

The browser inbox binds to loopback only. No public port binding is used, and
no `0.0.0.0` host address appears in the published ports.

## Readiness check

The service uses Mailpit's supported readiness endpoint through the image's own
binary: `/mailpit readyz`. The probe runs inside the container, so it needs no
host tool and no shell. The container reports `healthy` once Mailpit accepts
connections.

Verified locally: `docker compose --profile mail up -d mailpit` reported
`tran-mailpit-1 ... Up (healthy)`, the inbox answered
`GET http://127.0.0.1:8025/api/v1/info` with HTTP 200, and a synthetic SMTP
message appeared in `/api/v1/messages`.

## Message retention

Captured messages persist across restarts. The service sets
`MP_DATABASE=/data/mailpit.db` and mounts the dedicated named volume
`mailpit_data` at `/data`, so a stop/start cycle keeps the inbox, and an
operator can remove it deliberately with
`docker compose --profile mail down -v` or by deleting that volume.

Verified locally: a synthetic message remained in the inbox after `stop` /
`start` and after `rm -f` followed by `up -d` (the container was recreated while
the volume stayed). Without `MP_DATABASE`, Mailpit uses `/tmp` and the volume is
unused; the environment setting is required for this claim.

The reason: a local developer inbox is more useful when a signup or invitation
message survives an API or UI restart, and Mailpit's own retention limits keep
the volume bounded in practice.

`mailpit_data` is a new volume. The existing `rag_postgres_data` volume is not
removed, recreated, or changed.

## Activation links

Browser activation links must use the browser-reachable Tran address, not a
Docker service name. Set `ONBOARDING_ACTIVATION_URL` to the host UI origin, for
example `http://127.0.0.1:3000/activate`. Never set it to `http://mailpit:8025`
or to another internal Docker service name.

MAIL-003 implements the path form in `activation_link`: the configured base has
its trailing slash trimmed and the token is appended as `/<token>`. The captured
invitation message contains `http://127.0.0.1:3000/activate/<token>`; the browser
opened that exact URL. The query form (`?token=`) is no longer produced.

## Local Keycloak SMTP

`scripts/keycloak-mailpit-smtp.sh` configures the existing local realm through
`kcadm.sh`; it does not import or replace the realm, and it never enables
self-registration. It writes the previous realm representation to
`.run/keycloak-smtp-before.json` before changing `smtpServer`, and
`--revert` restores it.

```bash
scripts/keycloak-mailpit-smtp.sh --show
scripts/keycloak-mailpit-smtp.sh
scripts/keycloak-mailpit-smtp.sh --revert
```

`tran-keycloak` is on Docker's default bridge network, so it reaches the host
published SMTP port through `host.docker.internal:1025` (the default). If the
container is attached to the Compose network instead, set
`MAILPIT_SMTP_HOST=mailpit`. The realm keeps `registrationAllowed=false`; only
`smtpServer.host`, `port`, `from`, `fromDisplayName`, `auth`, `starttls`, and
`ssl` are written.

Verified locally by creating a temporary Keycloak identity and requesting a
password-reset email with the Admin API: the message "Update Your Account"
arrived in Mailpit for the synthetic address. The test identity is not part of
the committed evidence.

## Unchanged items

The `postgres` service definition and the `rag_postgres_data` volume are byte
identical to the pre-change file. No existing application service or volume is
removed or recreated.
