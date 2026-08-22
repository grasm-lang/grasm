# Grasm — conformance programs

One executable program per normative clause of `grasm-language-pack.md`. Each
header states the tag it demonstrates and the exact expected behaviour, and
every one of them is run by `cargo test -p grasm_spec`, so a program here
cannot describe behaviour the implementation does not have.

This is the companion to the definition rather than part of it. Read the
definition to write Grasm; read this when a rule is in dispute and you want to
see it settled by a program.

## `E-ARGS.gr`

```grasm
//! tag: E-ARGS
//! expect: output
//! 0
fn main() {
    // No arguments were passed after `--`.
    print("{args().len()}")
}
```

## `E-ARITH-FLOAT.gr`

```grasm
//! tag: E-ARITH-FLOAT
//! expect: output
//! 0.30000000000000004
//! inf
fn main() {
    print("{0.1 + 0.2}")
    print("{1.0 / 0.0}")
}
```

## `E-ARITH-INT.gr`

```grasm
//! tag: E-ARITH-INT
//! expect: output
//! 14
//! -9223372036854775808
//! -9223372036854775808
//! 0
//! 7 0 -7
fn main() {
    print("{2 + 3 * 4}")
    // Wrapping on overflow, two's complement.
    var max = 9223372036854775807
    print("{max + 1}")

    // `i64::MIN / -1` has no i64 result, so it wraps like any other overflow
    // rather than trapping. The hardware divide instruction does trap on it,
    // which is why this is worth a program: the native backend has to avoid
    // handing the CPU that operand pair.
    var min = -9223372036854775807
    min = min - 1
    print("{min / -1}")
    print("{min % -1}")

    print("{-7 / -1} {-7 % -1} {7 / -1}")
}
```

## `E-BENCH-WARMUP.gr`

```grasm
//! tag: E-BENCH-WARMUP
//! expect: output
//! 4
// A `bench` declaration is checked like any other body but is only executed by
// `grasm bench`, so running this program prints only what `main` prints.
bench "doubling" {
    let x = 2 * 2
    assert(x == 4)
}

fn main() {
    print("{2 * 2}")
}
```

## `E-BORROWED-READ.gr`

```grasm
//! tag: E-BORROWED-READ
//! expect: output
//! Hello World|15|[Hello, World]|15
//! [2, 1, 3] 3 false
//! [a, b] [1, 2] [(a, 1), (b, 2)] 2
//! 15 3 2
//! 2000 40000 1 20
// A read-only method on a plain variable does not touch the value's
// refcount: the variable already holds a reference for the whole call, so
// the usual +1/-1 around it is traffic and nothing else.
//
// This program exists to prove the value survives it. Every receiver is read
// several times and then used again, and the last block builds and keeps two
// thousand lists while reading them, which is where an early free would show.
fn main() {
    let s = "  Hello World  "
    let xs = [3, 1, 2]
    var m: Map[str, i64] = {"a": 1, "b": 2}
    print("{s.trim()}|{s.to_upper().len()}|{s.words()}|{s.chars().len()}")
    print("{xs.rev()} {xs.len()} {xs.is_empty()}")
    print("{m.keys()} {m.values()} {m.entries()} {m.len()}")
    // Still usable after every one of those.
    print("{s.len()} {xs.len()} {m.len()}")

    var kept: [[i64]] = []
    var i = 0
    while i < 2000 {
        var ys: [i64] = []
        var j = 0
        while j < 20 {
            ys.push(j)
            j = j + 1
        }
        if ys.len() > 0 and not ys.is_empty() {
            kept.push(ys)
        }
        i = i + 1
    }
    var total = 0
    for k in kept {
        total = total + k.len()
    }
    var held: Map[str, [i64]] = {}
    held.set("a", kept.get(0) ?? [])
    print("{kept.len()} {total} {held.len()} {(held.get("a") ?? []).len()}")
}
```

## `E-CLOSURE-CAPTURE.gr`

```grasm
//! tag: E-CLOSURE-CAPTURE
//! expect: output
//! 2 one
//! 2 one
//! 3 two
// A closure captures values, at the moment it is created.
//
// A handler's operations are closures, so a handler cannot hold mutable
// state: what it captured is what it keeps, however the binding moves on.
// The compiled backend stored the pointer without holding a reference to it,
// so the binding looked unshared, copy-on-write never fired, and the handler
// saw every later mutation — a wrong answer, and a crash when the binding was
// reassigned and freed what the handler still pointed at.
effect Snap {
    fn size() -> i64
    fn text() -> str
}

fn main() {
    var xs = [1, 2]
    var s = "one"
    with Snap handled_by SnapHandler{ size: || xs.len(), text: || s } {
        print("{Snap.size()} {Snap.text()}")
        xs.push(3)
        s = "two"
        print("{Snap.size()} {Snap.text()}")
    }
    print("{xs.len()} {s}")
}
```

## `E-COALESCE-DIVERGE.gr`

```grasm
//! tag: E-COALESCE-DIVERGE
//! expect: output
//! 1 0
//! 10 -1
//! 7
// `?? fail(e)` — absence turned into a failure, which is the shape most
// fallible function bodies take.
type E = { m: str }

fn need(m: Map[str, i64], k: str) -> i64 ! E {
    m.get(k) ?? fail(E{ m: "no {k}" })
}

fn doubled(o: Option[i64]) -> i64 ! E {
    let v = o ?? fail(E{ m: "none" })
    v * 2
}

fn main() {
    let m = {"a": 1}
    print("{need(m, "a") ?? 0} {need(m, "b") ?? 0}")
    print("{doubled(Some(5)) ?? 0 - 1} {doubled(None) ?? 0 - 1}")
    // A fallback that does produce a value still does.
    print("{m.get("zz") ?? 7}")
}
```

## `E-COALESCE-LAZY.gr`

```grasm
//! tag: E-COALESCE-LAZY
//! expect: output
//! evaluated 1
//! evaluated 2
//! 2
//! 3
//! 9
// The right side runs only when it is needed, and defaults chain.
//
// `??` groups to the right, so each default may itself be absent. Grouping
// left made the middle operand have to be a plain value, and
// `first() ?? second() ?? fallback` did not compile.
type E = | Bad

fn loud(n: i64) -> Option[i64] {
    print("evaluated {n}")
    if n == 2 {
        Some(n)
    } else {
        None
    }
}

fn fallible(b: bool) -> i64 ! E {
    if b {
        Ok(9)
    } else {
        Err(Bad)
    }
}

fn main() {
    // `loud(3)` is never reached, and neither is the final `0`.
    print("{loud(1) ?? loud(2) ?? loud(3) ?? 0}")
    var m: Map[str, i64] = {}
    m.set("c", 3)
    print("{m.get("a") ?? m.get("b") ?? m.get("c") ?? 0}")
    print("{fallible(false) ?? fallible(true) ?? 0}")
}
```

## `E-CONST-REF.gr`

```grasm
//! tag: E-CONST-REF
//! expect: error
//! holds a reference
// A constant may not hold a reference, however it got one.
//
// A constant is evaluated again at every mention, so a reference in one is a
// fresh slot each time: two mentions on one line print two different
// references, `release` gives up a slot nothing else names, and a loop leaks
// one per iteration. The check looked for a literal `ref(...)` in the
// initializer, so a helper returning one walked straight past it — and so
// would a struct or a list holding one.
type Node = { id: i64 }

fn mk() -> ref Node {
    ref(Node{ id: 1 })
}

let ROOT = mk()

fn main() {
    print("{ROOT} {ROOT}")
}
```

## `E-CONST.gr`

```grasm
//! tag: E-CONST
//! expect: output
//! Sales report is 77 wide, tab 9
//! #####
//! [10, 20, 30] 3
//! 77 154
//! 4 6
// A module-level `let` is a constant. Without them every layout constant had
// to be a zero-argument function, which is what a program ends up full of.
let TABLE_WIDTH = 77
let TAB = 9
let TITLE: str = "Sales report"
let LIMITS = [10, 20, 30]
/// A constant may be computed from another declared before it.
let DOUBLE_WIDTH = TABLE_WIDTH * 2
/// And from a function, or naming one as a value — a dispatch table is the
/// shape this is for.
let TWICE_TWO = two() * 2
let AGGS: Map[str, fn([i64]) -> i64] = {"sum": total, "count": howmany}

fn two() -> i64 {
    2
}

fn total(xs: [i64]) -> i64 {
    xs.sum()
}

fn howmany(xs: [i64]) -> i64 {
    xs.len()
}

fn bar(n: i64) -> str {
    "#".repeat(n.min(TABLE_WIDTH))
}

fn main() {
    print("{TITLE} is {TABLE_WIDTH} wide, tab {TAB}")
    print("{bar(5)}")
    print("{LIMITS} {LIMITS.len()}")
    print("{TABLE_WIDTH} {DOUBLE_WIDTH}")
    let f = AGGS.get("sum") ?? howmany
    print("{TWICE_TWO} {f([1, 2, 3])}")
}
```

## `E-COW.gr`

```grasm
//! tag: E-COW
//! expect: output
//! 3 4
fn main() {
    // Copy-on-write is never observable: the copy is independent.
    var xs = [1, 2, 3]
    var ys = xs
    ys.push(4)
    print("{xs.len()} {ys.len()}")
}
```

## `E-DIV-ZERO.gr`

```grasm
//! tag: E-DIV-ZERO
//! expect: panic
//! division by zero
fn main() {
    let a = 1
    let b = 0
    print("{a / b}")
}
```

## `E-EFFECT-NO-INOUT.gr`

```grasm
//! tag: E-EFFECT-NO-INOUT
//! expect: error
//! cannot take an `inout` parameter
// An effect operation cannot take `inout`.
//
// A handler runs through a function value, so the write landed in a copy the
// caller never saw: the handler reported success, the caller read back an
// empty list, and every engine agreed on the wrong answer — which is the one
// shape no differential test can catch. It typechecked, so the only way to
// learn this was to run it and disbelieve the result.
effect Audit {
    fn record(inout sink: [str], entry: str)
}

fn main() {
    print("unreachable")
}
```

## `E-ENTRY.gr`

```grasm
//! tag: E-ENTRY
//! expect: output
//! started
fn main() {
    print("started")
}
```

## `E-EQ-STRUCTURAL.gr`

```grasm
//! tag: E-EQ-STRUCTURAL
//! expect: output
//! true true false
type P = { x: i64, y: [str] }
fn main() {
    let a = P{ x: 1, y: ["k"] }
    let b = P{ x: 1, y: ["k"] }
    let c = P{ x: 2, y: ["k"] }
    print("{a == b} {[1, 2] == [1, 2]} {a == c}")
}
```

## `E-ERR-PROPAGATE.gr`

```grasm
//! tag: E-ERR-PROPAGATE
//! expect: output
//! caught Bad
//! skipped rest
type E = | Bad
fn failing() -> i64 ! E {
    fail(Bad)
}
fn caller(inout reached: bool) -> i64 ! E {
    let v = try failing()
    reached = true
    v
}
fn main() {
    var reached = false
    match caller(reached) {
        Ok(v) -> print("ok {v}")
        Err(e) -> print("caught {e}")
    }
    if not reached {
        print("skipped rest")
    }
}
```

## `E-HANDLER-INNERMOST.gr`

```grasm
//! tag: E-HANDLER-INNERMOST
//! expect: output
//! 1
//! 2
//! 1
effect Clock {
    fn now() -> i64
}
fn main() {
    with Clock handled_by ClockHandler{ now: || 1 } {
        print("{Clock.now()}")
        with Clock handled_by ClockHandler{ now: || 2 } {
            print("{Clock.now()}")
        }
        print("{Clock.now()}")
    }
}
```

## `E-HANDLER-MISSING.gr`

```grasm
//! tag: E-HANDLER-MISSING
//! expect: error
//! it was declared outside the `with` block
// A handled effect cannot leave the block that handles it.
//
// The closure's `Cfg` is discharged where it is written, because that is
// where the handler is. Storing it in `f`, which outlives the block, carried
// the discharge along: `f()` afterwards found no handler and reached the
// runtime panic this clause says well-typed programs never do.
effect Cfg {
    fn value() -> i64
}

fn main() {
    var f: fn() -> i64 / {Cfg} = || 0
    with Cfg handled_by CfgHandler{ value: || 1 } {
        f = || Cfg.value()
    }
    print("{f()}")
}
```

## `E-HANDLER-OPS.gr`

```grasm
//! tag: E-HANDLER-OPS
//! expect: error
//! is an operation of a handler
// A handler's operations are called by performing the effect.
//
// They are the one function value with no row to check — that exemption is
// what lets `LogHandler{ info: |m| print(m) }` be written at all. Reading one
// back out turned the exemption into a laundry: this `quiet` declares the
// empty row and prints, `par_map(h.info)` ran it on several cores, and one
// carrying a handled effect escaped its `with` block.
effect Log {
    fn info(m: str)
}

fn quiet(h: LogHandler) / {} {
    h.info("io from a function declaring nothing")
}

fn main() {
    quiet(LogHandler{ info: |m| print("{m}") })
}
```

## `E-HANDLER-OWN-ROW.gr`

```grasm
//! tag: E-HANDLER-OWN-ROW
//! expect: error
//! performs effect(s) `io` not in its declared row
// A handler's own effects belong to whoever installs it.
//
// They were charged where the handler was *constructed*, which is the same
// place only when the handler is written at the `with`. One function call
// away, a declared row meant nothing: this `run` installs a printing handler
// and claimed to perform nothing.
effect Log {
    fn info(m: str)
}

fn run(h: LogHandler) / {} {
    with Log handled_by h {
        Log.info("x")
    }
}

fn main() {
    run(LogHandler{ info: |m| print("{m}") })
}
```

## `E-HANDLER-SCOPE.gr`

```grasm
//! tag: E-HANDLER-SCOPE
//! expect: output
//! 42
effect Cfg {
    fn value() -> i64
}
fn deep() -> i64 {
    Cfg.value()
}
fn middle() -> i64 {
    deep()
}
fn main() {
    with Cfg handled_by CfgHandler{ value: || 42 } {
        print("{middle()}")
    }
}
```

## `E-IF-DIVERGE.gr`

```grasm
//! tag: E-IF-DIVERGE
//! expect: output
//! 25 ok2 0.25 true
//! N 7
//! 15 -1
//! 25
//! 0 -> err z
//! 1 -> 100
//! 2 -> 50
// A branch that leaves — `fail`, `panic`, `return` — has no value, so the
// `if` is worth whatever the *other* branch produced.
//
// Taking the type from the then branch alone meant `if c { fail(e) } else
// { v }` in tail position had no merge value at all: the compiled function
// returned a zero, with exit 0 and nothing reported. For a boxed payload it
// was a SIGSEGV later, in whichever code read the value back.
type E = { c: str }
type V = | N(n: i64) | S(s: str)

fn fi(y: i64) -> i64 ! E {
    if y == 0 { fail(E{ c: "z" }) } else { 100 / y }
}

fn fs(y: i64) -> str ! E {
    if y == 0 { fail(E{ c: "z" }) } else { "ok{y}" }
}

fn ff(y: i64) -> f64 ! E {
    if y == 0 { fail(E{ c: "z" }) } else { 1.0 / y.as_f64() }
}

fn fb(y: i64) -> bool ! E {
    if y == 0 { fail(E{ c: "z" }) } else { y > 1 }
}

/// A boxed payload: this was the SIGSEGV.
fn fv(y: i64) -> V ! E {
    if y == 0 { fail(E{ c: "z" }) } else { V.N(y) }
}

/// An `else if` chain of failures ending at the one arm with a value.
fn chain(y: i64) -> i64 ! E {
    if y == 0 {
        fail(E{ c: "a" })
    } else if y == 1 {
        fail(E{ c: "b" })
    } else {
        y * 3
    }
}

fn main() {
    print("{fi(4) ?? 0 - 1} {fs(2) ?? "?"} {ff(4) ?? 0.0} {fb(2) ?? false}")
    match fv(7) {
        Ok(V.N(n)) -> print("N {n}")
        Ok(V.S(s)) -> print("S {s}")
        Err(e) -> print("e {e.c}")
    }
    print("{chain(5) ?? 0 - 1} {chain(1) ?? 0 - 1}")
    // Stored and read back, which is where the boxed case crashed.
    var m: Map[str, i64 ! E] = {}
    m.set("a", fi(4))
    // `??` groups to the right, so the layered unwrap — an `Option` of a
    // fallible value — says so with brackets.
    print("{(m.get("a") ?? fi(1)) ?? 0 - 1}")
    // And the whole shape end to end.
    for n in [0, 1, 2] {
        match fi(n) {
            Ok(v) -> print("{n} -> {v}")
            Err(e) -> print("{n} -> err {e.c}")
        }
    }
}
```

## `E-IF-FALLIBLE-MERGE.gr`

```grasm
//! tag: E-IF-FALLIBLE-MERGE
//! expect: output
//! -1
//! -1 3
//! -1
//! 9 7 -1
//! 3
// An `if` whose branches are `Err(e)` and `Ok(v)` is worth `T ! E`.
//
// `Err(e)` says nothing about the ok half — there is no value to read it off
// — so a branch that is only `Err` cannot type the merge on its own. Taking
// it anyway made `xs.map(|x| if p { Err(e) } else { Ok(x) })` produce a list
// whose elements `??` then unboxed as the wrong thing: SIGSEGV, empty stderr,
// exit 139, from a compiled build that had reported no problem at all.
type E = | Bad

fn main() {
    let r = [2].map(|x| if x == 2 { Err(Bad) } else { Ok(x) })
    print("{r[0] ?? 0 - 1}")
    let p = [2, 3].par_map(|x| if x == 2 { Err(Bad) } else { Ok(x) })
    print("{p[0] ?? 0 - 1} {p[1] ?? 0 - 1}")
    var m: Map[str, i64] = {}
    m.set("a", 2)
    let v = m.values().map(|x| if x == 2 { Err(Bad) } else { Ok(x) })
    print("{v[0] ?? 0 - 1}")
    // Every arm that can say something, and no arm that leaves. Folding a
    // `return` arm in made the inner match worth nothing, so the enclosing
    // function returned whatever was in the register — `0` where the answer
    // was `9`.
    print("{g(true, 3) ?? -1} {g(true, 0) ?? -1} {g(false, 3) ?? -1}")
    // One branch builds the whole `T ! E`, the other gives the ok half, which
    // a fallible body may do either way ([T-OK-ERR-RETURN]).
    print("{h(false) ?? -1}")
}

fn g(live: bool, x: i64) -> i64 ! E {
    match live {
        false -> fail(Bad)
        _ -> match x {
            0 -> return 7
            _ -> x * 3
        }
    }
}

fn h(b: bool) -> i64 ! E {
    if b {
        Err(Bad)
    } else {
        3
    }
}
```

## `E-IF-VALUE.gr`

```grasm
//! tag: E-IF-VALUE
//! expect: output
//! big
//! 0
//! 1 true ()
fn describe(n: i64) -> str {
    if n > 10 {
        "big"
    } else {
        "small"
    }
}
fn takes_unit(u: ()) -> i64 {
    1
}

fn noop() {
}

fn main() {
    print(describe(50))
    var count = 0
    if false {
        count = 1
    }
    print("{count}")
    // `()` has one value, and that is the only thing of its type. It binds,
    // compares, prints and crosses a function boundary like any other value —
    // the compiled backend used to refuse each of those in turn.
    let nothing: () = ()
    let produced = noop()
    print("{takes_unit(nothing)} {nothing == produced} {produced}")
}
```

## `E-INDEX-BOUNDS.gr`

```grasm
//! tag: E-INDEX-BOUNDS
//! expect: panic
//! last of a scalar list: 3
//! last of a boxed list: b
//! a negative index is out of bounds, not counted from the end: true
//! index 3 out of bounds (len 3)
// Indexing past the end panics, and the interesting index is the *first* one
// past it.
//
// This program used to read `xs[5]` on a list of two, which is past the end by
// three and tests nothing an off-by-one would break. Changing the check from
// `i >= len` to `i > len` — so `xs[len]` reads one element beyond the last —
// left every gate in this repository green: 218 differential programs, the
// leak check, 31 whole programs and this suite. A bounds check is exactly the
// place where only the boundary is worth testing.
//
// Both representations are covered: a `[i64]` is stored unboxed and a `[str]`
// is a list of pointers, and they are two different index paths in the
// runtime.
fn main() / {io} {
    let xs = [1, 2, 3]
    let ss = ["a", "b"]

    // The last valid index of each, which must *not* panic.
    print("last of a scalar list: {xs[xs.len() - 1]}")
    print("last of a boxed list: {ss[ss.len() - 1]}")

    // One before the beginning. There is no wrapping: a negative index is out
    // of bounds rather than counting from the end, which `get` reports as
    // `None` and indexing reports as a panic.
    print("a negative index is out of bounds, not counted from the end: {xs.get(0 - 1) == None}")

    // Exactly one past the end — the case an off-by-one lets through.
    print("{xs[xs.len()]}")
}
```

## `E-INOUT-FAIL.gr`

```grasm
//! tag: E-INOUT-FAIL
//! expect: output
//! 1 -1 [1, 2]
//! -1 [1, 2]
// A function that *fails* still returns, and the writes it made before
// failing happened.
//
// The interpreter dropped `inout` writebacks on the error path, so a `push`
// before a `fail` was silently undone — and the compiled binary kept it. The
// two engines disagreed about what the program had *done*, not just what it
// answered.
type E = { m: str }
type Box = { xs: [i64] }

fn push_then_fail(inout xs: [i64], v: i64) -> i64 ! E {
    xs.push(v)
    if v > 1 {
        fail(E{ m: "too big" })
    }
    v
}

fn add(inout b: Box, v: i64) -> i64 ! E {
    b.xs.push(v)
    if v > 1 {
        fail(E{ m: "no" })
    }
    v
}

/// Through a `try` chain, where the failure crosses two frames.
fn outer(inout b: Box) -> i64 ! E {
    let a = try add(b, 1)
    let c = try add(b, 2)
    a + c
}

fn main() {
    var xs: [i64] = []
    let a = push_then_fail(xs, 1) ?? 0 - 1
    let b = push_then_fail(xs, 2) ?? 0 - 1
    print("{a} {b} {xs}")

    var box = Box{ xs: [] }
    print("{outer(box) ?? 0 - 1} {box.xs}")
}
```

## `E-INOUT-WRITEBACK.gr`

```grasm
//! tag: E-INOUT-WRITEBACK
//! expect: output
//! 3 [1, 2]
//! 2 [2, 2, 9] a! 3.0 true
fn bump(inout n: i64) {
    n += 1
}
fn add(inout xs: [i64], v: i64) {
    xs.push(v)
}

// The argument may name a *place* rather than a binding of its own
// ([T-INOUT-PLACE]), and the write-back has to land back in that place. A
// place holds a boxed value where an `inout` parameter is a pointer to a slot
// of the callee's own type, so the two have to meet: passing the box straight
// through, the compiled engine incremented its *address* and wrote the result
// into the field, and reading the field back said `unbox: not an i64` for a
// program `grasm run` answered. Every scalar type is here because the
// conversion is per type, and a nested field and a list element because the
// path is walked twice — once to read, once to write.
type Inner = { v: i64 }
type Outer = { i: Inner, xs: [i64], s: str, f: f64, b: bool }

fn app(inout s: str) {
    s = s + "!"
}
fn scale(inout x: f64) {
    x = x * 2.0
}
fn flip(inout b: bool) {
    b = not b
}
fn addall(inout xs: [i64]) {
    xs.push(9)
}

fn main() {
    var n = 2
    bump(n)
    var xs = [1]
    add(xs, 2)
    print("{n} {xs}")

    var o = Outer{ i: Inner{ v: 1 }, xs: [1, 2], s: "a", f: 1.5, b: false }
    bump(o.i.v)
    bump(o.xs[0])
    app(o.s)
    scale(o.f)
    flip(o.b)
    addall(o.xs)
    print("{o.i.v} {o.xs} {o.s} {o.f} {o.b}")
}
```

## `E-LIST-CONCAT.gr`

```grasm
//! tag: E-LIST-CONCAT
//! expect: output
//! [1, 2, 3]
//! [a, b, c]
//! [1.5, 2.5] [true, false]
//! [1] [1] []
//! [1, 2] [3] 3 kept
//! [[1], [2], [3]]
//! [0, 1, 4]
fn main() {
    print("{[1, 2] + [3]}")
    print("{["a"] + ["b", "c"]}")
    print("{[1.5] + [2.5]} {[true] + [false]}")
    // An empty operand on either side, and both.
    let e: [i64] = []
    print("{e + [1]} {[1] + e} {e + e}")
    // Neither operand is changed.
    let a = [1, 2]
    let b = [3]
    let joined = a + b
    print("{a} {b} {joined.len()} kept")
    // Lists of lists concatenate as lists, not element-wise.
    print("{[[1], [2]] + [[3]]}")
    // Building up in a loop.
    var acc: [i64] = []
    for i in 0..3 {
        acc = acc + [i * i]
    }
    print("{acc}")
}
```

## `E-LOOP-LIST.gr`

```grasm
//! tag: E-LOOP-LIST
//! expect: output
//! alpha beta gamma
//! 6
//! 0
fn main() {
    // The loop variable is the element itself, not an index.
    let words = ["alpha", "beta", "gamma"]
    var joined = ""
    for w in words {
        joined = joined + w + " "
    }
    print(joined.trim())

    var total = 0
    for n in [1, 2, 3] {
        total = total + n
    }
    print("{total}")

    // An empty list runs the body zero times.
    var count = 0
    let none: [i64] = []
    for _x in none {
        count = count + 1
    }
    print("{count}")
}
```

## `E-LOOP-RANGE-MAX.gr`

```grasm
//! tag: E-LOOP-RANGE-MAX
//! expect: output
//! 1 1 3
//! 10 0 0
// A range may reach the ends of `i64`.
//
// The interpreter built `a..=b` as `a..(b + 1)`, which wraps at the top of the
// range: `mx..=mx` iterated over nothing. The compiled backend incremented
// and then re-tested `i <= end`, so `i` wrapped and the loop never ended. One
// program, an empty loop and an endless one, from an overflow neither engine
// reported.
fn main() {
    let mx = 9223372036854775807
    let mn = 0 - mx - 1
    var a = 0
    for i in mx..=mx {
        a = a + 1
    }
    var b = 0
    for i in mn..=mn {
        b = b + 1
    }
    var c = 0
    for i in (mx - 2)..=mx {
        c = c + 1
    }
    print("{a} {b} {c}")
    // The ordinary cases, unchanged: inclusive counts the end, exclusive does
    // not, and a backwards range runs zero times rather than forever.
    var d = 0
    for i in 0..=4 {
        d = d + i
    }
    var e2 = 0
    for i in 3..3 {
        e2 = e2 + 1
    }
    var f = 0
    for i in 3..=2 {
        f = f + 1
    }
    print("{d} {e2} {f}")
}
```

## `E-LOOP-RANGE.gr`

```grasm
//! tag: E-LOOP-RANGE
//! expect: output
//! 0123
//! 01234
//! empty
fn main() {
    var a = ""
    for i in 0..4 {
        a += "{i}"
    }
    print(a)
    var b = ""
    for i in 0..=4 {
        b += "{i}"
    }
    print(b)
    var n = 0
    for _i in 5..5 {
        n += 1
    }
    if n == 0 {
        print("empty")
    }
}
```

## `E-MAIN-ERR.gr`

```grasm
//! tag: E-MAIN-ERR
//! expect: output-failing
//! error: unhandled error from main: Bad
type E = | Bad
fn main() ! E {
    fail(Bad)
}
```

## `E-MAP-LIT.gr`

```grasm
//! tag: E-MAP-LIT
//! expect: output
//! {a: 1, b: 2}
//! {a: 9}
//! {} 0
//! {k: 1} 3 ()
//! 2 entries
//! [k1, v1, k2, v2]
fn main() {
    // Written order does not matter: iteration is sorted (E-MAP-ORDER).
    let ordered = {"b": 2, "a": 1}
    print("{ordered}")
    // A repeated key keeps the last value written.
    let dup = {"a": 1, "a": 9}
    print("{dup}")
    // `{}` is the empty map where a map is expected...
    var empty: Map[str, i64] = {}
    print("{empty} {empty.len()}")
    // ...including as a struct field, which supplies the type.
    var b = Holder{ items: {} }
    b.items.set("k", 1)
    // Braces holding statements are still a block; `{}` is the empty map,
    // and the empty block — being useless as an expression — is spelled `()`.
    let block = { let n = 2
        n + 1 }
    print("{b.items} {block} {()}")
    // Each key is evaluated before its own value, and entries left to right.
    var log: [str] = []
    let m = {key(log, "k1"): val(log, 1), key(log, "k2"): val(log, 2)}
    print("{m.len()} entries")
    print("{log}")
}

