#!/usr/bin/env bash
# The Grasm backend for x86-64 (docs/self-hosting.md §6.16, stage S7) must
# print and exit exactly as the Cranelift x86-64 build does, and as the
# interpreter does, on every corpus program in the subset -- `crosstest` with
# the Grasm backend as the compiler, run under Rosetta on this arm64 machine.
#
# For each corpus program with a `main` (the same TIR and the same subset
# rule as `s3subset`):
#
#   * membership -- the backend compiles every program the rule has in and
#     refuses (exit 3) every one it has out, as for arm64;
#   * agreement -- `s3 --target x86_64` compiles it; the system linker links
#     the object with the Rust runtime built for x86_64-apple-darwin (until
#     `compiler/link` links x86-64, step four's work: this step's recorded
#     gap); `grasm build --native --target x86_64-apple-darwin` makes the
#     Cranelift object, linked the same way; both run with `arch -x86_64`,
#     stdin closed. stdout, the exit status and stderr must be identical to
#     the Cranelift build's, and stdout and the exit status to `grasm run`'s,
#     whose stderr is compared with its panic trace taken out ([E-PANIC-TRACE]),
#     as `crosstest` compares them.
#
# A program that links a system library is set aside, counted, and must be
# one `sdltest` builds, as `s3subset` requires.
#
# `S3=<binary>` tests that driver; `S3_EXTRA` adds arguments to each
# compilation; `S3_STOP_AT_FIRST=1` stops at the first failure (the
# controls, `scripts/x64controls.sh`). `S3_RT_X64=<archive>` names the x86-64
# runtime; otherwise it is built, as `crosstest` builds it.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/trace.sh"
. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
corpus_require

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
s3_driver

TARGET=x86_64-apple-darwin
if [ -z "${S3_RT_X64:-}" ]; then
    if [ "${GRASM_NO_BUILD:-}" != 1 ] && ! cargo build --release -q -p grasm_rt_abi --target "$TARGET"; then
        echo "error: the runtime for $TARGET does not build (rustup target add $TARGET?)"
        exit 1
    fi
    S3_RT_X64=./target/$TARGET/release/libgrasm_rt_abi.a
fi
if [ ! -f "$S3_RT_X64" ]; then
    echo "error: $S3_RT_X64 is missing"
    exit 1
fi
if ! arch -x86_64 /usr/bin/true 2>/dev/null; then
    echo "error: x86-64 programs do not run here (is Rosetta installed?)"
    exit 1
fi

s3_collect
EXTRA=${S3_EXTRA:-}
STOP=${S3_STOP_AT_FIRST:-}
stopped=0
in_subset=0
out_subset=0
pass=0
fail=0
timedout=0
external=0
declare -a FAILED=()
declare -a TIMEDOUT=()
declare -a EXTERNAL=()

