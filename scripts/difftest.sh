#!/usr/bin/env bash
# Differential test: for every runnable Grasm program, the native build must
# produce byte-identical stdout and the same exit code as the interpreter.
#
# Programs the native backend cannot compile yet are reported as SKIP with the
# feature it named — never as a pass.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=./target/release/grasm
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

programs=$(find examples projects -name '*.gr' | sort)

for src in $programs; do
    # Library modules (no `main`) are not standalone programs.
    if ! grep -qE '^\s*(pub )?fn main\(' "$src"; then
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

    interp_out=$("$GRASM" run "$src" 2>&1)
    interp_code=$?
    native_out=$("$bin" 2>&1)
    native_code=$?

    if [ "$interp_out" == "$native_out" ] && [ "$interp_code" == "$native_code" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        FAILED+=("$src")
        echo "MISMATCH: $src (exit $interp_code vs $native_code)"
        diff <(echo "$interp_out") <(echo "$native_out") | head -12
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
