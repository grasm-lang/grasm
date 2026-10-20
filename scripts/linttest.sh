#!/usr/bin/env bash
# The linter reads every Grasm program in the tree without falling over.
#
# This is the bar `apps/grlint` set for itself — "anything it cannot handle is
# a finding rather than an excuse" — applied to the compiler's half of the
# linter. It is deliberately *not* a style gate: requiring this repository to
# be free of every finding would make the rules unaddable, since turning a new
# one on would break the build until every existing hit was rewritten.
#
# What it checks instead is that `grasm lint` terminates with a report on every
# file it can parse, never panics, and never returns an exit code that means
# something went wrong internally. A finding is exit 1; a panic is 101 and a
# signal is 128+n, and those are what fail this gate.
set -uo pipefail
cd "$(dirname "$0")/.."

GRASM="${GRASM_BIN:-target/release/grasm}"
[ -x "$GRASM" ] || cargo build --release -p grasm >/dev/null 2>&1

linted=0
findings=0
unparseable=0
broke=0
# The repository's own files, tracked or new: not whatever else sits in the
# directory. `find .` also read the copy of the whole corpus in a git-ignored
# worktree kept inside the repository, and judged it.
for f in $(git ls-files --cached --others --exclude-standard -- '*.gr' | sort -u); do
    out=$("$GRASM" lint "$f" 2>&1)
    code=$?
    case "$code" in
        0)
            linted=$((linted + 1))
            ;;
        1)
            case "$out" in
                *"does not parse"*)
                    unparseable=$((unparseable + 1))
                    ;;
                *)
                    linted=$((linted + 1))
                    n=$(printf '%s\n' "$out" | sed -n 's/^findings: //p')
                    findings=$((findings + ${n:-0}))
                    ;;
            esac
            ;;
        *)
            broke=$((broke + 1))
            echo "  exit $code: $f"
            printf '%s\n' "$out" | head -3
            ;;
    esac
done

echo "linted: $linted   findings: $findings   does not parse: $unparseable   broke: $broke"
[ "$broke" -eq 0 ]
