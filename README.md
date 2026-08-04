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

## Toolchain

```
cargo build --release            # builds ./target/release/grasm
grasm run <file.gr> [-- args]    # typecheck + execute (follows imports)
grasm test <file.gr>             # run `test "name" { }` blocks
grasm check <file.gr>            # diagnostics only (caret + help lines)
grasm build <file.gr> [-o out]   # self-contained single-file executable
grasm fmt <file.gr> [--check]    # canonical indentation (comment-safe)
grasm tokens <file.gr>           # lexer debug dump
```

Phase 2 additions: **modules** (`import utils` / `import lib.math`, qualified
`utils.fn()` access, `pub` enforced cross-module), **`Map[K, V]`** (typed,
deterministic iteration), and **`grasm build`** (bundles all modules with the
runtime into one distributable executable).

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

`scripts/difftest.sh` is the correctness gate: every program in `examples/` and
`projects/` is run through both backends and the outputs must match byte for
byte. Current status — **16 identical, 0 mismatched, 1 unsupported** (the router
project needs first-class function values). Anything the backend can't compile
names the missing feature and points at the interpreter; it never silently
miscompiles.

`scripts/crosstest.sh` runs the same gate against a *cross-compiled* target to
prove the backend isn't accidentally host-specific — currently **16 identical,
0 mismatched** for x86_64 built on arm64. CI (`.github/workflows/ci.yml`) runs
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
is tracked as the `async` effect. Scopes nest.

```grasm
var results: [i64] = []
parallel {
    spawn { results.push(work(10)) }
    spawn { results.push(work(100)) }
}
// both tasks are guaranteed complete here
```

**Honest limitation: this is concurrency and determinism, not parallelism.**
Tasks run cooperatively on one thread in spawn order, so the same program
always produces the same output (which is what MASTERPLAN §8.6 wants for
reproducible tests) but nothing runs on a second core. Real parallelism needs
the runtime's `Rc`-based values replaced with `Arc` + `Send`/`Sync`, which
would slow down all single-threaded code — that trade is a deliberate later
milestone, not an oversight. The native backend reports `parallel`/`spawn` as
unsupported and defers to the interpreter.

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
| `crates/grasm` | the `grasm` CLI |
| `examples/` | small demos (hello, errors, effects, ownership) |
| `projects/beginner` | validation: games, todo, text stats, unit converter |
| `projects/intermediate` | validation: JSON parser, CSV pipeline, KV store, calculator |
| `projects/advanced` | validation: router+middleware, scheduler, WAL mini-DB, **Mini-Grasm interpreter written in Grasm** |
| `spec/` | normative specification + one conformance program per clause |
| `editors/` | VS Code extension and LSP setup for other editors |
| `scripts/` | `difftest.sh` (native vs interpreter), `crosstest.sh` (cross-target) |
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
