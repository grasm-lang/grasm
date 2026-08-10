#!/usr/bin/env bash
# Does a compiled program free everything it allocated?
#
# The differential harness compares output, and a program that leaks produces
# exactly the right output — so a leak is invisible to every other check here.
# This runs the corpus with `GRASM_LEAK_CHECK=1`, which makes the runtime
# report allocations still live when `main` returns and exit 7.
#
# It is the prerequisite for Tier 1: a `region` works by *not* decrementing
# inside the block and sweeping at the brace, and a miscount there is either a
# double-free or a leak. Only one of those announces itself.
#
# One cause is already isolated, and it accounts for a good share of the 25.
# A *tuple pattern* binding a boxed component leaks that component:
#
#     let p = (1, "one")
#     match p { (a, b) -> print("{a} {b}") }     # leaks the string
#
# A tuple of two integers is clean, and a *variant* pattern binding a string is
# clean, so it is specific to the tuple path. There, each component is read out
# with `grasm_list_get`, which returns a +1, and bound to a temporary made with
# `new_var` — which is not in a scope, so nothing releases it. The variant path
# binds through the scope and is released with it.
#
# The fix is one of "release the temporary" or "bind it into the scope", and
# which one is right depends on whether the sub-pattern's own binding takes the
# +1 or aliases it. Getting that backwards is a use-after-free, so it wants
# tracing rather than a guess — and now there is a check that will confirm it
# either way.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=./target/release/grasm
WORK="${TMPDIR:-/tmp}/grasm_leak_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

clean=0
leaked=0
skipped=0
declare -a LEAKS=()

programs=$(find examples projects spec/conformance benchmarks/grasm -name '*.gr' | sort)

for src in $programs; do
    grep -qE '^\s*(pub )?fn main\(' "$src" || continue
    grep -q '^//! expect: error$' "$src" && continue
    name=$(echo "$src" | tr '/.' '__')
    bin="$WORK/$name"
    if ! "$GRASM" build "$src" --native -o "$bin" >/dev/null 2>&1; then
        skipped=$((skipped + 1))
        continue
    fi
    err=$(GRASM_LEAK_CHECK=1 "$bin" 2>&1 >/dev/null)
    code=$?
    if [ $code -eq 7 ]; then
        leaked=$((leaked + 1))
        LEAKS+=("$src — $(grep -o 'leak: .*' <<<"$err" | head -1)")
    else
        clean=$((clean + 1))
    fi
done

echo
echo "=========================================="
echo "leak-free: $clean   leaking: $leaked   not compiled: $skipped"
echo "=========================================="

if [ "$leaked" -gt 0 ]; then
    echo
    echo "LEAKS:"
    printf '  %s\n' "${LEAKS[@]}"
    echo
    echo "Not yet a gate. 27 programs were leaking the day this check was"
    echo "written and 17 are now; the compiled backend does not yet free"
    echo "everything it allocates, and no other check here can see that,"
    echo "because a leaking program produces exactly the right output. It goes"
    echo "into CI when the count reaches zero — wiring it in before then would"
    echo "make the gate red on arrival and teach everyone to skip it."
    exit 1
fi
exit 0
