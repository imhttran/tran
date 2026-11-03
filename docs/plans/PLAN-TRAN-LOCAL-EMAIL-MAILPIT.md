# Tran local email with Mailpit

Status: Proposed implementation plan
Date: 2026-10-30
Repository: `imhttran/tran`

## Project

Run Mailpit in Docker and use it to test Tran invitations and Keycloak email
locally. Complete one invitation from the captured email through sign-in,
pending review, administrator approval, and archive access. Open a PR with the
implementation and focused verification evidence.

Mailpit is a local test inbox. This plan does not enable delivery to real family
members' inboxes. Local completion does not establish production readiness;
public app and Keycloak URLs, real mail delivery, and the first real-person
rollout remain separate work.

Official Docker reference: https://mailpit.axllent.org/docs/install/docker/

## Summary

Starting assumptions: the supplied implementation reports describe an HTTP
invitation adapter, a fake adapter for tests, path-based activation links, and
archive access deferred until link approval. Confirm these facts against the
latest code before implementation; this plan does not claim that the current
repository or deployment has been inspected. The documentation could not be
retrieved while drafting this plan, so confirm the supported image tag,
configuration, ports, and health-check method during preflight. Do not use an
unverified example as deployment evidence.

Scope and constraints:

- Add Mailpit to the existing local Docker Compose setup.
- Add an SMTP invitation adapter using the existing delivery abstraction.
- Preserve the HTTP and fake adapters.
- Configure local Keycloak SMTP without enabling self-registration.
- Use synthetic people, accounts, and recipient addresses for verification.
- Preserve invitation authorization, reviewed linking, and deferred archive
  grants.
- Keep mail capture local. Do not configure Mailpit relay or forwarding.
- Do not change production settings, expose services publicly, send real
  invitations, or merge the PR.
- Preserve unrelated changes, database volumes, and SOP execution evidence.

Stop dependent work only for a concrete blocker: unavailable local Docker or
Keycloak access, missing required credentials, a port conflict that cannot be
resolved safely, or an unsupported SOP lifecycle transition. Finish independent
work and report the exact unblock action. Do not reset state, recreate existing
databases, weaken checks, or claim completion from unchanged code.

## MAIL-001 — Inspect and prepare

1. Fetch the latest `origin/main` and read repository instructions.
2. Create an isolated branch or worktree using the repository convention.
3. Inspect Docker Compose, local startup commands, delivery adapters, activation
   routes, Keycloak configuration, and relevant tests.
4. Identify whether the API runs on the host or in Docker. Support the existing
   arrangement without forcing an application migration into containers.
5. Confirm the activation-link fix is present. If missing, include the required
   upstream change without duplicating its implementation.
6. Check SOP lifecycle state. Use the supported plan workflow if required. Do
   not overwrite an unrelated active plan or edit lifecycle state directly.

Deliverable: A short implementation note listing the affected files, runtime
topology, and selected Mailpit image version or digest.

### Dependencies

None

### Deliverables

- `docs/architecture/LOCAL-EMAIL-MAILPIT-PREREQUISITES.md`.

### Acceptance Criteria

- The implementation note lists the affected files, the runtime topology (API on host or in Docker), and the selected Mailpit image version or digest.
- Docker Compose, local startup commands, delivery adapters, activation routes, Keycloak configuration, and relevant tests are traced to concrete repository paths.
- The activation-link fix is confirmed present, or the required upstream change is identified without duplicating its implementation.
- No unresolved conflict with the active plan, unrelated working-tree changes, or database volumes remains.

## MAIL-002 — Add the local Mailpit service

1. Add a `mailpit` service using a pinned official image version or digest.
2. Follow the repository convention for optional development services or
   Compose profiles.
3. Bind the browser inbox to loopback. Publish SMTP to loopback only when
   host-based applications need it.
