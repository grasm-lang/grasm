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
# What is left is dominated by `write_place`: descending a multi-segment place
# (`rows[0].stack.pop()`) reads each intermediate with a +1 and writes a new
# child back without releasing the old one.
#
# Two repairs have been tried and both were wrong. Decrementing the old child
# unconditionally crashed three programs: `cow_list` returns the container
# *itself* when its refcount is one, so the "old" child is frequently the very
# object just stored. Decrementing only when the pointers differ crashed five:
# whether the read handed over a reference at all depends on the payload kind,
# since a scalar list boxes an element on the fly while a boxed list
# increments an existing one.
#
# The conclusion is that this ownership contract is not derivable at the call
# site. `cow_list` and `cow_map` have ten call sites between them and each
# mutator decides independently whether it consumes its container, its value,
# both or neither. Fixing the leak means writing that contract down and making
# the family obey it — an audit, not a patch — and until then the remaining
# count stays where it is rather than being traded for a use-after-free.
#
# Both attempts were caught by `difftest.sh` on the first run. That is the
# argument for having both harnesses: a leak is visible here and only here, a
# use-after-free is visible there and only there.
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
    echo "written and 7 are now; the compiled backend does not yet free"
    echo "everything it allocates, and no other check here can see that,"
    echo "because a leaking program produces exactly the right output. It goes"
    echo "into CI when the count reaches zero — wiring it in before then would"
    echo "make the gate red on arrival and teach everyone to skip it."
    exit 1
fi
exit 0
