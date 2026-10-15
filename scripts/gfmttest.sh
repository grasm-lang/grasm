#!/bin/sh
# The formatter written in Grasm, held to the formatter written in Rust.
#
# Both read the same file and must write the same bytes. A difference is a
# defect in one of them and this gate does not say which -- only that the
# self-hosted port and the implementation it was ported from no longer agree.
#
# This is the whole point of the port: `grasm fmt` is the first piece of this
# toolchain that Grasm hosts, and a claim that it hosts it is worth exactly as
# much as the gate behind it.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
GFMT=${GFMT:-/tmp/gfmt}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

checked=0
differ=0
for f in $(find packages apps ProjectGold examples tools benchmarks/grasm -name '*.gr' 2>/dev/null | sort); do
    cp "$f" "$work/rust.gr"
    "$GRASM" fmt "$work/rust.gr" >/dev/null 2>&1 || continue
    "$GFMT" "$f" "$work/grasm.gr" >/dev/null 2>&1 || {
        echo "    gfmt failed on $f"
        differ=$((differ + 1))
        continue
    }
    checked=$((checked + 1))
    if ! cmp -s "$work/rust.gr" "$work/grasm.gr"; then
        differ=$((differ + 1))
        [ "$differ" -le 5 ] && {
            echo "    disagree: $f"
            diff "$work/rust.gr" "$work/grasm.gr" | head -4 | sed 's/^/      /'
        }
    fi
done

echo "    checked: $checked   disagreeing: $differ"
[ "$differ" -eq 0 ]
