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
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
WASM=${WASM:-./target/wasm32-unknown-unknown/release/grasm_wasm.wasm}

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi
# Built here rather than assumed, because the module on disk is not evidence
# that the module still *builds*. It was not, once: `grasm_interp` grew a
# dependency on the 64-bit runtime, wasm32 stopped compiling entirely, and this
# gate went on comparing a module from the previous week and reporting 182
# identical. A staleness check that trusts a stale reference is not one.
#
# Skipped when the target is not installed, with a message rather than a pass:
# a machine without wasm32 cannot check this and should not pretend to.
if rustc --print target-list 2>/dev/null | grep -qx wasm32-unknown-unknown; then
    if ! cargo build --release --target wasm32-unknown-unknown -p grasm_wasm 2>&1 | tail -20; then
        echo "error: the wasm build failed — the module below would be stale"
        exit 1
    fi
elif [ ! -f "$WASM" ]; then
    echo "error: wasm32-unknown-unknown is not installed and $WASM is missing — run:"
    echo "  rustup target add wasm32-unknown-unknown"
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

# Where the source bundles for multi-module programs are written.
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
# shellcheck disable=SC2086
programs=$(corpus_programs)

for src in $programs; do
    grep -qE '^\s*(pub )?fn main\(' "$src" || continue
    grep -q '^//! expect: error$' "$src" && continue

    # A program with `import` used to be skipped here — "wasm32 has no
    # filesystem", which is true of the *disk loader* and not of the engine. A
    # source bundle is that program with its modules laid end to end, the
    # interpreter loads one through the same function the disk uses, and
    # `grasm build --source` writes one. So the modules are bundled and the
    # bundle is what wasm runs.
    #
    # It was the largest category of skip by far — 38 of 72 — and every one of
    # them was coverage lost for a reason the engine did not have (F81).
    #
    # `subject` is what the shape checks below read: for a bundled program that
    # is every module, not only the entry. A library that opens a socket is a
    # program that needs one, and grepping the entry file alone could not see
    # it.
    subject="$src"
    run_src="$src"
    if grep -qE '^\s*import ' "$src"; then
        bundle="$WORK/$(echo "$src" | tr '/.' '__').bundle.gr"
        if ! "$GRASM" build "$src" --source -o "$bundle" >/dev/null 2>&1; then
            skip=$((skip + 1))
            SKIPPED+=("$src — its modules could not be bundled")
            continue
        fi
        subject="$bundle"
        run_src="$bundle"
    fi
    # This list is the harness's model of what the target cannot do, and it
    # has to grow when the library does. `write_bytes` and `proc_pipeline`
    # arrived after it was written, so two conformance programs were reported
    # as *mismatches* rather than skips — the same drift as the committed
    # module below, in the other direction.
    for need in par_map:threads par_filter:threads spawn:threads \
                tcp_listen:sockets tcp_accept:sockets tcp_connect:sockets \
                udp_bind:sockets net_wait:sockets \
                read_file:filesystem write_file:filesystem \
                read_bytes:filesystem write_bytes:filesystem \
                random_bytes:filesystem \
                list_dir:filesystem make_dir:filesystem \
                proc_run:processes proc_pipeline:processes; do
        # Code, not prose. The name was matched anywhere in the file, so
        # `T-EXTERN-VALUE.gr` -- which mentions `par_map` in a comment
        # explaining why a purity proof matters -- was skipped as needing
        # threads it never asks for. A skip is coverage lost, and one taken
        # for a facility the program does not use is lost for nothing.
        if grep -vE '^\s*(//|#)' "$subject" | grep -q "${need%%:*}"; then
            skip=$((skip + 1))
            SKIPPED+=("$src — needs ${need##*:}, which wasm32-unknown-unknown has none of")
            continue 2
        fi
    done
    # `extern fn` reaches a C symbol, which needs a linker and a process that
    # has one loaded. A wasm module has neither, and says so by name.
    if grep -qE '^\s*(pub )?extern fn ' "$subject"; then
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
    w_out=$(node scripts/wasm/run.mjs "$WASM" "$run_src" 2>"$w_err"); w_code=$?

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

# The *committed* modules, not just the one just built.
#
# Everything above tests a fresh build. The pages under ProjectGold serve a
# checked-in copy, and nothing was comparing the two — so that copy sat four
# months behind the language, refusing `eprint` and `byte_len` in a browser
# while every command-line engine accepted them. Found by a project trying to
# run its own source in the browser; it would not have been found by this
# script, which is the point of adding this to it.
#
# There is one real module and the pages that need it link to it, so a refresh
# is one copy rather than four -- every change to the compiler made all four
# stale, and a chore repeated every commit is one that eventually is not done.
# `-h` skips the links; without it the glob also matched `web/grasm.wasm`
# through its own name and the count said five stale modules where four exist.
stale=0
# `web` is a child of ProjectGold, so the glob already names it; listing it
# explicitly as well counted it twice and reported five stale modules where
# four exist -- and then two where one did.
for committed in ProjectGold/*/grasm.wasm; do
    [ -f "$committed" ] && [ ! -h "$committed" ] || continue
    if ! cmp -s "$WASM" "$committed"; then
        echo "stale: $committed differs from a fresh build of grasm_wasm"
        stale=$((stale + 1))
    fi
done
if [ "$stale" -gt 0 ]; then
    echo
    echo "$stale committed wasm module(s) are behind the language. Refresh with:"
    echo "  (the pages under ProjectGold link to web/grasm.wasm, so this is all of them)"
    echo "  cargo build --release --target wasm32-unknown-unknown -p grasm_wasm"
    echo "  cp target/wasm32-unknown-unknown/release/grasm_wasm.wasm ProjectGold/web/grasm.wasm"
    exit 1
fi
