#!/bin/sh
# Focused regression checks for scripts/port-guard.sh.
#
#   scripts/test-port-guard.sh
#
# Starts one throwaway loopback listener (python3) and stops only that process.
# No database, API, browser, or other server is touched.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=scripts/port-guard.sh
. "$ROOT/scripts/port-guard.sh"

failures=0
pass() { printf 'ok - %s\n' "$1"; }
fail() {
	printf 'not ok - %s\n' "$1" >&2
	failures=$((failures + 1))
}

# Bind an ephemeral port, report it, then serve until killed.
ready=$(mktemp)
python3 - "$ready" <<'PY' &
import socket, sys, time
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 0))
s.listen(1)
with open(sys.argv[1], "w") as f:
    f.write(str(s.getsockname()[1]))
while True:
    time.sleep(1)
PY
listener=$!
trap 'kill "$listener" 2>/dev/null || true; rm -f "$ready"' EXIT INT TERM

i=0
while [ ! -s "$ready" ] && [ "$i" -lt 50 ]; do
	sleep 0.1
	i=$((i + 1))
done
busy=$(cat "$ready")
if [ -z "$busy" ]; then
	echo "test-port-guard: the throwaway listener did not start" >&2
	exit 1
fi

# A free port: bind and release one, so nothing listens there.
free=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')

if port_listening "$busy"; then pass "a listening port is detected"; else fail "a listening port is detected"; fi
if port_listening "$free"; then fail "a free port is not reported as listening"; else pass "a free port is not reported as listening"; fi

if message=$(require_port_free "web-e2e: API" "$busy" API_PORT 2>&1); then
	fail "a taken port is refused"
else
	pass "a taken port is refused"
	case "$message" in
		*"port $busy"*"already in use"*) pass "the refusal names the port" ;;
		*) fail "the refusal names the port (got: $message)" ;;
	esac
	case "$message" in
		*"API_PORT=<port>"*) pass "the refusal names the override variable" ;;
		*) fail "the refusal names the override variable (got: $message)" ;;
	esac
fi
if require_port_free "web-e2e: API" "$free" API_PORT >/dev/null 2>&1; then
	pass "a free port is accepted"
else
	fail "a free port is accepted"
fi

if kill -0 "$listener" 2>/dev/null; then
	pass "the guard leaves the existing listener running"
else
	fail "the guard leaves the existing listener running"
fi

# Ownership: the listener's own pid passes; any other pid is rejected when lsof
# can see listeners (skipped otherwise, matching the helper).
if [ -n "$(port_listener_pids "$busy")" ]; then
	if require_own_listener "web-e2e: API" "$listener" "$busy" >/dev/null 2>&1; then
		pass "the started process is accepted as the listener"
	else
		fail "the started process is accepted as the listener"
	fi
	if require_own_listener "web-e2e: API" "$$" "$busy" >/dev/null 2>&1; then
		fail "a different process holding the port is rejected"
	else
		pass "a different process holding the port is rejected"
	fi
else
	pass "listener ownership check skipped: lsof is unavailable"
fi

if [ "$failures" -ne 0 ]; then
	echo "test-port-guard: $failures check(s) failed" >&2
	exit 1
fi
echo "test-port-guard: all checks passed"
