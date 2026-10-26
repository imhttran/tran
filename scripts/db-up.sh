#!/bin/sh
# Start the database backend this checkout is configured to use and apply the
# RAG schema to it.
#
#   scripts/db-up.sh
#
# Explicit configuration wins:
#   DB_BACKEND=docker  start docker compose and wait for health
#   DB_BACKEND=native  require an already-running native PostgreSQL and do not
#                      start or manage it
#
# With DB_BACKEND unset, an already-reachable server is left alone; otherwise
# macOS defaults to Docker and Linux defaults to native PostgreSQL. Native
# PostgreSQL is never started by this project on Linux, and Docker is never
# required there.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=scripts/e2e-db-env.sh
. "$ROOT/scripts/e2e-db-env.sh"

load_env_defaults "${DB_ENV_FILE:-$ROOT/.env}"

DB_HOST=${DB_HOST:-127.0.0.1}
DB_USER=${DB_USER:-rag}
DB_PASSWORD=${DB_PASSWORD:-rag}
PGCONNECT_TIMEOUT=${PGCONNECT_TIMEOUT:-3}
PGOPTIONS=${PGOPTIONS:-"-c client_min_messages=warning"}
export PGCONNECT_TIMEOUT PGOPTIONS

require_psql() {
	command -v psql >/dev/null 2>&1 || {
		echo "db-up: psql is required to apply the schema but was not found on PATH." >&2
		exit 1
	}
}

probe_url() {
	printf 'postgres://%s:%s@%s:%s/postgres?sslmode=disable' \
		"$DB_USER" "$DB_PASSWORD" "$DB_HOST" "$1"
}

# Print the application DSN. An explicit DATABASE_URL wins; otherwise probe the
# standard endpoints (native first on Linux, Docker first on macOS) and use the
# `rag` database on the first reachable server.
resolve_dsn() {
	if [ -n "${DATABASE_URL:-}" ]; then
		printf '%s' "$DATABASE_URL"
		return 0
	fi
	case "$(uname -s 2>/dev/null || echo unknown)" in
		Darwin) ports=${DB_PROBE_PORTS:-"5434 5432 5435"} ;;
		*) ports=${DB_PROBE_PORTS:-"5432 5434 5435"} ;;
	esac
	for port in $ports; do
		if psql "$(probe_url "$port")" -w -Atqc 'select 1' >/dev/null 2>&1; then
			printf 'postgres://%s:%s@%s:%s/rag?sslmode=disable' \
				"$DB_USER" "$DB_PASSWORD" "$DB_HOST" "$port"
			return 0
		fi
	done
	return 1
}

apply_schema() {
	dsn=$1
	name=$(database_name "$dsn")
	case "$name" in
		'' | *[!A-Za-z0-9_]*)
			echo "db-up: unsafe database name '$name' in DATABASE_URL" >&2
			exit 1
			;;
	esac
	admin=$(with_database "$dsn" postgres)
	if [ "$(psql "$admin" -w -Atqc "select 1 from pg_database where datname = '$name'")" != "1" ]; then
		echo "db-up: creating database $name"
		psql "$admin" -w -v ON_ERROR_STOP=1 -qc "CREATE DATABASE \"$name\""
	fi
	for file in "$ROOT"/migrations/*.sql; do
		[ -f "$file" ] || continue
		psql "$dsn" -w -v ON_ERROR_STOP=1 -q -f "$file" >/dev/null
	done
	printf 'db-up: schema applied to %s\n' "$(database_identity "$dsn")"
}

require_psql
dsn=$(resolve_dsn 2>/dev/null || true)
if [ -n "$dsn" ]; then
	printf 'db-up: PostgreSQL is already reachable at %s; nothing to start\n' \
		"$(database_identity "$dsn")"
	apply_schema "$dsn"
	exit 0
fi

backend=${DB_BACKEND:-}
if [ -z "$backend" ]; then
	case "$(uname -s 2>/dev/null || echo unknown)" in
		Darwin) backend=docker ;;
		*) backend=native ;;
	esac
fi

case "$backend" in
	docker)
		if ! command -v docker >/dev/null 2>&1; then
			echo "db-up: DB_BACKEND=docker but docker is not installed" >&2
			exit 1
		fi
		if ! docker compose version >/dev/null 2>&1; then
			echo "db-up: DB_BACKEND=docker but 'docker compose' is unavailable" >&2
			exit 1
		fi
		echo "db-up: starting PostgreSQL with docker compose"
		docker compose up -d --wait
		;;
	native)
		echo "db-up: DB_BACKEND=native but no PostgreSQL server is reachable." >&2
		echo "db-up: start PostgreSQL 16 and set DATABASE_URL/GENEALOGY_DATABASE_URL," >&2
		echo "db-up: or set DB_BACKEND=docker to use docker compose." >&2
		exit 1
		;;
	*)
		echo "db-up: unknown DB_BACKEND '$backend' (expected native or docker)" >&2
		exit 1
		;;
esac

dsn=$(resolve_dsn) || {
	echo "db-up: the backend started but no PostgreSQL server is reachable" >&2
	exit 1
}
apply_schema "$dsn"
