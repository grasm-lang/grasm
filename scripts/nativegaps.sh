#!/usr/bin/env bash
# Checks the `--native` gap list in docs/tutorial.md against the compiler.
#
# The list is a promise about what a compiled build refuses. A promise nobody
# checks is a promise that rots: one of these starting to compile would leave
# the documentation telling a reader not to try something that works.
set -uo pipefail

cd "$(dirname "$0")/.."
# Overridable so a gate can be run against a debug build, where Cranelift
# keeps its own assertions on and the runtime checks integer overflow.
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
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

expect_refused "a with block inside parallel or spawn" 'effect Clock {
    fn now() -> i64
}
fn main() {
    parallel {
        spawn {
            with Clock handled_by ClockHandler{ now: || 7 } {
                print("{Clock.now()}")
            }
        }
    }
}'

expect_compiled "exit as a match arm value" 'fn pick(n: i64) -> i64 {
    match n {
        0 -> exit(4)
        _ -> n
    }
}
fn main() {
    print("{pick(1)}")
}'

expect_refused "a handler used as a value" 'effect Log {
    fn info(m: str)
}
fn main() {
    let h = LogHandler{ info: |m| print("L: {m}") }
    with Log handled_by h {
        Log.info("one")
    }
    print("{h}")
}'

expect_refused "a generic instantiated at a handler type" 'effect Clock {
    fn now() -> i64
}
fn idg[T](x: T) -> i64 {
    1
}
fn main() {
    let h = ClockHandler{ now: || 1 }
    print("{idg(h)}")
}'

expect_refused "an effect handler as a struct field type" 'effect Cfg {
    fn get() -> i64
}
type Box = { h: CfgHandler }
fn main() {
    let b = Box{ h: CfgHandler{ get: || 5 } }
    with Cfg handled_by b.h {
        print("{Cfg.get()}")
    }
}'

# Was a gap, and the reason it was one is worth keeping: `IoError` is in
# nobody's source, so nothing registered it in the backend's type table, and
# every program that wanted to tell NotFound from IoFailed was refused. The
# backend now reads the variant list from the checker's own table.
expect_compiles "matching the prelude IoError's variants" 'fn main() {
    match read_file("/no/such/file") {
        Ok(s) -> print("{s.len()}")
        Err(e) -> match e {
            NotFound(p) -> print("nf {p}")
            _ -> print("other")
        }
    }
}'

# Was a gap, and a smaller one than it looked. Effect *operation* calls were
# already fully dynamic — `grasm_handler_lookup` by effect and operation name,
# then an indirect call — so nothing about dispatch had to change. What was
# missing was a way to carry a handler: the operations are lifted from a
# literal at compile time, and a `with` on a parameter has no literal to lift.
# A handler value is now a block of (code, env) pairs built in the frame that
# wrote the literal, which is exactly why it may be passed down and still may
# not be returned or stored.
expect_compiles "an effect handler as a parameter type" 'effect Log {
    fn info(m: str)
}
fn run(h: LogHandler) / {io} {
    with Log handled_by h {
        Log.info("x")
    }
}
fn main() {
    run(LogHandler{ info: |m| print("{m}") })
}'

# Not a shape anyone writes on purpose, but the build used to run until it was
# killed — no diagnostic, unbounded memory — where every other gap here is a
# refusal by name at build time.
# Was a gap: the probe called an effect a "receiver whose type is not known".
expect_compiled "a match whose arms are effect operations" 'effect Log {
    fn info(m: str)
}
fn main() {
    with Log handled_by LogHandler{ info: |m| print(m) } {
        let s = 1
        match s {
            1 -> Log.info("a")
            _ -> Log.info("b")
        }
    }
}'

# Was a gap, and the fix was to stop reconstructing an answer the checker had
# already written down. A closure whose parameters nothing local fixes learns
# them from its call site, inference reaches both, and the backend was rebuilding
# that from the shapes visible at the closure — which cannot reach a call site in
# another statement. It asks now, and the reconstruction stays as the fallback
# for closures inference genuinely leaves unresolved.
expect_compiles "a closure whose parameter types nothing fixes" 'fn main() {
    let f = |p| p.0 + p.1
    for (i, v) in [10, 20].enumerate() {
        print("{f((i, v))}")
    }
}'

# Was a gap: a `for` variable and a binding declared after the closure are
# both typed from what the block already says.
expect_compiled "a closure called with a for variable" 'fn main() {
    let f = |x| x * 2
    for v in [1, 2] {
        print("{f(v)}")
    }
}'

