# Grasm — conformance programs: semantics

Evaluation — what a program does when it runs. 71 executable program(s), one per normative clause of
`grasm-language-pack.md`. Each header states the tag it demonstrates and the
exact expected behaviour, and every one is run by `cargo test -p grasm_spec`, so
a program here cannot describe behaviour the implementation does not have.

This is the companion to the definition rather than part of it. Read the
definition to write Grasm; read this when a rule is in dispute and you want to
see it settled by a program.

The programs are split by area because together they no longer fit in one
context, and a reader with a rule in dispute needs one area rather than four.
The other parts are `grasm-conformance-grammar`, `grasm-conformance-types`, `grasm-conformance-semantics`, `grasm-conformance-library`.

## `E-ACTOR.gr`

```grasm
//! tag: E-ACTOR
//! expect: output
//! 3 deposits, balance 160
//! refused 500
//! 3 deposits, balance 160
// An account several tasks deposit into. Its state is reached only through
// its methods, and a method cannot wait, so each runs to its end before any
// other task runs: no deposit is lost between reading and writing the
// balance.
actor Account(opening: i64) {
    var balance: i64 = opening
    var deposits: i64 = 0

    fn deposit(inout self, amount: i64) {
        self.balance += amount
        self.deposits += 1
    }

    fn withdraw(inout self, amount: i64) -> bool {
        if amount > self.balance {
            false
        } else {
            self.balance -= amount
            true
        }
    }

    fn summary(self) -> str {
        "{self.deposits} deposits, balance {self.balance}"
    }
}

fn main() / {io, async} {
    let acct = Account(100)
    concurrent {
        for amount in [10, 20, 30] {
            go {
                acct.deposit(amount)
            }
        }
    }
    print(acct.summary())
    // A copy of the handle is the same account.
    let same = acct
    if not same.withdraw(500) {
        print("refused 500")
    }
    print(acct.summary())
}
```

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
//! -17: -8,-1 -4,-1 0,-17 -17,0 8,-1 4,-1 0,-17 17,0
//! -8: -4,0 -2,0 0,-8 -8,0 4,0 2,0 0,-8 8,0
//! -1: 0,-1 0,-1 0,-1 -1,0 0,-1 0,-1 0,-1 1,0
//! 0: 0,0 0,0 0,0 0,0 0,0 0,0 0,0 0,0
//! 7: 3,1 1,3 0,7 7,0 -3,1 -1,3 0,7 -7,0
//! 4611686018427387904: 2305843009213693952,0 1152921504606846976,0 4503599627370496,0 4611686018427387904,0 -2305843009213693952,0 -1152921504606846976,0 -4503599627370496,0 -4611686018427387904,0
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

    // A *constant* divisor that is a power of two is compiled as shifts
    // rather than as a divide, and the truncation has to come out the same:
    // `/` rounds toward zero and `%` takes the sign of the dividend, so a
    // negative dividend needs a bias the shift does not supply by itself.
    // Both signs of divisor, because `x % -d` is `x % d` and `x / -d` is not.
    for x in [-17, -8, -1, 0, 7, 4611686018427387904] {
        let a = "{x / 2},{x % 2} {x / 4},{x % 4} {x / 1024},{x % 1024}"
        let b = "{x / 1},{x % 1} {x / -2},{x % -2} {x / -4},{x % -4}"
        let c = "{x / -1024},{x % -1024} {x / -1},{x % -1}"
        print("{x}: {a} {b} {c}")
    }
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

## `E-CHANNEL.gr`

