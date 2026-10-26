#!/usr/bin/env bash
# The Grasm frontend's lexer against the Rust one (docs/self-hosting.md, S2).
#
# `compiler/front` is the compiler's frontend written in Grasm. Its lexer is
# held to `grasm tokens`: every `.gr` file in the repository -- the corpus
# and everything else git tracks, tests, fixtures, tools and `compiler/`
# included -- is lexed by both, and the two token streams must be the same
# bytes: every token's line, column and kind with its value, in order, on
# stdout; every lexical error's position and message on stderr; and the
# same exit status. Streams, not counts: a lexer that had the right number
# of tokens with one kind wrong would pass a count.
#
# The token streams carry each token's start; its end is held by `asttest`,
# whose spans are built from token ends, and by the CST printed back.
#
# Every file is compared or is a failure, and the counts must add up to the
# files git lists. `FRONT=` names a prebuilt frontend (the controls use it);
# `FILES=` a file listing the paths to compare instead of every one.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

FRONT=${FRONT:-}
if [ -z "$FRONT" ]; then
    # Built here, natively and once, so the gate cannot pass against a
    # frontend from before the source it claims to check.
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
    git ls-files '*.gr' >"$WORK/files"
fi
listed=$(wc -l <"$WORK/files" | tr -d ' ')

same=0
differ=0
tokens=0
errors=0
declare -a FAILED=()
while IFS= read -r f; do
    "$GRASM" tokens "$f" >"$WORK/ref.out" 2>"$WORK/ref.err"
    rc=$?
    "$FRONT" tokens "$f" >"$WORK/got.out" 2>"$WORK/got.err"
    gc=$?
    if [ "$rc" -ne 0 ] && [ "$rc" -ne 1 ]; then
        differ=$((differ + 1))
        FAILED+=("$f — grasm tokens exited $rc: $(head -1 "$WORK/ref.err")")
        continue
    fi
    if [ "$rc" = "$gc" ] && cmp -s "$WORK/ref.out" "$WORK/got.out" && cmp -s "$WORK/ref.err" "$WORK/got.err"; then
        same=$((same + 1))
        tokens=$((tokens + $(wc -l <"$WORK/ref.out")))
        errors=$((errors + $(wc -l <"$WORK/ref.err")))
    else
        differ=$((differ + 1))
        FAILED+=("$f — exit $rc (grasm tokens) vs $gc (front tokens)")
        if [ "$differ" -le 5 ]; then
            echo "MISMATCH: $f (exit $rc vs $gc)"
            diff "$WORK/ref.out" "$WORK/got.out" | head -6 | sed 's/^/  /'
            diff "$WORK/ref.err" "$WORK/got.err" | head -4 | sed 's/^/  /'
        fi
    fi
done <"$WORK/files"

echo
echo "files: $listed   identical: $same   mismatched: $differ"
echo "tokens: $tokens   lexical errors: $errors   (in the identical files)"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -50
fi
# Every listed file was compared or failed: anything else is a gap in this
# script.
if [ $((same + differ)) -ne "$listed" ]; then
    echo "the counts do not add up: $same + $differ is not the $listed files listed"
    exit 1
fi
if [ "$differ" -ne 0 ]; then
    exit 1
fi
# A gate that compared nothing has checked nothing; and the repository's
# own files, less a margin, when it was given no list.
if [ "$same" -lt 1 ] || [ "$tokens" -lt 1 ]; then
    echo "nothing was compared"
    exit 1
fi
if [ -z "${FILES:-}" ] && { [ "$same" -lt 1800 ] || [ "$tokens" -lt 3500000 ]; }; then
    echo "too few files ($same) or tokens ($tokens) for this to mean anything"
    exit 1
fi
exit 0
