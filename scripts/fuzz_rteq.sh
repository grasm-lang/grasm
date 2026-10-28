#!/usr/bin/env bash
# Equality and order on the Grasm runtime must be the Rust runtime's
# (docs/self-hosting.md §6.11).
#
# `compiler/rt/probe/eqfuzz.gr` makes random pairs of values of every kind
# `==` and `cmp` reach -- scalars boxed in lists, floats with NaN and both
# zeros, strings, nested lists, a list held as scalars against the same one
# from a map's keys, maps, tuples, `Option`s and enums, structs with float
# fields, ranges, `bytes`, units, function values named and closed over --
# often equal and often nearly, and prints each answer. It is built twice,
# linked as it ships and with the Grasm runtime interposed
# (`scripts/lib/rtswap.sh`); the swapped build must take `val_eq` and
# `val_cmp` from the Grasm runtime, and every seed must print the same lines
# from both.
#
#   RTEQ_SEEDS (20) seeds of RTEQ_ROUNDS (300) rounds. `RT_OBJ=`, `S3=` and
#   `LINK=` as for `scripts/rtswap.sh`.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
. "$(dirname "$0")/lib/link.sh"
. "$(dirname "$0")/lib/rtswap.sh"
s3_driver
link_driver
rtswap_runtime
rtswap_families
rtswap_linker

SEEDS=${RTEQ_SEEDS:-20}
ROUNDS=${RTEQ_ROUNDS:-300}

if ! "$GRASM" build compiler/rt/probe/eqfuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/eqfuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
for need in val_eq val_cmp; do
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime, so it compared nothing"
        exit 1
    fi
done

same=0
answers=0
k=1
while [ "$k" -le "$SEEDS" ]; do
    "$WORK/f" "$k" "$ROUNDS" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    "$WORK/f.swap" "$k" "$ROUNDS" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $k: the two runtimes differ (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | sed 's/^/    /'
        head -2 "$WORK/b.err" | sed 's/^/    /'
        exit 1
    fi
    n=$(grep -c . "$WORK/a.out")
    if [ "$ac" != 0 ] || [ "$n" != "$ROUNDS" ]; then
        echo "FAIL: seed $k: $n of $ROUNDS rounds, exit $ac"
        exit 1
    fi
    answers=$((answers + $(tr -d '\n-' <"$WORK/a.out" | wc -c)))
    same=$((same + 1))
    k=$((k + 1))
done
echo "seeds identical: $same of $SEEDS ($((SEEDS * ROUNDS)) rounds, $answers answers of == and cmp on each runtime)"
[ "$same" -eq "$SEEDS" ] && [ "$SEEDS" -ge 1 ]
