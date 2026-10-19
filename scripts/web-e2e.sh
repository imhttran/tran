#!/bin/sh
# Run the RUST-008A real-browser E2E suite.
#
#   scripts/web-e2e.sh
#
# Starts the production Axum API (real PostgreSQL + Ollama) on loopback, starts
# the Next.js app on 127.0.0.1:3000, runs Playwright against the real browser,
# then stops both servers and deletes only this run's corpus.
#
# The E2E suite is operator-run verification: Node.js and browser execution are
# outside SOP's command policy. The deterministic Rust-side contract is covered
# by rust/acceptance/api_contract.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

if [ -f .env ]; then set -a; . ./.env; set +a; fi

if [ -z "${DATABASE_URL:-}" ]; then
	echo "web-e2e: DATABASE_URL is required (set it or provide .env)" >&2
	exit 1
fi

API_PORT="${API_PORT:-8080}"
CORPUS="rust-008a-e2e-$(date +%Y%m%dT%H%M%S)-$$"
export RAG_CORPUS="$CORPUS"
export OLLAMA_CHAT_MODEL="${OLLAMA_CHAT_MODEL:-llama3.2:3b}"
export MIN_SIMILARITY="${MIN_SIMILARITY:-0.3}"
export REQUEST_TIMEOUT="${REQUEST_TIMEOUT:-180s}"

echo "web-e2e: corpus $CORPUS"
echo "web-e2e: building the API"
cargo build --locked --bin api

api_log=$(mktemp)
API_HOST=127.0.0.1 API_PORT="$API_PORT" target/debug/api >"$api_log" 2>&1 &
API_PID=$!

cleanup() {
	kill "$API_PID" 2>/dev/null || true
	wait "$API_PID" 2>/dev/null || true
	./scripts/web-control.sh stop >/dev/null 2>&1 || true
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DELETE FROM documents WHERE corpus = '$CORPUS'" >/dev/null 2>&1 || true
	rm -f "$api_log"
}
trap cleanup EXIT INT TERM

echo "web-e2e: waiting for the API health endpoint"
i=0
while [ "$i" -lt 60 ]; do
	if curl -fsS -m 2 -o /dev/null "http://127.0.0.1:$API_PORT/api/health"; then
		break
	fi
	if ! kill -0 "$API_PID" 2>/dev/null; then
		echo "web-e2e: API exited during startup:" >&2
		cat "$api_log" >&2
		exit 1
	fi
	sleep 1
	i=$((i+1))
done
if [ "$i" -ge 60 ]; then
	echo "web-e2e: API did not become healthy" >&2
	cat "$api_log" >&2
	exit 1
fi

echo "web-e2e: starting Next.js"
./scripts/web-control.sh stop >/dev/null 2>&1 || true
./scripts/web-control.sh start

echo "web-e2e: running Playwright"
./scripts/web-control.sh e2e

echo "web-e2e: ok"
