#!/bin/sh
# Run the RUST-008A real-browser E2E suite.
#
#   scripts/web-e2e.sh
#
# Starts the production Axum API (real PostgreSQL + Ollama) on loopback, starts
# the Next.js app on 127.0.0.1:${WEB_PORT:-3000}, runs Playwright against the real browser,
# then stops both servers and deletes only this run's corpus.
#
# The E2E suite is operator-run verification: Node.js and browser execution are
# outside SOP's command policy. The deterministic Rust-side contract is covered
# by rust/acceptance/api_contract.
#
# Database selection: an explicit caller value always wins over .env, and both
# DSNs must name a disposable database (`*_test` or `*_disposable`). Application
# databases are refused before the first migration, seed, or delete. Isolation
# never depends on moving .env aside.
#
# Ports: the API (API_PORT, default 8080), the test OIDC issuer (E2E_AUTH_PORT,
# default 9099), and Next.js (WEB_PORT, default 3000) must be free. A taken port
# stops the run before any database work; nothing already running is touched.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

# Caller values win; .env only fills the gaps.
. "$ROOT/scripts/e2e-db-env.sh"
. "$ROOT/scripts/port-guard.sh"
load_env_defaults "$ROOT/.env"

if [ -z "${DATABASE_URL:-}" ]; then
	echo "web-e2e: DATABASE_URL is required (set it or provide .env)" >&2
	exit 1
fi
if [ -z "${GENEALOGY_DATABASE_URL:-}" ]; then
	echo "web-e2e: GENEALOGY_DATABASE_URL is required (set it or provide .env)" >&2
	exit 1
fi
# Refuse application databases before any destructive setup. Everything below
# this point may migrate, seed, truncate, or delete rows.
if ! require_disposable_database "DATABASE_URL (RAG)" "$DATABASE_URL"; then
	exit 1
fi
if ! require_disposable_database "GENEALOGY_DATABASE_URL" "$GENEALOGY_DATABASE_URL"; then
	exit 1
fi
if ! require_distinct_databases "$DATABASE_URL" "$GENEALOGY_DATABASE_URL"; then
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
WEB_PORT="${WEB_PORT:-3000}"
export WEB_PORT
WEB_ORIGIN="http://127.0.0.1:${WEB_PORT}"

# Every server this run talks to must be one it started. A port that is already
# taken (for example a developer API on 8080 connected to the real databases)
# would otherwise answer the readiness checks and receive the suite's requests.
# Check all ports before the first migration, seed, or delete.
require_port_free "web-e2e: test OIDC issuer" "$AUTH_PORT" E2E_AUTH_PORT
require_port_free "web-e2e: API" "$API_PORT" API_PORT
# A Next.js server that web-control.sh itself manages is stopped and restarted
# below, as before; any other listener on the web port is refused.
web_pid=$(cat "$ROOT/web/.run/web.pid" 2>/dev/null || true)
if [ -z "$web_pid" ] || ! kill -0 "$web_pid" 2>/dev/null; then
	require_port_free "web-e2e: Next.js" "$WEB_PORT" WEB_PORT
fi

# The Next.js proxy forwards /api to API_URL. .env usually points it at the
# developer API, so pin it to this run's API instead.
if [ -n "${API_URL:-}" ] && [ "$API_URL" != "http://127.0.0.1:${API_PORT}" ]; then
	echo "web-e2e: ignoring API_URL=$API_URL; the Next.js proxy uses this run's API on port $API_PORT"
