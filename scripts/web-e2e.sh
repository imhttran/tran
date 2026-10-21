#!/bin/sh
# Run the RUST-008A real-browser E2E suite.
#
#   scripts/web-e2e.sh
#
# Starts the production Axum API (real PostgreSQL + Ollama) on loopback, starts
# the Next.js app on 127.0.0.1:3000, runs Playwright against the real browser,
# then stops both servers and deletes only this run's corpus.
#
# The E2E suite is operator-run verification: Node.js and browser execution are
# outside SOP's command policy. The deterministic Rust-side contract is covered
# by rust/acceptance/api_contract.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

if [ -f .env ]; then set -a; . ./.env; set +a; fi

if [ -z "${DATABASE_URL:-}" ]; then
	echo "web-e2e: DATABASE_URL is required (set it or provide .env)" >&2
	exit 1
fi

API_PORT="${API_PORT:-8080}"
CORPUS="rust-008a-e2e-$(date +%Y%m%dT%H%M%S)-$$"
export RAG_CORPUS="$CORPUS"
export OLLAMA_CHAT_MODEL="${OLLAMA_CHAT_MODEL:-llama3.2:3b}"
export MIN_SIMILARITY="${MIN_SIMILARITY:-0.3}"
export REQUEST_TIMEOUT="${REQUEST_TIMEOUT:-180s}"

# Test-only OIDC provider. The API verifies RS256 tokens against this local
# JWKS; the browser tests use the same cryptographically valid tokens and the
# Next.js login flow. Keys and accounts are synthetic and disposable.
AUTH_PORT="${E2E_AUTH_PORT:-9099}"
E2E_KEY_FILE="/tmp/tran-e2e-oidc-key-$$.pem"
E2E_ISSUER="http://127.0.0.1:${AUTH_PORT}"
E2E_AUDIENCE="tran-e2e"
export AUTH_ISSUER="$E2E_ISSUER"
export AUTH_AUDIENCE="$E2E_AUDIENCE"
export JWKS_URL="${E2E_ISSUER}/jwks"
export AUTH_RATE_LIMIT="${AUTH_RATE_LIMIT:-1000}"
export AUTH_CLOCK_SKEW_SECS="${AUTH_CLOCK_SKEW_SECS:-60}"
export JWKS_CACHE_TTL_SECS="${JWKS_CACHE_TTL_SECS:-60}"
export OIDC_ISSUER="$E2E_ISSUER"
export OIDC_CLIENT_ID="$E2E_AUDIENCE"
export OIDC_REDIRECT_URI="http://127.0.0.1:3000/api/auth/callback"
export OIDC_AUTHORIZATION_ENDPOINT="${E2E_ISSUER}/authorize"
export OIDC_TOKEN_ENDPOINT="${E2E_ISSUER}/token"
export OIDC_END_SESSION_ENDPOINT="${E2E_ISSUER}/logout"
export OIDC_COOKIE_SECURE="false"
export HOF_THIRD_BRANCH_KEY="branch-3"
# The fake adapter proves the send gate without real email. The Rust
# integration suite separately proves the unconfigured adapter fails closed.
export ONBOARDING_MAIL_ADAPTER="${ONBOARDING_MAIL_ADAPTER:-fake}"
export ONBOARDING_ACTIVATION_URL="${ONBOARDING_ACTIVATION_URL:-http://127.0.0.1:3000/activate}"
export ONBOARDING_SEND_INTERVAL_MS="${ONBOARDING_SEND_INTERVAL_MS:-0}"

echo "web-e2e: starting the test OIDC issuer"
python3 scripts/e2e-auth.py serve --port "$AUTH_PORT" --key-file "$E2E_KEY_FILE" >/tmp/tran-e2e-auth.log 2>&1 &
AUTH_PID=$!
i=0
while [ "$i" -lt 50 ]; do
	if curl -fsS -m 1 -o /dev/null "$JWKS_URL"; then
		break
	fi
	sleep 0.2
	i=$((i+1))
done
if [ "$i" -ge 50 ]; then
	echo "web-e2e: the test OIDC issuer did not become ready" >&2
	cat /tmp/tran-e2e-auth.log >&2 || true
	exit 1
fi

echo "web-e2e: corpus $CORPUS"
E2E_CSRF_TOKEN="e2e-csrf-token"
export E2E_CSRF_TOKEN
E2E_ADMIN_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-admin --role admin --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_ADMIN_TOKEN
export E2E_SESSION_TOKEN="$E2E_ADMIN_TOKEN"
E2E_VIEWER_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-viewer --role viewer --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_VIEWER_TOKEN
# A second admin: a nomination cannot be approved by its own nominator.
E2E_APPROVER_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-approver --role admin --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_APPROVER_TOKEN

