#!/bin/sh
# Resolve, prepare, and run against disposable PostgreSQL test databases.
#
#   scripts/test-db.sh status
#   scripts/test-db.sh prepare
#   scripts/test-db.sh env
#   scripts/test-db.sh exec -- <command...>
#
# Explicit configuration wins:
#   DATABASE_URL / GENEALOGY_DATABASE_URL             app or test DSNs
#   TEST_DATABASE_URL / TEST_GENEALOGY_DATABASE_URL   explicit test DSN overrides
#
# When a DSN is absent the script probes a PostgreSQL server on the standard
# endpoints (native first on Linux, Docker first on macOS) and derives a
# disposable *_test database from it. It only creates databases and applies
# idempotent migrations; it never drops, truncates, or resets one.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=scripts/e2e-db-env.sh
. "$ROOT/scripts/e2e-db-env.sh"

# .env is a fallback only: values already exported by the caller win.
load_env_defaults "${DB_ENV_FILE:-$ROOT/.env}"

DB_HOST=${DB_HOST:-127.0.0.1}
DB_USER=${DB_USER:-rag}
DB_PASSWORD=${DB_PASSWORD:-rag}
PGCONNECT_TIMEOUT=${PGCONNECT_TIMEOUT:-3}
# Idempotent migrations emit NOTICE for every object they skip. Keep warnings
# and errors, but keep the validation log readable.
PGOPTIONS=${PGOPTIONS:-"-c client_min_messages=warning"}
export PGCONNECT_TIMEOUT PGOPTIONS

require_psql() {
	if ! command -v psql >/dev/null 2>&1; then
		echo "test-db: psql is required but was not found on PATH." >&2
		echo "test-db: install the PostgreSQL client (Linux) or use the Docker image (macOS)." >&2
		exit 1
	fi
}

# The database name is interpolated into SQL identifiers and a pg_database
# lookup. Refuse anything outside the safe identifier alphabet.
require_safe_name() {
	case "$1" in
		'' | *[!A-Za-z0-9_]*)
			echo "test-db: unsafe database name '$1'" >&2
			exit 1
			;;
	esac
}

probe_url() {
	printf 'postgres://%s:%s@%s:%s/postgres?sslmode=disable' \
		"$DB_USER" "$DB_PASSWORD" "$DB_HOST" "$1"
}

# Print the first reachable server authority. Linux prefers a native server on
# 5432; macOS prefers the Docker compose port; 5434 stays in the list because it
# is this repository's documented default.
detect_base() {
	case "$(uname -s 2>/dev/null || echo unknown)" in
		Darwin) ports=${DB_PROBE_PORTS:-"5434 5432 5435"} ;;
		*) ports=${DB_PROBE_PORTS:-"5432 5434 5435"} ;;
	esac
	for port in $ports; do
		if psql "$(probe_url "$port")" -w -Atqc 'select 1' >/dev/null 2>&1; then
			printf 'postgres://%s:%s@%s:%s' "$DB_USER" "$DB_PASSWORD" "$DB_HOST" "$port"
			return 0
		fi
	done
	return 1
}

test_dsn() {
	name=$(database_name "$1")
	case "$name" in
		*_test | *_disposable) printf '%s' "$1" ;;
		*) with_database "$1" "${name}_test" ;;
	esac
}

