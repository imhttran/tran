# shellcheck shell=sh
# Shared loopback port checks for the scripts that start local servers.
#
# Sourced, not executed. A port counts as taken when any available method sees
# a listener: lsof, ss, or a direct connect probe. The probe matters because
# lsof without root only lists the caller's own processes, and macOS has no ss.
# Nothing here stops, kills, or signals a process.

# port_listening PORT
# Succeeds when something accepts TCP connections on 127.0.0.1:PORT.
port_listening() {
	_pg_port=$1
	if command -v lsof >/dev/null 2>&1 &&
		[ -n "$(lsof -nP -iTCP:"$_pg_port" -sTCP:LISTEN -t 2>/dev/null)" ]; then
		return 0
	fi
	if command -v ss >/dev/null 2>&1 &&
		ss -ltn 2>/dev/null | grep -qE "[:.]${_pg_port}[[:space:]]"; then
		return 0
	fi
	if command -v python3 >/dev/null 2>&1; then
		python3 - "$_pg_port" <<'PY' && return 0
import socket, sys
s = socket.socket()
s.settimeout(1)
try:
    s.connect(("127.0.0.1", int(sys.argv[1])))
except OSError:
    sys.exit(1)
finally:
    s.close()
PY
	elif command -v nc >/dev/null 2>&1; then
		nc -z -w 1 127.0.0.1 "$_pg_port" >/dev/null 2>&1 && return 0
	fi
	return 1
}

# port_listener_pids PORT
# Prints the PIDs listening on PORT, one per line, when lsof can see them.
port_listener_pids() {
	if command -v lsof >/dev/null 2>&1; then
		lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null || true
	fi
}

# require_port_free LABEL PORT OVERRIDE_VAR
# Fails with a clear message when PORT is already taken. Never touches the
# process that holds it.
require_port_free() {
	_pg_label=$1
	_pg_port=$2
	_pg_var=$3
	if ! port_listening "$_pg_port"; then
		return 0
	fi
	echo "$_pg_label: port $_pg_port on 127.0.0.1 is already in use by another process." >&2
	_pg_pids=$(port_listener_pids "$_pg_port")
	if [ -n "$_pg_pids" ]; then
		for _pg_pid in $_pg_pids; do
			echo "  pid $_pg_pid: $(ps -o command= -p "$_pg_pid" 2>/dev/null || echo unknown)" >&2
		done
	fi
	echo "  Refusing to start: requests would reach that process instead of this run's server." >&2
	echo "  Stop it yourself, or choose a free port with $_pg_var=<port>." >&2
	return 1
}

# require_own_listener LABEL PID PORT
# Fails when lsof shows PORT held by processes that do not include PID. Skipped
# (succeeds) when lsof is unavailable or sees no listener.
require_own_listener() {
	_pg_label=$1
	_pg_pid=$2
	_pg_port=$3
	_pg_pids=$(port_listener_pids "$_pg_port")
	if [ -z "$_pg_pids" ]; then
		return 0
	fi
	for _pg_seen in $_pg_pids; do
		if [ "$_pg_seen" = "$_pg_pid" ]; then
			return 0
		fi
	done
	echo "$_pg_label: port $_pg_port is held by pid(s) $(echo "$_pg_pids" | tr '\n' ' ')but this run started pid $_pg_pid." >&2
	return 1
}
