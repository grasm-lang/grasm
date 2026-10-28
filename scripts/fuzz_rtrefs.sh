#!/usr/bin/env bash
# Generational references on the Grasm runtime must be the Rust runtime's
# (docs/self-hosting.md §6.11).
#
# `compiler/rt/probe/reffuzz.gr` keeps a graph of nodes held by reference --
# made, read and written through, walked, released so their slots are used
# again and old references go stale, asked whether alive -- and prints what
# it read. It is built twice, linked as it ships and with the Grasm runtime
# interposed (`scripts/lib/rtswap.sh`); the swapped build must take
# `ref_new`, `ref_get` and `ref_release` from the Grasm runtime, and every
# seed must print and exit the same from both: once ending in a read through
# a released reference, which must panic as stale, and once ending normally
# with the leak check on, which must find nothing live (what the heap still
# holds is released first, as a root).
#
#   RTREFS_SEEDS (20) seeds of RTREFS_ROUNDS (400) rounds. `RT_OBJ=`, `S3=`
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

SEEDS=${RTREFS_SEEDS:-20}
ROUNDS=${RTREFS_ROUNDS:-400}

if ! "$GRASM" build compiler/rt/probe/reffuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/reffuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
for need in ref_new ref_get ref_release ref_alive ref_make_unique; do
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime, so it compared nothing"
        exit 1
    fi
done

same=0
stale=0
k=1
while [ "$k" -le "$SEEDS" ]; do
    for mode in stale end; do
        arg=""
        [ "$mode" = stale ] && arg=stale
        GRASM_LEAK_CHECK=1 "$WORK/f" "$k" "$ROUNDS" $arg >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
        ac=$?
        GRASM_LEAK_CHECK=1 "$WORK/f.swap" "$k" "$ROUNDS" $arg >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
        bc=$?
        if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
            echo "FAIL: seed $k ($mode): the two runtimes differ (exit $ac and $bc)"
            diff "$WORK/a.out" "$WORK/b.out" | head -6 | sed 's/^/    /'
            diff "$WORK/a.err" "$WORK/b.err" | head -4 | sed 's/^/    /'
            exit 1
        fi
        if [ "$mode" = end ] && { [ "$ac" != 0 ] || ! grep -qx 'leak check: nothing live at exit' "$WORK/a.err"; }; then
            echo "FAIL: seed $k: the run did not end clean (exit $ac): $(head -2 "$WORK/a.err" | tr '\n' ' ')"
            exit 1
        fi
        if [ "$mode" = stale ] && [ "$ac" = 101 ]; then
            if ! grep -q '^panic: stale reference' "$WORK/a.err"; then
                echo "FAIL: seed $k: exit 101 but not for a stale reference: $(head -1 "$WORK/a.err")"
                exit 1
            fi
            stale=$((stale + 1))
        fi
    done
    same=$((same + 1))
    k=$((k + 1))
done
echo "seeds identical: $same of $SEEDS ($((2 * SEEDS * ROUNDS)) rounds, $stale ending in the stale-reference panic, every other run leak-free)"
[ "$same" -eq "$SEEDS" ] && [ "$SEEDS" -ge 1 ] && [ "$stale" -ge 1 ]
