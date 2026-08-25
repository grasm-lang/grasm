#!/usr/bin/env bash
# Differential test: for every runnable Grasm program, the native build must
# produce byte-identical stdout and the same exit code as the interpreter.
#
# Programs the native backend cannot compile yet are reported as SKIP with the
# feature it named — never as a pass.
set -uo pipefail

cd "$(dirname "$0")/.."
# Overridable so a gate can be run against a debug build, where Cranelift
# keeps its own assertions on and the runtime checks integer overflow.
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
. "$(dirname "$0")/lib/trace.sh"
fresh_or_stop "./target/release/grasm"
WORK="${TMPDIR:-/tmp}/grasm_difftest_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

pass=0
fail=0
skip=0
declare -a FAILED=()
declare -a SKIPPED=()

# Overridable so a slower build can be held to the same programs without the
# benchmarks, which are sized for a release interpreter and take minutes each
# under a debug one.
DIRS=${DIRS:-"examples projects spec/conformance benchmarks/grasm"}
# shellcheck disable=SC2086
programs=$(find $DIRS -name '*.gr' | sort)

for src in $programs; do
    # Library modules (no `main`) are not standalone programs.
    if ! grep -qE '^\s*(pub )?fn main\(' "$src"; then
        continue
    fi
    # Conformance programs tagged `expect: error` are meant not to compile;
    # there is no runtime behaviour for the two engines to disagree about.
    if grep -q '^//! expect: error$' "$src"; then
        continue
    fi
    name=$(echo "$src" | tr '/.' '__')
    bin="$WORK/$name"

    build_log=$("$GRASM" build "$src" --native -o "$bin" 2>&1)
    if [ ! -x "$bin" ]; then
        reason=$(echo "$build_log" | grep -o "does not yet support [^—]*" | head -1)
        [ -z "$reason" ] && reason=$(echo "$build_log" | head -1)
        skip=$((skip + 1))
        SKIPPED+=("$src — ${reason:-unknown}")
        continue
    fi

    # stdout and the exit code are what a program *produces*, and they must
    # match exactly. stderr carries diagnostics, and a panic's stack trace is
    # the one thing on it the two engines are allowed to differ about —
    # [E-PANIC-TRACE] says which of them prints one and why — so that block is
    # stripped before stderr is compared. Everything else on stderr, the panic
    # line included, has to be identical.
    interp_err=$(mktemp)
    native_err=$(mktemp)
    interp_out=$("$GRASM" run "$src" 2>"$interp_err")
    interp_code=$?
    native_out=$("$bin" 2>"$native_err")
    native_code=$?
    interp_diag=$(strip_trace "$(cat "$interp_err")")
    native_diag=$(cat "$native_err")
    rm -f "$interp_err" "$native_err"

    if [ "$interp_out" == "$native_out" ] \
        && [ "$interp_code" == "$native_code" ] \
        && [ "$interp_diag" == "$native_diag" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        FAILED+=("$src")
        echo "MISMATCH: $src (exit $interp_code vs $native_code)"
        diff <(echo "$interp_out") <(echo "$native_out") | head -12
        diff <(echo "$interp_diag") <(echo "$native_diag") | head -6
    fi
done

echo
echo "=========================================="
echo "identical: $pass   mismatched: $fail   unsupported: $skip"
echo "=========================================="

if [ "$skip" -gt 0 ]; then
    echo
    echo "Not yet compilable natively (interpreter handles these):"
    printf '  %s\n' "${SKIPPED[@]}"
fi

if [ "$fail" -gt 0 ]; then
    echo
    echo "FAILURES:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
exit 0
