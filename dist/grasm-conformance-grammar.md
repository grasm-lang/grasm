# Grasm — conformance programs: grammar

The grammar — what is a program. 20 executable program(s), one per normative clause of
`grasm-language-pack.md`. Each header states the tag it demonstrates and the
exact expected behaviour, and every one is run by `cargo test -p grasm_spec`, so
a program here cannot describe behaviour the implementation does not have.

This is the companion to the definition rather than part of it. Read the
definition to write Grasm; read this when a rule is in dispute and you want to
see it settled by a program.

The programs are split by area because together they no longer fit in one
context, and a reader with a rule in dispute needs one area rather than four.
The other parts are `grasm-conformance-grammar`, `grasm-conformance-types`, `grasm-conformance-semantics`, `grasm-conformance-library`.

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
