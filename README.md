# Grasm

A statically typed language with **Rust-class safety ideas at Go-class effort**:
value semantics with ownership (`own`/`inout`, no lifetime annotations), an
**algebraic effect system** (capability-checked I/O, handler-based DI), typed
error unions (`T ! E` with `try`/`fail`), generics, ADTs with exhaustive
matching — and a single-binary toolchain.

This repository contains the working Grasm-Core implementation (Rust) plus the
full strategic blueprint ([MASTERPLAN.md](MASTERPLAN.md)).

```grasm
effect Clock { fn now() -> i64 }

type Shape = | Circle(r: f64) | Rect(w: f64, h: f64)

fn area(s: Shape) -> f64 {
    match s {
        Circle(r) -> 3.14159 * r ^ 2
        Rect(w, h) -> w * h
    }
}

fn load(path: str) -> str ! IoError / {io} {
    try read_file(path)
}

fn main() {
    with Clock handled_by ClockHandler{ now: || 1000 } {
        print("t={Clock.now()}: total {[Circle(1.0), Rect(2.0, 3.0)].map(|s| area(s)).sum()}")
    }
}
```

## Where it actually stands

`docs/northstar.md` records the MASTERPLAN's own measurable goals against
measurements, taken by `./scripts/northstar.sh` rather than remembered. Three
are met, three are missed with numbers attached, and four measure features that
are not built. `spec/core.md` does the same for the kernel-language commitment:
36 constructs against a budget of 30, with the six that would close the gap
named.

## Install

Download the archive for your platform, unpack it, and put both files
somewhere on your `PATH` — `grasm` links `libgrasm_rt.a` for native builds and
finds it beside itself.

```
tar -xzf grasm-macos-arm64.tar.gz
cd grasm-macos-arm64
./grasm run hello.gr
```

From source:

```
git clone <repo> && cd grasm
cargo build --release --workspace     # produces target/release/grasm
```

`grasm run` needs nothing but the binary. `grasm build --native` also needs
`libgrasm_rt.a`, which sits next to `grasm` in both layouts; set
`GRASM_RUNTIME` to override.

### Your first program

```grasm
fn main() {
    print("hello, world")
}
```

```
grasm run hello.gr                    # typecheck and run
grasm build hello.gr --native -o hello && ./hello
```

`dist/grasm-language-pack.md` is the whole language — grammar, types,
semantics and a tutorial — in about 40k tokens. It is small enough to hand to an
AI assistant in full, which is how you get useful completions for a language it
has never seen. `dist/grasm-conformance-pack.md` carries the runnable program for
every rule, another 37k tokens; you need it when a rule is in dispute and you
want it settled by a program rather than by prose.

`grasm_wasm` builds the interpreter for `wasm32-unknown-unknown`, so a Grasm
program runs in a browser tab with no server: `cargo build --release --target
wasm32-unknown-unknown -p grasm_wasm`, then see [docs/wasm.md](docs/wasm.md).
It is the reference interpreter rather than a second implementation, and
`scripts/wasmtest.sh` holds it to the same agreement gate as everything else —
183 identical, 0 mismatched.

## Toolchain

```
cargo build --release            # builds ./target/release/grasm
grasm run <file.gr> [-- args]    # typecheck + execute (follows imports)
grasm test <file.gr>             # run `test "name" { }` blocks
        --coverage               # report the statement lines tests reached
        --seed N --cases N       # property generation (replayable)
grasm bench <file.gr>            # run `bench "name" { }` blocks, ns/op
grasm check <file.gr>            # diagnostics only (caret + help lines)
grasm build <file.gr> [-o out]   # self-contained single-file executable
grasm doc <file.gr> [-o out.md]  # API docs from `///` comments
grasm fmt <file.gr> [--check]    # canonical indentation (comment-safe)
grasm lsp                        # language server (stdio)
grasm dap                        # debug adapter (stdio) — breakpoints, stepping
grasm deps [dir]                 # resolve deps, check capabilities, write grasm.lock
        --locked                 # fail if anything changed since grasm.lock (CI)
