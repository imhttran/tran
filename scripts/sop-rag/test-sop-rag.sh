#!/bin/sh
# Self-test for env.sh, retrieval.sh and ingest.sh. No database and no Ollama
# are needed.
#
# A stub retrieval binary in a temporary directory (set through SOP_RAG_BIN_DIR)
# echoes stdin and records its environment to a file. The four acceptance
# scenarios run against that stub. Later scenarios exercise ingest.sh with a
# stub ingest binary.
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
wrapper=$script_dir/retrieval.sh
ingest=$script_dir/ingest.sh

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT INT TERM

bin_dir=$workdir/bin
mkdir -p "$bin_dir"
env_dump=$workdir/env.dump

write_stub() {
	cat >"$bin_dir/retrieval" <<STUB
#!/bin/sh
env >"$env_dump"
exec cat
STUB
	chmod +x "$bin_dir/retrieval"
}

fail() {
	printf 'FAIL: %s\n' "$1" >&2
	exit 1
}

# --- Scenario A: inherited values lose to env.sh values ---
write_stub
dumped=$(DATABASE_URL='postgres://wrong/db' \
	OLLAMA_URL='http://wrong:1' \
	EMBED_DIM='99' \
	SOP_RAG_BIN_DIR="$bin_dir" \
	sh "$wrapper" </dev/null)
[ -z "$dumped" ] || fail "scenario A: wrapper wrote to stdout"
if ! grep -qx 'DATABASE_URL=postgres://rag:rag@127.0.0.1:5436/rag?sslmode=disable' "$env_dump"; then
	fail "scenario A: DATABASE_URL did not come from env.sh"
fi
if ! grep -qx 'OLLAMA_URL=http://localhost:11434' "$env_dump"; then
	fail "scenario A: OLLAMA_URL did not come from env.sh"
fi
if ! grep -qx 'EMBED_DIM=768' "$env_dump"; then
	fail "scenario A: EMBED_DIM did not come from env.sh"
fi
printf 'PASS: scenario A\n'

# --- Scenario B: SOP_RAG_DATABASE_URL changes DATABASE_URL ---
write_stub
new_db='postgres://rag:rag@example:5555/other?sslmode=disable'
dumped=$(SOP_RAG_DATABASE_URL="$new_db" SOP_RAG_BIN_DIR="$bin_dir" sh "$wrapper" </dev/null)
[ -z "$dumped" ] || fail "scenario B: wrapper wrote to stdout"
if ! grep -qx "DATABASE_URL=$new_db" "$env_dump"; then
	fail "scenario B: DATABASE_URL did not follow SOP_RAG_DATABASE_URL"
fi
printf 'PASS: scenario B\n'

# --- Scenario C: stdin passes through unchanged, stdout has no extra bytes ---
# Command substitution strips trailing newlines, so append a sentinel and strip
# it after, keeping the payload's final newline intact.
write_stub
payload='hello stdin
second line
'
actual=$(printf '%s' "$payload" | SOP_RAG_BIN_DIR="$bin_dir" sh "$wrapper"; printf x)
actual=${actual%x}
if [ "$actual" != "$payload" ]; then
	fail "scenario C: stdin did not round-trip unchanged"
fi
printf 'PASS: scenario C\n'

# --- Scenario D: missing binary exits non-zero with empty stdout ---
missing_dir=$workdir/missing
mkdir -p "$missing_dir"
out_file=$workdir/stdout.txt
if SOP_RAG_BIN_DIR="$missing_dir" sh "$wrapper" </dev/null >"$out_file" 2>/dev/null; then
	fail "scenario D: wrapper succeeded without a binary"
fi
if [ -s "$out_file" ]; then
	fail "scenario D: wrapper wrote to stdout without a binary"
fi
printf 'PASS: scenario D\n'

# --- Scenario E: ingest.sh --dry-run lists the Summary's expected corpus ---
# The expected folder set comes from the plan Summary, not from corpus.txt, so
# a wrong folder in corpus.txt fails this scenario instead of echoing it.
dry_out=$(SOP_RAG_BIN_DIR="$missing_dir" sh "$ingest" --dry-run)
printf '%s\n' "$dry_out" | grep -qx 'README.md' \
	|| fail "scenario E: dry-run did not list README.md"
for dir in docs/architecture docs/guides docs/reference; do
	expected_files=$(CDPATH= cd -- "$script_dir/../.." && find "$dir" -type f -name '*.md' | sort)
	[ -n "$expected_files" ] || fail "scenario E: expected folder $dir has no Markdown files"
	for file in $expected_files; do
		printf '%s\n' "$dry_out" | grep -qxF "$file" \
			|| fail "scenario E: dry-run did not list $file"
	done
done
if printf '%s\n' "$dry_out" | grep -qE '^docs/(operations|plans|reports|history)/'; then
	fail "scenario E: dry-run listed an excluded docs folder"
fi
printf 'PASS: scenario E\n'

# --- Scenario G0: the guide names the Summary's corpus folders ---
# The guide must name docs/architecture/ and must not name docs/operations/.
guide=$script_dir/../../docs/guides/sop-rag.md
[ -f "$guide" ] || fail "scenario G0: docs/guides/sop-rag.md is missing"
grep -qF 'docs/architecture/' "$guide" \
	|| fail "scenario G0: guide does not name docs/architecture/"