for k in "${!S3_SRC[@]}"; do
    if [ -n "$STOP" ] && [ $((fail + timedout)) -gt 0 ]; then
        stopped=1
        break
    fi
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    rule=${S3_RULE[$k]}
    obj="$WORK/p.o"
    rm -f "$obj" "$WORK/p.x" "$WORK/p.clx" "$WORK/cl.o"
    case "$rule" in
    external:*)
        external=$((external + 1))
        EXTERNAL+=("$src")
        continue
        ;;
    esac
    # shellcheck disable=SC2086
    "$S3" "$tir" -o "$obj" --target x86_64 $EXTRA >"$WORK/s3.out" 2>"$WORK/s3.err"
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
    if [ "$code" != 0 ]; then
        fail=$((fail + 1))
        FAILED+=("$src — in the subset, and the backend answered $code: $(head -3 "$WORK/s3.err" | tr '\n' ' ')")
        continue
    fi
    if ! cc -arch x86_64 "$obj" "$S3_RT_X64" -o "$WORK/p.x" -Wl,-dead_strip >"$WORK/link.log" 2>&1; then
        fail=$((fail + 1))
        FAILED+=("$src — the object does not link: $(head -2 "$WORK/link.log" | tr '\n' ' ')")
        continue
    fi
    if ! "$GRASM" build "$src" --native --target "$TARGET" -o "$WORK/cl" >"$WORK/cl.log" 2>&1 ||
        ! cc -arch x86_64 "$WORK/cl.o" "$S3_RT_X64" -o "$WORK/p.clx" >>"$WORK/cl.log" 2>&1; then
        fail=$((fail + 1))
        FAILED+=("$src — the Cranelift x86-64 build fails: $(head -1 "$WORK/cl.log")")
        continue
    fi
    $S3_RUN arch -x86_64 "$WORK/p.clx" >"$WORK/cl.out" 2>"$WORK/cl.err" </dev/null
    cl_code=$?
    $S3_RUN arch -x86_64 "$WORK/p.x" >"$WORK/x.out" 2>"$WORK/x.err" </dev/null
    x_code=$?
    # The interpreter runs under the corpus's limit (`corpus.sh`), as in
    # `crosstest`: a compiled program's 20 s is far too little for it (a
    # parallel map or a password hash, interpreted, takes minutes).
    $RUN_LIMIT "$GRASM" run "$src" >"$WORK/i.out" 2>"$WORK/i.err" </dev/null
    i_code=$?
    if [ "$cl_code" = 124 ] || [ "$x_code" = 124 ] || [ "$i_code" = 124 ]; then
        timedout=$((timedout + 1))
        TIMEDOUT+=("$src — Cranelift $cl_code, s3 $x_code, interpreter $i_code")
        echo "TIMED OUT: $src"
        continue
    fi
    i_diag=$(strip_trace "$(cat "$WORK/i.err")")
    x_diag=$(cat "$WORK/x.err")
    if cmp -s "$WORK/cl.out" "$WORK/x.out" && [ "$cl_code" = "$x_code" ] && cmp -s "$WORK/cl.err" "$WORK/x.err" &&
        cmp -s "$WORK/i.out" "$WORK/x.out" && [ "$i_code" = "$x_code" ] && [ "$i_diag" == "$x_diag" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        FAILED+=("$src — exit $cl_code (Cranelift x86-64) vs $x_code (s3 x86-64) vs $i_code (interpreter)")
        echo "MISMATCH: $src (exit $cl_code Cranelift, $x_code s3, $i_code interpreter)"
        diff "$WORK/cl.out" "$WORK/x.out" | head -8 | sed 's/^/  /'
        diff "$WORK/cl.err" "$WORK/x.err" | head -6 | sed 's/^/  /'
        diff "$WORK/i.out" "$WORK/x.out" | head -4 | sed 's/^/  interp: /'
    fi
done

echo
echo "target: $TARGET, run with arch -x86_64"
echo "subset: $in_subset in, $out_subset out, $external external, of ${#S3_SRC[@]} programs with a main"
echo "identical: $pass   mismatched: $fail   timed out: $timedout   no TIR: ${#S3_NOTIR[@]}"
if [ "$external" -gt 0 ]; then
    corpus_external_covered "${EXTERNAL[@]}" || exit 1
fi
if [ ${#S3_NOTIR[@]} -gt 0 ]; then
    echo
    echo "emit no TIR:"
    printf '  %s\n' "${S3_NOTIR[@]}"
fi
if [ ${#TIMEDOUT[@]} -gt 0 ]; then
    echo
    echo "TIMED OUT after ${S3_RUN_SECONDS}s compiled or ${RUN_LIMIT_SECONDS}s interpreted (not agreement):"
    printf '  %s\n' "${TIMEDOUT[@]}"
fi
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
if [ $((in_subset + out_subset + external)) -ne ${#S3_SRC[@]} ]; then
    echo "the counts do not add up: $in_subset + $out_subset + $external is not ${#S3_SRC[@]}"
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