grasm publish [dir]              # copy a package into a registry (versions are immutable)
grasm add <name> <version>       # add a registry dependency to grasm.toml
grasm tokens <file.gr>           # lexer debug dump
```

Phase 2 additions: **modules** (`import utils` / `import lib.math`, qualified
`utils.fn()` access, `pub` enforced cross-module), **`Map[K, V]`** (typed,
deterministic iteration), and **`grasm build`** (bundles all modules with the
runtime into one distributable executable).

### Performance

Six programs written the same way in Grasm, C, Rust, Go, Nim, Zig and Python.
Best of 5, milliseconds, Apple M-series. Nim and Zig implement the four
programs where the comparison is about the language rather than about which
concurrency primitive each one reaches for.

| program  |   C |  Rust |  Go | Nim | Zig | grasm-native | Python |
|----------|----:|------:|----:|----:|----:|-------------:|-------:|
| fib      |   5 |     5 |   6 |   6 |   5 |           11 |    131 |
| sieve    |   7 |     7 |   8 |   9 |   7 |           17 |    365 |
| matmul   |   5 |     5 |   8 |   9 |   6 |           10 |    942 |
| wordfreq |  31 |    26 |  18 |  23 |  10 |           54 |    135 |
| sort     |  37 |    12 |  50 |   — |   — |       **20** |    223 |
| parmap   |  51 |    51 |  90 |   — |   — |          105 |  3,455 |

Best of 7. **The harness floor is 2 ms** — that is what timing `/usr/bin/true`
costs, and it is fork, exec and wait rather than anything a program did. So the
top three rows are mostly floor: C's 5 ms `fib` is about 3 ms of work and
Grasm's 11 ms is about 9 ms, which is nearer 3x than the 2.2x the raw numbers
show. Subtract 2 from every cell before taking a ratio on a short row.

Two runs an hour apart agreed within 2 ms on every cell — and that on its own
proves nothing, which this file learned the hard way: the previous table was
also reproduced, and was wrong by a constant 21 ms in every cell (F27). What
makes these numbers believable is that the *timer* is now checked too. The
harness measures a program that does nothing before it measures anything else,
and refuses to run if that takes more than 10 ms.

`./scripts/bench.sh` builds every implementation, checks each one against the
interpreter's answer, and refuses to report a timing for a wrong one — an
optimiser that deletes the work being measured is the classic way a benchmark
lies. It also refuses to run at all when the machine's load average is above a
third of its core count, because a busy machine does not just make everything
slower, it makes everything slower *unevenly*, and the multi-core rows collapse
in a way that reads as a regression that is not there.

Neither of those caught the error above. Both are checks on the *inputs* to a
measurement — is the answer right, is the machine quiet — and the timer itself
was never checked against a program of known duration. It is now: the C sieve
takes about 5 ms, and a harness that says 30 is measuring itself.

Native Grasm **beats C and Go on `sort`** — 20 ms against 37 and 50, with Rust
ahead at 12. Everywhere else it is roughly **2x C** on the scalar programs.
Scalar lists store elements inline and a list a loop only indexes is
reached through its data pointer, so `matmul`'s innermost loop is 17
instructions with no calls.

`wordfreq` is the honest weak spot. It is allocation: every value is a
refcounted box and the loop builds a string per iteration.
Small-string optimisation took it from 117 ms to 98 ms — both figures on the
old harness, so both carry its 21 ms; the *difference* is real — by removing the *second*
allocation for text under sixteen bytes — the storage change alone made it
slower, and the win came from the constructors that stopped building a `String`
only to copy it (`docs/northstar.md` records the measurements, including the
attempts that were reverted). `benchmarks/README.md` records two things this
table used to blame that measurement cleared — the bounds check and the absence
of SIMD, each worth 0 ms.

Compile time is ~66 ms/program — about Go's with a warm cache, faster than C,
roughly 4× faster than Rust. Binaries are ~457 KB after link-time
dead-stripping — Rust's size, a fifth of Go's. `benchmarks/README.md` also
covers concision, keyword surface, and —
explicitly — what is *not* measured, learning curve included.

### Debugging

A panic prints a stack trace to stderr, innermost frame first:

```text
panic: index 10 out of bounds (len 3)
stack trace (innermost first):
  main::level_three at trace.gr:2:5
  main::level_two   at trace.gr:6:5
  main::level_one   at trace.gr:11:5
  main::main        at trace.gr:16:5
