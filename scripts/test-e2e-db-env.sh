#!/bin/sh
# Focused regression checks for scripts/e2e-db-env.sh.
#
#   scripts/test-e2e-db-env.sh
#
# No database, network, API, or browser is touched. Every case runs in this
# process and only inspects exported variables and guard exit codes.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=scripts/e2e-db-env.sh
. "$ROOT/scripts/e2e-db-env.sh"

failures=0
pass() { printf 'ok - %s\n' "$1"; }
fail() {
	printf 'not ok - %s\n' "$1" >&2
	failures=$((failures + 1))
}
expect_eq() { # description expected actual
	if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (expected '$2', got '$3')"; fi
}
expect_ok() { # description command...
	description=$1
	shift
	if "$@" >/dev/null 2>&1; then pass "$description"; else fail "$description"; fi
}
expect_refused() { # description command...
	description=$1
	shift
	if "$@" >/dev/null 2>&1; then fail "$description"; else pass "$description"; fi
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# --- caller values win over .env defaults -----------------------------------
cat >"$work/app.env" <<'ENV'
# A commented line, an app database, a quoted value, and a CRLF line.
DATABASE_URL=postgres://rag:rag@127.0.0.1:5435/rag?sslmode=disable
GENEALOGY_DATABASE_URL=postgres://rag:rag@127.0.0.1:5435/tran_genealogy?sslmode=disable
QUOTED="from file"
UNSET_ONLY=from-file
ENV
printf 'CRLF_ONLY=from-file\r\n' >>"$work/app.env"

DATABASE_URL='postgres://rag:rag@127.0.0.1:5435/rag_e2e_test?sslmode=disable'
export DATABASE_URL
# Make the fallback cases deterministic even if the caller has these set.
unset GENEALOGY_DATABASE_URL 2>/dev/null || true
unset QUOTED 2>/dev/null || true
unset UNSET_ONLY 2>/dev/null || true
unset CRLF_ONLY 2>/dev/null || true

load_env_defaults "$work/app.env"

expect_eq "an explicit caller DATABASE_URL is not overwritten by .env" \
	"postgres://rag:rag@127.0.0.1:5435/rag_e2e_test?sslmode=disable" "$DATABASE_URL"
expect_eq "an unset value is filled from .env" \
	"postgres://rag:rag@127.0.0.1:5435/tran_genealogy?sslmode=disable" "$GENEALOGY_DATABASE_URL"
expect_eq "quotes are stripped" "from file" "$QUOTED"
expect_eq "a plain value is exported" "from-file" "$UNSET_ONLY"
expect_eq "a CRLF line is exported without the CR" "from-file" "$CRLF_ONLY"

# An explicitly empty caller value is still the caller's choice; the runner's
# presence check reports it instead of silently retargeting the run.
EMPTY_ONLY=
export EMPTY_ONLY
printf 'EMPTY_ONLY=from-file\n' >"$work/empty.env"
load_env_defaults "$work/empty.env"
expect_eq "an explicitly empty caller value wins" "" "$EMPTY_ONLY"

# --- disposable-database guard ----------------------------------------------
expect_ok "a *_test database is accepted" \
	require_disposable_database "DATABASE_URL" "postgres://rag:rag@127.0.0.1:5435/rag_test?sslmode=disable"
expect_ok "a *_disposable database is accepted" \
	require_disposable_database "DATABASE_URL" "postgres://rag:rag@h/genealogy_disposable"
expect_refused "the development rag database is refused" \
	require_disposable_database "DATABASE_URL" "postgres://rag:rag@127.0.0.1:5435/rag?sslmode=disable"
expect_refused "the imported genealogy database is refused" \
	require_disposable_database "GENEALOGY_DATABASE_URL" "postgres://rag:rag@127.0.0.1:5435/tran_genealogy?sslmode=disable"
expect_refused "an empty database name is refused" \
	require_disposable_database "DATABASE_URL" "postgres://rag:rag@127.0.0.1:5435/"

# -- The two targets must differ ---------------------------------------------
expect_refused "the same database twice is refused" \
	require_distinct_databases \
	"postgres://rag:rag@h:5432/rag_test?sslmode=disable" \
	"postgres://other:other@h:5432/rag_test"
expect_ok "different databases are accepted" \
	require_distinct_databases \
	"postgres://rag:rag@h:5432/rag_test" \
	"postgres://rag:rag@h:5432/genealogy_test"

if [ "$failures" -ne 0 ]; then
	printf '\n%d check(s) failed\n' "$failures" >&2
	exit 1
fi
printf '\nall checks passed\n'
