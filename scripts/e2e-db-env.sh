#!/bin/sh
# Database DSN and disposable-target guards shared by the test runners.
#
# Sourced by scripts/web-e2e.sh and scripts/test-db.sh. It lives in its own file
# so the rules can be exercised without starting PostgreSQL, Ollama, the API, or
# a browser.
#
# Rules:
#   * An explicit caller value always wins over the .env default.
#   * Every database the runner mutates must be disposable. The runner applies
#     migrations, seeds fixtures, and deletes rows, so it refuses any database
#     whose name does not end in _test or _disposable *before* the first write.
#   * The RAG and genealogy targets must be different databases.
#
# The runner never moves .env aside to isolate a run: isolation comes from
# passing disposable DSNs, and .env is only a fallback for values the caller
# did not set.

# load_env_defaults FILE
#
# Export the values in FILE that the caller has not already set. Variables that
# are already present in the environment are never overwritten, so
# `DATABASE_URL=... scripts/web-e2e.sh` keeps the caller's database even when
# .env defines a different one. This is deliberately not `set -a; . .env`:
# sourcing a file overwrites caller values and would silently retarget the run.
load_env_defaults() {
	file=$1
	[ -f "$file" ] || return 0
	while IFS= read -r line || [ -n "$line" ]; do
		# Drop a Windows CR, then any leading blanks, so indented comments and
		# CRLF files behave the same as plain ones.
		line=$(printf '%s' "$line" | tr -d '\r')
		trimmed=${line#"${line%%[![:space:]]*}"}
		case "$trimmed" in
			'' | \#*) continue ;;
		esac
		case "$trimmed" in
			*=*) ;;
			*) continue ;;
		esac
		key=${trimmed%%=*}
		value=${trimmed#*=}
		# Only names that are safe to hand to export.
		case "$key" in
			'' | *[!A-Za-z0-9_]*) continue ;;
		esac
		# An existing value, including an explicitly empty one, wins.
		if printenv "$key" >/dev/null 2>&1; then
			continue
		fi
		# Strip one layer of matching quotes, as a shell would.
		case "$value" in
			\"*\") value=${value#\"}; value=${value%\"} ;;
			\'*\') value=${value#\'}; value=${value%\'} ;;
		esac
		export "$key=$value"
	done <"$file"
}

# database_name DSN
#
# Print the database name, ignoring the query string.
database_name() {
	dsn=${1%%\?*}
	printf '%s' "${dsn##*/}"
}

# database_identity DSN
#
# Print `database@host:port`, ignoring userinfo and the query string, so two
# DSNs that reach the same database compare equal.
database_identity() {
	dsn=${1%%\?*}
	name=${dsn##*/}
	authority=${dsn%/*}
	authority=${authority##*@}
	printf '%s@%s' "$name" "$authority"
}

# with_database DSN NAME
#
# Replace the database name in DSN, preserving userinfo, host, port, and query.
with_database() {
	dsn=$1
	name=$2
	query=
	case "$dsn" in
		*\?*) query="?${dsn#*\?}"; dsn=${dsn%%\?*} ;;
	esac
	authority=${dsn%/*}
	printf '%s/%s%s' "$authority" "$name" "$query"
}

# require_disposable_database LABEL DSN
#
# Fail unless DSN names a disposable database. This is the guard that keeps a
# mis-set environment from migrating or deleting an application database.
require_disposable_database() {
	label=$1
	name=$(database_name "$2")
	if [ -z "$name" ]; then
		echo "web-e2e: $label must name a database explicitly (got '$2')" >&2
		return 1
	fi
	case "$name" in
		*_test | *_disposable) ;;
		*)
			echo "web-e2e: refusing $label database '$name': the E2E runner migrates, seeds, and deletes data, so it only accepts a disposable database whose name ends in _test or _disposable" >&2
			return 1
			;;
	esac
	return 0
}

# require_distinct_databases RAG_DSN GENEALOGY_DSN
#
# The runner writes to both databases, so they must not be the same one.
require_distinct_databases() {
	rag=$(database_identity "$1")
	genealogy=$(database_identity "$2")
	if [ "$rag" = "$genealogy" ]; then
		echo "web-e2e: DATABASE_URL and GENEALOGY_DATABASE_URL must be different databases (both are $rag)" >&2
		return 1
	fi
	return 0
}
