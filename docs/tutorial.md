# Grasm in an hour

A guided tour, in the order the pieces build on each other. Every program here
runs — `grasm run <file>` — and the code blocks are the same ones the
conformance suite and `examples/` exercise, so nothing drifts out of date
silently.

If you want the exact rules rather than the tour, read `spec/`. This document
explains; the spec decides.

---

## 1. A first program

```
fn main() {
    print("hello, world")
}
```

`grasm run hello.gr` typechecks and executes. `grasm build hello.gr --native`
produces a standalone binary through Cranelift.

There are no semicolons, no imports needed for `print`, and no type
annotations required where they can be inferred:

```
fn main() {
    let name = "world"
    let count = 3
    for i in 1..=count {
        print("hello, {name} ({i})")
    }
}
```

`{...}` inside a string interpolates any expression. `1..=3` includes 3;
`1..3` excludes it.

## 2. Bindings

`let` is immutable, `var` is not. That is the whole rule.

```
fn main() {
    let x = 1
    var total = 0
    total += x        // fine
    // x = 2         // error: cannot assign to `x` — it is immutable
    print("{total}")
}
```

Values have **value semantics**: assigning or passing never creates an alias,
so mutating one binding can never be observed through another.

```
fn main() {
    var a = [1, 2, 3]
    var b = a
    b.push(4)
    print("{a.len()} {b.len()}")   // 3 4
}
```

Copies are lazy — a copy costs a reference-count bump and only becomes a real
copy on mutation of a shared value — but that is an optimization you cannot
observe.

## 3. Types

Scalars are `i64`, `f64`, `bool`, `str`. Collections are `[T]` and
`Map[K, V]`. Annotate when you want to; infer when you do not.

```
type Point = { x: f64, y: f64 }

type Shape =
    | Circle(r: f64)
    | Rect(w: f64, h: f64)

fn area(s: Shape) -> f64 {
    match s {
        Circle(r) -> 3.14159 * r * r
        Rect(w, h) -> w * h
    }
}

fn main() {
    print("{area(Circle(2.0))}")
}
```

`match` must be exhaustive. Adding a variant to `Shape` turns every incomplete
match into a compile error rather than a surprise at runtime.

Maps iterate in sorted key order, so printing one is deterministic:

```
fn main() {
    var counts = map()
    counts.set("pear", 2)
    counts.set("fig", 1)
    print("{counts}")            // same output on every run
}
```

## 4. Errors are values

There are no exceptions. A function that can fail says so in its type with
`! E`, raises with `fail`, and callers propagate with `try`.

```
fn divide(a: i64, b: i64) -> i64 ! str {
    if b == 0 {
        fail("division by zero")
    }
    a / b
}

fn compute() -> i64 ! str {
    let half = try divide(10, 2)
    try divide(half, 0)
}

fn main() {
    match compute() {
        Ok(v) -> print("got {v}")
        Err(e) -> print("failed: {e}")
    }
}
```

You cannot forget to handle one: using a fallible value without `try` or a
`match` is a compile error. An error that reaches `main` exits 1 with a
message rather than exiting 0 as though nothing happened.

A **panic** is different: it means a bug (index out of bounds, divide by zero,
a failed `assert`), prints `panic: <message>`, and exits 101. Panics are not
for recoverable conditions.

`Option` covers absence:

```
fn main() {
    let xs = [3, 1, 4]
    print("{xs.min() ?? 0}")           // 1
    print("{xs.find(|x| x > 99) ?? -1}")  // -1
}
```

`??` evaluates its right side only when the left is `None`.

## 5. Effects

A function's type says what it *does*, not just what it returns. Built-in
effects are `io` and `async`; you can declare your own.

```
effect Clock {
    fn now() -> i64
}

fn stamp() -> str / {Clock} {
    "at {Clock.now()}"
}

fn main() {
    with Clock handled_by ClockHandler{ now: || 1700000000 } {
        print(stamp())
    }
}
```

