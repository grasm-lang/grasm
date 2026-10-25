#!/usr/bin/env bash
# Does `packages/xxhash` agree with the reference implementation?
#
# The oracle is the `xxhash` Python package, which wraps the author's C
# library. Lengths are chosen for the branches -- under 16 and under 32 skip
# the four-lane loop, and the tail takes 8 bytes, then 4, then one at a time
# -- across four seeds including one that is itself a prime from the
# algorithm.
#
# The arithmetic is the part worth checking hardest. Grasm has no unsigned
# 64-bit integer, so every add and multiply is done in halves to stay inside
# `i64`; a shortcut that relied on wrapping would pass here and abort in a
# debug build, where Rust's overflow checks are on.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

PY=""
for cand in "${MCP_PYTHON:-}" ./.mcpvenv/bin/python python3; do
    [ -n "$cand" ] && command -v "$cand" >/dev/null 2>&1 &&
        "$cand" -c "import xxhash" 2>/dev/null && { PY=$cand; break; }
done
if [ -z "$PY" ]; then
    # A skip is not a pass: allgates.sh reads the exit code.
    echo "    the xxhash reference is not here, so nothing was checked:"
    echo "      python3 -m venv .mcpvenv && ./.mcpvenv/bin/pip install xxhash"
    exit 1
fi

if ! "$GRASM" build packages/xxhash-demo/src/main.gr --native -o "$work/x" \
    >"$work/build.log" 2>&1; then
    echo "    the demo did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

"$work/x" >"$work/digests.txt" 2>&1 || {
    echo "    the demo did not run:"
    sed 's/^/      /' "$work/digests.txt"
    exit 1
}

"$PY" scripts/lib/xxhashcheck.py <"$work/digests.txt"
