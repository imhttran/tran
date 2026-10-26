.PHONY: db-up db-down db-schema test test-db-up validate fmt-json ingest ask eval rust-build rust-test rust-fmt rust-clippy rust-acceptance-evidence rust-acceptance-final-layout rust-acceptance-clean-checkout rust-acceptance-k3 rust-acceptance-ingest-md-text rust-acceptance-live-rag rust-acceptance-api-contract rust-acceptance-functional transition-evidence rust-check check web-check-env web-install web-start web-stop web-status web-typecheck web-build web-check web-e2e genealogy-migrate genealogy-import app-restart app-update admin-setup
# Convenience targets for the Rust-only ingest/query workflow.
#
#   make db-up        start PostgreSQL and apply the schema
#   make db-schema    (re)apply every migration in migrations/
#   make test         Rust unit and integration tests
#   make fmt-json     rewrite every tracked JSON file with jq
#   make ingest       load examples/loan-policy.md
#   make ask Q="…"    ask a question (omit Q to be prompted)
#   make eval         score retrieval against evals/retrieval.json
#   make web-start    start the Next.js UI on 127.0.0.1:3000 (background)
#   make web-stop     stop the Next.js server started by make web-start
#   make web-status   report the server state, URL, and log path
#
#   make genealogy-migrate  apply genealogy migrations
#   make genealogy-import   import CATALOG (idempotent)
#   make app-restart        restart only the API/UI processes the scripts own
#   make app-restart FORCE=1 also stop listeners on the configured API/UI ports
#   make app-update         validate, migrate, import, rebuild, restart, report
#   make admin-setup        explicitly configure an administrator from Keycloak
#
# app-update never runs admin-setup: administrator provisioning stays explicit.
#
# After pulling a merge, run:
#   git pull --ff-only
#   make app-update
#
# Override inputs on the command line, e.g.
#   make ingest FILE=examples/other.md
#   make ask Q="When is a late fee assessed?"
#   make genealogy-import CATALOG=docs/tran/other-catalog.md

# Load local .env as defaults only. A makefile assignment beats the environment,
# so including .env directly would replace a value the caller exported (for
# example `DATABASE_URL=.../rag_test make web-e2e` or `API_URL=... make
# app-update`). The helper writes a copy with the caller's keys removed.
ENV_DEFAULTS := $(shell ./scripts/env-defaults.sh .env .run/make-defaults.env)
ifneq ($(ENV_DEFAULTS),)
-include $(ENV_DEFAULTS)
endif
DATABASE_URL ?= postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable
export DATABASE_URL
ifneq ($(strip $(GENEALOGY_DATABASE_URL)),)
export GENEALOGY_DATABASE_URL
endif
FILE ?= examples/loan-policy.md
Q ?=
CATALOG ?= docs/tran/Tran-Family-People-Catalog.md
ADMIN_CORPUS ?= trusted
KEYCLOAK_USERNAME ?=

# Start the database and apply the RAG schema to the resolved DSN. The backend
# follows DB_BACKEND (native|docker); with it unset, an already-running server is
# left alone, macOS starts Docker, and Linux requires native PostgreSQL.
db-up:
	./scripts/db-up.sh

# Stop the database. Only the Docker backend is stopped; native PostgreSQL is
# managed by the operating system.
db-down:
	./scripts/db-down.sh

# Run the Rust tests against disposable test databases. The runner resolves and
# prepares them, exports REQUIRE_TEST_DATABASE=1 so a missing database fails the
# suite, and never touches an application database.
test:
	./scripts/test-db.sh exec -- cargo test --locked

# Create and migrate only the disposable test databases.
test-db-up:
	./scripts/test-db.sh prepare

# Full validation with portable database configuration and a stable log at
# .run/validation/latest.log. This is the single command SOP validation runs.
validate:
	./scripts/validate.sh

