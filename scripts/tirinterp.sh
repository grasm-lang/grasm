#!/usr/bin/env bash
# The TIR interpreter must print and exit exactly as `grasm run` does
# (docs/self-hosting.md, stage S0).
#
# `tools/tirinterp` is written in Grasm and runs a program from its TIR alone.
# If it agrees with the reference interpreter on a program, the TIR of that
# program carried everything the program means: nothing had to be looked up
# in the source, inferred again, or resolved twice. Where it cannot agree,
# either it has a bug or TIR is missing something -- and the second is the
# thing this stage exists to find before a backend is built on TIR.
#
# Two checks per program:
#
#   * round trip -- every program that emits TIR is read by the interpreter's
#     own reader and printed back, and the bytes must be the ones `grasm tir`
#     wrote. The reader is what everything else stands on, and a reader that
#     quietly drops a field reads a different program.
#   * agreement -- every program with a `main` is run both ways, stdin closed,
#     and stdout, the exit status and stderr must match. The one thing on
#     stderr the two may differ about is the reference's panic trace
#     ([E-PANIC-TRACE]): the interpreter is a native build, which prints none.
#
# A program this interpreter structurally cannot run is skipped by a rule that
# names the reason, and the count is printed. A skip is coverage lost, so
# there is one rule, it is narrow, and it is asked of the TIR rather than
# guessed from a failure.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
. "$(dirname "$0")/lib/trace.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
corpus_require

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# Built here, natively and once, so the gate cannot pass against an
# interpreter from before the source it claims to check.
TIRINTERP="$WORK/tirinterp"
if ! "$GRASM" build tools/tirinterp/src/main.gr --native -o "$TIRINTERP" >"$WORK/build.log" 2>&1; then
    echo "error: tools/tirinterp does not build:"
    sed 's/^/  /' "$WORK/build.log"
    exit 1
fi

roundtrip=0
rt_failed=0
pass=0
fail=0
timedout=0
mains=0
declare -a FAILED=()
declare -a TIMEDOUT=()
declare -a SKIPPED=()
skip_extern=0

for src in $(corpus_programs); do
    # Conformance programs that must not compile have no TIR.
    if grep -c '^//! expect: error$' "$src" >/dev/null; then
        continue
    fi
    tir="$WORK/p.tir"
    if ! "$GRASM" tir "$src" >"$tir" 2>"$WORK/emit.err"; then
        rt_failed=$((rt_failed + 1))
        FAILED+=("$src — emits no TIR: $(head -1 "$WORK/emit.err")")
        continue
    fi
    if "$TIRINTERP" --print "$tir" "$WORK/back.tir" 2>"$WORK/rt.err" && cmp -s "$tir" "$WORK/back.tir"; then
        roundtrip=$((roundtrip + 1))
    else
        rt_failed=$((rt_failed + 1))
        FAILED+=("$src — reading and printing its TIR does not give the same bytes $(head -1 "$WORK/rt.err")")
    fi

    grep -cE '^[[:space:]]*(pub )?fn main\(' "$src" >/dev/null || continue
    mains=$((mains + 1))

    # A C function is reached through a linker, or through a process that
    # already has it loaded, which is how `grasm run` finds `write` and
    # `system` in itself. The interpreter is a Grasm program, and Grasm can
    # call only the C functions it was compiled against: it has no function
    # pointers to call a symbol it looked up (docs/self-hosting.md §5). So a
    # program that declares one -- to call it, or to pass it as a value, as
    # `T-EXTERN-VALUE` passes `sqrt` to `map` -- is outside what any
    # interpreter written in Grasm can run today. Asked of the TIR, which
    # writes every `extern` declaration out.
    if grep -c '^(extern ' "$tir" >/dev/null; then
        skip_extern=$((skip_extern + 1))
        SKIPPED+=("$src — declares a C function (\`extern fn\`)")
        continue
    fi

    ref_out="$WORK/ref.out"
    ref_err="$WORK/ref.err"
    tir_out="$WORK/tir.out"
    tir_err="$WORK/tir.err"
    $RUN_LIMIT "$GRASM" run "$src" >"$ref_out" 2>"$ref_err" </dev/null
    ref_code=$?
    $RUN_LIMIT "$TIRINTERP" "$tir" >"$tir_out" 2>"$tir_err" </dev/null
    tir_code=$?
    if [ "$ref_code" = 124 ] || [ "$tir_code" = 124 ]; then
        timedout=$((timedout + 1))
        TIMEDOUT+=("$src — grasm run $ref_code, tirinterp $tir_code")
        continue
    fi
    ref_diag=$(strip_trace "$(cat "$ref_err")")
    tir_diag=$(cat "$tir_err")
    if cmp -s "$ref_out" "$tir_out" && [ "$ref_code" = "$tir_code" ] && [ "$ref_diag" == "$tir_diag" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        FAILED+=("$src — exit $ref_code (grasm run) vs $tir_code (tirinterp)")
        echo "MISMATCH: $src (exit $ref_code vs $tir_code)"
        diff "$ref_out" "$tir_out" | head -8 | sed 's/^/  /'
        diff <(echo "$ref_diag") <(echo "$tir_diag") | head -6 | sed 's/^/  /'
    fi
done

skipped=$skip_extern
echo
echo "round trip: $roundtrip identical, $rt_failed not"
echo "identical: $pass   mismatched: $fail   timed out: $timedout   skipped (declares a C function): $skipped"

if [ ${#SKIPPED[@]} -gt 0 ]; then
    echo
    echo "skipped -- the interpreter structurally cannot run these:"
    printf '  %s\n' "${SKIPPED[@]}"
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

# Every program with a `main` was compared, skipped by a named rule, or is a
# failure above: the counts must add up, or a program fell through a gap in
# this script.
if [ $((pass + fail + timedout + skipped)) -ne "$mains" ]; then
    echo "the counts do not add up: $pass + $fail + $timedout + $skipped is not $mains programs with a main"
    exit 1
fi
if [ "$fail" -ne 0 ] || [ "$rt_failed" -ne 0 ] || [ "$timedout" -ne 0 ]; then
    exit 1
fi
# A gate that compared nothing has checked nothing. The floors are the
# corpus as it stands, less a margin; a shrinking corpus should be noticed.
if [ -z "${DIRS:-}" ] && { [ "$pass" -lt 600 ] || [ "$roundtrip" -lt 1000 ]; }; then
    echo "too few programs compared ($pass) or round-tripped ($roundtrip) for this to mean anything"
    exit 1
fi
if [ "$pass" -lt 1 ] || [ "$roundtrip" -lt 1 ]; then
    echo "nothing was compared"
    exit 1
fi
exit 0
