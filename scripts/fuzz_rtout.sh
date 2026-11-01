#!/usr/bin/env bash
# Output and the ways out of a program on the Grasm runtime must be the Rust
# runtime's (docs/self-hosting.md §6.12): the one output buffer, its
# flushes, and every way out flushing it first.
#
# `compiler/rt/probe/outfuzz.gr` prints and eprints text of every length
# either side of the buffers' 8 KiB, values of every kind, lines an
# `eprint` interrupts while their pieces are made, and ends in each way a
# program can: returning, a panic, `exit` past 255 and below 0, a failed
# assertion, a division by zero, an index out of bounds, a panic with a
# value, the error `main` returns. It is built twice -- linked as it ships
# and with the Grasm runtime alone (`scripts/lib/rtswap.sh`), which must
# take the output and exit names from the Grasm runtime -- and each seed and
# each ending is run on both three ways: standard output and standard error
# apart, which must hold the same bytes, and the two into one file (`2>&1`),
# whose order is when each stream was written out, which must be the same
# too; and the exit status.
#
#   RTOUT_SEEDS (20) seeds of RTOUT_ROUNDS (60) rounds, each ending in every
#   one of the 9 ways out. `RT_OBJ=`, `S3=` and `LINK=` as for
#   `scripts/rtswap.sh`.
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

SEEDS=${RTOUT_SEEDS:-20}
ROUNDS=${RTOUT_ROUNDS:-60}

if ! "$GRASM" build compiler/rt/probe/outfuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/outfuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
for need in out_text out_nl out_to_stderr panic_msg exit report_main_err; do
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime, so it compared nothing"
        exit 1
    fi
done

runs=0
bytes=0
declare -a SEEN=()
k=1
while [ "$k" -le "$SEEDS" ]; do
    e=0
    while [ "$e" -le 8 ]; do
        "$WORK/f" "$k" "$ROUNDS" "$e" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
        ac=$?
        "$WORK/f.swap" "$k" "$ROUNDS" "$e" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
        bc=$?
        "$WORK/f" "$k" "$ROUNDS" "$e" >"$WORK/a.both" 2>&1 </dev/null
        "$WORK/f.swap" "$k" "$ROUNDS" "$e" >"$WORK/b.both" 2>&1 </dev/null
        if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err" || ! cmp -s "$WORK/a.both" "$WORK/b.both"; then
            echo "FAIL: seed $k, ending $e: the two runtimes differ (exit $ac and $bc)"
            diff "$WORK/a.out" "$WORK/b.out" | head -4 | cut -c1-160 | sed 's/^/    out: /'
            diff "$WORK/a.err" "$WORK/b.err" | head -4 | cut -c1-160 | sed 's/^/    err: /'
            diff "$WORK/a.both" "$WORK/b.both" | head -4 | cut -c1-160 | sed 's/^/    together: /'
            exit 1
        fi
        if [ ! -s "$WORK/a.both" ]; then
            echo "FAIL: seed $k, ending $e: nothing written"
            exit 1
        fi
        SEEN[$e]="${SEEN[$e]:-} $ac"
        bytes=$((bytes + $(wc -c <"$WORK/a.both")))
        runs=$((runs + 1))
        e=$((e + 1))
    done
    k=$((k + 1))
done
# Each ending must have ended as it says: 0, 101 for the panics, the low
# eight bits for `exit`, 1 for `main`'s error.
codes=""
e=0
for want in 0 101 4X 255 101 101 101 101 1; do
    got=$(echo "${SEEN[$e]}" | tr ' ' '\n' | grep . | LC_ALL=C sort -u | tr '\n' ' ' | sed 's/ $//')
    case "$want" in
        4X) if ! [[ "$got" =~ ^(4[4-9]|5[0-3])( (4[4-9]|5[0-3]))*$ ]]; then echo "FAIL: ending $e exited $got, not 300 to 309's low eight bits"; exit 1; fi ;;
        *) if [ "$got" != "$want" ]; then echo "FAIL: ending $e exited $got, not $want"; exit 1; fi ;;
    esac
    codes="$codes $got"
    e=$((e + 1))
done
echo "runs identical, apart and together: $runs of $((SEEDS * 9)) ($SEEDS seeds of $ROUNDS rounds, each ending all 9 ways; $bytes bytes together)   exit statuses by ending:$codes"
[ "$runs" -eq $((SEEDS * 9)) ] && [ "$SEEDS" -ge 1 ]
