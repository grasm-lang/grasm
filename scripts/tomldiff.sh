#!/usr/bin/env bash
# The manifests, read three ways (docs/self-hosting.md, S2 part two, and the
# TOML gate S8 asks for, begun here).
#
# `compiler/front` reads `grasm.toml` with `packages/toml` (`front toml`);
# the Rust toolchain reads it with `grasm_pkg::toml` (`grasm toml`); Python's
# `tomllib` is the third reader (`scripts/lib/tomldump.py`). Every manifest
# git tracks goes through all three, each printing every key as a dotted path
# with its value's kind and value, and the three must be the same bytes --
# or all three refuse the file.
#
# The entries of an array of tables are numbered and kept in the file's
# order, so a key read into the wrong entry shows (`tircontrols` plants one).
#
# The counts must add up to the manifests listed. `FRONT=` names a prebuilt
# frontend; `FILES=` a list of files.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if ! "$GRASM" toml /dev/null >/dev/null 2>&1; then
    echo "error: $GRASM has no \`toml\` command, so there is nothing to compare against"
    exit 1
fi
FRONT=${FRONT:-}
if [ -z "$FRONT" ]; then
    FRONT="$WORK/front"
    if ! "$GRASM" build compiler/front/src/main.gr --native -o "$FRONT" >"$WORK/build.log" 2>&1; then
        echo "error: compiler/front does not build:"
        sed 's/^/  /' "$WORK/build.log"
        exit 1
    fi
fi

# No manifest writes an array of tables, so `scripts/lib/tomlcases` does.
if [ -n "${FILES:-}" ]; then
    cp "$FILES" "$WORK/files"
else
    git ls-files '*grasm.toml' 'scripts/lib/tomlcases/*.toml' >"$WORK/files"
fi
listed=$(wc -l <"$WORK/files" | tr -d ' ')

# A refusal is compared as the word: each reader says why in its own words.
norm() {
    sed -E 's/^refused.*/refused/' "$1"
}

same=0
refused=0
keys=0
differ=0
declare -a FAILED=()
while IFS= read -r f; do
    "$GRASM" toml "$f" >"$WORK/r.out" 2>&1
    "$FRONT" toml "$f" >"$WORK/g.out" 2>&1
    python3 scripts/lib/tomldump.py "$f" >"$WORK/p.out" 2>&1
    norm "$WORK/r.out" >"$WORK/r.txt"
    norm "$WORK/g.out" >"$WORK/g.txt"
    norm "$WORK/p.out" >"$WORK/p.txt"
    if cmp -s "$WORK/r.txt" "$WORK/g.txt" && cmp -s "$WORK/r.txt" "$WORK/p.txt"; then
        if [ "$(cat "$WORK/r.txt")" = refused ]; then
            refused=$((refused + 1))
        else
            same=$((same + 1))
            keys=$((keys + $(wc -l <"$WORK/r.txt")))
        fi
    else
        differ=$((differ + 1))
        if ! cmp -s "$WORK/r.txt" "$WORK/g.txt"; then
            FAILED+=("$f — packages/toml and the Rust reader differ: $(diff "$WORK/r.txt" "$WORK/g.txt" | sed -n 2p | cut -c1-120)")
        else
            FAILED+=("$f — tomllib differs: $(diff "$WORK/r.txt" "$WORK/p.txt" | sed -n 2p | cut -c1-120)")
        fi
    fi
done <"$WORK/files"

echo "manifests: $listed   read alike by all three: $same ($keys keys)   refused by all three: $refused   disagree: $differ"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -40
fi
if [ $((same + refused + differ)) -ne "$listed" ]; then
    echo "the counts do not add up to the $listed manifests listed"
    exit 1
fi
if [ "$differ" -ne 0 ]; then
    exit 1
fi
if [ -z "${FILES:-}" ] && { [ "$same" -lt 1400 ] || [ "$keys" -lt 5000 ]; }; then
    echo "too few manifests ($same) or keys ($keys) compared for this to mean anything"
    exit 1
fi
exit 0