4. Use the Compose service name for container-to-container SMTP traffic.
5. Add a supported readiness check and a documented start/stop command.
6. Choose and document whether captured messages persist across restarts. Never
   remove existing application volumes.

Expected addresses, subject to preflight confirmation:

| Caller | SMTP address |
| --- | --- |
| API or Keycloak on the same Compose network | `mailpit:1025` |
| API running on the host | `127.0.0.1:1025` |
| Browser viewing the local inbox | `http://127.0.0.1:8025` |

Browser activation links must use the browser-reachable Tran address, not the
internal Docker service name.

### Dependencies

- MAIL-001

### Deliverables

- `docker-compose.yml` with the `mailpit` service and a readiness check.
- Documented start/stop command and message-retention choice for local development.

### Acceptance Criteria

- Compose configuration validates and the `mailpit` service uses a pinned official image version or digest.
- Mailpit becomes ready and the local inbox opens on a loopback-bound browser address.
- SMTP is reachable at the documented container-to-container and host addresses, and browser activation links use the browser-reachable Tran address.
- No mail relay, forwarding, or public port binding is enabled.
- Existing application volumes and services are unchanged.

## MAIL-003 — Add SMTP invitation delivery

1. Extend the existing adapter selector with `ONBOARDING_MAIL_ADAPTER=smtp`.
2. Reuse a suitable existing Rust SMTP library. If none exists, choose a
   maintained library compatible with the repository's runtime and toolchain.
3. Add configuration validation, a bounded send timeout, and explicit TLS modes.
   Permit unauthenticated plaintext only through explicit local configuration.
   Never downgrade TLS after a connection failure.
4. Support sender address, SMTP host, port, optional credentials, and timeout.
   Reuse existing naming conventions where possible.
5. Route individual send, resend, and bulk delivery through the same adapter
   abstraction and activation-link builder.
6. Preserve existing audit, retry, and delivery-state behavior. Do not record
   success when SMTP rejects or fails to accept a message.
7. Report SMTP acceptance accurately. Do not claim delivery to a real inbox.
8. Keep credentials, invitation tokens, and message bodies out of ordinary logs.

Proposed configuration keys; confirm names during implementation:

```dotenv
ONBOARDING_MAIL_ADAPTER=smtp
ONBOARDING_SMTP_HOST=mailpit
ONBOARDING_SMTP_PORT=1025
ONBOARDING_SMTP_TLS=none
ONBOARDING_SMTP_FROM=tran@example.test
ONBOARDING_SMTP_TIMEOUT_SECS=10
# Optional for a future authenticated SMTP provider:
# ONBOARDING_SMTP_USERNAME=
# ONBOARDING_SMTP_PASSWORD=
ONBOARDING_ACTIVATION_URL=http://localhost:3000/activate
```

Use `127.0.0.1` for the SMTP host when the API runs outside Docker. Resolve the
activation origin from the existing local browser and OIDC configuration; do
not mix `localhost` and `127.0.0.1` inadvertently. Keep credentials,
invitation tokens, and message bodies out of ordinary logs.

### Dependencies

- MAIL-001
- MAIL-002

### Deliverables

- SMTP adapter in `src/onboarding/delivery.rs` and its selector/configuration in `src/config.rs`.
- `Cargo.toml` and `Cargo.lock` dependency changes only if no suitable SMTP library exists.
- `.env.example` documentation for the SMTP configuration keys.

### Acceptance Criteria

- `ONBOARDING_MAIL_ADAPTER=smtp` selects the SMTP adapter, and the HTTP, fake, and unconfigured adapters retain their existing behavior.
- Configuration validation covers host, port, sender address, TLS mode, optional credentials, and timeout, and invalid configuration fails with a clear error.
- Unauthenticated plaintext is permitted only through explicit local configuration; TLS is never downgraded after a connection failure.
- Individual send, resend, and bulk delivery use the same adapter abstraction and activation-link builder.
- SMTP acceptance is reported accurately: a rejection or failure is not recorded as success, and no claim of delivery to a real inbox is made.
- Captured links use the path-based `/activate/<token>` form.
- Credentials, invitation tokens, and message bodies do not appear in ordinary logs.
- Send timeout is bounded.

