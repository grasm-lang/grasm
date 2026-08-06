#!/usr/bin/env bash
# Cross-language benchmarks: the same five programs in Grasm, C, Rust, Go, and
# Python, built and timed the same way.
#
# Every implementation prints the same checksum, and a timing is only reported
# once the checksum matches. An optimizer that deletes the work being measured
# is the classic way a benchmark lies, and a wrong answer is not a fast answer.
#
# Written for bash 3.2, which is what macOS ships — no associative arrays.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=./target/release/grasm
BENCH=benchmarks
WORK="${TMPDIR:-/tmp}/grasm_bench_$$"
mkdir -p "$WORK"
export GOCACHE="$WORK/gocache"
trap 'rm -rf "$WORK"' EXIT

PROGRAMS="fib sieve matmul wordfreq sort parmap"
LANGS="c rust go grasm-native grasm-interp python"
# Repeats per program. Enough to smooth scheduler noise without making the
# whole run take longer than anyone will wait for.
REPS="${BENCH_REPS:-3}"

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

have() { command -v "$1" >/dev/null 2>&1; }

# A busy machine does not just make everything slower — it makes it slower
# *unevenly*. Absolute times inflate, and multi-core rows collapse, because
# the speedup a benchmark is measuring is competing with whatever else wants
# the cores. Numbers taken that way look like a regression and are not one.
#
# The harness already refuses to report a timing for a wrong answer. This is
# the same rule applied to the other way a benchmark lies.
LOAD_OK=$(python3 - <<'PY'
import os
try:
    one = os.getloadavg()[0]
    cpus = os.cpu_count() or 1
except OSError:
    print("1 unknown 0 0"); raise SystemExit
print(f"{1 if one <= cpus * 0.35 else 0} measured {one:.2f} {cpus}")
PY
)
set -- $LOAD_OK
if [ "$1" = "0" ]; then
    echo "refusing to benchmark: load average $3 on $4 cores"
    echo
    echo "Something else is using this machine. Absolute times will be inflated"
    echo "and the multi-core rows will understate the speedup, which reads as a"
    echo "regression that is not there. Wait for the machine to be idle, or set"
    echo "BENCH_IGNORE_LOAD=1 to record numbers you know not to publish."
    [ -z "${BENCH_IGNORE_LOAD:-}" ] && exit 1
    echo
    echo "BENCH_IGNORE_LOAD set — continuing; these numbers are not publishable."
fi

# Milliseconds of wall clock for one run, or empty if it failed.
time_ms() {
    local start end code
    start=$(python3 -c 'import time; print(int(time.perf_counter()*1000))')
    "$@" >"$WORK/out" 2>"$WORK/err"
    code=$?
    end=$(python3 -c 'import time; print(int(time.perf_counter()*1000))')
    [ $code -ne 0 ] && return 1
    echo $((end - start))
}

# Best of REPS, the least noisy summary of a short run.
best_ms() {
    local best="" t i=0
    while [ $i -lt "$REPS" ]; do
        t=$(time_ms "$@") || return 1
        if [ -z "$best" ] || [ "$t" -lt "$best" ]; then best=$t; fi
        i=$((i + 1))
    done
    echo "$best"
}

# The command that runs one (language, program) pair, or empty if unavailable.
run_cmd() {
    case "$1" in
        grasm-interp) echo "$GRASM run $BENCH/grasm/$2.gr" ;;
        python) have python3 && echo "python3 $BENCH/python/$2.py" ;;
        grasm-native) [ -x "$WORK/grasm_$2" ] && echo "$WORK/grasm_$2" ;;
        c) [ -x "$WORK/c_$2" ] && echo "$WORK/c_$2" ;;
        rust) [ -x "$WORK/rs_$2" ] && echo "$WORK/rs_$2" ;;
        go) [ -x "$WORK/go_$2" ] && echo "$WORK/go_$2" ;;
    esac
}

echo "building..."
for p in $PROGRAMS; do
    "$GRASM" build "$BENCH/grasm/$p.gr" --native -o "$WORK/grasm_$p" >/dev/null 2>&1
    have cc && cc -O2 -pthread -o "$WORK/c_$p" "$BENCH/c/$p.c" >/dev/null 2>&1
    have rustc && rustc -O -o "$WORK/rs_$p" "$BENCH/rust/$p.rs" >/dev/null 2>&1
    have go && go build -o "$WORK/go_$p" "$BENCH/go/$p.go" >/dev/null 2>&1
done

# Results are kept in files rather than an associative array so this runs on
# the bash macOS ships.
wrong=0
for p in $PROGRAMS; do
    # The reference answer comes from the interpreter: it is the definitional
    # implementation, so it is what everything else has to match.
    expected=$("$GRASM" run "$BENCH/grasm/$p.gr" 2>/dev/null)
    echo "$expected" >"$WORK/expected_$p"
    for lang in $LANGS; do
        cmd=$(run_cmd "$lang" "$p")
        [ -z "$cmd" ] && continue
        # shellcheck disable=SC2086
        got=$($cmd 2>/dev/null)
        if [ "$got" != "$expected" ]; then
            echo "$lang:$p — got [$got] want [$expected]" >>"$WORK/wrong"
            echo "WRONG" >"$WORK/r_${lang}_$p"
            wrong=1
            continue
        fi
        # shellcheck disable=SC2086
        ms=$(best_ms $cmd) && echo "$ms" >"$WORK/r_${lang}_$p"
    done
    echo "  $p done"
done

echo
printf '%-14s' "program"
for lang in $LANGS; do printf '%14s' "$lang"; done
echo
printf '%-14s' "--------------"
for lang in $LANGS; do printf '%14s' "------------"; done
echo

for p in $PROGRAMS; do
    printf '%-14s' "$p"
    for lang in $LANGS; do
        f="$WORK/r_${lang}_$p"
        if [ ! -f "$f" ]; then
            printf '%14s' "-"
        else
            v=$(cat "$f")
            if [ "$v" = "WRONG" ]; then
                printf '%14s' "WRONG"
            else
                printf '%13s ' "${v}ms"
            fi
        fi
    done
    echo
done

echo
echo "best of $REPS runs, wall clock in milliseconds (lower is better)"
echo "checksums verified against the interpreter before timing"

if [ $wrong -ne 0 ]; then
    echo
    echo "WRONG ANSWERS (not timed):"
    sed 's/^/  /' "$WORK/wrong"
    exit 1
fi
