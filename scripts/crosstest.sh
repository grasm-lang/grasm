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
# A runtime older than its own source links against yesterday's symbol set, so
# every program using anything added since fails to link. That looked exactly
# like "the backend cannot compile this" the one time it happened, and cost
# seven programs of coverage without a word of complaint.
if [ -n "$(find crates/grasm_rt/src -name '*.rs' -newer "$RT" 2>/dev/null)" ]; then
    echo "error: $RT is older than crates/grasm_rt/src — rebuild it:"
    echo "  cargo build --release -p grasm_rt --target $TARGET"
    exit 1
fi

pass=0
fail=0
skip=0
declare -a FAILED=()
declare -a SKIPPED=()

# Same scan roots as difftest.sh: a program the host backend is held to must
# hold on every other target too, or "portable" means only what was sampled.
for src in $(find examples projects spec/conformance benchmarks/grasm -name '*.gr' | sort); do
    grep -qE '^[[:space:]]*(pub )?fn main\(' "$src" || continue
    # Conformance programs tagged `expect: error` are meant not to compile.
    grep -q '^//! expect: error$' "$src" && continue
    name=$(echo "$src" | tr '/.' '__')

    # A skip says the toolchain could not produce a binary, so it has to say
    # which step gave up and why. Counting it silently is how a stale artifact
    # passes for a backend limitation.
    if ! "$GRASM" build "$src" --native --target "$TARGET" -o "$WORK/$name" >"$WORK/$name.buildlog" 2>&1; then
        skip=$((skip + 1))
        SKIPPED+=("$src — compile: $(head -1 "$WORK/$name.buildlog")")
        continue
    fi
    cc -arch "$ARCH" "$WORK/$name.o" "$RT" -o "$WORK/$name.exe" 2>"$WORK/$name.linklog"
    if [ ! -x "$WORK/$name.exe" ]; then
        skip=$((skip + 1))
        SKIPPED+=("$src — link: $(head -1 "$WORK/$name.linklog")")
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

if [ "$skip" -gt 0 ]; then
    echo
    echo "UNSUPPORTED (each one is coverage this run did not get):"
    printf '  %s\n' "${SKIPPED[@]}"
fi

if [ "$fail" -gt 0 ]; then
    printf 'FAILED: %s\n' "${FAILED[@]}"
    exit 1
fi
exit 0
