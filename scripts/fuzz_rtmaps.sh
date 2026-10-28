#!/usr/bin/env bash
# Maps on the Grasm runtime must be the Rust runtime's maps, placement and
# all (docs/self-hosting.md §6.11).
#
# `compiler/rt/probe/mapfuzz.gr` drives the map family by its C names over a
# handful of maps -- keys of every kind, `set` by value and by bytes, `get`
# three ways, deletions in bursts that leave tombstones, counting, `keys`,
# `values` and `entries`, maps shared and then changed -- and prints a line
# per operation. It is built twice, linked as it ships and with the Grasm
# runtime interposed (`scripts/lib/rtswap.sh`); each must take every map
# name but `has` and `is_empty` (which answer an `i8` a program's `extern
# fn` cannot declare; the corpus reaches both) from its runtime, and:
#
#   * with the seed pinned on both sides (the program exports `getentropy`,
#     which both runtimes draw their seed from), each line carries a digest
#     of the whole table -- counts, every control byte, every full bucket's
#     key and value -- and the two builds must print the same bytes: the
#     same hash for the same seed, every entry in the same bucket;
#   * without a pin, each runtime draws its own seed as in any program, and
#     the lines -- answers, lengths, what iteration presents -- must still be
#     the same.
#
# Every run must end with nothing it made still live. RTMAPS_SEEDS (30)
# pinned seeds and RTMAPS_FREE (5) unpinned ones, RTMAPS_OPS (1500)
# operations each. `RT_OBJ=`, `S3=` and `LINK=` as for `scripts/rtswap.sh`.
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

SEEDS=${RTMAPS_SEEDS:-30}
FREE=${RTMAPS_FREE:-5}
OPS=${RTMAPS_OPS:-1500}

if ! "$GRASM" build compiler/rt/probe/mapfuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/mapfuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
missing=0
for need in $(awk '$1 == "maps" { print $2 }' compiler/rt/families.txt); do
    case "$need" in map_has | map_is_empty) continue ;; esac
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime"
        missing=$((missing + 1))
    fi
done
[ "$missing" = 0 ] || exit 1
taken=$(grep -c '^grasm map_' "$WORK/f.swaplog")

# Runs seed `$1` on both builds, pinned to `$2` when given; fails naming the
# first line that differs.
both() {
    "$WORK/f" "$1" "$OPS" ${2:+"$2"} >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    local ac=$?
    "$WORK/f.swap" "$1" "$OPS" ${2:+"$2"} >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    local bc=$?
    if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $1${2:+ pinned to $2}: the two runtimes differ (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | cut -c1-200 | sed 's/^/    /'
        head -2 "$WORK/b.err" | sed 's/^/    /'
        return 1
    fi
    local n
    n=$(grep -vc '^live ' "$WORK/a.out")
    if [ "$ac" != 0 ] || [ "$n" != "$OPS" ] || ! grep -qx 'live 0' "$WORK/a.out"; then
        echo "FAIL: seed $1: $n of $OPS operations, exit $ac, $(grep '^live ' "$WORK/a.out")"
        return 1
    fi
}

pinned=0
k=1
while [ "$k" -le "$SEEDS" ]; do
    both "$k" "$((7919 * k + 104729))" || exit 1
    pinned=$((pinned + 1))
    k=$((k + 1))
done
free=0
k=1
while [ "$k" -le "$FREE" ]; do
    both "$((1000 + k))" "" || exit 1
    free=$((free + 1))
    k=$((k + 1))
done
echo "pinned seeds with every table identical: $pinned of $SEEDS   unpinned seeds identical: $free of $FREE   ($OPS operations each; map names taken from Grasm: $taken)"
[ "$pinned" -eq "$SEEDS" ] && [ "$free" -eq "$FREE" ] && [ "$SEEDS" -ge 1 ]
