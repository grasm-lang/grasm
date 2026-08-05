#!/usr/bin/env bash
# Cross-compilation differential test.
#
# Builds every runnable program for a non-host target, links it against a
# runtime built for that target, runs it, and requires byte-identical output
# to the interpreter. Proves the backend is genuinely portable rather than
# accidentally host-specific.
#
# Usage: scripts/crosstest.sh [target-triple]
#   default target: x86_64-apple-darwin (runs under Rosetta on arm64 macOS)
set -uo pipefail

cd "$(dirname "$0")/.."
TARGET="${1:-x86_64-apple-darwin}"
ARCH="${TARGET%%-*}"
GRASM=./target/release/grasm
RT="target/$TARGET/release/libgrasm_rt.a"
WORK="${TMPDIR:-/tmp}/grasm_crosstest_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi
if [ ! -f "$RT" ]; then
    echo "error: runtime for $TARGET missing — run:"
    echo "  rustup target add $TARGET && cargo build --release -p grasm_rt --target $TARGET"
    exit 1
fi

pass=0
fail=0
skip=0
declare -a FAILED=()

# Same scan roots as difftest.sh: a program the host backend is held to must
# hold on every other target too, or "portable" means only what was sampled.
for src in $(find examples projects spec/conformance benchmarks/grasm -name '*.gr' | sort); do
    grep -qE '^[[:space:]]*(pub )?fn main\(' "$src" || continue
    # Conformance programs tagged `expect: error` are meant not to compile.
    grep -q '^//! expect: error$' "$src" && continue
    name=$(echo "$src" | tr '/.' '__')

    if ! "$GRASM" build "$src" --native --target "$TARGET" -o "$WORK/$name" >/dev/null 2>&1; then
        skip=$((skip + 1))
        continue
    fi
    cc -arch "$ARCH" "$WORK/$name.o" "$RT" -o "$WORK/$name.exe" 2>/dev/null
    if [ ! -x "$WORK/$name.exe" ]; then
        skip=$((skip + 1))
        continue
    fi

    # stdout and the exit code are what a program *produces* and must match
    # exactly. stderr carries diagnostics, and the interpreter's panic stack
    # trace has no native counterpart (see README, "Debugging"), so that block
    # is stripped before stderr is compared — same rule as difftest.sh.
    native_err=$(mktemp)
    interp_err=$(mktemp)
    native_out=$("$WORK/$name.exe" 2>"$native_err")
    native_code=$?
    interp_out=$("$GRASM" run "$src" 2>"$interp_err")
    interp_code=$?
    interp_diag=$(grep -v -e '^stack trace (innermost first):$' -e '^  [A-Za-z_][A-Za-z0-9_:]* at ' "$interp_err")
    native_diag=$(cat "$native_err")
    rm -f "$native_err" "$interp_err"

    if [ "$native_out" == "$interp_out" ] \
        && [ "$native_code" == "$interp_code" ] \
        && [ "$interp_diag" == "$native_diag" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        FAILED+=("$src")
        echo "MISMATCH: $src (exit $interp_code vs $native_code)"
        diff <(echo "$interp_out") <(echo "$native_out") | head -10
    fi
done

echo
echo "=========================================="
echo "target: $TARGET"
echo "identical: $pass   mismatched: $fail   unsupported: $skip"
echo "=========================================="

if [ "$fail" -gt 0 ]; then
    printf 'FAILED: %s\n' "${FAILED[@]}"
    exit 1
fi
exit 0
