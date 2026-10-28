#!/usr/bin/env bash
# Lists and ranges on the Grasm runtime must be the Rust runtime's
# (docs/self-hosting.md §6.11).
#
# `compiler/rt/probe/listfuzz.gr` builds lists of every representation --
# scalar integers, floats (-0.0, NaN, the infinities among them), bools and
# `u64`s; strings, tuples, options, lists and structs in boxes -- by literal,
# push, `map`, `filter` and a filled range; shares them and writes into the
# copies (the write must copy first, and the other must not see it); reads
# them every way; sorts, reverses, slices, joins, zips, enumerates, searches
# and sums them; and makes ranges values. It is built twice, linked as it
# ships and with the Grasm runtime interposed (`scripts/lib/rtswap.sh`); the
# swapped build must take the family's names from the Grasm runtime, and
# every seed must print and exit the same from both, with the leak check on
# and finding nothing live -- and so must each of the family's panics the
# program can end in.
#
#   RTLISTS_SEEDS (20) seeds of RTLISTS_ROUNDS (200) rounds. `RT_OBJ=`, `S3=`
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

SEEDS=${RTLISTS_SEEDS:-20}
ROUNDS=${RTLISTS_ROUNDS:-200}

if ! "$GRASM" build compiler/rt/probe/listfuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/listfuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
for need in list_push list_push_scalar list_set list_sort list_sort_by list_append_begin list_pin list_concat list_remove list_take range_to_list range_len; do
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime, so it compared nothing"
        exit 1
    fi
done
taken=$(grep -cE '^grasm (list|range)_' "$WORK/f.swaplog")

# Runs `$@` after the seed on both builds; fails saying how they differ.
both() {
    local k=$1
    shift
    GRASM_LEAK_CHECK=1 "$WORK/f" "$k" "$@" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    GRASM_LEAK_CHECK=1 "$WORK/f.swap" "$k" "$@" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $k ($*): the two runtimes differ (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | cut -c1-200 | sed 's/^/    /'
        diff "$WORK/a.err" "$WORK/b.err" | head -4 | sed 's/^/    /'
        return 1
    fi
}

same=0
k=1
while [ "$k" -le "$SEEDS" ]; do
    both "$k" "$ROUNDS" || exit 1
    # A round's text may hold a newline of its own, so its lines are not
    # counted; it must finish, and say something.
    if [ "$ac" != 0 ] || [ ! -s "$WORK/a.out" ] || ! grep -qx 'leak check: nothing live at exit' "$WORK/a.err"; then
        echo "FAIL: seed $k: exit $ac, or something left live:"
        head -3 "$WORK/a.err" | sed 's/^/    /'
        exit 1
    fi
    same=$((same + 1))
    k=$((k + 1))
done
panics=0
p=0
while [ "$p" -le 5 ]; do
    both 1 3 "$p" || exit 1
    if [ "$ac" != 101 ] || ! grep -q '^panic: ' "$WORK/a.err"; then
        echo "FAIL: panic case $p did not panic (exit $ac)"
        exit 1
    fi
    panics=$((panics + 1))
    p=$((p + 1))
done
echo "seeds identical: $same of $SEEDS ($((SEEDS * ROUNDS)) rounds, each leak-free)   panics identical: $panics of 6   names of the family taken from Grasm: $taken"
[ "$same" -eq "$SEEDS" ] && [ "$SEEDS" -ge 1 ]