fi
export API_URL="http://127.0.0.1:${API_PORT}"
export E2E_BASE_URL="$WEB_ORIGIN"

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
export OIDC_REDIRECT_URI="${WEB_ORIGIN}/api/auth/callback"
export OIDC_AUTHORIZATION_ENDPOINT="${E2E_ISSUER}/authorize"
export OIDC_TOKEN_ENDPOINT="${E2E_ISSUER}/token"
export OIDC_END_SESSION_ENDPOINT="${E2E_ISSUER}/logout"
export OIDC_COOKIE_SECURE="false"
# Automatic browser-session renewal. The store key encrypts refresh tokens at
# rest; the internal secret authenticates the Next.js server to the Rust API.
export OIDC_REVOCATION_ENDPOINT="${E2E_ISSUER}/revoke"
export WEB_SESSION_SECRET="${WEB_SESSION_SECRET:-$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')}"
export SESSION_STORE_KEY="${SESSION_STORE_KEY:-$(python3 -c 'import base64, os; print(base64.urlsafe_b64encode(os.urandom(32)).decode().rstrip("="))')}"
export SESSION_IDLE_SECONDS="${SESSION_IDLE_SECONDS:-1800}"
export SESSION_ABSOLUTE_SECONDS="${SESSION_ABSOLUTE_SECONDS:-28800}"
export SESSION_REFRESH_WINDOW_SECONDS="${SESSION_REFRESH_WINDOW_SECONDS:-60}"
export HOF_THIRD_BRANCH_KEY="branch-3"
# The fake adapter proves the send gate without real email. The Rust
# integration suite separately proves the unconfigured adapter fails closed.
export ONBOARDING_MAIL_ADAPTER="${ONBOARDING_MAIL_ADAPTER:-fake}"
export ONBOARDING_ACTIVATION_URL="${ONBOARDING_ACTIVATION_URL:-${WEB_ORIGIN}/activate}"
export ONBOARDING_SEND_INTERVAL_MS="${ONBOARDING_SEND_INTERVAL_MS:-0}"

echo "web-e2e: starting the test OIDC issuer"
python3 scripts/e2e-auth.py serve --port "$AUTH_PORT" --key-file "$E2E_KEY_FILE" >/tmp/tran-e2e-auth.log 2>&1 &
AUTH_PID=$!
# Until the full cleanup below is installed, an early failure (for example an
# unreachable database during seeding) must not leave the issuer running.
early_cleanup() {
	kill "$AUTH_PID" 2>/dev/null || true
	wait "$AUTH_PID" 2>/dev/null || true
	rm -f "$E2E_KEY_FILE"
}
trap early_cleanup EXIT INT TERM
i=0
while [ "$i" -lt 50 ]; do
	# Check our own process first, so a foreign server on the port can never
	# satisfy the readiness probe.
	if ! kill -0 "$AUTH_PID" 2>/dev/null; then
		echo "web-e2e: the test OIDC issuer exited during startup:" >&2
		cat /tmp/tran-e2e-auth.log >&2 || true
		exit 1
	fi
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
if ! require_own_listener "web-e2e: test OIDC issuer" "$AUTH_PID" "$AUTH_PORT"; then
	kill "$AUTH_PID" 2>/dev/null || true
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
E2E_DEACTIVATED_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-deactivated --role viewer --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_DEACTIVATED_TOKEN
# A dedicated branch reviewer: a linked viewer with no administrator role. The
# branch grant itself is created by the branch-review spec through the admin API.
E2E_REVIEWER_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-reviewer --role viewer --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_REVIEWER_TOKEN
# A dedicated reviewer identity the security suite deactivates, so the shared
# reviewer used by the other branch-review specs keeps its access.
E2E_REVIEWER_OFF_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-reviewer-off --role viewer --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_REVIEWER_OFF_TOKEN
# A distinctive synthetic value stored as an approved, owner-private profile
# value for the reviewed person. The security suite proves this exact value is
# absent from every branch review response and page.
E2E_PRIVATE_PROFILE_MARKER="ZX-PRIVATE-PROFILE-MARKER-4f21"
export E2E_PRIVATE_PROFILE_MARKER
# An approved family-visible biography for the same synthetic person. The family
# profile suite proves it is returned while the private value is not.
E2E_FAMILY_PROFILE_MARKER="ZX-FAMILY-PROFILE-BIOGRAPHY-7c14"
export E2E_FAMILY_PROFILE_MARKER
# An unsubmitted draft for the same person. The family profile page must never
# render it.
E2E_FAMILY_DRAFT_MARKER="ZX-FAMILY-PROFILE-DRAFT-3a02"
export E2E_FAMILY_DRAFT_MARKER
# A dedicated linked member the family profile suite revokes and deactivates.
E2E_FAMILY_MEMBER_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-family-member --role viewer --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_FAMILY_MEMBER_TOKEN
# Dedicated owner for the privacy-restriction scenario.
E2E_RESTRICT_OWNER_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-restrict-owner --role viewer --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_RESTRICT_OWNER_TOKEN
E2E_RESTRICTABLE_MARKER="ZX-RESTRICTABLE-BIOGRAPHY-5b38"
export E2E_RESTRICTABLE_MARKER
# Invitation flow: two identities that are deliberately NOT provisioned at seed
# time, so the activation path has to provision them. One invitation drives the
# API flow and one drives the browser flow. Nothing is ever emailed.
E2E_INVITEE_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-invitee --role viewer --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_INVITEE_TOKEN
E2E_INVITEE_TWO_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-invitee-2 --role viewer --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_INVITEE_TWO_TOKEN
E2E_INVITATION_TOKEN=$(python3 -c 'import secrets; print(secrets.token_hex(32))')
export E2E_INVITATION_TOKEN
E2E_INVITATION_TWO_TOKEN=$(python3 -c 'import secrets; print(secrets.token_hex(32))')
export E2E_INVITATION_TWO_TOKEN
export E2E_CORPUS="$CORPUS"

