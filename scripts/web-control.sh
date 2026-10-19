#!/bin/sh
# Manage the Next.js prototype (UI-002) from the repository root.
#
#   scripts/web-control.sh check-env|install|typecheck|build|start|stop|status
#
# Every web command first resolves and validates the Node/npm toolchain through
# this one script, so discovery is never duplicated.
#
# Toolchain resolution order (no user name or single Zed version is hard-coded):
#   1. explicit NODE_BIN / NPM_BIN overrides
#   2. node / npm on PATH
#   3. Zed's bundled Node (any versioned install directory)
# Missing or unsupported tools stop with setup instructions; nothing is installed
# or upgraded automatically.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
WEB_DIR="$ROOT/web"
RUN_DIR="$WEB_DIR/.run"
PID_FILE="$RUN_DIR/web.pid"
LOG_FILE="$RUN_DIR/web.log"
HOST="${WEB_HOST:-127.0.0.1}"
PORT="${WEB_PORT:-3000}"
URL="http://$HOST:$PORT"

# zed_node_dirs lists candidate Zed node bin directories (newest first is not
# assumed; the caller takes the last match, which sorts newest by version).
zed_node_dirs() {
	for d in \
		"$HOME"/.local/share/zed/node/node-*/bin \
		"$HOME"/Library/Application\ Support/Zed/node/node-*/bin; do
		if [ -d "$d" ]; then printf '%s\n' "$d"; fi
	done
}

find_node() {
	if [ -n "${NODE_BIN:-}" ]; then
		if [ -x "$NODE_BIN" ]; then echo "$NODE_BIN"; return 0; fi
		echo "web: NODE_BIN is set but not executable: $NODE_BIN" >&2
		return 1
	fi
	if command -v node >/dev/null 2>&1; then command -v node; return 0; fi
	found=""
	for d in $(zed_node_dirs); do
		if [ -x "$d/node" ]; then found="$d/node"; fi
	done
	if [ -n "$found" ]; then echo "$found"; return 0; fi
	return 1
}

find_npm() {
	if [ -n "${NPM_BIN:-}" ]; then
		if [ -x "$NPM_BIN" ]; then echo "$NPM_BIN"; return 0; fi
		echo "web: NPM_BIN is set but not executable: $NPM_BIN" >&2
		return 1
	fi
	if command -v npm >/dev/null 2>&1; then command -v npm; return 0; fi
	found=""
	for d in $(zed_node_dirs); do
		if [ -x "$d/npm" ]; then found="$d/npm"; fi
	done
	if [ -n "$found" ]; then echo "$found"; return 0; fi
	return 1
}