# Rewrite every tracked JSON file with jq's formatting. This expands the
# hand-compacted one-line arrays in .zed/settings.json and evals/retrieval.json,
# so the first run produces a large diff. Keys are left in the order written;
# add -S to jq below if you would rather sort them.
fmt-json:
	for f in $$(git ls-files -- '*.json'); do \
		jq . "$$f" > "$$f.tmp" || { rm -f "$$f.tmp"; echo "invalid json: $$f" >&2; exit 1; }; \
		mv "$$f.tmp" "$$f"; \
	done

# Chunk FILE and store it (safe to re-run).
ingest:
	cargo run --locked --bin ingest $(FILE)

# Ask a question; omit Q to be prompted.
ask:
	cargo run --locked --bin query "$(Q)"

# Score retrieval against evals/retrieval.json.
eval:
	cargo run --locked --bin eval

db-schema:
	@for f in migrations/*.sql; do \
		psql "$$DATABASE_URL" -v ON_ERROR_STOP=1 -f "$$f" || exit 1; \
	done

# ---- Local application lifecycle (after a merge) ------------------------------
# These targets update local data and processes using the code already checked
# out. They never fetch, switch branches, stash, reset, or discard changes; run
# `git pull --ff-only` yourself first. See docs/guides/usage.md.
genealogy-migrate:
	./scripts/local-app.sh migrate

genealogy-import:
	./scripts/local-app.sh import "$(CATALOG)"

app-restart:
	./scripts/local-app.sh restart $(if $(filter 1,$(FORCE)),--force,)

app-update:
	./scripts/local-app.sh update "$(CATALOG)"

# Explicit administrator setup. Requires KEYCLOAK_URL, KEYCLOAK_REALM,
# AUTH_ISSUER, and DATABASE_URL; it prompts for provider credentials it does not
# receive from the environment. Never invoked by app-update.
admin-setup:
	./scripts/admin-setup.sh --username "$(KEYCLOAK_USERNAME)" --corpus "$(ADMIN_CORPUS)"

# ---- Rust checks (the package lives at the repository root) -------------------
# Reused by .githooks so the hooks and the Makefile never diverge.
rust-fmt:
	cargo fmt --check

rust-build:
	cargo build --locked

rust-test:
	./scripts/test-db.sh exec -- cargo test --locked

rust-clippy:
	cargo clippy --locked --all-targets -- -D warnings

# RUST-007C final-layout and clean-checkout acceptance.
rust-acceptance-evidence:
	cargo test --locked --manifest-path acceptance/evidence/Cargo.toml

rust-acceptance-final-layout:
	cargo test --locked --manifest-path acceptance/final_layout/Cargo.toml

rust-acceptance-clean-checkout:
	cargo test --locked --manifest-path acceptance/clean_checkout/Cargo.toml

# RUST-007E functional-closure acceptance. These need the dedicated database;
# the live target additionally needs a real Ollama. They are explicit targets
# rather than part of `rust-check` so an ordinary edit loop does not require
# either service.
rust-acceptance-k3:
	cargo test --locked --manifest-path acceptance/k3_parity/Cargo.toml

rust-acceptance-ingest-md-text:
	cargo test --locked --manifest-path acceptance/ingest_md_text/Cargo.toml

rust-acceptance-live-rag:
	cargo test --locked --manifest-path acceptance/live_rag/Cargo.toml

rust-acceptance-api-contract:
	cargo test --locked --manifest-path acceptance/api_contract/Cargo.toml

rust-acceptance-functional: rust-acceptance-k3 rust-acceptance-ingest-md-text

transition-evidence: rust-acceptance-evidence rust-acceptance-final-layout rust-acceptance-clean-checkout

rust-check: rust-fmt rust-build rust-test rust-clippy transition-evidence

# ---- Web checks (the Next.js UI under web/) -------------------------------------
web-typecheck:
	./scripts/web-control.sh typecheck

web-build:
	./scripts/web-control.sh build

web-check: web-typecheck web-build

# Real-browser E2E. Starts the Axum API and Next.js, runs Playwright, and cleans
# up only the run corpus. Needs PostgreSQL and Ollama.
web-e2e:
	./scripts/web-e2e.sh

# Everything: the Rust checks and the web checks.
check: rust-check web-check