```grasm
//! tag: E-CHANNEL
//! expect: panic
//! Some(1) Some(2) None None
//! select 1
//! select 0
//! send on a closed channel
// A buffered channel needs no receiver to be waiting: two values fit. They
// come out in the order they went in, then `None` for as long as anyone asks.
fn main() / {io, async} {
    let ch = chan_new(2)
    chan_send(ch, 1)
    chan_send(ch, 2)
    chan_close(ch)
    print("{chan_recv(ch)} {chan_recv(ch)} {chan_recv(ch)} {chan_recv(ch)}")
    // `chan_select` names the first channel a `chan_recv` would not wait on:
    // one holding a value, or one closed.
    let a = chan_new(1)
    let b = chan_new(1)
    chan_send(b, 9)
    print("select {chan_select([a, b])}")
    chan_close(a)
    print("select {chan_select([a, b])}")
    chan_send(ch, 3)
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

## `E-CONCURRENT.gr`

```grasm
//! tag: E-CONCURRENT
//! expect: output
//! squares 14
//! after the block
// The block waits for both tasks, so the line after it prints last. Each task
// sees `n` by value; what comes back comes back on a channel.
fn main() / {io, async} {
    let n = 4
    let ch = chan_new()
    concurrent {
        go {
            for i in 0..n {
                chan_send(ch, i * i)
            }
            chan_close(ch)
        }
        go {
            var total = 0
            while true {
                match chan_recv(ch) {
                    Some(v) -> total += v
                    None -> break
                }
            }
            print("squares {total}")
        }
    }
    print("after the block")
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
//! 2 143 0 0 69 1 100 101 169
// A list grown in a loop, and grown again by a loop inside that one: what a
// writer appending each block's bytes does. The compiled engine once made room
// for the inner loop's elements by moving the list while the outer loop still
// held it open, and the bytes went to the old storage.
fn blocks(n: i64, size: i64) -> [i64] {
    var out: [i64] = [n]
    var b = 0
    while b < n {
        out.push(b)
        for i in 0..size {
            out.push(b * 100 + i)
        }
        b = b + 1
    }
    out
}

fn main() {
    // Copy-on-write is never observable: the copy is independent.
    var xs = [1, 2, 3]
    var ys = xs
    ys.push(4)
    print("{xs.len()} {ys.len()}")
    let bs = blocks(2, 70)
    print("{bs[0]} {bs.len()} {bs[1]} {bs[2]} {bs[71]} {bs[72]} {bs[73]} {bs[74]} {bs[bs.len() - 1]}")
}
```

## `E-DEADLOCK.gr`

```grasm
//! tag: E-DEADLOCK
//! expect: panic
//! first
//! deadlock
// Each task waits to hear from the other before it says anything, so neither
// ever does. That is reported, not left to hang.
fn main() / {io, async} {
    let a = chan_new()
    let b = chan_new()
    concurrent {
        go {
            print("first")
            let x = chan_recv(a) ?? 0
            chan_send(b, x)
        }
        go {
            let y = chan_recv(b) ?? 0
            chan_send(a, y)
        }
    }
    print("never")
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
//! true true true true false
type P = { x: i64, y: [str] }

fn main() {
    let a = P{ x: 1, y: ["k"] }
    let b = P{ x: 1, y: ["k"] }
    let c = P{ x: 2, y: ["k"] }
    print("{a == b} {[1, 2] == [1, 2]} {a == c}")
    // `bytes` compared false against itself in every engine, so the gates
    // that ask whether the engines agree could not see it (F93). Built four
    // ways here, because the representation differs by how it was made.
    let literal = bytes_of([104, 105])
    let text = "hi".to_bytes()
    let cut = bytes_of([32, 104, 105, 32]).slice(1, 1 + 2)
    let joined = bytes_of([104]).concat(bytes_of([105]))
    print("{literal == literal} {literal == text} {literal == cut} " +
        "{literal == joined} {literal == bytes_of([104, 106])}")
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
//! 7
//! 7
// A handler installed by `with` is in scope for everything the block reaches,
// however many calls deep — the operation is resolved where it is *performed*,
// not where it is written.
//
// The second half is the same property approached from the other side: the
// handler itself travels as a value, and the function that installs it never
// saw the literal. An operation call cannot tell the difference, because it
// was never a direct call — it is a lookup by effect and operation name, and
// both halves of this program fill the same table.
effect Cfg {
    fn value() -> i64
}
fn deep() -> i64 {
    Cfg.value()
}
fn middle() -> i64 {
    deep()
}
fn install(h: CfgHandler) / {io, Cfg} {
    with Cfg handled_by h {
        print("{middle()}")
    }
}
fn main() {
    with Cfg handled_by CfgHandler{ value: || 42 } {
        print("{middle()}")
    }
    let seven = CfgHandler{ value: || 7 }
    install(seven)
    install(CfgHandler{ value: || 7 })
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
//! -1 -1
//! -1 -1
//! 11 22
//! 0 -> err z
//! 1 -> 100
//! 2 -> 50
// A branch that leaves — `fail`, `panic`, `return` — has no value, so the
// `if` is worth whatever the *other* branch produced, and when *both* leave
// it is worth nothing at all.
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

/// Both branches leave, so the `if` produces nothing and there is nothing to
/// bind. The statement has to stop there: binding a placeholder wrote
/// instructions into a block control had already left, and the backend
/// reported its own verifier error as a codegen bug.
fn both_bound(y: i64) -> i64 ! E {
    let v: i64 ! E = if y == 0 {
        fail(E{ c: "a" })
    } else {
        fail(E{ c: "b" })
    }
    v
}

/// The same shape where the `if` is returned rather than bound, which is
/// where this was first found and fixed.
fn both_returned(y: i64) -> i64 ! E {
    return if y == 0 {
        fail(E{ c: "a" })
    } else {
        fail(E{ c: "b" })
    }
}

/// And where both branches `return` outright.
fn both_left(y: i64) -> i64 {
    let v = if y == 0 {
        return 11
    } else {
        return 22
    }
    v
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
    // Both branches leaving, bound and returned and left outright.
    print("{both_bound(0) ?? 0 - 1} {both_bound(3) ?? 0 - 1}")
    print("{both_returned(0) ?? 0 - 1} {both_returned(3) ?? 0 - 1}")
    print("{both_left(0)} {both_left(3)}")
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
//! [1, 3, 3] [1, 3, 3, 4]
//! [9, 2] [1, 2] [[7], [2]] [[1], [2]]
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

// A later argument may read the variable an `inout` place lives in. The
// compiled engine moved the place out first -- leaving a hole until the
// write-back -- and `h.xs[1]` then panicked "index on non-list" where
// `grasm run` read the list it was given.
type Holder = { xs: [i64] }
type Grid = { rows: [[i64]] }

fn put(inout xs: [i64], i: i64, v: i64) {
    xs[i] = v
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

    var h = Holder{ xs: [1, 2, 3] }
    put(h.xs, 1, h.xs[1] + 1)
    let before = "{h.xs}"
    h.xs.push(h.xs.len() + 1)
    print("{before} {h.xs}")

    // A write through a field changes only this holder: a copy taken before
    // keeps what it had, whether the write moves the list out of an unshared
    // holder and back or copies it out of a shared one.
    var a = Holder{ xs: [1, 2] }
    let kept = a
    a.xs[0] = 9
    var g = Grid{ rows: [[1], [2]] }
    let gkept = g
    g.rows[0][0] = 7
    print("{a.xs} {kept.xs} {g.rows} {gkept.rows}")
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
//! [0, 1, 10, 11]
//! [0, 1, 2, 3, 4]
//! 5 [0, 1, 2]
//! 60 3
//! 1 2 3
//! 120 3
//! 1 2 3
//! 20 2
//! 14
//! 6 0
//! 201 101 102
type Cell = { v: i64 }

/// Every mention of `c` is a field read, so the loop may lend its element.
fn peek(c: Cell) -> i64 {
    c.v * 10
}

/// Mentions `c` bare, so it is not inferred read-only and the loop has to hand
/// over a reference rather than a loan. ([T-MOVE-LOOP] refuses `own` here
/// outright -- a moved element would be gone by the next iteration -- so this
/// is the shape that actually distinguishes the two paths.)
fn stash(c: Cell) -> i64 {
    let held = [c]
    held[0].v * 20
}

/// A walk over a field: `for k in n.kids` reads the list off the node in
/// place, and the recursive call is spliced with `k` lent to it.
type Tree = { v: i64, kids: [Tree] }

fn sum_tree(n: Tree) -> i64 {
    var s = n.v
    for k in n.kids {
        s = s + sum_tree(k)
    }
    s
}

type Holder = { items: [Cell] }

/// Lent parameters bound straight to the caller's locals, where the names
/// cross: `pair(b, a)` must not let the first binding change what the second
/// argument reads.
fn pair(a: Cell, b: Cell) -> i64 {
    a.v * 100 + b.v
}

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

    // A list filled from a *nested* loop.
    //
    // The compiled backend holds a list's buffer open for the loop that fills
    // it, so a push is a store rather than a call. Two loops filling the same
    // list each held their own length, and the outer one closed last with the
    // length it took before the inner loop ran — so this came out empty.
    var pairs: [i64] = []
    for i in 0..2 {
        for j in 0..2 {
            pairs.push(i * 10 + j)
        }
    }
    print("{pairs}")

    // A loop whose *condition* reads the list it fills.
    //
    // The same optimisation keeps the length in a register until the loop
    // ends. `k.len()` in the condition then read the length the list had
    // before the first push, every time, and the loop never finished.
    var k: [i64] = []
    while k.len() < 5 {
        k.push(k.len())
    }
    print("{k}")

    // And a list read *after* the loop that filled it knows how long it is.
    var made: [i64] = []
    for i in 0..3 {
        made.push(i)
    }
    print("{made.len() + 2} {made}")

    // A loop over boxed elements lends each one rather than taking a
    // reference to it: the list holds the element open for the whole body, so
    // the +1 and the -1 buy nothing. What must not change is the answer, or
    // the list, or who owns what.
    let cells = [Cell{ v: 1 }, Cell{ v: 2 }, Cell{ v: 3 }]
    var sum = 0
    for c in cells {
        sum = sum + peek(c)
    }
    // The list is untouched by having been walked.
    print("{sum} {cells.len()}")
    print("{cells[0].v} {cells[1].v} {cells[2].v}")

    // The same loop where the callee keeps what it is given. Lending here
    // would hand out something the list still holds, so the loop gives a
    // reference instead — and the list still comes out whole.
    var kept = 0
    for c in cells {
        kept = kept + stash(c)
    }
    print("{kept} {cells.len()}")
    print("{cells[0].v} {cells[1].v} {cells[2].v}")

    // Leaving the body early releases nothing that was only lent.
    var seen = 0
    for c in cells {
        if c.v == 3 {
            break
        }
        if c.v == 1 {
            continue
        }
        seen = seen + peek(c)
    }
    print("{seen} {cells.len() - 1}")

    // Walked through a field, recursively.
    let leaf = Tree{ v: 3, kids: [] }
    let t = Tree{ v: 1, kids: [Tree{ v: 2, kids: [leaf, leaf] }, Tree{ v: 5, kids: [] }] }
    print("{sum_tree(t)}")

    // A body that replaces the struct it is walking cannot be lent the list:
    // the old list has to survive until the loop is done with it.
    var h = Holder{ items: [Cell{ v: 1 }, Cell{ v: 2 }, Cell{ v: 3 }] }
    var got = 0
    for c in h.items {
        h = Holder{ items: [] }
        got = got + c.v
    }
    print("{got} {h.items.len()}")

    let a = Cell{ v: 1 }
    let b = Cell{ v: 2 }
    print("{pair(b, a)} {pair(a, a)} {pair(a, b)}")
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
//! 0 [] [0]
//! 1 [0] [01]
//! 2 [01] [012]
//! 3 [012] [0123]
//! 4 [0123] [01234]
//! 5 [01234] [012345]
//! 6 [012345] [0123456]
//! 7 [0123456] [01234567]
//! 8 [01234567] [012345678]
//! 9 [012345678] [0123456789]
//! 3 4
//! -5 0
//! 0 B
//! 1 B
//! 2 1B
//! 3 12B
//! 4 12B
//! 5 124B
//! 6 1245B
//! 7 1245B
//! 8 12457B
//! 9 124578B
//! 10 124578B
//! 11 1245780B
//! 12 12457801
//! 1,2,4,5,7,8,10,11,
//! 3
//! 7
//! 4 6
//! 325 325 15 325
//! [-0.061111111111111116, -0.016666666666666663, 0.016666666666666663, 0.0444444444444444, 0.06904761904761902, 0.09166666666666667, 0.11296296296296293]
//! [1.6912037037037035, 1.8958333333333333, 2.1508333333333334, 2.4259259259259265, 2.7085884353741494, 2.992708333333333, 3.274948559670783]
//! [1.0, 1.75, 2.875, 4.5625, 7.09375, 10.890625, 16.5859375]
// A counted loop is compiled four iterations at a time while four are left,
// then one at a time for the remainder — so every trip count around that
// four, both range forms, and both ways out of the body have to answer what
// a loop compiled one iteration at a time would.
// A `while` whose condition is tested again at the bottom of the body rather
// than by jumping back to the top. `continue` has to reach that test and not
// skip it, and a body that always leaves never reaches it at all.
fn first_over(xs: [i64], limit: i64) -> i64 {
    var i = 0
    while i < xs.len() {
        return xs[i] + limit
    }
    0 - 1
}

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

    // Every trip count from none to twice the group.
    for m in 0..10 {
        var s = ""
        for i in 0..m {
            s += "{i}"
        }
        var t = ""
        for i in 0..=m {
            t += "{i}"
        }
        print("{m} [{s}] [{t}]")
    }

    // The ends of the number line, where deciding whether four more fit
    // would overflow if it were written as `i + 4 <= end`.
    let lo = 0 - 9223372036854775807 - 1
    var low = 0
    for _i in lo..lo + 3 {
        low += 1
    }
    var high = 0
    for _i in 9223372036854775804..=9223372036854775807 {
        high += 1
    }
    print("{low} {high}")

    // Negative bounds, and a backwards range, which runs not at all.
    var sum = 0
    for i in -5..5 {
        sum = sum + i
    }
    var never = 0
    for _i in 5..0 {
        never += 1
    }
    print("{sum} {never}")

    // `break` leaves, and `continue` skips the rest of its own iteration —
    // including the copies of the body that would have followed it.
    for m in 0..13 {
        var hit = ""
        for i in 0..12 {
            if i == m {
                hit += "B"
                break
            }
            if i % 3 == 0 {
                continue
            }
            hit += "{i % 10}"
        }
        print("{m} {hit}")
    }

    var w = 0
    var line = ""
    while w < 11 {
        w += 1
        if w % 3 == 0 {
            continue
        }
        line += "{w},"
    }
    print(line)
    print("{first_over([1, 2], 2)}")
    let ys = [4, 9, 7, 0, 5]
    var k = 0
    while ys[k] != 0 and k < ys.len() {
        if ys[k] == 7 {
            break
        }
        k += 1
    }
    print("{ys[k]}")
    var n = 0
    var m = 0
    while n < 4 {
        n += 1
        m = m + if n % 2 == 0 { 2 } else { 1 }
    }
    print("{n} {m}")

    // Loops a compiled build may run without a bounds check per element,
    // once it has checked the whole range up front. The answers are the
    // ones any loop gives: including a range that runs no iterations at all,
    // one written backwards, a base below zero and the last element reached
    // by an inclusive range.
    let n = 5
    var grid = (0..n * n).map(|v| v)
    var sum = 0
    for r in 0..n {
        for c in 0..n {
            grid[r * n + c] += 1
            sum += grid[r * n + c]
        }
    }
    var still = sum
    for c in 3..3 {
        still += grid[c + 100]
    }
    for c in 5..2 {
        still += grid[c + 100]
    }
    let back = 0 - 2
    var neg = 0
    for c in 2..7 {
        neg += grid[back + c]
    }
    var incl = 0
    for c in 0..=24 {
        incl += grid[c]
    }
    print("{sum} {still} {neg} {incl}")

    // `f64` loops a compiled build may run two elements at a time. Every lane
    // is the scalar operation, unfused and in the same order, so the digits
    // are the ones any engine prints: odd lengths leave a remainder, and a
    // loop where each element reads the one before it must not be split.
    let fl = 7
    var fx = (0..fl).map(|v| v.as_f64() * 0.1 + 0.3)
    var fy = (0..fl).map(|v| 1.0 / (v.as_f64() + 3.0))
    for v in 0..fl {
        fy[v] = 2.5 * fx[v] + fy[v]
    }
    for v in 0..fl {
        fx[v] -= fy[v] / 3.0
        fy[v] *= 0.0 - fx[v] + 1.5
    }
    var chain = (0..fl).map(|_v| 1.0)
    for v in 0..fl - 1 {
        chain[v + 1] = chain[v] * 1.5 + 0.25
    }
    print("{fx}")
    print("{fy}")
    print("{chain}")
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
    let m = { key(log, "k1"): val(log, 1), key(log, "k2"): val(log, 2) }
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
//! 832040 832041 1028458
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

    // Independent: each task writes its own `var` from a pure call and reads
    // no other task's. A compiled build may run these on separate cores; the
    // answer is the one running them in order gives.
    var a = 0
    var b = 0
    var c = 0
    parallel {
        spawn { a = fib(30) }
        spawn { b = fib(30) + 1 }
        spawn { c = fib(29) * 2 }
    }
    print("{a} {b} {c}")
}

fn fib(n: i64) -> i64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
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
//! once
//! 8
//! 8 4
//! [7, 7, 7] [1, 2, 3]
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

    // A call is a call however the backend chooses to compile it. A small
    // function may be spliced into its caller instead of called, and the
    // three things that can go wrong if it is are all visible from here.
    //
    // The argument is evaluated once, where it is written, not once per
    // mention of the parameter in the body.
    print("{twice(note("once", 4))}")
    // A parameter may share a name with something the caller has bound, and
    // the caller's binding is untouched by the call.
    let n = 4
    print("{twice(n)} {n}")
    // And a callee's own local may share a name with a list the caller is
    // filling in a loop, which is the shape that reaches furthest into a
    // backend: `out` here is two different lists.
    var out: [i64] = []
    for v in [1, 2, 3] {
        out.push(v)
        let _ = filled()
    }
    print("{filled()} {out}")
}

// `n` is also a name `main` binds; `v + v` mentions the parameter twice.
fn twice(n: i64) -> i64 {
    let v = n
    return v + v
}

// `out` is also a name `main` binds, and fills in a loop.
fn filled() -> [i64] {
    var out: [i64] = []
    out.push(7)
    out.push(7)
    out.push(7)
    return out
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
//! 6 jobs, in order
//! 0 1 4 9 16 25
//! same as one core
//! 3 kept
type P = { a: i64, b: str }

/// A short list whose elements are expensive is worth splitting, and says the
/// same thing however it was split.
type Job = { n: i64, label: str }

/// Expensive enough that the runtime chooses to spread six elements, and
/// exact, so the answer cannot depend on how they were divided.
//
// The count used to be the whole question: fewer than 256 elements meant one
// core, whatever was in them, so a thousand-element `x * x` was threaded and
// six elements of real work were not. What the clause promises is the answer,
// and the answer is the same either way — which is why this file can be
// written at all.
fn work(j: Job) -> i64 {
    var spin = 0
    var i = 0
    while i < 400000 {
        spin = (spin + i) % 7919
        i = i + 1
    }
    return j.n * j.n + spin * 0
}

fn heavy_even(j: Job) -> bool {
    var spin = 0
    var i = 0
    while i < 400000 {
        spin = (spin + i) % 7919
        i = i + 1
    }
    return j.n % 2 == 0 and spin >= 0
}

fn few_but_expensive() {
    var jobs: [Job] = []
    var k = 0
    while k < 6 {
        jobs.push(Job{ n: k, label: "j{k}" })
        k = k + 1
    }
    let done = jobs.par_map(|j| work(j))
    print("{done.len()} jobs, in order")
    var line = ""
    for v in done {
        line = line + (if line == "" { "" } else { " " }) + "{v}"
    }
    print(line)
    var serial: [i64] = []
    for j in jobs {
        serial.push(work(j))
    }
    print(if serial == done { "same as one core" } else { "DIFFERENT" })
    print("{jobs.par_filter(|j| heavy_even(j)).len()} kept")
}

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

    few_but_expensive()
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

## `E-SCHEDULE.gr`

```grasm
//! tag: E-SCHEDULE
//! expect: output
//! ping 0
//! pong 0
//! ping 1
//! pong 10
//! ping 2
//! pong 20
// Two rendezvous channels force strict alternation, and the fixed order fixes
// which of the two prints first: the task whose `go` ran first.
fn main() / {io, async} {
    let ping = chan_new()
    let pong = chan_new()
    concurrent {
        go {
            for i in 0..3 {
                print("ping {i}")
                chan_send(ping, i)
                let _back = chan_recv(pong)
            }
        }
        go {
            for _i in 0..3 {
                let n = chan_recv(ping) ?? 0
                print("pong {n * 10}")
                chan_send(pong, n)
            }
        }
    }
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
//! 300
//! 302
//! 303 304
//! 304 305
//! yzα w
//! 300 600
//! 301 601 q
fn main() {
    // Characters, not bytes: each of these is multi-byte in UTF-8.
    let s = "héllo"
    print("{"abc".len()} {s.substr(0, 3).len()}")

    // `len` counts characters, and for text long enough to be worth it the
    // count is remembered rather than rescanned — `while i < s.len()` asked
    // for it once per character and read the whole string each time, which is
    // quadratic. What must hold is that the remembered count never outlives
    // the text it described.
    var a = ""
    for _i in 0..300 { a += "x" }
    print("{a.len()}")
    a += "yz"
    print("{a.len()}")

    // ASCII until it is not. A count remembered while the text was one byte
    // per character would be wrong the moment it stops being.
    a += "\u{03b1}"
    print("{a.len()} {a.byte_len()}")
    a += "w"
    print("{a.len()} {a.byte_len()}")
    print("{a.substr(300, 3)} {a.substr(a.len() - 1, 1)}")

    // Multi-byte from the start, grown past the same threshold.
    var b = ""
    for _i in 0..300 { b += "\u{00e9}" }
    print("{b.len()} {b.byte_len()}")
    b += "q"
    print("{b.len()} {b.byte_len()} {b.substr(300, 1)}")
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
//! abab
//! xyxy
//! pp1q
//! nn-2-true-1.5
//! 01234
//! [0:0][1:2][2:4][3:6][4:8]
//! 1390 545082
//! x0:-1234:-2468:-3702:-4936:
//! 012345 21
//! 0123
//! 00 01 02 10 11 12 20 21 22 
//! false-false-true-
//! a0b0a1b1a2b2
//! ab-ab
//! xyxy
//! 0a1b2c
//! q-0-q-1-
//! xya zwb
// A string built by appending to itself is the same string however the
// backend chooses to build it. `s = s + rest` and `s += rest` are compiled
// as an append rather than a rebuild — in place when nothing else holds the
// string, and an interpolation's pieces go onto the buffer one at a time
// rather than being built into a string of their own first. So each of the
// shapes where the left and right sides can be the *same* value has to come
// out the way a rebuild would.
fn main() {
    print("hello, " + "world")

    // The right side is the left side: two references to one string, so
    // nothing can write into it in place.
    var s = "ab"
    s = s + s
    print(s)
    // The same, reached through an interpolation.
    var t = "xy"
    t = t + "{t}"
    print(t)
    // And read twice in one interpolation, once for its text and once for a
    // length that is taken before any of it is appended.
    var u = "p"
    u = u + "{u}{u.len()}q"
    print(u)
    // Every piece kind in one, through `+=`.
    var v = "n"
    v += "{v}-{1 + 1}-{true}-{1.5}"
    print(v)

    // The loop shapes, which are why any of this exists.
    var w = ""
    for i in 0..5 {
        w = w + "{i}"
    }
    print(w)
    var z = ""
    for i in 0..5 {
        z += "[{i}:{i * 2}]"
    }
    print(z)

    // A string a loop only appends to has its buffer held open, and the
    // length in the value itself is not written back until the loop ends —
    // so every shape that could read a stale one has to keep it off that
    // path, and every shape that takes it has to come out the same.
    //
    // Past the inline buffer and past a growth of the heap one.
    var g = ""
    for i in 0..300 {
        g += "[{i}]"
    }
    var gh = 0
    for c in g.codes() {
        gh = (gh * 31 + c) % 1000003
    }
    print("{g.len()} {gh}")

    // Negative and multi-digit, which the one-digit store does not take.
    var m = "x"
    for i in 0..5 {
        m = m + "{0 - i * 1234}:"
    }
    print(m)

    // Read inside the loop: the length it sees must be the length it has.
    var r = ""
    var seen = 0
    for i in 0..6 {
        r += "{i}"
        seen = seen + r.len()
    }
    print("{r} {seen}")

    // Left early, so what was written before the `break` still counts.
    var e = ""
    for i in 0..10 {
        if i == 4 {
            break
        }
        e += "{i}"
    }
    print(e)

    // A nested loop appending to the same string.
    var n = ""
    for i in 0..3 {
        for j in 0..3 {
            n += "{i}{j} "
        }
    }
    print(n)

    // A piece that is not an integer, so the whole loop keeps the older path.
    var b2 = ""
    for i in 0..3 {
        b2 += "{i > 1}-"
    }
    print(b2)

    // A chain of appends is the same append, twice over. `c = c + x + y`
    // parses as `((c + x) + y)`, so the name sits at the end of the left
    // spine rather than directly on the left; reading only the direct form
    // left this rebuilding the whole string each time round, which is
    // quadratic — 200,000 bytes of hex took 697 ms where one piece takes 3.
    var c = ""
    for i in 0..3 {
        c = c + "a{i}" + "b{i}"
    }
    print(c)

    // Appending in place changes the name as it goes, so a chain that reads
    // the name again cannot take that path: the later piece would see what
    // the earlier one already wrote. This gave `ab-ab-` for `ab-ab`.
    var d = "ab"
    d = d + "-" + d
    print(d)

    // One piece is safe either way, because it is read before the append.
    var twice = "xy"
    twice = twice + twice
    print(twice)

    // A chain of three, mixing an interpolation with a plain string.
    var e3 = ""
    let tail = ["a", "b", "c"]
    for i in 0..3 {
        e3 = e3 + "{i}" + tail[i]
    }
    print(e3)

    // A chain whose head is a *different* name is not an append at all.
    var f3 = ""
    let q = "q"
    for i in 0..2 {
        f3 = f3 + q + "-{i}-"
    }
    print(f3)

    // Binding a name a second time in the same block releases what the first
    // binding owned. The scope releases only what it holds when it closes, so
    // without that the first value is simply lost — and it takes an append to
    // bite, because a bare literal is immortal and has no reference to lose.
    // This exited 7 under `GRASM_LEAK_CHECK=1` with one allocation live.
    var sh = "xy"
    sh = sh + "a"
    let first = sh
    var sh = "zw"
    sh = sh + "b"
    print("{first} {sh}")
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
//! hi! hi hi
//! hixxx hi
//! [1, 2] [1, 2, 9]
//! 1
//! [5] [5, 7] 1
//! [3, 4] [3, 4]
//! [8] [0]
//! 9.0 7 true | 1.0 3 true
//! 10.5 14 false | 9.0 7 true
//! 9.0 7 true | 9.0 100 true
//! 0.0 1 true
//! 1.0 1 false
//! 20.0 3 true
//! 3.0 3 false
//! 8 3.0 3 false 3.0 3 false
//! [2] [1] [3] [3, 3] []
type Box = { n: i64 }
type Bag = { items: [i64] }
type Two = { a: [i64], b: [i64] }

// A block's tail takes the locals it reads last instead of copying them. Each
// of these has to come out the same as if it copied.
fn grown() -> [i64] {
    var v = [1]
    v.push(2)
    v
}

fn bagged(keep: [i64]) -> Bag {
    var mine = keep
    let alias = mine
    print("{alias.len()}")
    Bag{ items: mine }
}

fn both() -> Two {
    let v = [3, 4]
    Two{ a: v, b: v }
}

fn pick(c: bool) -> [i64] {
    let v = [8]
    if c { v } else { [0] }
}

// A struct of scalars may be compiled as its fields, each in a register,
// rather than as one block of memory. A copy of it is then a snapshot that
// later writes to the original must not reach, in every direction.
type Pt = { x: f64, n: i64, ok: bool }

fn bump(p: Pt) -> Pt {
    var q = p
    q.x += 1.5
    q.n *= 2
    q.ok = not q.ok
    q
}

fn show(p: Pt) -> str {
    "{p.x} {p.n} {p.ok}"
}

fn snapshots() {
    var a = Pt{ x: 1.0, n: 3, ok: true }
    let snap = a
    a.x = 9.0
    a.n += 4
    print("{show(a)} | {show(snap)}")
    var b = bump(a)
    print("{show(b)} | {show(a)}")
    b = a
    a.n = 100
    print("{show(b)} | {show(a)}")
    var ps: [Pt] = []
    for i in 0..4 {
        var p = Pt{ x: i.as_f64(), n: i, ok: i % 2 == 0 }
        if p.ok {
            p.x = p.x * 10.0
        } else {
            p.n -= 1
        }
        ps.push(p)
        p.n = 999
    }
    for i in 0..ps.len() {
        var e = ps[i]
        e.n += 1
        ps[i] = e
        e.n = -5
    }
    var total = 0
    for p in ps {
        total += p.n
        print(show(p))
    }
    var c = ps[0]
    c = ps[3]
    print("{total} {show(c)} {show(ps[3])}")
}

// A struct chosen by an `if` or a `match` and returned: each branch's value
// is the one handed back, held once -- not copied and left behind.
type Halves = { left: [i64], right: [i64] }

fn halves(xs: [i64], flip: bool) -> Halves {
    if flip {
        Halves{ left: [xs[1]], right: [xs[0]] }
    } else {
        Halves{ left: [xs[0]], right: [xs[1]] }
    }
}

fn pick_halves(n: i64) -> Halves {
    match n {
        0 -> return Halves{ left: [], right: [] }
        _ -> return Halves{ left: [n], right: [n, n] }
    }
}

fn main() {
    var a = [1, 2]
    var b = a
    b.push(3)
    print("{a.len()} {b.len()}")

    var p = Box{ n: 1 }
    var q = p
    q.n = 99
    print("{p.n} {q.n}")

    // A **string literal** is one shared value, made once and never freed —
    // which is an implementation choice and must stay invisible. Value
    // semantics is what makes it invisible: appending to a literal produces a
    // new string rather than changing the one every other occurrence of that
    // text is also using.
    //
    // Without this the first `s += "!"` in a program would rewrite every
    // `"hi"` in it, which is the kind of defect that looks like the compiler
    // going mad.
    var s = "hi"
    s += "!"
    var t = "hi"
    print("{s} {t} {"hi"}")

    // The same, growing in a loop: `s + "x"` never touches the literal `s`
    // started as.
    var g = "hi"
    for i in 0..3 {
        g = g + "x"
    }
    print("{g} {"hi"}")

    let g1 = grown()
    var g2 = grown()
    g2.push(9)
    print("{g1} {g2}")

    let orig = [5]
    var bag = bagged(orig)
    bag.items.push(7)
    print("{orig} {bag.items} {orig.len()}")

    var two = both()
    print("{two.a} {two.b}")

    print("{pick(true)} {pick(false)}")

    snapshots()
    let h = halves([1, 2], true)
    let k = pick_halves(3)
    print("{h.left} {h.right} {k.left} {k.right} {pick_halves(0).left}")
}
```
