#!/bin/sh
# Configure the local Tran Keycloak realm to send mail to Mailpit.
#
#   scripts/keycloak-mailpit-smtp.sh            # configure
#   scripts/keycloak-mailpit-smtp.sh --show     # read-only
#   scripts/keycloak-mailpit-smtp.sh --revert   # restore the saved SMTP map
#
# Only `smtpServer.*` on the existing realm is changed. The script never
# enables self-registration and never changes users, roles, clients, or flows.
# The Keycloak admin password stays inside the container's bootstrap-admin
# environment; it is never printed or placed in the host shell history.
#
# The mail sink stays local. No relay or forwarding is configured.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

CONTAINER=${KEYCLOAK_CONTAINER:-tran-keycloak}
REALM=${KEYCLOAK_REALM:-tran}
MAILPIT_HOST=${MAILPIT_SMTP_HOST:-host.docker.internal}
MAILPIT_PORT=${MAILPIT_SMTP_PORT:-1025}
MAILPIT_FROM=${MAILPIT_SMTP_FROM:-tran@example.test}
KCADM=/opt/keycloak/bin/kcadm.sh
BACKUP=${KEYCLOAK_SMTP_BACKUP:-.run/keycloak-smtp-before.json}

say() { printf '%s\n' "$*"; }
die() { printf 'keycloak-mailpit-smtp: %s\n' "$*" >&2; exit 1; }

command -v docker >/dev/null 2>&1 || die "docker is required"
docker inspect "$CONTAINER" >/dev/null 2>&1 || die "container not found: $CONTAINER"

login() {
	# $0 is the kcadm path passed after the shell string. The container's own
	# bootstrap-admin variables supply the password.
	docker exec "$CONTAINER" sh -c '
		exec "$0" config credentials \
			--server http://127.0.0.1:8080 \
			--realm master \
			--user "${KC_BOOTSTRAP_ADMIN_USERNAME:-admin}" \
			--password "${KC_BOOTSTRAP_ADMIN_PASSWORD:?KC_BOOTSTRAP_ADMIN_PASSWORD is not set in the container}"
	' "$KCADM"
}

show() {
	login
	# `--fields smtpServer` renders map values as `{}`, so read the realm and
	# print only the two relevant keys.
	docker exec "$CONTAINER" "$KCADM" get "realms/$REALM" | python3 -c '
import json, sys
realm = json.load(sys.stdin)
print(json.dumps({
    "smtpServer": realm.get("smtpServer", {}),
    "registrationAllowed": realm.get("registrationAllowed"),
}, indent=2))
'
}

configure() {
	login
	mkdir -p "$(dirname "$BACKUP")"
	docker exec "$CONTAINER" "$KCADM" get "realms/$REALM" >"$BACKUP"
	say "saved previous SMTP settings to $BACKUP"

	smtp_json=$(printf '{"host":"%s","port":"%s","from":"%s","fromDisplayName":"Tran local","auth":"false","starttls":"false","ssl":"false"}' \
		"$MAILPIT_HOST" "$MAILPIT_PORT" "$MAILPIT_FROM")
	docker exec "$CONTAINER" "$KCADM" update "realms/$REALM" -s "smtpServer=$smtp_json"
	say "configured $REALM SMTP host=$MAILPIT_HOST port=$MAILPIT_PORT (plaintext local sink)"
	show
}

revert() {
	[ -f "$BACKUP" ] || die "no saved settings at $BACKUP"
	previous=$(python3 -c 'import json,sys; print(json.dumps(json.load(open(sys.argv[1])).get("smtpServer", {}), separators=(",",":")))' "$BACKUP")
	login
	if [ "$previous" = "{}" ]; then
		docker exec "$CONTAINER" "$KCADM" update "realms/$REALM" -d smtpServer
		say "removed the SMTP map; the realm is back to its previous empty value"
	else
		docker exec "$CONTAINER" "$KCADM" update "realms/$REALM" -s "smtpServer=$previous"
		say "restored the previous SMTP map"
	fi
	show
}

case "${1:-}" in
	'') configure ;;
	--show) show ;;
	--revert) revert ;;
	*) die "usage: $0 [--show|--revert]" ;;
esac
