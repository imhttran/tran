#!/bin/sh
# Run the retrieval binary with the shared SOP RAG settings.
#
# Sources env.sh next to this script, then execs $SOP_RAG_BIN_DIR/retrieval,
# passing stdin and stdout straight through. When the binary is missing it
# writes one line to stderr and exits non-zero, without writing to stdout.
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=scripts/sop-rag/env.sh
. "$script_dir/env.sh"

retrieval_bin=$SOP_RAG_BIN_DIR/retrieval
if [ ! -x "$retrieval_bin" ]; then
	printf '%s\n' "retrieval binary not found or not executable: $retrieval_bin" >&2
	exit 127
fi

exec "$retrieval_bin" "$@"