```

`grasm dap` is a Debug Adapter Protocol server over stdio: breakpoints,
step in/over/out, stack frames, locals, and variable lookup. It drives the
reference interpreter through a hook, so what you see while stepping is exactly
what `grasm run` does — there is no separate debug evaluator that can disagree
with the real one.

**Native gap:** stack traces are an interpreter feature. A natively compiled
binary prints the panic message and exits 101 without a trace, because frame
tracking would cost a store per statement in released code.
`scripts/difftest.sh` compares stdout and exit codes strictly and excludes only
this trace block from its stderr comparison.

### Testing

One declaration form covers three kinds of test. `test "name" { ... }` is a
unit test; adding parameters makes it a **property** test, and the runner
generates the inputs:

```
test "reversing twice is the identity" (xs: [i64]) {
    assert(reverse(reverse(xs)) == xs)
}
```

Pointing a property at `str` is fuzzing — the generator emits tabs, newlines,
quotes, backslashes, and multi-byte characters. A failure is reported with a
*shrunk* counterexample (the smallest input that still fails) plus the seed
that found it, so `--seed N --cases N` replays it exactly. `bench "name"
{ ... }` bodies are warmed up, then sampled and reported in ns/op.
See `examples/testing_tour.gr`.

### Documentation

`///` comments document the declaration that follows. `grasm doc` renders them
as Markdown with signatures taken from the AST, so a documented effect row or
error type cannot drift from the real one.

````
/// Adds two integers.
///
/// ```
/// assert(add(2, 3) == 5)
/// ```
fn add(a: i64, b: i64) -> i64 {
    a + b
}
````

The fenced block is **executable** — `grasm test` runs it in a program that
keeps the real module structure, so an example may write `mathlib.square(4)`
exactly as a consumer would. Documentation that stops being true fails the
suite. Lines beginning `# ` are hidden setup: they run but are not shown.

`docs/tutorial.md` is a guided tour whose code blocks are compiled and run by
the test suite. See `examples/documented.gr`.

### Native compilation

`grasm build <file.gr> --native` compiles to machine code through Cranelift and
links against `libgrasm_rt.a`. The backend covers scalars, strings, lists, maps,
tuples, structs, sum types, `match` (guards + nested patterns), `Option`,
`T ! E` fallibles with `try`/`fail`, effects with handlers, `inout`/`own`
params, `map`/`filter` (lowered as inline loops), and multi-module programs.

**Targets.** `--target <triple>` emits an object for any supported
architecture — verified for `x86_64` (Linux + macOS), `aarch64` (Linux +
macOS), `riscv64gc`, and big-endian `s390x`. Host builds link automatically;
cross builds emit the object for you to link with that target's toolchain and
a `libgrasm_rt.a` built for it:

```
rustup target add x86_64-apple-darwin
cargo build --release -p grasm_rt --target x86_64-apple-darwin
grasm build app.gr --native --target x86_64-apple-darwin -o app
cc -arch x86_64 app.o target/x86_64-apple-darwin/release/libgrasm_rt.a -o app
```

The backend also compiles structured concurrency, generic functions
(monomorphized per instantiation), and function values — a named function used
as data, a closure bound to a name, and lists of either, dispatched through an
indirect call.

`scripts/difftest.sh` is the correctness gate: every program in `examples/`,
`projects/`, `spec/conformance/`, and `benchmarks/` is run through both engines
and the outputs must match byte for byte. Current status — **130 identical, 0
mismatched, 0 unsupported**: nothing in the repository is left to the
interpreter. Anything the backend cannot compile names the missing feature and
points at the interpreter; it never silently miscompiles.

Those 130 programs are still a sample, and a sample only finds the
disagreements someone thought to look for. `scripts/fuzz_differential.py`
generates programs instead — well-typed by construction, and total by
construction, so codegen is the only thing left to disagree about. It earned
its place on its first run by finding a construct that ran under the
interpreter and would not compile natively, which the 130 had all missed.

`scripts/fuzz_fallible.py` narrows the aim at the shape that has produced the
most miscompiles by a wide margin: a fallible function whose value-position
`if`, `match`, `return` or annotated `let` mixes a bare ok half with `Ok(..)`,
`Err(..)`, `fail(..)` and a fallible call. Each of those positions is a
separate conversion in the backend and each has been wrong at least once —
turning an `Err` into an `Ok` carrying a heap address, inverting an exit code,
or aborting on a misaligned pointer. It found three of them on its first run.

