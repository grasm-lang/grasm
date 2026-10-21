#!/usr/bin/env bash
# The register allocator of the Grasm backend is checked, not trusted
# (docs/self-hosting.md §4: "a symbolic checker that verifies every
# allocation").
#
# `compiler/aarch64/src/check.gr` replays every allocated function and fails
# the build -- exit 4 -- on any read of a location that does not hold the
# value the code before allocation meant. It runs on every function of every
# compilation, so `scripts/s3subset.sh` already fails on a checker error. This
# gate makes the allocator work harder than the corpus does, and shows the
# checker can fail:
#
#   * stress -- every program in the subset is compiled again with the
#     allocator limited to 0, 2 and 5 registers, so nearly every value
#     spills and reloads, crosses calls in the few callee-saved registers
#     left, and meets block arguments in the stack. The checker must pass
#     each, and each binary must still print and exit as the Cranelift build
#     does. That spilling happened is asserted from the driver's own count.
#   * control -- with `--ra-corrupt` the driver plants the classic allocator
#     bug, one register given to two intervals that overlap, in every function
#     where two such intervals exist; the checker must catch every one.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
corpus_require

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
. "$(dirname "$0")/lib/native.sh"
s3_driver
s3_runtime
s3_collect

programs=0
runs=0
pass=0
fail=0
spilled=0
functions=0
planted=0
caught=0
declare -a FAILED=()

for k in "${!S3_SRC[@]}"; do
    [ "${S3_RULE[$k]}" = "in" ] || continue
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    programs=$((programs + 1))
    if ! grasm_build_native "$src" "$WORK/p.cl" >"$WORK/cl.log" 2>&1; then
        fail=$((fail + 1))
        FAILED+=("$src — the Cranelift build fails: $(head -1 "$WORK/cl.log")")
        continue
    fi
    $RUN_LIMIT "$WORK/p.cl" >"$WORK/cl.out" 2>"$WORK/cl.err" </dev/null
    cl_code=$?
    for regs in 0 2 5; do
        runs=$((runs + 1))
        rm -f "$WORK/p.o"
        "$S3" "$tir" -o "$WORK/p.o" --max-regs "$regs" --stats >"$WORK/s3.out" 2>"$WORK/s3.err"
        code=$?
        if [ "$code" != 0 ]; then
            fail=$((fail + 1))
            FAILED+=("$src — with $regs registers the backend answered $code: $(head -3 "$WORK/s3.err" | tr '\n' ' ')")
            continue
        fi
        # functions: F   checked: C   spill slots: S   words: W
        read -r _ f _ c _ _ s _ _ <"$WORK/s3.out"
        functions=$((functions + c))
        if [ "$c" != "$f" ]; then
            fail=$((fail + 1))
            FAILED+=("$src — with $regs registers only $c of $f functions passed the checker")
            continue
        fi
        if [ "$regs" = 0 ] && [ "$s" -gt 0 ]; then
            spilled=$((spilled + 1))
        fi
        if ! s3_link "$WORK/p.o" "$WORK/p.s3" "$WORK/link.log"; then
            fail=$((fail + 1))
            FAILED+=("$src — with $regs registers the object does not link")
            continue
        fi
        $RUN_LIMIT "$WORK/p.s3" >"$WORK/s3run.out" 2>"$WORK/s3run.err" </dev/null
        s3_code=$?
        if cmp -s "$WORK/cl.out" "$WORK/s3run.out" && [ "$cl_code" = "$s3_code" ] && cmp -s "$WORK/cl.err" "$WORK/s3run.err"; then
            pass=$((pass + 1))
        else
            fail=$((fail + 1))
            FAILED+=("$src — with $regs registers: exit $cl_code (Cranelift) vs $s3_code (s3)")
            echo "MISMATCH: $src with $regs registers (exit $cl_code vs $s3_code)"
            diff "$WORK/cl.out" "$WORK/s3run.out" | head -6 | sed 's/^/  /'
        fi
    done
    # The control.
    "$S3" "$tir" --ra-corrupt >"$WORK/corrupt.out" 2>&1
    # ra-corrupt: corrupted N of M functions, the checker caught K
    line=$(grep '^ra-corrupt:' "$WORK/corrupt.out")
    n=$(echo "$line" | sed -E 's/.*corrupted ([0-9]+) of.*/\1/')
    kk=$(echo "$line" | sed -E 's/.*caught ([0-9]+)$/\1/')
    if [ -z "$line" ] || [ -z "$n" ] || [ -z "$kk" ]; then
        fail=$((fail + 1))
        FAILED+=("$src — the corruption control printed nothing it could be held to")
        continue
    fi
    planted=$((planted + n))
    caught=$((caught + kk))
    if [ "$kk" != "$n" ]; then
        fail=$((fail + 1))
        FAILED+=("$src — $n corruptions planted, the checker caught $kk")
    fi
done

echo "programs: $programs   stressed runs: $runs   identical: $pass   failed: $fail"
echo "functions checked: $functions   programs that spilled with no registers: $spilled"
echo "control: planted: $planted   caught: $caught"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
fi
[ "$fail" -eq 0 ] || exit 1
[ "$pass" -eq "$runs" ] || exit 1
# Every program spills when it has no registers at all; one that did not was
# not stressed.
[ "$spilled" -eq "$programs" ] || { echo "only $spilled of $programs programs spilled with no registers"; exit 1; }
# Every program has at least one function with two overlapping intervals (the
# entry point's arguments), so every program must have had one planted.
if [ "$planted" -lt "$programs" ] || [ "$caught" -ne "$planted" ]; then
    echo "the control planted $planted errors in $programs programs and the checker caught $caught"
    exit 1
fi
if [ -z "${DIRS:-}" ] && [ "$programs" -lt "$S3_SUBSET_FLOOR" ]; then
    echo "only $programs programs in the subset; the floor is $S3_SUBSET_FLOOR"
    exit 1
fi
[ "$programs" -gt 0 ] || { echo "nothing was checked"; exit 1; }
exit 0
