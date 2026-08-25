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

. "$(dirname "$0")/lib/trace.sh"
. "$(dirname "$0")/lib/fresh.sh"
cd "$(dirname "$0")/.."
TARGET="${1:-x86_64-apple-darwin}"
ARCH="${TARGET%%-*}"
GRASM=./target/release/grasm
RT="target/$TARGET/release/libgrasm_rt.a"
WORK="${TMPDIR:-/tmp}/grasm_crosstest_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

fresh_or_stop ./target/release/grasm
if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi
# The runtime for the other target is this gate's own artefact, and a gate
# builds what it tests. It used to only complain: a `cargo clean` removed the
# archive, the gate refused to start, and — because it was not in the sweep
# anyone actually ran — nothing cross-compiled for weeks. A runtime older than
# its own source is the same failure more quietly, linking against yesterday's
# symbol set so every program using anything added since fails to link.
if [ "${GRASM_NO_BUILD:-}" != 1 ]; then
    if ! cargo build --release -q -p grasm_rt --target "$TARGET"; then
        echo "error: the runtime for $TARGET does not build. If the target is"
        echo "       not installed: rustup target add $TARGET"
        exit 1
    fi
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
    interp_diag=$(strip_trace "$(cat "$interp_err")")
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
        # Three things are compared and only one was ever shown. A program
        # whose stdout and exit code agree and whose *diagnostics* differ was
        # reported as a mismatch with an empty diff under it, which reads as a
        # harness fault rather than a finding.
        if [ "$native_out" != "$interp_out" ]; then
            echo "  stdout:"
            diff <(echo "$interp_out") <(echo "$native_out") | head -10
        fi
        if [ "$interp_diag" != "$native_diag" ]; then
            echo "  diagnostics:"
            diff <(echo "$interp_diag") <(echo "$native_diag") | head -10
        fi
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