if grep -qF 'docs/operations/' "$guide"; then
	fail "scenario G0: guide names docs/operations/"
fi
printf 'PASS: scenario G0\n'

# --- Scenario F: ingest.sh runs the stub once per listed file ---
record=$workdir/ingest.record
record_stub_dir=$workdir/record-bin
mkdir -p "$record_stub_dir"
cat >"$record_stub_dir/ingest" <<STUB
#!/bin/sh
printf '%s|%s\\n' "\$1" "\$RAG_CORPUS" >>"$record"
STUB
chmod +x "$record_stub_dir/ingest"
SOP_RAG_BIN_DIR="$record_stub_dir" sh "$ingest"
expected=$(SOP_RAG_BIN_DIR="$missing_dir" sh "$ingest" --dry-run)
expected_calls=$(printf '%s\n' "$expected" | wc -l | tr -d ' ')
actual_calls=$(wc -l <"$record" | tr -d ' ')
[ "$actual_calls" = "$expected_calls" ] \
	|| fail "scenario F: expected $expected_calls ingest calls, got $actual_calls"
if grep -qv '|tran-docs$' "$record"; then
	fail "scenario F: a call did not have RAG_CORPUS=tran-docs"
fi
printf 'PASS: scenario F\n'

# --- Scenario G: a failing stub stops ingest.sh and names the file ---
fail_stub_dir=$workdir/fail-bin
mkdir -p "$fail_stub_dir"
cat >"$fail_stub_dir/ingest" <<'STUB'
#!/bin/sh
printf 'boom\n' >&2
exit 1
STUB
chmod +x "$fail_stub_dir/ingest"
err_file=$workdir/fail.err
if SOP_RAG_BIN_DIR="$fail_stub_dir" sh "$ingest" >/dev/null 2>"$err_file"; then
	fail "scenario G: ingest.sh succeeded with a failing stub"
fi
first_expected=$(printf '%s\n' "$expected" | head -n 1)
if ! grep -qF "$first_expected" "$err_file"; then
	fail "scenario G: stderr did not name the failing file"
fi
printf 'PASS: scenario G\n'

# --- Scenario H: rag.config.yaml holds the rag: block and one step 5 comment ---
config=$script_dir/rag.config.yaml
[ -f "$config" ] || fail "scenario H: rag.config.yaml is missing"

first_line=$(grep -vE '^[[:space:]]*$' "$config" | head -n 1)
case $first_line in
'#'*) ;;
*) fail "scenario H: first non-empty line is not a '#' comment" ;;
esac
printf '%s\n' "$first_line" | grep -qF 'step 5' \
	|| fail "scenario H: leading comment does not name operator step 5"

# The file must hold the comment plus exactly the rag block, in order.
cat >"$workdir/expected.config" <<'EOF'
rag:
  enabled: true
  executable: /Users/imhttran/agentic-workspace/projects/tran/scripts/sop-rag/retrieval.sh
  corpus: tran-docs
  allowed_corpora:
    - tran-docs
  timeout_ms: 5000
  top_k: 5
  context_budget_bytes: 4096
EOF
if ! grep -v -e '^[[:space:]]*$' "$config" | grep -v -e '^#' \
	| diff -u "$workdir/expected.config" - >/dev/null; then
	fail "scenario H: rag block does not match the expected block"
fi
printf 'PASS: scenario H\n'

# --- Scenario I: executable is absolute and names the RAG-101 wrapper ---
# --- Scenario J: corpus is listed in allowed_corpora ---
block=$(grep -v -e '^[[:space:]]*$' "$config" | grep -v -e '^#')
if ! printf '%s\n' "$block" | grep -qxF '  enabled: true'; then
	fail "scenario I: enabled is not true"
fi
if ! printf '%s\n' "$block" | grep -qxF '  timeout_ms: 5000'; then
	fail "scenario I: timeout_ms is not 5000"
fi
if ! printf '%s\n' "$block" | grep -qxF '  top_k: 5'; then
	fail "scenario I: top_k is not 5"
fi
if ! printf '%s\n' "$block" | grep -qxF '  context_budget_bytes: 4096'; then
	fail "scenario I: context_budget_bytes is not 4096"
fi
executable=$(printf '%s\n' "$block" | sed -n 's/^  executable: //p')
case $executable in
/*) ;;
*) fail "scenario I: executable is not an absolute path" ;;
esac
case $executable in
*/scripts/sop-rag/retrieval.sh) ;;
*) fail "scenario I: executable does not end in scripts/sop-rag/retrieval.sh" ;;
esac
printf 'PASS: scenario I\n'

corpus=$(printf '%s\n' "$block" | sed -n 's/^  corpus:[[:space:]]*//p' | head -n 1)
[ -n "$corpus" ] || fail "scenario J: corpus is not set"
if ! printf '%s\n' "$block" | grep -qxF "    - $corpus"; then
	fail "scenario J: corpus is not listed in allowed_corpora"
fi
printf 'PASS: scenario J\n'

printf 'PASS: sop-rag scripts\n'
