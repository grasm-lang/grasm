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

A `let` at module level is a **constant** — one value for the whole program:

```
let TABLE_WIDTH = 77
let TITLE = "Sales report"

fn main() {
    print("{TITLE}, {TABLE_WIDTH} wide")
}
```

A constant's value cannot perform effects. It is evaluated wherever its name
is used, so an effect in it would happen once per mention; a value that has to
be computed once, with effects, is a function.

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

A tuple can be split into names, in a `let` or in a loop header:

```
fn main() {
    let (a, b) = (1, "two")
    print("{a} {b}")

    for (i, w) in ["x", "y"].enumerate() {
        print("{i}{w}")
    }
}
```

Those are the only destructuring forms — there is no `if let`, and patterns
elsewhere appear only in `match`. They matter most with maps, where
`entries()` hands back pairs, and with `enumerate()` and `zip()`.

Write a map with `{key: value}`, or build one up from `{}`. Maps iterate in
sorted key order, so printing one is deterministic — and what it prints is
what you would type to write it back:

```
fn main() {
    let ripeness = {"pear": 2, "fig": 1}
    print("{ripeness}")          // {fig: 1, pear: 2} — sorted, every run

    var counts: Map[str, i64] = {}
    counts.set("pear", 2)
    print("{counts}")
}
```

`{}` is the empty map. Braces holding *statements* are a block, and an empty
block has no use as an expression, so `()` is how you write unit. The braces
of `if`, `for`, `while`, `match` and function bodies are always blocks.

Lists join with `+`, and sort by a key you extract — which is how a list of
structs gets sorted at all, since a struct has no order of its own:

```
type Run = { name: str, ms: i64 }

fn main() {
    var runs = [Run{ name: "b", ms: 30 }, Run{ name: "a", ms: 10 }]
    runs.sort_by(|r| r.ms)
    for r in runs {
        print("{r.name} {r.ms}")
    }

    let all = [1, 2] + [3]
    print("{all}")
}
```

Text is characters, not bytes, and `code_at` / `codes` / `to_char` move
between a character and its Unicode code point — which is what lets you
compute on text rather than only cut and compare it:

