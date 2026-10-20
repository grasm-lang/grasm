#!/usr/bin/env bash
# Every Grasm program in the tree is canonically formatted.
#
# `grasm fmt` is checked two ways, and they answer different questions. A test
# in `grasm_syntax` proves the formatter cannot *break* a program — the token
# stream it emits equals the one it read, over this same corpus. This gate
# proves the corpus is actually *in* that form, which is what makes
# `fmt --check` worth running in anyone's editor or CI.
#
# One file is excluded, and it is the only one:
#
#   spec/conformance/L-WHITESPACE.gr proves the lexer treats tabs and runs of
#   spaces as separators. Its odd layout is the fixture. Formatting it would
#   delete the thing it tests, so it is exempt by name rather than by pattern —
#   a pattern would silently exempt the next file that drifts.
set -uo pipefail
cd "$(dirname "$0")/.."

EXCLUDE="spec/conformance/L-WHITESPACE.gr"
GRASM="${GRASM_BIN:-target/release/grasm}"
[ -x "$GRASM" ] || cargo build --release -p grasm >/dev/null 2>&1

checked=0
unformatted=0
unparseable=0
# The repository's own files, tracked or new: not whatever else sits in the
# directory. `find .` also read the copy of the whole corpus in a git-ignored
# worktree kept inside the repository, and judged it.
for f in $(git ls-files --cached --others --exclude-standard -- '*.gr' | sort -u); do
    [ "$f" = "$EXCLUDE" ] && continue
    out=$("$GRASM" fmt "$f" --check 2>&1)
    if [ $? -eq 0 ]; then
        checked=$((checked + 1))
        continue
    fi
    # A conformance fixture for a *syntax* error does not parse, and source
    # that does not parse has no canonical form — the formatter refuses it on
    # purpose. Counted, so the number moving is visible.
    case "$out" in
        *"does not parse"*) unparseable=$((unparseable + 1)) ;;
        *)
            checked=$((checked + 1))
            unformatted=$((unformatted + 1))
            echo "  not formatted: $f"
            ;;
    esac
done

echo "formatted: $checked   unformatted: $unformatted   does not parse: $unparseable   exempt: 1"
[ "$unformatted" -eq 0 ]
