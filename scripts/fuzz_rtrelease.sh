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
#     overwritten, so a buffer freed early is read as garbage) with the leak
#     check on, and each round's checksum and live box count, the exit
#     status and stderr -- the leak check's verdict, which counts every
#     thread's boxes -- must be the same, and nothing left live;
#   * a few seeds are run under `leaks --atExit` on both builds without
#     `par_map`'s workers, and neither may leak a block: a buffer the walk
#     forgets to free is a leak there, while the box counts above would not
#     see it. A buffer freed twice is `malloc` stopping the program, which
#     the first check sees. With the workers, the Rust runtime leaks a small
#     block per worker thread for each of its thread-locals the thread
#     touched, on purpose (`grasm_sys::tls`, whose destructor keeps it) --
#     in both builds, since its worker code runs in both -- and `leaks`, which
#     scans memory for pointers, does not see every one of them every run
#     (13 to 16 of the 16 here, over eight runs of the same seed). So there
#     one seed's Grasm build may leak only blocks of the sizes its Rust build
#     leaks, and of each size no more than there are workers (`hw.ncpu`, how
#     many `par_map` starts): what this runtime keeps per thread it frees
#     when the thread ends, and a block of its own left behind would be
#     another size or one too many;
#   * a few seeds are run on both builds linked with the poisoned runtime
#     (`scripts/rtswap.sh`'s), which keeps every freed box out of circulation
#     with a count no live box holds: they must agree, and neither may report
#     a use after free.
#
# The swapped build must take `grasm_val_dec` from the Grasm runtime, or it
# compared nothing. `RTREL_SEEDS`, `RTREL_ROUNDS`, `RTREL_LEAKS` and
# `RTREL_POISONED` size it; `RT_OBJ=`, `S3=` and `LINK=` name prebuilt tools
# (scripts/rtcontrols.sh), and `RT_POISON=` the poisoned runtime as for
# `scripts/rtswap.sh` (empty leaves that run out).
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
POISONED=${RTREL_POISONED:-5}
if [ -z "${RT_POISON+x}" ]; then
    if ! cargo build --release -q -p grasm_rt_abi --features grasm_rt/poison --target-dir target/s3rc-poison; then
        echo "error: the poisoned runtime does not build"
        exit 1
    fi
    RT_POISON=target/s3rc-poison/release/libgrasm_rt_abi.a
fi
if [ -n "${RT_POISON:-}" ]; then
    if [ ! -f "$RT_POISON" ]; then
        echo "error: RT_POISON=$RT_POISON is missing"
        exit 1
    fi
    export RTSWAP_POISON="$RT_POISON"
else
    POISONED=0
fi

