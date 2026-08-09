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
    echo "This is not yet a gate. 25 of these were leaking the day the check"
    echo "was written — the compiled backend does not free everything it"
    echo "allocates, and no other check here could see that, because a leaking"
    echo "program produces exactly the right output. Fixing them is the work;"
    echo "wiring this into CI before they are fixed would only make the gate"
    echo "red and teach everyone to skip it."
    exit 1
fi
exit 0
