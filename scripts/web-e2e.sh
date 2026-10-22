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
# A dedicated linked member the family profile suite revokes and deactivates.
E2E_FAMILY_MEMBER_TOKEN=$(python3 scripts/e2e-auth.py token --issuer "$E2E_ISSUER" --audience "$E2E_AUDIENCE" --subject e2e-family-member --role viewer --corpus "$CORPUS" --key-file "$E2E_KEY_FILE")
export E2E_FAMILY_MEMBER_TOKEN

echo "web-e2e: seeding test identities"
# Base RAG schema and the FAM-002 document lifecycle metadata. These are
# idempotent; the document views must exist before the API starts.
for migration in 001_init 002_ingestion_metadata 003_add_language 004_page \
	005_add_corpus 009_document_lifecycle 010_document_security_hardening; do
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-f "migrations/$migration.sql" >/dev/null
done
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/006_identity.sql >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/007_user_lifecycle.sql >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/008_identity_issuer_subject.sql >/dev/null
# No password-reset provider in E2E: the reset action must fail closed.
unset PASSWORD_RESET_URL PASSWORD_RESET_TOKEN
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "DELETE FROM users WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver', 'e2e-deactivated', 'e2e-member', 'e2e-reviewer', 'e2e-reviewer-off', 'e2e-family-member')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO users (subject, issuer) VALUES ('e2e-admin', '$E2E_ISSUER'), ('e2e-viewer', '$E2E_ISSUER'), ('e2e-approver', '$E2E_ISSUER'), ('e2e-deactivated', '$E2E_ISSUER'), ('e2e-reviewer', '$E2E_ISSUER'), ('e2e-reviewer-off', '$E2E_ISSUER'), ('e2e-family-member', '$E2E_ISSUER')" >/dev/null
# A disposable account the admin accounts test changes and deactivates.
# The address uses the reserved .test domain; nothing is ever sent to it.
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO users (subject, issuer, email, display_name) VALUES ('e2e-member', '$E2E_ISSUER', 'e2e-member@example.test', 'Synthetic E2E Member')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO role_assignments (user_id, role, granted_by) SELECT id, 'admin', 'e2e' FROM users WHERE subject IN ('e2e-admin', 'e2e-approver')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO role_assignments (user_id, role, granted_by) SELECT id, 'viewer', 'e2e' FROM users WHERE subject IN ('e2e-viewer', 'e2e-reviewer', 'e2e-reviewer-off', 'e2e-family-member')" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO corpus_grants (user_id, corpus, granted_by) SELECT id, '$CORPUS', 'e2e' FROM users WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver', 'e2e-member', 'e2e-reviewer', 'e2e-reviewer-off', 'e2e-family-member')" >/dev/null

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
	0020_branch_review_proposals; do
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
E2E_ADMIN_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-admin)
E2E_APPROVER_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-approver)
E2E_DEACTIVATED_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-deactivated)
E2E_REVIEWER_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-reviewer)
E2E_REVIEWER_OFF_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-reviewer-off)
E2E_FAMILY_MEMBER_FINGERPRINT=$(python3 -c "import hashlib,sys;print(hashlib.sha256((sys.argv[1]+chr(10)+sys.argv[2]).encode()).hexdigest())" "$E2E_ISSUER" e2e-family-member)
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO onboarding_person_account_link (person_id, account_id, linked_by, identity_fingerprint) SELECT person_id, $E2E_ADMIN_ACCOUNT_ID, 'e2e-seed', '$E2E_ADMIN_FINGERPRINT' FROM person WHERE person_key = 'e2e-profile-admin'" >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO onboarding_person_account_link (person_id, account_id, linked_by, identity_fingerprint) SELECT person_id, $E2E_APPROVER_ACCOUNT_ID, 'e2e-seed', '$E2E_APPROVER_FINGERPRINT' FROM person WHERE person_key = 'e2e-profile-approver'" >/dev/null
psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "INSERT INTO onboarding_person_account_link (person_id, account_id, linked_by, identity_fingerprint) SELECT person_id, $E2E_DEACTIVATED_ACCOUNT_ID, 'e2e-seed', '$E2E_DEACTIVATED_FINGERPRINT' FROM person WHERE person_key = 'e2e-profile-deactivated'" >/dev/null

# USR-005B: synthetic branch-review people with verified branch claims, plus the
# reviewer's own account-person link. Reuses the synthetic branch rows above.
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
	-f scripts/seed-branch-review-e2e.sql >/dev/null
# The privacy assertion is only meaningful if the private value really exists.
# psql does not interpolate variables in `-c`, so the query comes from stdin.
seeded_private=$(psql "$GENEALOGY_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-v marker="$E2E_PRIVATE_PROFILE_MARKER" -tA <<'SQL'
SELECT count(*) FROM onboarding_profile_value WHERE value_text = :'marker';
SQL
)
if [ "$seeded_private" != "1" ]; then
	echo "web-e2e: the private profile fixture was not seeded (found ${seeded_private:-none}). An approved revision is immutable, so a changed marker needs fresh disposable databases." >&2
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
	rm -rf "$E2E_MEDIA_ROOT"
	./scripts/web-control.sh stop >/dev/null 2>&1 || true
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM documents WHERE corpus = '$CORPUS'" >/dev/null 2>&1 || true
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM audit_events WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver', 'e2e-deactivated', 'e2e-member', 'e2e-reviewer', 'e2e-reviewer-off', 'e2e-family-member')" >/dev/null 2>&1 || true
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM users WHERE subject IN ('e2e-admin', 'e2e-viewer', 'e2e-approver', 'e2e-deactivated', 'e2e-member', 'e2e-reviewer', 'e2e-reviewer-off', 'e2e-family-member')" >/dev/null 2>&1 || true
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