echo "web-e2e: seeding test identities"
# Base RAG schema and the FAM-002 document lifecycle metadata. These are
# idempotent; the document views must exist before the API starts.
for migration in 001_init 002_ingestion_metadata 003_add_language 004_page \
	005_add_corpus 009_document_lifecycle 010_document_security_hardening \
	011_browser_sessions; do
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-f "migrations/$migration.sql" >/dev/null
done
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/006_identity.sql >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/007_user_lifecycle.sql >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/008_identity_issuer_subject.sql >/dev/null
# No password-reset provider in E2E: the reset action must fail closed.
unset PASSWORD_RESET_URL PASSWORD_RESET_TOKEN
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "DELETE FROM users WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver', 'e2e-deactivated', 'e2e-member', 'e2e-reviewer', 'e2e-reviewer-off', 'e2e-family-member', 'e2e-restrict-owner', 'e2e-invitee', 'e2e-invitee-2')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO users (subject, issuer) VALUES ('e2e-admin', '$E2E_ISSUER'), ('e2e-viewer', '$E2E_ISSUER'), ('e2e-approver', '$E2E_ISSUER'), ('e2e-deactivated', '$E2E_ISSUER'), ('e2e-reviewer', '$E2E_ISSUER'), ('e2e-reviewer-off', '$E2E_ISSUER'), ('e2e-family-member', '$E2E_ISSUER'), ('e2e-restrict-owner', '$E2E_ISSUER')" >/dev/null
# A disposable account the admin accounts test changes and deactivates.
# The address uses the reserved .test domain; nothing is ever sent to it.
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO users (subject, issuer, email, display_name) VALUES ('e2e-member', '$E2E_ISSUER', 'e2e-member@example.test', 'Synthetic E2E Member')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO role_assignments (user_id, role, granted_by) SELECT id, 'admin', 'e2e' FROM users WHERE subject IN ('e2e-admin', 'e2e-approver')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO role_assignments (user_id, role, granted_by) SELECT id, 'viewer', 'e2e' FROM users WHERE subject IN ('e2e-viewer', 'e2e-reviewer', 'e2e-reviewer-off', 'e2e-family-member', 'e2e-restrict-owner')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO corpus_grants (user_id, corpus, granted_by) SELECT id, '$CORPUS', 'e2e' FROM users WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver', 'e2e-member', 'e2e-reviewer', 'e2e-reviewer-off', 'e2e-family-member', 'e2e-restrict-owner')" >/dev/null

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
for migration in \
	0007_onboarding \
	0008_onboarding_identity \
	0009_onboarding_link_audit \
	0010_onboarding_foreign_keys \
	0011_onboarding_foreign_key_reconciliation \
	0012_profile_revisions \
	0013_profile_revision_actor_scope \
	0014_profile_revision_draft_withdrawal \
	0015_profile_media \
	0016_profile_media_withdrawal \
	0017_profile_media_withdrawal_audit \
	0018_onboarding_batch \
	0019_branch_reviewer_grants \
	0020_branch_review_proposals \
	0021_profile_field_restriction \
	0022_profile_revision_restriction_generation \
	0023_document_extraction \
	0024_document_evidence_provenance \
	0025_assertion_dates \
	0026_authority_decisions \
	0027_source_narrative \
	0028_archive_photograph_inventory \
	0029_narrative_review_search \
	0030_lowercase_usernames \
	0031_archive_photograph_assets \
	0032_archive_photograph_link_provenance \
	0033_onboarding_invitation_contact_draft \
	0034_onboarding_invitation_delivery \
	0035_onboarding_link_request; do
	psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
		-f "genealogy/migrations/$migration.sql" >/dev/null