/// Records that it ran, then answers with the key it was given.
fn key(inout log: [str], k: str) -> str {
    log.push(k)
    k
}

/// The same, for a value.
fn val(inout log: [str], v: i64) -> i64 {
    log.push("v{v}")
    v
}

type Holder = { items: Map[str, i64] }
```

## `E-MAP-ORDER.gr`

```grasm
//! tag: E-MAP-ORDER
//! expect: output
//! [alpha, beta, zulu]
//! {alpha: 1, beta: 2, zulu: 3}
//! across the inline boundary: [, 0123456789abcde, 0123456789abcdef, a, b]
//! the same text two ways is one key: 2 entries
fn main() {
    var m = map()
    m.set("zulu", 3)
    m.set("alpha", 1)
    m.set("beta", 2)
    print("{m.keys()}")
    print("{m}")

    // Short keys are held inline and long ones on the heap, so ordering has to
    // be by *content* and not by which of the two a key happens to be. Sorting
    // by representation would put every long key after every short one, and
    // this is the clause that would silently stop being true.
    var wide = map()
    for w in ["b", "a", "0123456789abcdef", "0123456789abcde", ""] {
        wide.set(w, w.len())
    }
    print("across the inline boundary: {wide.keys()}")

    // And equality by content: text built up piece by piece is the same key as
    // the identical literal, whichever side of the boundary it lands on.
    var same = map()
    same.set("0123456789abcde", 1)
    same.set("0123456789abcd" + "e", 2)
    same.set("0123456789abcdef", 1)
    same.set("0123456789abcde" + "f", 2)
    print("the same text two ways is one key: {same.len()} entries")
}
```

## `E-MATCH-BUILTIN-ARMS.gr`

```grasm
//! tag: E-MATCH-BUILTIN-ARMS
//! expect: output
//! 2.718281828459045
//! 0 false
// A match whose every arm is a builtin call still yields a value.
//
// The compiled backend types a match by probing its arms, and no builtin free
// function was in the probe's table. When *no* arm could be typed the result
// was not a refusal: "nothing could be typed" and "every arm yields nothing"
// were the same answer, so a value match compiled as a statement match and the
// enclosing function returned whatever was there. `args()` in both arms of a
// fallible function returned `0`. The fuzzer found it the day boolean
// expressions started being generated at all.
fn constant(p: i64) -> f64 ! str {
    match p {
        0 -> e()
        _ -> pi()
    }
}

fn from_env(p: i64) -> [str] ! str {
    match p {
        0 -> args()
        _ -> args()
    }
}

fn exists(p: i64) -> bool ! str {
    match p {
        0 -> file_exists("/definitely/not/here")
        _ -> file_exists("/nor/here")
    }
}

fn main() {
    match constant(0) {
        Ok(v) -> print("{v}")
        Err(x) -> print("{x}")
    }
    let n = match from_env(0) {
        Ok(v) -> v.len()
        Err(_) -> -1
    }
    let b = match exists(0) {
        Ok(v) -> v
        Err(_) -> true
    }
    print("{n} {b}")
}
```

## `E-MATCH-ORDER.gr`

```grasm
//! tag: E-MATCH-ORDER
//! expect: output
//! first
fn main() {
    let n = 5
    let r = match n {
        x if x > 0 -> "first"
        5 -> "second"
        _ -> "third"
    }
    print(r)
}
```

## `E-MINMAX-OPTION.gr`

```grasm
//! tag: E-MINMAX-OPTION
//! expect: output
//! 1 9
//! fig pear
//! true true
fn main() {
    print("{[3, 1, 9].min() ?? 0} {[3, 1, 9].max() ?? 0}")
    print("{["pear", "fig"].min() ?? "?"} {["pear", "fig"].max() ?? "?"}")
    let none: [i64] = []
    print("{none.min().is_none()} {none.max().is_none()}")
}
```

## `E-NOT-PARALLEL.gr`

```grasm
//! tag: E-NOT-PARALLEL
//! expect: output
//! sequential
fn main() {
    // Tasks run one at a time, so a task always observes the previous
    // task's completed writes — never a partial interleaving.
    var log = ""
    parallel {
        spawn {
            log += "seq"
        }
        spawn {
            log += "uential"
        }
    }
    print(log)
}
```

## `E-NUM-PROMOTE.gr`

```grasm
//! tag: E-NUM-PROMOTE
//! expect: output
//! 5.5 5.5
//! true true
//! true true true
//! 2.5 7.5
//! 3.5
// An integer *literal* meeting an `f64` becomes one — on either side, and for
// arithmetic and comparison alike. The rule used to be written as "mixing
// promotes", which is not what happens, and comparisons had an asymmetry
// nothing recorded: `5.0 == 5` was accepted and `5 == 5.0` was not.
fn main() {
    let f = 2.5
    print("{3 + f} {f + 3}")
    // Equality, both ways round.
    print("{5.0 == 5} {5 == 5.0}")
    // Ordering, both ways round.
    print("{5.5 < 6} {6 > 5.5} {6 >= 5.5}")
    // A literal in any operand position.
    print("{f * 1} {f * 3}")
    // A non-literal `i64` is converted where the program says so, and only
    // there ([T-INT-TO-FLOAT]).
    let n = 1
    print("{n.as_f64() + f}")
}
```

## `E-ORDER-LTR.gr`

```grasm
//! tag: E-ORDER-LTR
//! expect: output
//! left
//! right
//! 3
//! index
//! value
//! [[7], [0]]
//! [15, 20, 30] 1
//! [[10, 21], [30, 40]] 2
//! recv
//! arg
//! [[1, 9]]
//! 6 [[5], []] [0]
// Subexpressions evaluate left to right — including an assignment's target.
//
// The walk that writes a value is what evaluates the target's index, so doing
// it after the value ran `xs[f()] = g()` as `g` then `f`. Both engines agreed
// on the wrong order, which is why no differential test could see it.
fn note(tag: str, v: i64) -> i64 {
    print("{tag}")
    v
}

fn main() {
    let sum = note("left", 1) + note("right", 2)
    print("{sum}")
    var rows = [[0], [0]]
    rows[note("index", 0)] = [note("value", 7)]
    print("{rows}")
    // Once, and only once: a compound assignment reads and writes the same
    // place, and the index that names it is one expression. The interpreter
    // evaluated it twice and the compiled backend three times, so they wrote
    // to different elements and disagreed about the answer.
    var i = 0
    var xs = [10, 20, 30]
    xs[step(i)] += 5
    print("{xs} {i}")
    // Nested indices run outermost-first, which is leftmost-first.
    var j = 0
    var d = [[10, 20], [30, 40]]
    d[step(j)][step(j)] += 1
    print("{d} {j}")
    // A mutating method names its receiver's place once too. Reading the
    // receiver and then writing the result back walked the path twice, so
    // `k[q.remove(0)].remove(0)` took an element from one list and deleted it
    // from another.
    var g = [[1]]
    g[note("recv", 0)].push(note("arg", 9))
    print("{g}")
    var q = [1, 0]
    var k = [[5], [6]]
    let taken = k[q.remove(0)].remove(0)
    print("{taken} {k} {q}")
}

fn step(inout n: i64) -> i64 {
    let was = n
    n = n + 1
    was
}
```

## `E-OWN-RUNTIME.gr`

```grasm
//! tag: E-OWN-RUNTIME
//! expect: output
//! 3 3
fn consume(own xs: [i64]) -> i64 {
    xs.len()
}
fn plain(xs: [i64]) -> i64 {
    xs.len()
}
fn main() {
    let a = [1, 2, 3]
    let b = [1, 2, 3]
    // `own` and a plain parameter behave identically at runtime.
    print("{consume(a)} {plain(b)}")
}
```

## `E-PAD-CYCLE.gr`

```grasm
//! tag: E-PAD-CYCLE
//! expect: output
//! [007]
//! [ababx]
//! [x-=-=-]
//! [wide]
fn main() {
    print("[{"7".pad_start(3, "0")}]")
    // A multi-character fill repeats and is cut to the exact width.
    print("[{"x".pad_start(5, "ab")}]")
    print("[{"x".pad_end(6, "-=")}]")
    // Already wide enough: unchanged.
    print("[{"wide".pad_start(2, "-")}]")
}
```

## `E-PANIC-EXIT.gr`

```grasm
//! tag: E-PANIC-EXIT
//! expect: output
//! [c]
//! [abcdef]
//! []
//! [2, 3]
//! caught
// A library argument no allocation can satisfy is a panic, not an abort.
//
// `substr(2, i64::MAX)` overflowed `start + len` to a negative end and
// panicked inside Rust — a slice message carrying the developer's absolute
// path, exit 1 in one engine and 134 in another. `repeat` and `pad` reached
// the allocator's own failure message and SIGABRT. Every one of these is a
// number a program can compute, so each answers with a sentence and exit 101,
// or clamps where the clause says it clamps.
fn main() {
    let mx = 9223372036854775807
    print("[{"abc".substr(2, mx)}]")
    print("[{"abcdef".substr(0 - 9223372036854775807 - 1, mx)}]")
    print("{[1, 2, 3].slice(mx, 2)}")
    print("{[1, 2, 3].slice(1, mx)}")
    // The ones that cannot clamp panic, and a panic is catchable only by not
    // reaching it — so this program stops short and says so.
    if mx < 0 {
        print("{"ab".repeat(mx)}")
        print("{"ab".pad_start(mx, "x")}")
    }
    print("caught")
}
```

## `E-PANIC-TRACE.gr`

```grasm
//! tag: E-PANIC-TRACE
//! expect: panic
//! stack overflow: recursion too deep
// The panic line is the same in every engine; only the trace after it differs.
//
// The two engines stop at different depths on purpose — one counts calls, the
// other measures remaining stack — and both bounds are implementation-defined
// ([E-RECURSION-DEPTH]). The interpreter used to name its own number here, so
// the same failure read as two different failures depending on how you ran it.
fn down(n: i64) -> i64 {
    down(n + 1)
}

fn main() {
    print("{down(0)}")
}
```

## `E-PAR-FAIL.gr`

```grasm
//! tag: E-PAR-FAIL
//! expect: output
//! []
//! []
//! [1, 3]
// A failure inside a parallel closure is the whole call's failure — not a
// value in the result. The native backend runs such a closure sequentially,
// because its workers have nowhere to put an error.
type E = { m: str }

fn doubled(xs: [i64]) -> [i64] ! E {
    xs.par_map(|x| if x == 2 { fail(E{ m: "two" }) } else { x })
}

fn kept(xs: [i64]) -> [i64] ! E {
    xs.par_filter(|x| if x == 2 { fail(E{ m: "two" }) } else { true })
}

fn main() {
    print("{doubled([1, 2, 3]) ?? []}")
    print("{kept([1, 2, 3]) ?? []}")
    // Without a failing element the parallel path runs as usual.
    print("{kept([1, 3]) ?? []}")
}
```

## `E-PAR-FILTER-ORDER.gr`

```grasm
//! tag: E-PAR-FILTER-ORDER
//! expect: output
//! [5, 8, 9, 4]
//! true
//! true
//! true
//! true
fn main() {
    // Survivors come back in input order, not completion order.
    print("{[5, 2, 8, 1, 9, 4].par_filter(|x| x > 3)}")

    // Enough elements to cross the threshold where workers are used.
    var xs: [i64] = []
    for i in 0..20000 {
        xs.push(i)
    }
    print("{xs.par_filter(|x| x % 7 == 3) == xs.filter(|x| x % 7 == 3)}")

    let none: [i64] = []
    print("{none.par_filter(|x| x > 0).is_empty()}")

    // The same promise where the elements and the captures are *boxed*. A
    // survivor is a new reference to a value the input list still holds, so
    // this is also where a miscounted reference would show up as a leak or a
    // double free rather than as a wrong answer.
    let texts = xs.map(|x| "t{x % 331}")
    print("{texts.par_filter(|s| s.len() > 3) == texts.filter(|s| s.len() > 3)}")

    let want = "t7"
    print("{texts.par_filter(|s| s == want) == texts.filter(|s| s == want)}")
}
```

## `E-PAR-MAP-ORDER.gr`

```grasm
//! tag: E-PAR-MAP-ORDER
//! expect: output
//! true
//! [1, 4, 9, 16, 25]
//! true
//! true
//! true
//! true
//! P{a: 9, b: v4}
type P = { a: i64, b: str }

fn main() {
    // Enough elements to cross the threshold where workers are used, so this
    // exercises the threaded path rather than the sequential fallback.
    var xs: [i64] = []
    for i in 0..20000 {
        xs.push(i)
    }
    // Same answer as `map`, in input order, however many cores ran it.
    print("{xs.par_map(|x| x * 2) == xs.map(|x| x * 2)}")

    print("{[1, 2, 3, 4, 5].par_map(|x| x * x)}")

    let none: [i64] = []
    print("{none.par_map(|x| x + 1).is_empty()}")

    // The same promise where the elements, the captures or the results are
    // *boxed*. These ran sequentially until the reference count became
    // atomic — which is invisible to a program by this clause, and is exactly
    // why the clause is written about the answer rather than about threads.
    let texts = xs.par_map(|x| "n{x % 13}")
    print("{texts == xs.map(|x| "n{x % 13}")}")

    // A boxed capture: every worker reads the same value.
    let tag = "-t"
    print("{texts.par_map(|s| s + tag) == texts.map(|s| s + tag)}")

    // Boxed in, scalar out.
    print("{texts.par_map(|s| s.len()) == texts.map(|s| s.len())}")

    let ps = xs.par_map(|x| P{ a: x, b: "v{x % 5}" })
    print("{ps[9]}")
}
```

## `E-PAR-VALUES.gr`

```grasm
//! tag: E-PAR-VALUES
//! expect: output
//! [6, 8]
//! [4]
//! [3.0, 5.0]
//! [true]
// A parallel operation works on any list, however the list was built.
//
// The compiled backend picks a representation from the *static* type, so a
// `[i64]` is normally stored as raw words. `Map.values()` builds a boxed list
// even when its elements are scalars, and every other operation copes with
// both — `sum`, `sort`, indexing, `fold`, `map`, `filter`, `==`. The parallel
// ones did not, so this built and then panicked `par_map on a non-scalar
// list` while `grasm run` answered correctly: a miscompile, where §11
// promises a refusal by name at build time.
fn main() {
    var m: Map[str, i64] = {}
    m.set("a", 3)
    m.set("b", 4)
    print("{m.values().par_map(|x| x * 2)}")
    print("{m.values().par_filter(|x| x > 3)}")
    var f: Map[str, f64] = {}
    f.set("a", 1.5)
    f.set("b", 2.5)
    print("{f.values().par_map(|x| x * 2.0)}")
    var b: Map[str, bool] = {}
    b.set("a", true)
    b.set("b", false)
    print("{b.values().par_filter(|x| x)}")
}
```

## `E-PARALLEL-JOIN.gr`

```grasm
//! tag: E-PARALLEL-JOIN
//! expect: output
//! 3
fn main() {
    var done: [i64] = []
    parallel {
        spawn {
            done.push(1)
        }
        spawn {
            done.push(2)
        }
        spawn {
            done.push(3)
        }
    }
    print("{done.len()}")
}
```

## `E-POW-NEG.gr`

```grasm
//! tag: E-POW-NEG
//! expect: panic
//! negative integer exponent
fn main() {
    let e = 0 - 1
    print("{2 ^ e}")
}
```

## `E-POW.gr`

```grasm
//! tag: E-POW
//! expect: output
//! 1024 1024.0
//! 1.5 0.5 0.25
// `^` is exponentiation, not xor.
//
// Coming from the C family, `^` reads as bitwise xor, and `2 ^ 10` being 1024
// rather than 8 is the kind of difference that produces a wrong answer instead
// of an error. It was in the grammar as an operator and nowhere as a meaning.
fn main() {
    print("{2 ^ 10} {2.0 ^ 10.0}")
    // A float exponent may be fractional or negative; an integer one may not
    // be negative, which is [E-POW-NEG].
    print("{2.25 ^ 0.5} {2.0 ^ -1.0} {2.0 ^ -2.0}")
}
```

## `E-PRINT-ATOMIC.gr`

```grasm
//! tag: E-PRINT-ATOMIC
//! expect: output
//!   side 3
//!   side 4
//! x=6 y=8
// `print` evaluates every interpolation before it writes anything.
//
// Emitting each piece as it was computed saved an allocation and cost two
// guarantees: a panic partway along a line left the earlier pieces on stdout,
// against [E-PANIC-EXIT], and a nested `print` landed in the middle of the
// outer line instead of before it. This program pins the second; the first is
// in scripts/agreetest.sh, which can see stderr and the exit code.
fn side(n: i64) -> i64 {
    print("  side {n}")
    n * 2
}

fn main() {
    print("x={side(3)} y={side(4)}")
}
```

## `E-RECURSION-DEPTH.gr`

```grasm
//! tag: E-RECURSION-DEPTH
//! expect: output
//! 10000
//! 10000
// Ten thousand nested calls is the guaranteed floor, so it has to work in
// both engines. The interpreter recurses through its host's stack, and used
// to exhaust it — aborting with no output — a thousand calls before its own
// depth guard could report anything.
fn down(n: i64) -> i64 {
    if n <= 0 {
        0
    } else {
        down(n - 1) + 1
    }
}

/// Mutual recursion counts against the same bound.
fn ping(n: i64) -> i64 {
    if n <= 0 {
        0
    } else {
        pong(n - 1) + 1
    }
}

fn pong(n: i64) -> i64 {
    if n <= 0 {
        0
    } else {
        ping(n - 1) + 1
    }
}

fn main() {
    print("{down(10000)}")
    print("{ping(10000)}")
}
```

## `E-RECURSION-PARALLEL.gr`

```grasm
//! tag: E-RECURSION-PARALLEL
//! expect: output
//! 4000000
// Deep recursion inside a parallel closure is bounded the same way.
//
// A `par_map` worker runs on its own stack, and it took the platform default —
// small enough that the guard computed its floor below the end of it and could
// never fire. Recursion this deep in a worker then ran off the stack for real:
// SIGBUS, exit 138, nothing on stderr, in the one place [E-RECURSION-DEPTH]
// promises a panic and exit 101. Inputs small enough to run sequentially were
// unaffected, which is why it took a few hundred elements to see.
fn descend(n: i64) -> i64 {
    if n <= 0 {
        0
    } else {
        descend(n - 1) + 1
    }
}

fn main() {
    var xs: [i64] = []
    for i in 0..500 {
        xs.push(8000)
    }
    print("{xs.par_map(|x| descend(x)).sum()}")
}
```

## `E-SEARCH-SHORTCIRCUIT.gr`

```grasm
//! tag: E-SEARCH-SHORTCIRCUIT
//! expect: output
//! any saw 1
//! any saw 2
//! true
//! all saw 1
//! all saw 2
//! all saw 3
//! false
fn main() {
    let xs = [1, 2, 3, 4]
    let hit = xs.any(|x| {
        print("any saw {x}")
        x == 2
    })
    print("{hit}")
    let ok = xs.all(|x| {
        print("all saw {x}")
        x < 3
    })
    print("{ok}")
}
```

## `E-SHORTCIRCUIT.gr`

```grasm
//! tag: E-SHORTCIRCUIT
//! expect: output
//! false
//! true
fn boom(inout hit: bool) -> bool {
    hit = true
    true
}
fn main() {
    var hit = false
    let r = false and boom(hit)
    print("{hit}")
    let r2 = true or boom(hit)
    print("{r2}")
}
```

## `E-SPAWN-ORDER.gr`

```grasm
//! tag: E-SPAWN-ORDER
//! expect: output
//! abc
fn main() {
    var log = ""
    parallel {
        spawn {
            log += "a"
        }
        spawn {
            log += "b"
        }
        spawn {
            log += "c"
        }
    }
    print(log)
}
```

## `E-SPAWN-SHARED.gr`

```grasm
//! tag: E-SPAWN-SHARED
//! expect: output
//! 30
fn main() {
    let base = 10
    var total = 0
    parallel {
        spawn {
            total += base
        }
        spawn {
            total += base * 2
        }
    }
    print("{total}")
}
```

## `E-STR-CHARS.gr`

```grasm
//! tag: E-STR-CHARS
//! expect: output
//! 3 3
fn main() {
    // Characters, not bytes: each of these is multi-byte in UTF-8.
    let s = "héllo"
    print("{"abc".len()} {s.substr(0, 3).len()}")
}
```

## `E-STR-COMPARE.gr`

```grasm
//! tag: E-STR-COMPARE
//! expect: output
//! true false
fn main() {
    print("{"abc" < "abd"} {"b" < "a"}")
}
```

## `E-STR-CONCAT.gr`

```grasm
//! tag: E-STR-CONCAT
//! expect: output
//! hello, world
fn main() {
    print("hello, " + "world")
}
```

## `E-TASK-PANIC.gr`

```grasm
//! tag: E-TASK-PANIC
//! expect: panic
//! task exploded
fn main() {
    parallel {
        spawn {
            panic("task exploded")
        }
        spawn {
            print("must not run")
        }
    }
}
```

## `E-TEST-PROPERTY.gr`

```grasm
//! tag: E-TEST-PROPERTY
//! expect: test-output
//! test "addition commutes" ... ok (500 cases)
//! test "every int is tiny" ... FAILED
test "addition commutes" (a: i64, b: i64) {
    assert(a + b == b + a)
}

// A property passes only if *every* generated case holds.
test "every int is tiny" (n: i64) {
    assert(n < 3)
}

fn main() {
}
```

## `E-TEST-SEED.gr`

```grasm
//! tag: E-TEST-SEED
//! expect: test-output
//! counterexample: n = 1
//! "never 2" ... FAILED counterexample: n = 2
//! "never MIN" ... FAILED counterexample: n = -9223372036854775808
// Generation is a pure function of the seed, so this report is identical on
// every run and every machine.
//
// The default seed is fixed, which makes the opening cases the only ones a
// reader can count on — so they walk the boundary list in order rather than
// drawing from it. Drawn, each value was merely likely: `2` first appeared at
// case 138 of the default seed and the default run is 100 cases, so a
// property false at `2` was reported as holding.
test "no positive ints" (n: i64) {
    assert(n <= 0)
}

test "never 2" (n: i64) {
    assert(n != 2)
}

test "never MIN" (n: i64) {
    assert(n != 0 - 9223372036854775807 - 1)
}

fn main() {
}
```

## `E-TEST-SHRINK.gr`

```grasm
//! tag: E-TEST-SHRINK
//! expect: test-output
//! counterexample: n = 3
//! counterexample: xs = [0, 0]
//! counterexample: s = "aaa"
// The generator produces large values, but the reported counterexample is the
// smallest one that still fails.
test "under three" (n: i64) {
    if n > 0 {
        assert(n < 3)
    }
}

test "at most one element" (xs: [i64]) {
    assert(xs.len() < 2)
}

// A shrunk string is simple as well as short. Length was the only thing
// reduced at first, so a property failing on any three characters reported
// whichever three the generator produced — `s = "🙂/🙂"` is length-minimal
// and says nothing about why the property failed.
test "under three characters" (s: str) {
    assert(s.len() < 3)
}

fn main() {
}
```

## `E-VALUE-SEMANTICS.gr`

```grasm
//! tag: E-VALUE-SEMANTICS
//! expect: output
//! 2 3
//! 1 99
type Box = { n: i64 }
fn main() {
    var a = [1, 2]
    var b = a
    b.push(3)
    print("{a.len()} {b.len()}")

    var p = Box{ n: 1 }
    var q = p
    q.n = 99
    print("{p.n} {q.n}")
}
```

## `G-ARM-CONTROL.gr`

```grasm
//! tag: G-ARM-CONTROL
//! expect: output
//! 3
fn main() {
    var n = 0
    for i in 0..10 {
        match i {
            3 -> break
            _ -> n = i
        }
    }
    print("{n + 1}")
}
```

## `G-BENCH-CONTEXTUAL.gr`

```grasm
//! tag: G-BENCH-CONTEXTUAL
//! expect: output
//! 42
fn bench(n: i64) -> i64 {
    n * 2
}