An effect that reaches `main` without a handler is a *compile* error. This is
what makes tests hermetic by construction: a function that touches the clock
or the network cannot pretend otherwise, and a test supplies a handler instead
of a mocking framework.

The same property is what the package manager enforces at the dependency
boundary — see §8.

## 6. Ownership without a borrow checker

Three parameter modes, and the default is the one you want:

- plain — the callee gets a value; the caller keeps theirs.
- `inout` — the callee mutates the caller's binding in place. The caller
  passes a mutable place, and the same place cannot be passed as `inout` twice
  in one call.
- `own` — the caller gives the value up. Using it afterwards is a compile
  error, **for types that are not copied**. `i64`, `f64`, `bool`, and `()` are
  copied ([T-COPY]), so `own` on one of those transfers nothing and the caller
  keeps using its binding. `str`, lists, maps, and structs do move. Write the
  demonstration with one of those, or it will appear to disprove the rule it
  is demonstrating:

```
fn consume(own xs: [i64]) -> i64 {
    xs.len()
}

fn main() {
    let xs = [1, 2, 3, 4, 5]
    print("{consume(xs)}")
    // print("{xs.len()}")   // error: use of moved value `xs`
}
```

```
fn bump(inout n: i64) {
    n += 1
}

fn main() {
    var count = 0
    bump(count)
    print("{count}")     // 1
}
```

The mode lives on the parameter, so the call site stays plain. No lifetimes,
no borrow annotations on types, no `&`/`&mut` sprinkled through the code.

## 6a. Functions as parameters

`fn(A, B) -> R` is a type, so a program can declare its own higher-order
function rather than being limited to the built-in ones:

```
fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn double(n: i64) -> i64 {
    n * 2
}

fn main() {
    print("{apply(double, 5)}")
    print("{apply(|n| n + 1, 5)}")
}
```

A closure passed to such a parameter takes its parameter types from it, so it
needs no annotation.

The type carries an effect row, and **no row means the empty row**. So
`fn(i64) -> i64` accepts only functions that perform nothing, and passing one
that prints is an error where you pass it — not a surprise inside. Write
`fn(i64) -> i64 / {io}` when the caller should be allowed to hand over
something effectful. This is the one place a row is written rather than
inferred, because a type has no body to infer from.

## 7. Structured concurrency

`parallel` opens a scope; `spawn` adds a task to it. The scope does not exit
until every task inside it has finished.

```
fn main() {
    var log: [str] = []
    parallel {
        spawn {
            log.push("first")
        }
        spawn {
            log.push("second")
        }
    }
    print(log.join(" "))     // first second
}
```

Tasks run in spawn order, deterministically: the same program always produces
the same output. That determinism is deliberate and is *not* multi-core
speedup — `parallel` buys you structure and reproducibility, not throughput,
because its tasks share the enclosing scope and running them at once would
race.

Throughput has its own form, and the effect system is what makes it safe:

```
fn main() {
    let xs = [1, 2, 3, 4, 5]
    print("{xs.par_map(|x| x * x)}")
}
```

`par_map` and `par_filter` spread the work across cores. The function must be
effect-free and **the compiler proves it** — put a `print` inside and the
build fails — so there are no `Send`/`Sync` bounds to satisfy and no race
detector to run. Results come back in input order whatever the core count is,
so the answer never depends on the machine.

A panicking task aborts the whole scope; tasks spawned after it do not run.

## 8. Modules and packages

```
import lib.math

fn main() {
    print("{math.square(4)}")
}
```

`pub` controls what crosses a module boundary. `import helper` finds
`helper.gr` beside the importing file; `import lib.math` finds `lib/math.gr`.
No manifest is needed for either — a manifest is what turns a directory into a
*package*, which is what dependencies and capabilities are about.

A package is a directory laid out like this:

```
mathlib/
  grasm.toml          the manifest
  src/
    main.gr           the entry file — what `import mathlib` resolves to
```