echo "web-e2e: seeding test identities"
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/006_identity.sql >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/007_user_lifecycle.sql >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "DELETE FROM users WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO users (subject, issuer) VALUES ('e2e-admin', '$E2E_ISSUER'), ('e2e-viewer', '$E2E_ISSUER'), ('e2e-approver', '$E2E_ISSUER')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO role_assignments (user_id, role, granted_by) SELECT id, 'admin', 'e2e' FROM users WHERE subject IN ('e2e-admin', 'e2e-approver')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO role_assignments (user_id, role, granted_by) SELECT id, 'viewer', 'e2e' FROM users WHERE subject = 'e2e-viewer'" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO corpus_grants (user_id, corpus, granted_by) SELECT id, '$CORPUS', 'e2e' FROM users WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver')" >/dev/null

if [ -z "${GENEALOGY_DATABASE_URL:-}" ]; then
	echo "web-e2e: GENEALOGY_DATABASE_URL is required for genealogy integration tests" >&2
	exit 1
fi
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-f genealogy/migrations/0001_genealogy_schema_version.sql >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-f genealogy/migrations/0002_catalog_keys.sql >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-f genealogy/migrations/0003_assertions_evidence_and_links.sql >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-f genealogy/migrations/0004_review_workflow.sql >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-f genealogy/migrations/0005_source_kind.sql >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "DROP TABLE IF EXISTS head_of_family_audit_event, head_of_family_appointment, head_of_family_nomination CASCADE" >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-f genealogy/migrations/0006_head_of_family.sql >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-f scripts/seed-genealogy-e2e.sql >/dev/null

# Bulk onboarding (BULK-005). Recreate the onboarding schema so a reused
# disposable database never carries a partial prior migration.
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "DROP TABLE IF EXISTS onboarding_batch_audit, onboarding_batch_item, onboarding_batch, onboarding_recipient_contact, onboarding_link_audit, onboarding_person_account_link, onboarding_invitation, onboarding_username_reservation, onboarding_branch_reviewer_grant, onboarding_profile_review_event, onboarding_profile_revision, onboarding_profile_value, onboarding_media_metadata CASCADE" >/dev/null
for migration in \
	0007_onboarding.sql \
	0008_onboarding_identity.sql \
	0009_onboarding_link_audit.sql \
	0010_onboarding_foreign_keys.sql \
	0011_onboarding_foreign_key_reconciliation.sql \
	0012_profile_revisions.sql \
	0013_onboarding_batch.sql; do
	psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
		-f "genealogy/migrations/$migration" >/dev/null
done
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-f scripts/seed-bulk-onboarding-e2e.sql >/dev/null

echo "web-e2e: building the API"
cargo build --locked --bin api

api_log=$(mktemp)
API_HOST=127.0.0.1 API_PORT="$API_PORT" target/debug/api >"$api_log" 2>&1 &
API_PID=$!

cleanup() {
	kill "$API_PID" 2>/dev/null || true
	wait "$API_PID" 2>/dev/null || true
	kill "$AUTH_PID" 2>/dev/null || true
	wait "$AUTH_PID" 2>/dev/null || true
	rm -f "$E2E_KEY_FILE"
	./scripts/web-control.sh stop >/dev/null 2>&1 || true
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM documents WHERE corpus = '$CORPUS'" >/dev/null 2>&1 || true
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM audit_events WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver')" >/dev/null 2>&1 || true
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM users WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver')" >/dev/null 2>&1 || true
	rm -f "$api_log"
}
trap cleanup EXIT INT TERM

echo "web-e2e: waiting for the API health endpoint"
i=0
while [ "$i" -lt 60 ]; do
	if curl -fsS -m 2 -o /dev/null "http://127.0.0.1:$API_PORT/api/health"; then
		break
	fi
	if ! kill -0 "$API_PID" 2>/dev/null; then
		echo "web-e2e: API exited during startup:" >&2
		cat "$api_log" >&2
		exit 1
	fi
	sleep 1
	i=$((i+1))
done
if [ "$i" -ge 60 ]; then
	echo "web-e2e: API did not become healthy" >&2
	cat "$api_log" >&2
	exit 1
fi

echo "web-e2e: starting Next.js"
./scripts/web-control.sh stop >/dev/null 2>&1 || true
./scripts/web-control.sh start

echo "web-e2e: running Playwright"
./scripts/web-control.sh e2e

echo "web-e2e: ok"