bench "doubling" {
    let x = bench(1)
    assert(x == 2)
}

fn main() {
    // `bench` is only a keyword before a string at declaration position.
    print("{bench(21)}")
}
```

## `G-DECL-ORDER.gr`

```grasm
//! tag: G-DECL-ORDER
//! expect: output
//! 20
// `main` calls a function declared after it, and uses a type declared later.
fn main() {
    let b = Box{ n: 10 }
    print("{double(b)}")
}
fn double(b: Box) -> i64 {
    b.n * 2
}
type Box = { n: i64 }
```

## `G-LET-TUPLE.gr`

```grasm
//! tag: G-LET-TUPLE
//! expect: output
//! 1 two
//! fig=1
//! pear=2
//! 3
//! 3
//! 0a
//! 1b
//! 1x
//! 2y
//! 30
// Binding a tuple's components by name.
//
// `m.entries()` hands back `[(K, V)]` and splitting it used to cost a whole
// `match` arm — a type system charging rent rather than paying for itself.
fn main() {
    let (a, b) = (1, "two")
    print("{a} {b}")

    var m = map()
    m.set("pear", 2)
    m.set("fig", 1)
    var total = 0
    // `for (k, v) in …` is exactly the loop below with the split written
    // into the header instead of the first line of the body.
    for (k, v) in m.entries() {
        print("{k}={v}")
        total = total + v
    }
    print("{total}")

    // The long form still means the same thing.
    var again = 0
    for e in m.entries() {
        let (k, v) = e
        again = again + v + k.len() - k.len()
    }
    print("{again}")

    // It works over any list of tuples, not just `entries()`.
    for (i, w) in ["a", "b"].enumerate() {
        print("{i}{w}")
    }
    for (n, w) in [1, 2].zip(["x", "y"]) {
        print("{n}{w}")
    }

    // `var` binds them mutably.
    var (p, q) = (10, 20)
    p = p + q
    print("{p}")
}
```

## `G-MAP-BLOCK-AMBIG.gr`

```grasm
//! tag: G-MAP-BLOCK-AMBIG
//! expect: output
//! map {a: 1}
//! block 3
//! empty {}
//! body ok
//! cond ok
//! arm [2] {a: 1}
type Cfg = { on: bool }

fn main() {
    // An expression then `:` — a map.
    let m = {"a": 1}
    print("map {m}")
    // Statements — a block, whose value is its last expression.
    let b = { let n = 2
        n + 1 }
    print("block {b}")
    // Nothing at all — the empty map.
    let e: Map[str, i64] = {}
    print("empty {e}")
    // The braces of `if`/`for`/`while`/`match` and a function body are not in
    // expression position, so they stay blocks even when the first thing in
    // them could start a map entry.
    if true {
        let a = 1
        print("body {label(a)}")
    }
    // A condition's brace opens the body (G-STRUCT-AMBIG), and the body is a
    // block, not a map, even though `on: true` would read as an entry.
    let c = Cfg{ on: true }
    if c.on {
        print("cond ok")
    }
    // In a match arm, `{}` is the empty block — an arm that does nothing.
    var kept: [i64] = []
    for n in [1, 2] {
        match n {
            1 -> {}
            _ -> kept.push(n)
        }
    }
    // A non-empty map literal is still a map there.
    let picked = match 1 {
        1 -> {"a": 1}
        _ -> {"b": 2}
    }
    print("arm {kept} {picked}")
}

fn label(n: i64) -> str {
    if n == 1 { "ok" } else { "no" }
}
```

## `G-MATCH-ARM-EXPR.gr`

```grasm
//! tag: G-MATCH-ARM-EXPR
//! expect: error
//! expected an expression but found `for`
// A match arm's body is an expression. The grammar read
// `( expression | statement )`, which the parser had never accepted — so a
// reader who trusted it got this error with no way to tell which of the two
// was wrong. Write `1 -> { for i in 0..2 { ... } }` when you want a statement.
fn main() {
    let n = 1
    match n {
        1 -> for i in 0..2 { print("{i}") }
        _ -> print("no")
    }
}
```

## `G-NEST-LIMIT.gr`

```grasm
//! tag: G-NEST-LIMIT
//! expect: output
//! ok 1
// Depth is bounded, and the bound is on the tree rather than on brackets.
//
// None of these three is deep: the point is that the second and third build a
// spine as long as they are while opening no brackets at all, which is how a
// few thousand terms used to abort the toolchain with a stack overflow and
// nothing on stderr. They are counted the same way now.
fn main() {
    let nested = ((((((((((1))))))))))
    let chain = 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1
    let postfix = (1).max(1).max(1).max(1).min(1)
    if nested == 1 and chain == 8 {
        print("ok {postfix}")
    }
}
```

## `G-PAT-BIND.gr`

```grasm
//! tag: G-PAT-BIND
//! expect: output
//! none-case bound-42
//! bound 7 borrowed-name
type S = | Empty | Full(v: i64)
fn describe(s: S) -> str {
    match s {
        Empty -> "none-case"
        Full(v) -> "bound-{v}"
    }
}
// *Of the scrutinee's type*. `None` is a nullary variant of `Option`, and an
// `i64` is not an `Option`, so this binds. The compiled backend asked whether
// the name was a nullary variant of any type in the program, so it emitted a
// variant test against a scalar: the arm's body then read a name nothing had
// bound and the binary died with SIGSEGV and no output, while the interpreter
// printed the answer.
fn borrowed(n: i64) -> str {
    match n {
        None -> "bound {None}"
    }
}

// A variant that carries fields is refused rather than bound, naming them:
// `Circle` for `| Circle(r: i64)` is a variant test written without its
// payload far more often than it is a binding, and reading it as a binding
// would silently shadow the variant it looks like.
//
//     match s { Full -> "x" }
//     error: variant `Full` has 1 field(s) — bind them: `Full(..)`

// And the other way: over its own type, the same spelling still tests.
fn own_type(s: S) -> str {
    match s {
        Empty -> "borrowed-name"
        Full(v) -> "full-{v}"
    }
}

fn main() {
    print("{describe(Empty)} {describe(Full(42))}")
    print("{borrowed(7)} {own_type(Empty)}")
}
```

## `G-PIPE.gr`

```grasm
//! tag: G-PIPE
//! expect: output
//! 12
fn double_all(xs: [i64]) -> [i64] {
    xs.map(|x| x * 2)
}
fn total(xs: [i64]) -> i64 {
    xs.sum()
}
fn main() {
    print("{[1, 2, 3] |> double_all |> total}")
}
```

## `G-PLACE.gr`

```grasm
//! tag: G-PLACE
//! expect: error
//! cannot assign to a tuple component
// A tuple component is not a place.
//
// `place = identifier ( "." identifier | "[" expression "]" )*`, and an
// identifier cannot start with a digit — so `.0` never parsed as part of one.
// The checker accepted this anyway and the program panicked with `cannot
// access field `0``, a message about *reading* for a program that wrote; the
// same write spelled `t.0.push(x)` mutated the tuple under `grasm run` and
// was refused by the compiled backend.
fn main() {
    var t = (1, "x")
    t.0 = 5
    print("{t}")
}
```

## `G-STMT-LAST.gr`

```grasm
//! tag: G-STMT-LAST
//! expect: output
//! 9
fn compute() -> i64 {
    let a = 4
    let b = 5
    a + b
}
fn main() {
    print("{compute()}")
}
```

## `G-STMT-TERM.gr`

```grasm
//! tag: G-STMT-TERM
//! expect: error
//! expected end of line after statement
fn main() {
    let a = 1 let b = 2
}
```

## `G-STRUCT-AMBIG.gr`

```grasm
//! tag: G-STRUCT-AMBIG
//! expect: output
//! taken
type P = { ok: bool }
fn main() {
    let flag = true
    // `flag {` is a condition + block, not a struct literal.
    if flag {
        print("taken")
    }
    // Parenthesize to build a struct in that position.
    if (P{ ok: true }).ok {
        // reachable
    }
}
```

## `G-STRUCT-NONEMPTY.gr`

```grasm
//! tag: G-STRUCT-NONEMPTY
//! expect: error
//! struct `Empty` has no fields
// A struct has at least one field.
//
// `{}` after a name is a block — that is what makes `handled_by h { … }`
// unambiguous — so a zero-field struct is a type no value of which can be
// written. The grammar made both the field list and the literal's contents
// optional, so this was declared and accepted, and `Empty{}` then failed with
// "expected end of line after statement": a parse error about the wrong thing,
// for a declaration the grammar had offered. Use a sum variant for a value
// that carries nothing.
type Empty = { }

fn main() {
    print("unreachable")
}
```

## `G-STRUCT-UPDATE.gr`

```grasm
//! tag: G-STRUCT-UPDATE
//! expect: output
//! run n=3 total=8.0 min=1.0 max=4.0
//! 3 99 run
//! 3
//! 1 2
// Taking the fields you are not changing from a value you already have.
type Stats = { count: i64, total: f64, min: f64, max: f64, label: str }

/// The shape this exists for: an accumulator that otherwise restates every
/// field at every step.
fn add(s: Stats, v: f64) -> Stats {
    Stats{ ..s, count: s.count + 1, total: s.total + v, min: s.min.min(v), max: s.max.max(v) }
}

type Pair = { a: i64, b: i64 }

fn main() {
    var s = Stats{ count: 0, total: 0.0, min: 1.0e308, max: 0.0 - 1.0e308, label: "run" }
    for v in [3.0, 1.0, 4.0] {
        s = add(s, v)
    }
    print("{s.label} n={s.count} total={s.total.to_fixed(1)} min={s.min.to_fixed(1)} max={s.max.to_fixed(1)}")

    // The base is unchanged: this builds a new value ([E-VALUE-SEMANTICS]).
    let bumped = Stats{ ..s, count: 99 }
    print("{s.count} {bumped.count} {bumped.label}")

    // No overrides at all is a copy.
    let copy = Stats{ ..s }
    print("{copy.count}")

    // Overriding every field is allowed, and the base still decides the type.
    let p = Pair{ a: 9, b: 9 }
    let q = Pair{ ..p, a: 1, b: 2 }
    print("{q.a} {q.b}")
}
```

## `G-TEST-PARAMS.gr`

```grasm
//! tag: G-TEST-PARAMS
//! expect: error
//! supplied by the runner
test "the runner owns its inputs" (inout n: i64) {
    n = n + 1
}
fn main() {
}
```

## `G-TYPE-FALLIBLE.gr`

```grasm
//! tag: G-TYPE-FALLIBLE
//! expect: output
//! held=5
//! b!bad x
//! p=7
//! k=9
//! 3 42
//! ok 1
//! err bad x
//! ok 41
// `[i64 ! E]` is what `map` over a fallible function produces. The checker
// could always infer it; until now nothing could write it down, so a program
// that wanted to name the type declared a sum type by hand to carry it.
type E = { m: str }

fn parse(s: str) -> i64 ! E {
    s.parse_int() ?? fail(E{ m: "bad {s}" })
}

/// A fallible type as a parameter, which is the point of being able to write
/// it: this function could not be declared at all before.
fn total(rs: [i64 ! E]) -> i64 {
    var sum = 0
    for r in rs {
        sum = sum + (r ?? 0)
    }
    sum
}

/// A fallible parameter, which is what an error-reporting helper needs.
fn show(label: str, r: i64 ! E) -> str {
    match r {
        Ok(v) -> "{label}={v}"
        Err(e) -> "{label}!{e.m}"
    }
}

type Box = { r: i64 ! E, tag: str }

fn main() {
    // Held as a value, which the annotation is what permits.
    let held: i64 ! E = parse("5")
    print("{show("held", held)}")
    // A struct field, a tuple member, a type argument.
    let b = Box{ r: parse("x"), tag: "b" }
    print("{show(b.tag, b.r)}")
    let pair: (i64 ! E, str) = (parse("7"), "p")
    print("{show(pair.1, pair.0)}")
    let m: Map[str, i64 ! E] = {"k": parse("9")}
    print("{show("k", m.get("k") ?? parse("0"))}")

    let rs: [i64 ! E] = ["1", "x", "41"].map(parse)
    print("{rs.len()} {total(rs)}")
    for r in rs {
        match r {
            Ok(v) -> print("ok {v}")
            Err(e) -> print("err {e.m}")
        }
    }
}
```

## `G-TYPE-NOT-QUALIFIED.gr`

```grasm
//! tag: G-TYPE-NOT-QUALIFIED
//! expect: error
//! a type is named without a module qualifier
// Types share one global namespace ([T-MODULE-NS]), so a module qualifier on
// a type name would carry no information. The grammar used to permit one,
// which meant the only way to find out was the error.
type Thing = { n: i64 }

fn f(t: main.Thing) -> i64 {
    t.n
}

fn main() {
    print("{f(Thing{ n: 1 })}")
}
```

## `G-TYPE-UNIT.gr`

```grasm
//! tag: G-TYPE-UNIT
//! expect: output
//! done
fn noop() -> () {
}
fn main() {
    noop()
    print("done")
}
```

## `G-WILDCARD-BINDING.gr`

```grasm
//! tag: G-WILDCARD-BINDING
//! expect: output
//! discarded
//! 2
//! two wildcards bind separately
//! 3
//! b
// `_` in binding position means "do not name this".
//
// It is accepted anywhere a name is bound — `let`, a tuple `let`, and `for` —
// and not only in a `match` pattern. Each `_` is a distinct binding, so a tuple
// may ignore both halves, and none of them can be read back.
fn seven() -> i64 {
    7
}

fn main() {
    let _ = seven()
    print("discarded")

    let (_, b) = (1, 2)
    print("{b}")

    let (_, _) = (3, 4)
    print("two wildcards bind separately")

    var n = 0
    for _ in 0..3 {
        n = n + 1
    }
    print("{n}")

    for (_, v) in [(1, "a"), (2, "b")] {
        if v == "b" {
            print("{v}")
        }
    }
}
```

## `L-BOOL.gr`

```grasm
//! tag: L-BOOL
//! expect: output
//! true false
fn main() {
    print("{true} {false}")
}
```

## `L-COMMENT-BLOCK.gr`

```grasm
//! tag: L-COMMENT-BLOCK
//! expect: output
//! 1
fn main() {
    /* outer /* nested */ still a comment */
    print("1")
}
```

## `L-COMMENT-LINE.gr`

```grasm
//! tag: L-COMMENT-LINE
//! expect: output
//! 1
fn main() {
    // a whole-line comment
    print("1") // and a trailing one
}
```

## `L-CONTEXTUAL.gr`

```grasm
//! tag: L-CONTEXTUAL
//! expect: output
//! 1
//! 2
//! has 2
//! two
//! 2
// A contextual keyword is an ordinary name everywhere else.
//
// `region` is an *expression* form, and the rule that a following `{` opens a
// body — the one that makes `if User{..}.ok` need brackets — was not applied
// to it. So `for x in region { … }` read the loop's own brace as a region
// body, and a variable called `region` could not be used in a condition, a
// loop header or a `match` scrutinee at all.
fn main() {
    let region = [1, 2]
    for x in region {
        print("{x}")
    }
    if region.len() > 0 {
        print("has {region.len()}")
    }
    match region.len() {
        2 -> print("two")
        _ -> print("other")
    }
    // And the expression form still works where a brace really does open one.
    print("{region { 1 + 1 }}")
}
```

## `L-DISPLAY.gr`

```grasm
//! tag: L-DISPLAY
//! expect: output
//! -7
//! 1.5
//! 1000000.0
//! 1000000000000000.0
//! 1e300 has a point too
//! ()
//! true
//! b c
//! [1, 2, 3]
//! [a, b c]
//! [[1, 2], [3]]
//! (1, one, true)
//! {k: 1, z: 2}
//! P{x: 3, y: hi}
//! A
//! B(7)
//! Some(1)
//! None
//! [(), ()]
//! (1, ())
//! Some(())
//! ref#0.0
//! [ref#0.0]
//! Some(ref#0.0)
//! <fn main::dbl>
//! <closure>
// Every type's rendering, because a program's output is what its correctness
// is judged on. Strings are unquoted even inside a collection, so `[a, b c]`
// is two elements and the second contains a space.
type P = { x: i64, y: str }

type S =
    | A
    | B(n: i64)

type N = { v: i64 }

fn main() {
    print("{-7}")
    print("{1.5}")
    print("{1000000.0}")
    // Past 1e15, where the old rule quietly stopped applying: a float has to
    // stay distinguishable from an integer at every magnitude, or reading a
    // program's output means guessing which type produced it.
    print("{1000000000000000.0}")
    let big = 1.0e300
    let shown = "{big}"
    print("1e300 has a point too")
    assert(shown.contains("."))
    // Unit, which [T-PRIM] lists and the table did not.
    print("{()}")
    print("{true}")
    print("{"b c"}")
    print("{[1, 2, 3]}")
    print("{["a", "b c"]}")
    print("{[[1, 2], [3]]}")
    print("{(1, "one", true)}")
    var m = map()
    m.set("z", 2)
    m.set("k", 1)
    print("{m}")
    print("{P{ x: 3, y: "hi" }}")
    print("{A}")
    print("{B(7)}")
    print("{[1].first()}")
    print("{[1].get(9)}")
    // A value keeps its rendering inside a container. Both of these were
    // boxed as plain integers by the compiled backend, so a list of units
    // printed `[0, 0]` and a list of references printed the packed slot and
    // generation as a number — a wrong answer rather than a refusal, and for
    // the reference also a disclosure of where the slab put it.
    print("{[(), ()]}")
    print("{(1, ())}")
    print("{Some(())}")
    let r = ref(N{ v: 1 })
    print("{r}")
    print("{[r]}")
    print("{Some(r)}")
    release(r)
    // A function value shows the name it was declared with; a closure has
    // none to show. Both render, and [L-DISPLAY] is what output is judged on.
    print("{dbl}")
    let c: fn(i64) -> i64 = |x| x + 1
    print("{c}")
}

fn dbl(n: i64) -> i64 {
    n * 2
}
```

## `L-ENCODING.gr`

```grasm
﻿//! tag: L-ENCODING
//! expect: output
//! héllo ✓ 日本語
//! no bom in sight
fn main() {
    // Source is UTF-8; strings and comments may hold any Unicode: ✓ 日本語
    print("héllo ✓ 日本語")
    // This file begins with a byte order mark. It is ignored — legal UTF-8
    // that editors write unasked, and lexed as a character it made the first
    // declaration "unexpected character" for something that renders as
    // nothing.
    print("no bom in sight")
}
```

## `L-ESCAPE.gr`

```grasm
//! tag: L-ESCAPE
//! expect: output
//! a	b
//! quote" brace{ close}
//! [1]
//! 😀 1 [128512]
//! é
//! [a, b]
//! A3B
//! a}b
fn main() {
    print("a\tb")
    print("quote\" brace\{ close\}")
    // A character with no other spelling. A group separator used to be
    // written `"{1.to_char()}"` — an interpolation whose only purpose was to
    // reach a character the source could not say.
    print("\u{1}".codes())
    let smile = "\u{1F600}"
    print("{smile} {smile.len()} {smile.codes()}")
    print("\u{e9}")
    print("a\u{1}b".split("\u{1}"))
    // An escape covers the byte after it, inside an interpolation too: the
    // `\}` of a nested string is not the brace that closes the
    // interpolation. Counted as one, it ended the interpolation early and the
    // diagnostic that followed carried a raw NUL byte.
    print("A{"x\}y".len()}B")
    print("a\}b")
}
```

## `L-FLOAT.gr`

```grasm
//! tag: L-FLOAT
//! expect: output
//! 3.14 1000000000.0 0.0025
fn main() {
    print("{3.14} {1e9} {2.5e-3}")
}
```

## `L-IDENT.gr`

```grasm
//! tag: L-IDENT
//! expect: output
//! 6
fn main() {
    let snake_case = 1
    let _leading = 2
    let with2digits = 3
    print("{snake_case + _leading + with2digits}")
}
```

## `L-INT.gr`

```grasm
//! tag: L-INT
//! expect: output
//! 255 10 1000000
fn main() {
    print("{0xFF} {0b1010} {1_000_000}")
}
```

## `L-INTERP-ONE.gr`

```grasm
//! tag: L-INTERP-ONE
//! expect: error
//! an interpolation holds one expression
// `{...}` holds exactly one expression, and anything left over is an error.
//
// It used to be a silent truncation: `"A{1 2}B"` printed `A1B`. The parser
// read one expression and discarded the rest of the braces' contents without
// a word, so a typo inside `{...}` quietly deleted part of the output. Both
// engines agreed on the wrong answer, which is why no differential test could
// ever have found it.
fn main() {
    print("A{1 2}B")
}
```

## `L-INTERP.gr`

```grasm
//! tag: L-INTERP
//! expect: output
//! n=3 sum=7 nested=YES
fn main() {
    let n = 3
    print("n={n} sum={3 + 4} nested={if n > 1 { "YES" } else { "no" }}")
}
```

## `L-KEYWORD.gr`

```grasm
//! tag: L-KEYWORD
//! expect: error
//! expected an identifier
fn main() {
    // `type` is reserved and cannot name a variable.
    let type = 1
}
```

## `L-LEADING-DOT.gr`

```grasm
//! tag: L-LEADING-DOT
//! expect: output
//! hello; world
//! 5
//! 3 3
//! 5 chained across five lines
//! 10
// A newline before a leading `.` does not end the statement, so a chain reads
// down the left margin instead of along one unreadable line.
//
// Safe for exactly one reason: no statement may begin with a `.`, so a newline
// in front of one was never separating two statements.
fn main() / {io} {
    let cleaned = "  Hello, World  "
        .trim()
        .to_lower()
        .replace(",", ";")
    print(cleaned)

    // Bytes, which is where this was first missed: a packet is built out of
    // one `concat` per field.
    let packet = bytes_of([1, 2])
        .concat(bytes_of([3]))
        .concat(bytes_of([4, 5]))
    print("{packet.len()}")

    // Blank lines and comments between the links are fine — consecutive
    // newlines collapse into one, and one is absorbed.
    let parts = "x,y,z"
        .split(",")

        // still the same statement
        .len()
    print("{parts} {"a b c".words().len()}")

    // A chain that spans more lines than it has calls.
    let long = "  MIXED  "
        .trim()
        .to_lower()
        .repeat(1)
        .len()
    print("{long} chained across five lines")

    // The rule is about the newline, not about chaining: a chain on one line
    // means the same thing.
    print("{"aaaaa".repeat(2).len()}")
}
```

## `L-LIB-AS-I64.gr`

```grasm
//! tag: L-LIB-AS-I64
//! expect: output
//! 3 -3
//! 9223372036854775807 -9223372036854775808 0
// `as_i64` saturates. It is total, so it has an answer for every f64 and never
// panics — but "truncates toward zero" was the whole of what was written down,
// which says nothing about the values that have no i64 at all.
fn main() {
    print("{3.7.as_i64()} {(-3.7).as_i64()}")
    let huge = 1.0e30
    let nan = 0.0 / 0.0
    print("{huge.as_i64()} {(-huge).as_i64()} {nan.as_i64()}")
}
```

## `L-LIB-BASE.gr`

```grasm
//! tag: L-LIB-BASE
//! expect: output
//! ff 11111111 377 73
//! -ff 0 1 -8000000000000000
//! 7fffffffffffffff 1000000000000000000000000000000000000000000000000000000000000000
//! 255 10 1295 -255
//! not a digit in this base: true
//! a prefix is not accepted: true
//! to_base(10) agrees with to_str: true
//! round trip 2..36 over -600..600: true
// Base ten is not the only base a systems program needs. Every protocol, hash,
// colour and permission mask is read and written in hex or binary.
fn main() / {io} {
    print("{255.to_base(16)} {255.to_base(2)} {255.to_base(8)} {255.to_base(36)}")

    // `i64::MIN` goes through its unsigned magnitude, because negating it
    // overflows — the classic place a hand-written conversion is wrong.
    let min = 0 - 9223372036854775807 - 1
    print("{(0 - 255).to_base(16)} {0.to_base(2)} {1.to_base(10)} {min.to_base(16)}")
    print("{9223372036854775807.to_base(16)} {min.to_base(2).substr(1, 64)}")

    print("{"ff".parse_int_base(16) ?? 0 - 1} {"1010".parse_int_base(2) ?? 0 - 1} {"zz".parse_int_base(36) ?? 0 - 1} {"-ff".parse_int_base(16) ?? 0 - 1}")

    // A digit outside the base is not read as something else.
    print("not a digit in this base: {"19".parse_int_base(8) == None}")

    // The base is the argument, not something guessed from the text.
    print("a prefix is not accepted: {"0xff".parse_int_base(16) == None}")

    var same = true
    for n in 0 - 600..600 {
        if n.to_base(10) != n.to_str() {
            same = false
        }
    }
    print("to_base(10) agrees with to_str: {same}")

    var ok = true
    for n in 0 - 600..600 {
        for b in 2..37 {
            if (n.to_base(b).parse_int_base(b) ?? n + 1) != n {
                ok = false
            }
        }
    }
    print("round trip 2..36 over -600..600: {ok}")
}
```

## `L-LIB-BITS.gr`

```grasm
//! tag: L-LIB-BITS
//! expect: output
//! and 48 or 252 xor 204 not -241
//! shl 3840 shr 15
//! -4 15
//! 1 -9223372036854775808 0
//! rotr32 of 0x12345678 by 8 = 0x78123456
//! masked to 32 bits: 4294967295
// Bit manipulation on `i64`, as methods ([L-LIB-BITS]).
fn main() {
    let a = 0xF0
    let b = 0x3C
    print("and {a.bit_and(b)} or {a.bit_or(b)} xor {a.bit_xor(b)} not {a.bit_not()}")
    print("shl {a.shl(4)} shr {a.shr(4)}")

    // `shr` keeps the sign, `ushr` fills with zero. That is the whole reason
    // both exist.
    print("{(0 - 16).shr(2)} {(0 - 16).ushr(60)}")

    // The shift count is masked to 0..63, so none of these traps.
    print("{1.shl(64)} {1.shl(63)} {256.shr(0 - 1)}")

    // A 32-bit rotate, which is what SHA-256 is built from. There is no
    // unsigned 32-bit type, so the mask is written out.
    let mask32 = 0xFFFFFFFF
    let v = 0x12345678
    let n = 8
    let rotated = (v.ushr(n)).bit_or(v.shl(32 - n)).bit_and(mask32)
    print("rotr32 of 0x12345678 by 8 = 0x{hex(rotated)}")
    print("masked to 32 bits: {mask32.bit_and(0 - 1)}")
}

