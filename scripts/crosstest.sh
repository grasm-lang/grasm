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
. scripts/lib/corpus.sh
TARGET="${1:-x86_64-apple-darwin}"
ARCH="${TARGET%%-*}"
GRASM=./target/release/grasm
RT="target/$TARGET/release/libgrasm_rt_abi.a"
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
    if ! cargo build --release -q -p grasm_rt_abi --target "$TARGET"; then
        echo "error: the runtime for $TARGET does not build. If the target is"
        echo "       not installed: rustup target add $TARGET"
        exit 1
    fi
fi
if [ ! -f "$RT" ]; then
    echo "error: runtime for $TARGET missing — run:"
    echo "  rustup target add $TARGET && cargo build --release -p grasm_rt_abi --target $TARGET"
    exit 1
fi

pass=0
fail=0
skip=0
external=0
declare -a FAILED=()
declare -a SKIPPED=()
declare -a EXTERNAL=()

# Same scan roots as difftest.sh: a program the host backend is held to must
# hold on every other target too, or "portable" means only what was sampled.
for src in $(corpus_programs); do
    grep -qE '^[[:space:]]*(pub )?fn main\(' "$src" || continue
    # Conformance programs tagged `expect: error` are meant not to compile.
    grep -q '^//! expect: error$' "$src" && continue
    # A program needing a system library needs *that target's* copy of it,
    # which is a second machine-shaped requirement on top of the search path:
    # the SDL2 installed here is arm64 and this builds for x86_64. Excluded
    # the same way `difftest.sh` and `leakcheck.sh` exclude it, through the
    # one definition in lib/corpus.sh.
    if corpus_external "$src"; then
        external=$((external + 1))
        EXTERNAL+=("$src")
        continue
    fi
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
echo "identical: $pass   mismatched: $fail   unsupported: $skip   external: $external"
echo "=========================================="

# "Coverage this run did not get" and then exit 0 was the same shape as a
# generator counting a decline: a clean run against a corpus that mostly did
# not run. It reported 60 identical and 166 unsupported once, and passed. A
# clean tree cross-compiles all of them, so zero is the right expectation and
# anything else is either a real gap -- which belongs in `nativegaps.sh` -- or
# a broken run.
if [ "$skip" -gt 0 ]; then
    echo
    echo "UNSUPPORTED (each one is coverage this run did not get):"
    printf '  %s\n' "${SKIPPED[@]}"
    echo
    echo "$skip program(s) did not cross-compile. That is coverage lost, not a"
    echo "clean run: every program in the corpus builds for this target."
    echo
    echo "If the count is large and the reasons are blank, check the machine's"
    echo "load before the compiler: this reported 60 identical and 166"
    echo "unsupported once, at a load average of 132 on eight cores, and the"
    echo "same tree was clean when the machine was idle."
    exit 1
fi

if [ "$fail" -gt 0 ]; then
    printf 'FAILED: %s\n' "${FAILED[@]}"
    exit 1
fi
# A program needing a system library was not cross-compiled, so it was not
# checked here. Only honest if another gate builds it, which is asserted.
if [ "$external" -gt 0 ]; then
    echo
    echo "needs a system library for the target, so built by sdltest instead:"
    printf '  %s\n' "${EXTERNAL[@]}"
    corpus_external_covered "${EXTERNAL[@]}" || exit 1
fi
exit 0