`scripts/fuzz_generics.py` aims at the other place the backend has to work out
a type it was never told: a generic body instantiated at two types, a function
held in a binding and called later, a closure passed through a helper, and a
place written back through `inout`. Where the type is written down the backend
reads it; in these four shapes it probes for one, and a probe is a guess.

`scripts/fuzz_rows.py` is not a differential test but a *soundness* one, over
the property the capability ceiling rests on: a function value's effect row
travels with it, whatever carries it and whatever binds it. It enumerates the
grid — fourteen carriers (a list, a tuple, a struct field, a sum payload, a map
value, an `Option`, a `type` alias, a block-valued initializer, a bare
binding) against five call sites (a
direct call, `map`, `par_map`, a helper, a closure) — and checks both
directions: an effectful value must be refused inside a `/ {}` function, and a
pure one must be accepted. Five consecutive cold-evaluation rounds found a hole
in that property, and each was one cell of a grid nobody had written down.
`--packages` runs each carrier again across a package boundary, which is the
question the ceiling is actually for: a `capabilities = []` dependency that
holds an effectful function value has to fail the build in its *consumer*.

`scripts/fuzz_handlers.py` covers what the other generators' grammars cannot
reach: effect handlers and pattern binders, which is where both of this
project's SIGSEGVs came from — an operation declared `-> () ! E` whose body
gave the bare `()` half, and a bare identifier in a pattern that happened to
name a nullary variant of another type. It varies where the handler literal is
written, whether a captured binding changes between that point and the `with`,
what the operation returns, and which pattern shapes the body matches.

`scripts/fuzz_extern.py` enumerates the other half of that property: every
spelling by which a declared C function can be named or called. Naming one is
the permission to call it, so each spelling costs the same row — but only some
were charged. `n.labs()` charged nothing while `labs(n)` charged `io`, so a
package declaring `capabilities = ["unsafe"]` was reported as reaching no `io`
while its binary called libc; and `unsafe { sqrt }` carried one onto
`par_map`'s workers. Eighteen spellings, each checked against a row missing
`io` (which must fail) and against the complete row (which must compile and
run).

`scripts/crosstest.sh` runs the same gate against a *cross-compiled* target to
prove the backend isn't accidentally host-specific — currently **130 identical,
0 mismatched, 0 unsupported** for x86_64 built on arm64. CI
(`.github/workflows/ci.yml`) runs
build + tests + clippy + the differential gate on linux-x86_64, macos-arm64,
and macos-x86_64, plus object emission for all five architectures.

Measured on this machine (user CPU time, release builds):

| benchmark | native | interpreter |
|---|---|---|
| `fib(32)` | 0.01s | 2.72s |
| 200k list pushes + sum | 0.00s | 0.08s |
| 300k filter + map pipeline | 0.01s | 0.15s |

## Concurrency

`parallel { spawn { ... } spawn { ... } }` is a **structured concurrency**
scope: every task spawned inside is joined before the scope exits, task bodies
are typechecked, `break`/`continue` cannot cross a task boundary, and spawning
is tracked as the `async` effect. Scopes nest, and both engines compile it.

A task body may only use bindings that existed before the `parallel` block
began, plus its own locals. Tasks run *after* the block finishes, so anything
the block declared — a loop variable included — is already gone; that is a
compile error ([T-TASK-CAPTURE]) rather than a runtime failure.

```grasm
var results: [i64] = []
parallel {
    spawn { results.push(work(10)) }
    spawn { results.push(work(100)) }
}
// both tasks are guaranteed complete here
```

**`parallel`/`spawn` is concurrency and determinism, not parallelism — on
purpose.** Tasks run on one thread in spawn order, so the same program always
produces the same output. That is not a missing feature: tasks share the
enclosing scope by design ([E-SPAWN-SHARED]), so running them concurrently
would race. Structure and reproducibility live here.

Throughput lives in `par_map`:

```grasm
let out = xs.par_map(|x| expensive(x))    // 5.9x on 8 cores
```

Two properties make that safe without a `Send` bound, a mutex, or a race
detector — and both are checked, not hoped for:

