#!/usr/bin/env bash
# Two builds of the compiler, on the same programs, alternately.
#
# `bench.sh` refuses to run on a busy machine, and is right to: absolute times
# inflate unevenly under load and the multi-core rows collapse, so a number
# taken that way is not comparable with anything. But a *ratio between two
# builds of this compiler* survives load, as long as neither build gets a
# quieter minute than the other — which is what alternating them buys, and how
# the stack-guard cost was measured here before.
#
# So this answers one question only: did the change make Grasm faster than
# Grasm was. The cross-language table stays `bench.sh`'s job, on an idle
# machine.
#
#   scripts/abbench.sh <baseline grasm> [<candidate grasm>] [programs...]
#
# The candidate defaults to ./target/release/grasm — the build in the tree.
# A baseline is a *directory's worth*: `grasm` needs `libgrasm_rt.a` beside it
# to link anything, so snapshot both —
#
#   mkdir -p /tmp/base && cp target/release/grasm target/release/libgrasm_rt.a /tmp/base/
# Each program is run `BENCH_REPS` times per side, alternating A B A B, and
# the *best* time of each side is compared: the fastest run is the one least
# interrupted, which is the closest thing to an idle machine a busy one has.
set -uo pipefail

cd "$(dirname "$0")/.."
BASE=${1:?usage: abbench.sh <baseline grasm> [candidate grasm] [programs...]}
shift
CAND=./target/release/grasm
if [ $# -gt 0 ] && [ -x "$1" ]; then
    CAND=$1
    shift
fi
PROGRAMS=${*:-"fib sieve matmul wordfreq tree sort parmap parwords"}
REPS="${BENCH_REPS:-7}"

WORK="${TMPDIR:-/tmp}/grasm_ab_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

for side in base cand; do
    exe=$BASE
    [ "$side" = "cand" ] && exe=$CAND
    if [ ! -x "$exe" ]; then
        echo "error: $exe is not executable"
        exit 1
    fi
done

echo "baseline:  $BASE"
echo "candidate: $CAND"
echo

# Microsecond-resolution milliseconds for one run — *processor* time, not
# wall time.
#
# To three decimal places, because `getrusage` reports microseconds and a
# whole-millisecond figure cannot see a ten-percent change in a thirty-
# millisecond program: a real win measured that way came out looking like a
# two-millisecond loss until the same programs were run at twenty times the
# size.
#
# Wall time on a loaded machine is mostly a measurement of the other programs:
# two identical builds came out 1.68x apart that way. Processor time is what
# this program was charged for, and a scheduler taking the core away does not
# add to it. For the multi-core rows it is the total work across threads
# rather than the latency, which is the right thing to compare when the
# question is whether the compiler emits less work.
time_ms() {
    python3 - "$@" <<'INNER'
import resource
import subprocess
import sys
before = resource.getrusage(resource.RUSAGE_CHILDREN)
done = subprocess.run(sys.argv[1:], capture_output=True)
after = resource.getrusage(resource.RUSAGE_CHILDREN)
cpu = ((after.ru_utime - before.ru_utime) +
       (after.ru_stime - before.ru_stime)) * 1000
text = done.stdout.decode().strip().replace("\n", " ")[:60]
if done.returncode != 0:
    print("-1.0 " + done.stderr.decode()[-80:].replace("\n", " "))
else:
    print("%.3f %s" % (cpu, text))
INNER
}

# The smaller of two decimal times, ignoring a failed run (-1).
lower() {
    awk -v a="$1" -v b="$2" 'BEGIN { print (a >= 0 && a < b) ? a : b }'
}

printf '%-10s %9s %9s %8s   %s\n' program baseline candidate change answer
for p in $PROGRAMS; do
    src=benchmarks/grasm/$p.gr
    [ -f "$src" ] || continue
    if ! "$BASE" build "$src" --native -o "$WORK/a_$p" >"$WORK/a_$p.log" 2>&1; then
        printf '%-10s %9s %9s %8s   %s\n' "$p" "-" "-" "-" "baseline build failed"
        continue
    fi
    if ! "$CAND" build "$src" --native -o "$WORK/b_$p" >"$WORK/b_$p.log" 2>&1; then
        printf '%-10s %9s %9s %8s   %s\n' "$p" "-" "-" "-" "candidate build failed"
        continue
    fi
    best_a=999999.0
    best_b=999999.0
    answer_a=""
    answer_b=""
    n=0
    while [ "$n" -lt "$REPS" ]; do
        set -- $(time_ms "$WORK/a_$p")
        ms=$1; shift; out="$*"
        best_a=$(lower "$ms" "$best_a")
        answer_a=$out
        set -- $(time_ms "$WORK/b_$p")
        ms=$1; shift; out="$*"
        best_b=$(lower "$ms" "$best_b")
        answer_b=$out
        n=$((n + 1))
    done
    # A faster wrong answer is not a result. The two builds must agree.
    if [ "$answer_a" != "$answer_b" ]; then
        printf '%-10s %9s %9s %8s   %s\n' "$p" "$best_a" "$best_b" "DIFFER" \
            "$answer_a | $answer_b"
        continue
    fi
    change=$(python3 -c "
a, b = $best_a, $best_b
print('same' if b == a else ('%.2fx faster' % (a / b) if b < a else '%.2fx SLOWER' % (b / a)))")
    printf '%-10s %9s %9s %8s   %s\n' "$p" "$best_a" "$best_b" "" "$change"
done