fn hex(v: i64) -> str {
    let digits = "0123456789abcdef"
    var out = ""
    var i = 7
    while i >= 0 {
        let nibble = (v.ushr(i * 4)).bit_and(15)
        out = out + digits.substr(nibble, 1)
        i = i - 1
    }
    out
}
```

## `L-LIB-BYTES.gr`

```grasm
//! tag: L-LIB-BYTES
//! expect: panic
//! 0x48c7c001000000
//! 7 bytes, first 72, third 192
//! 0x48c7c0 and 0x48c7ff
//! [72, 199, 192, 1, 0, 0, 0]
//! text is 5 characters and 6 bytes
//! round trip: héllo
//! written and read back identically: true
//! empty is empty, seven bytes are not: true false
//! last byte and last slice: 0 0x00
//! index 7 out of bounds (len 7)
// `bytes` is the type `str` deliberately is not ([L-LIB-BYTES]).
//
// The sequence below is x86-64 `mov rax, 1`. Written through a `str` it lands
// on disk as nine bytes, because 0xC7 is re-encoded as UTF-8 — which is the
// reason this type exists.
fn main() / {io} {
    let code = bytes_of([0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00])
    print("{code}")
    print("{code.len()} bytes, first {code.at(0)}, third {code.at(2)}")
    print("{code.slice(0, 3)} and {code.slice(0, 2).concat(bytes_of([0xFF]))}")
    print("{code.to_list()}")

    // A byte is not a character.
    let text = "héllo"
    print("text is {text.len()} characters and {text.to_bytes().len()} bytes")
    print("round trip: {text.to_bytes().to_str()}")

    let dir = "grasm_bytes_conformance"
    let _mk = make_dir(dir) ?? ()
    let path = dir + "/code.bin"
    match write_bytes(path, code) {
        Err(e) -> print("could not write: {e}")
        Ok(_) -> match read_bytes(path) {
            Err(e) -> print("could not read: {e}")
            Ok(back) -> print("written and read back identically: {back.to_list() == code.to_list()}")
        }
    }
    let _rm = remove_file(path) ?? ()

    // `is_empty` was the one method here the compiled backend could not build:
    // it widened `icmp`'s answer to the width it already had, which Cranelift
    // rejects, so every `b.is_empty()` in the language was a "codegen bug".
    print("empty is empty, seven bytes are not: {bytes_of([]).is_empty()} {code.is_empty()}")

    // `at` and `slice` are specified to panic out of range, and the index
    // worth testing is the first one that is. The last byte and the last
    // one-byte slice must work; one past must not.
    print("last byte and last slice: {code.at(code.len() - 1)} {code.slice(code.len() - 1, 1)}")
    print("{code.at(code.len())}")
}
```

## `L-LIB-CMP.gr`

```grasm
//! tag: L-LIB-CMP
//! expect: output
//! -1 0 1
//! -1 1 -1
//! -1 0 1
//! -1 1
//! [(a, 3), (b, 3), (c, 1)]
// The total order sorting uses ([L-LIB-SORT-NAN]), as a number.
//
// It was specified, used internally by `sort`, and callable by nothing — so a
// program needing it reimplemented it with `<` and `>`, got NaN wrong, and
// scrambled a column exactly as that clause warns. A language that names the
// hazard should hand over the fix.
fn nan() -> f64 {
    "nan".parse_float() ?? 0.0
}

/// What `cmp` makes writable: a comparator sort, with per-key direction
/// decided at run time rather than baked into a key expression.
fn sort_desc_by_count(rows: [(str, i64)]) -> [(str, i64)] {
    var out: [(str, i64)] = []
    for r in rows {
        var at = out.len()
        var i = 0
        for o in out {
            // Count descending, then name ascending.
            let by_count = r.1.cmp(o.1)
            let better = if by_count != 0 { by_count > 0 } else { r.0.cmp(o.0) < 0 }
            if better and i < at {
                at = i
            }
            i += 1
        }
        var next: [(str, i64)] = []
        var j = 0
        for o in out {
            if j == at {
                next.push(r)
            }
            next.push(o)
            j += 1
        }
        if at >= out.len() {
            next.push(r)
        }
        out = next
    }
    out
}

fn main() {
    print("{(1).cmp(2)} {(2).cmp(2)} {(3).cmp(2)}")
    // NaN sorts after every number, which is the part hand-rolling gets wrong.
    print("{(1.0).cmp(2.0)} {nan().cmp(1.0)} {(1.0).cmp(nan())}")
    print("{"a".cmp("b")} {"b".cmp("b")} {"c".cmp("b")}")
    print("{(1, 2).cmp((1, 3))} {(2, 0).cmp((1, 9))}")
    print("{sort_desc_by_count([("c", 1), ("a", 3), ("b", 3)])}")
}
```

## `L-LIB-CODES.gr`

```grasm
//! tag: L-LIB-CODES
//! expect: panic
//! 104 233
//! [104, 233, 108, 108, 111]
//! hé
//! héllo
//! 615042
//! non-bytes: 1 [8364]
//! last code point: 111
//! code_at 5 out of bounds (len 5)
fn main() {
    // Code points, counted in characters — `é` is one character, two bytes.
    print("{"héllo".code_at(0)} {"héllo".code_at(1)}")
    print("{"héllo".codes()}")
    // And back again.
    print("{104.to_char()}{233.to_char()}")
    // A full round trip through integers.
    var back = ""
    for c in "héllo".codes() {
        back = back + c.to_char()
    }
    print("{back}")
    // What the pair makes possible: arithmetic over text.
    print("{hash("grasm")}")
    // Characters outside ASCII are one code point, not their byte count.
    print("non-bytes: {"€".len()} {"€".codes()}")

    // The clause says `code_at` panics out of range, and the index worth
    // testing is the first one that is: exactly `len`. Nothing here tested it,
    // and changing the check from `i >= n` to `i > n` left every gate green
    // while the compiled build read past the end of the string and died with
    // SIGABRT where `grasm run` panicked. Reading one character too far is the
    // only mistake a bounds check can make.
    print("last code point: {"héllo".code_at("héllo".len() - 1)}")
    print("{"héllo".code_at("héllo".len())}")
}

/// A string hash, which cannot be written without code points at all.
fn hash(s: str) -> i64 {
    var h = 0
    for c in s.codes() {
        h = (h * 31 + c) % 1000007
    }
    h
}
```

## `L-LIB-EPRINT.gr`

```grasm
//! tag: L-LIB-EPRINT
//! expect: output
//! one
//! two
// `eprint` writes to standard error; `print` writes to standard output.
//
// The conformance harness compares stdout, so the two `eprint` lines below must
// NOT appear in the expected output above. That absence is the whole test: if
// `eprint` ever wrote to stdout, this program would fail with four lines where
// two were declared.
fn main() / {io} {
    print("one")
    eprint("this is a diagnostic and must not reach stdout")
    print("two")
    let n = 21
    eprint("interpolation works here too: {n * 2}")
}
```

## `L-LIB-EXIT.gr`

```grasm
//! tag: L-LIB-EXIT
//! expect: output-failing
//! about to fail
// `exit(code)` ends the program with the low eight bits of `code`.
//
// Clamping to 0..=255 instead reported *success* for `exit(-1)`, which is the
// most common spelling of "fail" in a script: the shell saw 0, and the three
// engines agreed on it.
fn main() {
    print("about to fail")
    exit(-1)
}
```

## `L-LIB-FIXED.gr`

```grasm
//! tag: L-LIB-FIXED
//! expect: output
//! 3 4 1 -1
//! 0.13 0.38 -0.13
//! 2.67 9.99 1.00
//! 100 0.01
//! 289.22 289 289.22222
//! -1.5 0.000
//! 0.3333
//! [      3.50]
//! inf -inf NaN
//! 6.875 2.292
fn main() {
    // Ties go away from zero, matching `round` ([L-LIB-ROUND]).
    print("{(2.5).to_fixed(0)} {(3.5).to_fixed(0)} {(0.5).to_fixed(0)} {(0.0 - 0.5).to_fixed(0)}")
    print("{(0.125).to_fixed(2)} {(0.375).to_fixed(2)} {(0.0 - 0.125).to_fixed(2)}")
    // A tie is a property of the stored value: 2.675 is really 2.67499...
    print("{(2.675).to_fixed(2)} {(9.995).to_fixed(2)} {(1.005).to_fixed(2)}")
    // Carrying past the point.
    print("{(99.5).to_fixed(0)} {(0.005).to_fixed(2)}")
    let mean = 289.2222222222222
    print("{mean.to_fixed(2)} {mean.to_fixed(0)} {mean.to_fixed(5)}")
    // Negatives and zero keep the requested width.
    print("{(0.0 - 1.5).to_fixed(1)} {0.0.to_fixed(3)}")
    print("{(1.0 / 3.0).to_fixed(4)}")
    // It composes with the padding a column needs.
    print("[{(3.5).to_fixed(2).pad_start(10, " ")}]")
    // Not-finite values ignore the precision, as [L-DISPLAY] renders them.
    let z = 0.0
    print("{(1.0 / z).to_fixed(2)} {((0.0 - 1.0) / z).to_fixed(0)} {(z / z).to_fixed(3)}")
    var total = 0.0
    for x in [1.5, 2.25, 3.125] {
        total = total + x
    }
    print("{total.to_fixed(3)} {(total / 3.0).to_fixed(3)}")
}
```

## `L-LIB-FREE.gr`

```grasm
//! tag: L-LIB-FREE
//! expect: output
//! 0 true
fn main() {
    // `args()` is empty with no `--` arguments; a fresh map has no entries.
    // Annotated because `map()` is an empty literal like `[]`, and one whose
    // type nothing determines is refused ([T-EMPTY-LIT]).
    var m: Map[str, i64] = map()
    print("{args().len()} {m.is_empty()}")
    assert(1 + 1 == 2)
    assert_eq(2 * 2, 4)
}
```

## `L-LIB-FS.gr`

```grasm
//! tag: L-LIB-FS
//! expect: output
//! listing is sorted: [a.txt, b.txt, c.txt]
//! exists before: true
//! exists after: false
//! listing after removal: [b.txt, c.txt]
//! removing an absent file reports NotFound
//! listing an absent directory reports NotFound
// Files can be removed, and a directory can be read ([L-LIB-FS]).
fn main() / {io} {
    let dir = "grasm_fs_conformance"
    // The program makes its own working directory, so it is self-contained and
    // leaves nothing behind. `make_dir` succeeds if it already exists.
    let _mk = make_dir(dir) ?? ()
    // Written out of order on purpose: `list_dir` sorts, so the order below is
    // the sorted one and not the order the filesystem happens to hold.
    let _b = write_file(dir + "/b.txt", "b") ?? ()
    let _c = write_file(dir + "/c.txt", "c") ?? ()
    let _a = write_file(dir + "/a.txt", "a") ?? ()

    match list_dir(dir) {
        Ok(names) -> print("listing is sorted: {names}")
        Err(e) -> print("could not list: {e}")
    }
    print("exists before: {file_exists(dir + "/a.txt")}")
    match remove_file(dir + "/a.txt") {
        Ok(_) -> {}
        Err(e) -> print("could not remove: {e}")
    }
    print("exists after: {file_exists(dir + "/a.txt")}")
    match list_dir(dir) {
        Ok(names) -> print("listing after removal: {names}")
        Err(e) -> print("could not list: {e}")
    }
    match remove_file(dir + "/a.txt") {
        Ok(_) -> print("removed a file that was not there")
        Err(e) -> print("removing an absent file reports NotFound")
    }
    match list_dir(dir + "/nope") {
        Ok(_) -> print("listed a directory that was not there")
        Err(e) -> print("listing an absent directory reports NotFound")
    }
    // Leave nothing behind: running the suite twice is the same as once.
    let _rb = remove_file(dir + "/b.txt") ?? ()
    let _rc = remove_file(dir + "/c.txt") ?? ()
}
```

## `L-LIB-IO.gr`

```grasm
//! tag: L-LIB-IO
//! expect: output
//! missing: NotFound(grasm_absent_9f3c.txt)
//! exists: false
//! env: unset
//! types: ok
// The io builtins report absence in three different ways, on purpose — and
// nothing pinned which was which, so `read_file` sat documented as returning
// `Option[str]` while it was fallible.
//
// Only the failing paths are exercised, because those are the ones that need
// no filesystem state and give the same answer on every machine. The writing
// side is still *typechecked*, in a branch that never runs.
fn main() {
    // Fallible: the error says why.
    match read_file("grasm_absent_9f3c.txt") {
        Ok(s) -> print("missing: read {s.len()}")
        Err(e) -> print("missing: {e}")
    }
    // bool: the question was already yes-or-no.
    print("exists: {file_exists("grasm_absent_9f3c.txt")}")
    // Option: an unset variable has no reason to give.
    print("env: {env("GRASM_A_VARIABLE_NOBODY_SETS_9F3C") ?? "unset"}")
    // Typechecked, never run — enough to pin the signatures.
    if file_exists("grasm_absent_9f3c.txt") {
        // A write can fail, and its failure is not discardable: the shape
        // that printed "wrote the report" with no file on disk was exactly
        // this call as a bare statement.
        match write_file("grasm_absent_9f3c.txt", "unused") {
            Ok(_) -> print("wrote")
            Err(e) -> print("{e}")
        }
        let line = input()
        let ms = now_millis()
        print("{line} {ms}")
    }
    print("types: ok")
}
```

## `L-LIB-LIST.gr`

```grasm
//! tag: L-LIB-LIST
//! expect: output
//! 3 false 3 -1 3 2
//! [1, 2, 3] 4 [3, 2, 1] [1, 2] true 1-2-3
//! 6 1 3
//! [2, 4, 6] [2, 3, 4] [2, 3]
//! true true 2
//! 6 6 123
fn main() {
    var xs = [3, 1, 2]
    print("{xs.len()} {xs.is_empty()} {xs[0]} {xs.get(9) ?? 0 - 1} {xs.first() ?? 0} {xs.last() ?? 0}")
    xs.push(4)
    let popped = xs.pop() ?? 0
    xs.sort()
    // `join` takes [str], so numbers are mapped to text first.
    print("{xs} {popped} {xs.rev()} {xs.slice(0, 2)} {xs.contains(2)} {xs.map(|v| v.to_str()).join("-")}")
    print("{xs.sum()} {xs.min() ?? 0} {xs.max() ?? 0}")
    print("{xs.map(|v| v * 2)} {xs.par_map(|v| v + 1)} {xs.filter(|v| v > 1)}")
    print("{xs.any(|v| v > 2)} {xs.all(|v| v > 0)} {xs.find(|v| v > 1) ?? 0}")

    // A left fold, whose accumulator type comes from the initial value —
    // `[i64].fold("", ..)` produces a `str`.
    print("{xs.fold(0, |a, b| a + b)} {xs.fold(1, |a, b| a * b)} {xs.fold("", |a, b| a + b.to_str())}")
}
```

## `L-LIB-MAP.gr`

```grasm
//! tag: L-LIB-MAP
//! expect: output
//! 2 false 1 true [a, b] [1, 2]
//! [(a, 1), (b, 2)]
//! a=1
//! b=2
//! true 1
fn main() {
    var m = map()
    m.set("b", 2)
    m.set("a", 1)
    print("{m.len()} {m.is_empty()} {m.get("a") ?? 0} {m.has("b")} {m.keys()} {m.values()}")

    // Pairs, in key order. Without this the only way to walk a map was to
    // iterate `keys()` and look each one up again — a probe per entry, and a
    // `?? default` for a key that certainly exists.
    print("{m.entries()}")
    for e in m.entries() {
        match e {
            (k, v) -> print("{k}={v}")
        }
    }

    let gone = m.delete("b")
    print("{gone} {m.len()}")
}
```

## `L-LIB-MATH.gr`

```grasm
//! tag: L-LIB-MATH
//! expect: output
//! 3.141593 2.718282
//! 0.0000 1.0000 0.785398
//! 2.718282 1.000000 3.0000 3.0000
//! 5.00 3.00 4.0000 1.047198
//! 3.141593 180.00 1.1752 0.7616
//! 99.0
// The math library, and the reason it is a library at all.
//
// Reaching libm through `extern fn cos` works, and it costs a capability: a
// declared C function performs `io` ([T-EXTERN-VALUE]), so a package declaring
// `capabilities = []` could not compute a cosine. A pure function should not
// cost a capability.
fn shadowed() -> f64 {
    // `pi` and `e` are the only builtins that yield to a function of the same
    // name. `e` is common enough as an identifier that taking it outright
    // would break working programs.
    99.0
}

fn main() {
    print("{pi().to_fixed(6)} {e().to_fixed(6)}")
    print("{(0.0).sin().to_fixed(4)} {(0.0).cos().to_fixed(4)} {(1.0).atan().to_fixed(6)}")
    print("{(1.0).exp().to_fixed(6)} {e().ln().to_fixed(6)} {(8.0).log2().to_fixed(4)} {(1000.0).log10().to_fixed(4)}")
    print("{(3.0).hypot(4.0).to_fixed(2)} {(27.0).cbrt().to_fixed(2)} {(81.0).log(3.0).to_fixed(4)} {(0.5).acos().to_fixed(6)}")
    print("{(180.0).to_radians().to_fixed(6)} {pi().to_degrees().to_fixed(2)} {(1.0).sinh().to_fixed(4)} {(1.0).tanh().to_fixed(4)}")
    print("{shadowed()}")
}
```

## `L-LIB-NET.gr`

```grasm
//! tag: L-LIB-NET
//! expect: output
//! listening
//! a chosen port is in range
//! empty wait set: 0 ready
//! a handle that names nothing is an Err
//! before dial: nothing ready
//! after dial: the listener
//! before send: nothing ready
//! after send: the connection
//! read: ping
//! after hangup: the connection
//! read at end: 0 bytes
//! closed twice is fine
//! a closed handle reports rather than panics
//! nothing is listening there, and that is an Err
//! an unresolvable name is an Err too
//! as text, 0xff 0xfe became: [239, 191, 189, 239, 191, 189, 65, 66, 0, 67]
//! as bytes, they survived: [255, 254, 65, 66, 0, 67]
// The `net` capability: handles are integers, payloads are text.
//
// Port 0 asks the system to choose, so this program can run anywhere without
// colliding with whatever else holds a port. It does not accept a connection —
// that would need a second process — and the shapes it does check are the ones
// with an answer that does not depend on the machine.
/// Names the ready handles by their role, so the transcript does not depend on
/// which numbers this machine's socket table happened to hand out.
fn report(step: str, ready: [i64], listener: i64, conn: i64) / {io} {
    var out = ""
    for h in ready {
        if h == listener {
            out = out + "the listener "
        } else if h == conn {
            out = out + "the connection "
        } else {
            out = out + "handle {h} "
        }
    }
    if out == "" {
        print("{step}: nothing ready")
    } else {
        print("{step}: {out.trim()}")
    }
}

fn main() / {io, net} {
    match tcp_listen(0) {
        Err(e) -> print("could not listen")
        Ok(listener) -> {
            print("listening")

            // Port 0 asked the system to choose. Reading the choice back is
            // what makes that spelling usable: the rest of this program dials
            // the listener it just opened, on a machine where the port it
            // would otherwise have picked may already be held.
            let port = net_port(listener) ?? 0
            if port > 0 and port <= 65535 {
                print("a chosen port is in range")
            } else {
                print("a chosen port is NOT in range: {port}")
            }

            // --- readiness --------------------------------------------------
            //
            // `net_wait` is what lets one thread hold many conversations.
            // Each step checks readiness *before* the event that causes it as
            // well as after: a `net_wait` that always answered "ready" would
            // pass a test that only looked at the second half, and would make
            // every server written against it busy-spin.

            // An empty wait set is answered immediately and is not an error —
            // a server that has just closed its last connection asks exactly
            // this on its next turn.
            match net_wait([], 0) {
                Ok(r) -> print("empty wait set: {r.len()} ready")
                Err(e) -> print("empty wait set: FAILED")
            }

            // A handle that names nothing is an ordinary failure, the same as
            // using a connection after closing it — never a panic.
            match net_wait([4242], 0) {
                Ok(r) -> print("a handle that names nothing was accepted, which it must not")
                Err(e) -> print("a handle that names nothing is an Err")
            }

            report("before dial", net_wait([listener], 0) ?? [], listener, -1)

            match tcp_connect("127.0.0.1", port) {
                Err(e) -> print("could not dial the listener")
                Ok(client) -> {
                    // The handshake completed, so accepting will not block.
                    report("after dial", net_wait([listener], 1000) ?? [], listener, -1)

                    match tcp_accept(listener) {
                        Err(e) -> print("could not accept")
                        Ok(conn) -> {
                            // Accepted, but the client has said nothing yet.
                            report("before send", net_wait([conn], 0) ?? [], listener, conn)

                            match tcp_write(client, "ping") {
                                Ok(_) -> {}
                                Err(e) -> print("could not write")
                            }

                            // Both handles in one call, so the answer also
                            // shows the ready ones being picked out rather
                            // than the whole set coming back.
                            report("after send", net_wait([listener, conn], 1000) ?? [], listener, conn)
                            print("read: {tcp_read(conn) ?? "(nothing)"}")

                            // A hung-up peer is ready: the read that follows
                            // returns nothing, which is how a caller learns
                            // the conversation ended. Reporting it as
                            // not-ready would leave a dead connection in the
                            // wait set forever.
                            net_close(client)
                            report("after hangup", net_wait([conn], 1000) ?? [], listener, conn)
                            print("read at end: {(tcp_read(conn) ?? "").len()} bytes")
                            net_close(conn)
                        }
                    }
                }
            }

            // Closing twice is not an error, for the same reason releasing a
            // `ref` twice is not: cleanup order is not the caller's to track.
            net_close(listener)
            net_close(listener)
            print("closed twice is fine")
            // Using a closed handle is an ordinary failure, never a panic and
            // never a wrong answer.
            match tcp_read(listener) {
                Ok(_) -> print("a closed handle answered, which it must not")
                Err(e) -> print("a closed handle reports rather than panics")
            }
            // `tcp_connect` is the client half. Port 1 is privileged and
            // nothing listens there, so this is a refusal rather than a
            // machine-dependent answer.
            match tcp_connect("127.0.0.1", 1) {
                Ok(_) -> print("something answered on port 1, which it must not")
                Err(e) -> print("nothing is listening there, and that is an Err")
            }
            match tcp_connect("no.such.host.invalid", 80) {
                Ok(_) -> print("an invalid name resolved, which it must not")
                Err(e) -> print("an unresolvable name is an Err too")
            }

            // Text and bytes are two reads, not one with a flag, and the
            // difference is visible: `tcp_read` replaces what is not valid
            // UTF-8, which is right for a protocol made of text and destroys a
            // JPEG. The same six bytes are sent twice and read both ways.
            binary_round_trip()
        }
    }
}

/// The same payload read as text and as bytes. Sent twice over two fresh
/// connections so neither read can see the other's data.
fn binary_round_trip() / {net, io} {
    let payload = [255, 254, 65, 66, 0, 67]
    print("as text, 0xff 0xfe became: {echo_as(payload, true)}")
    print("as bytes, they survived: {echo_as(payload, false)}")
}

/// Sends `payload` to a listener on this machine and reads it back, as text
/// when `as_text` and as bytes otherwise. Answers with the byte values seen.
fn echo_as(payload: [i64], as_text: bool) -> [i64] / {net} {
    match tcp_listen(0) {
        Err(e) -> []
        Ok(srv) -> {
            var seen: [i64] = []
            match net_port(srv) {
                Err(e) -> {}
                Ok(port) -> match tcp_connect("127.0.0.1", port) {
                    Err(e) -> {}
                    Ok(cli) -> {
                        match tcp_write_bytes(cli, bytes_of(payload)) {
                            Err(e) -> {}
                            Ok(_) -> match tcp_accept(srv) {
                                Err(e) -> {}
                                Ok(conn) -> {
                                    if as_text {
                                        seen = (tcp_read(conn) ?? "").to_bytes().to_list()
                                    } else {
                                        seen = (tcp_read_bytes(conn) ?? bytes_of([])).to_list()
                                    }
                                    net_close(conn)
                                }
                            }
                        }
                        net_close(cli)
                    }
                }
            }
            net_close(srv)
            seen
        }
    }
}
```

## `L-LIB-NUM.gr`

```grasm
//! tag: L-LIB-NUM
//! expect: output
//! 4 2.0 3.0 3.0 3.0
//! 3 5 7 3 2.0
fn main() {
    print("{(0 - 4).abs()} {(2.5).floor()} {(2.1).ceil()} {(2.6).round()} {(9.0).sqrt()}")
    // Conversion is by method: as_f64 widens, as_i64 truncates.
    print("{(3).min(5)} {(3).max(5)} {(7).to_str()} {(3.7).as_i64()} {(2).as_f64()}")
}
```

## `L-LIB-OPTION.gr`

```grasm
//! tag: L-LIB-OPTION
//! expect: output
//! false 3 true 0
//! 3
fn main() {
    let xs = [3, 1]
    let o = xs.get(0)
    print("{o.is_none()} {o.unwrap_or(0)} {xs.get(99).is_none()} {xs.get(99) ?? 0}")
    // For when absence is a bug rather than a case: the message is required,
    // because a panic nobody can read is not better than the `??` it replaced.
    print("{xs.first().expect("a two-element list has a first")}")
}
```

## `L-LIB-PAIRS.gr`

```grasm
//! tag: L-LIB-PAIRS
//! expect: output
//! [(0, a), (1, b), (2, c)]
//! [(1, x), (2, y)]
//! [] []
//! a0 b1 c2
//! 1x 2y
fn main() {
    let ws = ["a", "b", "c"]
    print("{ws.enumerate()}")
    // `zip` stops at the shorter of the two — pairing past the end of one
    // would have to invent an element.
    print("{[1, 2, 3].zip(["x", "y"])}")
    // Empty on either side gives empty.
    let none: [i64] = []
    print("{none.enumerate()} {none.zip(ws)}")
    // What they are for: naming both halves in the loop header.
    var out: [str] = []
    for (i, w) in ws.enumerate() {
        out.push("{w}{i}")
    }
    print("{out.join(" ")}")
    var pairs: [str] = []
    for (n, w) in [1, 2].zip(["x", "y"]) {
        pairs.push("{n}{w}")
    }
    print("{pairs.join(" ")}")
}
```

## `L-LIB-PARSE.gr`

```grasm
//! tag: L-LIB-PARSE
//! expect: output
//! inf NaN inf -inf
//! none none
//! 42 -7 none none
//! [1.5, -2.5] true
//! cdef |  |
// What a parser does with input that is nearly a number. Untrusted data
// reaches these, and every surprise here becomes a surprise somewhere else:
// `"1e400"` parsing to `inf` is what put an infinity into `as_i64` and into a
// sort, so both of those had to be specified too.
fn show(o: Option[f64]) -> str {
    match o {
        Some(v) -> "{v}"
        None -> "none"
    }
}