- **The function must be effect-free, and the compiler proves it.** Put a
  `print` inside and the build fails. The effect system already knows what a
  function does, so parallel-safety is a type-level fact.
- **Results come back in input order whatever the core count**
  ([E-PAR-MAP-ORDER]), so the answer never depends on the machine that ran it.

Anything the threaded path cannot take — boxed elements, boxed captures —
falls back to a sequential map. Slower, never wrong.

`parallel` and `spawn` are *contextual* keywords — they only introduce a scope
when a block follows, so code that already uses those words as function or
variable names keeps compiling.

## What the language guarantees today

- **No nulls** — `Option[T]` + `??` / `?.`-style access via `.get()`.
- **Errors are values** — `T ! E` unions; unhandled fallibles are compile
  errors; `try` propagates, `fail(e)` raises, `match Ok/Err` handles.
- **Ownership Tier-0** — `own` params move (use-after-move is a compile
  error), `inout` params are exclusive mutable places with real write-back,
  everything else copies (copy-on-write under the hood).
- **Effects as capabilities** — `print`/fs are the `io` effect; user effects
  (`effect Clock { ... }`) must be handled (`with E handled_by H { }`) before
  `main`; declared rows (`/ {io}`) are enforced upper bounds. A function
  cannot secretly touch the network of tomorrow.
- **Exhaustive matching** — sums, bools, Ok/Err, and bool-tuple products are
  coverage-checked with precise missing-case messages.

## Packages and capabilities

A package carries a `grasm.toml` manifest declaring its version, path
dependencies, and — the interesting part — the effects it is permitted to
perform:

```toml
[package]
name = "mathlib"
version = "0.2.0"
capabilities = []          # computes; touches nothing

[dependencies.utils]
path = "../utils"
version = "^0.3.0"         # 0.3.x, and no other line
```

`version` is optional and comes in two forms. `=0.3.1` is that version and no
other; `^0.3.1` — also written bare as `0.3.1` — is that version or any later
one that does not change the leading non-zero number. That is the same
compatibility rule [the stability policy](docs/stability.md) states for the
language, so `^0.3.1` stops at `0.4.0` and `^1.2.3` stops at `2.0.0`.

A name resolves to exactly one package. Reaching one name through two
different directories is an error naming both, rather than the build linking
whichever copy the walk saw first and handing the other dependent something it
never asked for.

Drop the `path` and the dependency comes from a **registry** — a directory of
`<name>/<version>/` packages, found via `GRASM_REGISTRY` or the nearest
`.grasm/registry`. It is a directory rather than a service on purpose:
fetching packages is a different problem from deciding which ones to use, and
only the second is the language's business.

Versions are chosen by **minimal version selection**. Each requirement names a
lowest acceptable version; each package gets the highest floor anyone stated,
resolved to the lowest version the registry actually holds at or above it, and
that choice is then checked against every requirement. No search, no
backtracking — so the result is the same on every machine, and **adding a
dependency cannot silently upgrade one you already had**. Requirements that
cannot all hold are an error listing who asked for what:

```
error: no version of `text` satisfies every requirement:
  `app` requires ^2.0.0
  `old` requires ^1.0.0
```

This is where the capability ceiling starts earning its keep, because it now
applies to code you did not write. A registry package that ships a patch
release quietly reading the filesystem while still declaring
`capabilities = []` fails **every consumer's build**; and a published version
whose contents change under the same version number fails
`grasm deps --locked` on its checksum.

`grasm deps` resolves the graph (dependencies before dependents, cycles,
unsatisfied version requirements, and name mismatches reported), infers what
each package actually does from its source, and **fails the build if any
package exceeds its declared ceiling** —
in every consumer, at compile time. A dependency that quietly starts reading
files in a patch release breaks the build rather than shipping. See
[`examples/workspace/`](examples/workspace/) for a runnable demonstration.

A ceiling is written in **effects**, so it is not a fixed permission list.
Declare `effect Network` and a package can be bounded by
`capabilities = ["Network"]` exactly as it is by `io`. The names are checked
against the effects the program actually defines, so a typo cannot masquerade
as a granted permission:

```
error: package `lib` declares capability `Netwrok`, which is not an effect
       this program defines — did you mean `Network`?
```

