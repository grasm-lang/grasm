# ProjectGold — thirty programs that try to break the language

A language is not tested by its own test suite. Its own test suite was written
by the people who knew where the edges were, and it therefore probes exactly the
places they already thought about. Thirty ordinary programs, written to a list
somebody else wrote, probe the places nobody thought about — which is why this
directory exists and why every failure in it is recorded rather than smoothed
over.

Every program here is held to the same standard as the rest of the repository:
**`grasm run`, `grasm build`, and `grasm build --native` must agree byte for
byte on stdout and on the exit code.** `./run.sh` checks that, and a program
that only works under one engine is a defect with a number, not a program.

## Before any of it: why WebAssembly at all, and why not MLIR

This came up as a direct question and it deserves a direct answer, because the
answer changes what gets built.

### What exists today is not a wasm compiler

`grasm_wasm` is the **reference interpreter compiled to wasm32**. It is not code
generation. That distinction is the whole of its value: `grasm_interp` depends on
`grasm_syntax` and `grasm_sema` and nothing else, and neither touches the host,
so the entire frontend and semantics crossed over with no reimplementation.
Nothing in it *can* disagree with `grasm run` about what a program means, because
it is `grasm run`. A wasm code generator would be a fourth engine and would have
to earn its place in the agreement gate first.

So "should Grasm emit wasm directly" is still an open question, and it is the one
MLIR is being proposed for.

### MLIR is not a code generator, and that is the crux

MLIR is infrastructure for **defining IRs** — dialects — and lowering between
them. It does not produce machine code or wasm. To get either, you lower MLIR to
LLVM IR and use LLVM's backend. The 2025 WebAssembly Workshop paper proposing a
native Wasm dialect says so in its own motivation: *"MLIR already supports
compiling to Wasm through the LLVM backend."* The native Wasm dialect is an RFC,
not a shipped thing.

So "use MLIR instead of a wasm backend" is a category error. MLIR would sit
*above* the backend, and LLVM would still be underneath.

### And LLVM would cost the three things Grasm has actually measured

| | measured today | with LLVM underneath |
|---|---|---|
| frontend throughput (G3) | **571k LOC/s** | unchanged — LLVM is not the frontend |
| compile time | **~66 ms/program**, ~4× faster than Rust | seconds; LLVM is most of why Rust is slow |
| toolchain | one binary, Cranelift is a crate | a ~100 MB dependency with a long build |
| binary size (G5) | 328 KB, 28 KB from target | unchanged — the gap is Rust's `std` |

MLIR earns its keep when a language has many high-level domain abstractions to
lower progressively — tensors, GPU kernels, polyhedral loops — and wants
optimisation *across* those levels. Grasm's measured bottleneck is not
optimisation. It is that `wordfreq` boxes every string, which is a value
representation problem MLIR does not touch. Adopting MLIR to fix a
representation problem is buying a crane to open a jar.

### What "more modern and faster" actually looks like *for Grasm*

Three bets, and two of them are specific to this language in a way MLIR is not:

**1. Emit wasm directly, no LLVM.** Wasm is a stack machine; emitting it is far
easier than emitting x86. AssemblyScript, Grain and MoonBit all do it through
**Binaryen**, which is explicitly built to be "powerful enough to be used as a
compiler backend by itself" and carries wasm-specific optimisations a
general-purpose compiler would not do. This is the boring, correct route.

**2. WasmFX — stack switching, which is effect handlers, and it is at phase 3.**
This is the one worth being excited about. WebAssembly is gaining native
continuations *specified in terms of effect handlers*, with a Wasmtime
implementation. Every other language targeting wasm has to do a whole-program
CPS transformation to get async, generators or lightweight threads across. The
MLIR Wasm-dialect proposal names this exact difficulty: continuations *"could be
done easily using stack-switching constructs — which are nontrivial to generate
from LLVM IR."*

Grasm's differentiator is an algebraic effect system with handlers. `with E
handled_by h` maps onto `cont.new` / `suspend` / `resume` close to one-to-one.
**Every other language pays a transformation to reach what Grasm already has in
its type system.** Going through LLVM would throw that advantage away at the one
step where it pays.

**3. The Component Model and WASI as the capability boundary.** Grasm's
capability ceilings — a package that may perform `io` and may not perform `net` —
are the same shape as WASI's capability imports. `[T-CAPABILITY]` and WASI
Preview 2 are two spellings of one idea, and nothing else in the language needs
to change for them to meet.

**Conclusion.** MLIR is the wrong axis: it is above the backend, it drags LLVM
underneath, and it would trade measured compile-time wins for optimisation Grasm
is not currently limited by. The modern bet is direct wasm emission with the
effect system landing on stack switching and the capability system landing on
the component model. That is a story no other language can tell, and MLIR would
not help tell it.

### "Before wasm, how did Grasm not work?"

It worked. It could not be *shown* to work without a checkout.

Running a Grasm program meant cloning a repository and building a Rust
workspace. The calculator went through three backends to get a browser in front
of it — Python, then a dependency-free Rust server, then `server.gr` once `net`
became a capability — and every one of them was scaffolding around the fact that
the language could not reach a browser. G1 (time for a developer to ship) and
G10 (adoption) are the two goals that measure whether the language is any good to
use, and both are unmeasured because nobody outside this repository has run a
Grasm program. That was the thing that did not work.

## What these thirty projects will actually test

Six gaps are already known before a line is written, found by reading the list
against the language:

| gap | blocks | status |
|---|---|---|
| **no bitwise operators** — no `>>`, `<<`, `&`, `\|`, `^` | 18 (BMP), 25 (DNS), 27 (SHA-256) | must be added |
| **no byte type** — a socket and a file both carry `str` | 18, 21 (binary), 25 | design work |
| **no random** | 2, 9 | in-language PRNG; tests overflow |
| **no TCP connect** — `net` listens and accepts only | 22 (scraper), 30 (chat client) | small addition |
| **no UDP** | 25 (DNS) | small addition |
| **no process spawn** | 17 (shell) | `extern fn` may reach it |

The other twenty-four should be writable today. Where one is not, the reason gets
written down in that project's own `README.md` and lands in
`docs/known-issues.md` with a severity, exactly like any other defect.

## Layout

```
ProjectGold/
  MASTERPLAN.md      this file
  RESULTS.md         the scoreboard, regenerated by run.sh
  run.sh             builds and runs every project under all three engines
  NN-name/
    main.gr          the program
    expected.txt     exact expected stdout
    input.txt        stdin, when the program reads any
    README.md        what it tests, what it found, what it could not do
```

## Phase 2 — full-stack

Thirteen full-stack projects follow, and they are deliberately second. They need
a browser story and a server story to be true at the same time, and both halves
now exist: `grasm_wasm` in the tab and `net` on the server. The gaps that phase
will surface — sessions, binary upload, WebSockets, non-blocking I/O — are listed
in `PHASE2.md` once phase 1 has told us which of them the language can already
carry.
