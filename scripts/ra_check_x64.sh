#!/usr/bin/env bash
# `ra_check` for the x86-64 target (docs/self-hosting.md §6.16, stage S7):
# the shared allocators and their checker (`compiler/vcode`) held to the
# x86-64 description, as `scripts/ra_check.sh` holds them to AArch64's.
#
#   * stress -- every program in the subset compiled by `s3 --target x86_64`
#     with 0, 2 and 5 registers of each class under the release allocator,
#     and with every register and with 2 under linear scan; spilled values
#     go through r10 and r11 (doubles through xmm14 and xmm15), values that
#     cross a call into the five callee-saved registers or the stack (System
#     V preserves no xmm register). The checker must pass every function and
#     each binary must print and exit as the Cranelift x86-64 build does,
#     both run under Rosetta;
#   * control -- `--ra-corrupt`, `=fp` and `=class` plant their errors in
#     every function they can, and the checker must catch each.
#
# The system linker links each object with the x86-64 Rust runtime
# (`S3_RT_X64=`, else built as `crosstest` builds it). `S3_STOP_AT_FIRST=1`
# stops at the first failure (`scripts/x64controls.sh`).
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
TARGET=x86_64-apple-darwin
if [ -z "${S3_RT_X64:-}" ]; then
    if [ "${GRASM_NO_BUILD:-}" != 1 ] && ! cargo build --release -q -p grasm_rt_abi --target "$TARGET"; then
        echo "error: the runtime for $TARGET does not build"
        exit 1
    fi
    S3_RT_X64=./target/$TARGET/release/libgrasm_rt_abi.a
fi
[ -f "$S3_RT_X64" ] || { echo "error: $S3_RT_X64 is missing"; exit 1; }
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
fp_in_memory=0
declare -a FAILED=()

# corrupt KIND -- plants KIND in the program `$tir`, and sets `cn` (planted)
# and `ck` (caught) from the driver's line, or fails the program.
corrupt() {
    local flag=$1
    cn=""
    ck=""
    "$S3" "$tir" "$flag" --target x86_64 >"$WORK/corrupt.out" 2>&1
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
    rm -f "$WORK/cl.o" "$WORK/p.cl"
    if ! "$GRASM" build "$src" --native --target "$TARGET" -o "$WORK/cl" >"$WORK/cl.log" 2>&1 ||
        ! cc -arch x86_64 "$WORK/cl.o" "$S3_RT_X64" -o "$WORK/p.cl" >>"$WORK/cl.log" 2>&1; then
        fail=$((fail + 1))
        FAILED+=("$src — the Cranelift x86-64 build fails: $(head -1 "$WORK/cl.log")")
        continue
    fi
    $S3_RUN arch -x86_64 "$WORK/p.cl" >"$WORK/cl.out" 2>"$WORK/cl.err" </dev/null
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
        "$S3" "$tir" -o "$WORK/p.o" --target x86_64 --max-regs "$regs" $ra --stats >"$WORK/s3.out" 2>"$WORK/s3.err"
        code=$?
        if [ "$code" != 0 ]; then
            fail=$((fail + 1))
            FAILED+=("$src — with $what the backend answered $code: $(head -3 "$WORK/s3.err" | tr '\n' ' ')")
            continue
        fi
        # functions: F   checked: C   spill slots: S   bytes: B   with doubles: D
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
        if ! cc -arch x86_64 "$WORK/p.o" "$S3_RT_X64" -o "$WORK/p.s3" -Wl,-dead_strip >"$WORK/link.log" 2>&1; then
            fail=$((fail + 1))
            FAILED+=("$src — with $what the object does not link")
            continue
        fi
        $S3_RUN arch -x86_64 "$WORK/p.s3" >"$WORK/s3run.out" 2>"$WORK/s3run.err" </dev/null
        s3_code=$?
        if cmp -s "$WORK/cl.out" "$WORK/s3run.out" && [ "$cl_code" = "$s3_code" ] && cmp -s "$WORK/cl.err" "$WORK/s3run.err"; then
            pass=$((pass + 1))
        else
            fail=$((fail + 1))
            FAILED+=("$src — with $what: exit $cl_code (Cranelift x86-64) vs $s3_code (s3 x86-64)")
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
        # System V preserves no xmm register across a call, so a program
        # whose every double lives across one keeps them all in memory, and
        # a double given a general register cannot be planted in it: counted
        # apart, where `ra_check` (arm64, d8..d15 preserved) requires one.
        if [ "$cn" -lt 1 ]; then
            fp_in_memory=$((fp_in_memory + 1))
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
echo "control, a double in a general register: planted: $class_planted   caught: $class_caught   (programs where no double is given a register, so none can be planted: $fp_in_memory)"
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
if [ "$class_planted" -lt $((fp_programs - fp_in_memory)) ] || [ "$class_caught" -ne "$class_planted" ] || [ "$fp_programs" -lt 1 ] || [ "$fp_in_memory" -ge "$fp_programs" ]; then
    echo "the class control planted $class_planted errors in $fp_programs programs with doubles and the checker caught $class_caught"
    exit 1
fi
if [ -z "${DIRS:-}" ] && [ "$programs" -lt "$S3_SUBSET_FLOOR" ]; then
    echo "only $programs programs in the subset; the floor is $S3_SUBSET_FLOOR"
    exit 1
fi
[ "$programs" -gt 0 ] || { echo "nothing was checked"; exit 1; }
exit 0