The lockfile records a SHA-256 over each package's sorted sources, so a change
to a dependency's contents is detectable.

Registries and version-range resolution are not implemented; dependencies are
path-based today.

### The supply-chain benchmark

The one row no other language in the comparison can fill.
`./scripts/security_bench.sh` runs the *same* tampered-dependency attack in
four languages: a trusted pure library's next version reads `/etc/passwd`,
keeping its signature and result.

| language | toolchain says | why |
|----------|----------------|-----|
| **Grasm** | **COMPILE ERROR** | dependency performs `[io]`, permits `[]` |
| Rust | builds clean | `std::fs` is ambient |
| Go | builds clean | `os.ReadFile` importable anywhere |
| Python | compiles clean | `open` needs no declaration |

Only Grasm's build fails, because the dependency's inferred `io` exceeds its
declared ceiling. The others put this on a different layer (runtime sandbox,
review, `cargo-vet`); the narrow, measured claim is that Grasm's *toolchain*
catches it and theirs do not. See `benchmarks/security/`.

## Specification

[`spec/`](spec/) is the normative description of the language: lexical rules and
grammar, the type system, and evaluation semantics. Every rule carries a stable
tag such as **[E-ARITH-INT]**, and `cargo test -p grasm_spec` fails if a tagged
rule has no conformance program, if a program names a rule that no longer
exists, or if any program's behavior differs from what it declares. The
specification therefore cannot drift from the implementation without a red test.

Currently **65 normative clauses, 65 conformance programs**.

## Layout

| Path | What |
|---|---|
| `crates/grasm_syntax` | lexer, parser, AST, diagnostics, formatter |
| `crates/grasm_sema` | typechecker (inference, generics, effects rows, ownership) |
| `crates/grasm_interp` | definitional tree-walking interpreter + test runner |
| `crates/grasm_native` | Cranelift AOT backend (`grasm build --native`) |
| `crates/grasm_rt` | C-ABI runtime linked into native executables |
| `crates/grasm_lsp` | language server (`grasm lsp`) |
| `crates/grasm_pkg` | manifests, resolution, capability ceilings, lockfiles |
| `crates/grasm_spec` | specification conformance harness |
| `crates/grasm` | the `grasm` CLI |
| `examples/` | small demos (hello, errors, effects, ownership) |
| `projects/beginner` | validation: games, todo, text stats, unit converter |
| `projects/intermediate` | validation: JSON parser, CSV pipeline, KV store, calculator |
| `projects/advanced` | validation: router+middleware, scheduler, WAL mini-DB, **Mini-Grasm interpreter written in Grasm** |
| `spec/` | normative specification + one conformance program per clause |
| `editors/` | VS Code extension and LSP setup for other editors |
| `scripts/` | `difftest.sh` (native vs interpreter), `crosstest.sh` (cross-target), `fuzz_differential.py`, `fuzz_fallible.py`, `fuzz_generics.py`, `fuzz_rows.py`, `fuzz_handlers.py` and `fuzz_extern.py` (generated programs) |
| `MASTERPLAN.md` | the full v0.3 strategic blueprint this implements Phase 0–1 of |

## Status

Grasm-Core (the master plan's §19 kernel language), Phase-2 features (modules,
maps, self-contained builds), and the **Cranelift native backend** are complete
and validated: 175 Rust tests + 40+ in-language test blocks across 17 programs,
an adversarial fuzz/stress suite, a native-vs-interpreter differential gate, and
zero clippy warnings, and a 3-platform CI matrix. The interpreter remains the
reference semantics; the native backend is checked against it on every program,
on both the host and a cross-compiled target. Next milestones (see MASTERPLAN
§13): LSP, concurrency runtime, package manager, first-class function values.

## Project

Apache-2.0 (`LICENSE`, `NOTICE`). Pre-1.0 — `docs/stability.md` says exactly
what that means, and `spec/stability.md` records the level of every normative
clause. A clause with no recorded level fails the test suite, so the policy
cannot drift away from the specification.

- `CONTRIBUTING.md` — the gates, and the one rule the rest follows from
- `GOVERNANCE.md` — how decisions get made today, without pretending otherwise
- `SECURITY.md` — the threat model, and what it does not cover
- `CODE_OF_CONDUCT.md`, `CHANGELOG.md`