# engines_range prints the declared "node" and "npm" engine ranges from
# web/package.json (two lines), using the resolved node to read the file.
engines_range() {
	# Pure-shell parse of the "engines" block only (independent of the resolved
	# node, so an incompatible/old node is still judged correctly).
	block=$(awk '/"engines"[[:space:]]*:/{f=1} f{print} f&&/}/{exit}' "$WEB_DIR/package.json")
	nr=$(printf '%s\n' "$block" | sed -n 's/.*"node"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
	pr=$(printf '%s\n' "$block" | sed -n 's/.*"npm"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
	printf '%s\n%s\n' "$nr" "$pr"
}
min_major() { printf '%s' "$1" | sed -n 's/.*>=\([0-9][0-9]*\).*/\1/p'; }
max_major() { printf '%s' "$1" | sed -n 's/.*<\([0-9][0-9]*\).*/\1/p'; }
major_of() { printf '%s' "$1" | sed -n 's/^[v]*\([0-9][0-9]*\).*/\1/p'; }

setup_hint() {
	{
		echo "web: no supported $1 toolchain was found."
		echo "  Supported versions are declared in web/package.json (\"engines\")."
		echo "  Fix by any of:"
		echo "    - put node/npm on PATH;"
		echo "    - install Zed's bundled Node;"
		echo "    - set NODE_BIN=/path/to/node and NPM_BIN=/path/to/npm."
		echo "  This check never installs or upgrades tools."
	} >&2
}

incompatible() {
	{
		echo "web: $1 $2 is outside the supported range ($3)."
		echo "  Use a supported version; this check never installs or upgrades tools."
	} >&2
}

# check_env resolves the toolchain, exports the resolved node dir on PATH (so npm
# and its child processes share it), and validates both versions.
check_env() {
	node=$(find_node) || { setup_hint node; return 1; }
	npm=$(find_npm) || { setup_hint npm; return 1; }
	PATH="$(dirname "$node"):$PATH"; export PATH
	NODE="$(command -v node 2>/dev/null || echo "$node")"
	NPM="$npm"

	node_v=$("$NODE" --version 2>/dev/null || true)
	npm_v=$("$NPM" --version 2>/dev/null || true)
	if [ -z "$node_v" ]; then setup_hint node; return 1; fi
	if [ -z "$npm_v" ]; then setup_hint npm; return 1; fi

	ranges=$(engines_range)
	nrange=$(printf '%s\n' "$ranges" | sed -n '1p')
	prange=$(printf '%s\n' "$ranges" | sed -n '2p')

	nmin=$(min_major "$nrange"); nmax=$(max_major "$nrange")
	pmin=$(min_major "$prange"); pmax=$(max_major "$prange")
	nmaj=$(major_of "$node_v"); pmaj=$(major_of "$npm_v")

	if [ -n "$nmin" ] && [ "$nmaj" -lt "$nmin" ]; then incompatible node "$node_v" "$nrange"; return 1; fi
	if [ -n "$nmax" ] && [ "$nmaj" -ge "$nmax" ]; then incompatible node "$node_v" "$nrange"; return 1; fi
	if [ -n "$pmin" ] && [ "$pmaj" -lt "$pmin" ]; then incompatible npm "$npm_v" "$prange"; return 1; fi
	if [ -n "$pmax" ] && [ "$pmaj" -ge "$pmax" ]; then incompatible npm "$npm_v" "$prange"; return 1; fi

	echo "web toolchain ok: node $NODE ($node_v, supported $nrange); npm $NPM ($npm_v, supported $prange)"
}

cmd_check_env() { check_env; }

cmd_install() {
	check_env
	cd "$WEB_DIR"
	echo "web: npm ci/install"
	exec "$NPM" install --no-audit --no-fund
}

cmd_run() { # $1 = npm script (typecheck|build|e2e)
	check_env
	cd "$WEB_DIR"
	if [ "$1" = "build" ]; then
		# Keep the production build out of the dev server's .next/ cache, and
		# start from a clean dist dir so stale state cannot break the build.
		NEXT_DIST_DIR="${NEXT_DIST_DIR:-.next-build}"; export NEXT_DIST_DIR
		rm -rf "$WEB_DIR/$NEXT_DIST_DIR"
	fi
	echo "web: npm run $1"
	exec "$NPM" run "$1"
}

read_pid() { if [ -f "$PID_FILE" ]; then cat "$PID_FILE" 2>/dev/null; fi; }
alive() { if [ -n "$1" ] && kill -0 "$1" 2>/dev/null; then return 0; fi; return 1; }
port_in_use() { if ss -ltn 2>/dev/null | grep -qE "[:.]$PORT[[:space:]]"; then return 0; fi; return 1; }

cmd_start() {
	check_env
	pid=$(read_pid)
	if alive "$pid"; then
		echo "already running (pid $pid) — not starting a duplicate"
		echo "url: $URL"
		echo "log: $LOG_FILE"
		return 0
	fi
	if [ -n "$pid" ]; then echo "removing stale pid file"; rm -f "$PID_FILE"; fi
	if port_in_use; then
		echo "port $PORT is already in use; refusing to start (another service is left untouched)" >&2
		return 1
	fi
	mkdir -p "$RUN_DIR"
	cd "$WEB_DIR"
	setsid sh -c 'echo $$ > "$1"; exec "$2" run dev' sh "$PID_FILE" "$NPM" > "$LOG_FILE" 2>&1 &
	i=0
	while [ "$i" -lt 60 ]; do
		if curl -fsS -m 2 -o /dev/null "$URL/"; then
			echo "started (pid $(read_pid))"
			echo "url: $URL"
			echo "log: $LOG_FILE"
			return 0
		fi
		sleep 1; i=$((i+1))
	done
	echo "server did not become ready; tail of $LOG_FILE:" >&2
	tail -20 "$LOG_FILE" >&2
	return 1
}

cmd_stop() {
	pid=$(read_pid)
	if [ -z "$pid" ]; then echo "not running (no pid file)"; return 0; fi
	if ! alive "$pid"; then echo "removing stale pid file"; rm -f "$PID_FILE"; echo "not running"; return 0; fi
	kill -TERM -"$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
	i=0
	while alive "$pid" && [ "$i" -lt 20 ]; do sleep 0.5; i=$((i+1)); done
	if alive "$pid"; then kill -KILL -"$pid" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true; fi
	rm -f "$PID_FILE"
	echo "stopped (pid $pid)"
}

cmd_status() {
	pid=$(read_pid)
	if alive "$pid"; then
		if curl -fsS -m 2 -o /dev/null "$URL/"; then
			echo "running (pid $pid)"; echo "url: $URL"; echo "log: $LOG_FILE"
		else
			echo "process $pid is alive but not responding at $URL"; echo "log: $LOG_FILE"
		fi
	elif [ -n "$pid" ]; then
		echo "not running (stale pid file: $PID_FILE)"
	else
		echo "not running"
	fi
}

case "${1:-}" in
	check-env) cmd_check_env ;;
	install) cmd_install ;;
	typecheck) cmd_run typecheck ;;
	build) cmd_run build ;;
	e2e) cmd_run e2e ;;
	start) cmd_start ;;
	stop) cmd_stop ;;
	status) cmd_status ;;
	*) echo "usage: $0 check-env|install|typecheck|build|e2e|start|stop|status" >&2; exit 2 ;;
esac
