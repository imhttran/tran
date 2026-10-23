#!/bin/sh
# Explicit initial-administrator setup.
#
#   make admin-setup KEYCLOAK_USERNAME=alice
#   scripts/admin-setup.sh --username alice [--corpus trusted] [--dry-run]
#
# The wrapper loads the effective configuration with the same caller-wins rule
# as the rest of the local scripts, then hands off to admin_setup.py. It never
# runs as part of app-update, and it never writes genealogy data or creates an
# account-to-person link.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

if ! command -v python3 >/dev/null 2>&1; then
	echo "admin-setup: python3 is required" >&2
	exit 1
fi

# Caller values win; .env only fills the gaps.
# shellcheck source=scripts/e2e-db-env.sh
. "$ROOT/scripts/e2e-db-env.sh"
load_env_defaults "$ROOT/.env"

exec python3 "$ROOT/scripts/admin_setup.py" "$@"
