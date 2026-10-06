#!/usr/bin/env bash
# The workload every "N times faster than Python" claim is made on, run across
# five implementations of the *same* algorithm at the same size, interleaved.
#
# Interleaved because this machine drifts: the same binary varies by 30% run to
# run under load, and a sequential sweep cannot tell four implementations apart
# through that. Each round runs every binary once; the median of the rounds is
# the figure. Every implementation must print the same answer first, or the
# comparison is of two different programs.
set -uo pipefail
cd "$(dirname "$0")"
ROOT=$(cd ../../.. && pwd)
OUT="${TMPDIR:-/tmp}/grasm-mandelbrot"
ROUNDS="${ROUNDS:-7}"
mkdir -p "$OUT"

cc -O3 -o "$OUT/c" mandelbrot.c || exit 1
rustc -O -o "$OUT/rs" mandelbrot.rs || exit 1
"$ROOT/target/release/grasm" build --native mandelbrot_serial.gr -o "$OUT/gr_ser" >/dev/null || exit 1
"$ROOT/target/release/grasm" build --native mandelbrot_parallel.gr -o "$OUT/gr_par" >/dev/null || exit 1

want=$("$OUT/c")
for b in rs gr_ser gr_par; do
    got=$("$OUT/$b")
    [ "$got" = "$want" ] || { echo "ANSWERS DIFFER: c=$want $b=$got"; exit 1; }
done
got=$(python3 mandelbrot.py)
[ "$got" = "$want" ] || { echo "ANSWERS DIFFER: c=$want python=$got"; exit 1; }
echo "all implementations answer $want"

for b in c rs gr_ser gr_par; do "$OUT/$b" >/dev/null; done   # first-exec tax
for i in $(seq 1 "$ROUNDS"); do
    line=""
    for b in c rs gr_ser gr_par; do
        t=$( { /usr/bin/time -p "$OUT/$b" >/dev/null; } 2>&1 | awk '/^real/{printf "%.3f", $2}')
        line="$line $b=$t"
    done
    echo "round $i:$line"
done
echo "python (once, it is slow):"
t0=$(date +%s.%N); python3 mandelbrot.py >/dev/null; t1=$(date +%s.%N)
echo "  python=$(echo "$t1 - $t0" | bc)"
