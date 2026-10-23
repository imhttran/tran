#!/bin/sh
# Local application lifecycle for the Tran family archive.
#
#   scripts/local-app.sh migrate [CATALOG]
#   scripts/local-app.sh import [CATALOG]
#   scripts/local-app.sh restart
#   scripts/local-app.sh update [CATALOG]
#   scripts/local-app.sh status
#   scripts/local-app.sh stop
#
# It loads the effective configuration from the environment, falling back to
# .env only for values the caller did not set, and never prints a DSN.
#
# Boundaries:
#   * It never starts, stops, or resets PostgreSQL, Keycloak, containers, or
#     stored data. Only the genealogy migrations and the idempotent catalog
#     import write data.
#   * It only stops API/UI processes it started (recorded in .run/api.pid and
#     web/.run/web.pid). A server it did not start is reported, never killed.
#   * It never switches branches, pulls, stashes, resets, or discards changes.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

# Shared, caller-wins .env loader and DSN helpers.
# shellcheck source=scripts/e2e-db-env.sh
. "$ROOT/scripts/e2e-db-env.sh"
load_env_defaults "$ROOT/.env"

CATALOG_DEFAULT="docs/tran/Tran-Family-People-Catalog.md"

RUN_DIR="$ROOT/.run"
API_PID_FILE="$RUN_DIR/api.pid"
API_LOG_FILE="$RUN_DIR/api.log"
API_BIN="$ROOT/target/debug/api"

API_HOST="${API_HOST:-127.0.0.1}"
API_PORT="${API_PORT:-8080}"
API_URL="${API_URL:-http://$API_HOST:$API_PORT}"
API_HEALTH="${API_URL%/}/api/health"

WEB_HOST="${WEB_HOST:-127.0.0.1}"
WEB_PORT="${WEB_PORT:-3000}"
WEB_URL="http://$WEB_HOST:$WEB_PORT"

