#!/bin/sh
# Run the project's full validation with portable PostgreSQL test databases.
#
#   scripts/validate.sh
#
# The script resolves and prepares disposable test databases, exports them to
# every check, and records stdout/stderr in a stable log. It runs each configured
# command once (no duplicate full-suite runs), stops at the first failure, and
# exits with that command's status.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
LOG_DIR=${VALIDATION_LOG_DIR:-$ROOT/.run/validation}
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
LOG="$LOG_DIR/latest.log"
ARCHIVE="$LOG_DIR/validation-$STAMP.log"

mkdir -p "$LOG_DIR"
: >"$LOG"
trap 'cp "$LOG" "$ARCHIVE" 2>/dev/null || true' EXIT

cd "$ROOT"

# Resolve the disposable test DSNs. `env` prints single-quoted assignments only;
# credentials are never written to the log.
eval "$("$ROOT/scripts/test-db.sh" env)"
export DATABASE_URL GENEALOGY_DATABASE_URL REQUIRE_TEST_DATABASE
"$ROOT/scripts/test-db.sh" prepare

run() {
	printf '\n== %s\n' "$*" >>"$LOG"
	status=0
	"$@" >>"$LOG" 2>&1 || status=$?
	if [ "$status" -ne 0 ]; then
		printf 'validation: FAILED (exit %s): %s\n' "$status" "$*" >&2
		printf 'validation: log %s\n' "$LOG" >&2
		tail -n 40 "$LOG" >&2
		exit "$status"
	fi
}

run cargo build --locked
run cargo test --locked
run cargo fmt --check
run cargo clippy --locked --all-targets -- -D warnings
run make web-check
run make web-e2e
run make transition-evidence

printf '\nvalidation: PASS (log %s)\n' "$LOG"
