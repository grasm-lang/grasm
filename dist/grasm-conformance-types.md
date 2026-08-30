# Grasm — conformance programs: types

The type and effect system — what a program means before it runs. 96 executable program(s), one per normative clause of
`grasm-language-pack.md`. Each header states the tag it demonstrates and the
exact expected behaviour, and every one is run by `cargo test -p grasm_spec`, so
a program here cannot describe behaviour the implementation does not have.

This is the companion to the definition rather than part of it. Read the
definition to write Grasm; read this when a rule is in dispute and you want to
see it settled by a program.

The programs are split by area because together they no longer fit in one
context, and a reader with a rule in dispute needs one area rather than four.
The other parts are `grasm-conformance-grammar`, `grasm-conformance-types`, `grasm-conformance-semantics`, `grasm-conformance-library`.

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

## `T-COMPARE.gr`

```grasm
//! tag: T-COMPARE
//! expect: output
//! true false
//! true false
//! true false
//! true false
//! true false true false
//! true false
//! true true
//! true false
// `==` and `!=` are defined on any two values of one type; ordering needs a
// type that has one.
//
// Which types those are lived in the code and nowhere else, so a change that
// widened or narrowed the set would have looked exactly like a change that did
// not. `scripts/fuzz_operators.py` holds every operator against every
// representation — 176 cells — and this program is the part of it the
// specification promises.
type P = { n: i64 }
type Col = | Red | Green

fn main() {
    // Equality reaches every representation, boxed or not.
    print("{1 == 1} {1 == 2}")
    print("{"a" == "a"} {"a" == "b"}")
    print("{[1, 2] == [1, 2]} {[1, 2] == [3]}")
    print("{P{ n: 1 } == P{ n: 1 }} {P{ n: 1 } == P{ n: 2 }}")
    print("{Red == Red} {Red == Green} {Some(1) == Some(1)} {Some(1) == None}")
    print("{0..3 == 0..3} {0..3 == 0..=3}")

    // Ordering is lexicographic on a tuple, and needs every component to have
    // an order of its own — which is why a tuple holding a `bool` has none.
    let a: (i64, str) = (1, "z")
    let b: (i64, str) = (2, "a")
    print("{a < b} {(1, "a") < (1, "b")}")
    print("{"a" < "b"} {2.5 < 1.5}")
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
//! every builtin that touches the world is charged io
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

// **Every** builtin that reaches outside the program is charged `io`, and the
// list of them is the thing that has to stay complete.
//
// `read_bytes` and `write_bytes` arrived with [L-LIB-BYTES], beside `read_file`
// and `write_file`, and were left off it. So a function declaring `/ {}` could
// read any file on the disk, and — because a package's ceiling is computed from
// the same rows — a package declaring `capabilities = []` could too. That is
// exactly the property [T-CAPABILITY] exists to provide, so the omission was
// not a gap in the effect system but a hole straight through it.
//
// This function names all of them. It never runs: mentioning a builtin inside a
// row that declares `io` is what makes the checker charge it, and a build where
// any one of these stopped being charged would fail to compile rather than
// silently permitting it.
fn every_world_touching_builtin(path: str) / {io} {
    // Answers discarded with `match` rather than held as annotated `! IoError`
    // values: what matters here is that naming a builtin charges `io`, and
    // `match` says that with the least around it.
    //
    // It was once also a necessity — the native backend had no declaration for
    // the prelude's `IoError` and refused the annotated form — and that is no
    // longer true. Left as it is because the shape is right, not because it has
    // to be.
    match read_file(path) { Ok(v) -> {} Err(e) -> {} }
    match write_file(path, "") { Ok(v) -> {} Err(e) -> {} }
    match read_bytes(path) { Ok(v) -> {} Err(e) -> {} }
    match write_bytes(path, bytes_of([])) { Ok(v) -> {} Err(e) -> {} }
    match read_bytes_at(path, 0, 1) { Ok(v) -> {} Err(e) -> {} }
    match append_bytes(path, bytes_of([])) { Ok(v) -> {} Err(e) -> {} }
    match file_size(path) { Ok(v) -> {} Err(e) -> {} }
    match sync_file(path) { Ok(v) -> {} Err(e) -> {} }
    match rename_file(path, path) { Ok(v) -> {} Err(e) -> {} }
    match remove_file(path) { Ok(v) -> {} Err(e) -> {} }
    match make_dir(path) { Ok(v) -> {} Err(e) -> {} }
    match list_dir(path) { Ok(v) -> {} Err(e) -> {} }
    let _e = file_exists(path)
    // Never reached, so this does not wait for a line — the function is
    // type-checked and not called, which is the whole technique.
    let _l = input()
    let _i = now_millis()
    let _mono = monotonic_millis()
    sleep_millis(0)
    match random_bytes(0) { Ok(v) -> {} Err(e) -> {} }
    let _j = env(path)
    let _k = args()
    eprint("")
}

/// The same enumeration for `net`. Charged correctly today — checked by hand
/// when F35 was found — and listed here so that the *next* one added is caught
/// by a build failure rather than by an audit that happens to be run.
fn every_net_builtin(host: str, port: i64) / {net} {
    match tcp_listen(port) { Ok(v) -> {} Err(e) -> {} }
    match tcp_connect(host, port) { Ok(v) -> {} Err(e) -> {} }
    match tcp_accept(port) { Ok(v) -> {} Err(e) -> {} }
    match tcp_read(port) { Ok(v) -> {} Err(e) -> {} }
    match tcp_read_bytes(port) { Ok(v) -> {} Err(e) -> {} }
    match tcp_write(port, "") { Ok(v) -> {} Err(e) -> {} }
    match tcp_write_bytes(port, bytes_of([])) { Ok(v) -> {} Err(e) -> {} }
    match net_wait([], 0) { Ok(v) -> {} Err(e) -> {} }
    match net_port(port) { Ok(v) -> {} Err(e) -> {} }
    match udp_bind(port) { Ok(v) -> {} Err(e) -> {} }
    match udp_send(port, host, port, bytes_of([])) { Ok(v) -> {} Err(e) -> {} }
    match udp_recv(port) { Ok(v) -> {} Err(e) -> {} }
    match udp_recv_from(port) { Ok(v) -> {} Err(e) -> {} }
    net_close(port)
}

/// And for `proc`.
fn every_proc_builtin(program: str) / {proc} {
    match proc_run(program, []) { Ok(v) -> {} Err(e) -> {} }
    match proc_pipeline([], "") { Ok(v) -> {} Err(e) -> {} }
}

fn main() / {io, async} {
    writes()
    concurrent()
    // Not called — the point is that it *type-checks* as `/ {io}` and would
    // not if any of those stopped being charged.
    print("every builtin that touches the world is charged io")
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
//! aliased: through a name
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

    through(LogHandler{ info: |m| print("{m}"), warn: |m| print("W: {m}") })
}

fn doubled() -> i64 {
    Cfg.value()
}

// The handler type an `effect` declares is a type like any other, so an alias
// may name it. Effects are collected after types, and aliases used to be
// resolved in the pass that collected them, so this line was "unknown type
// `LogHandler`" while a struct field naming the same type worked (F68).
type Sink = LogHandler

fn through(h: Sink) / {io, Log} {
    with Log handled_by h {
        Log.info("aliased: through a name")
    }
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
    with io handled_by ioHandler { nothing: || 1 } {
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
//! 2
//! -1
// `??` over a fallible value, which used to be a four-line `match`.
type E = { m: str }

// An *effect operation* under `??`, which is the same clause and a different
// path through the backend. `m.get(k) ?? d` is fused into one call that never
// builds an Option, and the recogniser for it matches a call named `get` taking
// one argument — which an operation can also be. It probed the receiver to
// confirm a map, `Src` is an effect rather than a value, and the failed probe
// was propagated instead of meaning "not this shape": the program was refused
// (F42). `match Src.get(1)` compiled and an annotated `let` compiled, so only
// this spelling showed it.
effect Src {
    fn get(n: i64) -> i64 ! E
}

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

    with Src handled_by SrcHandler{ get: |n| if n > 0 { Ok(n * 2) } else { Err(E{ m: "low" }) } } {
        print("{Src.get(1) ?? 0}")
        print("{Src.get(0) ?? 0 - 1}")
    }
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

## `T-GENERIC-TYPE.gr`

```grasm
//! tag: T-GENERIC-TYPE
//! expect: output
//! 1 x
//! a 2
//! 2 deep
//! Pair{fst: 1, snd: x}
//! true false
//! has 5
//! str text
//! none
//! good 1
// A generic type declares a family; a use fixes its parameters.
//
// The two-instantiations case is the one that has to work and the one that
// broke first, as a wrong answer rather than a refusal: `Has(5)` and
// `Has("text")` in one program, where a by-name index of variants held one
// parent per name, so the second constructor built the first's type and the
// match unboxed a string pointer as an integer.
type Pair[A, B] = { fst: A, snd: B }
type Opt[T] = | Has(v: T) | Nothing
type Res[T, E] = | Good(v: T) | Bad(e: E)