say() { printf '%s\n' "$*"; }
warn() { printf 'local-app: %s\n' "$*" >&2; }
die() { printf 'local-app: %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 ||
	die "curl is required to check the API and UI health endpoints"

read_pid() { if [ -f "$1" ]; then cat "$1" 2>/dev/null; fi; }
alive() { if [ -n "${1:-}" ] && kill -0 "$1" 2>/dev/null; then return 0; fi; return 1; }

# Print how to stop a process these scripts did not start. Never kill it.
report_foreign_server() { # label port
	warn "$1: a server is already listening on port $2 that these scripts did not start."
	if command -v lsof >/dev/null 2>&1; then
		warn "Stop it yourself and re-run, for example:"
		warn "  lsof -nP -iTCP:$2 -sTCP:LISTEN"
		warn "  kill <pid>"
	else
		warn "Stop the process you started (for example Ctrl-C in its terminal), then re-run."
	fi
}

# --- configuration ----------------------------------------------------------

require_databases() {
	[ -n "${DATABASE_URL:-}" ] ||
		die "DATABASE_URL is not set. Set it in the environment or in .env."
	[ -n "${GENEALOGY_DATABASE_URL:-}" ] ||
		die "GENEALOGY_DATABASE_URL is not set. The genealogy database is never derived from DATABASE_URL; set it explicitly in the environment or in .env."
	rag=$(database_identity "$DATABASE_URL")
	genealogy=$(database_identity "$GENEALOGY_DATABASE_URL")
	[ "$rag" != "$genealogy" ] ||
		die "DATABASE_URL and GENEALOGY_DATABASE_URL must be different databases (both are $rag)"
}

probe_database() { # label dsn variable-name
	if ! command -v psql >/dev/null 2>&1; then
		warn "psql not found; skipping the $1 connectivity probe (${3} is still validated by the command that uses it)"
		return 0
	fi
	if ! psql "$2" -Atqc 'select 1' >/dev/null 2>&1; then
		die "cannot reach the $1 database configured by ${3}. Check the server and the DSN; no credentials are printed."
	fi
}

check_config() {
	require_databases
	probe_database "RAG" "$DATABASE_URL" "DATABASE_URL"
	probe_database "genealogy" "$GENEALOGY_DATABASE_URL" "GENEALOGY_DATABASE_URL"
}

require_catalog() {
	catalog=${1:-$CATALOG_DEFAULT}
	[ -f "$catalog" ] ||
		die "catalog not found: $catalog. Set CATALOG=... to choose another file."
	printf '%s\n' "$catalog"
}

# --- data -------------------------------------------------------------------

run_migrations() {
	require_databases
	say "applying genealogy migrations"
	cargo run --locked --bin genealogy -- migrate
}

run_import() { # catalog
	catalog=$(require_catalog "${1:-}")
	require_databases
	say "importing $catalog"
	say "(idempotent: person identity, citations, namesakes, and review decisions are preserved)"
	cargo run --locked --bin genealogy -- import --apply "$catalog"
}

build_api() {
	say "building the API"
	cargo build --locked --bin api
}

# --- API process ------------------------------------------------------------

api_responds() { curl -s -m 2 -o /dev/null "$API_HEALTH" 2>/dev/null; }

stop_api() {
	pid=$(read_pid "$API_PID_FILE")
	if [ -z "$pid" ]; then
		rm -f "$API_PID_FILE" 2>/dev/null || true
		return 0
	fi
	if ! alive "$pid"; then
		say "api: removing stale pid file"
		rm -f "$API_PID_FILE"
		return 0
	fi
	# With setsid the pid leads its own process group, so signal the group first
	# and fall back to the single process (macOS has no setsid).
	kill -TERM -"$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
	i=0
	while alive "$pid" && [ "$i" -lt 20 ]; do
		sleep 0.5
		i=$((i + 1))
	done
	if alive "$pid"; then
		kill -KILL -"$pid" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
	fi
	rm -f "$API_PID_FILE"
	say "api: stopped (pid $pid)"
}

start_api() {
	pid=$(read_pid "$API_PID_FILE")
	if alive "$pid"; then
		say "api: already running (pid $pid)"
		return 0
	fi
	if [ -n "$pid" ]; then rm -f "$API_PID_FILE"; fi
	if api_responds; then
		report_foreign_server "api" "$API_PORT"
		return 1
	fi
	if [ ! -x "$API_BIN" ]; then
		build_api || die "API build failed; not starting"
	fi
	mkdir -p "$RUN_DIR"
	# Run in its own session when `setsid` exists (so stop can signal the whole
	# group); macOS has no setsid, so fall back to a plain detached shell.
	# shellcheck disable=SC2016 # $1.. are expanded by the child shell.
	if command -v setsid >/dev/null 2>&1; then
		setsid sh -c 'echo $$ > "$1"; shift; exec "$@"' sh "$API_PID_FILE" "$API_BIN" \
			>"$API_LOG_FILE" 2>&1 &
	else
		sh -c 'echo $$ > "$1"; shift; exec "$@"' sh "$API_PID_FILE" "$API_BIN" \
			>"$API_LOG_FILE" 2>&1 &
	fi
	i=0
	while [ "$i" -lt 60 ]; do
		if api_responds; then
			say "api: started (pid $(read_pid "$API_PID_FILE"))"
			return 0
		fi
		if ! alive "$(read_pid "$API_PID_FILE")"; then
			warn "api: exited during startup; see $API_LOG_FILE"
			return 1
		fi
		sleep 1
		i=$((i + 1))
	done
	warn "api: did not become ready at $API_HEALTH; see $API_LOG_FILE"
	return 1
}

# --- UI process (owned by scripts/web-control.sh) ---------------------------

stop_web() { "$ROOT/scripts/web-control.sh" stop; }

start_web() {
	if ! "$ROOT/scripts/web-control.sh" start; then
		report_foreign_server "ui" "$WEB_PORT"
		return 1
	fi
}

web_status() { "$ROOT/scripts/web-control.sh" status; }

web_responds() { curl -s -m 2 -o /dev/null "$WEB_URL/" 2>/dev/null; }

# Fail before touching anything when a server we do not own occupies a port.
preflight_servers() {
	api_pid=$(read_pid "$API_PID_FILE")
	if ! alive "$api_pid" && api_responds; then
		report_foreign_server "api" "$API_PORT"
		return 1
	fi
	web_pid=$(read_pid "$ROOT/web/.run/web.pid")
	if ! alive "$web_pid" && web_responds; then
		report_foreign_server "ui" "$WEB_PORT"
		return 1
	fi
}

# --- commands ---------------------------------------------------------------

status_app() {
	say "api url: $API_URL"
	say "api log: $API_LOG_FILE"
	pid=$(read_pid "$API_PID_FILE")
	if alive "$pid"; then
		say "api: running (pid $pid)"
		if body=$(curl -s -m 2 "$API_HEALTH" 2>/dev/null); then
			say "api health: $body"
		else
			say "api health: no response"
		fi
	else
		say "api: not running"
	fi
	say "ui url: $WEB_URL"
	say "ui log: $ROOT/web/.run/web.log"
	web_status || true
}

restart_app() {
	preflight_servers || exit 1
	stop_api
	stop_web
	start_api || exit 1
	start_web || exit 1
}

cmd_migrate() {
	check_config
	run_migrations
}

cmd_import() {
	catalog=$(require_catalog "${1:-}")
	require_databases
	probe_database "genealogy" "$GENEALOGY_DATABASE_URL" "GENEALOGY_DATABASE_URL"
	run_import "$catalog"
}

cmd_restart() {
	require_databases
	restart_app
	status_app
}

cmd_update() {
	catalog=$(require_catalog "${1:-}")
	check_config
	say "configuration ok"
	run_migrations || die "genealogy migration failed; stopping before import"
	run_import "$catalog" || die "catalog import failed; stopping before restart"
	build_api || die "API build failed; stopping before restart"
	restart_app
	status_app
}

cmd_stop() {
	stop_api
	stop_web
}

usage() {
	cat >&2 <<'USAGE'
usage: scripts/local-app.sh <command> [catalog]

  migrate            apply genealogy migrations
  import [catalog]   import the catalog (default docs/tran/Tran-Family-People-Catalog.md)
  restart            restart only the API/UI processes these scripts own
  update [catalog]   validate, migrate, import, rebuild, restart, and report status
  status             report API/UI state, URLs, health, and log paths
  stop               stop only the API/UI processes these scripts own
USAGE
	exit 2
}

case "${1:-}" in
	migrate) shift; cmd_migrate ;;
	import) shift; cmd_import "${1:-}" ;;
	restart) shift; cmd_restart ;;
	update) shift; cmd_update "${1:-}" ;;
	status) shift; status_app ;;
	stop) shift; cmd_stop ;;
	*) usage ;;
esac