if ! "$GRASM" build compiler/rt/probe/releasefuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ] ||
    { [ "$POISONED" -gt 0 ] && [ ! -x "$WORK/f.swap.poison" ]; }; then
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
    MallocScribble=1 GRASM_LEAK_CHECK=1 "$WORK/f" "$k" "$ROUNDS" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    MallocScribble=1 GRASM_LEAK_CHECK=1 "$WORK/f.swap" "$k" "$ROUNDS" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $k: the two runtimes differ (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | sed 's/^/    /'
        head -3 "$WORK/b.err" | sed 's/^/    /'
        exit 1
    fi
    n=$(grep -vcE '^(workers|reused) ' "$WORK/a.out")
    if [ "$n" != "$ROUNDS" ] || [ "$ac" != 0 ] || ! grep -qx 'leak check: nothing live at exit' "$WORK/a.err"; then
        echo "FAIL: seed $k ran $n of $ROUNDS rounds and exited $ac on both: $(head -2 "$WORK/a.err" | tr '\n' ' ')"
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
# Runs `$1` under `leaks` with `$2` worker rounds, setting `line` to what it
# said and `n` to how many blocks leaked; stops the gate if it said nothing.
leaked() {
    leaks --atExit -- "$WORK/$1" "$k" "$ROUNDS" "$2" >"$WORK/leaks.txt" 2>&1 </dev/null
    line=$(grep -E 'leaks? for [0-9]+ total leaked bytes' "$WORK/leaks.txt" | tail -1)
    leakruns=$((leakruns + 1))
    if [ -z "$line" ]; then
        echo "FAIL: leaks said nothing about $1 seed $k: $(tail -2 "$WORK/leaks.txt" | tr '\n' ' ')"
        exit 1
    fi
    n=$(echo "$line" | sed -E 's/.*: ([0-9]+) leaks? for.*/\1/')
    # Each leaked block's size, one per line.
    grep -oE 'ROOT LEAK: <[^>]*> \[[0-9]+\]' "$WORK/leaks.txt" | sed -E 's/.*\[([0-9]+)\]$/\1/' >"$WORK/$1.sizes"
}
while [ "$k" -le "$LEAKRUNS" ]; do
    for b in f f.swap; do
        leaked "$b" 0
        if [ "$n" != 0 ]; then
            leaky=$((leaky + 1))
            echo "FAIL: $b seed $k: $line"
        fi
    done
    k=$((k + 1))
done
worker_leaks="not run"
if [ "$LEAKRUNS" -gt 0 ]; then
    k=1
    leaked f "$ROUNDS"
    rust_n=$n
    leaked f.swap "$ROUNDS"
    grasm_n=$n
    workers=$(sysctl -n hw.ncpu)
    rust_sizes=$(sort -un "$WORK/f.sizes" | tr '\n' ' ')
    worker_leaks="$grasm_n blocks against the Rust build's $rust_n, of sizes ${rust_sizes% }, $workers workers"
    if [ "$grasm_n" != "$(wc -l <"$WORK/f.swap.sizes" | tr -d ' ')" ]; then
        leaky=$((leaky + 1))
        echo "FAIL: f.swap seed 1 with par_map's workers: leaks reported $grasm_n blocks and listed $(wc -l <"$WORK/f.swap.sizes" | tr -d ' ')"
    fi
    for size in $(sort -un "$WORK/f.swap.sizes"); do
        count=$(grep -cx "$size" "$WORK/f.swap.sizes")
        if ! grep -qx "$size" "$WORK/f.sizes"; then
            leaky=$((leaky + 1))
            echo "FAIL: f.swap seed 1 with par_map's workers: $count leaked blocks of $size bytes, a size the Rust build does not leak"
        elif [ "$count" -gt "$workers" ]; then
            leaky=$((leaky + 1))
            echo "FAIL: f.swap seed 1 with par_map's workers: $count leaked blocks of $size bytes, more than its $workers workers"
        fi
    done
fi

p_same=0
k=1
while [ "$k" -le "$POISONED" ]; do
    GRASM_LEAK_CHECK=1 "$WORK/f.poison" "$k" "$ROUNDS" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    GRASM_LEAK_CHECK=1 "$WORK/f.swap.poison" "$k" "$ROUNDS" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    if grep -q 'use after free' "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $k: a use after free under the poisoned runtime: $(grep -h 'use after free' "$WORK/a.err" "$WORK/b.err" | head -1)"
        exit 1
    fi
    if [ "$ac" != "$bc" ] || [ "$ac" != 0 ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $k: the two runtimes differ under the poisoned runtime (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | sed 's/^/    /'
        head -3 "$WORK/b.err" | sed 's/^/    /'
        exit 1
    fi
    p_same=$((p_same + 1))
    k=$((k + 1))
done

echo "seeds identical: $same of $SEEDS ($rounds rounds, each build under MallocScribble)   leaks runs: $leakruns   leaking: $leaky (with workers: $worker_leaks)   poisoned runtime: $p_same of $POISONED seeds agreeing with no use after free"
[ "$leaky" = 0 ] || exit 1
[ "$same" = "$SEEDS" ] && [ "$SEEDS" -ge 1 ] || exit 1
[ "$p_same" = "$POISONED" ] || exit 1
exit 0
