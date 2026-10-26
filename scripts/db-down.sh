#!/bin/sh
# Stop the Docker database backend, if this checkout is using it.
#
#   scripts/db-down.sh
#
# Native PostgreSQL is managed by the operating system and is never stopped by
# this project. With DB_BACKEND unset, Docker is stopped only when its compose
# service is actually running in this checkout.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)

backend=${DB_BACKEND:-}
if [ -z "$backend" ]; then
	if ! command -v docker >/dev/null 2>&1 || ! docker compose version >/dev/null 2>&1; then
		backend=native
	elif [ -n "$(docker compose ps -q 2>/dev/null)" ]; then
		backend=docker
	else
		backend=native
	fi
fi

case "$backend" in
	docker)
		echo "db-down: stopping PostgreSQL (docker compose down)"
		docker compose down
		;;
	native)
		echo "db-down: native PostgreSQL is managed by the operating system; nothing to stop"
		;;
	*)
		echo "db-down: unknown DB_BACKEND '$backend' (expected native or docker)" >&2
		exit 1
		;;
esac
