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
    ran=$((ran + 1))
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
    local want got want_code got_code
    want=$("$GRASM" run "$WORK/p.gr" 2>&1); want_code=$?
    got=$("$WORK/out" 2>&1); got_code=$?
    if [ "$want" != "$got" ]; then
        echo "  DISAGREE: $name"
        echo "    interp: $want"
        echo "    native: $got"
        fail=1
        return
    fi
    # The exit code, not only the output. A program can answer correctly and
    # then die: a named function used as a handler operation printed the right
    # value and took SIGBUS on the way out (F39), because it released an
    # environment that was a stack address. Comparing stdout alone called that
    # agreement.
    if [ "$want_code" != "$got_code" ]; then
        echo "  DISAGREE ON EXIT: $name — interp $want_code, native $got_code"
        echo "    output matched, so this is a fault after the answer"
        fail=1
        return
    fi
    # And it must not leak. These programs are not in the corpus, so
    # scripts/leakcheck.sh never sees them — every gap closed here would
    # otherwise be checked for agreement and not for whether it releases what
    # it allocates.
    local leaks
    leaks=$(GRASM_LEAK_CHECK=1 "$WORK/out" 2>&1 | grep -c "^leak:")
    if [ "$leaks" != "0" ]; then
        echo "  LEAKS: $name"
        GRASM_LEAK_CHECK=1 "$WORK/out" 2>&1 | grep "^leak:" | sed 's/^/    /'
        fail=1
        return
    fi
    ran=$((ran + 1))
    echo "  compiles: $name"
}

# Every case must actually run.
#
# `expect_compiles` is not a function — the function is `expect_compiled` — and
# eleven cases were written with the wrong name. Bash reported each as a command
# not found on stderr, nothing set `fail`, and this script printed "the gap list
# matches the compiler" while a quarter of it did nothing. That is the same
# defect as F36 in a different place: a gate that reports a number it did not
# check.
#
# So the count is checked against the source. A case that does not run is a
# failure, whatever the reason.
# Any `expect_` line counts as declared, whatever it is spelt. A misspelt one
# is then declared and never runs, and the totals disagree — which is the case
# F40 was, and which a regex naming only the two real helpers cannot see.
declared=$(grep -cE '^expect_[a-z_]* "' "$0")
ran=0
echo "checking the documented --native gaps ..."

# Was a gap, for structs. A generic type is kept whole until a use site fixes
# its parameters — the same treatment a generic function gets — and the member
# is named for its arguments. That name is how two members are told apart and is
# not what the type is called: it stops at the value's edge, so `print` shows
# `Pair{..}` and not `Pair[i64,str]{..}`.
expect_compiled "a generic type declaration" 'type Pair[A, B] = { fst: A, snd: B }
fn main() {
    print("{Pair{ fst: 1, snd: 2 }.fst}")
}'

expect_compiled "an effect handler as a return type" 'effect Clock {
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

expect_compiled "a with block inside parallel or spawn" 'effect Clock {
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

expect_compiled "a handler used as a value" 'effect Log {
    fn info(m: str)
}
fn main() {
    let h = LogHandler{ info: |m| print("L: {m}") }
    with Log handled_by h {
        Log.info("one")
    }
    print("{h}")
}'

expect_compiled "a type parameter the binding fixes" 'type Box[V] = { items: [V] }
fn empty_box[V]() -> Box[V] {
    let b: Box[V] = Box{ items: [] }
    b
}
fn add[V](b: Box[V], v: V) -> Box[V] {
    var xs = b.items
    xs.push(v)
    let out: Box[V] = Box{ items: xs }
    out
}
fn main() {
    let a: Box[str] = empty_box()
    var n: Box[i64] = empty_box()
    n = add(n, 7)
    print("{a.items.len()} {n.items}")
}'

expect_compiled "a handler a call answers, in an argument" 'effect Cfg {
    fn value() -> i64
}
fn from_ref(slot: ref i64) -> CfgHandler {
    CfgHandler{ value: || slot.value }
}
fn wrap(a: CfgHandler) -> CfgHandler {
    CfgHandler{ value: || {
        var n = 0
        with Cfg handled_by a {
            n = Cfg.value()
        }
        n * 2
    } }
}
fn main() {
    let x = ref(21)
    with Cfg handled_by wrap(from_ref(x)) {
        print("{Cfg.value()}")
    }
}'

