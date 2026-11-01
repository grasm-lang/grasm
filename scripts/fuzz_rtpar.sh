#!/usr/bin/env bash
# The parallel workers on the Grasm runtime must be the Rust runtime's
# (docs/self-hosting.md §6.12): `par_map`, `par_filter`, a `parallel`
# block's tasks and `net_workers`, on threads the runtime starts.
#
# `compiler/rtsys/parfuzz.gr` first maps a list of strings over the workers
# into strings made on them and released here; then maps and filters lists
# of integers, floats, booleans and strings, short (run where they are) and
# long or costly (spread over workers, or, once a thread has run a parallel
# region, run there: docs/known-issues.md), with captured values, one inside
# another; runs independent tasks, each on a thread, and tasks sharing a
# list, in order; and `net_workers` of each count up to four, its workers
# reaching the network. It is built twice -- linked as it ships and with
# the Grasm runtime alone (`scripts/lib/rtswap.sh`), which must take every
# name of the family -- and every seed must print, write to
# standard error and exit the same on both, with the leak check on (every
# worker's boxes counted when it ends); and so must each way a worker can
# end the program: a panic in a closure, a recursion too deep for a
# worker's stack, too many network workers, a division by zero in a filter.
#
#   RTPAR_SEEDS (12) seeds of RTPAR_ROUNDS (40) rounds. `RT_OBJ=`, `S3=` and
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

SEEDS=${RTPAR_SEEDS:-12}
ROUNDS=${RTPAR_ROUNDS:-40}

if ! "$GRASM" build compiler/rtsys/parfuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rtsys/parfuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
for need in par_map_scalar par_map_mixed par_filter_scalar par_filter_boxed par_tasks_next net_workers net_workers_indices; do
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime, so it compared nothing"
        exit 1
    fi
done

both() {
    GRASM_LEAK_CHECK=1 "$WORK/f" "$@" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    GRASM_LEAK_CHECK=1 "$WORK/f.swap" "$@" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: parfuzz $*: the two runtimes differ (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | cut -c1-200 | sed 's/^/    /'
        diff "$WORK/a.err" "$WORK/b.err" | head -4 | sed 's/^/    /'
        exit 1
    fi
}

same=0
k=1
while [ "$k" -le "$SEEDS" ]; do
    both "$k" "$ROUNDS"
    if [ "$ac" != 0 ] || [ "$(grep -c . "$WORK/a.out")" -ne $((ROUNDS + 1)) ] || ! grep -q 'nothing live at exit' "$WORK/a.err"; then
        echo "FAIL: parfuzz seed $k: exit $ac, $(grep -c . "$WORK/a.out") lines, $(tail -1 "$WORK/a.err")"
        exit 1
    fi
    same=$((same + 1))
    k=$((k + 1))
done
spread=$(grep -cE ' (map|filter) [a-z ]*(3[0-9][0-9]|[4-6][0-9][0-9]):' "$WORK/a.out")
# Workers that panic together may each write their message before the
# process is gone, on either runtime: there the distinct lines of standard
# error are compared, and standard output and the status exactly.
both_panic() {
    GRASM_LEAK_CHECK=1 "$WORK/f" "$@" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    GRASM_LEAK_CHECK=1 "$WORK/f.swap" "$@" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || [ "$(sort -u "$WORK/a.err")" != "$(sort -u "$WORK/b.err")" ]; then
        echo "FAIL: parfuzz $*: the two runtimes differ (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | cut -c1-200 | sed 's/^/    /'
        diff "$WORK/a.err" "$WORK/b.err" | head -4 | sed 's/^/    /'
        exit 1
    fi
}

panics=0
p=0
while [ "$p" -le 3 ]; do
    both_panic 1 1 "$p"
    if [ "$ac" != 101 ] || ! grep -q '^panic: ' "$WORK/a.err"; then
        echo "FAIL: parfuzz panic case $p did not panic (exit $ac)"
        exit 1
    fi
    panics=$((panics + 1))
    p=$((p + 1))
done
echo "seeds identical: $same of $SEEDS ($((SEEDS * ROUNDS)) rounds, each leak-free; in the last seed $spread long enough to spread)   panics identical: $panics of 4"
[ "$same" -eq "$SEEDS" ] && [ "$SEEDS" -ge 1 ]