fn showi(o: Option[i64]) -> str {
    match o {
        Some(v) -> "{v}"
        None -> "none"
    }
}

fn main() {
    // Infinities and NaN are accepted, and overflow gives an infinity.
    print("{show("inf".parse_float())} {show("nan".parse_float())} {show("1e400".parse_float())} {show("-inf".parse_float())}")
    // Text that is not a number is `None`, not zero.
    print("{show("abc".parse_float())} {showi("abc".parse_int())}")
    // Space and a sign are fine; source-literal spellings are not.
    print("{showi(" +42 ".parse_int())} {showi("-7".parse_int())} {showi("1_000".parse_int())} {showi("0xFF".parse_int())}")
    // Which is why a program reading untrusted numbers can ask.
    var kept: [f64] = []
    for c in ["1.5", "1e400", "nan", "-2.5"] {
        let v = c.parse_float() ?? 0.0
        if v.is_finite() {
            kept.push(v)
        }
    }
    print("{kept} {(0.0 / 0.0).is_nan()}")
    // `substr` clamps rather than panicking, unlike indexing.
    print("{"abcdef".substr(2, 100)} | {"abc".substr(5, 2)} |")
}
```

## `L-LIB-PROC.gr`

```grasm
//! tag: L-LIB-PROC
//! expect: output
//! echo: code 0 out "hello world" err empty
//! exit status is carried: 3
//! stderr is separate from stdout: to-err
//! arguments do not go through a shell: a b;c
//! a program that cannot be started is an Err: true
//! and that is different from one that ran and failed: 1
//! a pipeline feeds each stage into the next: a b c
//! input reaches the first stage: hello
//! the status is the last stage's: 4
//! an empty pipeline is an Err: true
//! a stage that cannot start is an Err: true
// Running another program, under a capability of its own.
//
// `proc` is not `io` and not `net`. It is strictly larger than either: a
// program that can run `curl` has the network whether or not it declared
// `net`. The tools used here are the ones POSIX guarantees.
fn main() / {io, proc} {
    match proc_run("echo", ["hello", "world"]) {
        Err(e) -> print("echo failed")
        Ok(r) -> print("echo: code {r.0} out \"{r.1.trim()}\" err {if r.2.len() == 0 { "empty" } else { r.2 }}")
    }

    match proc_run("sh", ["-c", "exit 3"]) {
        Err(e) -> print("sh failed")
        Ok(r) -> print("exit status is carried: {r.0}")
    }

    // The two streams do not run together, which is the reason they are
    // returned separately rather than joined.
    match proc_run("sh", ["-c", "echo to-err 1>&2"]) {
        Err(e) -> print("sh failed")
        Ok(r) -> print("stderr is separate from stdout: {r.2.trim()}{r.1.trim()}")
    }

    // Arguments are passed as a list, never through a shell, so a `;` in one of
    // them is a character and not a command separator.
    match proc_run("echo", ["a", "b;c"]) {
        Err(e) -> print("echo failed")
        Ok(r) -> print("arguments do not go through a shell: {r.1.trim()}")
    }

    // Not being able to start a program is a different thing from a program
    // that ran and returned a failure, and the two are not confusable.
    let missing = match proc_run("no-such-program-anywhere", []) {
        Err(e) -> true
        Ok(r) -> false
    }
    print("a program that cannot be started is an Err: {missing}")
    match proc_run("sh", ["-c", "exit 1"]) {
        Err(e) -> print("sh failed")
        Ok(r) -> print("and that is different from one that ran and failed: {r.0}")
    }

    // A pipeline: one child's stdout *is* the next one's stdin, which is the
    // thing `proc_run` cannot express because it waits.
    //
    // `sort` and `uniq` and not `wc`: `wc -l` right-aligns its count on the
    // BSD tools and does not on the GNU ones, and a gate whose answer depends
    // on which machine ran it is the defect F18 records.
    match proc_pipeline([("sort", []), ("uniq", [])], "b\na\nb\nc\na\n") {
        Err(e) -> print("pipeline failed")
        Ok(r) -> print("a pipeline feeds each stage into the next: {r.1.trim().lines().join(" ")}")
    }

    // `input` feeds the first stage. A filter with nothing to filter is not a
    // filter, and `proc_run` had no way to supply it.
    match proc_pipeline([("cat", [])], "hello\n") {
        Err(e) -> print("pipeline failed")
        Ok(r) -> print("input reaches the first stage: {r.1.trim()}")
    }

    // The answer is the *last* stage's, which is what every shell means.
    match proc_pipeline([("echo", ["ignored"]), ("sh", ["-c", "cat > /dev/null; exit 4"])], "") {
        Err(e) -> print("pipeline failed")
        Ok(r) -> print("the status is the last stage's: {r.0}")
    }

    let empty = match proc_pipeline([], "") {
        Err(e) -> true
        Ok(r) -> false
    }
    print("an empty pipeline is an Err: {empty}")

    let missing = match proc_pipeline([("no-such-program-anywhere", [])], "") {
        Err(e) -> true
        Ok(r) -> false
    }
    print("a stage that cannot start is an Err: {missing}")
}
```

## `L-LIB-RANGE-OPS.gr`

```grasm
//! tag: L-LIB-RANGE-OPS
//! expect: output
//! [0, 1, 2, 0, 1]
//! 10
//! [0, 2, 4, 6, 8]
//! [0, 1, 2, 3]
//! 7
//! 165
// A range answers the list combinators, producing a list of the same elements
// `for i in a..b` walks.
//
// Building a list from a count needed `var xs: [i64] = []`, a `for`, a `push`
// and two closing braces — four lines where `(0..n).map(f)` says the same
// thing, in every program that builds a list from a count. [G11] budgets Grasm
// at or below Python's token count; that pattern was most of the gap.
//
// There is one definition of what a range contains and not two: this engine
// turns the range into the list and reuses the list method, and the compiled
// backend rewrites the call into the loop it already compiles, since it has no
// range *value* (§11). So the three engines agree by construction rather than
// by two implementations happening to match.
fn main() {
    print("{(0..5).map(|i| i % 3)}")
    print("{(1..=4).sum()}")
    print("{(0..10).filter(|i| i % 2 == 0)}")
    print("{(0..4).to_list()}")
    print("{(0..7).len()}")
    // Composes with the list surface, since what comes back *is* a list.
    print("{(0..10).map(|i| i * i).filter(|n| n % 2 == 1).sum()}")
}
```

## `L-LIB-REMOVE.gr`

```grasm
//! tag: L-LIB-REMOVE
//! expect: panic
//! 10 30 [20, 40]
//! b [a, c]
//! x 1
//! [3, 1, 2]
//! the last element can be removed: 2
//! remove 1 out of bounds (len 1)
// Taking an element out by index, which is what draining a queue from the
// front needs.
type Job = { id: i64, name: str }

fn main() {
    var q = [10, 20, 30, 40]
    let first = q.remove(0)
    let mid = q.remove(1)
    print("{first} {mid} {q}")

    // Boxed elements too.
    var names = ["a", "b", "c"]
    let b = names.remove(1)
    print("{b} {names}")
    var jobs = [Job{ id: 1, name: "x" }, Job{ id: 2, name: "y" }]
    let j = jobs.remove(0)
    print("{j.name} {jobs.len()}")

    // The shape it exists for: a queue drained in order.
    var ready = [3, 1, 2]
    var order: [i64] = []
    while ready.len() > 0 {
        order.push(ready.remove(0))
    }
    print("{order}")

    // The clause says `remove` panics out of range "like indexing", and the
    // index worth testing is the first one that is: exactly `len`. Removing
    // the *last* element must work, and removing one past it must not — the
    // pair is the whole of what a bounds check can get wrong.
    var last = [1, 2]
    let two = last.remove(last.len() - 1)
    print("the last element can be removed: {two}")
    let _gone = last.remove(last.len())
    print("unreachable {_gone}")
}
```

## `L-LIB-ROUND.gr`

```grasm
//! tag: L-LIB-ROUND
//! expect: output
//! 3.0 4.0 -3.0 -1.0
//! 2.0 3.0 -2.0 -0.0
//! inf -inf NaN
//! -9223372036854775808
fn main() {
    // Ties go away from zero, not to even.
    print("{(2.5).round()} {(3.5).round()} {(0.0 - 2.5).round()} {(0.0 - 0.5).round()}")
    // Non-ties are unremarkable, and `-0.4` rounds to `-0.0`, not `0.0`.
    print("{(2.4).round()} {(2.6).round()} {(0.0 - 2.4).round()} {(0.0 - 0.4).round()}")
    // Division by zero on floats is not a panic; it is the IEEE answer.
    let z = 0.0
    print("{1.0 / z} {(0.0 - 1.0) / z} {z / z}")
    // The one integer with no positive counterpart wraps to itself.
    let least = 0 - 9223372036854775807 - 1
    print("{least.abs()}")
}
```

## `L-LIB-SORT-BY.gr`

```grasm
//! tag: L-LIB-SORT-BY
//! expect: output
//! al 25
//! cy 30
//! bo 30
//! [3, 2, 1]
//! [a, bb, ccc]
//! y
//! z
//! [7] []
type P = { name: str, age: i64 }

fn main() {
    // A list of structs has no order of its own; the key does.
    var ps = [P{ name: "cy", age: 30 }, P{ name: "al", age: 25 }, P{ name: "bo", age: 30 }]
    ps.sort_by(|p| p.age)
    // Stable: `cy` was written before `bo` and both are 30, so it stays first.
    for p in ps {
        print("{p.name} {p.age}")
    }
    // Any i64/f64/str key works, including a computed one.
    var ns = [3, 1, 2]
    ns.sort_by(|n| 0 - n)
    print("{ns}")
    var ws = ["ccc", "a", "bb"]
    ws.sort_by(|w| w.len())
    print("{ws}")
    // A key extracted from a named function works the same as a closure.
    var ps2 = [P{ name: "z", age: 1 }, P{ name: "y", age: 0 }]
    ps2.sort_by(age_of)
    for p in ps2 {
        print("{p.name}")
    }
    // An empty list and a single element are both no-ops, not errors.
    var one = [7]
    one.sort_by(|n| n)
    var none: [i64] = []
    none.sort_by(|n| n)
    print("{one} {none}")
}

fn age_of(p: P) -> i64 {
    p.age
}
```

## `L-LIB-SORT-NAN.gr`

```grasm
//! tag: L-LIB-SORT-NAN
//! expect: output
//! [1.0, 2.0, 3.0, 4.0, 5.0, NaN]
//! [-inf, -1.0, 0.0, 1.0, inf, NaN]
//! [(1.0, a), (2.0, b), (NaN, c)]
// A NaN in a list of floats used to leave the whole list unsorted, not just
// the NaN misplaced: calling every NaN comparison "equal" is not an order,
// and a sort given a non-order returns whatever it happens to return. One
// `nan` cell in an otherwise clean file was enough to scramble a column.
fn nan() -> f64 {
    "nan".parse_float() ?? 0.0
}

fn main() {
    var xs = [5.0, 4.0, 3.0, nan(), 2.0, 1.0]
    xs.sort()
    print("{xs}")
    // NaN sorts last; the infinities sit at the ends where they belong.
    var ys = [1.0, nan(), 0.0 - 1.0, 1.0 / 0.0, 0.0, (0.0 - 1.0) / 0.0]
    ys.sort()
    print("{ys}")
    // And the same order is what a float sort key uses.
    var ps = [(nan(), "c"), (2.0, "b"), (1.0, "a")]
    ps.sort_by(|p| p.0)
    print("{ps}")
}
```

## `L-LIB-STR.gr`

```grasm
//! tag: L-LIB-STR
//! expect: output
//! 11 false
//! ell abc 2
//! true true true 6
//! HeLLo,WorLd x HELLO,WORLD hello,world abab
//! chars 1 3 3 | bytes 4 4 3
//! 007 7.. 2 5 2.5
fn main() {
    let s = "Hello,World"
    print("{s.len()} {s.is_empty()}")
    // substr is (start, length) — not (start, end).
    print("{s.substr(1, 3)} {"abcdef".substr(0, 3)} {s.split(",").len()}")
    print("{s.contains("World")} {s.starts_with("He")} {s.ends_with("ld")} {s.index_of("W") ?? 0 - 1}")
    print("{s.replace("l", "L")} {"  x ".trim()} {s.to_upper()} {s.to_lower()} {"ab".repeat(2)}")
    // Characters and bytes are the same number only for ASCII, and a length
    // that goes on the wire has to be the second one.
    print("chars {"🙂".len()} {"añb".len()} {"abc".len()} | bytes {"🙂".byte_len()} {"añb".byte_len()} {"abc".byte_len()}")
    print("{"7".pad_start(3, "0")} {"7".pad_end(3, ".")} {"a\nb".lines().len()} {"5".parse_int() ?? 0} {"2.5".parse_float() ?? 0.0}")
}
```

## `L-LIB-SUM.gr`

```grasm
//! tag: L-LIB-SUM
//! expect: output
//! 6 0
//! 6.0 3.0
//! 0
//! 0.0 0.0000
//! 0.0
// The sum of an empty list is the zero of the element type.
//
// The last two lines were the deviation this file used to work around. The
// interpreter read int-versus-float off the elements, and an empty list has
// none, so `[f64].sum()` was `0`: it printed without a point and then panicked
// `no method to_fixed on 0` when the caller used the `f64` the checker had
// promised. This program asserted only what both engines agreed on, so it
// passed throughout. The deviation was recorded on the grounds that fixing it
// needed the interpreter to carry element types; it needed the checker to
// mention what it already knew.
fn total(xs: [f64]) -> f64 {
    xs.sum()
}

fn main() {
    // Integers, empty and not.
    let some: [i64] = [1, 2, 3]
    let none: [i64] = []
    print("{some.sum()} {none.sum()}")

    // Floats with elements are unambiguous in both engines.
    let fs: [f64] = [1.0, 2.0, 3.0]
    print("{fs.sum()} {fs.filter(|x| x < 3.5).sum() - 3.0}")

    // A filter that removes everything still has the element type.
    let gone = some.filter(|x| x > 99)
    print("{gone.sum()}")

    // An empty float list, directly and through a function that declares the
    // return type — the shape that used to panic.
    let empty: [f64] = []
    print("{empty.sum()} {total(empty).to_fixed(4)}")
    // And an emptied one, where the element type comes from the source list.
    print("{fs.filter(|x| x > 99.0).sum()}")
}
```

## `L-LIB-TO-STR.gr`

```grasm
//! tag: L-LIB-TO-STR
//! expect: output
//! 1.0-2.5-0.0-1000000000000000.0
//! 0.0 3.0 0.0
//! 7 -12
// `to_str` gives exactly what `print` gives.
//
// The interpreter handed the job to its host language, which drops the point
// on a whole float. So `(1.0).to_str()` was `"1"` under `grasm run` and `"1.0"`
// compiled, and the documented idiom below printed a `[f64]` as a list of
// integers — the one thing [L-DISPLAY] exists to prevent, in the engine this
// specification calls normative.
fn main() {
    let xs: [f64] = [1.0, 2.5, 0.0, 1.0e15]
    print("{xs.map(|v| v.to_str()).join("-")}")
    // Through a conversion, and through a sum with nothing to add.
    let empty: [f64] = []
    print("{(0 - 0.0).to_str()} {(3).as_f64().to_str()} {empty.sum().to_str()}")
    // An integer is still an integer.
    print("{(7).to_str()} {(0 - 12).to_str()}")
}
```

## `L-LIB-TRIM-SIDES.gr`

```grasm
//! tag: L-LIB-TRIM-SIDES
//! expect: output
//! [  hi] [hi  ] [hi]
//! [    let x = 1]
// Trimming one side. `trim` alone loses the distinction, and a line's leading
// indentation is often exactly what you are trying to keep.
fn main() {
    let s = "  hi  "
    print("[{s.trim_end()}] [{s.trim_start()}] [{s.trim()}]")
    // The case it exists for: dropping the trailing newline and spaces a file
    // leaves on a line without flattening the indentation that carries meaning.
    let line = "    let x = 1   "
    print("[{line.trim_end()}]")
}
```

## `L-LIB-TUPLE.gr`

```grasm
//! tag: L-LIB-TUPLE
//! expect: output
//! 1 2 3 4
//! 1.5 1000.0 10
// Tuple components, including nested ones.
//
// A number directly after `.` is a component, not the start of a float:
// `t.1.1.0` used to lex as `t` `.` `1.1` `.` `0` and report "expected an
// identifier but found float `1.1`", so a nested component could only be
// written with brackets. Ordinary floats and ranges are unaffected.
fn main() {
    let t = (1, (2, (3, 4)))
    print("{t.0} {t.1.0} {t.1.1.0} {t.1.1.1}")
    let f = 1.5
    let g = 1.0e3
    var n = 0
    for i in 1..5 {
        n = n + i
    }
    print("{f} {g} {n}")
}
```

## `L-LIB-UDP.gr`

```grasm
//! tag: L-LIB-UDP
//! expect: output
//! bound two sockets
//! sent 5 bytes
//! ready: 1 of 1
//! got 5 bytes: 1 2 255 0 65
//! a stream operation on a datagram socket is an Err
//! a datagram operation on a listener is an Err
//! nothing more arrived
// Datagrams: whole messages, to an address named at send time, carrying bytes.
//
// Both sockets ask for port 0 and read the choice back with `net_port`, so the
// program runs on a machine where the ports it would otherwise have picked are
// already held.
fn spell(b: bytes) -> str {
    var out = ""
    for i in 0..b.len() {
        out = out + "{b.at(i)} "
    }
    out.trim()
}

fn main() / {io, net} {
    match udp_bind(0) {
        Err(e) -> print("could not bind")
        Ok(server) -> {
            match udp_bind(0) {
                Err(e) -> print("could not bind a second time")
                Ok(client) -> {
                    print("bound two sockets")
                    let port = net_port(server) ?? 0

                    // A packed binary payload, which is the reason the payload
                    // type is `bytes` and not `str`: 255 and 0 are not text and
                    // a lossy UTF-8 conversion would not return them.
                    let msg = bytes_of([1, 2, 255, 0, 65])
                    match udp_send(client, "127.0.0.1", port, msg) {
                        Err(e) -> print("could not send")
                        Ok(n) -> print("sent {n} bytes")
                    }

                    // Readiness works on a datagram handle exactly as it does
                    // on a stream one, because they share the table.
                    let ready = net_wait([server], 1000) ?? []
                    print("ready: {ready.len()} of 1")

                    match udp_recv(server) {
                        Err(e) -> print("could not receive")
                        Ok(got) -> print("got {got.len()} bytes: {spell(got)}")
                    }

                    // The kinds do not silently substitute for one another.
                    match tcp_read(server) {
                        Ok(_) -> print("a stream read on a datagram socket answered, which it must not")
                        Err(e) -> print("a stream operation on a datagram socket is an Err")
                    }
                    match tcp_listen(0) {
                        Err(e) -> print("could not listen")
                        Ok(listener) -> {
                            match udp_recv(listener) {
                                Ok(_) -> print("a datagram read on a listener answered, which it must not")
                                Err(e) -> print("a datagram operation on a listener is an Err")
                            }
                            net_close(listener)
                        }
                    }

                    // One datagram was sent and one was received, so nothing is
                    // left: a second `net_wait` finds the socket quiet.
                    let again = net_wait([server], 100) ?? []
                    if again.len() == 0 {
                        print("nothing more arrived")
                    } else {
                        print("something arrived that was never sent")
                    }
                    net_close(client)
                }
            }
            net_close(server)
        }
    }
}
```

## `L-LIB-WORDS.gr`

```grasm
//! tag: L-LIB-WORDS
//! expect: output
//! [2026-08-09T10:00:00Z, INFO, server, started]
//! 10 4
//! [] [] [a, b, c]
//! [a, , b] [a, b]
fn main() {
    // A line of aligned columns: what `split(" ")` cannot read.
    let line = "2026-08-09T10:00:00Z INFO     server   started"
    print("{line.words()}")
    print("{line.split(" ").len()} {line.words().len()}")
    // Empty, all-whitespace, and mixed whitespace kinds.
    print("{"".words()} {"   ".words()} {"\ta\nb  c ".words()}")
    // The two are different on purpose: `split` keeps the empty, `words` does
    // not, and a CSV needs the first while a log line needs the second.
    print("{"a,,b".split(",")} {"a  b".words()}")
}
```

## `L-MAXMUNCH.gr`

```grasm
//! tag: L-MAXMUNCH
//! expect: output
//! true
fn f() -> i64 {
    1
}
fn main() {
    // `->` is one token; `<=` is one token.
    print("{f() <= 1}")
}
```

## `L-NEWLINE.gr`

```grasm
//! tag: L-NEWLINE
//! expect: output
//! 6
fn main() {
    let a = 1


    let b = 2
    // Newlines are insignificant inside ( ) and [ ].
    let xs = [
        a,
        b,
        3,
    ]
    print("{xs.sum()}")
}
```

## `L-RAW-STRING.gr`

```grasm
//! tag: L-RAW-STRING
//! expect: output
//! 3
//! [{"users": [{"name": "ada"}], "ok": true}]
//! [a	tab and "quotes" kept]
//! [a \n that is two characters]
//! 4 [97, 92, 110, 98]
//! interpolation still works: 2
//! empty:[] one:[x]
// A raw block: every character literal, newlines included.
let DOC = """
{"users": [{"name": "ada"}], "ok": true}
a	tab and "quotes" kept
a \n that is two characters
"""

let ESCAPES = """a\nb"""
let EMPTY = """"""
let ONE = """x"""

fn main() {
    print("{DOC.lines().len()}")
    for l in DOC.lines() {
        print("[{l}]")
    }
    // `\n` inside a raw block is a backslash and an `n`, not a newline.
    print("{ESCAPES.len()} {ESCAPES.codes()}")
    // Ordinary strings are unaffected.
    print("interpolation still works: {1 + 1}")
    print("empty:[{EMPTY}] one:[{ONE}]")
}
```

## `L-STRING.gr`

```grasm
//! tag: L-STRING
//! expect: error
//! unterminated string
fn main() {
    let s = "no closing quote
}
```

## `L-UFCS.gr`

```grasm
//! tag: L-UFCS
//! expect: output
//! 10
//! 10
//! 7
//! 6
//! 3
// A free function whose first parameter accepts the receiver may be called in
// method position. The two spellings are the same call.
fn double(n: i64) -> i64 {
    n * 2
}

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn total(xs: [i64]) -> i64 {
    xs.sum()
}

/// A user function named `len` does not shadow the built-in `len` method:
/// built-ins are resolved first.
fn len(s: str) -> i64 {
    999
}

fn main() {
    print("{double(5)}")
    print("{5.double()}")
    print("{3.add(4)}")
    print("{[1, 2, 3].total()}")
    print("{"abc".len()}")
}
```

## `L-WHITESPACE.gr`

```grasm
//! tag: L-WHITESPACE
//! expect: output
//! 3
fn main() {
	let    a	=	1
	let  b  =  2
	print("{a+b}")
}
```

## `T-ALIAS-NOT-RECURSIVE.gr`

```grasm
//! tag: T-ALIAS-NOT-RECURSIVE
//! expect: error
//! type `Loop` is defined in terms of itself
// An alias is another spelling of the type it names, so it has nowhere to
// stop.
//
// Accepted at the declaration, it was then walked recursively by every pass
// that reads a written type: the process aborted with a Rust stack overflow,
// exit 134 and no diagnostic at all — and a dependency carrying the line took
// its consumer's toolchain down with it, after `grasm deps` had printed a
// partial success line. [G-NEST-LIMIT]: exceeding a cap is a diagnostic,
// never a crash.
//
// A struct or a sum may refer to itself ([T-RECURSIVE]), because a value of
// one is built out of *other* values — `Tree` below is legal and compiles.
type Loop = [Loop]

type Tree = | Node(kids: [Tree]) | Leaf

fn main() {
    print("{Tree.Leaf}")
}
```

## `T-ARM-TYPES.gr`

```grasm
//! tag: T-ARM-TYPES
//! expect: output
//! 3.0
//! 2.0
//! ok
// Every arm of a `match` used as a value produces the same type.
//
// This program is here because a `match` whose arms the backend could not
// type did not fail — inside a fallible function it compiled, and then
// panicked at run time with `unbox: not an f64` and no source location.
// Wrong code, from arms that were perfectly well typed by the checker and
// merely unknown to the backend's own inference table.
type Diag = { msg: str }

fn call(name: str, a: f64, b: f64) -> f64 ! Diag {
    match name {
        "sqrt" -> a.sqrt()
        "min" -> a.min(b)
        _ -> fail(Diag{ msg: "unknown function {name}" })
    }
}

fn show(name: str, a: f64, b: f64) {
    match call(name, a, b) {
        Ok(v) -> print("{v}")
        Err(d) -> print("{d.msg}")
    }
}

fn main() {
    show("sqrt", 9.0, 0.0)
    show("min", 9.0, 2.0)
    print("ok")
}
```

## `T-CAPABILITY.gr`

```grasm
//! tag: T-CAPABILITY
//! expect: error
//! performs effect(s) `io` not in its declared row
// A capability ceiling is a statement about what escapes a package.
//
// The single-file half of the rule is the effect row: what a function
// performs is inferred from its source, not declared by hand, so a function
// that quietly starts printing is a compile error rather than a reviewer's
// problem. `crates/grasm/tests/cli_tests.rs` holds the package half, which
// needs a manifest and a consumer and so cannot be one file.
fn quiet() -> i64 / {} {
    print("this escapes")
    1
}