expect_compiled "a match whose live arms all yield nothing" 'fn go(o: Option[i64], word: str) -> str {
    var m = map()
    match o {
        None -> return "none"
        Some(v) -> {
            match word {
                "a" -> m.set("k", true)
                _ -> m.set("k", false)
            }
        }
    }
    "{m.get("k") ?? false}"
}
fn main() {
    print("{go(Some(1), "a")} {go(Some(1), "z")} {go(None, "a")}")
}'

expect_compiled "a generic instantiated at a handler type" 'effect Clock {
    fn now() -> i64
}
fn idg[T](x: T) -> i64 {
    1
}
fn main() {
    let h = ClockHandler{ now: || 1 }
    print("{idg(h)}")
}'

expect_compiled "an effect handler as a struct field type" 'effect Cfg {
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
expect_compiled "matching the prelude IoError's variants" 'fn main() {
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
expect_compiled "an effect handler as a parameter type" 'effect Log {
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
expect_compiled "a closure whose parameter types nothing fixes" 'fn main() {
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

# Was a gap. The closure is inlined into the loop, so its `fail` was lexically
# inside the enclosing function and lowered as a return from it — `[0, 1].map(|x|
# fail(e))` held as `[i64 ! E]` reported an unhandled error from `main` where the
# interpreter produced a list of `Err`s. A `fail` there now goes to a block that
# pushes the `Err` as the element and lets the loop carry on, releasing what the
# closure body bound and nothing the loop still owns.
expect_compiled "fail inside a fallible closure passed to map" 'type E = { m: str }
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
expect_compiled "a fold seeded with an unannotated map()" 'fn add(acc: Map[str, i64], n: i64) -> Map[str, i64] {
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
expect_compiled "a range held as a value" 'fn main() {
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
expect_compiled "map with a function that returns nothing" 'fn main() {
    let r = ["a"].map(|s| print(s))
    print("{r.len()}")
}'

# Was a gap. Monomorphization binds a type parameter by matching each declared
# parameter against the argument's actual type, and that matcher had no case for
# a *function* type — so `f: fn(T) -> K` bound `T` from `xs` and left `K`
# determined by nothing. The closure passed for `f` had always known its own
# type; nothing looked inside it. `ref T` and `T ! E` were missing for the same
# reason and are handled now too.
expect_compiled "a type parameter only in a function result" 'fn key_by[T, K](xs: [T], f: fn(T) -> K) -> [K] {
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

# A handler value is a block of (code, env) pairs in the frame that wrote the
# literal, so it may be passed *down* and must not escape. Returning one is the
# shape that only exists because handler values do, and it is a refusal the
# safety of the passing case depends on — not a gap waiting to be closed. If it
# starts compiling because handler environments moved to the heap, this is where
# that has to be argued for rather than noticed.
expect_compiled "returning a handler parameter" 'effect S {
    fn a() -> i64
}
fn pass(h: SHandler) -> SHandler {
    h
}
fn main() {
    with S handled_by pass(SHandler{ a: || 1 }) {
        print("{S.a()}")
    }
}'

# Compiles, and is sound for the same reason passing is: `g` is a copy of a
# pointer into the caller's frame, which outlives this one.
expect_compiled "binding a handler parameter to a local" 'effect S {
    fn a() -> i64
}
fn take(h: SHandler) / {io, S} {
    let g = h
    with S handled_by g {
        print("{S.a()}")
    }
}
fn main() {
    take(SHandler{ a: || 1 })
}'

expect_compiled "a handler bound by a for loop" 'effect Src {
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

# Was a gap. A named function used as an operation needs only the wrapper that
# gives it the `(env, params…)` shape every function value has — with one
# difference that is not optional: a function value owns its environment and
# releases it, while a handler operation is handed the address of a stack slot
# in the frame that installed it. Sharing the wrapper decremented through a live
# frame, which returned the right answer and then crashed.
expect_compiled "a handler op that is a named fn" 'effect Src {
    fn get() -> str
    fn tag() -> str
}

fn named() -> str {
    "x"
}

fn other() -> str {
    "y"
}

fn main() {
    with Src handled_by SrcHandler{ get: named, tag: other } {
        print("{Src.get()}{Src.tag()}")
    }
}'

