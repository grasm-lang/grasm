#!/usr/bin/env bash
# Checks the `--native` gap list in docs/tutorial.md against the compiler.
#
# The list is a promise about what a compiled build refuses. A promise nobody
# checks is a promise that rots: one of these starting to compile would leave
# the documentation telling a reader not to try something that works.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=./target/release/grasm
WORK="${TMPDIR:-/tmp}/grasm_gaps_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

fail=0

# expect_refused <name> <source>
expect_refused() {
    local name="$1" src="$2"
    printf '%s\n' "$src" >"$WORK/p.gr"
    rm -f "$WORK/out"
    local err
    err=$("$GRASM" build "$WORK/p.gr" --native -o "$WORK/out" 2>&1 | grep -o "does not yet support[^—]*")
    if [ -z "$err" ]; then
        echo "  NOW COMPILES: $name"
        echo "    remove it from the table in docs/tutorial.md §11"
        fail=1
    else
        echo "  refused: $name"
    fi
    # Whatever happens, the interpreter must run it — that is what makes this
    # a gap in the backend rather than a limit of the language.
    if ! "$GRASM" run "$WORK/p.gr" >/dev/null 2>&1; then
        echo "  INTERPRETER FAILS TOO: $name — this is not a backend gap"
        fail=1
    fi
}

# The other direction: a shape that is *not* in the table has to compile, and
# has to agree with the interpreter. A gap list is only honest if the things
# left off it work.
expect_compiled() {
    local name="$1" src="$2"
    printf '%s\n' "$src" >"$WORK/p.gr"
    rm -f "$WORK/out"
    if ! "$GRASM" build "$WORK/p.gr" --native -o "$WORK/out" >"$WORK/build.log" 2>&1; then
        echo "  REFUSED: $name"
        sed 's/^/    /' "$WORK/build.log"
        fail=1
        return
    fi
    local want got
    want=$("$GRASM" run "$WORK/p.gr" 2>&1)
    got=$("$WORK/out" 2>&1)
    if [ "$want" != "$got" ]; then
        echo "  DISAGREE: $name"
        echo "    interp: $want"
        echo "    native: $got"
        fail=1
        return
    fi
    echo "  compiles: $name"
}

echo "checking the documented --native gaps ..."

expect_refused "a generic type declaration" 'type Pair[A, B] = { fst: A, snd: B }
fn main() {
    print("{Pair{ fst: 1, snd: 2 }.fst}")
}'

expect_refused "an effect handler as a return type" 'effect Clock {
    fn now() -> i64
}
fn mk() -> ClockHandler {
    ClockHandler{ now: || 1 }
}
fn main() {
    with Clock handled_by mk() {
        print("{Clock.now()}")
    }
}'

expect_refused "a range held as a value" 'fn main() {
    let r = 1..5
    var t = 0
    for i in r {
        t = t + i
    }
    print("{t}")
}'

expect_refused "par_map with a closure bound to a name" 'fn main() {
    let d = |n| n * 2
    print("{[1, 2].par_map(d)}")
}'

expect_refused "map with a function that returns nothing" 'fn main() {
    let r = ["a"].map(|s| print(s))
    print("{r.len()}")
}'

expect_refused "a type parameter only in a function result" 'fn key_by[T, K](xs: [T], f: fn(T) -> K) -> [K] {
    xs.map(f)
}
fn main() {
    print("{key_by([1, 2], |n| n * 2)}")
}'

# An annotated empty literal compiles — the gap is only the case where there
# is genuinely nothing to infer from. Both halves are checked, because the
# refusal used to cover both and told the reader to add an annotation that was
# already there.
expect_refused "an empty list literal with no type in view" 'fn main() {
    var xs = []
    xs = [1, 2]
    print("{xs}")
}'

expect_compiled "an annotated empty list, however deeply nested" 'fn main() {
    let a: [i64] = []
    let b: [[i64]] = [[]]
    let c: [[[str]]] = [[[]]]
    let d: [Map[str, i64]] = [{}]
    print("{a} {b} {c} {d}")
}'

echo
if [ $fail -ne 0 ]; then
    echo "the documented gap list is out of date"
    exit 1
fi
echo "the gap list matches the compiler"