expect_compiled "a closure literal in a match arm" 'fn main() {
    let n = 1
    let f: fn(i64) -> i64 = match n {
        1 -> |x| x + 1
        _ -> |x| x * 2
    }
    let g: fn(i64) -> i64 = if n > 0 { |x| x + 10 } else { |x| x - 10 }
    print("{f(10)} {g(10)}")
}'

expect_refused "fail inside a fallible closure passed to map" 'type E = { m: str }
fn main() -> () ! E {
    let ys: [i64 ! E] = [0, 1].map(|x| fail(E{ m: "d" }))
    let v = ys[0] ?? 4
    print("{v}")
}'

# Was a gap, and the cause was asymmetry rather than difficulty: an empty *list*
# asked the checker what it had resolved the literal to and an empty *map* did
# not, though the checker records both ([T-EMPTY-LIT]). So the seed arrived as a
# map with unknown key and value, and the refusal said the seed and the function
# differed when they were written identically.
expect_compiles "a fold seeded with an unannotated map()" 'fn add(acc: Map[str, i64], n: i64) -> Map[str, i64] {
    var a = acc
    a.set("k", n)
    a
}
fn main() {
    let m = [1, 2, 3].fold(map(), add)
    print("{m.len()}")
}'

# The other direction for the two that stopped being gaps: a handler operation
# whose body is a bare `fail`, and a function whose whole body leaves.
expect_compiled "a fallible handler op whose body is a bare fail" 'type E = { m: str }
effect A {
    fn go() -> i64 ! E
}
fn main() {
    with A handled_by AHandler{ go: || fail(E{ m: "x" }) } {
        print("{A.go() ?? 0 - 1}")
    }
}'

# `exit` rather than `panic`, so the comparison is not about the stack trace a
# compiled binary deliberately does not carry ([E-PANIC-TRACE]).
expect_compiled "a body that only leaves" 'fn boom() -> i64 {
    exit(3)
}
fn main() {
    print("{boom()}")
}'

expect_refused "polymorphic recursion" 'fn f[T](x: T, n: i64) -> i64 {
    if n <= 0 {
        0
    } else {
        f([x], n - 1)
    }
}
fn main() {
    print("{f(1, 40)}")
}'

# Was a gap. `for i in 0..n` is compiled into a counted loop and never builds
# a range at all — which is why it costs nothing — so there was no range value
# for a binding to hold. There is one now, and the counted loop is unchanged.
expect_compiles "a range held as a value" 'fn main() {
    let r = 1..5
    var t = 0
    for i in r {
        t = t + i
    }
    print("{t}")
}'

expect_compiled "par_map with a closure bound to a name" 'fn sq(n: i64) -> i64 {
    n * n
}

fn go(xs: [i64], f: fn(i64) -> i64) -> [i64] {
    xs.par_map(f)
}

fn main() {
    let g = sq
    let h = |n| n + 1
    print("{go([1, 2], sq)} {[1, 2].par_map(g)} {[1, 2].par_map(h)}")
}'

# Was a gap, and was never a limitation: `RtPayload::Unit` exists precisely so
# a list of nothings prints as `[(), ()]`. The rest of the refusal was advice —
# write a `for` loop — and advice that makes two engines disagree about which
# programs exist is worse than no advice.
expect_compiles "map with a function that returns nothing" 'fn main() {
    let r = ["a"].map(|s| print(s))
    print("{r.len()}")
}'

# Was a gap. Monomorphization binds a type parameter by matching each declared
# parameter against the argument's actual type, and that matcher had no case for
# a *function* type — so `f: fn(T) -> K` bound `T` from `xs` and left `K`
# determined by nothing. The closure passed for `f` had always known its own
# type; nothing looked inside it. `ref T` and `T ! E` were missing for the same
# reason and are handled now too.
expect_compiles "a type parameter only in a function result" 'fn key_by[T, K](xs: [T], f: fn(T) -> K) -> [K] {
    xs.map(f)
}
fn main() {
    print("{key_by([1, 2], |n| n * 2)}")
}'

# An annotated empty literal compiles — the gap is only the case where there
# is genuinely nothing to infer from. Both halves are checked, because the
# refusal used to cover both and told the reader to add an annotation that was
# already there.
expect_compiled "a match no arm of which has an inferable type" 'fn pick(p: i64) -> Map[str, i64] ! str {
    match p {
        0 -> map()
        _ -> map()
    }
}

fn main() {
    match pick(0) {
        Ok(m) -> print("{m.len()}")
        Err(e) -> print("{e}")
    }
}'

# The half that must keep working: builtins the probe *does* know. Every one of
# these compiled to a silent `0` before, which no refusal would have caught.
# Was a documented gap. `scripts/fuzz_places.py` found it by enumerating the
# place shapes, and the backend now reads the place, calls, and writes back —
# the same read-modify-write the mutating methods already did through a place.
expect_compiled "an inout argument that is not a plain variable" 'type Bag = { items: [i64] }