done
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-f scripts/seed-genealogy-e2e.sql >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO family_branch (branch_key, branch_label, branch_number) VALUES ('branch-1', 'First Branch', 1), ('branch-2', 'Second Branch', 2), ('branch-3', 'Third Branch', 3) ON CONFLICT (branch_key) DO NOTHING" >/dev/null
# Bulk onboarding synthetic fixtures. This reseeds only synthetic-bulk people,
# batch rows, invitations, and links; run it before the profile link seed below.
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-f scripts/seed-bulk-onboarding-e2e.sql >/dev/null

# USR-004D: seed synthetic profile members. No real family data is used. The
# links are inserted directly so the browser can exercise the real owner-scoped
# profile API; the viewer link is created and revoked by the browser spec itself.
echo "web-e2e: seeding synthetic profile members"
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "DELETE FROM person WHERE person_key IN ('e2e-profile-admin', 'e2e-profile-approver', 'e2e-profile-deactivated', 'e2e-profile-viewer')" >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO person (person_key, display_name) VALUES ('e2e-profile-admin', 'Synthetic E2E Admin Member'), ('e2e-profile-approver', 'Synthetic E2E Approver Member'), ('e2e-profile-deactivated', 'Synthetic E2E Deactivated Member'), ('e2e-profile-viewer', 'Synthetic E2E Viewer Member')" >/dev/null

E2E_ADMIN_ACCOUNT_ID=$(psql "$DATABASE_URL" -tAc "SELECT id FROM users WHERE subject = 'e2e-admin' AND issuer = '$E2E_ISSUER'")
E2E_APPROVER_ACCOUNT_ID=$(psql "$DATABASE_URL" -tAc "SELECT id FROM users WHERE subject = 'e2e-approver' AND issuer = '$E2E_ISSUER'")
E2E_DEACTIVATED_ACCOUNT_ID=$(psql "$DATABASE_URL" -tAc "SELECT id FROM users WHERE subject = 'e2e-deactivated' AND issuer = '$E2E_ISSUER'")
E2E_REVIEWER_ACCOUNT_ID=$(psql "$DATABASE_URL" -tAc "SELECT id FROM users WHERE subject = 'e2e-reviewer' AND issuer = '$E2E_ISSUER'")
E2E_REVIEWER_OFF_ACCOUNT_ID=$(psql "$DATABASE_URL" -tAc "SELECT id FROM users WHERE subject = 'e2e-reviewer-off' AND issuer = '$E2E_ISSUER'")
E2E_FAMILY_MEMBER_ACCOUNT_ID=$(psql "$DATABASE_URL" -tAc "SELECT id FROM users WHERE subject = 'e2e-family-member' AND issuer = '$E2E_ISSUER'")
E2E_RESTRICT_ACCOUNT_ID=$(psql "$DATABASE_URL" -tAc "SELECT id FROM users WHERE subject = 'e2e-restrict-owner' AND issuer = '$E2E_ISSUER'")
E2E_ADMIN_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-admin)
E2E_APPROVER_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-approver)
E2E_DEACTIVATED_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-deactivated)
E2E_REVIEWER_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-reviewer)
E2E_REVIEWER_OFF_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-reviewer-off)
E2E_FAMILY_MEMBER_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-family-member)
E2E_RESTRICT_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-restrict-owner)
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO onboarding_person_account_link (person_id, account_id, linked_by, identity_fingerprint) SELECT person_id, $E2E_ADMIN_ACCOUNT_ID, 'e2e-seed', '$E2E_ADMIN_FINGERPRINT' FROM person WHERE person_key = 'e2e-profile-admin'" >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO onboarding_person_account_link (person_id, account_id, linked_by, identity_fingerprint) SELECT person_id, $E2E_APPROVER_ACCOUNT_ID, 'e2e-seed', '$E2E_APPROVER_FINGERPRINT' FROM person WHERE person_key = 'e2e-profile-approver'" >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO onboarding_person_account_link (person_id, account_id, linked_by, identity_fingerprint) SELECT person_id, $E2E_DEACTIVATED_ACCOUNT_ID, 'e2e-seed', '$E2E_DEACTIVATED_FINGERPRINT' FROM person WHERE person_key = 'e2e-profile-deactivated'" >/dev/null