fn show(o: Opt[i64]) -> str {
    match o {
        Has(v) -> "has {v}"
        Nothing -> "none"
    }
}

fn main() {
    // A struct literal reads its arguments off the fields it gives.
    let p = Pair{ fst: 1, snd: "x" }
    print("{p.fst} {p.snd}")
    let q = Pair{ fst: "a", snd: 2 }
    print("{q.fst} {q.snd}")
    let nested = Pair{ fst: Pair{ fst: 1, snd: 2 }, snd: "deep" }
    print("{nested.fst.snd} {nested.snd}")
    // The member's name is how an implementation tells two apart. It is not
    // what the type is called, and printing one says `Pair`.
    print("{p}")
    print("{p == Pair{ fst: 1, snd: "x" }} {p == Pair{ fst: 2, snd: "x" }}")

    // A sum constructor reads them off its arguments. Two members of one
    // family in one program are two types.
    print("{show(Has(5))}")
    let b = Has("text")
    match b {
        Has(v) -> print("str {v}")
        Nothing -> print("none")
    }
    let n: Opt[i64] = Nothing
    print("{show(n)}")

    // A parameter no argument mentions has to be written down.
    let r: Res[i64, str] = Good(1)
    match r {
        Good(v) -> print("good {v}")
        Bad(e) -> print("bad {e}")
    }
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
    let bound = match Plain{
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
//! a sum referent, written through a copy: Green Green
//! and an Option the same way: Some(2)
//! into the referent, not over it: [1, 9] [4, 50] [7, 80]
//! released, and the reference knows: false
// `r.value` reads and writes what a reference points at.
//
// Without it `ref(0)` typechecked and nothing in the language could observe
// the result — a value that could be constructed and never used.
type Point = { x: i64 }
type Col = | Red | Green
type Held = { cell: ref i64 }
type Held2 = { cell: ref [i64] }

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

    // A sum has no field names either, so it is exactly the case `.value`
    // describes. Asking whether the referent was a *named* type excluded it
    // along with the structs, and since rebinding a `var` only moves the
    // local, nothing could write through a `ref` to a sum at all: readable
    // through `match`, never writable, on the tier that exists so a reference
    // is a shared cell rather than a copy.
    let col = ref(Red)
    let col_copy = col
    col_copy.value = Green
    print("a sum referent, written through a copy: {col.value} {col_copy.value}")

    let opt = ref(Some(1))
    opt.value = Some(2)
    print("and an Option the same way: {opt.value}")

    // A write *inside* the referent, rather than replacing it. `.value` names
    // the referent, so it is spent by reaching one and what follows applies to
    // the referent itself -- passed on whole, the rest of the walk was asked
    // for `list.value[0]`. Reading the same place always compiled, and so did
    // replacing the whole referent one line above.
    let cell = ref([1, 2])
    cell.value[1] = 9
    var cells2: [ref [i64]] = [ref([4, 5])]
    cells2[0].value[1] = 50
    let boxed = Held2{ cell: ref([7, 8]) }
    boxed.cell.value[1] = 80
    print("into the referent, not over it: {cell.value} {cells2[0].value} {boxed.cell.value}")

    release(cell)
    release(boxed.cell)
    for c in cells2 {
        release(c)
    }
    release(col)
    release(opt)
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
//! 7
//! deep 4
//! [1, 2, 3] 3
//! deep [1, 2, 3] 99
//! 1 42
//! 1 42
type Point = { x: i64, y: i64 }

// A field path is read by **borrowing** every step of it. `o.i.v` used to fetch
// `i` and increment it, read `v`, then release `i` — and that pair exists only
// to protect a value the root already protects, because every intermediate in a
// path rooted at a live local is reachable from that local.
//
// That is an optimisation, so it must be invisible, and these are the four ways
// it could stop being:
//
//   * a chain ending in a **boxed** field has to hand out an owned value, or it
//     dangles the moment the root changes;
//   * a chain through a **shared** value must not let a write through one copy
//     be seen through the other;
//   * a chain under an **index** must not borrow at all — an index can run code
//     that frees what an earlier step borrowed;
//   * and the plain scalar read has to still answer correctly.
type D = { n: i64, tag: str }
type C = { d: D, xs: [i64] }
type B = { c: C }
type A = { b: B }

fn main() {
    var p = Point{ x: 3, y: 0 }
    p.y = 4
    print("{p.x} {p.y}")

    var a = A{ b: B{ c: C{ d: D{ n: 7, tag: "deep" }, xs: [1, 2, 3] } } }
    print("{a.b.c.d.n}")

    // Boxed results out of a borrowed chain are owned by whoever asked.
    let t = a.b.c.d.tag
    let l = a.b.c.xs
    print("{t} {t.len()}")
    print("{l} {l.len()}")

    // Replacing the root must not disturb them. If the chain had handed out
    // borrowed pointers these would now read freed memory.
    a = A{ b: B{ c: C{ d: D{ n: 99, tag: "gone" }, xs: [] } } }
    print("{t} {l} {a.b.c.d.n}")

    // A copy shares structure, and a write through one copy is not visible
    // through the other ([E-VALUE-SEMANTICS]) — which the borrow must not
    // weaken, since it hands out the *same* interior pointer to both.
    var one = A{ b: B{ c: C{ d: D{ n: 1, tag: "x" }, xs: [9] } } }
    var two = one
    two.b.c.d.n = 42
    print("{one.b.c.d.n} {two.b.c.d.n}")

    // Under an index the owning path is used instead, and answers the same.
    var many: [A] = []
    many.push(one)
    many.push(two)
    print("{many[0].b.c.d.n} {many[1].b.c.d.n}")
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
