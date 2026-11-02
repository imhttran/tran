#!/bin/sh
# Live check: prove the real retrieval service answers through the wrapper.
#
# Sends two rag-retrieval/v1 requests through retrieval.sh next to this script:
#   1. an allowed query for corpus tran-docs, which must return status OK with
#      at least one cited evidence item from tran-docs;
#   2. the same query for corpus not-allowed, which must return status
#      UNAUTHORIZED_SCOPE.
# Responses are parsed with python3. On failure the script prints the status and
# the producer's diagnostics.error_code to stderr, and prints the operator steps
# when the status is UNAVAILABLE. It exits zero only when both checks pass.
#
# Docker, Ollama and the built binaries are needed. Run the operator steps first.
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
wrapper=$script_dir/retrieval.sh

request_allowed='{"schema_version":"rag-retrieval/v1","query":"how does branch review work","top_k":3,"filters":{"corpus":"tran-docs"},"deadline_ms":5000}'
request_denied='{"schema_version":"rag-retrieval/v1","query":"how does branch review work","top_k":3,"filters":{"corpus":"not-allowed"},"deadline_ms":5000}'

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT INT TERM

# Parse one response on stdin and print a verdict. The parser exits non-zero
# when the response does not meet the check for its request kind.
parser=$workdir/parse.py
cat >"$parser" <<'PY'
import json
import sys

kind = sys.argv[1]
raw = sys.stdin.read()
try:
    response = json.loads(raw)
except ValueError:
    print("STATUS=UNPARSEABLE")
    print("ERROR_CODE=<none>")
    print("DETAIL=response was not JSON")
    sys.exit(1)

if not isinstance(response, dict):
    print("STATUS=UNPARSEABLE")
    print("ERROR_CODE=<none>")
    print("DETAIL=response was not a JSON object")
    sys.exit(1)

status = response.get("status")
if not isinstance(status, str):
    status = "<missing>"
print("STATUS=%s" % status)

diagnostics = response.get("diagnostics")
error_code = "<none>"
if isinstance(diagnostics, dict):
    value = diagnostics.get("error_code")
    if isinstance(value, str) and value:
        error_code = value
print("ERROR_CODE=%s" % error_code)


def fail(detail):
    print("DETAIL=%s" % detail)
    sys.exit(1)


if kind == "denied":
    if status != "UNAUTHORIZED_SCOPE":
        fail("expected status UNAUTHORIZED_SCOPE")
    print("DETAIL=scope refused as expected")
    sys.exit(0)

if status != "OK":
    fail("expected status OK with cited tran-docs evidence")

evidence = response.get("evidence")
if not isinstance(evidence, list):
    fail("no evidence list in the response")

for item in evidence:
    if not isinstance(item, dict):
        continue
    corpus = item.get("corpus")
    source = item.get("source")
    if corpus != "tran-docs":
        continue
    if not isinstance(source, str):
        continue
    if source.startswith("docs/") or source == "README.md":
        print("CITED_SOURCE=%s" % source)
        print("DETAIL=found cited tran-docs evidence")
        sys.exit(0)

fail("no evidence item with corpus tran-docs and source under docs/ or README.md")
PY

operator_steps() {
	cat >&2 <<'EOF'
Operator steps: build the retrieval and ingest binaries, start the database on
port 5436 (compose.port.yml), pull nomic-embed-text in Ollama, run
scripts/sop-rag/ingest.sh, and append scripts/sop-rag/rag.config.yaml to config.
EOF
}

fail() {
	printf 'FAIL: %s\n' "$1" >&2
	exit 1
}

# check_request NAME KIND REQUEST
# Send one request through the wrapper and report the parsed verdict.
check_request() {
	name=$1
	kind=$2
	request=$3
	out=$workdir/$name.out
	err=$workdir/$name.err

	if ! printf '%s\n' "$request" | sh "$wrapper" >"$out" 2>"$err"; then
		printf 'FAIL: %s: retrieval.sh failed\n' "$name" >&2
		if [ -s "$err" ]; then
			cat "$err" >&2
		fi
		exit 1
	fi

	if ! verdict=$(python3 "$parser" "$kind" <"$out"); then
		status=$(printf '%s\n' "$verdict" | sed -n 's/^STATUS=//p')
		error_code=$(printf '%s\n' "$verdict" | sed -n 's/^ERROR_CODE=//p')
		detail=$(printf '%s\n' "$verdict" | sed -n 's/^DETAIL=//p')
		printf 'FAIL: %s: status=%s error_code=%s\n' \
			"$name" "${status:-<missing>}" "${error_code:-<none>}" >&2
		if [ -n "${detail:-}" ]; then
			printf 'FAIL: %s: %s\n' "$name" "$detail" >&2
		fi
		if [ "${status:-}" = "UNAVAILABLE" ]; then
			operator_steps
		fi
		exit 1
	fi

	cited=$(printf '%s\n' "$verdict" | sed -n 's/^CITED_SOURCE=//p')
	if [ -n "$cited" ]; then
		printf 'PASS: %s: cited %s\n' "$name" "$cited"
	else
		printf 'PASS: %s\n' "$name"
	fi
}

check_request allowed allowed "$request_allowed"
check_request denied denied "$request_denied"

printf 'PASS: live retrieval check\n'
