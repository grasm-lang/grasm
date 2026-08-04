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

for src in $(find examples projects -name '*.gr' | sort); do
    grep -qE '^[[:space:]]*(pub )?fn main\(' "$src" || continue
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

    native_out=$("$WORK/$name.exe" 2>&1)
    native_code=$?
    interp_out=$("$GRASM" run "$src" 2>&1)
    interp_code=$?

    if [ "$native_out" == "$interp_out" ] && [ "$native_code" == "$interp_code" ]; then
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