# Was a gap. A `ref` is a slot and a generation packed into one word, and two
# are equal when they name the same slot at the same generation — so the
# comparison is the word. It compiled for a *list* of refs already, because the
# runtime's deep equality reaches the same answer element by element; only two
# bare ones had nowhere to go.
expect_compiled "== on two bare refs" 'type N = { v: i64 }

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
expect_compiled "a call through a non-name callee" 'fn double(n: i64) -> i64 {
    n * 2
}

let TABLE: [fn(i64) -> i64] = [double]

fn main() {
    print("{TABLE[0](21)}")
}'

# Was a gap. `lookup_fn` does not carry generics, so a generic free function
# called UFCS-style had no symbol and the call was refused. It is instantiated
# from the receiver and the arguments now — the receiver is the first parameter
# ([L-UFCS]), which is where the type parameters come from.
expect_compiled "a UFCS call of a generic function" 'fn describe[T](xs: [T]) -> str {
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

# The shape that made the refusal right in the first place: a generic free
# function with the same name as a function-typed field. The checker resolves it
# to the function, and this must too — calling the field printed a heap address
# and multiplied it by two, at exit 0 with nothing on stderr. Run and compared
# here, which a compile-only test cannot do.
expect_compiled "a generic free function crossed with a field of the same name" 'type Box = { tag: fn() -> str }

fn tag[T](x: T) -> i64 {
    77
}

fn main() {
    let b = Box{ tag: || "a string" }
    print("{b.tag()}")
}'

# And the other direction: a *non-generic* free function of the same name must
# not win, because the checker resolves that one to the field.
expect_compiled "a field of the same name as a non-generic free function" 'type Box = { tag: fn() -> str }

fn tag(xs: [i64]) -> i64 {
    9
}

fn main() {
    let b = Box{ tag: || "f" }
    print("{b.tag()}")
}'


# A constructor fixes the sum's parameters from its arguments, the way a struct
# literal fixes a struct's from its fields. Two instantiations in one program is
# the case that has to work and the one that broke first: the by-name variant
# index holds one parent per name, so `Has("text")` found the `Opt[i64]` that
# `Has(5)` left there and printed a pointer as an integer. Which member a
# constructor names is a question for the use site, and generic instantiations
# are not in that index at all now.
expect_compiled "a variant of a generic sum type" 'type Opt[T] = | Has(v: T) | Nothing

fn main() {
    let o = Has(5)
    match o {
        Has(v) -> print("{v}")
        Nothing -> print("none")
    }
}'


# Two members of one family in one program, which is what the by-name index
# could not represent.
expect_compiled "two instantiations of one generic sum" 'type Opt[T] = | Has(v: T) | Nothing

fn main() {
    let a = Has(5)
    let b = Has("text")
    match a {
        Has(v) -> print("{v}")
        Nothing -> print("none")
    }
    match b {
        Has(v) -> print("{v}")
        Nothing -> print("none")
    }
}'

# `Good(1)` for `Res[T, E]` is no longer here: the *checker* refuses it on every
# engine now ([T-GENERIC-TYPE]), so it is a language rule and not a backend gap.
# Its test lives in grasm_sema, as `a_constructor_may_not_leave_a_parameter_open`.



# The conditional shape, which has to get both paths right: an element from the
# ok branch and an element from the failing one, in one list.
expect_compiled "a map closure that fails for some elements only" 'type E = { m: str }

fn main() -> () ! E {
    let ys: [i64 ! E] = [1, 2, 3].map(|x| if x == 2 { fail(E{ m: "two" }) } else { Ok(x * 10) })
    var i = 0
    while i < ys.len() {
        print("{ys[i] ?? -1}")
        i = i + 1
    }
}'

# A `fold` whose accumulator is fallible: the failure becomes the accumulator
# and the next round receives it. This disagreed — the interpreter finished and
# printed the fallback, the compiled build reported an unhandled error from
# `main` — and `eval_fold` has its own path that never had the guard the map
# loop had (F43).
expect_compiled "fail inside a fallible fold closure" 'type E = { m: str }

fn main() {
    let seed: i64 ! E = Ok(0)
    let t: i64 ! E = [1, 2].fold(seed, |a, x| if x == 2 { fail(E{ m: "two" }) } else { a })
    print("{t ?? -1}")
}'

