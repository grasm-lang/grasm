#!/usr/bin/env bash
# The Grasm frontend's TIR against the Rust frontend's (docs/self-hosting.md,
# S2 part two).
#
# `compiler/front` loads a program, checks it and prints its TIR
# (`front tir`); `grasm tir` is the reference. Every corpus program and every
# other `.gr` file git tracks goes through both, and for each one:
#
#   * both accept it, and the two TIR texts are the same bytes -- every
#     file and import line (the loader), every type, every decision; or
#   * both refuse it (a parse error, a type error, an effect error, a module
#     that does not load). Which diagnostic is part three's question; that
#     one side accepts what the other refuses is this gate's.
#
# Every tenth program the Grasm side accepts is emitted twice and must be the
# same bytes both times. The counts must add up to the files listed, and too
# few compared fails: a gate that compared nothing has checked nothing.
#
# `FRONT=` names a prebuilt frontend (the controls use it); `FILES=` a file
# listing the paths to compare instead.
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
    { corpus_programs; git ls-files '*.gr'; } | sort -u >"$WORK/files"
fi
listed=$(wc -l <"$WORK/files" | tr -d ' ')

same=0
refused=0
differ=0
twice=0
unstable=0
holes=0
modules=0
bytes=0
i=0
declare -a FAILED=()
while IFS= read -r f; do
    i=$((i + 1))
    "$GRASM" tir "$f" >"$WORK/ref.tir" 2>"$WORK/ref.err"
    rc=$?
    "$FRONT" tir "$f" >"$WORK/got.tir" 2>"$WORK/got.err"
    gc=$?
    if [ "$rc" = 0 ] && [ "$gc" = 0 ]; then
        if cmp -s "$WORK/ref.tir" "$WORK/got.tir"; then
            same=$((same + 1))
            bytes=$((bytes + $(wc -c <"$WORK/ref.tir")))
            modules=$((modules + $(grep -c '^(file ' "$WORK/ref.tir")))
            if [ $((same % 10)) = 0 ]; then
                twice=$((twice + 1))
                "$FRONT" tir "$f" >"$WORK/again.tir" 2>/dev/null
                if ! cmp -s "$WORK/got.tir" "$WORK/again.tir"; then
                    unstable=$((unstable + 1))
                    FAILED+=("$f — two runs of front tir differ")
                fi
            fi
        else
            differ=$((differ + 1))
            line=$(cmp "$WORK/ref.tir" "$WORK/got.tir" 2>&1 | head -1 | sed 's/.* line //')
            FAILED+=("$f — the TIR differs, first at line $line: $(sed -n "${line}p" "$WORK/ref.tir" | cut -c1-120)")
        fi
    elif [ "$rc" != 0 ] && [ "$gc" != 0 ]; then
        if grep -q 'hole(s)$' "$WORK/ref.err"; then
            holes=$((holes + 1))
            FAILED+=("$f — grasm tir leaves holes")
        else
            refused=$((refused + 1))
        fi
    elif [ "$rc" = 0 ]; then
        differ=$((differ + 1))
        FAILED+=("$f — grasm tir accepts it and front tir refuses it: $(head -1 "$WORK/got.err" | cut -c1-160)")
    else
        differ=$((differ + 1))
        FAILED+=("$f — front tir accepts it and grasm tir refuses it: $(grep -m1 'error' "$WORK/ref.err" | cut -c1-160)")
    fi
done <"$WORK/files"

echo "files: $listed   TIR identical: $same   both refuse: $refused   disagree: $differ   holes: $holes"
echo "modules compared: $modules   bytes compared: $bytes   emitted twice: $twice   unstable: $unstable"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -60
fi
if [ $((same + refused + differ + holes)) -ne "$listed" ]; then
    echo "the counts do not add up to the $listed files listed"
    exit 1
fi
if [ "$differ" -ne 0 ] || [ "$holes" -ne 0 ] || [ "$unstable" -ne 0 ]; then
    exit 1
fi
if [ "$same" -lt 1 ] || [ "$bytes" -lt 1 ]; then
    echo "nothing was compared"
    exit 1
fi
# The repository's programs, less a margin: a corpus that shrank, or a
# frontend that started refusing, is noticed.
if [ -z "${FILES:-}" ] && { [ "$same" -lt 1800 ] || [ "$refused" -lt 60 ] || [ "$twice" -lt 170 ]; }; then
    echo "too few programs compared ($same), refused ($refused) or emitted twice ($twice) for this to mean anything"
    exit 1
fi
exit 0