```
fn hash(s: str) -> i64 {
    var h = 0
    for c in s.codes() {
        h = (h * 31 + c) % 1000007
    }
    h
}

fn main() {
    print("{hash("grasm")} {104.to_char()}")
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

A closure captures the **values** it uses at the moment it is created, not the
bindings ([E-CLOSURE-CAPTURE]). Changing a captured variable afterwards is not
visible inside, and assigning to one inside changes only the closure's copy:

```
fn main() {
    var n = 1
    let plus_n = |x| x + n
    n = 100
    print("{plus_n(0)}")     // 1, not 100
}
```

That has one consequence worth knowing before you design around it: **an
effect handler cannot hold mutable state**, because a handler's operations are
closures. A `Store` whose `write` updates a captured list updates a copy, and
the next `read` will not see it. Handlers supply values and accept output;
state that changes belongs in the caller. `examples/ledger/` is that lesson
written down after getting it wrong.

Note that `spawn { … }` does *not* work this way — a task body shares the
enclosing scope ([E-SPAWN-SHARED]). Two block-shaped constructs, two rules.

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

Which is also why only the compiled backend actually uses them: `grasm run`
and `grasm build -o` produce a bundled *interpreter*, and there a `par_map` is
a `map`. The answer is identical — that is the whole point of requiring purity
— but the throughput is not, so measure with `grasm build --native`.

A panicking task aborts the whole scope; tasks spawned after it do not run.
A `return` inside a task body returns from the function the `parallel` block
is in, and the rest of the scope — the tasks not yet run, and the statements
after the block — is abandoned. That is a way out, not a way to end one task.

The two forms divide the work between them, and it is worth knowing which you
want before you reach for one. **`for x in xs { spawn { … x … } }` is not
writable**: a task's body reads the enclosing scope as it stands when the
tasks run, so a loop variable is gone by then, and the compiler says so. The
number of tasks a `parallel` block starts is fixed where it is written.

So: a *runtime-sized* amount of work over data is `par_map`/`par_filter`,
which is also the form that gets real cores and that the compiler proves safe.
`parallel`/`spawn` is for a fixed set of distinct activities that share results
through the enclosing scope.

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
It has to name a file *inside* the package: the lockfile's checksum, the
capability ceiling and `grasm publish` all cover the package's own directory,
so an `entry` reaching out of it with `..` — or an absolute path — is refused
rather than silently leaving all three describing a tree the program does not
use. It has to end in `.gr` for the same reason: those three collect the
package's Grasm sources, and an entry named anything else was executed but
recorded nowhere — `--locked` reported "up to date" after it changed, and
`grasm publish` shipped a package that could not build into a slot that cannot
be republished.
Everything a package exports lives behind that one file, so a dependency's
other modules are reached as `import mathlib.helpers`.

"The package's sources" is **one** answer to all three questions, which it was
not: the checksum skipped `target/` and every dot-directory, both of which are
legal import path segments; the `.gr` extension was matched case-sensitively,
so `HELPER.GR` — what `import helper` resolves to on a case-insensitive
filesystem — was invisible to the checksum, to `publish` and to the ceiling
alike; and `publish` walked from the entry's directory while the checksum
walked from the package's, so one released version had two checksums. A
directory symlink is followed once: a package holding two self-links took
`deps` and `publish` from "34 copies into an immutable slot" to "never
returns".

A package with a `path` dependency cannot be published at all. Only the
package's own directory travels, so the release named a dependency that
resolved outside the version directory — every consumer failed on a version
that cannot be republished.

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
leaving it out means "do not check me". A key `[package]` does not recognise is
refused rather than ignored, because that difference is exactly what a typo
would spend: `capabilitys = []` used to leave the package unrestricted, and the
only trace was a column in `grasm deps`. Because effects are inferred from source, a dependency that
quietly starts reading the filesystem exceeds its declared ceiling and fails
to build **in every consumer** — a compile error, not a runtime surprise, and
not something a reviewer has to happen to notice.

A capability is what a package needs from **outside** itself, so what counts is
what *escapes*: the effects `main` performs, the effects left in a `pub`
function's row for a consumer to handle, and the effects its `test` and `bench`
bodies and its **doc examples** perform. That last group surprises people — a
test is not part of the interface, and an example is documentation — but
`grasm test` runs the tests *and the doc examples* of every imported module,
so a dependency's test body and its `///` fenced blocks really do execute in
your process. The consequence is worth knowing: a `capabilities = []` package
cannot `print` from a test or from a doc example, which rules out the usual
`/// print("{f(2)}")` — write the example as `assert(f(2) == 4)` instead, which
performs nothing and checks more. Every module in the package counts,
not only the ones its entry file imports — a consumer reaches `pub fn f` with
`import dep.module` whether or not the dependency's own entry mentions that
module, so a `capabilities = []` package could otherwise keep a filesystem
reader in an unimported sibling file and be reported as performing nothing.
A *dependency's* `pub` rows do not count toward the consumer, whose ceiling is
what it needs, not what everything it depends on offers; what the consumer
actually calls is already in the row of whatever calls it. An effect you declare, perform and
handle inside your own package needs nothing from anyone and is not a
capability — but the handler answers for what it really does, so a `Log`
handler that calls `print` puts `io` in the ceiling whoever asked for it.

`grasm deps` resolves the graph, checks every package against its ceiling, and
writes `grasm.lock` with a SHA-256 over each package's manifest and sources.
The manifest is in the digest because the `capabilities` line is what a
consumer is relying on: three releases with identical code and different
ceilings hashed the same without it.

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

