# Local email with Mailpit — MAIL-001 prerequisites

Status: Preflight implementation note for MAIL-001
Plan: `docs/plans/PLAN-TRAN-LOCAL-EMAIL-MAILPIT.md`

This note records the results of the MAIL-001 preflight: the concrete repository
paths for each traced area, the runtime topology, the activation-link finding,
the selected Mailpit image pin, the affected files for the later stages, and the
lifecycle and working-tree checks. Mailpit runs as a local test inbox only. This
note does not send messages to real recipients.

## Path inventory

Each traced area maps to a concrete repository path.

| Area | Path | Notes |
| --- | --- | --- |
| Docker Compose | `docker-compose.yml` | One service, `postgres` (`pgvector/pgvector:pg17`). Loopback port `5434:5432`, named volume `rag_postgres_data`, `pg_isready` healthcheck. No Compose profile convention and no `mailpit` service yet. |
| Local startup commands | `README.md`, `Makefile`, `scripts/web-control.sh` | `make db-up`, `make ingest`, `make ask`, `make eval`, `make web-check`, `make web-e2e`; `cargo run --locked --bin api`; `./scripts/web-control.sh start`. |
| Delivery adapters | `src/onboarding/delivery.rs`, `src/onboarding/smtp_delivery.rs`, `src/config.rs`, `.env.example` | `MailAdapter` trait with `UnconfiguredAdapter` (default, fails closed), `FakeAdapter`, `HttpMailAdapter`, and `SmtpMailAdapter` (lettre, MAIL-003). Selector keys `ONBOARDING_MAIL_ADAPTER`, `ONBOARDING_MAIL_*`, `ONBOARDING_SMTP_*`. Bulk send in `send_batch`. |
| Activation route and link builder | `src/onboarding/delivery.rs` (`activation_link`), `src/config.rs` (`ONBOARDING_ACTIVATION_URL`) | The builder appends the path segment `/<token>` to the configured base. The route is configured by `ONBOARDING_ACTIVATION_URL` (default `/activate`). See the finding below. |
| Keycloak configuration | `docs/operations/RBAC-OPERATIONS.md` (Keycloak administrator setup), `src/config.rs` (`KEYCLOAK_URL` and related keys) | Keycloak is an external identity provider reached through `KEYCLOAK_URL`; there is no realm import file and no Keycloak service in `docker-compose.yml`. SMTP setup for local Keycloak is an operator procedure owned by MAIL-004. |
| Relevant tests | `src/onboarding/delivery.rs`, `src/onboarding/smtp_delivery.rs`, `src/config.rs` (unit tests), `tests/` (integration suites) | Delivery tests include `smtp_adapter_reports_acceptance_only_on_a_2xx_final_reply`, `smtp_adapter_reports_a_rejection_as_failure_without_leaking`, `smtp_adapter_does_not_downgrade_a_tls_mode`, `smtp_adapter_times_out_on_a_silent_server`, `activation_link_is_a_path_segment`, and `the_smtp_adapter_requires_valid_settings`. Repository gates: `cargo build --locked`, `cargo test --locked`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`. Integration tests use `scripts/test-db.sh env` disposable DSNs; the live database guard is unchanged. |

## Runtime topology

API on host, UI on host, PostgreSQL in Docker.

- The API runs on the host with `cargo run --locked --bin api` on `127.0.0.1:8080`.
- The UI runs on the host with `./scripts/web-control.sh start` on `127.0.0.1:3000`.
- PostgreSQL runs in Docker as the `postgres` service in `docker-compose.yml`, published on loopback port `5434`.

The application is not migrated into containers. Mailpit must support this
arrangement, so the SMTP endpoint for the host-based API is `127.0.0.1:1025`.

## Activation-link finding

MAIL-001 originally found the link builder producing `?token=<token>`. The
upstream activation-link fix (PR #140, commit `23291dd`) landed the required
path-based form in `activation_link` in `src/onboarding/delivery.rs`: the base
URL is trimmed and the token is percent-encoded as one path segment. The unit
test `activation_link_is_a_path_segment` pins `/activate/<token>`, and the
integration expectations use the same form. The route remains the value of
`ONBOARDING_ACTIVATION_URL` (default `/activate`). MAIL-003 reuses that fix and
does not duplicate it.

Result: the path-based `/activate/<token>` fix is **present**. No duplicate
activation-link implementation was added.

## Selected Mailpit image

| Item | Value |
| --- | --- |
| Image | `axllent/mailpit` |
| Version pin | `v1.21` (tag `axllent/mailpit:v1.21`) |
| Source | Official Mailpit Docker documentation, https://mailpit.axllent.org/docs/install/docker/ |

MAIL-002 consumes this pin. If a precise digest is required, MAIL-002 records the
`@sha256:` digest from the registry at that time; this note selects the version
tag as the pin.

## SMTP and inbox addresses

| Caller | Address |
| --- | --- |
| Container caller (same Compose network) | `mailpit:1025` |
| Host caller (the host-based API) | `127.0.0.1:1025` |
| Browser viewing the local inbox | `http://127.0.0.1:8025` |

Browser activation links must use the browser-reachable Tran address, not the
internal Docker service name. MAIL-002 must publish SMTP to loopback for the
host-based API and must not enable relay, forwarding, or public binding.

## Affected files for later stages

- MAIL-002: `docker-compose.yml` (add the `mailpit` service and readiness check).
- MAIL-003: `src/onboarding/delivery.rs`, `src/config.rs`, `.env.example`, and
  `Cargo.toml` / `Cargo.lock` only if no suitable SMTP library exists.
- MAIL-004: the Keycloak development configuration mechanism recorded in
  `docs/operations/RBAC-OPERATIONS.md` and `src/config.rs` (`KEYCLOAK_URL`).
- MAIL-005: verification uses `tests/`, `src/onboarding/delivery.rs` tests, the
  repository gates above, and the running API, Keycloak, and Mailpit.

## Message retention (MAIL-002 decision)

MAIL-002 must decide whether captured Mailpit messages persist across restarts.
This note records the question only. MAIL-002 must not remove, recreate, or plan
the removal of existing application volumes.

## Lifecycle and working-tree check

- SOP lifecycle state was read only and not edited. The active plan is
  `docs/plans/PLAN-TRAN-LOCAL-EMAIL-MAILPIT.md`. No unrelated active plan was
  overwritten and no lifecycle state was edited directly.
- At MAIL-001 preflight the tree contained this note and the plan. Later MAIL-002
  through MAIL-005 added the Mailpit Compose service, SMTP adapter and
  configuration, Keycloak helper, path-form activation link, and verification
  notes. Those changes are recorded in
  `docs/architecture/LOCAL-EMAIL-MAILPIT-VERIFICATION.md`; no unrelated change
  was reverted. The branch is `main` with no local divergence from
  `origin/main`.
- No database volume was removed, recreated, or planned for removal. The
  `rag_postgres_data` volume in `docker-compose.yml` is untouched.

## Conflicts

No unresolved conflict remains with the active plan, unrelated working-tree
changes, or database volumes. The one open dependency, the path-based
`/activate/<token>` upstream change, is recorded above and owned outside
MAIL-001; MAIL-003 reuses it without duplicating its implementation.