# Invitation activation fixture. Two unlinked people with a pending invitation,
# one per invitee token. Deleting the people cascades their invitations, link
# requests, and links, so every run exercises a first-time activation.
echo "web-e2e: seeding invitation activation fixtures"
E2E_INVITATION_HASH=$(python3 -c "import hashlib,sys;print(hashlib.sha256(sys.argv[1].encode()).hexdigest())" "$E2E_INVITATION_TOKEN")
E2E_INVITATION_TWO_HASH=$(python3 -c "import hashlib,sys;print(hashlib.sha256(sys.argv[1].encode()).hexdigest())" "$E2E_INVITATION_TWO_TOKEN")
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "DELETE FROM person WHERE person_key IN ('e2e-invitee-person', 'e2e-invitee-person-2')" >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO person (person_key, display_name) VALUES ('e2e-invitee-person', 'Synthetic Invitee Person'), ('e2e-invitee-person-2', 'Synthetic Invitee Person Two')" >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO onboarding_invitation (person_id, token_hash, expires_at, status, created_by) SELECT person_id, '$E2E_INVITATION_HASH', now() + interval '1 day', 'pending', 'e2e-seed' FROM person WHERE person_key = 'e2e-invitee-person'" >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO onboarding_invitation (person_id, token_hash, expires_at, status, created_by) SELECT person_id, '$E2E_INVITATION_TWO_HASH', now() + interval '1 day', 'pending', 'e2e-seed' FROM person WHERE person_key = 'e2e-invitee-person-2'" >/dev/null

# USR-005B: synthetic branch-review people with verified branch claims, plus the
# reviewer's own account-person link. Reuses the synthetic branch rows above.
#
# The narrative fixture must be deterministic even when an interrupted run left
# rows behind. The database is disposable, so clear the narrative tables before
# the seed recreates them. TRUNCATE bypasses the append-only DELETE triggers.
echo "web-e2e: resetting synthetic narrative fixtures"
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "TRUNCATE source_narrative_review_event, source_narrative_link, source_narrative_passage RESTART IDENTITY CASCADE" >/dev/null
echo "web-e2e: seeding branch-review fixture"
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-v reviewer_account_id="$E2E_REVIEWER_ACCOUNT_ID" \
	-v reviewer_fingerprint="$E2E_REVIEWER_FINGERPRINT" \
	-v off_account_id="$E2E_REVIEWER_OFF_ACCOUNT_ID" \
	-v off_fingerprint="$E2E_REVIEWER_OFF_FINGERPRINT" \
	-v family_account_id="$E2E_FAMILY_MEMBER_ACCOUNT_ID" \
	-v family_fingerprint="$E2E_FAMILY_MEMBER_FINGERPRINT" \
	-v marker="$E2E_PRIVATE_PROFILE_MARKER" \
	-v family_marker="$E2E_FAMILY_PROFILE_MARKER" \
	-v draft_marker="$E2E_FAMILY_DRAFT_MARKER" \
	-v restrict_account_id="$E2E_RESTRICT_ACCOUNT_ID" \
	-v restrict_fingerprint="$E2E_RESTRICT_FINGERPRINT" \
	-v restrict_marker="$E2E_RESTRICTABLE_MARKER" \
	-f scripts/seed-branch-review-e2e.sql >/dev/null
