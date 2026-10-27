#!/usr/bin/env bash
# The release walk in Grasm against the Rust runtime's (docs/self-hosting.md
# §6.9, S5 part two).
#
# `compiler/rt/probe/releasefuzz.gr` builds random values of every kind the
# Grasm walk takes apart -- strings inline, on the heap and with a character
# index, lists inline and on the heap, tuples, scalar lists, both builders --
# nested in each other and around kinds the Rust runtime still takes apart,
# some held across rounds so what is dropped is shared, and drops them. It
# is built as `scripts/rtswap.sh` builds a program: linked as it ships and
# with the Grasm runtime interposed. Then:
#
#   * every seed is run on both builds under `MallocScribble` (freed memory
#     overwritten, so a buffer freed early is read as garbage), and each
#     round's checksum and live box count, the exit status and stderr must
#     be the same;
#   * a few seeds are run under `leaks --atExit` on both builds, and neither
#     may leak a block: a buffer the walk forgets to free is a leak there,
#     while the box counts above would not see it. A buffer freed twice is
#     `malloc` stopping the program, which the first check sees.
#
# The swapped build must take `grasm_val_dec` from the Grasm runtime, or it
# compared nothing. `RTREL_SEEDS`, `RTREL_ROUNDS` and `RTREL_LEAKS` size it;
# `RT_OBJ=`, `S3=` and `LINK=` name prebuilt tools (scripts/rtcontrols.sh).
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

SEEDS=${RTREL_SEEDS:-20}
ROUNDS=${RTREL_ROUNDS:-60}
LEAKRUNS=${RTREL_LEAKS:-3}

if ! "$GRASM" build compiler/rt/probe/releasefuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/releasefuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
if ! grep -qx 'grasm val_dec' "$WORK/f.swaplog"; then
    echo "FAIL: the swapped build did not take val_dec from the Grasm runtime, so it compared nothing"
    exit 1
fi

same=0
rounds=0
k=1
while [ "$k" -le "$SEEDS" ]; do
    MallocScribble=1 "$WORK/f" "$k" "$ROUNDS" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    MallocScribble=1 "$WORK/f.swap" "$k" "$ROUNDS" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $k: the two runtimes differ (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | sed 's/^/    /'
        head -3 "$WORK/b.err" | sed 's/^/    /'
        exit 1
    fi
    n=$(grep -c . "$WORK/a.out")
    if [ "$n" != "$ROUNDS" ] || [ "$ac" != 0 ]; then
        echo "FAIL: seed $k ran $n of $ROUNDS rounds and exited $ac on both"
        exit 1
    fi
    rounds=$((rounds + n))
    same=$((same + 1))
    k=$((k + 1))
done

command -v leaks >/dev/null 2>&1 || { echo "error: \`leaks\` is needed to see a buffer the walk does not free"; exit 1; }
leaky=0
leakruns=0
k=1
while [ "$k" -le "$LEAKRUNS" ]; do
    for b in f f.swap; do
        leaks --atExit -- "$WORK/$b" "$k" "$ROUNDS" >"$WORK/leaks.txt" 2>&1 </dev/null
        line=$(grep -E 'leaks? for [0-9]+ total leaked bytes' "$WORK/leaks.txt" | tail -1)
        leakruns=$((leakruns + 1))
        if [ -z "$line" ]; then
            echo "FAIL: leaks said nothing about $b seed $k: $(tail -2 "$WORK/leaks.txt" | tr '\n' ' ')"
            exit 1
        fi
        case "$line" in
            *": 0 leaks for 0 total leaked bytes."*) ;;
            *)
                leaky=$((leaky + 1))
                echo "FAIL: $b seed $k: $line"
                ;;
        esac
    done
    k=$((k + 1))
done

echo "seeds identical: $same of $SEEDS ($rounds rounds, each build under MallocScribble)   leaks runs: $leakruns   leaking: $leaky"
[ "$leaky" = 0 ] || exit 1
[ "$same" = "$SEEDS" ] && [ "$SEEDS" -ge 1 ] || exit 1
exit 0
