#!/usr/bin/env bash
# Values as text on the Grasm runtime must be the Rust runtime's, byte for
# byte (docs/self-hosting.md §6.11).
#
# `compiler/rt/probe/showfuzz.gr` makes random values of every kind `print`
# and interpolation show -- scalars at their extremes, floats special and
# ordinary, text with quotes, braces and characters past ASCII, nested
# lists, tuples, variants, maps keyed by every kind of key, structs with
# scalar fields, ranges, `bytes`, builders, units, function values and
# handler values -- and prints each and each interpolated. It is built
# twice, linked as it ships and with the Grasm runtime interposed
# (`scripts/lib/rtswap.sh`); the swapped build must take `out_val` and
# `str_from_val` from the Grasm runtime, and every seed must print the same
# bytes from both.
#
#   RTSHOW_SEEDS (20) seeds of RTSHOW_ROUNDS (100) rounds. `RT_OBJ=`, `S3=`
#   and `LINK=` as for `scripts/rtswap.sh`.
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

SEEDS=${RTSHOW_SEEDS:-20}
ROUNDS=${RTSHOW_ROUNDS:-100}

if ! "$GRASM" build compiler/rt/probe/showfuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/showfuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
for need in out_val str_from_val; do
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime, so it compared nothing"
        exit 1
    fi
done

same=0
bytes=0
k=1
while [ "$k" -le "$SEEDS" ]; do
    "$WORK/f" "$k" "$ROUNDS" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    "$WORK/f.swap" "$k" "$ROUNDS" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $k: the two runtimes differ (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | cut -c1-200 | sed 's/^/    /'
        head -2 "$WORK/b.err" | sed 's/^/    /'
        exit 1
    fi
    if [ "$ac" != 0 ] || ! grep -q '^made [0-9]' "$WORK/a.out"; then
        echo "FAIL: seed $k: the run did not finish (exit $ac)"
        exit 1
    fi
    bytes=$((bytes + $(wc -c <"$WORK/a.out")))
    same=$((same + 1))
    k=$((k + 1))
done
echo "seeds identical: $same of $SEEDS ($((SEEDS * ROUNDS)) rounds, $bytes bytes of values shown on each runtime)"
[ "$same" -eq "$SEEDS" ] && [ "$SEEDS" -ge 1 ]
