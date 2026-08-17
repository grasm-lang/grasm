#!/usr/bin/env bash
# The wasm build of the interpreter must agree with the interpreter.
#
# A fourth engine either enters the agreement gate or is not trusted, and this
# one is cheap to hold to the standard: it *is* the reference interpreter, built
# for a different machine, so a disagreement means the target changed a meaning
# rather than that a second implementation drifted.
#
# Three things genuinely differ, and each is a refusal rather than a wrong
# answer: wasm32-unknown-unknown has no threads (`par_map` runs on one core), no
# sockets (`net` cannot bind), and no filesystem (`read_file` fails as it would
# on a machine with no such file). Programs that need one are reported as SKIP
# with the reason — never as a pass.
set -uo pipefail
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
WASM=${WASM:-./target/wasm32-unknown-unknown/release/grasm_wasm.wasm}

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi
if [ ! -f "$WASM" ]; then
    echo "error: $WASM missing — run:"
    echo "  cargo build --release --target wasm32-unknown-unknown -p grasm_wasm"
    exit 1
fi
if ! command -v node >/dev/null 2>&1; then
    echo "error: node is needed to run a wasm module outside a browser"
    exit 1
fi

pass=0
fail=0
skip=0
declare -a FAILED=()
declare -a SKIPPED=()

DIRS=${DIRS:-"examples projects spec/conformance benchmarks/grasm"}
# shellcheck disable=SC2086
programs=$(find $DIRS -name '*.gr' | sort)

for src in $programs; do
    grep -qE '^\s*(pub )?fn main\(' "$src" || continue
    grep -q '^//! expect: error$' "$src" && continue

    # `import` reads sibling files, and this target has no filesystem. Skipped
    # by shape rather than by failure, so a real disagreement stays visible.
    if grep -qE '^\s*import ' "$src"; then
        skip=$((skip + 1))
        SKIPPED+=("$src — imports a module, and wasm32 has no filesystem")
        continue
    fi
    for need in par_map:threads tcp_listen:sockets tcp_accept:sockets \
                read_file:filesystem write_file:filesystem; do
        if grep -q "${need%%:*}" "$src"; then
            skip=$((skip + 1))
            SKIPPED+=("$src — needs ${need##*:}, which wasm32-unknown-unknown has none of")
            continue 2
        fi
    done
    # `extern fn` reaches a C symbol, which needs a linker and a process that
    # has one loaded. A wasm module has neither, and says so by name.
    if grep -qE '^\s*(pub )?extern fn ' "$src"; then
        skip=$((skip + 1))
        SKIPPED+=("$src — calls a C function, and a wasm module has no linker")
        continue
    fi
    # A wasm module's stack is fixed at link time and cannot be enlarged from
    # inside, so the depth [E-RECURSION-DEPTH] promises is not reachable here.
    # These two programs exist to recurse until something gives; on this target
    # the module's stack gives first. It is a smaller number, not a different
    # meaning — but it is a difference, and it is named rather than absorbed.
    case "$src" in
        */E-RECURSION-DEPTH.gr|*/E-PANIC-TRACE.gr)
            skip=$((skip + 1))
            SKIPPED+=("$src — recurses past the stack a wasm module is given")
            continue
            ;;
    esac

    i_err=$(mktemp); w_err=$(mktemp)
    i_out=$("$GRASM" run "$src" 2>"$i_err"); i_code=$?
    w_out=$(node scripts/wasm/run.mjs "$WASM" "$src" 2>"$w_err"); w_code=$?

    if [ "$i_out" != "$w_out" ]; then
        fail=$((fail + 1)); FAILED+=("$src — stdout differs")
    elif [ "$i_code" != "$w_code" ]; then
        fail=$((fail + 1)); FAILED+=("$src — exit $i_code (run) vs $w_code (wasm)")
    else
        pass=$((pass + 1))
    fi
    rm -f "$i_err" "$w_err"
done

echo "identical: $pass   mismatched: $fail   skipped: $skip"
if [ ${#SKIPPED[@]} -gt 0 ]; then
    echo
    echo "skipped — the host has no such facility:"
    printf '  %s\n' "${SKIPPED[@]}"
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "MISMATCHED:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
