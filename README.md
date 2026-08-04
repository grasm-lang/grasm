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
grasm run <file.gr> [-- args]    # typecheck + execute
grasm test <file.gr>             # run `test "name" { }` blocks
grasm check <file.gr>            # diagnostics only (caret + help lines)
grasm fmt <file.gr> [--check]    # canonical indentation (comment-safe)
grasm tokens <file.gr>           # lexer debug dump
```

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
| `crates/grasm` | the `grasm` CLI |
| `examples/` | small demos (hello, errors, effects, ownership) |
| `projects/beginner` | validation: games, todo, text stats, unit converter |
| `projects/intermediate` | validation: JSON parser, CSV pipeline, KV store, calculator |
| `projects/advanced` | validation: router+middleware, scheduler, WAL mini-DB, **Mini-Grasm interpreter written in Grasm** |
| `MASTERPLAN.md` | the full v0.3 strategic blueprint this implements Phase 0–1 of |

## Status

Grasm-Core milestone (the master plan's §19 kernel language): **complete and
validated** — 143 Rust tests + 39 in-language test blocks across 16 programs,
adversarial fuzz/stress suite, zero clippy warnings. Tree-walking interpreter
only (native backends, LSP, package manager: see the roadmap in MASTERPLAN §13).
