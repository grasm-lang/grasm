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
#     allocator limited to 0, 2 and 5 registers of each class, so nearly
#     every value spills and reloads -- doubles through d29..d31 as integers
#     through x15..x17 -- crosses calls in the few callee-saved registers
#     left, and meets block arguments in the stack. The checker must pass
#     each, and each binary must still print and exit as the Cranelift build
#     does. That spilling happened is asserted from the driver's own count.
#     The default allocator is the release one (`backtrack.gr`); the fast one
#     (`--ra=linear`, `regalloc.gr`) runs the same way with every register
#     and with 2, so both are held to the checker and to the output.
#   * control -- with `--ra-corrupt` the driver plants the classic allocator
#     bug, one register given to two intervals that overlap, in every function
#     where two such intervals exist; the checker must catch every one. Two
#     more plant only in the floating-point file, so its checking is seen
#     too: `--ra-corrupt=fp`, the same bugs among doubles alone, and
#     `--ra-corrupt=class`, a double given the general register of its
#     number. Every function that holds a double in a register (the driver's
#     `with doubles` count) must have the class error planted and caught.
#
# `S3_STOP_AT_FIRST=1` (the mutation controls', `scripts/s3controls.sh`)
# stops at the first program that fails: a planted fault needs one program to
# show it, not the whole corpus.
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
STOP=${S3_STOP_AT_FIRST:-}
stopped=0

programs=0
runs=0
pass=0
fail=0
spilled=0
functions=0
planted=0
caught=0
fp_planted=0
fp_caught=0
class_planted=0
class_caught=0
with_fp=0
fp_programs=0
declare -a FAILED=()

# corrupt KIND -- plants KIND in the program `$tir`, and sets `cn` (planted)
# and `ck` (caught) from the driver's line, or fails the program.
corrupt() {
    local flag=$1
    cn=""
    ck=""
    "$S3" "$tir" "$flag" >"$WORK/corrupt.out" 2>&1
    # ra-corrupt: corrupted N of M functions, the checker caught K
    local line
    line=$(grep '^ra-corrupt:' "$WORK/corrupt.out")
    cn=$(echo "$line" | sed -E 's/.*corrupted ([0-9]+) of.*/\1/')
    ck=$(echo "$line" | sed -E 's/.*caught ([0-9]+)$/\1/')
    if [ -z "$line" ] || [ -z "$cn" ] || [ -z "$ck" ]; then
        fail=$((fail + 1))
        FAILED+=("$src — the $flag control printed nothing it could be held to")
        return 1
    fi
    if [ "$ck" != "$cn" ]; then
        fail=$((fail + 1))
        FAILED+=("$src — $flag: $cn corruptions planted, the checker caught $ck")
    fi
    return 0
}