## MAIL-004 — Connect local Keycloak

1. Inspect the existing realm bootstrap or development configuration mechanism.
2. Add a repeatable local SMTP setup using that mechanism. Account for existing
   realms where startup imports do not update settings.
3. Apply only the local SMTP fields needed for Mailpit. Record previous values so
   the change can be reversed.
4. Keep self-registration, identity policies, roles, and existing users
   unchanged.
5. Use a dedicated test identity to trigger a supported Keycloak email action,
   such as password reset.

### Dependencies

- MAIL-001

### Deliverables

- Repeatable local Keycloak SMTP configuration through the repository's existing realm setup mechanism.
- Documentation of the previous values and the rollback procedure.

### Acceptance Criteria

- A Keycloak-generated message appears in Mailpit.
- Only the local SMTP fields needed for Mailpit are applied; self-registration, identity policies, roles, and existing users are unchanged.
- Existing realm data remains intact, and the setup works for both fresh and existing local realms.
- Previous values are recorded so the change can be reversed.

## MAIL-005 — Verify the critical flow

Use the running application APIs, local database, Keycloak, and Mailpit for one
focused integration path:

1. Prepare a synthetic living-person record and a dedicated Keycloak test
   account with no Tran archive access.
2. Send an invitation through the authorized application flow.
3. Retrieve the actual captured message from Mailpit using its supported
   interface.
4. Open the exact URL from that message. Do not reconstruct the URL in the
   browser test.
5. Complete sign-in and confirm the intended invitation survives the OIDC round
   trip.
6. Accept the invitation and confirm Pending review.
7. Confirm protected archive access is denied before approval.
8. Approve the person link through the existing admin panel.
9. Confirm viewer access opens without elevated permissions.

Add focused checks for SMTP failure and resend token behavior. Reuse existing
authorization and activation checks when their inputs are unchanged. Run
affected checks and required repository gates at integration. Do not expand this
into an exhaustive test campaign.

### Dependencies

- MAIL-002
- MAIL-003
- MAIL-004

### Deliverables

- Recorded verification evidence for the captured-email flow, the SMTP-failure check, and the resend token-replacement check.

### Acceptance Criteria

- The exact URL from the captured Mailpit message drives sign-in, and the intended invitation survives the OIDC round trip.
- Accepting the invitation shows Pending review, protected archive access is denied before approval, and administrator approval opens viewer access without elevated permissions.
- SMTP failure is reported correctly and is not recorded as success.
- Resend preserves the established token-replacement policy.
- Affected checks and the required repository gates run, with actual commands and results recorded.

## MAIL-006 — Document and open the PR

1. Update `.env.example` and the existing invitation mail documentation.
2. Document startup, inbox access, host/container addressing, local Keycloak
   setup, and rollback.
3. Explain that a future real SMTP provider needs separate credentials, sender
   configuration, and deployment verification.
4. Record verification results and remaining limitations. Exclude raw invitation
   URLs, tokens, credentials, and personal data from committed evidence.
5. Open a PR with the implementation, validation summary, and local operator
   instructions. Do not merge it.

### Dependencies

- MAIL-005

### Deliverables

- Updated `.env.example` and existing invitation mail documentation.
- A pull request containing the implementation, validation summary, and local operator instructions.

### Acceptance Criteria

- `.env.example` and the existing invitation mail documentation cover startup, inbox access, host/container addressing, local Keycloak setup, and rollback.
- The documentation explains that a future real SMTP provider needs separate credentials, sender configuration, and deployment verification.
- Committed evidence records verification results and remaining limitations and excludes raw invitation URLs, tokens, credentials, and personal data.
- A PR is opened with the implementation, validation summary, and local operator instructions, and it is not merged.
