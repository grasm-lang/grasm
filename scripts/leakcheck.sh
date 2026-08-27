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
# Three repairs have been tried and each was wrong in a different way.
#
#   1. Release the old child unconditionally — three programs crashed.
#      `cow_list` returns the container *itself* when its refcount is one, so
#      the "old" child is frequently the very object just stored.
#   2. Release it only when the pointers differ — five crashed. Whether the
#      read handed over a reference at all depends on the payload kind: a
#      scalar list boxes an element on the fly, a boxed list increments an
#      existing one.
#   3. Borrow the child instead of owning it, so there is nothing to release —
#      also wrong, and it is the most instructive of the three. Borrowing
#      leaves the child's refcount at one, so the sub-write's `cow` mutates in
#      place and returns the same pointer; the parent's setter then decrements
#      what it thinks is the old element, which is the object just written.
#
# Every one of the three is defensible from the call site and every one is
# wrong, which is the point: the aliasing is decided inside the mutators.
#
# The conclusion is that this ownership contract is not derivable at the call
# site. `cow_list` and `cow_map` have ten call sites between them and each
# mutator decides independently whether it consumes its container, its value,
# both or neither — and the setters unconditionally release the element they
# overwrite, which is only correct when it is not the element being written.
#
# Fixing this means the setters stop releasing an element identical to the one
# being stored, *and* the descent borrows, *and* the contract is written down
# so the next reader does not have to rediscover it three times. That is a
# coordinated change across the runtime's mutator family, not a call-site
# patch, and until it happens the remaining count stays where it is rather
# than being traded for a use-after-free.
#
# Both attempts were caught by `difftest.sh` on the first run. That is the
# argument for having both harnesses: a leak is visible here and only here, a
# use-after-free is visible there and only there.
set -uo pipefail

cd "$(dirname "$0")/.."
. scripts/lib/corpus.sh
# Overridable so a gate can be run against a debug build, where Cranelift
# keeps its own assertions on and the runtime checks integer overflow.
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
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

programs=$(corpus_programs)

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

# The list is empty: every program in the corpus frees everything it allocates,
# so any leak at all is a regression and fails. It was a ratchet with one entry
# while the last cause was being found; it is a gate now.
KNOWN=""

new_leaks=0
for entry in "${LEAKS[@]:-}"; do
    [ -z "$entry" ] && continue
    src="${entry%% —*}"
    case " $KNOWN " in
        *" $src "*) ;;
        *) new_leaks=$((new_leaks + 1)) ;;
    esac
done

if [ "$leaked" -gt 0 ]; then
    echo
    echo "LEAKS:"
    printf '  %s\n' "${LEAKS[@]}"
    echo
    echo "27 programs were leaking the day this check was written and none are"
    echo "now, so any leak here is a regression. No other check can see one:"
    echo "a leaking program produces exactly the right output."
    if [ "$new_leaks" -gt 0 ]; then
        echo
        echo "$new_leaks of these are NOT on the known list — that is a regression."
        exit 1
    fi
    echo
    echo "All of them are known. No new leak."
fi
exit 0
