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

`scripts/difftest.sh` is the correctness gate: every program in `examples/` and
`projects/` is run through both backends and the outputs must match byte for
byte. Current status — **16 identical, 0 mismatched, 1 unsupported** (the router
project needs first-class function values). Anything the backend can't compile
names the missing feature and points at the interpreter; it never silently
miscompiles.

Measured on this machine (user CPU time, release builds):

| benchmark | native | interpreter |
|---|---|---|
| `fib(32)` | 0.01s | 2.72s |
| 200k list pushes + sum | 0.00s | 0.08s |
| 300k filter + map pipeline | 0.01s | 0.15s |

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
| `MASTERPLAN.md` | the full v0.3 strategic blueprint this implements Phase 0–1 of |

## Status

Grasm-Core (the master plan's §19 kernel language), Phase-2 features (modules,
maps, self-contained builds), and the **Cranelift native backend** are complete
and validated: 175 Rust tests + 40+ in-language test blocks across 17 programs,
an adversarial fuzz/stress suite, a native-vs-interpreter differential gate, and
zero clippy warnings. The interpreter remains the reference semantics; the
native backend is checked against it on every program. Next milestones (see
MASTERPLAN §13): LSP, package manager, first-class function values.