`grasm test file.gr` runs the tests of every module the program imports, not
just that file's, and names each one by its module. `grasm bench` does the
same for benchmarks. A library whose tests are
failing should not look like a library whose tests passed.

Parameters make it a **property** test: the runner generates the inputs.
Pointing one at `str` is fuzzing — the generator emits tabs, newlines, quotes,
backslashes, and multi-byte characters.

A failure reports a *shrunk* counterexample — the smallest input that still
fails — plus the seed that found it:

```text
test "doubling is monotonic" ... FAILED on case 1 of 100
  assertion failed
  counterexample: n = 0
  replay: --seed 24301 --cases 100
```

`0` is the simplest witness the shrinker could reach: `0 * 2` is `0`, which is
not greater. The property is false for `0`, for every negative number, and for
`4611686018427387904` and up, where `n * 2` wraps — the shrinker reports the
simplest one it can, not the first one it found.

That seed is the *default*, and it does not change between runs: `grasm test`
twice runs the same hundred cases twice, so a suite that passes today passes
tomorrow. Running it again is not how you widen coverage — `--seed` is. What
the fixed run does guarantee is the boundary values: the first ten cases walk
them in order, so `0`, `±1`, `±2`, `10`, `100`, the `i64` extremes, NaN, both
infinities, both zeros, both `bool`s and the empty string are tried every time
rather than merely often. The rest of the run is the widening random mix.

`grasm test --coverage` reports which statement lines the suite reached —
counting the lines that doc examples reach, since those are tests too. It did
not, so a function reached *only* by its own documented example was reported
uncovered while the suite had just run it — and then it did, but only for the
entry file: a group of examples runs in a synthesized program where the owner's
source is inlined, so the lines an *imported* module's examples reached were
being read against the wrong file.
`bench "name" { ... }` bodies are warmed up, then sampled, and `grasm bench`
reports time per operation, in whatever unit keeps the figure readable.

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

The fenced block is **executable**. `grasm test` runs it as a test, compiled
in the module that owns it: that module's own items are in scope unqualified,
and everything else it imports is available the way the module has it. So an
example writes `square(4)` — the way the module's own code does — and the same
example passes whether the library is tested on its own or through a consumer.
Documentation that stops being true fails the suite instead of quietly
misleading the next reader.

Lines beginning `# ` inside a block are hidden setup: they run, but they are
not part of what a reader sees.

## 10a. Calling C

`extern fn` declares a C function. There is no body and no binding layer: a
compiled build resolves the symbol at link time, and `grasm run` finds it in
its own process.

```
extern fn sqrt(x: f64) -> f64
extern fn getpid() -> i64

fn main() {
    let r = unsafe {
        sqrt(16.0)
    }
    print("{r}")
}
```

Parameters and results are `i64` and `f64`. `bool` is deliberately not
allowed: C's is one byte and Grasm's is a register, and that kind of mismatch
surfaces as garbage far from where it was caused.

**It has to be inside `unsafe`, and it counts as both `unsafe` and `io`.**
Calling C is Tier 3 — outside everything the safe tiers guarantee — so the
source says so at the use, and the package says so in its manifest:

```
capabilities = ["io", "unsafe"]
```

Naming a C function without calling it needs `unsafe` as well: holding the
function *is* the ability to call it. What a C function does cannot be known,
so the safe assumption is the only one available — an `extern` use counts
against the ceiling, and a function that makes one cannot be handed to
`par_map`. A consumer can see `unsafe` in your manifest, transitively, and
refuse to depend on you: your JSON parser cannot quietly contain one.

## 10b. Knowing what you broke

`grasm api` prints a package's public surface — every `pub` item, sorted, one
line each. Save it, and a later `--against` says what changed:

```text
$ grasm api src/main.gr > v1.api
$ grasm api src/main.gr --against v1.api
CHANGED  lib::Point
    was  type Point = { x: i64, y: i64 }
    now  type Point = { x: f64, y: i64 }

breaking: 0 removed, 1 changed, 0 added — this is a MAJOR release
```