# The privacy assertion is only meaningful if the private value really exists.
# psql does not interpolate variables in `-c`, so the query comes from stdin.
# All three markers must exist, or the privacy and draft assertions would be
# testing nothing.
seeded_markers=$(psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-v marker="$E2E_PRIVATE_PROFILE_MARKER" \
	-v family_marker="$E2E_FAMILY_PROFILE_MARKER" \
	-v draft_marker="$E2E_FAMILY_DRAFT_MARKER" \
	-v restrict_marker="$E2E_RESTRICTABLE_MARKER" -tA <<'SQL'
SELECT (SELECT count(*) FROM onboarding_profile_value WHERE value_text = :'marker')
     + (SELECT count(*) FROM onboarding_profile_value WHERE value_text = :'family_marker')
     + (SELECT count(*) FROM onboarding_profile_revision WHERE new_value_text = :'draft_marker')
     + (SELECT count(*) FROM onboarding_profile_value WHERE value_text = :'restrict_marker');
SQL
)
if [ "$seeded_markers" != "4" ]; then
	echo "web-e2e: the profile fixtures were not all seeded (found ${seeded_markers:-none}, expected 4). An approved revision is immutable, so changed markers need fresh disposable databases." >&2
	exit 1
fi

# Intentionally override any inherited MEDIA_ROOT: browser tests must only write
# synthetic media into this run's disposable directory.
E2E_MEDIA_ROOT=$(mktemp -d)
export MEDIA_ROOT="$E2E_MEDIA_ROOT"
export E2E_API_URL="http://127.0.0.1:${API_PORT}"
export E2E_ISSUER="$E2E_ISSUER"
export E2E_ADMIN_ACCOUNT_ID E2E_APPROVER_ACCOUNT_ID E2E_DEACTIVATED_ACCOUNT_ID
export E2E_REVIEWER_ACCOUNT_ID
export E2E_REVIEWER_OFF_ACCOUNT_ID
export E2E_FAMILY_MEMBER_ACCOUNT_ID
export E2E_RESTRICT_ACCOUNT_ID

echo "web-e2e: building the API"
cargo build --locked --bin api

api_log=$(mktemp)
API_HOST=127.0.0.1 API_PORT="$API_PORT" target/debug/api >"$api_log" 2>&1 &
API_PID=$!