fn main() {
    print("{quiet()}")
}
```

## `T-CAPTURE-WRITE.gr`

```grasm
//! tag: T-CAPTURE-WRITE
//! expect: error
//! captured from outside
// A closure captures values, not bindings ([E-CLOSURE-CAPTURE]), so a write
// to a captured name changes a copy that is discarded when the call returns.
// It used to compile and silently do nothing — which is how a handler that
// appears to accumulate state accumulates none.
fn main() {
    var seen: [i64] = []
    let remember = |v| seen.push(v)
    remember(1)
    remember(2)
    print("{seen.len()}")
}
```

## `T-CONST-PUB.gr`

```grasm
//! tag: T-CONST-PUB
//! expect: output
//! 42 84
// `pub let` exports a constant. A bare `let` keeps it to its own module.
//
// `pub` on a constant used to parse, check, and do nothing at all: no other
// module could read one, `doc` omitted it and `api` omitted it. Syntax with no
// meaning is worse than an error, because it looks like it worked.
pub let WIDTH = 42
let SCALE = 2

pub fn scaled() -> i64 {
    WIDTH * SCALE
}

fn main() {
    print("{WIDTH} {scaled()}")
}
```

## `T-COPY.gr`

```grasm
//! tag: T-COPY
//! expect: output
//! 6 6
fn consume(own n: i64) -> i64 {
    n * 2
}
fn main() {
    let n = 3
    // Scalars copy, so `n` is still usable after an `own` call.
    print("{consume(n)} {consume(n)}")
}
```

## `T-DIVERGE.gr`

```grasm
//! tag: T-DIVERGE
//! expect: output
//! 4 -1
//! pos non-pos b c
//! -2 0
//! 3
//! 7
// `return`, `break` and `continue` leave before a value exists, so they fit
// wherever an expression is expected. Reading a returning arm as `()` used to
// put the type error on whichever *other* arm did produce a value.
type S = | A(v: i64) | B | C(s: str)

fn classify(s: S) -> str {
    match s {
        A(v) -> if v > 0 { "pos" } else { return "non-pos" }
        B -> return "b"
        C(t) -> t
    }
}

/// `continue` as an arm body, inside a loop.
fn first_negative(xs: [i64]) -> i64 {
    for x in xs {
        match x {
            0 -> continue
            _ -> if x < 0 { return x }
        }
    }
    0
}

/// `break` as an arm body.
fn sum_until_nine(xs: [i64]) -> i64 {
    var total = 0
    for x in xs {
        match x {
            9 -> break
            _ -> total = total + x
        }
    }
    total
}

/// `fail` already had this rule; the three now share it.
fn checked(n: i64) -> i64 ! S {
    match n {
        0 -> fail(S.B)
        _ -> n
    }
}

/// `fail` is an expression and may sit on the right of `??`; `return` is a
/// statement and may not ([E-COALESCE-DIVERGE]).
fn from_option(o: Option[i64]) -> i64 ! S {
    o ?? fail(S.B)
}

fn main() {
    print("{from_option(Some(4)) ?? 0} {from_option(None) ?? 0 - 1}")
    print("{classify(S.A(1))} {classify(S.A(-1))} {classify(S.B)} {classify(S.C("c"))}")
    print("{first_negative([0, 3, -2, 5])} {first_negative([1, 2])}")
    print("{sum_until_nine([1, 2, 9, 100])}")
    print("{checked(7) ?? 0}")
}
```

## `T-EFFECT-BUILTIN.gr`

```grasm
//! tag: T-EFFECT-BUILTIN
//! expect: output
//! io and async are built in
fn writes() / {io} {
    print("io and async are built in")
}
fn concurrent() / {async} {
    parallel {
        spawn {
            let x = 1
        }
    }
}
fn main() {
    writes()
    concurrent()
}
```

## `T-EFFECT-DECL.gr`

```grasm
//! tag: T-EFFECT-DECL
//! expect: output
//! 99
//! I: starting
//! W: disk almost full
//! a=1
//! b=2
//! 3
//! 42 9 0
//! 8
// Declaring `Cfg` also declares the handler type `CfgHandler`, whose fields
// are the operations.
effect Cfg {
    fn value() -> i64
}

// Operations take parameters, and more than one, and may return nothing.
// A handler closure receives them in declaration order.
effect Log {
    fn info(msg: str)
    fn warn(msg: str)
}

effect Kv {
    fn put(key: str, value: i64)
    fn size() -> i64
}

fn main() {
    with Cfg handled_by CfgHandler{ value: || 99 } {
        print("{Cfg.value()}")
    }

    // A handler body may itself perform effects — here `io` — which is the
    // shape almost every real handler has and the one worth demonstrating.
    with Log handled_by LogHandler{ info: |m| print("I: {m}"), warn: |m| print("W: {m}") } {
        Log.info("starting")
        Log.warn("disk almost full")
    }

    // A handler bound to a name first, rather than written at `handled_by`.
    let store = KvHandler{ put: |k, v| print("{k}={v}"), size: || 3 }
    with Kv handled_by store {
        Kv.put("a", 1)
        Kv.put("b", 2)
        print("{Kv.size()}")
    }

    // A handler that reads its enclosing scope. This is most of what a
    // handler is for — injecting a table, a configuration, a fake clock —
    // and it used to compile only under the interpreter.
    var table = map()
    table.set("x", 42)
    table.set("y", 9)
    with Kv handled_by KvHandler{ put: |k, v| print("{k}={v}"), size: || table.len() } {
        print("{table.get("x") ?? 0} {table.get("y") ?? 0} {table.get("z") ?? 0}")
    }

    // The captured value is read at the moment the handler is installed, and
    // an operation may be reached from any depth while the block is open.
    let scale = 4
    with Cfg handled_by CfgHandler{ value: || scale * 2 } {
        print("{doubled()}")
    }
}

fn doubled() -> i64 {
    Cfg.value()
}
```

## `T-EFFECT-DISCHARGE.gr`

```grasm
//! tag: T-EFFECT-DISCHARGE
//! expect: error
//! unhandled effect
effect Clock {
    fn now() -> i64
}
fn main() {
    print("{Clock.now()}")
}
```

## `T-EFFECT-FALLIBLE-CONSUME.gr`

```grasm
//! tag: T-EFFECT-FALLIBLE-CONSUME
//! expect: output
//! V DEFAULT
//! ok V / err no bad
//! got V / caught no bad
// A fallible effect operation answers with a `T ! E`, so all three ways of
// consuming one work — not only `try`.
//
// A handler is an ordinary closure, so calling one produced a bare value and a
// `fail` inside it propagated straight past the caller. `??` fell through to
// nothing and `match` panicked with "no match arm matched `V`" on the
// *success* path. Only `try` worked, which is the one form the original
// conformance program used.
type A = { a: str }

effect Src {
    fn fetch(k: str) -> str ! A
}

fn with_default(k: str) -> str / {Src} {
    Src.fetch(k) ?? "DEFAULT"
}

fn matched(k: str) -> str / {Src} {
    match Src.fetch(k) {
        Ok(v) -> "ok {v}"
        Err(e) -> "err {e.a}"
    }
}

fn propagated(k: str) -> str ! A / {Src} {
    let v = try Src.fetch(k)
    "got {v}"
}

fn main() {
    with Src handled_by SrcHandler{ fetch: |k| if k == "good" { "V" } else { fail(A{ a: "no {k}" }) } } {
        print("{with_default("good")} {with_default("bad")}")
        print("{matched("good")} / {matched("bad")}")
        let a = propagated("good") ?? "?"
        let b = match propagated("bad") {
            Ok(v) -> v
            Err(e) -> "caught {e.a}"
        }
        print("{a} / {b}")
    }
}
```

## `T-EFFECT-FALLIBLE.gr`

```grasm
//! tag: T-EFFECT-FALLIBLE
//! expect: output
//! 42.0
//! err: missing y
// An effect operation that can fail.
//
// The grammar has always allowed `! E` on an operation, and the checker used
// to drop it: an error raised in a handler crossed every function on the way
// out while `grasm check` reported no errors, which defeats the rule that a
// fallible value cannot be used as if it were plain. Now the operation's
// result is fallible, so a caller must `try` it and declare an error type of
// its own — and a handler must produce one.
type Diag = { msg: str }

effect Cfg {
    fn get(name: str) -> f64 ! Diag
}

fn read(name: str) -> f64 ! Diag / {Cfg} {
    let v = try Cfg.get(name)
    v * 2.0
}

fn show(name: str) / {Cfg, io} {
    match read(name) {
        Ok(v) -> print("{v}")
        Err(d) -> print("err: {d.msg}")
    }
}

fn main() {
    // A handler body producing a plain value is producing `Ok(value)`, the
    // same rule a fallible function's body follows.
    with Cfg handled_by CfgHandler{
        get: |n| if n == "x" { 21.0 } else { fail(Diag{ msg: "missing {n}" }) },
    } {
        show("x")
        show("y")
    }
}
```

## `T-EFFECT-OP-RESULT.gr`

```grasm
//! tag: T-EFFECT-OP-RESULT
//! expect: output
//! 8
//! 16
// An operation's declared result carries its row like any other written type.
//
// [E-HANDLER-OWN-ROW] exempts the handler's own operations from the installing
// function's row. It says nothing about the value an operation hands back, and
// that was read as though it did: an operation answering
// `-> fn(i64) -> i64 / {io}` gave out a printing function nothing charged, so
// a function declaring `/ {Mk}` printed, `par_map`'s purity proof passed, and
// a package permitting only `Mk` reached `io`. The same signature written as
// an ordinary `fn` was refused, one keyword apart.
//
// A *pure* result is not charged, which is what this program is: `Mk.get()`
// answers `fn(i64) -> i64`, so `twice` really does declare everything it
// performs.
effect Mk {
    fn get() -> fn(i64) -> i64
}

fn twice(n: i64) -> i64 / {Mk} {
    let f = Mk.get()
    f(f(n))
}

fn main() {
    with Mk handled_by MkHandler{ get: || |x| x * 2 } {
        print("{twice(2)}")
        print("{twice(4)}")
    }
}
```

## `T-EFFECT-RESERVED.gr`

```grasm
//! tag: T-EFFECT-RESERVED
//! expect: error
//! effect name `io` is reserved by the language
// A capability the runtime discharges is not an effect a program handles.
//
// `io` and `async` are not keywords, so this parsed — and the
// `with io handled_by ...` it made available discharged the *builtin* `io`
// for everything in the block. This function's declared row is empty and it
// printed; the same trick let `par_map` run a printing function on several
// cores, and let a dependency declaring `capabilities = []` read and write
// files while `grasm deps` certified it as performing nothing.
effect io {
    fn nothing() -> i64
}

fn quiet(n: i64) -> i64 / {} {
    with io handled_by ioHandler{ nothing: || 1 } {
        print("a declared-empty row printed")
    }
    n
}

fn main() {
    print("{quiet(1)}")
}
```

## `T-EFFECT-ROW.gr`

```grasm
//! tag: T-EFFECT-ROW
//! expect: error
//! not in its declared row
effect Clock {
    fn now() -> i64
}
fn quiet() / {} {
    let t = Clock.now()
}
fn main() {
    with Clock handled_by ClockHandler{ now: || 1 } {
        quiet()
    }
}
```

## `T-EMPTY-LIT.gr`

```grasm
//! tag: T-EMPTY-LIT
//! expect: output
//! [1]
//! [1, 2, 3]
//! 0
//! 1
// An empty literal takes its element type from wherever one reaches it.
//
// An annotation, a field, a parameter or a return type is the direct case;
// failing all of those, a later use answers it. The compiled backend reads a
// literal's type off the literal, so an empty one had nothing to read: the
// last three of these ran under `grasm run` and could not be built. The
// checker resolves them against the whole body, and now says so.
fn add(acc: [i64], x: i64) -> [i64] {
    var out = acc
    out.push(x)
    out
}

fn main() {
    var xs = []
    xs.push(1)
    print("{xs}")
    print("{[1, 2, 3].fold([], add)}")
    let missing: Option[[str]] = None
    let got = match missing {
        Some(v) -> v
        None -> []
    }
    print("{got.len()}")
    var m = {}
    m.set("a", 1)
    print("{m.get("a") ?? 0}")
}
```

## `T-ERR-SUBSET.gr`

```grasm
//! tag: T-ERR-SUBSET
//! expect: error
//! only declares
type A = | AErr
type B = | BErr
fn wide() -> i64 ! (A | B) {
    1
}
fn narrow() -> i64 ! A {
    try wide()
}
fn main() {
    match narrow() {
        Ok(v) -> print("{v}")
        Err(e) -> print("err")
    }
}
```

## `T-ERR-UNION-OPAQUE.gr`

```grasm
//! tag: T-ERR-UNION-OPAQUE
//! expect: error
//! has no field `key`
// A union error type cannot be narrowed back to one of its members.
//
// It can be raised, propagated, caught and printed — and a function declaring
// two error types can therefore say *that* it failed and not *which way*,
// which is usually why two were declared. [T-ERR-SUBSET] promotes a caller's
// error type to this union automatically, so a program arrives here without
// asking. The way to branch on the failure today is one error type that is a
// sum. This is recorded as a gap rather than left for a reader to discover.
type NotFound = { key: str }
type Cycle = { path: [str] }

fn boom(w: i64) -> i64 ! (NotFound | Cycle) {
    fail(NotFound{ key: "k" })
}

fn main() {
    match boom(0) {
        Ok(v) -> print("{v}")
        Err(e) -> print("{e.key}")
    }
}
```

## `T-EXTERN-POINTER.gr`

```grasm
//! tag: T-EXTERN-POINTER
//! expect: output
//! strlen: 5 0 11
//! atoi: 42 -7 0
//! memcmp equal: 0
//! memcmp orders: true true
//! the length is the program's to pass: 0 true
//! a str is copied, so the original is untouched: hello 5
//! every spelling is the same call: 5 5 5 5
// A `str` or `bytes` argument to an `extern fn` crosses as a pointer.
//
// The functions used here are the ones every C library has and whose answers
// do not depend on the machine.
extern fn strlen(s: str) -> i64
extern fn atoi(s: str) -> i64
extern fn memcmp(a: bytes, b: bytes, n: i64) -> i64

fn main() / {io, unsafe} {
    unsafe {
        // A `str` crosses NUL-terminated, so C finds the end where Grasm says
        // the string ends — including for the empty one.
        print("strlen: {strlen("hello")} {strlen("")} {strlen("hello world")}")
        print("atoi: {atoi("  42abc")} {atoi("-7")} {atoi("nope")}")

        let a = bytes_of([1, 2, 3])
        let b = bytes_of([1, 2, 3])
        let c = bytes_of([1, 2, 4])
        print("memcmp equal: {memcmp(a, b, 3)}")
        print("memcmp orders: {memcmp(a, c, 3) < 0} {memcmp(c, a, 3) > 0}")

        // `bytes` crosses without a terminator and without a length, so the
        // length is an ordinary argument. Comparing zero bytes compares
        // nothing, which is how a caller can tell C is being given the count
        // rather than finding one.
        print("the length is the program's to pass: {memcmp(a, c, 0)} {memcmp(a, c, 2) == 0}")

        // The copy handed to C is a copy: the Grasm string is not modified,
        // not terminated in place, and still its own length afterwards.
        let s = "hello"
        let n = strlen(s)
        print("a str is copied, so the original is untouched: {s} {s.len()}")

        // [L-UFCS] says the method spelling is the same call as the free one,
        // and a function value is the same function. The pointer conversion
        // lives at the call site, so each of these is a place it can be
        // missed — and two of them were: the compiled build handed C the
        // boxed value's own address and `strlen` answered 1 for every string,
        // while `grasm run` answered correctly.
        print("every spelling is the same call: {strlen(s)} {s.strlen()} {s |> strlen} {[s].map(strlen)[0]}")
    }
}
```

## `T-EXTERN-VALUE.gr`

```grasm
//! tag: T-EXTERN-VALUE
//! expect: output
//! 2.0 3.0
//! [1.0, 2.0, 3.0]
// A C function named without being called.
//
// The compiled backend made one a function value and the interpreter said
// `unknown name` — for a program `check` accepted. And naming one carried no
// effect at all, so `xs.par_map(sqrt)` typechecked: the purity proof, the
// declared row and a package's capability ceiling all passed something that
// reaches libc. Holding a C function is the same permission as calling one —
// and the row that travels with it says so: `{io, unsafe}`, not just `{io}`.
// A type written `fn(f64) -> f64 / {io}` does not accept one, which is what
// stopped `sqrt` from being pushed into a list annotated that way inside an
// `unsafe` block and called back out from a function declaring nothing.
extern fn sqrt(x: f64) -> f64

fn apply(f: fn(f64) -> f64 / {io, unsafe}, v: f64) -> f64 / {io, unsafe} {
    f(v)
}

fn main() {
    unsafe {
        // Naming one is the same permission as calling one, so the *name*
        // needs `unsafe` too, not just the call.
        let f = sqrt
        print("{f(4.0)} {apply(sqrt, 9.0)}")
        print("{[1.0, 4.0, 9.0].map(sqrt)}")
    }
}
```

## `T-EXTERN.gr`

```grasm
//! tag: T-EXTERN
//! expect: output
//! 7 4.0
//! 3.0 -2
//! true
// Calling C. The compiled backend resolves the symbol at link time; the
// interpreter finds it in its own process, which already has libc loaded.
// Both make the identical call.
//
// Signatures are `i64` and `f64` only. `bool` is excluded on purpose: C's is
// one byte and Grasm's is a register, and a silent width mismatch at an FFI
// boundary shows up as garbage three calls later.
extern fn abs(n: i64) -> i64
extern fn sqrt(x: f64) -> f64
extern fn pow(base: f64, exp: f64) -> f64
extern fn getpid() -> i64

fn main() {
    // Every use is inside `unsafe` — Tier 3 ([T-UNSAFE-FFI]). Leaving the
    // safe tiers is the thing being asked for, and it is asked for in the
    // source and again in the manifest.
    let a = unsafe {
        abs(0 - 7)
    }
    let s = unsafe {
        sqrt(16.0)
    }
    print("{a} {s}")
    unsafe {
        print("{pow(3.0, 1.0)} {abs(2) - 4}")
        // A real process id, so this proves the call actually happened.
        print("{getpid() > 0}")
    }
}
```

## `T-FAIL.gr`

```grasm
//! tag: T-FAIL
//! expect: error
//! declares no error type
type E = | Bad
fn no_error_clause() -> i64 {
    fail(Bad)
}
fn main() {
    print("{no_error_clause()}")
}
```

## `T-FALLIBLE-COALESCE.gr`

```grasm
//! tag: T-FALLIBLE-COALESCE
//! expect: output
//! 42
//! 0
//! []
//! 101
//! evaluated once
// `??` over a fallible value, which used to be a four-line `match`.
type E = { m: str }

fn parse(s: str) -> i64 ! E {
    match s.parse_int() {
        Some(n) -> n
        None -> fail(E{ m: "not a number: {s}" })
    }
}

fn names() -> [str] ! E {
    fail(E{ m: "none" })
}

/// Prints when it runs, so laziness is observable rather than asserted.
fn fallback() -> i64 {
    print("evaluated once")
    -1
}

fn main() {
    print("{parse("42") ?? 0}")
    print("{parse("oops") ?? 0}")
    print("{names() ?? []}")
    print("{(parse("1") ?? 0) + (parse("x") ?? 100)}")
    // The success path does not evaluate the fallback; the failing one does.
    let _ok = parse("5") ?? fallback()
    let _bad = parse("z") ?? fallback()
}
```

## `T-FALLIBLE-UNHANDLED.gr`

```grasm
//! tag: T-FALLIBLE-UNHANDLED
//! expect: error
//! can fail
// A fallible value may not be used as if it were `T`, and discarding it is
// using it.
//
// A bare statement was checked, and the *tail* of a block whose value goes
// nowhere was not — so `if c { write_file(p, s) }` swallowed the failure that
// `write_file(p, s)` on its own line reports. Loop bodies were the same. That
// is verbatim the shape this clause exists to prevent: "wrote the report",
// exit 0, nothing on stderr, and no file.
type E = | Bad

fn risky() -> i64 ! E {
    1
}

fn risky_of(n: i64) -> i64 ! E {
    n
}

fn main() {
    let v = risky()
    // A call to a `let`-bound closure. Its return type is not known until the
    // body is checked, and the bodies wait for a call site to say what the
    // parameters are — so the check ran too early, saw an inference variable
    // rather than a fallible type, and this line printed the next one and
    // exited 0.
    let f = |x| risky_of(x)
    f(1)
    if true {
        risky()
    }
    for _n in 0..1 {
        risky()
    }
    parallel {
        spawn {
            risky()
        }
    }
}

// A test body's value goes nowhere too, so a test whose whole body is a
// failing call used to *pass*.
test "checked" {
    risky()
}
```

## `T-FALLIBLE-UNIT.gr`

```grasm
//! tag: T-FALLIBLE-UNIT
//! expect: output
//! both survived
//! err boom
//! handler ok
//! handler err
// A fallible call that produces nothing.
//
// `() ! E` is what a setup call returns, and `f() ?? ()` is how you say its
// failure is deliberately ignored. The compiled backend gave the merge block
// a parameter for a value that does not exist, which Cranelift's verifier
// rejected — surfacing as "does not yet support (codegen bug in `main`)", a
// refusal naming nothing, which then masked the real refusal elsewhere in the
// same program.
//
// The same merge inside a *handler* had no arm at all: an operation declared
// `-> () ! E` whose body gives the bare `()` half produced no value, and the
// lifted body returned a null pointer typed as a fallible. The build
// succeeded and the program died with SIGSEGV and nothing on stderr — the one
// thing §11 promises never happens.
type E = { m: str }

effect Setup {
    fn once(b: bool) -> () ! E
}

fn nothing(b: bool) -> () ! E {
    if b {
        fail(E{ m: "boom" })
    }
}

fn main() {
    nothing(false) ?? ()
    nothing(true) ?? ()
    print("both survived")
    match nothing(true) {
        Ok(_) -> print("ok")
        Err(e) -> print("err {e.m}")
    }
    with Setup handled_by SetupHandler{ once: |b| if b { fail(E{ m: "h" }) } else { () } } {
        match Setup.once(false) {
            Ok(_) -> print("handler ok")
            Err(e) -> print("handler err {e.m}")
        }
        match Setup.once(true) {
            Ok(_) -> print("handler ok2")
            Err(e) -> print("handler err")
        }
    }
}
```

## `T-FALLIBLE-VALUE.gr`

```grasm
//! tag: T-FALLIBLE-VALUE
//! expect: error
//! found `fn(i64) -> bool ! E`
// A fallible function passed where a plain predicate is wanted.
//
// This used to typecheck, because a function referenced by name lost its
// `! E`. `filter` then tested `Ok(true)`, which is not `true`, so every
// element was dropped and the program printed `[]` and exited 0.
type E = { m: str }

fn odd(n: i64) -> bool ! E {
    if n == 3 {
        fail(E{ m: "three" })
    }
    n % 2 == 1
}

fn main() {
    print("{[1, 2, 3, 4].filter(odd)}")
}
```

## `T-FALLIBLE.gr`

```grasm
//! tag: T-FALLIBLE
//! expect: output
//! ok 5
//! err Bad
//! v1
//! d
//! err
//! -1 6
type E = | Bad
fn maybe(fail_it: bool) -> i64 ! E {
    if fail_it {
        fail(Bad)
    }
    5
}
fn main() {
    match maybe(false) {
        Ok(v) -> print("ok {v}")
        Err(e) -> print("err {e}")
    }
    match maybe(true) {
        Ok(v) -> print("ok {v}")
        Err(e) -> print("err {e}")
    }
    // A closure is a function boundary like any other: a `fail` inside a
    // *fallible* closure is that closure's own failure, and an ordinary value
    // out of one is its ok half. Neither was true — the failure unwound past
    // the `??` and the `match` written to handle it, out of `main`, and the
    // compiled build took the fallback on the success path.
    let f: fn(i64) -> str ! E = |n| if n == 0 {
        fail(Bad)
    } else {
        "v{n}"
    }
    print("{f(1) ?? "d"}")
    print("{f(0) ?? "d"}")
    match f(0) {
        Ok(v) -> print("ok {v}")
        Err(_) -> print("err")
    }
    // And with no annotation at all, where each branch knows one half.
    let g = |n| if n == 0 {
        Err(Bad)
    } else {
        Ok(n * 2)
    }
    print("{g(0) ?? -1} {g(3) ?? -1}")
}
```

## `T-FIELD-CALL.gr`

```grasm
//! tag: T-FIELD-CALL
//! expect: output
//! 10
//! 8
//! 6
//! 7
// A function held in a field is called through the field.
//
// Building the value and reading the field back both worked from the start
// and the call did not, so the only spelling that ran was `let f = n.run`
// followed by `f(5)` — which nothing pointed to. The same gap was left for a
// tuple component, where the error suggested declaring `fn 0(...)`, and for a
// field reached through a reference.
type Node = { run: fn(i64) -> i64 }

type Plain = { step: fn(i64) -> i64 }

fn twice(n: i64) -> i64 {
    n * 2
}

fn step(p: Plain, x: i64) -> i64 {
    x + 2
}

fn main() {
    let n = Node{ run: twice }
    print("{n.run(5)}")
    let pair = (twice, "label")
    print("{pair.0(4)}")
    let r = ref(Node{ run: twice })
    print("{r.run(3)}")
    // A field shadows nothing: a UFCS free function of the same name wins.
    print("{Plain{ step: twice }.step(5)}")
}
```

## `T-FN-TYPE-ROW-CARRIED.gr`

```grasm
//! tag: T-FN-TYPE-ROW-CARRIED
//! expect: error
//! but this one performs `io`
// A function's effect row travels with it, into every shape.
//
// The row check ran in one place: a call argument whose parameter type was
// written inline as `fn(...)`. Every other route from a named function into a
// written function type dropped the row — a return type, a `type Stage =
// fn(..)` alias, a struct field, a list element, a map value, a tuple member.
// Each was a way to hand a function that prints to a `/ {}` row, past
// [T-PAR-MAP-PURE], past the missing-handler check [E-HANDLER-MISSING] calls a
// backstop "well-typed programs never reach", and past a package's
// `capabilities = []` ceiling — which is the strongest guarantee here.
//
// The cause was that the row was not part of the *type*: `fn(i64) -> i64` and
// `fn(i64) -> i64 / {io}` were the same type, so there was nothing to compare.
fn shout(n: i64) -> i64 {
    print("noisy")
    n
}

