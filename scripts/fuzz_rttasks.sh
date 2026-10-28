#!/usr/bin/env bash
# Tasks, channels and fibers on the Grasm runtime must schedule as the Rust
# runtime and the interpreter do (docs/concurrency.md, [E-SCHEDULE];
# docs/self-hosting.md §6.11).
#
# `compiler/rttasks/taskfuzz.gr` runs rounds of random tasks over random
# channels -- rendezvous and buffered, senders with random scripts,
# receivers on one channel and receivers selecting over all, a task that
# closes them all once every sender is done -- each task printing what it
# does, so the output is the order the fibers ran in; some tasks recurse
# deeply on their own stacks first, and some run a `concurrent` block of
# their own. It is built twice, linked as it ships and with the Grasm runtime
# interposed (`scripts/lib/rtswap.sh`); the swapped build must take the
# family's names from the Grasm runtime. Every seed must print and exit the
# same from both builds, with the leak check on and finding nothing live;
# each seed's rounds, with nested blocks and without them (`flat`), must also
# print and exit as `grasm run` does -- the interpreter's scheduler being the
# model of which fiber runs, held apart from either runtime's; and each of
# the family's panics must end both builds the same way and the interpreter
# with the same first line. Until 2026-10-28 the nested rounds were held to
# the two runtimes only: the interpreter lost a wake meant for an outer
# block's task while a nested block ran (docs/known-issues.md, closed).
#
#   RTTASKS_SEEDS (20) seeds of RTTASKS_ROUNDS (40) rounds. `RT_OBJ=`, `S3=`
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

SEEDS=${RTTASKS_SEEDS:-20}
ROUNDS=${RTTASKS_ROUNDS:-40}
PROBE=compiler/rttasks/taskfuzz.gr

if ! "$GRASM" build "$PROBE" --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: $PROBE does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
for need in chan_new chan_send chan_recv chan_select chan_close conc_run stack_floor; do
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime, so it compared nothing"
        exit 1
    fi
done
taken=$(grep -cE '^grasm (chan_|conc_run|stack_)' "$WORK/f.swaplog")

# Runs `$@` on both builds; fails saying how they differ.
both() {
    GRASM_LEAK_CHECK=1 "$WORK/f" "$@" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    GRASM_LEAK_CHECK=1 "$WORK/f.swap" "$@" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: ($*): the two runtimes differ (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | sed 's/^/    /'
        diff "$WORK/a.err" "$WORK/b.err" | head -4 | sed 's/^/    /'
        return 1
    fi
}

# The interpreter on `$@`, against what the Rust build just printed.
model() {
    "$GRASM" run "$PROBE" -- "$@" >"$WORK/i.out" 2>"$WORK/i.err" </dev/null
    ic=$?
    "$WORK/f" "$@" >"$WORK/m.out" 2>"$WORK/m.err" </dev/null
    mc=$?
    if [ "$ic" != "$mc" ] || ! cmp -s "$WORK/i.out" "$WORK/m.out" || [ "$(head -1 "$WORK/i.err")" != "$(head -1 "$WORK/m.err")" ]; then
        echo "FAIL: ($*): the interpreter's schedule differs (exit $ic, compiled $mc)"
        diff "$WORK/m.out" "$WORK/i.out" | head -6 | sed 's/^/    /'
        head -2 "$WORK/i.err" | sed 's/^/    /'
        return 1
    fi
}

same=0
modelled=0
lines=0
k=1
while [ "$k" -le "$SEEDS" ]; do
    both "$k" "$ROUNDS" || exit 1
    if [ "$ac" != 0 ] || ! grep -qx 'leak check: nothing live at exit' "$WORK/a.err"; then
        echo "FAIL: seed $k: exit $ac, or something left live:"
        head -3 "$WORK/a.err" | sed 's/^/    /'
        exit 1
    fi
    lines=$((lines + $(wc -l <"$WORK/a.out")))
    model "$k" "$ROUNDS" || exit 1
    both "$k" "$ROUNDS" flat || exit 1
    model "$k" "$ROUNDS" flat || exit 1
    same=$((same + 1))
    modelled=$((modelled + 1))
    k=$((k + 1))
done
panics=0
p=0
while [ "$p" -le 5 ]; do
    both 1 0 "$p" || exit 1
    if [ "$ac" != 101 ] || ! grep -q '^panic: ' "$WORK/a.err"; then
        echo "FAIL: panic case $p did not panic (exit $ac)"
        exit 1
    fi
    model 1 0 "$p" || exit 1
    panics=$((panics + 1))
    p=$((p + 1))
done
echo "seeds identical on both runtimes: $same of $SEEDS ($((2 * SEEDS * ROUNDS)) rounds, $lines lines of schedule, each leak-free)   seeds as the interpreter schedules them, nested and flat: $modelled   panics identical: $panics of 6   names of the family taken from Grasm: $taken"
[ "$same" -eq "$SEEDS" ] && [ "$SEEDS" -ge 1 ]
