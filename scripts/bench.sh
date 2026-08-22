#!/usr/bin/env bash
# Cross-language benchmarks: the same programs in Grasm, C, Rust, Go, Nim, Zig
# and Python, built and timed the same way.
#
# Nim and Zig implement a subset — the four programs where the comparison is
# about the language rather than about which concurrency primitive each one
# reaches for — so their other cells read `-`.
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

PROGRAMS="fib sieve matmul wordfreq tree sort parmap parwords"
LANGS="c rust go nim zig grasm-native grasm-interp python"
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
#
# One `python3` that spawns the program and times it — *not* two that read a
# clock either side of it. Read either side, the second interpreter's own
# startup lands inside the measured interval: about 21 ms on this machine,
# added to every cell of every table this script has ever printed. A constant
# added to every measurement is the worst error a benchmark can have, because
# it does not look like noise — it compresses every ratio toward 1 and makes a
# slow implementation look close to a fast one. The C sieve takes about 5 ms
# and was reported at 30.
time_ms() {
    python3 - "$WORK" "$@" <<'TIMER'
import subprocess, sys, time

work, cmd = sys.argv[1], sys.argv[2:]
with open(work + "/out", "wb") as out, open(work + "/err", "wb") as err:
    started = time.perf_counter()
    done = subprocess.run(cmd, stdout=out, stderr=err)
    elapsed = (time.perf_counter() - started) * 1000
print(int(round(elapsed)))
sys.exit(0 if done.returncode == 0 else 1)
TIMER
}

# What the timer costs when there is nothing to time.
#
# This is the check that was missing when `time_ms` was measuring a Python
# interpreter's startup and adding it to every cell (ProjectGold F27). Two
# gates already watched the *inputs* to a measurement — is the answer right,
# is the machine quiet — and neither could see a timer that was wrong, because
# a constant offset reproduces perfectly.
#
# `true` does nothing, so the whole of this figure is fork, exec and wait. A
# few milliseconds is the floor of any wall-clock measurement of a process and
# is honest; anything more means the harness is timing itself again.
timer_floor() {
    local t best=""
    for _ in 1 2 3; do
        t=$(time_ms /usr/bin/true) || return 1
        if [ -z "$best" ] || [ "$t" -lt "$best" ]; then best=$t; fi
    done
    echo "$best"
}

FLOOR=$(timer_floor)
if [ -z "$FLOOR" ]; then
    echo "refusing to benchmark: the timer could not measure /usr/bin/true"
    exit 1
fi
if [ "$FLOOR" -gt 10 ]; then
    echo "refusing to benchmark: timing /usr/bin/true takes ${FLOOR}ms"
    echo
    echo "A program that does nothing cannot take that long. The harness is"
    echo "measuring itself, and every number it prints would carry the same"
    echo "amount — which does not look like noise and does not go away when"
    echo "the run is repeated. See ProjectGold/FINDINGS.md F27."
    exit 1
fi

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
        nim) [ -x "$WORK/nim_$2" ] && echo "$WORK/nim_$2" ;;
        zig) [ -x "$WORK/zig_$2" ] && echo "$WORK/zig_$2" ;;
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
    # Nim and Zig only implement the subset that has files; the rest are
    # reported as `-` rather than silently omitted.
    have nim && [ -f "$BENCH/nim/$p.nim" ] && \
        nim c -d:danger --opt:speed --hints:off --warnings:off \
            -o:"$WORK/nim_$p" "$BENCH/nim/$p.nim" >/dev/null 2>&1
    have zig && [ -f "$BENCH/zig/$p.zig" ] && \
        zig build-exe -O ReleaseFast -femit-bin="$WORK/zig_$p" \
            "$BENCH/zig/$p.zig" >/dev/null 2>&1
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
