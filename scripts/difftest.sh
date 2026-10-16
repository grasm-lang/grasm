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
. "$(dirname "$0")/lib/native.sh"
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
external=0
declare -a FAILED=()
declare -a SKIPPED=()
declare -a EXTERNAL=()

. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
programs=$(corpus_programs)

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
    # A program whose package graph declares a system library cannot be
    # compared by this gate at all: `grasm run` reaches only symbols already
    # loaded in its own process, so the interpreter side of the comparison
    # cannot call into C and there is no second opinion to have. That is
    # structural, not a gap -- `scripts/sdltest.sh` checks these against real
    # SDL2 instead.
    #
    # Asked of the compiler rather than inferred from a build failure or kept
    # as a list of filenames here. A list rots, and matching on the error text
    # would make this gate agree with whatever the message happens to say.
    if corpus_external "$src"; then
        external=$((external + 1))
        EXTERNAL+=("$src — $("$GRASM" links "$src" 2>/dev/null | tr '\n' ' ')")
        continue
    fi
    # One reused file for the whole corpus, so macOS admits a new executable
    # once instead of once per program (see lib/native.sh).
    bin="$WORK/p"

    build_log=$(grasm_build_native "$src" "$bin")
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
echo "identical: $pass   mismatched: $fail   unsupported: $skip   external: $external"
echo "=========================================="

# A program the backend declines is not a program this gate has checked, and
# §11's list is one entry long with nothing in the corpus on it — so a skip
# here is a defect. It was counted, listed *after* the summary, and passed
# anyway; `allgates.sh` prints only the summary line, so the listing was never
# read. A change that made `shapes.Circle(2.0)` uncompilable moved one program
# from "identical" to "unsupported" and every gate still said PASS (F76).
# `crosstest.sh` has treated a skip as a failure since the same mistake there.
if [ "$skip" -gt 0 ]; then
    echo
    echo "the native backend declined these, and §11 lists none of them:"
    printf '  %s\n' "${SKIPPED[@]}"
fi

if [ "$fail" -gt 0 ]; then
    echo
    echo "FAILURES:"
    printf '  %s\n' "${FAILED[@]}"
fi

# An excluded program is only excluded honestly if something else checks it.
# Counting them and printing the count is what F76 was about: the number was
# there and nobody compared it to anything. So the set this gate skipped must
# be exactly the set `sdltest.sh` builds -- a new program calling into C that
# nobody added to that gate fails here instead of quietly going unchecked.
if [ "$external" -gt 0 ]; then
    echo
    echo "not comparable, so checked by scripts/sdltest.sh instead:"
    printf '  %s\n' "${EXTERNAL[@]}"
    progs=()
    for entry in "${EXTERNAL[@]}"; do
        progs+=("${entry%% —*}")
    done
    corpus_external_covered "${progs[@]}" || exit 1
fi
if [ "$fail" -gt 0 ] || [ "$skip" -gt 0 ]; then
    exit 1
fi
exit 0