for k in "${!S3_SRC[@]}"; do
    if [ -n "$STOP" ] && [ "$fail" -gt 0 ]; then
        stopped=1
        break
    fi
    [ "${S3_RULE[$k]}" = "in" ] || continue
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    programs=$((programs + 1))
    prog_fp=0
    if ! grasm_build_native "$src" "$WORK/p.cl" >"$WORK/cl.log" 2>&1; then
        fail=$((fail + 1))
        FAILED+=("$src — the Cranelift build fails: $(head -1 "$WORK/cl.log")")
        continue
    fi
    $S3_RUN "$WORK/p.cl" >"$WORK/cl.out" 2>"$WORK/cl.err" </dev/null
    cl_code=$?
    for cfg in 0 2 5 linear:-1 linear:2; do
        runs=$((runs + 1))
        rm -f "$WORK/p.o"
        ra=""
        regs=$cfg
        what="$cfg registers"
        case "$cfg" in
            linear:-1) ra="--ra=linear"; regs=-1; what="the linear-scan allocator" ;;
            linear:*) ra="--ra=linear"; regs=${cfg#linear:}; what="the linear-scan allocator and $regs registers" ;;
        esac
        # shellcheck disable=SC2086
        "$S3" "$tir" -o "$WORK/p.o" --max-regs "$regs" $ra --stats >"$WORK/s3.out" 2>"$WORK/s3.err"
        code=$?
        if [ "$code" != 0 ]; then
            fail=$((fail + 1))
            FAILED+=("$src — with $what the backend answered $code: $(head -3 "$WORK/s3.err" | tr '\n' ' ')")
            continue
        fi
        # functions: F   checked: C   spill slots: S   words: W   with doubles: D
        read -r _ f _ c _ _ s _ _ _ _ dbl <"$WORK/s3.out"
        functions=$((functions + c))
        prog_fp=$dbl
        if [ "$c" != "$f" ]; then
            fail=$((fail + 1))
            FAILED+=("$src — with $what only $c of $f functions passed the checker")
            continue
        fi
        if [ "$regs" = 0 ] && [ "$s" -gt 0 ]; then
            spilled=$((spilled + 1))
        fi
        if ! s3_link "$WORK/p.o" "$WORK/p.s3" "$WORK/link.log"; then
            fail=$((fail + 1))
            FAILED+=("$src — with $what the object does not link")
            continue
        fi
        $S3_RUN "$WORK/p.s3" >"$WORK/s3run.out" 2>"$WORK/s3run.err" </dev/null
        s3_code=$?
        if cmp -s "$WORK/cl.out" "$WORK/s3run.out" && [ "$cl_code" = "$s3_code" ] && cmp -s "$WORK/cl.err" "$WORK/s3run.err"; then
            pass=$((pass + 1))
        else
            fail=$((fail + 1))
            FAILED+=("$src — with $what: exit $cl_code (Cranelift) vs $s3_code (s3)")
            echo "MISMATCH: $src with $what (exit $cl_code vs $s3_code)"
            diff "$WORK/cl.out" "$WORK/s3run.out" | head -6 | sed 's/^/  /'
        fi
    done
    # The controls.
    corrupt --ra-corrupt || continue
    planted=$((planted + cn))
    caught=$((caught + ck))
    corrupt --ra-corrupt=fp || continue
    fp_planted=$((fp_planted + cn))
    fp_caught=$((fp_caught + ck))
    corrupt --ra-corrupt=class || continue
    class_planted=$((class_planted + cn))
    class_caught=$((class_caught + ck))
    if [ "${prog_fp:-0}" -gt 0 ]; then
        fp_programs=$((fp_programs + 1))
        with_fp=$((with_fp + prog_fp))
        if [ "$cn" -lt 1 ]; then
            fail=$((fail + 1))
            FAILED+=("$src — $prog_fp functions hold doubles, and no class error could be planted")
        fi
    elif [ "$cn" != 0 ]; then
        fail=$((fail + 1))
        FAILED+=("$src — no function holds a double, yet $cn class errors were planted")
    fi
done

echo "programs: $programs   stressed runs: $runs   identical: $pass   failed: $fail"
echo "functions checked: $functions   programs that spilled with no registers: $spilled"
echo "programs with doubles: $fp_programs   functions with doubles: $with_fp"
echo "control, doubles only: planted: $fp_planted   caught: $fp_caught"
echo "control, a double in a general register: planted: $class_planted   caught: $class_caught"
echo "control: planted: $planted   caught: $caught"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
fi
if [ "$stopped" = 1 ]; then
    echo
    echo "stopped at the first failure (S3_STOP_AT_FIRST); the counts above are partial"
    exit 1
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
# The floating-point file is checked too: errors were planted among doubles,
# of both kinds, and every one was caught.
if [ "$fp_planted" -lt 1 ] || [ "$fp_caught" -ne "$fp_planted" ]; then
    echo "the doubles-only control planted $fp_planted errors and the checker caught $fp_caught"
    exit 1
fi
if [ "$class_planted" -lt "$fp_programs" ] || [ "$class_caught" -ne "$class_planted" ] || [ "$fp_programs" -lt 1 ]; then
    echo "the class control planted $class_planted errors in $fp_programs programs with doubles and the checker caught $class_caught"
    exit 1
fi
if [ -z "${DIRS:-}" ] && [ "$programs" -lt "$S3_SUBSET_FLOOR" ]; then
    echo "only $programs programs in the subset; the floor is $S3_SUBSET_FLOOR"
    exit 1
fi
[ "$programs" -gt 0 ] || { echo "nothing was checked"; exit 1; }
exit 0