type Box = { f: fn(i64) -> i64 }

fn main() {
    let b = Box{ f: shout }
    print("{b.f(1)}")
}
```

## `T-FN-TYPE-ROW-PARAM.gr`

```grasm
//! tag: T-FN-TYPE-ROW-PARAM
//! expect: error
//! performs nothing, but this one performs `io`
// A row in a parameter position is checked the other way round.
//
// `call` declares that it will be given a function performing nothing.
// Checking the two rows covariantly with the result let a list of `/ {io}`
// functions through, and `par_map` then ran the effect on several cores —
// [T-PAR-MAP-PURE]'s "a compile error, not a race", reached by putting the
// row in a parameter instead of in the callback.
fn shout(n: i64) -> i64 / {io} {
    print("side effect")
    n
}
fn call(f: fn(i64) -> i64) -> i64 {
    f(1)
}
fn main() {
    let fs = [shout]
    print("{fs.par_map(call)}")
}
```

## `T-FN-TYPE-ROW.gr`

```grasm
//! tag: T-FN-TYPE-ROW
//! expect: error
//! this closure performs `io`, which the expected type does not allow
// A closure's row is part of its type, wherever the type comes from.
//
// Only the *shape* was compared when a closure literal met a written function
// type, so an effectful closure went into a `fn(i64) -> i64` — a written
// empty row — on every route except a direct call argument: a struct field,
// a reassignment, a `push`, a `set`, an index write, `Some(...)`, a tuple.
// A named function was checked everywhere, so `f: shout` was refused and
// `f: |n| shout(n)` was not — the same function, one closure apart.
type Box = { f: fn(i64) -> i64 }

fn noisy(n: i64) -> i64 {
    print("{n}")
    n
}

fn pure_apply(f: fn(i64) -> i64, x: i64) -> i64 / {} {
    f(x)
}

fn main() {
    let b = Box{ f: |n| noisy(n) }
    print("{pure_apply(b.f, 1)}")
}
```

## `T-FN-TYPE.gr`

```grasm
//! tag: T-FN-TYPE
//! expect: output
//! 10
//! 6
//! 12
//! [2, 4, 6]
// A program declaring its own higher-order function, which needs a written
// function type: there is no body to infer a parameter's type from.
fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn twice(f: fn(i64) -> i64, x: i64) -> i64 {
    f(f(x))
}

/// Takes a function and a list, which is `map` written in the language.
fn over(f: fn(i64) -> i64, xs: [i64]) -> [i64] {
    xs.map(f)
}

fn double(n: i64) -> i64 {
    n * 2
}

fn main() {
    print("{apply(double, 5)}")
    // A closure takes its parameter types from the parameter it is passed to.
    print("{apply(|n| n + 1, 5)}")
    print("{twice(double, 3)}")
    print("{over(double, [1, 2, 3])}")
}
```

## `T-FN-VALUE-ROW.gr`

```grasm
//! tag: T-FN-VALUE-ROW
//! expect: error
//! function `quiet` performs effect(s) `io` not in its declared row
//! function `looped` performs effect(s) `io` not in its declared row
//! function `aliased` performs effect(s) `io` not in its declared row
// Naming a function carries its effect row, calling it or not.
//
// A built-in higher-order method's function parameter is deliberately
// unconstrained — `xs.map(|x| print(x))` is ordinary — so a *named* effectful
// function handed to one had no row to be checked against, and
// `[1, 2].map(shout)` inside a `/ {}` function typechecked and printed. One
// hop further, `let fs = [uses]` then `fs.map(call)`, defeated even the
// hand-written purity check on `par_map`: the effectful function is never
// called in the source, only mentioned, and something else calls it.
//
// The rule is the one already applied to a declared C function — holding one
// is the permission to call it. It is conservative, and it is what keeps
// `/ {}`, [T-PAR-MAP-PURE], [E-HANDLER-MISSING] and a package's
// `capabilities = []` from being one indirection away from meaningless.
fn shout(n: i64) -> i64 {
    print("noisy {n}")
    n
}

fn quiet() -> [i64] / {} {
    [1, 2].map(shout)
}

// And through every binder, not only `let`. `let f = fs[0]` was charged and
// `for f in fs` was not — the same program one keyword apart — which is a
// `for` loop away from a printing function running inside a `/ {}` function,
// on `par_map`'s workers, and in a package declaring `capabilities = []`.
type Cb = fn(i64) -> i64 / {io}

fn looped(fs: [fn(i64) -> i64 / {io}]) -> i64 / {} {
    for f in fs {
        return f(5)
    }
    0
}

// A `type` name counts as the type it stands for ([T-TYPE-ALIAS]). This is
// read off written syntax, and a row behind a name was not there: `f: Cb`
// recorded an *empty* row, which then shadowed every fallback.
fn aliased(f: Cb, x: i64) -> i64 / {} {
    f(x)
}

// An element type that declares no effects is charged nothing, whatever some
// other declaration in the file says.
fn dbl(n: i64) -> i64 {
    n * 2
}

fn pure_sum(fs: [fn(i64) -> i64]) -> i64 / {} {
    var t = 0
    for f in fs {
        t = t + f(2)
    }
    t
}

fn main() {
    print("{quiet()}")
    print("{looped([shout])}")
    print("{aliased(shout, 5)}")
    print("{pure_sum([dbl])}")
}
```

## `T-FN-VALUE.gr`

```grasm
//! tag: T-FN-VALUE
//! expect: output
//! 8
//! 12
//! 10 15
//! held in a struct, a map and a tuple: 8 12 15
//! returned from a function, and one built from another: 7 2
//! captured by value, so a later write is not seen: 1
//! -3 -1
fn double(n: i64) -> i64 {
    n * 2
}

fn triple(n: i64) -> i64 {
    n * 3
}

type Holder = { f: fn(i64) -> i64 }

fn adder(k: i64) -> fn(i64) -> i64 {
    |x| x + k
}

fn compose(f: fn(i64) -> i64) -> fn(i64) -> i64 {
    |x| f(f(x))
}

fn main() {
    // A function's name used as a value; called through the binding.
    let f = double
    print("{f(4)}")

    // A closure bound to a name. Its parameter type comes from the call.
    let times3 = |x| x * 3
    print("{times3(4)}")

    // Both kinds share one representation, so a list holds either.
    let stages = [double, triple]
    var out = ""
    for stage in stages {
        out = out + "{stage(5)} "
    }
    print(out.trim())

    // One representation means a function value goes anywhere a value goes,
    // not only into a list.
    let boxed = Holder{ f: double }
    var table: Map[str, fn(i64) -> i64] = map()
    table.set("t", triple)
    let pair = (triple, 0)
    let from_map = table.get("t") ?? double
    let from_pair = pair.0
    print("held in a struct, a map and a tuple: {boxed.f(4)} {from_map(4)} {from_pair(5)}")

    // A closure made by a function outlives the call that made it, and one
    // closure may capture another.
    let plus3 = adder(3)
    let twice_plus1 = compose(adder(1))
    print("returned from a function, and one built from another: {plus3(4)} {twice_plus1(0)}")

    // Capture is by value and at the moment of creation, so a later write to
    // the binding is not seen — the rule [E-CLOSURE-CAPTURE] states for a
    // handler's operations, holding for an ordinary closure too.
    var seen = 1
    let snapshot = |x| x + seen
    seen = 100
    print("captured by value, so a later write is not seen: {snapshot(0)}")

    // Negative literals are written directly — no need for `0 - n`.
    let below = -3
    print("{below} {[1, 2].get(9) ?? -1}")
}
```

## `T-GENERIC-FN.gr`

```grasm
//! tag: T-GENERIC-FN
//! expect: output
//! 1 a
fn first[T](xs: [T]) -> Option[T] {
    xs.get(0)
}
fn main() {
    print("{first([1, 2]) ?? 0} {first(["a"]) ?? "none"}")
}
```

## `T-GENERIC-INST.gr`

```grasm
//! tag: T-GENERIC-INST
//! expect: output
//! 2 2
fn count[T](xs: [T]) -> i64 {
    xs.len()
}
fn main() {
    // One generic body, two instantiations.
    print("{count([1, 2])} {count(["a", "b"])}")
}
```

## `T-INFER-LOCAL.gr`

```grasm
//! tag: T-INFER-LOCAL
//! expect: output
//! 3 2.5 hi true
fn main() {
    // No annotations: each type comes from the initializer.
    let i = 3
    let f = 2.5
    let s = "hi"
    let b = i > 1
    print("{i} {f} {s} {b}")
}
```

## `T-INOUT-EXCLUSIVE.gr`

```grasm
//! tag: T-INOUT-EXCLUSIVE
//! expect: error
//! passed as `inout` twice
fn both(inout a: [i64], inout b: [i64]) {
    a.push(1)
}
fn main() {
    var xs = [1]
    both(xs, xs)
}
```

## `T-INOUT-MUTABLE.gr`

```grasm
//! tag: T-INOUT-MUTABLE
//! expect: error
//! it is immutable
fn bump(inout n: i64) {
    n += 1
}
fn main() {
    let n = 1
    bump(n)
}
```

## `T-INOUT-PLACE.gr`

```grasm
//! tag: T-INOUT-PLACE
//! expect: error
//! must be a variable
fn bump(inout n: i64) {
    n += 1
}
fn main() {
    bump(41)
}
```

## `T-INOUT-REF.gr`

```grasm
//! tag: T-INOUT-REF
//! expect: error
//! cannot pass `r` as `inout` — it is reached through a `ref`
// Two sound ways of mutating that cannot be combined.
//
// `inout` snapshots the place at the call and writes it back at the return. A
// `ref` reaches the same value without that snapshot, so the writeback wins
// and the write made through the reference vanishes — silently, and the same
// way whichever order the two happen in. Exclusivity did not catch it: that
// rule compares binding names, and `r.xs` and `r` are two names for one slot.
type B = { xs: [i64] }

fn f(inout a: [i64], r: ref B) {
    a.push(1)
    r.xs.push(2)
}

fn main() {
    var r = ref(B{ xs: [0] })
    f(r.xs, r)
    print("{r.xs}")
}
```

## `T-INT-ALIAS.gr`

```grasm
//! tag: T-INT-ALIAS
//! expect: output
//! 3 5.0
fn add(a: i32, b: u8) -> int {
    a + b
}
fn main() {
    print("{add(1, 2)} {5.0}")
}
```

## `T-INT-TO-FLOAT-CARRIED.gr`

```grasm
//! tag: T-INT-TO-FLOAT-CARRIED
//! expect: output
//! [1.5, 3.0] 3.0
//! true true
//! 7.0 4.0 -2.0
//! 3.0 true
//! 1.5 3.0
//! 3.0 [1.0, 2.0] 5.0 3.0 3.0 1.0 2.0 2.0
//! [1.0, 2.5] [2.5, 1.0] 1.0 2.0
// An integer literal in an `f64` position is a float *everywhere*.
//
// [T-INT-TO-FLOAT] says the conversion happens at compile time, and only the
// checker knows which literals are affected. Neither engine was told, so each
// converted where it happened to remember to. The interpreter — the normative
// one — kept `xs.push(3)` into a `[f64]` as an integer, and that list then
// sorted wrong, compared unequal to the same list written `3.0`, and printed
// with no point. The compiled backend forgot elsewhere: a `Map[str, f64]`
// value panicked with `unbox: not an f64`, and `contains(3)` answered `false`
// to a comparison that was true.
type P = { v: f64 }

fn arg(v: f64) -> f64 {
    v
}

fn main() {
    var xs: [f64] = []
    xs.push(3)
    xs.push(1.5)
    xs.sort()
    print("{xs} {xs.max() ?? 0.0}")
    // Structural equality has to see the same value on both sides.
    let ys: [f64] = [1.0]
    var zs: [f64] = []
    zs.push(1)
    print("{P{ v: 1 } == P{ v: 1.0 }} {ys == zs}")
    // A parameter, a map value, a negated literal.
    var m: Map[str, f64] = {}
    m.set("a", 4)
    print("{arg(7)} {m.get("a") ?? 0.0} {arg(-2)}")
    // Inferred from a sibling element rather than from an annotation.
    let x = 1.5
    print("{[x, 3].max() ?? 0.0} {[x, 3, 2, -4].contains(3)}")
    // A method argument. The checker accepted this and the interpreter
    // panicked with `min expects a float`.
    print("{x.min(3)} {x.max(3)}")
    // Every shape that *carries* an expected type inward. Each of these was
    // once a type error about a program this clause says is well typed, and
    // the tail-expression one was the sharpest: `fn tail() -> f64 { 3 }` was
    // refused while `return 3` — the same value, one keyword apart — was
    // accepted and printed `3.0`.
    let list: [f64] = [1, 2]
    let tup: (f64, str) = (5, "a")
    let opt: Option[f64] = Some(3)
    let fal: f64 ! Bad = Ok(3)
    let mp: Map[str, f64] = {"k": 1}
    let matched: f64 = match 1 {
        1 -> 2
        _ -> 3.5
    }
    let branch: f64 = if true { 2 } else { 3.5 }
    print("{tail()} {list} {tup.0} {opt ?? 0.0} {fal ?? 0.0} {mp.get("k") ?? 0.0} {matched} {branch}")
    // Order does not matter: the literal adopts whichever side is a float,
    // whether it comes first or second.
    let first: Map[str, f64] = {"a": 1, "b": 2.5}
    let arms: f64 = match 1 {
        1 -> 2
        _ -> 3.5
    }
    print("{[1, 2.5]} {[2.5, 1]} {first.get("a") ?? 0.0} {arms}")
}

type Bad = | Bad

fn tail() -> f64 {
    3
}
```

## `T-INT-TO-FLOAT.gr`

```grasm
//! tag: T-INT-TO-FLOAT
//! expect: output
//! 3.0 6.0
fn main() {
    let x: f64 = 3
    print("{x} {x * 2}")
}
```

## `T-LET-IMMUTABLE.gr`

```grasm
//! tag: T-LET-IMMUTABLE
//! expect: error
//! it is immutable
fn main() {
    let x = 1
    x = 2
}
```

## `T-LIST.gr`

```grasm
//! tag: T-LIST
//! expect: output
//! [1, 2, 3] 3 1
fn main() {
    var xs: [i64] = [1, 2]
    xs.push(3)
    print("{xs} {xs.len()} {xs[0]}")
}
```

## `T-MAP-KEY-GENERIC.gr`

```grasm
//! tag: T-MAP-KEY-GENERIC
//! expect: error
//! map keys must be `i64`, `str`, or `bool`, not `f64`
// A type parameter used as a map key is checked at the call site.
//
// Only the parameters were checked, and this function puts `K` nowhere in its
// parameters — so `fresh(2.5)` typechecked and panicked with `invalid map key`
// at run time. The result is part of the instantiated signature too.
fn fresh[K](k: K) -> Map[K, i64] {
    var m: Map[K, i64] = {}
    m.set(k, 1)
    m
}

// And a parameter that reaches a map only *inside* a body, named in no part
// of the signature: `tally` returns an `i64` and takes a `[T]`, so nothing in
// what it declares says `T` is a key. It was the last shape where a checked
// program still panicked for a type error.
fn tally[T](xs: [T]) -> i64 {
    var m: Map[T, i64] = {}
    for x in xs {
        m.set(x, 1)
    }
    m.len()
}

fn main() {
    print("{fresh(2.5)}")
    print("{tally([1.5, 2.5])}")
}
```

## `T-MAP-KEY.gr`

```grasm
//! tag: T-MAP-KEY
//! expect: error
//! map keys must be
fn main() {
    var m: Map[[i64], str] = map()
}
```

## `T-MAP.gr`

```grasm
//! tag: T-MAP
//! expect: output
//! 2 7 true
fn main() {
    var m: Map[str, i64] = map()
    m.set("a", 7)
    m.set("b", 8)
    print("{m.len()} {m.get("a") ?? 0} {m.has("b")}")
}
```

## `T-MATCH-ALT.gr`

```grasm
//! tag: T-MATCH-ALT
//! expect: output
//! 1 2 0
//! 3 4 3
//! true false
// An arm listing alternatives, which covers every one of them.
//
// Without this, grouping a handful of literals meant an arm each — the kind
// of tax that pushes people to match on strings and throw away the sum type
// they were given.
type Shape =
    | Circle
    | Square
    | Tri

fn arity(name: str) -> i64 {
    match name {
        "sqrt" | "abs" | "round" -> 1
        "min" | "max" -> 2
        _ -> 0
    }
}

/// Alternatives count toward exhaustiveness: no `_` arm is needed here.
fn sides(s: Shape) -> i64 {
    match s {
        Circle | Tri -> 3
        Square -> 4
    }
}

fn small(n: i64) -> bool {
    match n {
        0 | 1 | 2 -> true
        _ -> false
    }
}

fn main() {
    print("{arity("abs")} {arity("min")} {arity("nope")}")
    print("{sides(Circle)} {sides(Square)} {sides(Tri)}")
    print("{small(1)} {small(9)}")
}
```

## `T-MATCH-EXHAUSTIVE.gr`

```grasm
//! tag: T-MATCH-EXHAUSTIVE
//! expect: error
//! not exhaustive
type S = | A | B | C
fn f(s: S) -> i64 {
    match s {
        A -> 1
        B -> 2
    }
}
fn main() {
    print("{f(A)}")
}
```

## `T-MATCH-GUARD.gr`

```grasm
//! tag: T-MATCH-GUARD
//! expect: error
//! not exhaustive
fn f(b: bool) -> i64 {
    match b {
        true -> 1
        false if b -> 0
    }
}
fn main() {
    print("{f(true)}")
}
```

## `T-MATCH-NESTED-COVER.gr`

```grasm
//! tag: T-MATCH-NESTED-COVER
//! expect: output
//! wrapped b plain none a
// The other side of [T-MATCH-NESTED]: payload patterns that between them cover
// the payload type do cover the variant, so the stricter rule does not push
// anyone toward a `_` that would hide the next variant added.
type Inner = | A | B
type Outer = | Wrap(i: Inner) | Plain

fn describe(o: Outer) -> str {
    match o {
        Wrap(A) -> "wrapped a"
        Wrap(B) -> "wrapped b"
        Plain -> "plain"
    }
}

fn opt(o: Option[Inner]) -> str {
    match o {
        Some(A) -> "a"
        Some(B) -> "b"
        None -> "none"
    }
}

fn main() {
    // And a binding payload still covers the variant outright.
    let bound = match Plain {
        Wrap(i) -> "w"
        Plain -> "plain"
    }
    print("{describe(Wrap(B))} {bound} {opt(None)} {opt(Some(A))}")
}
```

## `T-MATCH-NESTED.gr`

```grasm
//! tag: T-MATCH-NESTED
//! expect: error
//! match is not exhaustive — missing `Wrap`
// A payload pattern does not cover the whole variant.
//
// `Wrap(A)` matches only the `Wrap`s carrying `A`, and it was recorded as
// covering every `Wrap`. So this compiled and then panicked on `Wrap(B)` — the
// runtime surprise [T-MATCH-EXHAUSTIVE] exists to prevent, and the one the
// tutorial advertises against by name. The same held for an integer payload,
// an `Option` payload and an `Ok`/`Err` payload.
//
// `Wrap(A)` and `Wrap(B)` in two arms still cover `Wrap`, because between them
// they cover `Inner` — refusing that would push people to a `_` catch-all,
// which hides the next variant anyone adds.
type Inner = | A | B
type Outer = | Wrap(i: Inner) | Plain

fn describe(o: Outer) -> str {
    match o {
        Wrap(A) -> "wrapped a"
        Plain -> "plain"
    }
}

fn main() {
    print("{describe(Wrap(B))}")
}
```

## `T-MODULE-DEP.gr`

```grasm
//! tag: T-MODULE-DEP
//! expect: error
//! cannot load module
// A bare name is tried as a sibling source file first, then as a dependency
// the manifest declares. Here it is neither: there is no `undeclared_package.gr`
// beside this file and no `grasm.toml` naming it, so it does not resolve and
// the error names both places it looked.
//
// (A bare name *does* resolve when either one exists — `import helper` finds
// `helper.gr` next to the importing file with no manifest involved.)
import undeclared_package

fn main() {
    print("{undeclared_package.f()}")
}
```

## `T-MODULE-NS.gr`

```grasm
//! tag: T-MODULE-NS
//! expect: error
//! duplicate type name
type Thing = { a: i64 }
type Thing = { b: i64 }
fn main() {
}
```

## `T-MODULE-PUB.gr`

```grasm
//! tag: T-MODULE-PUB
//! expect: output
//! 4
fn double(n: i64) -> i64 {
    n * 2
}
fn main() {
    print("{double(2)}")
}
```

## `T-MODULE-RESOLVE.gr`

```grasm
//! tag: T-MODULE-RESOLVE
//! expect: error
//! cannot load module
// `import a.b` resolves to `a/b.gr` beside this file. Nothing declares one
// here, so the import fails rather than silently resolving to something else.
import missing.helper

fn main() {
    print("{helper.value()}")
}
```

## `T-MOVE-LOOP.gr`

```grasm
//! tag: T-MOVE-LOOP
//! expect: error
//! next loop iteration
// A move that would happen more than once.
//
// The closure below is the same case: it runs once per call, so the second
// call would find the value gone. That one was accepted while this one was
// refused, which are the same program written two ways.
fn consume(own s: str) -> i64 {
    s.len()
}

fn main() {
    let name = "ada"
    for _i in 0..3 {
        let n = consume(name)
    }
    let other = "grace"
    let f = |_x| consume(other)
    print("{f(1)} {f(2)}")
}
```

## `T-MUTATE-PLACE.gr`

```grasm
//! tag: T-MUTATE-PLACE
//! expect: error
//! cannot use `remove` on a temporary
// A mutating method needs somewhere to put the result.
//
// A `let` binding was already refused and a temporary was not, so a list
// literal accepted `remove` and panicked at run time — with a message quoting
// a compiler-internal `Span { .. }` rather than a source location.
fn main() {
    print("{[3, 1, 2].remove(0)}")
}
```

## `T-NO-NULL.gr`

```grasm
//! tag: T-NO-NULL
//! expect: output
//! 10 -1
fn main() {
    let xs = [10]
    // Absence is Option, never null.
    print("{xs.get(0) ?? 0} {xs.get(9) ?? 0 - 1}")
}
```

## `T-OK-ERR-RETURN.gr`

```grasm
//! tag: T-OK-ERR-RETURN
//! expect: output
//! 1 -1
//! 2 -1
//! 1 -1
//! 10 6 -1
//! 10 6 -1
// `Ok(v)` and `Err(e)` in a fallible function's own return position.
//
// They worked everywhere a fallible value could be *stored* and nowhere it
// was *produced*: the body's type was checked against the ok half alone, so
// returning the whole `T ! E` was an error — and the two runtimes then wrapped
// it a second time, giving `Ok(Ok(1))` in one and a raw pointer in the other.
type E = { m: str }
type Cell = i64 ! E

fn tail(b: bool) -> i64 ! E {
    if b {
        Ok(1)
    } else {
        Err(E{ m: "no" })
    }
}

/// The same through an alias, which is where it was first noticed.
fn held(b: bool) -> Cell {
    if b {
        Ok(1)
    } else {
        Err(E{ m: "no" })
    }
}

/// The same from an early `return`, which is how an exit mid-function is
/// written — and the one place the constructors did not work.
fn early(b: bool) -> i64 ! E {
    if b {
        return Ok(2)
    }
    return Err(E{ m: "no" })
}

/// A branch giving the bare ok half beside one giving the whole `T ! E`.
///
/// The merge took its type from whichever branch was written first, so
/// `Ok(n * 3)` travelled as a bare integer and was wrapped a second time on
/// the way out: the caller matched `Ok(v)` and printed a heap address where
/// the number was.
fn mixed(n: i64) -> i64 ! E {
    if n == 1 {
        10
    } else if n < 0 {
        Err(E{ m: "neg" })
    } else {
        Ok(n * 3)
    }
}

/// And the same mixture across `match` arms, which is a separate merge.
fn picked(n: i64) -> i64 ! E {
    match n {
        1 -> 10
        2 -> Ok(n * 3)
        _ -> Err(E{ m: "no" })
    }
}

fn main() {
    print("{tail(true) ?? 0 - 1} {tail(false) ?? 0 - 1}")
    print("{early(true) ?? 0 - 1} {early(false) ?? 0 - 1}")
    print("{held(true) ?? 0 - 1} {held(false) ?? 0 - 1}")
    print("{mixed(1) ?? 0 - 1} {mixed(2) ?? 0 - 1} {mixed(0 - 1) ?? 0 - 1}")
    print("{picked(1) ?? 0 - 1} {picked(2) ?? 0 - 1} {picked(3) ?? 0 - 1}")
}
```

## `T-OK-ERR.gr`

```grasm
//! tag: T-OK-ERR
//! expect: output
//! a = 1
//! b ! bad ref
//! 4
//! ok 7 / err gone
// `Ok(v)` and `Err(e)` build a fallible value.
//
// They were matchable and not constructible, so a program storing per-item
// results — which is what `T ! E` is for — needed a trampoline function whose
// entire body was `v`, and another whose body was `fail(e)`, because `fail` is
// legal only inside a fallible function.
type E = { c: str }
type V = | N(n: i64) | S(s: str)

fn describe(r: i64 ! E) -> str {
    match r {
        Ok(v) -> "ok {v}"
        Err(e) -> "err {e.c}"
    }
}

fn main() {
    var cells: Map[str, V ! E] = {}
    cells.set("a", Ok(V.N(1)))
    cells.set("b", Err(E{ c: "bad ref" }))
    for k in cells.keys() {
        match cells.get(k) ?? Err(E{ c: "missing" }) {
            Ok(V.N(n)) -> print("{k} = {n}")
            Ok(V.S(s)) -> print("{k} = {s}")
            Err(e) -> print("{k} ! {e.c}")
        }
    }
    // In a list, and consumed with `??`.
    let xs: [i64 ! E] = [Ok(1), Err(E{ c: "z" }), Ok(3)]
    var total = 0
    for r in xs {
        total = total + (r ?? 0)
    }
    print("{total}")
    // And passed to something expecting a fallible parameter.
    print("{describe(Ok(7))} / {describe(Err(E{ c: "gone" }))}")
}
```

## `T-OPTION.gr`

```grasm
//! tag: T-OPTION
//! expect: output
//! 5 none
fn describe(o: Option[i64]) -> str {
    match o {
        Some(v) -> "{v}"
        None -> "none"
    }
}
fn main() {
    print("{describe(Some(5))} {describe(None)}")
}
```

## `T-PAR-MAP-PURE.gr`

```grasm
//! tag: T-PAR-MAP-PURE
//! expect: error
//! effect-free
//! every element writes the same slot
// A parallel operation's function must be pure — in both senses.
//
// An empty effect row is the half the row system can see. The other half is
// mutation, which is not an effect at all, so a write through a captured `ref`
// went past this rule entirely: `[T-REF-CAPTURE]` allows exactly that write in
// an ordinary closure, and here every element's call holds the same slot. That
// is the interleaving the clause promises is a compile error rather than a
// race.
type Acc = { n: i64 }