resolve() {
	rag=${DATABASE_URL:-}
	if [ -z "$rag" ]; then
		base=$(detect_base) || {
			echo "test-db: no PostgreSQL server is reachable and DATABASE_URL is unset." >&2
			echo "test-db: start native PostgreSQL (Linux) or 'docker compose up -d --wait' (macOS)," >&2
			echo "test-db: or export DATABASE_URL and GENEALOGY_DATABASE_URL (see .env.example)." >&2
			return 1
		}
		rag="$base/rag?sslmode=disable"
	fi
	genealogy=${GENEALOGY_DATABASE_URL:-}
	if [ -z "$genealogy" ]; then
		genealogy=$(with_database "$rag" genealogy)
	fi

	RAG_TEST_DSN=${TEST_DATABASE_URL:-$(test_dsn "$rag")}
	GENEALOGY_TEST_DSN=${TEST_GENEALOGY_DATABASE_URL:-$(test_dsn "$genealogy")}

	require_disposable_database "test RAG" "$RAG_TEST_DSN"
	require_disposable_database "test genealogy" "$GENEALOGY_TEST_DSN"
	require_distinct_databases "$RAG_TEST_DSN" "$GENEALOGY_TEST_DSN"
	require_safe_name "$(database_name "$RAG_TEST_DSN")"
	require_safe_name "$(database_name "$GENEALOGY_TEST_DSN")"
}

ensure_database() {
	dsn=$1
	name=$(database_name "$dsn")
	admin=$(with_database "$dsn" postgres)
	if [ "$(psql "$admin" -w -Atqc "select 1 from pg_database where datname = '$name'")" != "1" ]; then
		echo "test-db: creating $name" >&2
		psql "$admin" -w -v ON_ERROR_STOP=1 -qc "CREATE DATABASE \"$name\"" ||
			{
				echo "test-db: create $name failed; grant CREATEDB to the test role or create it manually" >&2
				exit 1
			}
	fi
}

apply_migrations() {
	dsn=$1
	dir=$2
	for file in "$dir"/*.sql; do
		[ -f "$file" ] || continue
		psql "$dsn" -w -v ON_ERROR_STOP=1 -q -f "$file" >/dev/null
	done
}

prepare() {
	require_psql
	resolve
	ensure_database "$RAG_TEST_DSN"
	ensure_database "$GENEALOGY_TEST_DSN"
	apply_migrations "$RAG_TEST_DSN" "$ROOT/migrations"
	apply_migrations "$GENEALOGY_TEST_DSN" "$ROOT/genealogy/migrations"
}

status() {
	require_psql
	resolve
	printf 'RAG test:       %s\n' "$(database_identity "$RAG_TEST_DSN")"
	printf 'Genealogy test: %s\n' "$(database_identity "$GENEALOGY_TEST_DSN")"
}

# Print shell assignments for callers that need the resolved DSNs. Values are
# single-quoted, and a DSN containing a single quote is refused so the output
# stays safe to eval.
env_output() {
	require_psql
	resolve
	for value in "$RAG_TEST_DSN" "$GENEALOGY_TEST_DSN"; do
		case "$value" in
			*\'*)
				echo "test-db: refusing to emit a DSN containing a single quote" >&2
				exit 1
				;;
		esac
		if [ "$(printf '%s' "$value" | wc -l | tr -d ' ')" -gt 0 ]; then
			echo "test-db: refusing to emit a DSN containing a newline" >&2
			exit 1
		fi
	done
	printf "DATABASE_URL='%s'\n" "$RAG_TEST_DSN"
	printf "GENEALOGY_DATABASE_URL='%s'\n" "$GENEALOGY_TEST_DSN"
	printf "REQUIRE_TEST_DATABASE=1\n"
}

command=${1:-}
if [ $# -gt 0 ]; then shift; fi
case "$command" in
	status) status ;;
	prepare) prepare ;;
	env) env_output ;;
	exec)
		if [ "${1:-}" = "--" ]; then shift; fi
		if [ $# -eq 0 ]; then
			echo "usage: scripts/test-db.sh exec -- <command...>" >&2
			exit 2
		fi
		prepare
		export DATABASE_URL="$RAG_TEST_DSN"
		export GENEALOGY_DATABASE_URL="$GENEALOGY_TEST_DSN"
		export REQUIRE_TEST_DATABASE=1
		exec "$@"
		;;
	*)
		echo "usage: scripts/test-db.sh status|prepare|env|exec -- <command...>" >&2
		exit 2
		;;
esac
