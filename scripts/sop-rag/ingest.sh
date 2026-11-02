#!/bin/sh
# Ingest the SOP RAG corpus into the retrieval database.
#
# Sources env.sh next to this script, changes to the repository root, expands
# corpus.txt into the Markdown files that exist, and runs
# $SOP_RAG_BIN_DIR/ingest FILE once per file. It stops at the first failure and
# names the file that failed.
#
# Usage:
#   ingest.sh             ingest every corpus file
#   ingest.sh --dry-run   print the file list and run nothing
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=scripts/sop-rag/env.sh
. "$script_dir/env.sh"

# The corpus paths are relative to the repository root, two levels up.
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
cd "$repo_root"

corpus_list=$script_dir/corpus.txt
ingest_bin=$SOP_RAG_BIN_DIR/ingest

dry_run=0
case "${1:-}" in
"")		;;
--dry-run)	dry_run=1 ;;
*)
	printf '%s\n' "unknown argument: $1" >&2
	printf '%s\n' "usage: ingest.sh [--dry-run]" >&2
	exit 2
	;;
esac

if [ ! -f "$corpus_list" ]; then
	printf '%s\n' "corpus list not found: $corpus_list" >&2
	exit 1
fi

# Print one Markdown file per line. Skip comments, blank lines and missing
# paths. A directory entry expands to the .md files inside it.
list_files() {
	while IFS= read -r line || [ -n "$line" ]; do
		case "$line" in
		'#'*) continue ;;
		esac
		entry=$line
		[ -n "$entry" ] || continue
		case "$entry" in
		*.md)
			[ -f "$entry" ] && printf '%s\n' "$entry"
			;;
		*)
			[ -d "$entry" ] || continue
			find "$entry" -type f -name '*.md' | sort
			;;
		esac
	done <"$corpus_list"
}

files=$(list_files)
if [ -z "$files" ]; then
	printf '%s\n' "no corpus files found from $corpus_list" >&2
	exit 1
fi

if [ "$dry_run" -eq 1 ]; then
	printf '%s\n' "$files"
	exit 0
fi

if [ ! -x "$ingest_bin" ]; then
	first_file=$(printf '%s\n' "$files" | head -n 1)
	printf '%s\n' "ingest binary not found or not executable: $ingest_bin (file: $first_file)" >&2
	exit 127
fi

printf '%s\n' "$files" >"$script_dir/.ingest-files.$$"
trap 'rm -f "$script_dir/.ingest-files.$$"' EXIT INT TERM
while IFS= read -r file; do
	# The Rust ingest stores the bare file name unless RAG_SOURCE names the
	# repo-relative path that retrieval reports as evidence `source`.
	if ! RAG_SOURCE=$file "$ingest_bin" "$file"; then
		printf '%s\n' "ingest failed for file: $file" >&2
		exit 1
	fi
done <"$script_dir/.ingest-files.$$"