# `return` of an `if` whose branches both leave. The `if` has no value, and the
# form that asked for one invented a unit -- an `i8` zero -- which the fallible
# wrapping then handed to `grasm_box_f64`. Cranelift rejected the function and
# the backend reported it against itself as a codegen bug (F46).
expect_compiled "return of an if where every branch fails" 'type E = | Bad

fn f(n: i64) -> f64 ! E {
    return if n == 0 {
        fail(Bad)
    } else {
        fail(Bad)
    }
}

fn main() {
    print("{f(1) ?? 0.0}")
}'

# The same shape without the `return`. This one always compiled, which is what
# made the refusal look like a property of `fail` rather than of `return`.
expect_compiled "an if where every branch fails, in tail position" 'type E = | Bad

fn f(n: i64) -> f64 ! E {
    if n == 0 {
        fail(Bad)
    } else {
        fail(Bad)
    }
}

fn main() {
    print("{f(1) ?? 0.0}")
}'

# A closure declared inside another closure. The outer one cannot be typed
# without typing its body, its body calls the inner one, and a call to a name
# bound to a *function value* was not something the probe could type -- only
# declared functions were consulted. So every nesting was refused, capture or
# no capture, annotated or not (F51).
expect_compiled "a closure declared inside a closure" 'fn main() {
    let held = 1
    let outer = || {
        let inner = || held
        inner()
    }
    print("{outer()}")
}'

# The same nesting with nothing captured, which refused for the same reason
# and made the refusal look like it was about captures.
expect_compiled "a nested closure that captures nothing" 'fn main() {
    let outer = || {
        let inner = || 5
        inner()
    }
    print("{outer()}")
}'

# A closure returning a closure, twice over.
expect_compiled "a closure returning a closure" 'fn make() -> fn() -> fn() -> i64 {
    let held = 3
    || || held
}

fn main() {
    let f = make()
    let g = f()
    print("{g()} {g()}")
}'

# An empty `map()` as an element of a list literal. It has nothing to read a
# type off, so it takes one from the annotation the checker already resolved --
# which the lowering asks for and the probe did not. The same list built with
# `push` compiled, and `[[]]` in the same position compiled, so the asymmetry
# was between the two empty literals rather than the two spellings (F52).
expect_compiled "an empty map in a list literal" 'fn main() {
    let xs: [Map[str, i64]] = [map()]
    print("{xs.len()}")
}'

expect_compiled "an empty map in a nested list literal" 'fn main() {
    let xs: [[Map[str, i64]]] = [[map()]]
    print("{xs.len()}")
}'

# A range reaching a method from a shape that is not a name. The test for
# "does this hold a range" recognised a bare `Ident` and nothing else, which
# was every shape a range took when it was written (F53).
expect_compiled "a range that is a list element" 'fn main() {
    let xs = [0..3]
    print("{xs[0].len()} {xs[0].sum()}")
}'

expect_compiled "a range returned by a closure" 'fn main() {
    let f = || 2..6
    print("{f().len()} {f().map(|v| v * 2)}")
}'

expect_compiled "a range declared by name" 'type W = { span: range }

fn window(n: i64) -> range {
    0..n
}

fn total(r: range) -> i64 {
    r.sum()
}

fn main() {
    let w = W{ span: window(4) }
    print("{w.span} {total(w.span)}")
}'

# `return` out of a `with` block. The `with` reported itself an ordinary
# statement whatever its body did, so the statement after it was written into
# the block the return had already closed -- a verifier error the backend
# reported against itself (F60).
expect_compiled "a return out of a with block" 'effect Src {
    fn get() -> i64
}

fn leaves() -> i64 / {Src} {
    with Src handled_by SrcHandler{ get: || 7 } {
        return 1
    }
    0
}

fn main() {
    print("{leaves()}")
}'

# The same through a handler *value*, which takes the other path: there the
# scope holding the handler was released after the block had been closed.
expect_compiled "a return out of a with, handler by value" 'effect Src {
    fn get() -> i64
}

fn leaves(h: SrcHandler) -> i64 / {Src} {
    with Src handled_by h {
        return 1
    }
    0
}

fn main() {
    print("{leaves(SrcHandler{ get: || 7 })}")
}'

if [ "$ran" != "$declared" ]; then
    echo
    echo "only $ran of $declared cases ran — the rest were never reached,"
    echo "so this run checked less than it looks like it did."
    exit 1
fi
echo "the gap list matches the compiler"