It exits 1 on a breaking change, so a build can refuse one that the version
number does not admit to.

A version number is supposed to answer "will this break me". One a human typed
cannot answer it — it records what somebody *believed*. This one is read off
the code.

The limit is worth stating: this compares **shape**, not behaviour. A function
that keeps its signature and starts returning something else is a `PATCH` here
and a disaster for you. Deciding that automatically is undecidable, and a tool
that claimed to would be worse than one that says where it stops.

## 10c. Making a failure happen twice

A program is a function of its code and everything it read from outside itself.
The code is in the binary; the rest can be written down:

```text
$ GRASM_RECORD=j ./server          # the run that went wrong
$ GRASM_REPLAY=j grasm run src/main.gr
```

The second run does not read the clock, the disk, the network or the
environment. It takes every answer from `j` — so it behaves the same on your
laptop as it did on the machine that failed, as many times as you need, under a
debugger.

It also does not *write*. Replaying a `write_file` to reproduce a bug would
damage a disk. `print` is the deliberate exception: a replay whose output you
cannot see is not worth running.

`grasm journal` shows what was recorded:

```text
$ grasm journal j
j — 5 record(s), 264 bytes

  1  now_millis  1787564958324
  2  env         Some("/home/dev")
  3  read_file   Ok("port = 8080\n")
  4  tcp_listen  Err(IoFailed("Permission denied (os error 13)"))
  5  now_millis  1787564958324

  5 call(s) across 4 builtin(s): env 1, now_millis 2, read_file 1, tcp_listen 1
```

**Why this is cheap here and expensive elsewhere.** Knowing which calls are
non-deterministic is normally the hard part of building a record-and-replay
tool. In Grasm it was already written down: a function that reaches outside the
program declares an effect (§5), and one that does not *cannot*. The journal is
"every call with a row, in order, and what it answered".

**Divergence is refused, not guessed.** If the program takes a different path
from the recorded one, the run stops and says which call disagreed:

```text
grasm: at call 3 the recording says `read_file` and this run asked for
`env` — the program has taken a different path from the one recorded, so
this is not a replay of it
```

Handing back the next record anyway would produce something that looks like a
replay and is not, and a tool whose whole promise is *this is what happened*
must not be able to lie about it.

A recording is portable between engines, which is the case that matters: the
failure happens in the compiled build and the debugger is in `grasm run`. It
even replays in a browser tab — see [replay.md](replay.md).

## 11. What `--native` will not compile yet

`grasm run` and `grasm build --native` are two implementations of one
language, and `scripts/difftest.sh` holds them to identical output on every
program in the repository. The compiled backend is nonetheless a *smaller*
language than this document describes. It refuses what it cannot do, by name
and at build time — it never miscompiles it — but a program using one of these
runs only under `grasm run`:

| shape | example |
|-------|---------|
| a generic **type** declaration | `type Pair[A, B] = { fst: A, snd: B }` |
| `fail` inside a *fallible* closure passed to `map`, `filter` or `fold` | `xs.map(\|x\| fail(e))` where the result is held as `[T ! E]` — produce `Err(e)` instead of failing, or pass a named function |
| polymorphic recursion | `fn f[T](x: T) -> i64 { f([x]) }` — one compiled copy per type, and the types never stop growing |

Each of these is a defect being worked off, not a design boundary. The list is
checked against the implementation rather than remembered: if one of them
starts compiling, this table is what has to change.

## Where to go next

- `spec/` — the normative rules, each tagged and backed by a program in
  `spec/conformance/` that the test suite runs.
- `examples/` — runnable programs, including `stdlib_tour.gr` and
  `testing_tour.gr`.
- `projects/` — twelve larger programs, beginner through advanced.
- `docs/replay.md` — recording a run and replaying it, in full.
- `docs/wasm.md` — running a Grasm program in a browser tab.
- `MASTERPLAN.md` — where the language is going and why.
