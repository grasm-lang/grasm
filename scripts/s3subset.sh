#!/usr/bin/env bash
# The Grasm backend (docs/self-hosting.md, stage S3) must print and exit
# exactly as the Cranelift build does, on every corpus program in the subset
# it compiles.
#
# `compiler/s3` reads a program's TIR and writes a Mach-O object with its own
# instruction selection, register allocator, encoder and object writer; the
# object links with `cc` and the runtime, as a Cranelift object does. For each
# corpus program with a `main`:
#
#   * membership -- `scripts/s3_subset.py` states the subset and decides it
#     from the TIR. The backend decides it too, by refusing (exit 3) what it
#     does not compile. The two must agree both ways: a program the rule has
#     in that the backend refuses is coverage lost without a word, and one the
#     rule has out that the backend compiles is a rule that no longer says
#     what is checked.
#   * agreement -- every program in the subset is compiled by both backends,
#     linked, and run with stdin closed; stdout, the exit status and stderr
#     must be identical. Both are native builds, so neither prints a panic
#     trace ([E-PANIC-TRACE]) and stderr is compared whole.
#
# The count in the subset is printed and held to a floor (`scripts/lib/s3.sh`)
# so a subset that shrank cannot pass by checking less.
#
# `S3=<binary>` tests that driver instead of building one, and `S3_EXTRA`
# adds arguments to each compilation: the mutation controls
# (`scripts/s3controls.sh`) use both.
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
EXTRA=${S3_EXTRA:-}

in_subset=0
out_subset=0
pass=0
fail=0
timedout=0
declare -a FAILED=()
declare -a TIMEDOUT=()
declare -a IN=()

for k in "${!S3_SRC[@]}"; do
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    rule=${S3_RULE[$k]}
    obj="$WORK/p.o"
    rm -f "$obj"
    # shellcheck disable=SC2086
    "$S3" "$tir" -o "$obj" $EXTRA >"$WORK/s3.out" 2>"$WORK/s3.err"
    code=$?
    if [ "$rule" != "in" ]; then
        out_subset=$((out_subset + 1))
        if [ "$code" != 3 ]; then
            fail=$((fail + 1))
            FAILED+=("$src — outside the subset ($rule), but the backend answered $code, not a refusal: $(head -1 "$WORK/s3.err")")
        fi
        continue
    fi
    in_subset=$((in_subset + 1))
    IN+=("$src")
    if [ "$code" != 0 ]; then
        fail=$((fail + 1))
        FAILED+=("$src — in the subset, and the backend answered $code: $(head -3 "$WORK/s3.err" | tr '\n' ' ')")
        continue
    fi
    if ! s3_link "$obj" "$WORK/p.s3" "$WORK/link.log"; then
        fail=$((fail + 1))
        FAILED+=("$src — the object does not link: $(head -2 "$WORK/link.log" | tr '\n' ' ')")
        continue
    fi
    if ! grasm_build_native "$src" "$WORK/p.cl" >"$WORK/cl.log" 2>&1; then
        fail=$((fail + 1))
        FAILED+=("$src — the Cranelift build fails: $(head -1 "$WORK/cl.log")")
        continue
    fi
    $RUN_LIMIT "$WORK/p.cl" >"$WORK/cl.out" 2>"$WORK/cl.err" </dev/null
    cl_code=$?
    $RUN_LIMIT "$WORK/p.s3" >"$WORK/s3run.out" 2>"$WORK/s3run.err" </dev/null
    s3_code=$?
    if [ "$cl_code" = 124 ] || [ "$s3_code" = 124 ]; then
        timedout=$((timedout + 1))
        TIMEDOUT+=("$src — Cranelift $cl_code, s3 $s3_code")
        continue
    fi
    if cmp -s "$WORK/cl.out" "$WORK/s3run.out" && [ "$cl_code" = "$s3_code" ] && cmp -s "$WORK/cl.err" "$WORK/s3run.err"; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        FAILED+=("$src — exit $cl_code (Cranelift) vs $s3_code (s3)")
        echo "MISMATCH: $src (exit $cl_code vs $s3_code)"
        diff "$WORK/cl.out" "$WORK/s3run.out" | head -8 | sed 's/^/  /'
        diff "$WORK/cl.err" "$WORK/s3run.err" | head -6 | sed 's/^/  /'
    fi
done

echo
echo "in the subset, by scripts/s3_subset.py:"
printf '  %s\n' "${IN[@]}"
echo
echo "subset: $in_subset in, $out_subset out, of ${#S3_SRC[@]} programs with a main"
echo "identical: $pass   mismatched: $fail   timed out: $timedout   no TIR: ${#S3_NOTIR[@]}"

if [ ${#S3_NOTIR[@]} -gt 0 ]; then
    echo
    echo "emit no TIR:"
    printf '  %s\n' "${S3_NOTIR[@]}"
fi
if [ ${#TIMEDOUT[@]} -gt 0 ]; then
    echo
    echo "TIMED OUT after ${RUN_LIMIT_SECONDS}s (not agreement):"
    printf '  %s\n' "${TIMEDOUT[@]}"
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
fi

# Every program was in or out, and every one in was compared or failed: the
# counts must add up, or a program fell through a gap in this script.
if [ $((in_subset + out_subset)) -ne ${#S3_SRC[@]} ]; then
    echo "the counts do not add up: $in_subset + $out_subset is not ${#S3_SRC[@]}"
    exit 1
fi
if [ "$fail" -ne 0 ] || [ "$timedout" -ne 0 ] || [ ${#S3_NOTIR[@]} -ne 0 ]; then
    exit 1
fi
if [ "$pass" -ne "$in_subset" ]; then
    echo "$pass of $in_subset programs in the subset agreed"
    exit 1
fi
if [ -z "${DIRS:-}" ] && [ "$in_subset" -lt "$S3_SUBSET_FLOOR" ]; then
    echo "only $in_subset programs are in the subset; the floor is $S3_SUBSET_FLOOR"
    exit 1
fi
if [ "$pass" -lt 1 ]; then
    echo "nothing was compared"
    exit 1
fi
exit 0
