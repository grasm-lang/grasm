#!/usr/bin/env bash
# The Grasm frontend's loader against the Rust one (docs/self-hosting.md, S2
# part two).
#
# `front files P` loads a program the way `compiler/front` does -- the entry
# file, every module it imports, packages through their manifests -- and
# prints the `file` lines `grasm tir` begins its TIR with: each module's
# qualified name, alias, path and span base, in loading order, with the
# imports it resolved. For every corpus program:
#
#   * `grasm tir` accepts it: the lines are the same bytes;
#   * `grasm tir` refuses it for a module it could not load: `front files`
#     cannot load it either (exit 2);
#   * `grasm tir` refuses it for anything else: `front files` loads it.
#
# The counts must add up to the programs listed, and too few compared fails.
# `FRONT=` names a prebuilt frontend; `FILES=` a list of programs.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
corpus_require

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

FRONT=${FRONT:-}
if [ -z "$FRONT" ]; then
    FRONT="$WORK/front"
    if ! "$GRASM" build compiler/front/src/main.gr --native -o "$FRONT" >"$WORK/build.log" 2>&1; then
        echo "error: compiler/front does not build:"
        sed 's/^/  /' "$WORK/build.log"
        exit 1
    fi
fi

if [ -n "${FILES:-}" ]; then
    cp "$FILES" "$WORK/files"
else
    corpus_programs >"$WORK/files"
fi
listed=$(wc -l <"$WORK/files" | tr -d ' ')

same=0
loaded=0
unloadable=0
differ=0
modules=0
packaged=0
declare -a FAILED=()
while IFS= read -r f; do
    "$GRASM" tir "$f" >"$WORK/ref.tir" 2>"$WORK/ref.err"
    rc=$?
    "$FRONT" files "$f" >"$WORK/got.txt" 2>"$WORK/got.err"
    gc=$?
    if [ "$rc" = 0 ]; then
        grep -E '^\(file |^  \(import ' "$WORK/ref.tir" >"$WORK/ref.txt"
        if [ "$gc" = 0 ] && cmp -s "$WORK/ref.txt" "$WORK/got.txt"; then
            same=$((same + 1))
            n=$(grep -c '^(file ' "$WORK/ref.txt")
            modules=$((modules + n))
            if grep -q '^(file "[a-z_0-9]*\.' "$WORK/ref.txt"; then
                packaged=$((packaged + 1))
            fi
        else
            differ=$((differ + 1))
            FAILED+=("$f — the modules differ: $(diff "$WORK/ref.txt" "$WORK/got.txt" 2>&1 | sed -n 2p | cut -c1-140)")
        fi
    elif grep -qE 'cannot load module|cannot read' "$WORK/ref.err"; then
        if [ "$gc" = 2 ]; then
            unloadable=$((unloadable + 1))
        else
            differ=$((differ + 1))
            FAILED+=("$f — grasm cannot load it and front files can")
        fi
    elif [ "$gc" = 0 ]; then
        # Loaded by both; refused later (a parse, type or effect error),
        # which is tirdiff's and tirrefuse's to compare.
        loaded=$((loaded + 1))
    else
        differ=$((differ + 1))
        FAILED+=("$f — grasm loads it and front files cannot: $(head -1 "$WORK/got.err" | cut -c1-140)")
    fi
done <"$WORK/files"

echo "programs: $listed   same modules: $same ($modules modules; $packaged with a package's own import)   loaded, then refused: $loaded   unloadable by both: $unloadable   disagree: $differ"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -40
fi
if [ $((same + loaded + unloadable + differ)) -ne "$listed" ]; then
    echo "the counts do not add up to the $listed programs listed"
    exit 1
fi
if [ "$differ" -ne 0 ]; then
    exit 1
fi
if [ -z "${FILES:-}" ] && { [ "$same" -lt 1650 ] || [ "$packaged" -lt 100 ]; }; then
    echo "too few programs compared ($same), or with a package importing its own ($packaged), for this to mean anything"
    exit 1
fi
exit 0
