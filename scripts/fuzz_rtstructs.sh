#!/usr/bin/env bash
# Structs, variants, tuples and results on the Grasm runtime must be the
# Rust runtime's (docs/self-hosting.md §6.11).
#
# `compiler/rt/probe/structfuzz.gr` builds structs of every width with
# scalar fields among the boxed ones, reads and writes their fields -- in
# place, into one something else holds (which copies it first), and by
# pushing to a list field in place -- makes variants of every arity,
# `Option`s and results matched, defaulted and kept in lists, tuples, and
# lists of value structs with inline fields, and prints what it read and how
# many boxes are live. It is built twice, linked as it ships and with the
# Grasm runtime interposed (`scripts/lib/rtswap.sh`); the swapped build must
# take the family's names it reaches from the Grasm runtime, and every seed
# must print the same lines from both.
#
#   RTSTRUCTS_SEEDS (20) seeds of RTSTRUCTS_ROUNDS (300) rounds. `RT_OBJ=`,
#   `S3=` and `LINK=` as for `scripts/rtswap.sh`.
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

SEEDS=${RTSTRUCTS_SEEDS:-20}
ROUNDS=${RTSTRUCTS_ROUNDS:-300}

if ! "$GRASM" build compiler/rt/probe/structfuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/structfuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
for need in struct_make struct_get struct_set_field_at struct_take variant_new1 variant_new2 variant_push variant_field tuple2 tuple3 vals_new vals_push vals_at inline_absorb_str inline_absorb_list; do
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
    answers=$((answers + $(wc -c <"$WORK/a.out")))
    same=$((same + 1))
    k=$((k + 1))
done
taken=$(grep -cE '^grasm (struct|variant|tuple|try|result|ok_new|err_new|is_ok|inline|vals)' "$WORK/f.swaplog")
echo "seeds identical: $same of $SEEDS ($((SEEDS * ROUNDS)) rounds, $answers bytes of what was read on each runtime; names of the family taken from Grasm: $taken)"
[ "$same" -eq "$SEEDS" ] && [ "$SEEDS" -ge 1 ]