`src/main.gr` is the default; `entry = "src/lib.gr"` in `[package]` changes it.
Everything a package exports lives behind that one file, so a dependency's
other modules are reached as `import mathlib.helpers`.

```toml
[package]
name = "mathlib"
version = "0.1.0"
capabilities = []          # this package performs no effects

[dependencies.utils]
path = "../utils"
version = "^1.0.0"         # optional: 1.x, and no other line
```

`version` accepts a compatible range with `^` (a bare `1.0.0` means the same)
or one exact version with `=`. Before 1.0 the minor number carries the weight
of a major, so `^0.2.0` stops below `0.3.0`.

Drop the `path` and the package comes from a registry — a directory of
`<name>/<version>/` packages, pointed at by `GRASM_REGISTRY` or found as the
nearest `.grasm/registry`. Which version you get is the *lowest* one that
satisfies everybody, so adding a dependency never quietly upgrades something
else you were already using.

`capabilities` is a **ceiling**, and it is checked against what the code
actually does. **Omitting the key means unrestricted**, not empty — a package
with no `capabilities` line is not held to anything, and `grasm deps` reports
it as `(unrestricted)`. Write `capabilities = []` to mean "performs nothing";
leaving it out means "do not check me". Because effects are inferred from source, a dependency that
quietly starts reading the filesystem exceeds its declared ceiling and fails
to build **in every consumer** — a compile error, not a runtime surprise, and
not something a reviewer has to happen to notice.

`grasm deps` resolves the graph, checks every package against its ceiling, and
writes `grasm.lock` with a SHA-256 over each package's sources.

## 9. Testing

One declaration form covers three kinds of test.

```
fn reverse(xs: [i64]) -> [i64] {
    var out: [i64] = []
    var i = xs.len() - 1
    while i >= 0 {
        out.push(xs[i])
        i = i - 1
    }
    out
}

test "reverse of a known list" {
    assert(reverse([1, 2, 3]) == [3, 2, 1])
}

test "reversing twice is the identity" (xs: [i64]) {
    assert(reverse(reverse(xs)) == xs)
}
```

Parameters make it a **property** test: the runner generates the inputs.
Pointing one at `str` is fuzzing — the generator emits tabs, newlines, quotes,
backslashes, and multi-byte characters.

A failure reports a *shrunk* counterexample — the smallest input that still
fails — plus the seed that found it:

```text
test "doubling is monotonic" ... FAILED on case 5 of 100
  assertion failed
  counterexample: n = 4611686018427387904
  replay: --seed 24301 --cases 100
```

That number is exactly the smallest `n` where `n * 2` wraps.

`grasm test --coverage` reports which statement lines the suite reached.
`bench "name" { ... }` bodies are warmed up, then sampled, and `grasm bench`
reports ns/op.

## 10. Documentation that cannot rot

`///` comments document the declaration that follows. `grasm doc` renders them
as Markdown with signatures taken from the AST, so a documented effect row or
error type cannot drift from the real one.

````
/// Adds two integers.
///
/// Addition wraps on overflow.
///
/// ```
/// assert(add(2, 3) == 5)
/// ```
fn add(a: i64, b: i64) -> i64 {
    a + b
}
````

The fenced block is **executable**. `grasm test` runs it as a test, in a
program that keeps the real module structure — so an example may write
`mathlib.square(4)`, exactly as a consumer would. Documentation that stops
being true fails the suite instead of quietly misleading the next reader.

Lines beginning `# ` inside a block are hidden setup: they run, but they are
not part of what a reader sees.

## Where to go next

- `spec/` — the normative rules, each tagged and backed by a program in
  `spec/conformance/` that the test suite runs.
- `examples/` — runnable programs, including `stdlib_tour.gr` and
  `testing_tour.gr`.
- `projects/` — twelve larger programs, beginner through advanced.
- `MASTERPLAN.md` — where the language is going and why.