fn push_one(inout xs: [i64]) {
    xs.push(1)
}

fn main() {
    var b = Bag{ items: [] }
    push_one(b.items)
    print("{b.items}")
}'

expect_compiled "returning a closure" 'fn mk(n: i64) -> fn(i64) -> i64 {
    |x| x + n
}

fn twice(f: fn(i64) -> i64) -> fn(i64) -> i64 {
    |x| f(f(x))
}

fn dbl(n: i64) -> i64 {
    n * 2
}

fn main() {
    let add5 = mk(5)
    let g = twice(dbl)
    print("{add5(3)} {g(3)}")
}'

expect_refused "a handler bound by a for loop" 'effect Src {
    fn get() -> str
}

fn main() {
    let hs = [SrcHandler{ get: || "a" }, SrcHandler{ get: || "b" }]
    for h in hs {
        with Src handled_by h {
            print("{Src.get()}")
        }
    }
}'

# The neighbouring shapes that must keep compiling, so the three rows above
# stay as narrow as they claim to be.
expect_compiled "writing through a ref" 'type Node = { v: i64, kids: [i64] }

fn main() {
    let r = ref(Node{ v: 1, kids: [] })
    r.v = 42
    r.kids.push(7)
    print("{r.v} {r.kids}")
}'

# Through a cycle, which is what proved the first implementation wrong: the
# write has to land in the slab, not in a rebuilt copy stored over it.
expect_compiled "writing through a ref around a cycle" 'type G = { name: str, out: [ref G] }

fn main() {
    let a = ref(G{ name: "A", out: [] })
    let b = ref(G{ name: "B", out: [] })
    a.out.push(b)
    b.out.push(a)
    a.out[0].name = "B!"
    a.out[0].out[0].name = "A!"
    print("{b.name} {a.name}")
}'

expect_refused "a handler op that is a named fn" 'effect Src {
    fn get() -> str
}

fn named() -> str {
    "x"
}

fn main() {
    with Src handled_by SrcHandler{ get: named } {
        print("{Src.get()}")
    }
}'

# Was a gap. A `ref` is a slot and a generation packed into one word, and two
# are equal when they name the same slot at the same generation — so the
# comparison is the word. It compiled for a *list* of refs already, because the
# runtime's deep equality reaches the same answer element by element; only two
# bare ones had nowhere to go.
expect_compiles "== on two bare refs" 'type N = { v: i64 }

fn main() {
    let a = ref(N{ v: 1 })
    let b = ref(N{ v: 1 })
    print("{a == b}")
}'

# Was a gap. The callee is evaluated once, bound to a name the source cannot
# spell, and called by the path a named function value already used — binding it
# rather than calling in place because that path reads the value twice, once for
# the code pointer and once for the environment, and evaluating the callee twice
# would run its side effects twice.
expect_compiles "a call through a non-name callee" 'fn double(n: i64) -> i64 {
    n * 2
}

let TABLE: [fn(i64) -> i64] = [double]

fn main() {
    print("{TABLE[0](21)}")
}'

expect_refused "a UFCS call of a generic function" 'fn describe[T](xs: [T]) -> str {
    "n={xs.len()}"
}

fn main() {
    print("{[1, 2].describe()}")
}'

# The neighbouring shape that must keep compiling, so the row above stays as
# narrow as it claims: a struct *containing* a ref compares fine.
expect_compiled "== on a struct containing a ref" 'type N = { v: i64 }
type E = { to: ref N, w: i64 }

fn main() {
    let r = ref(N{ v: 1 })
    let a = E{ to: r, w: 1 }
    print("{a == a}")
}'

expect_compiled "an inout argument that is a plain variable" 'fn push_one(inout xs: [i64]) {
    xs.push(1)
}

fn main() {
    var xs: [i64] = []
    push_one(xs)
    print("{xs}")
}'

expect_compiled "a let-bound handler" 'effect Src {
    fn get() -> str
}

fn main() {
    let h = SrcHandler{ get: || "a" }
    with Src handled_by h {
        print("{Src.get()}")
    }
}'

expect_compiled "a match whose arms are builtin calls" 'fn constant(p: i64) -> f64 ! str {
    match p {
        0 -> e()
        _ -> pi()
    }
}

fn listing(p: i64) -> [str] ! str {
    match p {
        0 -> args()
        _ -> args()
    }
}

fn main() {
    match constant(0) {
        Ok(v) -> print("{v}")
        Err(e) -> print("{e}")
    }
    match listing(0) {
        Ok(v) -> print("{v.len()}")
        Err(e) -> print("{e}")
    }
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