cleanup() {
	# Capture the run status first: every command below overwrites $?.
	status=$?
	kill "$API_PID" 2>/dev/null || true
	wait "$API_PID" 2>/dev/null || true
	kill "$AUTH_PID" 2>/dev/null || true
	wait "$AUTH_PID" 2>/dev/null || true
	rm -f "$E2E_KEY_FILE"
	rm -rf "$E2E_MEDIA_ROOT"
	./scripts/web-control.sh stop >/dev/null 2>&1 || true
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM documents WHERE corpus = '$CORPUS'" >/dev/null 2>&1 || true
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM audit_events WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver', 'e2e-deactivated', 'e2e-member', 'e2e-reviewer', 'e2e-reviewer-off', 'e2e-family-member', 'e2e-restrict-owner')" >/dev/null 2>&1 || true
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM users WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver', 'e2e-deactivated', 'e2e-member', 'e2e-reviewer', 'e2e-reviewer-off', 'e2e-family-member', 'e2e-restrict-owner')" >/dev/null 2>&1 || true
	# Profile members have no assertion review history, so they can be removed.
	psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM person WHERE person_key IN ('e2e-profile-admin', 'e2e-profile-approver', 'e2e-profile-deactivated', 'e2e-profile-viewer')" >/dev/null 2>&1 || true
	# Branch-review people keep their assertions, their append-only review
	# events, and the synthetic source: a person delete would cascade into
	# assertion_review_event, which refuses deletion. Remove only the rows the
	# next seed re-creates, so teardown is safe and the fixture stays idempotent.
	psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM genealogy_fact_proposal WHERE person_id IN (SELECT person_id FROM person WHERE person_key LIKE 'e2e-branch-%')" >/dev/null 2>&1 || true
	psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM onboarding_person_account_link WHERE person_id IN (SELECT person_id FROM person WHERE person_key LIKE 'e2e-branch-%')" >/dev/null 2>&1 || true
	psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM onboarding_profile_value WHERE person_id IN (SELECT person_id FROM person WHERE person_key LIKE 'e2e-branch-%')" >/dev/null 2>&1 || true
	# PH-202: synthetic narrative-review people and passage. Review events are
	# append-only and intentionally survive; the passage and links are disposable.
	psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM source_narrative_passage WHERE passage_key LIKE 'synthetic-e2e-narrative-review-%'" >/dev/null 2>&1 || true
	psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM source_narrative_passage WHERE passage_key LIKE 'synthetic-e2e-narrative-page-%' OR passage_key = 'synthetic-e2e-narrative-accented'" >/dev/null 2>&1 || true
	psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM person WHERE person_key LIKE 'e2e-narrative-review-%' OR person_key LIKE 'e2e-narrative-accent-%'" >/dev/null 2>&1 || true
	# Keep the logs when the run failed so the real API error survives. The
	# directory is git-ignored, so a kept log never shows up in git status.
	if [ "$status" -ne 0 ]; then
		keep_dir="$ROOT/web/test-results"
		mkdir -p "$keep_dir"
		if [ -f "$api_log" ]; then
			cp "$api_log" "$keep_dir/api.log" 2>/dev/null || true
			echo "web-e2e: API log kept at $keep_dir/api.log" >&2
			tail -n 40 "$api_log" >&2 2>/dev/null || true
		fi
		if [ -f /tmp/tran-e2e-auth.log ]; then
			cp /tmp/tran-e2e-auth.log "$keep_dir/auth.log" 2>/dev/null || true
			echo "web-e2e: test OIDC issuer log kept at $keep_dir/auth.log" >&2
			tail -n 40 /tmp/tran-e2e-auth.log >&2 2>/dev/null || true
		fi
		rm -f "$api_log"
	else
		rm -f "$api_log" /tmp/tran-e2e-auth.log
	fi
}
trap cleanup EXIT INT TERM

echo "web-e2e: waiting for the API health endpoint"
i=0
while [ "$i" -lt 60 ]; do
	# Liveness before health: if our API failed to bind, a foreign server on
	# the same port must not pass for it.
	if ! kill -0 "$API_PID" 2>/dev/null; then
		echo "web-e2e: API exited during startup:" >&2
		cat "$api_log" >&2
		exit 1
	fi
	if curl -fsS -m 2 -o /dev/null "http://127.0.0.1:$API_PORT/api/health"; then
		break
	fi
	sleep 1
	i=$((i+1))
done
if [ "$i" -ge 60 ]; then
	echo "web-e2e: API did not become healthy" >&2
	cat "$api_log" >&2
	exit 1
fi
if ! require_own_listener "web-e2e: API" "$API_PID" "$API_PORT"; then
	exit 1
fi

echo "web-e2e: starting Next.js"
./scripts/web-control.sh stop >/dev/null 2>&1 || true
./scripts/web-control.sh start

echo "web-e2e: running Playwright"
./scripts/web-control.sh e2e

echo "web-e2e: ok"
