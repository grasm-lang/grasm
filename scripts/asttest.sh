#!/usr/bin/env bash
# The Grasm frontend's parser against the Rust one (docs/self-hosting.md, S2).
#
# `compiler/front` parses into a lossless concrete syntax tree and builds
# the AST from it. Two checks on every `.gr` file in the repository:
#
#   * the AST -- `grasm ast` prints `{:?}` of the Rust parser's AST, every
#     node with its span, the documentation, the wide literals, and then
#     each diagnostic; `front ast` must print the same bytes and exit the
#     same. A file that does not parse is held to it too: the AST after
#     recovery and every diagnostic.
#   * the CST -- the tree printed back must be the source, byte for byte.
#     Each token is printed with the trivia between it and the token before
#     it in the stream, so a token the tree lost, repeated or misplaced
#     shows; so does an error node dropped with what it held.
#
# Every file is compared or is a failure, and the counts must add up to the
# files git lists. `FRONT=` names a prebuilt frontend (the controls use it);
# `FILES=` a file listing the paths to compare instead.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if ! "$GRASM" ast /dev/null >/dev/null 2>&1; then
    echo "error: $GRASM has no \`ast\` command, so there is nothing to compare against"
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

if [ -n "${FILES:-}" ]; then
    cp "$FILES" "$WORK/files"
else
    git ls-files '*.gr' >"$WORK/files"
fi
listed=$(wc -l <"$WORK/files" | tr -d ' ')

ast_same=0
ast_differ=0
back_same=0
back_differ=0
rejected=0
diags=0
bytes=0
declare -a FAILED=()
while IFS= read -r f; do
    "$GRASM" ast "$f" >"$WORK/ref.out" 2>"$WORK/ref.err"
    rc=$?
    "$FRONT" ast "$f" >"$WORK/got.out" 2>"$WORK/got.err"
    gc=$?
    if [ "$rc" -ne 0 ] && [ "$rc" -ne 1 ]; then
        ast_differ=$((ast_differ + 1))
        FAILED+=("$f — grasm ast exited $rc: $(head -1 "$WORK/ref.err")")
    elif [ "$rc" = "$gc" ] && cmp -s "$WORK/ref.out" "$WORK/got.out" && cmp -s "$WORK/ref.err" "$WORK/got.err"; then
        ast_same=$((ast_same + 1))
        bytes=$((bytes + $(wc -c <"$WORK/ref.out")))
        if [ "$rc" = 1 ]; then
            rejected=$((rejected + 1))
            diags=$((diags + $(wc -l <"$WORK/ref.out") - 1))
        fi
    else
        ast_differ=$((ast_differ + 1))
        FAILED+=("$f — AST: exit $rc (grasm ast) vs $gc (front ast)")
        if [ "$ast_differ" -le 5 ]; then
            echo "MISMATCH: $f (exit $rc vs $gc)"
            cmp "$WORK/ref.out" "$WORK/got.out" | head -1 | sed 's/^/  /'
            head -2 "$WORK/got.err" | sed 's/^/  /'
        fi
    fi
    if "$FRONT" cst "$f" -o "$WORK/back" >"$WORK/cst.err" 2>&1 && cmp -s "$f" "$WORK/back"; then
        back_same=$((back_same + 1))
    else
        back_differ=$((back_differ + 1))
        FAILED+=("$f — the CST does not print back the source: $(cmp "$f" "$WORK/back" 2>&1 | head -1)$(head -1 "$WORK/cst.err")")
        if [ "$back_differ" -le 5 ]; then
            echo "NOT LOSSLESS: $f"
            cmp "$f" "$WORK/back" 2>&1 | head -1 | sed 's/^/  /'
        fi
    fi
done <"$WORK/files"

echo
echo "files: $listed   AST identical: $ast_same   AST mismatched: $ast_differ"
echo "CST printed back: $back_same   not: $back_differ"
echo "rejected by the parser: $rejected   diagnostics: $diags   bytes compared: $bytes"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -50
fi
if [ $((ast_same + ast_differ)) -ne "$listed" ] || [ $((back_same + back_differ)) -ne "$listed" ]; then
    echo "the counts do not add up to the $listed files listed"
    exit 1
fi
if [ "$ast_differ" -ne 0 ] || [ "$back_differ" -ne 0 ]; then
    exit 1
fi
if [ "$ast_same" -lt 1 ] || [ "$bytes" -lt 1 ]; then
    echo "nothing was compared"
    exit 1
fi
# The repository's own files, less a margin: a list that shrank is noticed.
# Some of them do not parse on purpose, and those are compared too.
if [ -z "${FILES:-}" ] && { [ "$ast_same" -lt 1800 ] || [ "$rejected" -lt 1 ]; }; then
    echo "too few files ($ast_same), or none the parser rejects ($rejected), for this to mean anything"
    exit 1
fi
exit 0