fn main() {
    let xs = [1, 2, 3]
    let ys = xs.par_map(|x| {
        print("{x}")
        x
    })
    let acc = ref(Acc{ n: 0 })
    let zs = xs.par_map(|x| {
        acc.n = acc.n + x
        x
    })
    // A `ref` made inside the function is one per element, and reading a
    // captured one shares nothing — neither is refused.
    let ws = xs.par_map(|x| {
        let mine = ref(Acc{ n: x })
        mine.n = mine.n * 2
        mine.n + acc.n
    })
    print("{ys} {zs} {ws}")
}
```

## `T-PARAM-INOUT-PLACES.gr`

```grasm
//! tag: T-PARAM-INOUT-PLACES
//! expect: output
//! 3 [1, 2]
//! 1 [2]
//! 8 [9]
//! [0, 1]
// Mutating a list held in a struct field, and taking from it.
//
// `vm.stack.pop()` is the core operation of anything keeping a stack in a
// struct, and the compiled backend refused it — so a VM had to use `slice`
// and turn an O(1) pop into O(depth).
type VM = { stack: [i64], mem: [i64] }

fn main() {
    var vm = VM{ stack: [1, 2, 3], mem: [1, 0] }
    print("{vm.stack.pop() ?? 0} {vm.stack}")
    print("{vm.stack.remove(0)} {vm.stack}")

    // Through an index as well as a field.
    var rows = [VM{ stack: [9, 8], mem: [] }]
    print("{rows[0].stack.pop() ?? 0} {rows[0].stack}")

    // And the mutating methods that were already reachable stay so.
    vm.mem.sort()
    print("{vm.mem}")
}
```

## `T-PARAM-INOUT.gr`

```grasm
//! tag: T-PARAM-INOUT
//! expect: output
//! 2
fn bump(inout n: i64) {
    n += 1
}
fn main() {
    var n = 1
    bump(n)
    print("{n}")
}
```

## `T-PARAM-LET.gr`

```grasm
//! tag: T-PARAM-LET
//! expect: output
//! 2 2
fn size(xs: [i64]) -> i64 {
    xs.len()
}
fn main() {
    let xs = [1, 2]
    // A plain parameter does not consume the caller's binding.
    print("{size(xs)} {size(xs)}")
}
```

## `T-PARAM-OWN.gr`

```grasm
//! tag: T-PARAM-OWN
//! expect: output
//! 3
fn consume(own s: str) -> i64 {
    s.len()
}
fn main() {
    let name = "ada"
    print("{consume(name)}")
}
```

## `T-PRIM.gr`

```grasm
//! tag: T-PRIM
//! expect: output
//! 42 2.5 true text
fn main() {
    let i: i64 = 42
    let f: f64 = 2.5
    let b: bool = true
    let s: str = "text"
    print("{i} {f} {b} {s}")
}
```

## `T-RECURSIVE.gr`

```grasm
//! tag: T-RECURSIVE
//! expect: output
//! 6
//! 3
// A type referring to itself, which is the first thing a parser needs and
// which the type section did not mention. Through a list, through a map, and
// through a plain field.
type Tree =
    | Leaf(n: i64)
    | Node(kids: [Tree])

type Doc =
    | Text(s: str)
    | Obj(fields: Map[str, Doc])

fn total(t: Tree) -> i64 {
    match t {
        Leaf(n) -> n
        Node(kids) -> kids.fold(0, |acc, k| acc + total(k))
    }
}

fn depth(d: Doc) -> i64 {
    match d {
        Text(_s) -> 1
        Obj(fields) -> 1 + fields.values().fold(0, |acc, v| acc.max(depth(v)))
    }
}

fn main() {
    print("{total(Node([Leaf(1), Node([Leaf(2), Leaf(3)])]))}")
    var inner = map()
    inner.set("a", Text("x"))
    var mid = map()
    mid.set("b", Obj(inner))
    print("{depth(Obj(mid))}")
}
```

## `T-REF-CAPTURE.gr`

```grasm
//! tag: T-REF-CAPTURE
//! expect: output
//! [started, finished] 2
// A captured `ref` may be written through, and that is what lets an effect
// handler accumulate.
//
// [T-CAPTURE-WRITE] refuses a closure that writes to a captured binding,
// because a closure captures *values* and the write would land in a copy
// nobody can see. A reference copies as a slot id, not as a value, so a write
// through one is visible to everyone holding it — the rule does not apply, and
// applying it anyway left a handler unable to accumulate anything at all. The
// obvious spelling was refused and so was the reference that answers it, which
// is what made `inout` on an effect operation look like the only option left
// ([E-EFFECT-NO-INOUT]) — and that silently discarded the write.
type Sink = { lines: [str] }

effect Audit {
    fn record(entry: str)
}

fn work() / {Audit} {
    Audit.record("started")
    Audit.record("finished")
}

fn main() {
    let sink = ref(Sink{ lines: [] })
    with Audit handled_by AuditHandler{ record: |e| sink.lines.push(e) } {
        work()
    }
    print("{sink.lines} {sink.lines.len()}")
}
```

## `T-REF-EQ.gr`

```grasm
//! tag: T-REF-EQ
//! expect: output
//! true true false true
//! false true false
// Two references are equal when they name the same slot and generation.
//
// The interpreter had no case for it, so a `ref` was not equal to *itself* —
// and the failure was contagious, because equality recurses: any struct, list,
// tuple or `Option` holding one was never equal to itself either. `contains`,
// `assert_eq` and every dedup silently misbehaved, against [E-EQ-STRUCTURAL],
// and the compiled backend disagreed because it compares the packed words and
// got this right by accident. A cycle-safe graph walk keyed on `seen.contains`
// looped forever under `grasm run` and terminated compiled — the same program.
//
// Liveness is not consulted. Releasing a slot bumps its generation, so a
// stale reference is never equal to a live one that reuses the slot; and two
// copies of the same released reference are equal, because a value is equal
// to itself and identity cannot depend on when it is asked.
type Node = { v: i64 }
type Edge = { to: ref Node, w: i64 }

fn main() {
    let r = ref(Node{ v: 1 })
    let e1 = Edge{ to: r, w: 5 }
    let e2 = e1
    var seen: [Edge] = [e1]
    print("{e1 == e1} {e1 == e2} {e1 != e1} {seen.contains(e1)}")
    // Released: still itself, still not the new occupant of its slot.
    // Compared through a struct, since `==` on two bare refs is one of the
    // shapes the compiled backend does not take yet (§11).
    let dead = r
    release(r)
    let fresh = ref(Node{ v: 2 })
    let held = Edge{ to: dead, w: 0 }
    print("{alive(dead)} {held == Edge{ to: r, w: 0 }} {held == Edge{ to: fresh, w: 0 }}")
}
```

## `T-REF-GENERATIONAL.gr`

```grasm
//! tag: T-REF-GENERATIONAL
//! expect: output
//! abcab
//! true 7
//! false true
// Tier 2 — `ref T`, the safe graph escape hatch.
//
// Tier 0's references are second-class: they can be passed but not stored,
// which is what removes lifetime annotations from safe code and also makes a
// cyclic or shared graph inexpressible. A `ref T` is a slot in a shared heap
// plus the generation that slot held when the reference was taken. It copies
// freely, so it can live in a struct or a list — that is the point — and every
// read checks the generation, so a reference to a released slot is a
// deterministic panic and never a read of whatever occupies the slot now.
type Node = { name: str, next: i64, v: i64 }

fn main() {
    // A cycle. Each node names the next by slot; the last points back.
    var slots: [ref Node] = []
    slots.push(ref(Node{ name: "a", next: 1, v: 7 }))
    slots.push(ref(Node{ name: "b", next: 2, v: 8 }))
    slots.push(ref(Node{ name: "c", next: 0, v: 9 }))
    var at = 0
    var walk = ""
    for step in 0..5 {
        walk = walk + slots[at].name
        at = slots[at].next
    }
    print("{walk}")

    let first = slots[0]
    print("{alive(first)} {first.v}")
    // Releasing a slot leaves every reference into its previous life stale,
    // and says so rather than reading the slot's next occupant.
    release(first)
    print("{alive(first)} {alive(slots[1])}")
}
```

## `T-REF-MATCH.gr`

```grasm
//! tag: T-REF-MATCH
//! expect: output
//! 3 6
// A reference to a sum type is matched through.
//
// Without this a `ref` to a sum could never be opened, and the two diagnostics
// formed a closed loop: `r.v` said "sum types are accessed via `match`", and
// the match said "pattern `Num(..)` cannot match a `ref Expr`". That excluded
// exactly the recursive shapes an AST or a graph node is made of — which is
// the case this tier exists for.
type Expr = | Num(v: i64) | Add(l: ref Expr, r: ref Expr)

fn eval(e: ref Expr) -> i64 {
    match e {
        Num(v) -> v
        Add(l, r) -> eval(l) + eval(r)
    }
}

fn main() {
    let one = ref(Num(1))
    let two = ref(Num(2))
    let sum = ref(Add(one, two))
    // And nested one level deeper, so the recursion is real.
    print("{eval(sum)} {eval(ref(Add(sum, ref(Num(3)))))}")
}
```

## `T-REF-STALE.gr`

```grasm
//! tag: T-REF-STALE
//! expect: panic
//! stale reference: the value it pointed to was released
// Reading through a released reference is a deterministic panic.
//
// This is the property that makes Tier 2 safe rather than merely convenient:
// the slot may already have been reused, and without the generation check the
// read would return whatever now lives there — a use-after-free with no
// symptom. The generation moves on release, so a reference taken before it can
// never match again.
type Node = { v: i64 }

fn main() {
    let r = ref(Node{ v: 7 })
    print("{r.v}")
    release(r)
    print("{r.v}")
}
```

## `T-REF-VALUE.gr`

```grasm
//! tag: T-REF-VALUE
//! expect: output
//! 0 1 4
//! hi hi there
//! 2 3
//! true false
//! 7.5
//! shared through a copy: 99 99
//! through a list: 100 6 2
//! through a struct field, shared both ways: 9 9
//! a struct referent still uses its own field names: 5
//! released, and the reference knows: false
// `r.value` reads and writes what a reference points at.
//
// Without it `ref(0)` typechecked and nothing in the language could observe
// the result — a value that could be constructed and never used.
type Point = { x: i64 }
type Held = { cell: ref i64 }

fn bump(r: ref i64) {
    r.value += 1
}

fn main() / {io} {
    let n = ref(0)
    let before = n.value
    n.value += 1
    let after = n.value
    n.value = n.value * 4
    print("{before} {after} {n.value}")

    // Every referent that has no fields of its own.
    let s = ref("hi")
    let was = s.value
    s.value = s.value + " there"
    print("{was} {s.value}")

    let xs = ref([1, 2])
    let n_before = xs.value.len()
    xs.value = xs.value + [3]
    print("{n_before} {xs.value.len()}")

    let b = ref(true)
    let b_before = b.value
    b.value = not b.value
    print("{b_before} {b.value}")

    let f = ref(2.5)
    f.value = f.value * 3.0
    print("{f.value}")

    // A reference is a plain word: it copies, and both copies name the same
    // slot. So a write through one is seen through the other — which is what
    // makes a `ref i64` a shared counter rather than a copy.
    let a = ref(0)
    let copy = a
    a.value = 99
    print("shared through a copy: {a.value} {copy.value}")

    // A write through a function parameter lands in the slab too.
    let counter = ref(0)
    bump(counter)
    bump(counter)
    bump(counter)

    // A reference reached through a container is still a reference, and
    // `.value` still writes the slab. The native backend refused this at
    // first while `grasm run` accepted it — a whole-referent write was
    // handled at the root of a place and not in the middle of one.
    var cells: [ref i64] = []
    for i in 0..3 {
        cells.push(ref(i))
    }
    cells[0].value = 100
    cells[1].value += 5
    print("through a list: {cells[0].value} {cells[1].value} {cells[2].value}")

    let shared = ref(1)
    let holder = Held{ cell: shared }
    holder.cell.value = 9
    print("through a struct field, shared both ways: {holder.cell.value} {shared.value}")

    // A struct referent keeps its own field names; `.value` is not offered
    // there, because it would shadow a real field or start meaning something
    // different the day one was added.
    let p = ref(Point{ x: 5 })
    print("a struct referent still uses its own field names: {p.x}")

    release(n)
    release(s)
    release(xs)
    release(b)
    release(f)
    release(a)
    release(counter)
    release(p)
    for c in cells {
        release(c)
    }
    release(shared)
    print("released, and the reference knows: {alive(n)}")
}
```

## `T-REF-WRITE.gr`

```grasm
//! tag: T-REF-WRITE
//! expect: output
//! 1 2
//! 5 9
//! 6 [7]
//! B! A!
//! 42 [1, 99, 7]
//! 5
//! 6 6
// A reference is written through the same way it is read through.
//
// The binding's own mutability is not the question — writing through a
// reference does not modify the reference — and neither is the mutability of
// whatever *holds* it. Only a reference at the root of the path was handled,
// so a node in a list and a node in a struct field, which is what a graph is
// made of, typechecked and then panicked `cannot access field of ref#0.0`.
type Node = { v: i64, kids: [i64] }

type Holder = { r: ref Node }

// A reference *to a reference*, which is what a graph edge is. The write
// descended into one slab value and panicked the moment it met the next —
// `cannot access field `v` of `ref#0.0`` — so a node holding a reference to a
// node was write-once. Reading the identical path worked.
type Graph = { name: str, out: [ref Graph] }

type Inner = { v: i64, xs: [i64] }

type Outer = { r: ref Inner, tag: str }

fn bump(rs: [ref Node]) {
    rs[0].v = rs[0].v + 1
}

fn main() {
    // `let`, not `var`: nothing on the way to the referent is modified.
    let rs = [ref(Node{ v: 1, kids: [] })]
    let h = Holder{ r: ref(Node{ v: 2, kids: [] }) }
    print("{rs[0].v} {h.r.v}")
    rs[0].v = 5
    h.r.v = 9
    print("{rs[0].v} {h.r.v}")
    // Through a parameter, where `var` is not even expressible.
    bump(rs)
    h.r.kids.push(7)
    print("{rs[0].v} {h.r.kids}")
    // Through two hops, and then around a cycle back to the start.
    let a = ref(Graph{ name: "A", out: [] })
    let b = ref(Graph{ name: "B", out: [] })
    a.out.push(b)
    b.out.push(a)
    a.out[0].name = "B!"
    a.out[0].out[0].name = "A!"
    print("{b.name} {a.name}")
    // Two reference hops, then a field, an index and a push. The compiled
    // backend refused every write through a reference until it stopped
    // rebuilding the referent and started writing into the slab: a rebuilt
    // copy stored afterwards erased whatever an inner hop had already
    // written, which is what the cycle above proves.
    let i = ref(Inner{ v: 1, xs: [1, 2] })
    let o = ref(Outer{ r: i, tag: "t" })
    o.r.v = 42
    o.r.xs[1] = 99
    o.r.xs.push(7)
    print("{i.v} {i.xs}")
    // A reference read back out of a map is the same reference.
    let m: Map[str, ref Inner] = {"a": i}
    let got = m.get("a") ?? i
    got.v = 5
    print("{i.v}")
    // And a copy of one writes to the same place: a reference copies as a
    // slot id, which is what makes it shared ([T-REF-GENERATIONAL]).
    let alias = o.r
    alias.v = 6
    print("{i.v} {o.r.v}")
}
```

## `T-REGION-ESCAPE.gr`

```grasm
//! tag: T-REGION-ESCAPE
//! expect: error
//! cannot mutate `out` from inside a region
// Tier 1 — `region { ... }`. Nothing allocated inside outlives the brace.
//
// The rule is what a region *is*, and it is what would make bulk deallocation
// sound: a scope whose allocations provably have no reader afterwards can be
// freed in one sweep. Writing to a binding declared outside is how a value
// would get out — and by a *mutating method*, not only by assignment, which is
// the case a rule about assignment alone would miss.
fn main() {
    var out: [str] = []
    region {
        var scratch: [str] = []
        scratch.push("fine — declared inside")
        out.push("not fine")
    }
    print("{out.len()}")
}
```

## `T-REGION-FOREIGN.gr`

```grasm
//! tag: T-REGION-FOREIGN
//! expect: error
//! cannot perform effect `Audit.record` inside a region
//! cannot call `via_helper` inside a region — it performs `Audit`
//! a function value cannot be used inside a region
// Whose code runs inside the region, not just what the region's own text says.
//
// The handler is written outside and captures a reference. Calling the effect
// from inside the region runs that outside body while the region is open, and
// what it is given is allocated inside — so the region's text contains no
// write to anything outside, and the value escapes anyway.
type Sink = { lines: [str] }

effect Audit {
    fn record(e: str)
}

/// The same reach, one hop further away. Every one of these rules was written
/// first against the syntax between the braces, and a one-line wrapper walked
/// through it — so the rule is about what a call can *reach*, and a helper
/// that performs the effect is refused exactly as the direct call is.
fn via_helper(e: str) / {Audit} {
    Audit.record(e)
}

fn main() {
    let sink = ref(Sink{ lines: [] })
    with Audit handled_by AuditHandler{ record: |e| sink.lines.push(e) } {
        region {
            let made = "inside"
            Audit.record(made)
        }
        region {
            via_helper("through a wrapper")
        }
    }

    // A closure carries its captures, so calling one written outside runs
    // outside code with the region open — however indirectly it is reached.
    // Taking it out of a container first is the spelling that walked past the
    // rule when the rule named bindings instead of values.
    let keepers: [fn(str) -> i64] = [|e| e.len()]
    region {
        let f = keepers[0]
        let _n = f("carried")
    }

    print("{sink.lines.len()}")
}
```

## `T-REGION-INOUT.gr`

```grasm
//! tag: T-REGION-INOUT
//! expect: error
//! cannot pass `inout` `kept` from inside a region
// The escape rule covers `inout`, because `inout` is a write.
//
// The direct spelling `kept.push(made)` was refused and this was not — so the
// same write, moved one call away, defeated assignment, mutating methods,
// deep places, `map.set`, writes through a `ref`, and `spawn`. Two independent
// evaluations found it within an hour of the rule shipping, which is what a
// rule stated in terms of syntax rather than of effect gets you.
fn stash(inout sink: [[str]], v: [str]) {
    sink.push(v)
}

fn main() {
    var kept: [[str]] = []
    region {
        let made = ["allocated inside"]
        stash(kept, made)
    }
    print("{kept.len()}")
}
```

## `T-REGION-REF.gr`

```grasm
//! tag: T-REGION-REF
//! expect: error
//! a `ref` cannot be used inside a region
// A reference is a way out of a region that names nothing outside it.
//
// `stash` is handed a reference the region never mentions by an outside name,
// and a plain parameter, not `inout` — so neither the escape rule nor the
// `inout` rule sees it. The value pushed was allocated inside, and reading it
// after the brace is the one thing a region promises cannot happen.
type Sink = { kids: [str] }

fn stash(s: ref Sink, n: str) {
    s.kids.push(n)
}

fn main() {
    let sink = ref(Sink{ kids: [] })
    region {
        let n = "born inside"
        stash(sink, n)
    }
    print("{sink.kids[0]}")
}
```

## `T-REGION.gr`

```grasm
//! tag: T-REGION
//! expect: output
//! 3 0
//! 9
// What a region is for: scratch work whose result is a number.
//
// Everything the block allocates is provably unreachable after the brace, so
// its peak memory is the block. A scalar may come out, because a scalar is not
// an allocation; a list may not.
fn main() {
    var kept: [str] = []
    let words = region {
        var parts: [str] = []
        parts.push("alpha")
        parts.push("beta")
        parts.push("gamma")
        parts.len()
    }
    print("{words} {kept.len()}")
    // Regions nest, and a region may be an ordinary statement.
    var total = 0
    for i in 0..3 {
        let n = region {
            var scratch: [str] = []
            scratch.push("x{i}")
            region {
                let inner = ["a", "b"]
                inner.len()
            } + scratch.len()
        }
        total = total + n
    }
    print("{total}")
}
```

## `T-STRUCT.gr`

```grasm
//! tag: T-STRUCT
//! expect: output
//! 3 4
type Point = { x: i64, y: i64 }
fn main() {
    var p = Point{ x: 3, y: 0 }
    p.y = 4
    print("{p.x} {p.y}")
}
```

## `T-SUM.gr`

```grasm
//! tag: T-SUM
//! expect: output
//! circle-2.0 dot
type Shape = | Circle(r: f64) | Dot
fn name(s: Shape) -> str {
    match s {
        Circle(r) -> "circle-{r}"
        Dot -> "dot"
    }
}
fn main() {
    print("{name(Circle(2.0))} {name(Dot)}")
}
```

## `T-TASK-CAPTURE.gr`

```grasm
//! tag: T-TASK-CAPTURE
//! expect: error
//! declared inside the `parallel` block
fn main() {
    var log: [str] = []
    parallel {
        for i in 0..3 {
            spawn {
                log.push("{i}")
            }
        }
    }
    print(log.join(","))
}
```

## `T-TRY.gr`

```grasm
//! tag: T-TRY
//! expect: error
//! declares no error type
type E = | Bad
fn risky() -> i64 ! E {
    1
}
// `main` and tests have an open error slot; an ordinary function does not.
fn caller() -> i64 {
    try risky()
}
fn main() {
    print("{caller()}")
}
```

## `T-TUPLE.gr`

```grasm
//! tag: T-TUPLE
//! expect: output
//! 1 is one
fn main() {
    let pair = (1, "one")
    match pair {
        (n, name) -> print("{n} is {name}")
    }
}
```

## `T-TYPE-ALIAS.gr`

```grasm
//! tag: T-TYPE-ALIAS
//! expect: output
//! <a> 6
//! 8 -1
//! {k: 1}
//! HI
// `type Name = str` — a name for a type, not a new type.
//
// The compiled backend refused every one of these, so a program that named its
// types could not be built at all: `type Name = str`, `type Ids = [i64]`, a
// function type, a map, and a fallible one.
type E = { c: str }
type Name = str
type Ids = [i64]
type Res = i64 ! E
type Counts = Map[Name, i64]
type Stage = fn(str) -> str

fn label(n: Name) -> Name {
    "<{n}>"
}

fn total(xs: Ids) -> i64 {
    xs.sum()
}

/// An alias whose target carries the `! E`: the function is fallible even
/// though its declaration does not say so directly.
fn risky(n: i64) -> Res {
    if n < 0 {
        fail(E{ c: "neg" })
    } else {
        n * 2
    }
}

fn up(s: str) -> str {
    s.to_upper()
}

fn run(f: Stage, v: str) -> str {
    f(v)
}

fn main() {
    print("{label("a")} {total([1, 2, 3])}")
    print("{risky(4) ?? 0 - 1} {risky(0 - 1) ?? 0 - 1}")
    var c: Counts = {}
    c.set("k", 1)
    print("{c}")
    print("{run(up, "hi")}")
}
```

## `T-TYPE-NO-PUB.gr`

```grasm
//! tag: T-TYPE-NO-PUB
//! expect: error
//! `pub` means nothing on type `Point`
// `pub` on a type is not a permission, so it is not accepted.
//
// Types share one namespace across the whole program ([T-MODULE-NS]), so a
// type declared without `pub` is already reachable from every module — which
// the grammar offered anyway, `grasm api` printed, and nothing enforced. That
// is the same defect [T-CONST-PUB] records for `pub let` and
// [G-STRUCT-NONEMPTY] for a zero-field struct: syntax that parses, checks, and
// does nothing, while reading to a person as a permission being granted.
pub type Point = { x: i64, y: i64 }

fn main() {
    print("{Point{ x: 1, y: 2 }.x}")
}
```

## `T-UNSAFE-FFI.gr`

```grasm
//! tag: T-UNSAFE-FFI
//! expect: error
//! can only be used inside `unsafe`
// Tier 3 is asked for, not fallen into.
//
// Calling C leaves everything the safe tiers guarantee, so it is spelled at
// the use — `unsafe { ... }` — and again in the manifest, where a consumer can
// see it. `T-EXTERN.gr` and `T-EXTERN-VALUE.gr` show the accepted form; this
// one pins the refusal, including that it covers *naming* a C function and not
// only calling it.
extern fn sqrt(x: f64) -> f64

fn main() {
    print("{sqrt(16.0)}")
}
```

## `T-UNSAFE-UFCS.gr`

```grasm
//! tag: T-UNSAFE-UFCS
//! expect: error
//! can only be used inside `unsafe`
// `x.f()` is the same call as `f(x)`, Tier 3 included.
//
// [L-UFCS] says the two spellings are one call, and only one of them was
// gated. `n.close()` reached libc with no `unsafe`, no `io` in the row, and no
// capability — a package declaring `capabilities = []` called a C function and
// `grasm deps` reported `performs []`. The compiled binary really made the
// call; the interpreter said `no method`, so the two engines disagreed about
// it as well.
extern fn labs(n: i64) -> i64

fn quiet(n: i64) -> i64 / {} {
    n.labs()
}

fn main() {
    print("{quiet(0 - 5)}")
}
```

## `T-USE-AFTER-MOVE.gr`

```grasm
//! tag: T-USE-AFTER-MOVE
//! expect: error
//! use of moved value
fn consume(own s: str) -> i64 {
    s.len()
}
fn main() {
    let name = "ada"
    let n = consume(name)
    print(name)
}
```

## `T-VAR-MUTABLE.gr`

```grasm
//! tag: T-VAR-MUTABLE
//! expect: output
//! 3 [1, 2]
fn main() {
    var n = 1
    n = 3
    var xs = [1]
    xs.push(2)
    print("{n} {xs}")
}
```
