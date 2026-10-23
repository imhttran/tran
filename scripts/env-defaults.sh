#!/bin/sh
# Write a filtered copy of an env file for `make` to include.
#
#   scripts/env-defaults.sh .env .run/make-defaults.env
#
# A makefile assignment beats the environment, so `-include .env` would replace
# values the caller exported (for example `API_URL=... make app-update`). This
# helper copies every line whose key is *not* already present in the process
# environment, leaving caller values untouched. It prints the path it wrote.
#
# Values are copied verbatim (quotes, `=`, spaces, and `#` included), so nothing
# is re-expanded or reinterpreted by make.
set -eu

# The output can contain the same secrets as .env; keep it private.
umask 077

source_file=${1:?usage: env-defaults.sh <source> <output>}
output_file=${2:?usage: env-defaults.sh <source> <output>}

mkdir -p "$(dirname "$output_file")"
: >"$output_file"

if [ ! -f "$source_file" ]; then
	printf '%s\n' "$output_file"
	exit 0
fi

while IFS= read -r line || [ -n "$line" ]; do
	line=$(printf '%s' "$line" | tr -d '\r')
	trimmed=${line#"${line%%[![:space:]]*}"}
	case "$trimmed" in
		'' | \#*)
			printf '%s\n' "$line" >>"$output_file"
			continue
			;;
	esac
	case "$trimmed" in
		*=*) ;;
		*)
			printf '%s\n' "$line" >>"$output_file"
			continue
			;;
	esac
	key=${trimmed%%=*}
	case "$key" in
		'' | *[!A-Za-z0-9_]*)
			printf '%s\n' "$line" >>"$output_file"
			continue
			;;
	esac
	# An existing value, including an explicitly empty one, wins.
	if printenv "$key" >/dev/null 2>&1; then
		continue
	fi
	printf '%s\n' "$line" >>"$output_file"
done <"$source_file"

chmod 600 "$output_file" 2>/dev/null || true
printf '%s\n' "$output_file"
