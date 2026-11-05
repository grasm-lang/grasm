# Phase 4: a self-hosted Grasm toolchain

**Decision, 2026-10-20.** Phase 4 makes the whole toolchain Grasm: the
compiler, its machine-code backends, the linker and the runtime, all built
by `grasm` itself. This goes further than MASTERPLAN's Phase 4, which
self-hosted the frontend and kept LLVM through FFI "pragmatism over
purity"; this plan supersedes that line. Everything else in MASTERPLAN
still holds, principle 8 above all: Grasm emits machine code and object
files itself and never ships by translating to C or any other language.

This document is the plan. It says what "done" means, where the work
starts from (measured, not remembered), the architecture the new compiler
is built to, the order the pieces are replaced in, how each replacement is
proved before it ships, and what is still undecided.

---

## 1. The finish line

The toolchain is self-hosted when all of these are true at once:

1. Every line of the shipped `grasm` binary and of the runtime its
   programs link is Grasm source, compiled by `grasm`.
2. No Rust, Cranelift, LLVM, C compiler or system linker takes part in
   building `grasm` or in a user's `grasm build --native`.
3. **Fixed point:** the compiler built by the compiler built from source
   (stage 2) builds a stage 3 that is bit-identical to stage 2.
4. **Diverse double-compiling** (Wheeler, "Fully Countering Trusting Trust
   through Diverse Double-Compiling", 2009): the same compiler source built
   through the archived Rust implementation reaches the same stage-2 bytes.
5. Every gate passes with the self-hosted toolchain as the compiler under
   test, and the Rust crates are deleted from the main branch (kept on an
   archive tag for item 4).

What legitimately remains below Grasm, and why:

- **macOS:** `libSystem` (through `dyld`). The macOS system-call interface
  is not a stable ABI; Go learned this and moved to libSystem in 1.11.
  Linking against it is a load command in our own Mach-O output, not a C
  toolchain.
- **Linux:** the kernel's system-call ABI, which is stable, so Linux
  binaries can be fully static with no libc at all.
- The OS loader and the CPU. Nothing else.

## 2. Where it starts (measured 2026-10-20)

| Rust crate | lines | role |
|---|---:|---|
| grasm_native | 24,627 | Cranelift lowering, straight from the AST |
| grasm_sema | 21,833 | resolution, types, effects, interfaces, linear types |
| grasm_rt (+ rt_abi, sys, fiber, journal) | 16,879 | runtime: 316 `extern "C"` functions, OS layer, fibers |
| grasm_interp | 10,600 | tree-walking definitional interpreter |
| grasm_syntax | 7,249 | lexer, parser, formatter, diagnostics |
| grasm (CLI) | 5,369 | commands, incremental cache, link step |
| grasm_pkg, lsp, dap, spec, wasm | 9,005 | packages, editor protocols, conformance runner, playground |
| **total** | **~95,600** | plus Cranelift 0.116 and regalloc2 as dependencies |

Facts that shape the plan:

- **There is no intermediate representation.** The interpreter, the native
  backend and the wasm build all walk the AST, guided by about twenty
  span-keyed side tables from the checker, in which an expression's type is
  stored *as rendered text* and parsed back by the backend. The backend
  still runs a second type inference of its own (`probe_type`, ~1,190
  lines) and does its own monomorphization. `check_program` runs several
  times per command. A line-by-line port would carry all of this into the
  new compiler; it is the first thing not to copy.
- **The runtime ABI is undocumented.** Every boxed value is a 64-byte
  refcounted `RtVal`; strings inline up to 38 bytes; lists inline up to
  four elements; maps are hashbrown with a seeded wyhash mix and sorted
  iteration; calls use the C convention, a hidden stack-floor argument for
  recursive functions and a split tag/payload result for fallible ones.
  None of it is in `spec/`.
- **Linking is borrowed:** `cc <objects> libgrasm_rt_abi.a`, then `strip`.
- **Native compile speed:** about 40-45k lines/s for a full native build;
  `check` at 365-400k lines/s (docs/compile-speed.md).
- **Runtime speed:** geomean 0.965x Rust on the benchmark set, but the
  tree-walking benchmark is 0.51x Rust, and a compiler is mostly tree and
  graph walking.
- **What Grasm already hosts:** `tools/gfmt` (a line-level port of the
  formatter, byte-identical to `grasm fmt` on the corpus); a Grasm lexer in
  ProjectGold 29 checked by token *counts* only; libraries that read every
  object format we will write (Mach-O load commands, chained fixups, export
  tries, symbols, DWARF line and address tables, ELF, ar, fat files), and
  two that already write what ld writes byte-for-byte (`codesig`'s ad-hoc
  signature, `exporttrie`'s encoder). There is no relocation reader or
  writer, no Mach-O or ELF image writer, and **no arm64 encoder** at all;
  `asm64` covers about thirty x86-64 encodings.
- **Scale gap:** the largest program ever built natively is about 15k lines
  across 19 packages (`b46test`). The self-hosted toolchain will be roughly
  ten times that, and it will find compiler bugs the corpus never reached.
  That is part of the point.

## 3. Principles

1. **Machine code, never a transpiler.** Object files are written directly
   (Zig's path). A one-off experiment may emit C; nothing that ships may.
2. **Differential at every seam.** The project's strongest tool is two
   implementations that must agree. Every new component replaces an old
   one only after agreeing with it on the whole corpus, the conformance
   suite and the fuzzers, with mutation controls showing the comparison can
   fail.
3. **Strangler, not big bang.** The Rust toolchain stays the shipping
   compiler until the Grasm one passes every gate. Each stage below leaves
   a working, releasable tree.
4. **Language freeze during the migration.** New language features would
   have to be built twice (Zig's stated reason for retiring its C++
   compiler). Only the systems subset (§5) is added; everything else waits
   for the cutover.
5. **Data-oriented compiler.** Nodes, types and instructions live in arrays
   addressed by integer ids; names and types are interned; side data is
   structure-of-arrays, as Zig's and Carbon's compilers are organised. It
   also sidesteps Grasm's slowest path, refcounted tree walking.
6. **Determinism and reproducibility are features.** Same input, same
   bytes: no timestamps, no paths, no iteration order that depends on
   hashing, on every builder.
7. Correctness over speed; no unsound global state; hash maps stay
   resistant to flooding.

## 4. The architecture the new compiler is built to

The structure MASTERPLAN §10 describes and the Rust compiler never reached:

```
source ─► lexer ─► CST (lossless, error-tolerant) ─► AST (node ids)
      ─► resolve ─► types + effects + interfaces + linear ─► TIR
TIR (typed, resolved, monomorphized; serializable — the seam)
      ─► MIR (SSA with block arguments, ownership-aware, explicit RC)
      ─► optimization ─► lowering rules per ISA ─► VCode (virtual registers)
      ─► register allocation ─► encoding + branch relaxation
      ─► object writer (Mach-O, ELF) ─► built-in linker ─► executable
TIR ─► definitional interpreter (Grasm)          MIR ─► wasm32 backend
```

| Layer | Choice | Why |
|---|---|---|
| Parser | hand-written recursive descent over a lossless CST with error nodes; iterative where depth is unbounded | resilience for the LSP; the language guarantees only 10,000 nested calls, so deep expressions must not recurse per node |
| TIR | typed, name-resolved, monomorphized IR with integer ids; binary and text forms | the seam: two frontends must produce identical TIR, and a backend can be tested on TIR from either |
| Incrementality | batch first; then query-based with red-green invalidation (Salsa / rustc model) keyed on item signatures | parity before cleverness; queries are the LSP's foundation later |
| MIR | SSA with block arguments (Cranelift, MLIR style), explicit `inc`/`dec`, ownership facts | phi-free SSA keeps passes simple; explicit RC is what the optimizer needs to remove |
| RC optimization | Perceus-style precise RC with borrow inference and reuse (Reinking et al., PLDI 2021; as in Koka and Lean 4) | removes most refcount traffic and enables in-place update, which today's copy-on-write leaves to chance |
| Mid-end | SCCP, GVN/CSE, LICM, inlining with a cost model, DCE; bounds-check elimination, loop versioning and the f64x2 vectorizer ported from today's backend; later an acyclic e-graph rewrite stage (Cranelift's ægraphs, 2023) | proven, incremental steps; e-graphs once the basics are measured |
| Instruction selection | declarative lowering rules per ISA, kept as Grasm data and checked for overlap (the idea of Cranelift's ISLE and Go's `.rules`) | rules are reviewable and testable one by one |
| Register allocation | dev: linear scan with live-range splitting (Poletto-Sarkar, second-chance binpacking); release: backtracking allocator modelled on regalloc2; a symbolic checker that verifies every allocation (as regalloc2 ships) | fast builds and good code, with the allocator itself checked rather than trusted |
| Encoding | table-driven encoders for aarch64 and x86-64, checked against `llvm-mc`/`as` across every operand form | encoder bugs are silent; exhaustive differential checking makes them loud |
| Objects | Mach-O first, then ELF; DWARF line tables and frame info; compact unwind | debuggers and profilers keep working |
| Linker | built-in Mach-O linker (arm64, then x86-64): symbol resolution, dead-strip, chained fixups, `LC_LOAD_DYLIB` for libSystem, ad-hoc code signature (`codesig`), no UUID; then ELF; parallel by design (mold, Wild) and incremental later | removes `cc`; arm64 macOS refuses unsigned executables, which `codesig` already handles |
| Runtime | rewritten in the systems subset against today's ABI first, so it drops in for `libgrasm_rt_abi.a` and the corpus proves it; redesigned only after | one variable at a time |
| Math | a Grasm libm with correctly rounded functions (the CORE-MATH project's results) | identical answers on every platform, which also ends the arm64/x86-64 ulp drift crosstest found in `pyrandom.gauss` |
| Interpreter | rewritten over TIR in Grasm | stays the definitional engine of MASTERPLAN §10.1; the two-engine gates continue unchanged |
| Wasm | a wasm32 backend from MIR | replaces the Rust-built playground and gives a portable bootstrap seed (§7) |

## 5. Prerequisite: the systems subset

The compiler itself needs little that Grasm lacks; the runtime needs a
lot. Measured gaps (docs/known-issues.md, spec/types.md):

| Gap | Needed by | Addition |
|---|---|---|
| no `u64`; no rotate, popcount, clz, ctz, byte swap | hashing, encoders, object formats | `u64`; bit intrinsics on every integer type |
| no growable byte buffer; `s = s + x` is quadratic | every emitter: code, objects, diagnostics | a byte buffer with amortized append, and a string builder |
| no raw memory | runtime, allocator, linker output | an address type with typed loads and stores, offset arithmetic, `size_of`/`align_of`, only inside `unsafe` under a new capability |
| no atomics | refcounts, parallel tasks | atomic load, store, compare-exchange and fetch-add with explicit orderings |
| no mutable globals or thread-locals | allocator pools, per-thread state | `thread_local` cells under the same capability (the compiler itself passes context explicitly) |
| no C-ABI export, function pointers or callbacks | `main` entry, signal handlers, libSystem callbacks | `export fn` with the C convention; function addresses |
| no way to write a context switch | fibers | naked functions, or backend-provided intrinsics for save/restore |
| map keys limited to `i64`/`str`/`bool` | compiler tables | tuple keys, or a documented packing idiom |

Each addition gets spec clauses, implementations in both engines,
conformance tests and a fuzz gate, exactly as fixed-width integers did.
This is the only language growth during the migration.

### 5.1 Raw memory and atomics: the design (S1 part two)

Written before the code, 2026-10-21. The normative text is [T-ADDR],
[T-MEM-UNSAFE], [L-LIB-MEM], [L-LIB-ATOMIC] and [E-MEM-UNCHECKED]; this
section is why they say what they say.

**What it builds on.** Tier 3 already has its gate: a C function is usable
only inside `unsafe` ([T-UNSAFE-FFI]), an `unsafe` block puts `unsafe` in its
function's row, rows reach the package's ceiling ([T-CAPABILITY]), and a
dependency over its ceiling fails every consumer's build. Raw memory adds no
second mechanism. Every raw-memory operation must sit inside `unsafe`, with
the same diagnostic a C function gets, and performs a new built-in capability,
**`mem`**, through the same rows. A package that uses raw memory therefore
declares `capabilities = [..., "unsafe", "mem"]`, and one that does not
declare `mem` fails its consumers' builds as it would for `io`. `mem` is a
capability of its own for the reason `net` and `proc` are: a manifest saying
`["unsafe", "mem"]` and not `io` tells a reviewer the package touches memory
and does not call C. `mem` is reserved as an effect name (an `effect mem`
with a handler would discharge it), exempt from the region rule as `unsafe`
is (an address never points into a region's arena), and impure, so `par_map`
and the multi-core `parallel` path refuse it.

**Surface.** One type, five free functions, methods on the type. No new
syntax: the parser is unchanged.

- `addr`: an address, one machine word. It copies, `==` compares it, it
  prints as an integer the language does not specify, it may sit in a list,
  a field, a tuple or an Option; it is not a map key, has no order and no
  operators. Holding one is safe; using one is not.
- `mem_alloc(size, align)`, `mem_free(a)`, `mem_addr(n)` (an address from an
  integer: forging, which is the point of `unsafe`), `mem_size_of(T)`,
  `mem_align_of(T)`. The `mem_` prefix because `size_of` is already a `pub
  fn` in ten packages and a built-in name cannot be redeclared.
- `a.offset(n)`, `a.diff(b)`, `a.as_i64()`, and `a.load_T()`/`a.store_T(v)`
  for each *layout type* `T`: the eight integer types, `f64` and `addr`.
- `a.atomic_load_T(o)`, `a.atomic_store_T(v, o)`, `a.atomic_cas_T(want,
  new, o)` and `a.atomic_add_T(n, o)` for the eight integer types. CAS and
  add answer the value that was there.

Two arguments are not values. `mem_size_of(u16)` takes a *type*, and an
atomic's ordering is one of `Relaxed`, `Acquire`, `Release`, `AcqRel`,
`SeqCst` written at the call. Grasm has no type-level expressions and no
compile-time evaluation, and both of these must be known when the program is
compiled (Zig requires an ordering to be `comptime` for the same reason). The
checker reads them where they are written, so a store that `Acquire`s or a
layout asked of a `str` is a compile error rather than a run-time panic.

**What is checked where.**

| | checker | interpreter | native |
|---|---|---|---|
| operation outside `unsafe`; `mem` over the ceiling | error | -- | -- |
| layout of a non-layout type; an ordering the operation cannot have | error | -- | -- |
| negative size, bad alignment, allocation that cannot be met | -- | panic | panic |
| null, wild, freed or out-of-bounds access; misaligned access; bad `mem_free` | -- | panic | **undefined** |

The last row is what `unsafe` is for. The interpreter detects all of it
because it has no real addresses to give out: it keeps raw memory in a
sandbox of its own, one byte array per allocation in a sorted map, at
deterministic addresses starting at 64 KiB, aligned as asked, with a gap
after each allocation and **never reused**, so a stale address always finds
the tombstone (or nothing) rather than someone else's bytes. Every access is
checked for a live allocation, bounds and alignment, and the panic says
which. The sandbox is shared by every task interpreter of a run, behind the
same `Rc<RefCell<..>>` discipline as the channel table, and is never
borrowed across a switch. At run time an address is the interpreter's
ordinary integer value, so lists, fields, printing and the journal need
nothing new. A compiled build checks nothing: an access is one load or store
instruction.

**Engines.** Both engines look a raw-memory operation up by span in a table
the checker writes (`Analysis::mem_ops`), the way they find a fixed-width
operation's width, rather than by method name: natively an address is held
as an `i64`, and a user's UFCS `fn offset(n: i64, ..)` must not turn into
pointer arithmetic. The native backend lowers loads and stores to Cranelift
`load`/`store` at the width (sign- or zero-extending narrow results), atomics
to `atomic_load`, `atomic_store`, `atomic_cas` and `atomic_rmw add`, and
`mem_alloc`/`mem_free` to two runtime functions over the system allocator
(zeroed, so both engines read zeros from fresh memory). Cranelift makes
every atomic sequentially consistent, which is at least what any ordering
asks for; the ordering is kept in TIR for the S3 backend, which will choose
`ldar`/`stlr`/`ldxr` by it.

**Refcounted values.** No operation takes the address of a Grasm value, and
a load or store moves only a layout type, never a `str`, list, struct,
closure or `ref`. So raw memory and the values the runtime counts are
disjoint: no address can name a live or freed `RtVal`, and nothing read from
memory becomes one. In the interpreter this is total, since its values are
not in the sandbox at all. Natively, `mem_addr` can forge an address into
the runtime's own heap; that is undefined behaviour, in the same class as a
C function writing through a pointer it was given, and it is exactly what
`unsafe` plus `mem` in the manifest make visible.

**Atomics and threads.** No Grasm program runs two threads over shared
memory today: `par_map` and the multi-core `parallel` path take only pure
work, and concurrent tasks are fibers on one thread. So the gates check what
can be observed now (values, results, widths, wrapping, the CAS rule, the
ordering rules) and, natively, that the instructions emitted are the atomic
ones. Atomicity under contention becomes testable when threads that share
memory arrive (S5's runtime, or `export fn` callbacks).

**TIR.** `addr` is a type; `mem_size_of` and `mem_align_of` print their
argument as `(type T)` and an atomic prints its ordering as `(order o)`;
everything else is an ordinary `builtin` or `method` node.

**Left for the owner.** Two choices were made conservatively and are worth a
second look: the capability's name (`mem`), and that the interpreter
*panics* on misuse rather than treating it as undefined (so a program that
runs under `grasm run` may still crash when compiled, but never the other
way round).

**Status.** Built to this design on 2026-10-21, with `scripts/fuzz_rawmem.py`
as its gate. The rest of §5 -- thread-locals, `export fn` and function
addresses, a context switch, wider map keys -- is S1 part three.

### 5.2 Function addresses and `export fn`: the design (S1 part three)

Written before the code, 2026-10-21. The normative text will be [T-FN-ADDR],
[L-LIB-CALL-C] and [T-EXPORT], with [T-EXTERN-POINTER] widened to `addr`.

**What it is for.** The runtime written in Grasm (S5) is entered from
compiled code through the C convention at some three hundred named entry
points; it hands C functions callbacks (`qsort`, `sigaction`,
`pthread_create`, `atexit`); and it calls through code addresses it holds (a
closure's code word, a function a table names). That is three things: the
address of a named function, a call through an address, and a function C can
find by its name.

**Surface.** One declaration form, one free function, one method. Everything
else is [T-ADDR]'s `addr`.

- `export fn name(a: i32, p: addr) -> i64 { ... }`: an ordinary Grasm function
  in every way -- Grasm code calls it like any other -- that is *also*
  reachable from C under its own name by the platform's C calling convention.
  `export` is a word only before `fn`, as `extern` is, so `hpke.export(..)`
  and `fn export(..)` keep compiling.
- `mem_fn_addr(f)`: the address of the named function `f`, an `addr`. `f` is
  written as a name (`f`, `module.f`) and is not evaluated, as `mem_size_of`'s
  type is not. For an `extern fn` it is the C function's own address; for a
  Grasm function, the address of an entry to it that takes the C convention.
- `a.call_c(Sig, x, y)`: calls the code at `a` by the C convention. `Sig` names
  a type alias for a function type -- `type Cmp = fn(addr, addr) -> i32` --
  written where a value would go and read by the checker, the way
  `mem_size_of(u16)` reads its type; the arguments are checked against its
  parameters and the call's type is its result. Only the parameter and result
  types are read; the call's row is fixed (below). An alias rather than a new
  type syntax keeps the parser unchanged outside `export`.

`extern fn` parameters and results gain `addr` ([T-EXTERN-POINTER]), which
is what lets an address reach C at all: `qsort(base, n, 8, mem_fn_addr(cmp))`,
`dlsym(handle, "sqrt")`.

**C-compatible signatures.** A Grasm function that is exported or has its
address taken has `let` parameters of layout types (the eight integer
types, `f64`, `addr`) and a layout result or none, and is not generic, not
fallible, not an `impl` method; and its row holds only capabilities that need
no handler -- `io`, `net`, `proc`, `unsafe`, `mem` -- because C reaches it
outside every `with`, and outside the task scheduler `async` needs. A
signature for `call_c` may have what an `extern fn` may declare: parameters of
an integer type, `f64`, `str`, `bytes` or `addr` (a `str` crossing as a
NUL-terminated copy, `bytes` as its buffer, as for an `extern` call), and an
integer, `f64` or `addr` result or none. `bool` is out everywhere, for the
reason [T-EXTERN] gives.

**Tier 3, through what already exists.** `mem_fn_addr` and `call_c` are
raw-memory operations ([T-MEM-UNSAFE]): inside `unsafe`, performing `mem`.
Taking an address also carries the function's own row, as naming an `extern`
carries `io` ([T-EXTERN-VALUE]): whoever holds the address can call it, so
`mem_fn_addr(sqrt)` performs `io`. A call through an address performs `io`
and `unsafe` besides, exactly as an `extern` call does, because what it
reaches cannot be known. An `export fn` is Tier 3 by being declared: C may
call it with any arguments, from any thread, any number of times. Its package
performs `unsafe` for the declaration, charged at it, and the function's row
escapes the package as a `pub` function's does. So each of the three is
visible in the manifest of the package that uses it and of every consumer's
dependency graph, through the ceiling that already exists, with no new
mechanism.

**Names.** An export's symbol is its own name, so the name is unique in the
program (two exports of one name are an error here, rather than a duplicate
symbol at link time) and is not `main`, which the compiler emits as the C
entry, nor a name the toolchain gives its own symbols: one beginning with
`grasm_` (the runtime) or `g_`, `gc_`, `gp_`, `gpf_`, `gv_`, `gx_` (the
compiler). S5's runtime, which must define `grasm_` symbols, lifts that
reservation in its own design when it is linked in place of
`libgrasm_rt_abi.a`. An export named like a C library function (`write`)
replaces it for the whole program, the statically linked runtime included;
that is Tier 3 and is said so in the clause.

**Natively.** For each Grasm function that is exported or whose address is
taken, the backend emits a *C entry*: a function with the C signature that
re-extends every narrow argument from its own width (C leaves the bits above
an `i8` or an `i32` unspecified, and this backend holds every integer as a
canonical word), asks for the thread's stack floor if the function can
recurse (the hidden argument C cannot pass, as `fn_value_wrapper` does), and
calls the function. An export's entry is the global symbol `name`; any other
is local, `gx_<module>_<name>`, emitted once, by the module that defines the
function, so a function's address is one value everywhere and `==` holds
across modules under the incremental build too (the set of address-taken
functions joins the layout fingerprint that keys the object cache).
`mem_fn_addr` is Cranelift's `func_addr`; `call_c` is `call_indirect` with
the C signature, its arguments crossing exactly as an `extern` call's do, a
narrow result read at its width. The system linker dead-strips a function
nothing references, and `strip` drops exports from the export trie, so every
export is named to both: `-exported_symbol _name` at the link (which also
roots it) and `strip -s` with the export list.

**Pointer authentication.** Grasm's executables are `arm64`, not `arm64e`
(`otool -h`: cpusubtype `ALL`); on `arm64` neither the system linker nor
dyld signs code pointers, and Cranelift has no `arm64e` target. An address is
used exactly as it was given and is never stripped, so `call_c` is a plain
`blr`. A future target that signs code pointers (`arm64e`, in S3 or S7) must
sign in `mem_fn_addr` and authenticate in `call_c` (`blraaz`); it is written
here so it is not lost. Return addresses are signed in the code: Cranelift
turns on `sign_return_address` with the B key for every function on macOS
(`pacibsp`/`retab`), and a C entry is a function like any other. But the
signature is not enforced. Measured 2026-10-27 (S5 part two, `rtdialect`):
in an `arm64` process on this machine `pacibsp` leaves the return address
unchanged, because the system enables the pointer-authentication keys only
for `arm64e` processes, so a corrupted return address would not be caught
by `retab`. The instructions are kept, and they cost what they cost, so that
an `arm64e` build enforces them unchanged. Showing them enforced needs an
`arm64e` process, which nothing here builds yet.

**Interpreter.** It has no machine code for a Grasm function and cannot make
any, so:

- `mem_fn_addr` of a Grasm function gives an address from the raw-memory
  sandbox, reserved like an allocation and never reused, one per function, so
  the same function always has the same address. `call_c` through it compares
  `Sig` with the function's own signature and calls it, or panics saying both
  (natively a mismatch is undefined).
- `mem_fn_addr` of an `extern fn` is the C function's real address, found in
  the interpreter's own process as a call finds it. `call_c` through an address
  C handed over -- that, or an `addr` an `extern` call returned -- is made as
  an `extern` call is, with the shapes it supports. Through any other address
  it panics: null, wild, or an allocation's.
- An address the sandbox made, memory or function, cannot be handed to C: C
  would be reading the sandbox's made-up numbers. Passing one to an `extern`
  panics; C's own addresses pass through. Loading or storing through a
  function's address panics.

So a program that gives C a callback runs compiled and panics under
`grasm run`. That is the one place the interpreter does less than the
compiled build rather than more, and the reason is the one above; those
programs are tested natively, by CLI tests and the fuzz gate, and the corpus
uses only what both engines can run. Under `GRASM_REPLAY`, `call_c` is
refused as an `extern` call is, in both engines and whatever the address
holds, because natively nothing can tell.

**TIR.** `mem_fn_addr`'s argument prints `(fnname "module::f")` and
`call_c`'s signature `(sig (addr addr) i32)`, leaves a consumer must not
evaluate, as `(type u16)` is. An export prints `(export "module::f")` right
after its function.

**The TIR interpreter, and the question the brief asks.** Calling an `extern
fn` through an address obtained at run time -- from `dlsym`, or loaded from a
table in raw memory -- **is in scope**: it is what `call_c` is. It also closes
the gap that made `scripts/tirinterp.sh` skip the eight programs that declare
an `extern fn`: `tools/tirinterp` finds the symbol in its own process with
`dlsym` (its own `extern fn`) and calls it with `call_c`. Grasm has no
variadic call, so each shape needs a call site with a signature of its own;
the interpreter enumerates exactly the shapes `crates/grasm_interp/src/ffi.rs`
does, so a shape outside them panics in both with the same words, and the skip
rule goes. Not in scope: variadic C functions, structs by value, and a
closure as a callback -- a closure has an environment C cannot pass; a C
callback that takes a `void *` context gets an `addr`.

**Threads.** A callback C calls on a thread of its own runs Grasm code there.
That is Tier 3, and it is sound as far as the language goes: the callback
receives only layout values, safe Grasm has no shared mutable state to race
on (no mutable globals, and §5.3's thread-locals are per thread), and the
runtime keeps its own state per thread or behind locks. What it does not get
on a foreign thread is a sized stack floor: the floor assumes the main
thread's budget, so deep recursion on a small C-made stack can fault instead
of panicking.

**Gate.** `scripts/fuzz_fnaddr.py`: random C-compatible functions over every
layout type, their addresses taken and called through `call_c`, and the same
values passed through an `extern` -- in both engines, against a Python model;
natively also C calling Grasm (`qsort` with a generated comparator over raw
memory, and exports found back with `dlsym` and called), and a caller that
leaves an argument's upper bits set, which only the C entry's re-extension
gets right. Mutation controls as in §5.1, one per engine and one per checker
rule.

**Status.** Built to this design on 2026-10-21, with
`scripts/fuzz_fnaddr.py` as its gate.

### 5.3 Thread-local cells: the design (S1 part three)

Written before the code, 2026-10-21. The normative text will be [T-TLS], with
`mem_tls` a row of [L-LIB-MEM]'s table (written so: a clause of its own cost
more lines than the budget had) and `mem_free` of a cell already undefined by
[E-MEM-UNCHECKED], since a cell is no allocation.

**What it is for.** The runtime written in Grasm keeps state per thread: an
allocator's free lists and pools, a scheduler's current task, a stack floor.
The owner's rule stands -- no `static mut`, no unsound global mutable state,
and safe Grasm must not be able to observe a data race -- and the compiler
itself passes its context explicitly, so this is for the runtime alone.

**Surface.** One declaration form and one free function; everything else is
raw memory ([L-LIB-MEM], [L-LIB-ATOMIC]).

- `thread_local POOL: addr` at module level declares a *cell* of a layout type
  (the eight integer types, `f64`, `addr`). Every thread has its own, and each
  starts as zero bits: `0`, `0.0`, the null address -- as `mem_alloc`'s memory
  does. There is no initializer, because there is no moment that is "before"
  a thread C creates. `thread_local` is a word only at the start of a
  declaration followed by a name and `:`, as `linear` is before `type`.
- `mem_tls(POOL)`: the address of the calling thread's cell, an `addr`. `POOL`
  is a name, as `mem_size_of`'s type is, resolved in the module that declares
  it: a cell is private to its module, and a module that wants to share one
  writes a function that hands out the address. It is a raw-memory operation
  ([T-MEM-UNSAFE]): inside `unsafe`, performing `mem`. The cell is then read
  and written as `T` -- `load_T`, `store_T`, the atomics -- at `T`'s size and
  alignment.

A cell holds a layout type, never a counted value, and is reached only
through an address, for the reason §5.1 keeps raw memory apart from the heap:
a counted value in per-thread storage would be shared between the tasks of a
thread and visible to whatever C does with the address, and the refcounts
were never made for that. With layout types only, the runtime's counting
never meets per-thread state, and there is no second mechanism to make safe.

**Why this cannot race in safe Grasm.** Safe code cannot reach a cell, since
`mem_tls` is `unsafe`. Grasm itself starts other threads only for `par_map`
and the multi-core `parallel`, and both refuse work that performs `mem`
([T-PAR-MAP-PURE]), so a cell is touched only by the thread that runs `main`
and by threads C starts (§5.2), each its own. Concurrent tasks are fibers on
one thread and share its cell: a value one task stores is seen by the next to
run, at the points [E-SCHEDULE] fixes, the same in both engines. Two threads
see one cell only if its address is handed across through C, which is Tier 3
and spelled in the manifest.

**Natively.** The runtime keeps one zeroed block per thread, made the first
time the thread asks and released when it ends (its own `thread_local!`, the
mechanism it already uses for its per-thread state; no `static mut`). The
compiler lays every cell of the program out in that block -- ordered by
qualified name, each at a multiple of its size -- and `mem_tls(POOL)` is a
call for the block, with the program's total size, plus `POOL`'s offset. The
layout is the whole program's, so it joins the layout fingerprint that keys
the incremental build's objects. Cranelift's own Mach-O thread-local
variables are not used: nothing here exercises them, crosstest runs x86-64
under Rosetta, and a runtime call is the same on both, costing about what a
Rust `thread_local!` access costs on macOS, which is also a call. The block is
not `mem_alloc` memory: freeing a cell's address is undefined.

**Interpreter.** One thread, so one cell per declaration: a sandbox
allocation of `T`'s size and alignment, zeroed, made the first time it is
asked for and never freed. Every access through it is checked like any
other; `mem_free` of it panics ("mem_free of a thread-local cell"), where a
compiled build would free something that was never allocated.

**TIR.** `(threadlocal "module::POOL" addr)` declares a cell, in source order
with the other declarations, and `mem_tls`'s argument prints
`(tlsname "module::POOL")`, a name a consumer must not evaluate.

**The TIR interpreter.** A cell is an allocation from the raw-memory sandbox,
which another branch is adding to `tools/tirinterp`. Until the two meet, the
loader reads the declarations and the operation and the run fails as the
other raw-memory programs do -- counted in the report, not skipped.

**Gate.** `scripts/fuzz_tls.py`: random programs over several cells of every
layout type, stored, loaded and updated atomically through their addresses,
held in lists and passed to functions, against a model of one value per
cell, in both engines; natively, threads C starts (`pthread_create` with a
Grasm entry, §5.2) each find their own cells zero, write them without
disturbing the main thread's, and add to one shared counter atomically -- the
first time a Grasm atomic runs under contention; the interpreter's refusal to
free a cell; and the checker's refusals: a non-layout type, `pub`, use outside
`unsafe`, a name that is not a cell, a cell of another module.

**Status.** Built to this design on 2026-10-21, with `scripts/fuzz_tls.py` as
its gate. Up to six threads C starts each kept their own cells and added to
one counter atomically to the exact total: the contention test §5.1 had to
leave for later.

### 5.4 Wider map keys: the design (S1 part three)

Written before the code, 2026-10-21. The normative text is [T-MAP-KEY],
[E-MAP-ORDER] and [L-LIB-MAP], widened.

**What it is for.** A compiler's tables are keyed by more than one thing: a
type and a field name, a module and a function, a node and a slot. Today a
key is an integer, a `str` or a `bool`, so a program joins the parts into a
string, which is slow, allocates per lookup, and is one forgotten separator
from two keys colliding.

**Two ways it could go, and the one taken.** The brief offered tuple keys or
a documented packing idiom with a helper. A packing idiom puts the
encoding in every program that needs one, and the encoding has to be
injective and preserve order, which an `a + ":" + b` is not. Tuple keys put
it in two engines and a runtime, once, where the gates can see it. So: tuple
keys.

**Surface.** No new syntax. [T-MAP-KEY] widens: a key type is an integer
type, `str`, `bool`, or a tuple of key types -- `Map[(str, i64), V]`,
`Map[(i64, (bool, str)), V]`. Every map operation, map literals and
[T-MAP-KEY-GENERIC]'s checking at call sites apply unchanged; a type
parameter inside a tuple key is a key parameter as one outside it is.

**Order.** [E-MAP-ORDER] gains one rule: tuple keys sort component by
component, left to right, each by its own key order -- which is the order
`sort` gives tuples of the types it orders ([L-LIB-TUPLE]), so a map's `keys()` and the
sorted list of its keys are the same list. Determinism is kept: iteration
reads the keys in that order and never the hash table's.

**Hashing and flooding.** A tuple key hashes its components, in order,
through the same seeded hasher every key goes through, so the
flooding resistance is the same as for its parts.

**Engines.** The interpreter's sorted `MapKey` gains a tuple variant whose
derived order is the lexicographic one. The runtime's `MapKey` gains one
too, built from a tuple value's components on insert and on lookup -- a
lookup with a tuple key allocates the probe key, where a `str` probe does
not; that cost is measured and left for S5, which rewrites the map anyway --
and turned back into a tuple value for `keys()` and `entries()`. The native
backend already passes a key it does not specialise as a boxed value, which
is what a tuple is. TIR needs nothing: a tuple type prints as one.

**The TIR interpreter.** Grasm's maps take only scalar keys of one static
type, so the interpreter, written in Grasm, keeps tuple-keyed entries in a
fifth host map keyed by text: each component written so that the text order
is the key order and no two keys share a text -- an integer as sixteen hex
digits with its sign bit flipped, a character as six hex digits of its code
point plus one, a string ended by six zeros -- with the key and the value
kept beside it. That is the packing idiom, once, in the one program that
cannot use the feature.

**Gate.** `scripts/fuzz_mapkeys.py`: random tuple key types two and three
deep over every key type, random sets, deletes, gets, `has`, `keys`,
`values`, `entries`, printing and equality, against a Python dict whose
iteration is sorted by the same rule, in both engines, leak-checked; keys
that differ only inside a component (`("a", 1)` against `("a", 2)`, `("ab",
"c")` against `("a", "bc")`) are drawn on purpose, which is where an encoding
or a hash would collide. The checker's refusals: a component with no order
(`f64`, a list, a struct).

**Status.** Built to this design on 2026-10-21, with
`scripts/fuzz_mapkeys.py` as its gate. A compiled lookup with a tuple key
took 64 ns against 53 for the same parts joined into a string and 26 for
them packed into one integer (docs/known-issues.md).

### 5.5 A context switch: deferred to S3, and what S5 will need

Decided 2026-10-21, as S1 part three's last row. The brief allowed deferring
it with the reason written down; it is deferred, for three reasons.

**No meaning both engines can give.** Every other row of §5 has one in the
interpreter: a sandbox address, a made-up function address that checks its
signature, a cell that is a sandbox allocation. A context switch moves the
machine's registers and stack pointer from one stack to another, and the
definitional interpreter walks trees and has no machine stack to move. A
program using one could only run compiled, and it is the one primitive whose
misuse the interpreter could not even describe. What fibers *mean* is already
in the language, in both engines and agreeing -- [E-SCHEDULE], `concurrent`
and `go`, channels -- and none of that changes; what S5 needs is the
mechanism underneath, and a mechanism is the backend's business.

**The Rust backend cannot emit one.** Cranelift has no inline assembly and
no naked functions. An intrinsic here would lower to a call to a routine
written in Rust's assembly -- `grasm_fiber`'s `arch::switch` and
`arch::prepare`, which the runtime already uses -- and the point of S5 is to
remove that Rust. Naming it in Grasm today would add a clause, a capability
story and a gate for a call the S5 runtime cannot keep.

**Its contract is S3's to fix.** Which registers are preserved across a
call in the new backend's own convention, how it lays frames out for the
stack-floor check, and how it signs return addresses are decided by the
aarch64 backend (S3), and a switch is exactly as correct as its agreement
with those decisions.

**What S5's fibers will need, from S3.** Two intrinsics in the backend's
lowering rules, each `unsafe`, performing `mem`, and both taking addresses
only (§5.1):

- `ctx_switch(from: addr, to: addr)`: store the callee-saved state at
  `from` -- on arm64 x19 to x28, x29, x30, sp and d8 to d15; on x86-64 rbx,
  rbp, r12 to r15 and rsp -- load it from `to`, and return into it. A leaf
  with no frame of its own, so it signs and authenticates nothing: each
  return address it moves stays signed against the stack it belongs to, and
  that stack comes back with it. Return-address signing (`pacibsp`/`retab`,
  B key, on every Cranelift function on macOS today) must keep working on
  both sides of a switch, and the gate must show it.
- `ctx_init(ctx: addr, stack_top: addr, entry: addr, arg: addr)`: lay a
  context out so the first switch into it calls `entry(arg)`, where `entry`
  is a C-convention function address (§5.2) that never returns.

And from what exists now: a stack with a guard page, from `mmap` and
`mprotect` through `extern fn` now that `addr` crosses (§5.2); each fiber's
own stack floor, which recursive functions take as a hidden argument and
which today is per thread -- a cell (§5.3) holding the running fiber's floor,
switched with it; and the per-thread scheduler state in cells too. The gate
for it is the corpus's concurrent programs run on the Grasm runtime
(`rtswap`), agreeing with both interpreters, plus a fuzz over random switch
orders against a model of which fiber runs.

**Status.** Not built, on purpose. Rows one to three of S1 part three are
§5.2 to §5.4.

**S1 part three, measured on its final tree (2026-10-21).** difftest 648
identical; crosstest (x86-64 under Rosetta) 648 identical; leakcheck 623
leak-free; usecheck 648 clean; agreetest 29; tirtest 1,671 emitted, no holes;
reprotest 160 built twice, none differing; replaytest 44 journaled; wasmtest
508 identical; fuzz_fnaddr, fuzz_tls and fuzz_mapkeys 40, 40 and 60 programs
with none disagreeing; fuzz_rawmem, fuzz_laws (218 laws), fuzz_modules,
nativegaps, gatecheck, fmttest, gfmttest, grammar, depth, litter, clippy and
764 unit tests clean. `tirinterp` read and printed back all 1,671 programs and
agreed on 646, the eight that declare a C function among them; the five it
did not are the raw-memory programs (four from part two, and T-TLS) waiting
for the sandbox another branch is adding to it. Seventeen planted faults, one
per engine per feature at least, were each caught.

### 5.6 The runtime dialect: what S5 part two needs

Written 2026-10-26, after S5 part one (§6.7) moved the runtime's stateless
families and stopped where state begins, as the design to build to; built
on 2026-10-27 (§6.9), with the differences the status below lists.

**What is missing.** Four things the Rust runtime has and the systems subset
does not give, each found by part one:

1. *Process-wide state:* the box allocator's pool and live count, the
   interned boxes, the reference slab, channels, and the map seed -- one per
   process, shared by every thread.
2. *Fixed bytes:* panic texts and variant names, which part one could only
   store as code (`compiler/rt/gen_text.py`), because a string literal is a
   counted value and the runtime must not allocate to say it is out of
   memory.
3. *The runtime's own per-thread state:* part of the runtime is the thing
   that hands out §5.3's per-thread block, and a runtime compiled on its own
   would lay its cells out at the same offsets as the program's.
4. *The context switch* §5.5 deferred: `ctx_switch` and `ctx_init`.

**The proposal: a dialect, not a language change.** The language stays
frozen until S9, and none of the four is something a program should be
able to write. So they are given to exactly one kind of package -- one
whose manifest says `runtime = true`, which only `compiler/rt` does -- and
the checker refuses each of them anywhere else, by name ("a process cell
is the runtime's; this package is not one"). A runtime package is compiled
only by the Grasm backend (`compiler/s3 --runtime`), never run by an
interpreter and never built by Cranelift, so the two-engine rule does not
reach it; what holds it to account is §6.7's gates -- `rtswap` over the
whole corpus, the leak and use-after-free runs, and a fuzz per family
against the Rust runtime.

- `process_local POOL: i64` declares a *process cell*: one per process, of
  a layout type, zero at start, reached only through `mem_process(POOL)`,
  an `addr`, inside `unsafe`, performing `mem` -- §5.3's rules with one
  difference that matters: every thread sees the same cell. So every
  access to one that more than one thread can reach is atomic
  ([L-LIB-ATOMIC]), or under a lock built from atomics; a plain load or
  store there is a data race, and the runtime's lint (`rtlint`, new) refuses
  any non-atomic access to a process cell outside a function marked as
  holding its lock. This is Rust's `static X: AtomicU64`, not `static mut`:
  the owner's rule against unsound global mutable state stands, and safe
  Grasm still cannot reach a cell at all. Natively a zeroed symbol in
  `__DATA,__bss`, addressed by `adrp`/`add`; no runtime call.
- `mem_static("text")` takes a string *literal*, never a value, and gives
  the `addr` of its UTF-8 bytes, NUL-terminated, in `__TEXT,__cstring`;
  its length is the literal's, known when it is compiled
  (`mem_static_len("text")`). Writing through it is undefined
  ([E-MEM-UNCHECKED]); the bytes are read-only pages, so it faults.
- `thread_local` in a runtime package compiles to Mach-O thread-local
  variables (`__thread_vars`, `__thread_bss`, `TLVP_LOAD_PAGE21`/
  `PAGEOFF12`), which the S4 linker already links and `linkdiff` checks
  against ld on a fixture -- so the runtime's cells are its own and never
  share the program's block. The program's cells keep §5.3's block, which
  the runtime then hands out.
- `ctx_switch(from: addr, to: addr)` and `ctx_init(ctx, stack_top, entry,
  arg)` as §5.5 specifies them, lowered by the aarch64 rules: a leaf that
  saves x19-x28, x29, x30, sp and d8-d15 and signs nothing, so
  return-address signing keeps working on both sides of a switch.

**Rust changes.** Only the frontend's: parsing `process_local`, typing the
four operations and the manifest flag, the checker's refusals, and their
TIR forms (`(processlocal ...)`, `(mstatic "...")`, `(ctxswitch ...)`), so
`grasm tir` can hand a runtime package to `compiler/s3`. The interpreter
and the Cranelift backend refuse a runtime package by name.

**Gates.** `rtswap` with each family as it moves; a fuzz of process cells
under threads C starts (the counter `fuzz_tls` already runs, on a shared
cell); `rtlint`'s refusals as conformance cases of the dialect; the
context switch against a model of which fiber runs, with pointer
authentication on, as §5.5 asks; and the checker's refusals outside a
runtime package, one per facility.

**Status.** Built to this design on 2026-10-27 (§6.9), with four
differences. The switch and the context maker are ordinary `builtin` nodes
in TIR, as every other raw-memory operation is, not a `(ctxswitch ...)`
form; `mem_process` names its cell as `(processname "m::C")` and the static
literal is `(mstatic "...")` under a `builtin`. They are lowered to calls of
leaves the backend writes into the runtime's object rather than inline, so
the calling convention says what a switch may change, and the runtime's
own thread-local variable is reached through a getter leaf for the same
reason. rtlint's marks are doc comments (`/// rtlint: guarded by L`,
`/// rtlint: holds L`). And the five names are built in for every package,
as every built-in's is: no package declared one.

## 6. Order of work

Each stage ends with its exit criteria met and every gate green. Stages
that do not depend on each other run in parallel.

**S0. The seam.** Specify TIR (`docs/tir.md`). Make the Rust compiler
emit it (`grasm build --emit tir`) from its AST and side tables, with a
text form that round-trips. Write a TIR interpreter in Grasm.
*Exit:* TIR for the whole corpus; the TIR interpreter's output agrees with
the Rust interpreter on every corpus program. That proves TIR carries the
whole meaning of a program before anything is built on it.

**S1. The systems subset** (§5) in the Rust compiler and runtime.
*Exit:* spec clauses, both engines, conformance tests, fuzz gate.

**S2. The frontend in Grasm:** lexer, CST parser, resolver, checker, TIR.
*Exit:* token streams identical to `grasm tokens` (streams, not counts);
AST dumps identical; TIR identical for every corpus program; the same
diagnostics at the same spans for every conformance and `check_tests`
case; a fuzz gate feeding both frontends. Check speed measured against
the Rust frontend's 365-400k lines/s.

**S3. The aarch64 backend:** MIR, lowering rules, both allocators and
the checker, encoder, Mach-O object writer, DWARF lines. It links with the
system linker at this stage, consuming TIR from the Rust frontend first, so
it does not wait for S2.
*Exit:* an encoding gate against `llvm-mc` covering every form the rules
emit; `otool`, `nm`, `dwarfdump` and `ld` accept every object; every corpus
program built this way prints and exits exactly as the Cranelift build
does; the differential and operator fuzzers pointed at it; benchmarks
measured against the Cranelift build.

**S4. The linker** for arm64 Mach-O.
*Exit:* every corpus program links and runs identically; `codesign -v`
accepts every output; `dyld_info` and `nm` agree with what ld-prime
produces for the same objects; two links of the same input are
bit-identical. `cc` leaves the native build.

**S5. The runtime in Grasm**, against today's ABI.
*Exit:* the whole corpus, `leakcheck`, `usecheck`, the fuzzers and the
benchmarks pass with the Grasm runtime linked in place of
`libgrasm_rt_abi.a`.

**S6. Self-compilation.** The Grasm compiler built by the Rust one (stage
1) builds itself (stage 2), which builds stage 3.
*Exit:* stage 2 and stage 3 are bit-identical; every gate passes with
stage 2 as the compiler.

**S7. More targets:** x86-64 Mach-O (crosstest becomes self-hosted), then
Linux ELF on x86-64 and arm64 with a system-call layer and an ELF linker.
*Exit:* crosstest, and a Linux CI run, for the first time in the project.

**S8. The rest of the toolchain:** every `grasm` subcommand (today 17),
the LSP on the query engine, DAP, packages, journal and replay, `doc` and
`api`, and the wasm32 backend with the playground built by it.
The manifest stays `grasm.toml`: self-hosting changes who reads it, not its
format (decided 2026-10-26; a manifest in Grasm's own syntax, as Zig's
`.zon`, would cost every TOML-aware tool and a migration of every package,
and the language is frozen until S9). Until then the Rust toolchain is the
oracle, so `compiler/front`'s loader reads manifests with `rtoml.gr`, a port
of the Rust reader `crates/grasm_pkg/src/toml.rs`, and its diagnostics about
a manifest are the Rust ones (§6.13); `packages/toml`, a reader of TOML
itself, serves only `front toml`, which `tomldiff` holds to the Rust reader
and to Python's `tomllib` on every manifest in the repository. S8 leaves
exactly one reader: either `rtoml.gr` is promoted into `packages/toml`, or
`packages/toml` replaces it in the loader, with `tomldiff` -- every manifest
in the repository, and fuzzed ones, read by it and by `tomllib`, agreeing --
held over the one that stays, so no two TOML readers survive the cutover.

**S9. Cutover.** A release built entirely by stage 2; the DDC check
published; Rust removed from the main branch and kept on the tag
`rust-final`; the bootstrap documented; the language freeze lifted.

Dependencies: S0 → S2 and S3; S1 → S5; S3 → S4; S2, S3, S4, S5 → S6;
S6 → S7, S8 → S9.

### 6.1 S3 part two: heap values, explicit RC, fallible functions

The Grasm backend holds every value the runtime boxes -- `str`, `bytes`,
lists, tuples, structs, sum types and `Option`, maps, results -- as the
pointer to the runtime's 64-byte `RtVal` that the Cranelift build holds,
built and read through the same `extern "C"` functions of `grasm_rt` with
the same arguments, so objects from either backend link against the same
`libgrasm_rt_abi.a`. Lists of integers, `u64`s and `bool`s are the
runtime's scalar lists; a tuple's elements and a variant's fields are
boxes; a struct holds a scalar field's bits and a boxed field's pointer,
as its descriptor says. The descriptors are the runtime's `StructDesc`,
written by the backend into a `__DATA,__data` section of the object, with
each name's address as an `ARM64_RELOC_UNSIGNED` pointer relocation; the
runtime caches its type in the descriptor's first word.

**Ownership** is explicit in MIR: `inc` and `dec` are instructions, each a
call of `grasm_val_inc`/`grasm_val_dec` after selection, as they are in the
Cranelift build. The rules (`compiler/mir/src/lower.gr`, "Reference
counting"):

- every boxed value an expression computes is owned -- one reference for
  whoever receives it -- and is a temporary until it is handed on or
  released, so a `return`, `break` or `continue` part-way through an
  expression releases it;
- a variable owns a reference while its scope is open: a read retains a
  copy (`inc`), leaving the scope by any path releases it, an assignment
  releases the old value after computing the new one;
- parameters are owned by the callee, the Cranelift build's default; an
  `inout` parameter's value travels back to the caller instead;
- each runtime call borrows or consumes its arguments as its contract says,
  and what was only lent is released after it.

Mutating a place -- `xs.push(x)`, `p.items.push(x)`, `g.rows[i][j] = v`,
`s += t` -- hands the place's own reference to the runtime and stores what
comes back, taking each container on the way down out of its parent when
the parent is its only holder (`grasm_struct_take`, `grasm_list_take`), so a
sole owner is changed in place rather than copied; the arguments and the
path's indices are computed first. This matches the Cranelift build's
ownership, not its elisions: its borrowed parameters, last-use moves,
pinned buffers and value structs are not reproduced, so the Grasm build
performs more `inc`/`dec` pairs. Removing the pairs that cancel is the
Perceus-style pass of §4, to be measured against these counts.

**Fallible functions** use the Cranelift build's split-result convention:
a function `-> T ! E` that is not generic and not named `main` takes the
address of a word as a hidden last argument and answers a tag, 0 for `Ok`
and 1 for `Err`; through the address it writes the `Ok` payload's bits (0
for unit) or an owned box, or the error's owned box. `try` on such a call
reads the tag and the word directly; anywhere else the pair is boxed into a
result (`grasm_result_box`), which `match` reads with `Ok`/`Err` patterns,
`??` reads lazily, and `try` takes apart with `grasm_try_take`. A fallible
`main` answers its whole result as a box, and the C entry reports its error
(`grasm_report_main_err`, exit 1) as the Cranelift build's does.

**Calls** pass the stack floor, the arguments and a split function's
address for its word in x0..x7 and then on the stack, eight bytes each, as
AAPCS64 does: the caller stores them at the bottom of its frame and the
callee reads them above its saved frame pointer. An `inout` argument is a
frame slot whose address the callee gets; a local's value is stored in it
where the argument stands and loaded back after the call, and a place
inside a variable (`bump(s.xs)`, `fill(rows[0])`) is read out of its
container -- moved out when no later argument reads the same variable --
and written back after the call, as the Cranelift build does.

The gate that holds the counting to account is `scripts/s3rc.sh`: every
program in the subset built by the Grasm backend is run under the
leak check (`GRASM_LEAK_CHECK=1`) and against the poisoned runtime
(`--features grasm_rt/poison`), as `leakcheck` and `usecheck` run the
Cranelift build.

### 6.2 S3 part three: doubles, function values, generics

Taken in the order the blocking counts gave (part two measured f64 in 198 of
the 362 programs left out, closures and function values in 187, generics in
74).

**Doubles** are a second register class. A MIR value carries its class
(`MirFn.vfp`), every operation fixes the class of what it reads and writes,
and SSA construction refuses a lowering that mixes them. Selection is by the
rule table, as for integers: `fadd fsub fmul fdiv fneg fabs fsqrt`,
`frintm`/`frintp` for `floor`/`ceil`, `scvtf`/`ucvtf`, and `fcvtzs`/`fcvtzu`
for the conversions to integers, which round toward zero and saturate with
NaN to 0 as the Cranelift build's `fcvt_to_sint_sat` does (a narrow target
is then clamped to its range). A literal's bits, which TIR writes exactly
(`0x400921fb54442d18`), are made in a general register and moved across with
`fmov`. `%`, `^`, `round`, `min`, `max`, the math library and the formatting
are the runtime's functions, the same ones the Cranelift build calls.
Comparisons go through a floating-point condition map -- `eq ne mi ls gt ge`,
not the integer `lt le`, which an unordered comparison satisfies -- checked
by the rule table's own checks against IEEE semantics on NaN, both zeros,
the infinities, the largest finite values and subnormals, as the integer map
is checked against two's complement. Where the runtime holds a double's bits
in a word -- a scalar list's element, a struct's field, a split result's
word, an interpolated piece -- `fmov` crosses between the files.

The allocator gives each class its own registers: d16..d28 and d0..d7 to an
interval that crosses no call, d8..d15 (whose low halves AAPCS64 has the
callee preserve, and which the prologue saves in pairs when the body writes
them) to one that does, and d29..d31 for reloads. Calls follow AAPCS64:
integers in x0..x7 and doubles in d0..d7, counted apart, whatever does not
fit on the stack in argument order, a double answered in d0. The allocation
checker tracks the floating-point registers as locations of their own and
checks every operand's class twice -- what selection put in a field (a
virtual register's class) and what allocation put there (a physical
register's file) -- so a double in a general register is an error before the
encoder sees it. Every new form is in `encodetest` against `clang -c`.

**Function values** are the runtime's boxed `Fn` (`grasm_fn_new`): the
address of compiled code and an environment, a list of the captured values.
The code takes the environment first and then the arguments, and owns all
of them. A closure's body is lifted into a function of its own; it captures
by value at its creation ([E-CLOSURE-CAPTURE]), each variable its body reads
and does not bind copied into the environment -- TIR does not list captures
(docs/tir.md §8), and a closure shares its function's variable numbering, so
they are the body's reads less its bindings -- and unpacks them at entry,
bound to the variable numbers they came from. A named function used as a
value is called through a wrapper that releases the empty environment and
calls it, asking for a stack floor where the recursion is entered, the
Cranelift build's `fn_value_wrapper`; its name is in the value, so it prints
as `<fn module::name>`. A call through a value reads the code and a
reference to the environment out of it (`grasm_fn_code`, `grasm_fn_env`) and
is a `blr` on the register holding the code. `map`, `filter`, `fold`,
`any`, `all`, `find`, `sort_by`, and `map` and `filter` of a range written in
place, are loops calling the value once per element in order: `map` keeps a
scalar result inline (a scalar list), `filter` keeps its input's
representation, the searches stop at the first element that decides them,
and `sort_by` computes every key over a copy of its receiver before taking
the receiver to sort it in place. The Cranelift build inlines the closure's
body into the loop instead, and a `return` in it then leaves the calling
function (docs/known-issues.md, open S1); the lifted closure returns from
itself, as the interpreter does.

**Generics** are instantiated before anything else reads the program
(`compiler/mir/src/mono.gr`): every use of a generic function -- a `call` or
a `fnref` with type arguments -- names an instance, `name[T1,T2]`, made the
first time it is asked for as a copy of the function with each `(generic n)`
replaced by its argument. A `boundcall` -- an interface method on a receiver
of a generic type -- is concrete once its function is: the receiver's type
picks the `impl`, and the call is to the function it provides, an instance
itself when the `impl` is generic. The field types of instantiated generic
types are made the same way, so a pattern taking a `Tree[i64]` apart reads
`Tree[i64]` fields rather than `Tree[T]` ones. The subset rule runs the same
pass, in Python, before it decides anything.

### 6.3 S3 part four: the rest of the corpus

Taken in the order that admitted the most programs per unit of work, measured
before each step by the blockers each program still had (a program counts
under each of its blockers, and as "only" under one when it has no other).
At the start, of the 177 programs out: references 72 (only 31; with the
generic code at a reference type they also kept out, 49), tasks 26 (23),
effects 22 (15), ranges held as values 22 (20), the clock, entropy and
process built-ins 19 (10), raw memory and C 16 (14), the network built-ins
15 (7), `push` on a receiver that is not a place 11 (always with a
reference), the builders 1, the bit operations at a width 3, and 7 more.
In: 496 after the one-call built-ins, 516 after ranges, 580 after
references, 602 after effects, 627 after tasks, 634 after the small
families, 640 after C functions, 645 after raw memory, 647 after function
addresses, and 648 after closures that fail the function around them --
every program with a `main` but the three that link SDL, which the S3 gates
set aside, counted, as `difftest` does (TIR does not say what a program
links; the gates ask `grasm links`).

**References** are one word ([T-REF-GENERATIONAL]), a slot of the runtime's
shared heap and its generation, held in an integer register and owning
nothing. Reading through one -- `r.value`, a field of what it points at, a
`match` on it -- asks the heap for an owned copy of the box
(`grasm_ref_get`, which panics on a stale reference). A write through one
lands in the heap, below the last reference its path crosses: the containers
above it are only read, what that reference points at is made the slot's only
holder (`grasm_ref_make_unique`) and the rest of the path written in place
under it, or, when the path ends at the referent (`r.value = x`), it is
replaced whole (`grasm_ref_set`). A mutating method through a reference is
the same: `r.items.push(x)` pushes into the referent's field in place. A list
of references is a scalar list of its own kind, a struct's field of one its
own field kind, as in the Cranelift build. A closure that only writes through
a captured reference reads it untyped, so the capture analysis takes a
written place's root too, typed by its binding.

**Ranges held as values** are the runtime's boxed `Range`; their elements are
built when walked (`grasm_range_to_list`), as the Cranelift build and the
interpreter both do, and their length is the bounds' (`grasm_range_len`).

**Effects.** A handler is a struct of function values, one per operation.
`with` installs it on the runtime's handler stack (`grasm_handler_push`,
`grasm_handler_op`), each operation's function value standing as both its
code and its environment, held by the block and released after the pop; an
operation asks the stack for the innermost handler's function
(`grasm_handler_lookup`) and calls it as any function value. Every way out of
the block -- its end, `break`, `continue`, `return`, `fail`, a failed `try` --
pops what it installed, as the Cranelift build does since F59 and F61.

**Tasks.** `par_map` and `par_filter` are the runtime's parallel maps over a
worker of the C signature `fn(*const i64, i64) -> i64`, one per pair of
element and result classes, whose one-word environment is the function value
(the threads share it; the counts are atomic). `net_workers` is the same
machine, a thread a worker. Where the Cranelift build maps in order on the
calling thread instead -- a result of no value, a function that can fail --
so does this one. A `parallel` block records the `spawn` sites that fire and
then runs their bodies in that order in the block's frame ([E-SPAWN-ORDER]);
the Cranelift build runs a block of independent `v = e` tasks the checker
proved (`proved`) on worker threads, and running them in order gives the
same values, which is what this backend does. A `concurrent` block collects a
closure of no arguments per `go` and hands them to the runtime's fiber
scheduler (`grasm_conc_run`); a channel is a handle, a word.

**Raw memory and C.** An `addr` is a word, as in the Cranelift build. Loads
and stores move 1, 2, 4 or 8 bytes (`ldrb`/`ldrsb`/`ldrh`/`ldrsh`/`ldr w`/
`ldrsw`, `strb`/`strh`/`str w`, and the existing 64-bit and `d` forms); every
atomic is sequentially consistent whatever ordering is written, as there:
`ldar`, `stlr`, and the LSE `casal` and `ldaddal`, at four widths each, a
narrow signed answer sign-extended. A compare-and-swap has three operands,
the third carried as a call's arguments are (`ir.has_cargs`, the rule
selector `S_C`). Thread-local cells are the runtime's per-thread block
(`grasm_tls_block`) at the offsets the Cranelift build's `tls_layout` gives,
cells in name order. An `extern fn` is called by the C convention with a
`str` as a NUL-terminated copy and `bytes` as its buffer, a narrow answer
read at its width, `grasm_foreign_call` told first; used as a value it goes
through a wrapper of the closure convention. `mem_fn_addr` of a C function is
its own address, loaded from the global offset table (`adrp`/`ldr` with
`ARM64_RELOC_GOT_LOAD_PAGE21`/`_PAGEOFF12`); of a Grasm function, its C
entry, which re-extends each narrow argument and asks for a stack floor if
the function can recurse. An `export fn`'s C entry is the global symbol of
its own name. `call_c` is a `blr` through the address with the arguments
crossing as an `extern` call's do.

**A closure whose failure fails the function around it.** TIR marks a
closure `total` when it cannot fail itself; one that is `total` and still
has a `fail` or `try` -- `xs.par_map(|x| if x == 2 { fail(E{..}) } else { x
})` -- fails the function it is written in ([E-PAR-FAIL]). The Cranelift
build gets that by inlining the body into the loop; this backend does the
same for such a closure written as the argument of `map`, `filter`, `fold`,
`any`, `all`, `find` and the parallel maps (which then map in order), and
keeps any other out.

The remaining one-liners -- a struct literal with `..base` (the base first,
then the fields as written, set by the runtime's copy-on-write setter), `==`
on `()`, the two builders, the bit operations at a width (`clz`, `rbit`,
`rev`, `rev16`, `ror`; the population count in bit-parallel arithmetic),
`checked_*`, `saturating_*` and `try_*` by the runtime -- are each the
Cranelift build's lowering. Thirty-three new instruction forms, every one in
`encodetest` against `clang -c`.

### 6.4 S3 part five: the fuzzers, the line table, the benchmarks

**The generators pointed at it.** Twenty of the twenty-two
`scripts/fuzz_*.py` build native programs; each now hands every program the
Cranelift build accepts to a third engine, `scripts/lib/s3engine.py` --
one copy, imported by all of them. It builds the program the way the S3
gates build the corpus (`grasm tir`, `compiler/s3`, `cc` with the runtime),
runs it beside the Cranelift binary with the same arguments, environment
and input, and requires the same stdout, exit status and stderr, and
nothing live at exit when the Cranelift build returns from `main`. A program
the backend refuses by name (exit 3) is counted as declined, with its
reasons, and never as agreeing; anything else is a failure; and a generator
whose engine compared nothing fails, so an engine that silently skipped
every program cannot read as one that agreed with all of them
(`s3controls`: `engine-skip`, `engine-unused`). The other two are
`fuzz_rows`, which only asks the checker, and `fuzz_s3`, which was written
for this backend. A disagreement names the engine `grasm run` sides with.

**The line table.** Every MIR instruction and terminator records the span
start of the TIR expression it was lowered from (a pattern's, for its
tests; a block's closing brace, for what leaving the block releases; the
function body's, for the prologue), and selection, allocation and emission
carry it to each word: a reload or a spill is its instruction's. The driver
maps each offset to a file and a line by reading the source files TIR names
(a file it cannot read maps to line 0, with a warning), and the Mach-O
writer adds what clang writes under `-gdwarf-4`, less types and variables
(`compiler/macho/src/dwarf.gr`): `__debug_abbrev`; `__debug_info`, one
compile unit over the whole of `__text` (its name the program's first file,
its directory `$PWD`) with a subprogram per function; `__debug_str`; and
`__debug_line`, one sequence per function and a row wherever the line
changes. Each `__text` address in them carries an `ARM64_RELOC_UNSIGNED`
relocation against the section, as clang's do; the linker leaves the
sections in the object, for `dsymutil` and debuggers. `objtest` requires
`dwarfdump --verify` to find no error in any object and, through
`dwarfdump --debug-line`, every instruction that loads the address of a
string literal written once in the program to map to that literal's file
and line -- the sample of instructions whose right line the source alone
says -- with no row past the end of its file (`s3controls`: `dwarf-line`
shifts every line by one).

### 6.5 S3 part five: the release allocator

`compiler/aarch64/src/backtrack.gr` is the allocator §4 asks of release
builds, after regalloc2's and LLVM's greedy allocator without their
splitting. Each virtual register's live range is what it is -- a segment per
block it is live in -- rather than linear scan's hull from first to last
live point, so two values whose hulls overlap but whose lives do not can
share a register. Values are taken longest first from a heap; when no
register is free, the values holding the one it could have are evicted if
every one of them has a lower spill weight -- its uses and definitions, each
counted eight times per enclosing loop, over the length of its range -- and
requeued, and a value evicted four times keeps what it has next; only a value
that can neither find a register nor buy one is spilled. A register a move
hints at is tried first: the physical register a call's argument or result
moves through, or the one the other side of a block argument's move already
holds, so the move becomes one from a register to itself, which emission
drops. The constraints, the spill code and the checker are linear scan's.
It is the driver's default; linear scan stays for fast builds
(`--ra=linear`). `ra_check` holds both to the checker and the Cranelift
build's output: the release allocator at 0, 2 and 5 registers of each class,
linear scan with every register and with 2 (`s3controls`: `backtrack` lets a
register held by another value read as free, and the checker must refuse
it).

Over the 648 programs of the subset, the release allocator's objects hold
11,202,505 instruction words to linear scan's 18,210,217 (38% fewer) and
20,514 spill slots to 121,785 (83% fewer), with linear scan's objects
byte-identical to those before it was written; every function passes the
checker under both. On the benchmarks, timed in the same rounds, its code's
time over linear scan's has a geometric mean of 0.976 (§11; one session,
under load).

### 6.6 S4: the linker

`compiler/link` is a Mach-O linker for arm64 written in Grasm: it takes the
objects either backend writes, the runtime archive and the libraries a
program's `links` names, and writes the signed executable `cc` used to.
`grasm build --native` runs it as `grasm-link`, found where `grasm` finds the
runtime; `GRASM_LINKER=cc` links with `cc` instead, for one release.

**What goes in.** The runtime archive `libgrasm_rt_abi.a` (401 LLVM-built
members, with a `__.SYMDEF` table of contents and `#1/` long names) uses
nine relocation types: 10,553 `UNSIGNED` (in `__DATA,__const`, `__data`,
`__eh_frame`, `__compact_unwind` and the DWARF), 7,793 `BRANCH26`, 3,424
`PAGE21` and 3,439 `PAGEOFF12`, 173 each of `GOT_LOAD_PAGE21` and
`_PAGEOFF12`, 314 `ADDEND`, 166 `SUBTRACTOR` (with as many `UNSIGNED`, in
`__eh_frame`) and 17 `POINTER_TO_GOT` (personalities, in `__eh_frame`); none
of the `TLVP` pair and no scattered relocation. Its sections: `__text`,
`__const`, `__cstring`, `__literal4/8/16`, `__gcc_except_tab`,
`__eh_frame`, `__compact_unwind`, `__DATA,__const`, `__data`, `__common`,
`__bss`, DWARF, and `__LLVM,__bitcode` -- no thread-local or initializer
section. 100 of the members are not `MH_SUBSECTIONS_VIA_SYMBOLS`, so each of
their sections is one atom. The Cranelift backend's objects add
`GOT_LOAD` references to their own data; the Grasm backend's, DWARF and
`l_` string labels. What comes from neither is taken from C fixtures
(`scripts/fixtures/link/`): thread-local variables (`TLVP` relocations,
descriptors bound to `__tlv_bootstrap`, the template), an export only
`dlsym` reaches, and 4,000 functions whose unwind information needs four
second-level pages where the corpus's largest program needs one.

**How it lays out.** As ld-prime does, which is not published, so its output
is the specification: every rule is in `compiler/link/src/main.gr`'s header,
each found by comparing its map file and bytes with ld's. The ones that
decide the most: files in command-line order with an archive's members by
their index in it; atoms in input order, cold functions last; sections in
the order the files first show them, `__literal*` into `__TEXT,__const`;
`__TEXT` placed against its end with room for the load commands ld would
have written; only anonymous C strings merged; a GOT load of something
defined here made an `add`; imports numbered by their first pointer;
`__unwind_info` as ld64 builds it with four differences. The default output
is what the native build shipped -- ld's, then `strip -no_uuid` -- so the
linker writes `strip`'s symbol table, its re-encoded export trie and its
signature too.

**What it leaves out**, and why each is allowed: an LC_UUID when stripping,
as shipped (with symbols, a SHA-256 hash of the image, since ld's is an MD5
over a debug map this linker does not write); a tool entry in
LC_BUILD_VERSION, since it is not ld; the debug map, whose N_OSO entries
would name objects in a directory deleted after the build, with their
times. It refuses by name what nothing it is given needs: x86_64, arm64e,
output other than an executable, common symbols, initializers, a
thread-local variable from a dylib, frameworks.

**What checks it.** `linktest` links every corpus program twice -- the
Cranelift backend's objects as the native build captured them, and the
Grasm backend's for the programs in its subset -- and requires each to run
exactly as its `cc`-linked build does, to pass `codesign -v --strict`, and
to be the same bytes when linked again; the C fixtures likewise, against
`cc`'s link of them.
`linkdiff` holds a quarter of the corpus and every program with an `export
fn`, a thread-local cell or a `links` to ld-prime: byte for byte against the
shipped build (given ld's tool entry, `-tool_version`, the one thing the
linker does not write), and, unstripped, `dyld_info -fixups -exports`, `nm
-m`, `otool -l` and every section's bytes against `cc`'s link of the same
objects; the C fixtures, symbol tables included, and the whole runtime --
all 401 members as objects, without dead-stripping -- likewise. `linkcontrols`
plants six faults in copies of the linker and `codesig` -- a `BRANCH26` one
instruction off, a `PAGEOFF12` scaled wrongly, each GOT entry bound to the
wrong import, one page's hash wrong in the signature, an `export fn` dropped
by dead-stripping, a thread-local descriptor left unbound -- and requires
the gates to fail on each, for its reason. Two are caught by `linkdiff`
alone: the three loads a wrongly scaled `PAGEOFF12` changes in `hello` are
on paths it never runs, and a descriptor left unbound runs anyway, because
dyld writes its own thunk into every descriptor it loads -- only the image
is wrong, and `dyld_info -fixups` says so.

### 6.7 S5 part one: the runtime in Grasm, beside the Rust one

**The inventory, measured.** `libgrasm_rt_abi.a` exports 345 C entry
points, `_grasm_*`, over 17,362 lines of Rust in the five crates a compiled
program links (`grasm_rt` 12,429 with `tasks.rs`, `grasm_sys` 3,594,
`grasm_fiber` 561, `grasm_journal` 660, `grasm_rt_abi` 118).
`scripts/rtinventory.py` measures the table below and reproduces it. Two
sources, each for what it can say exactly:

- *the compiled archive*: each family's entry points are followed through
  the relocations of the archive's 401 objects to every function they can
  reach; that gives the machine code by crate, the modules of `core`,
  `alloc` and `hashbrown` reached, and the C library functions called. It
  is exact for what was compiled, inlined code counted in its caller.
- *the source*: the lines of the entry points themselves, and of the items
  (functions, types with their `impl` blocks, statics) their calls and types
  reach, split into the lines only this family reaches and the lines it
  shares with others. Naming is not calling, so the second figure is an
  estimate; about 4,400 lines -- the value representation, `display`, the
  release walk, the panic path -- are reached by every family.
- *the corpus*: which entry points the corpus's objects name in a
  relocation (`scripts/rtswap.sh`, every program with a `main` but the SDL
  ones). The Cranelift backend declares every runtime function in every
  object, used or not, so an undefined symbol alone says nothing.

| family | entry points | named by the corpus | moved to Grasm | Rust lines: entry points / reached only here / shared | machine code reached, KB: ours / `core`+`alloc` / `hashbrown` | from `core`, `alloc`, `hashbrown` beyond the common set | C library beyond the common set |
|---|---:|---:|---:|---|---|---|---|
| values and counting | 22 | 18 | 2 | 380 / 462 / 4,560 | 27.3 / 12.2 / 0.8 | `alloc::collections`, `alloc::vec`, `hashbrown::map` | `getenv` (the leak check), `memcmp`, `memmove`, `strlen` |
| number formatting and parsing | 19 | 17 | 8 | 223 / 300 / 4,494 | 22.2 / 36.6 / 0.3 | `core::num` (`flt2dec`, `dec2flt`), `core::char`, `core::unicode`, `alloc::fmt`, `alloc::string` | `bzero`, `memmove`, `memset` |
| output | 9 | 9 | 4 | 71 / 84 / 4,364 | 25.5 / 38.7 / 0.3 | `core::num`, `alloc::fmt`, `alloc::string` | `bzero`, `memcmp`, `memset` |
| panics, exit, stack floor | 13 | 12 | 4 | 151 / 142 / 4,488 | 23.9 / 40.2 / 0.8 | `core::num`, `alloc::fmt`, `alloc::string`, `hashbrown::map` | `bzero`, `memcmp`, `memset` |
| math | 38 | 33 | 35 | 202 / 314 / 4,382 | 5.5 / 6.5 / 0.3 | -- | libm: `sin` ... `tanh`, `pow`, `fmod`, `hypot`, `cbrt`, `sqrt` (19) |
| strings | 51 | 49 | 5 | 696 / 685 / 4,841 | 43.3 / 61.1 / 0.3 | `core::unicode`, `core::char`, `core::num`, `alloc::ffi`, `alloc::str`, `alloc::string` | `bzero`, `memcmp`, `memset`, `strlen` |
| bytes | 21 | 21 | 0 | 312 / 381 / 4,711 | 35.1 / 16.6 / 3.0 | `alloc::fmt`, `alloc::string` | files: `open`, `read`, `close`, `strerror`; `memchr`, `calloc` |
| lists and ranges | 52 | 46 | 0 | 1,238 / 1,383 / 4,733 | 68.3 / 78.4 / 3.5 | `core::num`, `alloc::str`, `alloc::string`, `hashbrown::map` | directories: `opendir`, `readdir`, `closedir`; files |
| maps and hashing | 15 | 14 | 0 | 334 / 402 / 4,364 | 39.2 / 47.2 / 3.6 | `hashbrown::map`, `core::num`, `alloc::slice`, `alloc::string` | `getentropy` (the hash seed) |
| structs, variants, tuples, results | 29 | 29 | 0 | 619 / 631 / 4,389 | 22.0 / 8.5 / 0.3 | -- | `memcmp` |
| function values, effect handlers | 15 | 14 | 0 | 218 / 201 / 4,592 | 29.1 / 14.4 / 3.0 | `alloc::fmt`, `alloc::string` | files (the journal) |
| generational references | 6 | 6 | 0 | 186 / 189 / 4,417 | 13.9 / 8.1 / 0.3 | -- | -- |
| args, files, clock, net, proc, C, raw memory | 42 | 42 | 0 | 539 / 2,604 / 5,034 | 94.4 / 19.1 / 3.0 | `alloc::fmt`, `alloc::string` | 55: sockets, `getaddrinfo`, `kqueue`/`kevent`, `fork`/`execvp`/`waitpid`/`pipe`/`poll`, the file system, `clock_gettime`, `nanosleep`, `pthread_create` and `rwlock`s |
| tasks, channels, fibers, parallelism | 13 | 13 | 0 | 597 / 945 / 4,535 | 42.5 / 19.5 / 0.3 | `alloc::boxed`, `alloc::collections` | `mmap`, `mprotect`, `munmap` (fiber stacks), `pthread_create`/`join`, `sysconf` |
| **all** | **345** | **323** | **58** | 17,362 in the five crates | | | |

Every family reaches the same sixteen C functions: `malloc`, `realloc`,
`free`, `posix_memalign` and `memcpy` (the global allocator), `write` and
`exit` (output and the way out), `__error`, and the `pthread` mutex and key
functions behind the output buffers' locks and the thread-local state.
Beyond them, what each reaches is the table's last column: libm for math,
`getentropy` for maps, the file system and sockets and `fork`/`execvp` for
args/files/net/proc, `mmap`/`mprotect` and `pthread_create` for the tasks.
From Rust's own libraries every family takes `alloc::alloc`,
`alloc::raw_vec` and `alloc::sync`, `core::fmt` and `core::panicking` (the
panic messages are `format!`ed), and `hashbrown::raw` (any value's release
can reach a map's); beyond those, number formatting and parsing takes
`core::num` (Grisu and Dragon4 in `flt2dec`, and `dec2flt`), strings
`core::unicode` and `core::char`, maps `hashbrown::map`, tasks
`alloc::collections` and `alloc::boxed`.

The corpus's objects -- the Cranelift backend's for the 648 programs with a
`main` that link no system library -- name 323 of the 345 entry points. The
22 they never name include four the backends do without a call (`f64_abs`,
`f64_sqrt`, `f64_floor`, `f64_ceil` are an instruction each) and two reached
only from inside the runtime (`pow_u64` and `panic_neg_pow`, through
`pow_i64`); the rest are `env_data`, `live_allocations`,
`map_get_ptr_bytes`, `range_start`, `range_end`, `range_inclusive`,
`region_push`, `region_pop`, `str_build_end_inline`, `str_from_u64`,
`str_parse_u64_base`, `u64_to_char`, `val_release`, `vals_data`, `vals_len`
and `vals_pin`. `scripts/fuzz_rtmath.sh` calls the math ones by name for
that reason, and `fuzz_rtfmt` reaches `str_from_u64`.

**How a Grasm runtime is built.** `compiler/rt/` is a Grasm package in the
systems subset (§5): `addr` and raw memory, atomics, `extern fn` to the C
library, and an `export fn` per C entry point. It is built by our own
toolchain -- `grasm tir compiler/rt/src/main.gr`, then `s3 rt.tir -o rt.o
--runtime` -- and nothing in it is Rust's or Cranelift's. `--runtime` is the
one thing the Grasm backend lacked (`compiler/mir`, `compiler/s3`): a
program has a `main` and is reached from it; the runtime has none, is
reached from its exports, and gets no C entry point of its own; and its
calls to C are its own, so they are not reported to `grasm_foreign_call`,
which under `GRASM_REPLAY` would refuse them as a program's. Objects of
programs built without the flag are unchanged: the same TIR compiled by the
backend before and after gives the same bytes (109 corpus programs
compared). The backend's own lowering still calls the Rust runtime in two
places in the runtime's code -- `/` and `%` check their divisor through
`grasm_panic_div_zero` and `grasm_panic_rem_zero` -- and `families.txt`
lists both as the backend's. The code keeps four rules, each for a reason
the language gives:

- *no Grasm heap value* -- no `str`, list, map, struct, tuple or closure,
  only layout types and raw memory -- because every heap value is made by
  the runtime being written;
- *no thread-local cell*: a program's cells and the runtime's would share
  the one block `grasm_tls_block` hands out, laid out by each compilation
  separately, so they would overlap. Per-thread runtime state waits for the
  backend to give the runtime cells of its own (Mach-O thread-local
  variables, which `compiler/link` already links);
- *no process-wide state*: Grasm has no global, by design (§5.3). The Rust
  runtime keeps a few -- the map hash seed, the interned boxes, the
  reference slab, the channel table -- and they stay there until a way to
  hold one is designed (a §5-style addition, not made here);
- *fixed text is code*: Grasm has no static bytes, so each message is a
  function storing its words (`compiler/rt/src/text.gr`, written by
  `gen_text.py`, which the gates check is current).

A Grasm `export fn` may not be named `grasm_*` ([T-EXPORT] reserves it for
the toolchain), so the runtime's are `grt_<name>`, and the linker binds
them (below). That keeps the reservation, the checker and the language as
they are.

**How it coexists with the Rust archive: interposition by our linker.**
`compiler/link -interpose rt.o` loads the Grasm runtime's object in front
of `libgrasm_rt_abi.a`, and every reference *from another object named on
the command line* -- the program's -- to `_grasm_<name>` binds to
`_grt_<name>` where `rt.o` defines one. Every other reference binds as it
would without it: the Grasm runtime's own `extern fn grasm_<name>` reaches
the archive's, and the archive's members reach each other. That is what
dyld's interposing does for a dylib, and for the same two reasons. The
replacement can call the original, which is how a family that has moved
uses what has not. And nothing the original binds internally is
redirected -- which is not a loss: almost every call inside the Rust
runtime is to an internal function (`alloc`, `dec`, `display`, `emit`),
not to its exported name, and no link-time step can redirect those. Nothing
in the archive is renamed or dropped, so a family is moved by defining its
`grt_` names and nothing else. `-interpose_log` writes which runtime names
the program's objects took from each side, which is what the gates count.

The other two designs the brief offered were weighed and not taken. *Our
definitions over the archive's for the same name* needs the Grasm runtime
to define `_grasm_*`, which [T-EXPORT] forbids, and a rule for the
archive's own references that either reaches Grasm code from inside Rust
code that cannot be redirected anyway, or does not -- in which case it is
interposition. *A link-time rename of the archive's replaced symbols* works
the same at run time but leaves the archive's own symbol table lying about
what it defines; interposition states the binding where it is made.

**State moves last.** The Rust code that is still linked keeps using its
own state directly -- `alloc` takes a box from its pool and counts it live,
`rt_panic` flushes its own stdout buffer, `display` writes through `emit`
-- and none of that can be pointed at Grasm without changing the Rust
runtime. So each piece of state stays the Rust runtime's until the last
Rust code that touches it has moved, and the Grasm runtime reaches it
through the entry point that owns it. Four are used today, and
`compiler/rt/families.txt` lists them as `state`: `grasm_box_unit` (a fresh
box from the pool, counted live, `ARENA` and registered when a region is
open), `grasm_val_dec` on a box at count one (its release: the walk over
every kind of payload, the pool, the live count), `grasm_out_text` (the
output buffer every writer shares, flushed by each newline, panic and
exit), and `grasm_panic_msg` (the way out of a panic: flush stdout, write
`panic: <message>` to stderr, exit 101). The order this forces is the
strangler's: logic family by family now; then the release walk, once every
payload's layout has moved; then the allocator, the output buffers and the
panic path together, with the last Rust that uses them. No Rust change was
needed or made.

**The layouts the two share** (`compiler/rt/src/layout.gr`), from the Rust
source and, where Rust chose them, measured by `compiler/rt/probe/layout.gr`
-- a program linked with the Rust runtime that makes values through its
entry points and reads their bytes:

| what | layout | how known |
|---|---|---|
| a box, `RtVal` | 64 bytes, `repr(C)`: the count at 0 (an `AtomicI64`), the payload's tag at 8, its fields from 9 or 16 | source, measured |
| counts | `IMMORTAL` = `i64::MAX` (interned and literal values), `ARENA` = `i64::MAX - 1` (owned by an open region), `POISON` = `i64::MIN + 0x5EAD` (freed, poisoned build); a pooled free box holds the next one's address | source, measured (`POISON` in a probe linked with the poisoned runtime) |
| payload tags | `RtPayload` is `repr(u8)`, tags in declaration order: I64 0, Unit 1, Ref 2, F64 3, Bool 4, Str 5, Env 6, Handler 7, Range 8, Bytes 9, List 10, Tuple 11, ListScalar 12, Map 13, Struct 14, Variant 15, Fn 16, ListVals 17, U64 18, StrBuf 19, ByteBuf 20 | source; 17 measured |
| scalars | I64, F64, Ref, U64 the word at 16; Bool the byte at 9 | measured |
| a string, `Str(RtStr)` | `StrData<38>` at 16 with a tag byte of its own (0 inline, 1 heap); inline: length at 17, bytes 18..56, zero past the text (two inline strings compare as whole arrays); heap: a `String` at 24; the character index (`AtomicPtr<CharIndex>`, null until built) at 56 | measured |
| the character index | a `Box<CharIndex>` of 32 bytes: a `Vec<u32>` of each character's byte offset at 0 (empty, pointing at 4, for ASCII), the `u32` count at 24; built by `len` for text of 256 bytes and more, published by compare-and-swap | measured |
| `Vec<T>`, `String` | capacity at 0, pointer at 8, length at 16; an empty `Vec<u8>` points at 1 and grows to 8 first | measured |
| an interpolation builder | the handle is a `Box<String>`: the `String` at the handle | source, measured |
| lists, tuples | `ListData`, `repr(u8)` at 16: 0 inline (length at 17, four pointers 24..56), 1 heap (a `Vec` at 24) | source (`layout` module) |
| scalar lists | kind byte at 9; `ScalarData`, `repr(u8)` at 16: 0 words, 1 bytes (bools); its `Vec` at 24 | source |
| structs | the `StructTy` pointer at 16; `StructFields`, `repr(u8)` at 24: inline (length at 25, four words 32..64) or a `Vec` at 32; the backends' `StructDesc` is `repr(C)` (§6.1) | source |
| bytes | `RtBytes`, `repr(C)` at 16: pointer, length, `Arc<Vec<u8>>` | source |
| variants, `Fn`, `Env`, `Handler`, `Range`, `StrBuf`, `ByteBuf` | each variant's fields in declaration order from 16, since the variant's struct is `repr(C)` (a variant: two `&str`, then `VFields`; `Fn`: code, environment, `Option<String>` name); inside them Rust's own order (a `&str`'s two words, `VFields`, the `Option`'s niche, a `Vec`'s three) | source; not yet measured |
| maps | a `hashbrown` table of `(MapKey, *mut RtVal)` with the per-process `MapHash`: a 64-bit seed from `getentropy`, mixed by a wyhash-style 128-bit multiply-fold (not SipHash) | not yet measured |

`scripts/rtlayout.sh` holds every constant in `layout.gr` (40) to the
probe's measurement by name, and the nine the Rust source states outright
(the `layout` module's tag and offsets, the sentinels, the inline capacity,
the box size) to the source too.

**The families moved: 58 entry points.** `compiler/rt/families.txt` lists
them; each program's references to them bind to Grasm:

- *math* (35): integer power (wrapping, a negative exponent panics),
  `%` on floats (`fmod`), `abs`/`min`/`max`, rounding, and the libm
  wrappers, calling libm as the Rust runtime does; `core`'s own arithmetic
  (`to_radians`, `to_degrees`, `log(b)`) is written as `core` writes it,
  and `min`/`max` are LLVM's `minnum`/`maxnum`: arm64's `fminnm` and
  `fmaxnm` (libm's `fmin`/`fmax`), on operands whose signalling NaNs are
  made quiet first.
  `int_checked`, `int_saturating` and `int_try` answer `Option`s and wait
  for the variant layout.
- *number formatting* (12): `print`'s integers, `u64`s, floats and booleans
  (`out_*`), `to_str` of each (`str_from_*`), their pieces of an
  interpolated string (`str_build_*`), and `str_pin_i64`. A float is the
  shortest digits that read back as itself, by Dragon4 as `core` writes it
  (`flt2dec::strategy::dragon::format_shortest`, over 32-bit limbs in raw
  memory, with `decode` and `estimate_scaling_factor` as `core` has them),
  laid out as Rust's `{}` lays it out and given `.0` when it has no point.
  Rust tries Grisu first and falls back to Dragon4; both find the shortest
  digits closest to the value, so Dragon4 alone should give Rust's bytes,
  and `fuzz_rtfmt` finds it does over four million doubles. Not
  yet: parsing (`Option`s), `to_base` and `to_fixed`, and appending to a
  string in place (`str_append_*`, which spills and grows a string's own
  storage).
- *panics* (4): division and remainder by zero, a failed assertion, a
  negative exponent -- their messages, through the Rust runtime's way out.
- *strings, read* (5): `len` (characters: short text counted, longer
  text counted once into the string's character index exactly as the Rust
  runtime builds it, so that its `substr` and `code_at` read what this
  wrote), `byte_len`, `is_empty`, `==` and `cmp` (the bytes' order, which is
  UTF-8's code-point order).
- *reference counting* (2): `grasm_val_inc` and `grasm_val_dec` -- the
  count, its sentinels, and a check for the poisoned build's `POISON` that
  panics as the Rust runtime does there; a count reaching zero is released
  by the Rust runtime from one (state, above).

**The gates** (`scripts/`, in `allgates.sh`; `gatecheck` reads them as it
reads every gate), each run on this tree on 2026-10-26:

- `rtswap`: every corpus program with a `main`, built by `grasm build
  --native` with `scripts/lib/rtswap_link` as its linker -- so the same
  objects are linked as they ship and with the Grasm runtime interposed --
  and the Grasm backend's objects for its subset, linked both ways the same;
  both binaries run with stdin closed and must print, write to stderr and
  exit identically; then both again against the poisoned runtime, which must
  agree and report no use after free. Each swapped link's log is held to
  `families.txt`: a moved name taken from Rust fails the program. "programs
  with a main: 651, external set aside: 3; Cranelift objects: identical:
  648, failed: 0, timed out: 0; runtime names the programs reference: 323
  distinct, 51 taken from Grasm and 272 from Rust; per program, 4756
  references bound to Grasm and 31587 to Rust; programs that took nothing
  from Grasm (compared nothing): 12; poisoned runtime: agreeing with no use
  after free: 648; Grasm backend objects: in the subset: 648, identical:
  648". The twelve that took nothing print only fixed text or nothing at
  all; they are named in the gate's output.
- `rtlayout`: "layout constants: 40, measured and equal: 40, also stated by
  the Rust source: 9, POISON measured in the poisoned runtime".
- `fuzz_rtfmt`: `compiler/rt/probe/fmtfuzz.gr` built both ways, a million
  draws in twenty seeded chunks, compared by hash; 20,000 of the doubles
  also against Python's `repr` laid out as Rust lays it out. "chunks
  identical: 20 of 20 (1000000 draws: 4000000 doubles and 6000000 integers
  formatted, each on both runtimes)"; "against Python: 20000 doubles, 19981
  the same, 19 exact ties Rust breaks away from zero, 0 differing
  otherwise".
- `fuzz_rtmath`: `compiler/rt/probe/mathfuzz.gr`, every math entry point
  called by its C name, answers compared as bits. "chunks identical: 10 of
  10 (500000 draws, 16500000 calls of 33 entry points on each runtime)".
- `rtcontrols`: nine planted faults, each required to fail its gate for
  its reason: `dec` taking two (`rtswap` on `examples/`: a crash), a double
  one digit short and a string one byte long (`fuzz_rtfmt`), `min` of a
  signalling NaN as libm has it (`fuzz_rtmath`), `len` one too many
  (`rtswap` on the string conformance programs), a panic exiting 1
  (`rtswap` on `E-DIV-ZERO`: "exit 101 (Rust runtime) vs 1"), the inline
  length moved a byte (`rtlayout`, and `fuzz_rtfmt`), and two fallbacks to
  Rust -- a moved function no longer exported, and one calling Rust's own
  (`rtswap`'s check of `families.txt`, before any program runs). "checks of
  planted faults: 10, caught: 10, missed: 0". That the faults planted in the
  Grasm runtime's code change what programs print is also what shows the
  programs run that code.
- `scripts/rtwith.sh <command>` runs any gate or generator with every native
  program it builds -- by `grasm build --native` and by the Grasm backend
  (`scripts/lib/s3engine.py`) -- linked with the Grasm runtime in front;
  `leakcheck` and all twenty-two generators were run through it, each
  after a canary build showed the switch in effect. `leakcheck`: "leak-free:
  623, leaking: 0, not compiled: 0, external: 3, timed out: 0, crashed: 0,
  ended before the check: 25" (the 25 it knows end early). Every generator
  passed: among them `fuzz_differential` "programs: 500, mismatched: 0",
  `fuzz_operators` "cells: 176, disagreed: 0", `fuzz_panics` "programs:
  104, disagreed: 0", `fuzz_exits` "programs: 85, disagreed: 0",
  `fuzz_intwidth` "checked lines: 19900, disagreed: 0", `fuzz_laws` "laws:
  218, broken: 0" and `fuzz_s3` "programs: 60, identical: 60". `usecheck`
  itself was not run -- it builds a poisoned runtime with cargo into the
  checkout -- but `rtswap`'s poisoned run is its method, over the same
  programs.

The backend change (`--runtime`) was followed by the S3 gates, with the
backend built from this tree: `s3subset` "identical: 648, mismatched: 0";
`objtest` "objects: 648, reproducible: 648, literal loads on their lines:
108370, with problems: 0"; `ra_check` "programs: 648, stressed runs: 3240,
identical: 3240, failed: 0", its controls 43,494 of 43,494 caught;
`fuzz_s3` "programs: 60, compared: 60, identical: 60". The linker change
(`-interpose`) leaves every other link as it was: the 648 corpus programs'
objects linked by the shipped `grasm-link` and by this tree's are the same
bytes, all 648; `linktest`, `linkdiff` and `linkcontrols` were not run
again.

**Differences found.** One between the two runtimes, and the Grasm side
was wrong: `min` and `max` with a signalling NaN. `fuzz_rtmath` found
`min(sNaN, 1.0)` answered NaN where the Rust runtime answers `1.0`: libm's
`fmin` is `fminnm`, which takes a signalling NaN as a NaN, and LLVM's
`minnum` quiets its operands before `fminnm`. The Grasm runtime now does the
same, and `rtcontrols` plants the old code back (`nan-min`) to show the
fuzz sees it. No corpus program makes a signalling NaN, so `rtswap` could
not have. Against an independent witness one more, shared by both runtimes
and by the interpreter: when a double lies exactly halfway between the two
shortest digit strings, Rust's `flt2dec` takes the one farther from zero --
`1000000000000000.25` prints as `1000000000000000.3` -- where Python's
`repr` (and JavaScript, and Ryu) take the even one, `...0.2`. [L-DISPLAY] asks for the fewest digits and
does not say which, so neither is wrong; it is recorded in
docs/known-issues.md, and `fuzz_rtfmt` checks every such case is exactly
that tie. One fault outside the runtime was found and is recorded there,
open: an incremental native build of `compiler/link` that ran out of its
stack floor at once, where a clean build of the same source is right. And
one correction to what this stage was told: map hashing is not SipHash. The
Rust runtime's comment says, and its code is, wyhash's multiply-fold keyed
by a 64-bit seed drawn once per process from `getentropy` -- resistant to
keys chosen in advance, not to an attacker who measures the live table, as
the comment says -- and part two must keep exactly that.

**Left for part two.** The rest of strings (`substr`, `code_at` and the
rest over the character index, Unicode case mapping, splitting into lists,
concatenation and appending in place), lists
(`ListData`, `ScalarData`, the total order `sort` uses), structs, variants
and `Option`s (variant names need static bytes, or text built once per
thread), number parsing, `to_base`, `to_fixed`, `str_append_*`, bytes,
references, function values and handlers -- each a family over layouts this
section writes down, and each moved the same way. Then the release walk,
once every payload's layout is in Grasm; then the allocator, its live
count, regions and the leak check, the output buffers, `out_val` and the
panic and exit path together. Maps need a process-wide seed, the interned
boxes and the reference slab process-wide state, and the runtime's
per-thread state cells of its own: the language and the backend need a
design for each before those families move. Fibers need `ctx_switch` and
`ctx_init` from the backend (§5.5). And no timing: the Grasm runtime's code
has been measured for correctness only.

### 6.8 S2 part one: the lexer and the parser

`compiler/front` is the frontend in Grasm, up to the AST: the lexer, a
parser that builds a lossless concrete syntax tree with error nodes, and
the AST built from that tree. Each is held to the Rust frontend's output,
not to a description of it, because the Rust frontend's output is what
every later stage was built against: `grasm tokens` for the lexer, and for
the rest a dump added beside it, `grasm ast <file.gr>` -- `{:?}` of
`grasm_syntax::parse`'s module on one line, every node with its span, the
documentation and the wide literals, then each diagnostic on a line of its
own. That command is the one change this part makes to the Rust compiler.

**The lexer** (`lex.gr`) produces the Rust lexer's stream token for token:
every kind, span, literal value, lexical error and message, the leading-dot
rule, newline runs merged, the unterminated comment's error placed before
the end of the file. Debug's escapes need Rust's own tables of printable
and grapheme-extending characters; `scripts/gen_rustdebug.rs` reads them
from the `rustc` that built `grasm` and writes them into `rdebug.gr`.

**The tree.** The parser (`parse.gr`) makes events, as rust-analyzer's does:
a node is started and later completed with its kind, or left a tombstone if
its construct failed, and its tokens then fall to the node around it --
the error node recovery makes. A node found to have a node before it as its
first child (`a + b`, once `+` is seen) is started after the fact. Tokens
are not events: a node owns what was consumed between its events, so a
rewind is a truncation and a newline skipped in a bracket is never lost.
`cst.gr` builds the tree, with an interpolation's tokens under the string
they came from, and prints it back: each token with the trivia between it
and the token before it in the stream, so a token lost, repeated or moved
shows in the bytes.

**Decision for decision.** The AST must be the Rust parser's, after error
recovery as well, so the parser makes the same decisions: the same newline
modes, lookaheads and rewinds (a `{` read as a map and, failing that, as a
block, with the same memo), the same recovery points, the same 2048-level
nesting budget charged at the same tokens. Its quirks are kept where they
change the output, and the ones that are Rust bugs are recorded in
docs/known-issues.md rather than copied silently. The AST builder
(`ast.gr`) visits the tree children-first in one loop and makes every
desugaring the Rust parser makes as it goes -- `impl` methods as
`method@Type`, actors, contracts, pipelines, `for (a, b)`, `ref(v)` -- and
the methods of an `impl` or actor that failed part-way, which the Rust
parser's queue hands to the next declaration.

**Depth.** The Rust parser caps the tree at 2048 levels and recurses
within that on a 256 MB stack. The Grasm one recurses too, within the same
cap, and must stay inside the 6 MB a native Grasm thread has: precedence
climbing in one loop replaces the Rust parser's function per binary level,
unary, postfix and primary share a frame, and the shapes the cap does not
bound -- `^` and `??` chains, `else if` after `else if` -- are loops. With
the cap lifted it reaches 960 to 2,900 levels depending on the shape, at
least a third past where the cap stops it. Building the tree, building the
AST and printing it are loops over explicit stacks.

**What it cost to make fast.** A write through a struct's field costs about
25 ns and 3.5 ns more for each field the struct has (measured: a push to a
list field of a 24-field struct takes 130-160 ns, to a list held in its own
variable 1-3 ns); a struct field passed `inout` is copied in and out, so a
push inside the call copies the list. The lexer's position and token
buffer, the parser's events and the tree builder's arrays are therefore
whole variables, and each struct is assembled once at the end; the first
AST builder, which passed the AST as a field, took 7.6 s on a 7,000-line
file and now takes 30 ms. The rest of the compiler will want the same.

### 6.9 S5 part two: the runtime dialect

§5.6's design, built on 2026-10-27, and steps one to three of the brief:
the dialect in both frontends, its lowering by the Grasm backend, and the
lint that guards it; and the first part of step four, the release walk for
the payloads whose layout is held. The rest of the runtime's state has not
moved: the box allocator, its pool and live count, regions, the output
buffer, the panic path and the map seed are still the Rust runtime's, and
compiler/rt uses no part of the dialect yet.

**The dialect, in both frontends.** A package whose `grasm.toml` says
`runtime = true` may write `process_local NAME: T`, `mem_process(NAME)`,
`mem_static("text")`, `mem_static_len("text")`, `ctx_switch(from, to)` and
`ctx_init(ctx, top, entry, arg)` ([T-RUNTIME]). compiler/rt says it, and
two fixtures under `scripts/fixtures` exist to test it; nothing else does.
Anywhere else the checker refuses each part by name, with one diagnostic
per use: "a process cell is the runtime's; this package is not one",
"`ctx_switch` is the runtime's; this package is not one". Inside one, each
is a raw-memory operation -- inside `unsafe`, performing `mem` -- whose
cell or literal is a name, never a value; a process cell keeps [T-TLS]'s
rules (a layout type, private to its module, never `pub`). The manifest's
key is a boolean `grasm_pkg` parses; the loader reads it from the manifest
above each file, as it already reads dependencies, and hands the checker
the runtime package's modules. `grasm run`, `test`, `bench`, a bundled
build and the Cranelift build refuse a runtime package by name before they
do anything ("package `rt` is a runtime package (`runtime = true` in its
grasm.toml), so the interpreter does not take it ..."), and the interpreter
panics if it ever reaches a dialect operation. TIR gains `(processlocal
"m::C" i64)`, `(processname "m::C")` and `(mstatic "...")`; the switch and
the context maker are ordinary `builtin` nodes over addresses rather than
the `(ctxswitch ...)` §5.6 proposed, as every other raw-memory operation
is. compiler/front lexes and parses `process_local` as the Rust parser
does, a word only before a name and a colon.

What changed for every other package is one thing: the five names are
built in, as every built-in's name is, so a function can no longer be
called `ctx_switch`. Nothing in the repository was, and
`no_builtin_takes_a_name_the_corpus_already_uses` holds it so. compiler/rt's
probes are ordinary programs the Cranelift build links with the Rust
runtime, so they became a package of their own
(`compiler/rt/probe/grasm.toml`) rather than part of a package the Cranelift
build now refuses. Saying `runtime = true` left the Grasm runtime's TIR
byte for byte what it was.

**Lowered by the Grasm backend** (`s3 --runtime`; without the flag a TIR
holding any part of the dialect is refused by name before anything else):

- a process cell is a local symbol `_grt.cell.<module>.<name>` in
  `__DATA,__bss` (S_ZEROFILL, eight-byte aligned, no room in the file),
  addressed by `adrp`/`add` -- a new MIR operation, `celladdr`, and one rule;
- `mem_static` is the literal's address in `__TEXT,__cstring`, where the
  object writer ends it with a NUL; a literal holding a NUL of its own goes
  to `__TEXT,__const` with one added. `mem_static_len` is a constant;
- the runtime's own `thread_local` cells are Mach-O thread-local variables,
  laid out as clang lays them out: a descriptor `_grt.tls.<module>.<name>`
  in `__thread_vars` whose thunk is bound to `__tlv_bootstrap` and whose
  third word is the template `..$tlv$init` in `__thread_bss`. `mem_tls` of
  one calls a getter the backend writes, `_grt.tlv.<module>.<name>`:
  `adrp`/`ldr` through the TLVP relocations, the thunk called, a frame and
  `pacibsp`/`retab` like any compiled function. A program's cells keep
  §5.3's block, which is laid out by the program and never meets these;
- `ctx_switch` and `ctx_init` are calls of leaves the backend writes into the
  object (`compiler/aarch64/src/leaves.gr`) -- `bl`, so the calling
  convention, not the allocator, decides what a switch may change, which is
  what a switch is to whoever made it. A context is 176 bytes: x19..x28,
  x29 and x30 in pairs, sp, a zero word, d8..d15. `_grt.ctx_switch` stores
  the caller's and loads the other's with ten `stp` and ten `ldp` and a
  plain `ret`; `_grt.ctx_init` writes a context whose sp is the top rounded
  down to sixteen, whose x19 and x20 are the entry and its argument, whose
  x29 is zero and whose x30 is `_grt.ctx_start`, which moves the argument
  to x0, calls the entry with `blr` and stops at a `brk` should it return.
  None of the three signs or authenticates anything: the return address
  the switch saves is the one `bl` left unsigned in x30, each function on
  either stack signed its own against that stack's sp on entry and
  authenticates it on return, and the stack comes back with the context.
  One fact about this machine bounds what that shows: in an arm64 (not
  arm64e) process the return-address keys are off -- `pacibsp` leaves an
  address as it was, which `rtdialect` reports -- so the signing on both
  sides of a switch is there and runs, but nothing enforces it. Showing it
  enforced needs an arm64e process, and none was built here.

Six instruction forms are new -- `stp`/`ldp` of general and floating-point
pairs at a signed offset without write-back, and `adrp`/`ldr` through
TLVP relocations -- and each is in `encodetest`. The object writer gains
the zero-filled sections (after the DWARF ones, so a segment's file size and
memory size now differ, as in clang's objects) and `__thread_vars` with its
relocations. An object with none of these is byte for byte what it was:
the same TIR compiled by the backend before and after gives the same bytes
for all 110 corpus programs compared (every sixth one with a `main`).

**`rtlint`** (`scripts/rtlint.py`, gate `scripts/rtlint.sh`) reads a runtime
package's TIR and refuses, by function and line, a plain `load_T` or
`store_T` through an address derived from a process cell outside a function
marked as holding the cell's lock; such an address handed to anything that
may touch it plainly (a C function, `call_c`, `ctx_switch`, `ctx_init`,
`mem_free`); such an address put where the lint cannot follow it (stored
in memory, or returned to C by an `export fn`); a call of a lock-holding
function from one that neither holds the lock nor takes it (a
compare-and-swap on the lock's own cell); and a mark that names no integer
process cell of the module. Marks are doc comments -- `/// rtlint: guarded
by POOL_LOCK` above a cell, `/// rtlint: holds POOL_LOCK` above a function
-- so they sit beside what they say. "Derived from" is followed through
every value computed from the address (locals, `offset`, `as_i64` and back
through `mem_addr`, arithmetic, branches, loop variables, pattern bindings)
and across calls both ways, to a fixpoint over the program; a value read out
of memory, and a `bool`, are not the cell's address. Atomic access is
allowed anywhere. What a mark claims -- that a function marked as holding a
lock is entered only with it held -- is checked only as far as its callers
go: each must hold the lock or take it. Its conformance cases are
`scripts/fixtures/rtlint`, one module each, saying in `//! rtlint:` lines
what they must get.

**The gates**, each run on this tree on 2026-10-27, with this worktree's
compiler:

- `rtdialect` (new): `scripts/fixtures/rtdialect`, every part of the
  dialect, compiled twice by `s3 --runtime` (the same bytes), its forms
  read back with `otool`, `nm` and `dwarfdump --verify`, then linked with a
  C driver and the Rust runtime by `compiler/link` and by `cc` and run:
  "object: process cells 6, thread-local variables 2, TLVP relocations 4,
  leaves unsigned 3, functions signed 39"; "runs: 2 of 2   checks passed in
  the compiler/link build: 5 of 5" -- the cells zero, 8-aligned and apart;
  six threads adding 100,000 each to one cell, 600,000; five literals'
  bytes, lengths and NULs, and a write to one killed by a signal in a child;
  four threads each finding the runtime's thread-local cells zero and their
  own; three fibers switched round-robin for 200 rounds, 603 switches, each
  log entry and each fiber's total as the model says. And "pointer
  authentication: off (pacibsp leaves the address as it was)".
- `rtdialectcontrols` (new): five planted faults in the backend, each
  caught for its reason: the switch not saving x19 and x20 (the driver,
  built `-O2`, dies with a signal), the switch signing its return address
  (the object check), every process cell given the first one's symbol
  ("0 of 6 cells are 8-aligned, apart from the others and fixed"),
  `mem_static_len` a byte long ("literal 0 is 14 bytes, not 13"), and the
  thread-local getter answering the descriptor instead of calling its thunk
  ("the main thread's cells did not start at zero"). "controls: 5   caught:
  5   missed: 0".
- `rtlint` (new): "packages: 2   process cells: 6   functions: 139
  refused: 0" (compiler/rt and the dialect's fixture); "cases: 19   as
  expected: 19   refusals: 18"; two controls caught of two -- a plain store
  planted in the fixture's `fx_hit`, refused as "a plain store of process
  cell `main::HITS` outside a function holding its lock", and the lint
  planted to take a plain store for an atomic one, which fails its cases.
  The lint before it followed loop variables and pattern bindings missed
  two of the cases (`bad_loop`, `bad_match`).
- `encodetest`: "forms: 106   cases: 6416   identical: 6416   different:
  0", "emittable forms: 106   with no cases: 0" -- the six new forms among
  them.
- The ones the Rust change can reach, on this tree's Rust code and corpus:
  `difftest` "identical: 648   mismatched: 0   unsupported: 0   external: 3
  timed out: 0"; `tirtest` "emitted: 1671   holes: 0   not checking: 66
  emitted twice: 335   unstable: 0" (the sixty-sixth not checking is
  T-RUNTIME, which says it should not); `wasmtest` "identical: 508
  mismatched: 0   skipped: 143"; `clippy` clean; `unittests` 772 passed, 0
  failed; `grammar`, `gfmttest` "checked: 1532   disagreeing: 0".
- The S5 gates on the final tree: `rtswap` "programs with a main: 651
  external set aside: 3"; "Cranelift objects: identical: 648   failed: 0
  timed out: 0"; "runtime names the programs reference: 323 distinct, 51
  taken from Grasm and 272 from Rust; per program, 4756 references bound to
  Grasm and 31587 to Rust"; "programs that took nothing from Grasm (compared
  nothing): 12"; "poisoned runtime: agreeing with no use after free: 648";
  "Grasm backend objects: in the subset: 648   identical: 648". `rtlayout`
  "layout constants: 58   measured and equal: 58   also stated by the Rust
  source: 14   POISON measured in the poisoned runtime". `fuzz_rtfmt`
  "chunks identical: 20 of 20 (1000000 draws: 4000000 doubles and 6000000
  integers formatted, each on both runtimes)"; `fuzz_rtmath` "chunks
  identical: 10 of 10 (500000 draws, 16500000 calls of 33 entry points on
  each runtime)"; `rtcontrols` "checks of planted faults: 13   caught: 13
  missed: 0"; `leakcheck` through `rtwith.sh` (the switch shown in effect
  first) "leak-free: 623   leaking: 0   not compiled: 0   external: 3   timed
  out: 0   crashed: 0   ended before the check: 25".
- The backend's and the linker's: `s3subset` "identical: 648   mismatched:
  0   timed out: 0   no TIR: 0"; `objtest` "objects: 648   reproducible:
  648   literal loads on their lines: 108370   with problems: 0" and
  "runtime objects: 2   reproducible: 2"; `ra_check` "programs: 648
  stressed runs: 3240   identical: 3240   failed: 0", its control 43494 of
  43494 caught; `fuzz_s3` "programs: 60   compared: 60   identical: 60";
  `inctest` 5 scenarios, 5 stale objects caught; `linktest` "Cranelift
  objects: identical: 651" and "Grasm backend objects: in the subset: 648
  identical: 648   failed: 0"; `linkdiff` "sampled: 168   byte-identical to
  the shipped build: 168   tools and sections agree: 168   C fixtures agree:
  3 of 3   whole runtime (401 members) agrees: 1"; `reprotest` "built
  twice: 160   differ: 0".
- The frontend's: `tokentest` "files: 1913   identical: 1913   mismatched:
  0", "tokens: 3809090"; `asttest` "files: 1913   AST identical: 1913",
  "CST printed back: 1913   not: 0"; `parsediag` "programs: 1489",
  "rejected by the Rust parser: 709   same first diagnostic: 709   differ:
  0"; `fuzz_front` "programs: 1500   agreed: 1500   declined: 0
  disagreed: 0"; `frontcontrols` "controls: 8   caught: 8   missed: 0".
  And `gatecheck`, `fmttest` (1907 formatted, none not), `linttest`,
  `fuzz_tls`, `fuzz_rawmem`, `fuzz_fnaddr` and `fuzz_laws` (218 laws, none
  broken) passed.
- Not run: the whole suite, `s3controls`, `s3rc`, `linkcontrols`,
  `usecheck`, and the site's regeneration.

**The release walk, part one** (step four begun). When a program's
`grasm_val_dec` takes a count to zero, the walk over what the box was the
last owner of is now Grasm's (`compiler/rt/src/release.gr`) for strings --
inline, on the heap, with a character index -- lists and tuples, inline and
on the heap, scalar lists, the two builders, variants (no field, one held in
place, or a slice of them), function values (the environment box and an
`Option<String>` name) and bytes (the window's `Arc`, its last strong count
freeing the buffer and its last weak count the `Arc`): each element
released, those that reach zero taken apart in turn from a list of pending
boxes (a loop, never recursion), the payload's buffers freed as the Rust
runtime's `drop` frees them (its global allocator is `malloc` and `free`),
and the payload left a `Unit`. The box then goes back with a count of one to
the Rust runtime's `grasm_val_dec` -- its `state` entry already -- which
finds a `Unit` and returns the box to its pool and its live count. Maps,
structs, environments, handlers and lists of value structs are still taken
apart there, children and all; so is every release inside the Rust
runtime's own code, which calls its walk directly. Eighteen layout constants
are new (`LIST_*`, `SCALAR_VEC`, `STRBUF_STRING`, `BYTEBUF_VEC`,
`VARIANT_*`, `FN_*`, `BYTES_OWNER`, `ARC_*`), each measured by the probe, and
the five the Rust `layout` module states held to it too. One was not what it
was assumed to be: an `Option<String>`'s `None` is a capacity word of -1 in
this build of the Rust runtime, not the `isize::MAX + 1` one might expect,
which only the measurement says. No Rust change.

Its gate is `fuzz_rtrelease` (new): `compiler/rt/probe/releasefuzz.gr`
builds random values of every kind the walk takes apart, nested in each
other and around the kinds it does not, some held across rounds so what is
dropped is shared, `bytes` windows sharing one buffer, named function values
and closures, and drops them; built as `rtswap` builds a program, the two
builds run every seed under `MallocScribble` and must print the same
checksum and live box count each round, and a few seeds run under `leaks
--atExit`, where a buffer the walk forgets is a leak the box counts would not
see. "seeds identical: 20 of 20 (1200 rounds, each build under
MallocScribble)   leaks runs: 6   leaking: 0". `rtcontrols` gains three
planted faults, all caught: the walk not releasing a list's elements ("seed
1: the two runtimes differ"), not freeing a heap string's buffer ("f.swap
seed 1: ... 506 leaks for 222128 total leaked bytes"), and dropping a
`bytes` value's last strong count without freeing its `Arc` ("19 leaks for
912 total leaked bytes"), which also shows the programs run this code.
`rtlayout`: "layout constants: 58   measured and equal: 58   also stated by
the Rust source: 14".

**What is left** -- the rest of step four and all of step five: the release
walk for the other payloads, whose layouts are not yet held (a map's
`hashbrown` table and its keys, `StructTy` and a struct's fields, `Env`,
`Handler`, a list of value structs); then the allocator, its pool and live count,
regions and the leak check, the output buffers with `out_val` and the panic
and exit path, maps with the Rust runtime's hashing kept bit for bit, and
the stateless families that remain; and the fibers on `ctx_switch` against
a model of which fiber runs. What this part gives them: process cells for
the pool, the live count, the interned boxes, the reference slab, the
channel table and the map seed; static bytes for the panic texts and
variant names `gen_text.py` now writes as code; the runtime's own
per-thread cells; and the switch. Nothing has been timed.

### 6.10 S2 part two: the loader, the checker and TIR

`compiler/front` now goes from the AST to TIR: `front tir P` loads the
program, checks it and prints what `grasm tir P` prints, byte for byte, or
refuses it where `grasm tir` refuses it. About 14,000 lines of Grasm were
added to part one's 8,500:

| file | what it is | the Rust it follows |
|---|---|---|
| `loader.gr` | the module graph: imports, packages, manifests through `packages/toml`, the registry and minimal version selection, source bundles, runtime packages | `grasm_sema/src/loader.rs`, `grasm_pkg`'s `Manifest`, `resolve`, `Registry` |
| `prog.gr` | the loaded program's ASTs in one set of arrays, node ids and spans made global as `parse_with_offset` makes them | |
| `tys.gr` | types interned (an id per type, so `==` is structural equality), the unifier's substitution, type definitions, the display diagnostics use | `types.rs` |
| `effrows.gr` | the effect-row fixpoint the checker needs before it types anything, and closure literals' rows | `effects.rs` `rows_and_closures` |
| `sema.gr`, `semcall.gr`, `semmeth.gr`, `sempat.gr` | the checker: declarations, statements, inference, calls and built-ins, methods and fields, `match`, patterns, closures | `check.rs`, `mem.rs`, `builtins.rs` |
| `effcheck.gr` | the effect pass that reports: declared rows, `main` and tests, constants, actors, parallel operations, rows written in types | `effects.rs` `check_effects_multi_with` |
| `ceiling.gr` | every package's capability ceiling, from its own sources and from the program | `grasm`'s `check_dependency_ceilings`, `effects_of_package`, `effects_of_program`, `effects_by_package` |
| `emit.gr` | TIR's text | `grasm_tir/src/lib.rs` |

**Decision for decision.** The checker records what the Rust one records,
keyed by the same spans, in the same order: an expression's type is written
each time it is inferred and the last write stands; a type it was checked
against fills in only where nothing was inferred; a name, a call and a method
call are recorded when resolved and overwritten when resolved again; a closure
literal's return type is filed under its own start and its body's, so a
closure that is the body of another has the outer one's -- and the TIR says
`fallible` or `total` accordingly. The Rust checker's order of work is kept
wherever it shows: closure bodies wait to the end of their block and run
last first, field reads on a type not yet known wait with them, the arms of a
`match` whose value takes its type from the others (an integer literal, a list
of them, `Ok` of one) are checked after the rest, the ones that build `Ok` or
`Err` first, a probe whose diagnostics are thrown away (an alias
resolved before its target, the prefix of an assignment's place) still leaves
the types it inferred behind. `_` is named after the offset it was written at,
and the Rust parser writes that offset in the program's numbering, so the
merged AST renames it.

**Data-oriented, as §6.8 found it has to be.** A type is an id into flat
arrays, interned by its parts; the checker's state is one flat struct of
arrays and maps, never a container inside a container that is written to;
bindings are a flat stack with each scope's start; TIR nodes are made into an
arena children first -- the order the Rust emitter numbers variables in -- and
printed from it once. Two things about the language bit and are recorded in
docs/known-issues.md: a native build passes a struct argument as a later
`inout` argument in the same call left it, where the language and `grasm
run` pass it as it was (S1); and an `if` used as a statement whose branches'
values differ in type is refused by the native backend (S3). The frontend is
written so that neither reaches it.

**The gates.**

- `loaddiff`: for every corpus program, the modules `front files` loads --
  names, aliases, paths, span bases, imports -- against the file lines of
  `grasm tir`; a program that does not load must not load for both.
- `tomldiff`: every manifest git tracks, read by `packages/toml`, by the Rust
  toolchain's reader (`grasm toml`, a dump added for this gate and the one
  change this part makes to the Rust compiler) and by Python's `tomllib`.
  No manifest writes an array of tables, so two files in
  `scripts/lib/tomlcases` do; each entry is printed with its number and in
  the file's order, so a key read into the wrong entry shows, which
  `packages/toml` could not say before: its `Doc` now carries each key's
  `entries` path (`bin[1].name`) beside the flat one.
- `tirdiff`: every corpus program and every other `.gr` file git tracks,
  `front tir` against `grasm tir`, byte for byte, or refused by both; every
  tenth emitted twice and compared.
- `tirrefuse`: the conformance negatives and the programs inline in the Rust
  test suites, refused by both or accepted by both with the same TIR.
- `fuzz_tir`: ten generators -- the differential, generics, fallible,
  integer-width, map-key, handler, backend, effect-row, package-row and
  module-graph fuzzers' -- and one edit in nearly half the programs that
  usually breaks the types and keeps the syntax; the same TIR or both
  refuse, every engine counted.
- `frontsmoke`: every fourth conformance program that says what it prints,
  its TIR made by `compiler/front`, run by `tools/tirinterp` and compiled by
  `compiler/s3`, linked by `compiler/link`; each must print and exit as
  `grasm run` does.
- `tircontrols`: a package's import of one of its dependencies resolved to
  the top-level module of that name, an integer literal checked against `u8`
  typed `i64`, a call's type arguments made apart from the
  ones it was checked with, a called function's last effect left out of a
  row, a type mismatch accepted, `index` printed with its span before its
  type, two keys' values traded between the entries of an array of tables
  by `packages/toml`, the fuzzer skipping an engine.

**What they found, 2026-10-27**, each run once after the full suite's last
run on the main checkout had finished, with that checkout's compiler and
`compiler/front` built from this one, the load average 4-6 from other work
on the machine:

| gate | result |
|---|---|
| `loaddiff` | 1,737 programs: 1,671 load the same modules (5,403 modules; 321 programs with a package's own dependency), 64 load in both and are refused later, 2 load in neither, 0 disagree |
| `tomldiff` | 1,444 files (1,442 manifests and the two cases): all 1,444 read alike by the three readers, 9,559 keys; 0 disagree |
| `tirdiff` | 1,925 files: 1,858 the same TIR, 67 refused by both, 0 disagree, 0 holes; 6,024 modules and 420 MB of TIR compared; 185 emitted twice, all stable |
| `tirrefuse` | 889 programs (66 conformance negatives, 823 from the Rust test suites): 582 refused by both, all 66 negatives among them; 307 accepted by both with the same TIR; 0 disagree; 0 Rust panics |
| `fuzz_tir` | 700 programs (the gate's size): 434 the same TIR, 266 refused by both, 0 disagree; once at 20,000 (seed 101): 12,427 and 7,573, 0 disagree, 0 Rust panics |
| `frontsmoke` | 47 programs: 47 through `tirinterp` and 47 compiled by s3 and linked by grasm-link, each printing and exiting as `grasm run` does |
| `tircontrols` | 8 planted faults, 8 caught, each by its gate and for its reason |

Part one's gates, run again over the larger frontend: `tokentest` and
`asttest` 1,925 of 1,925 files identical, `parsediag` 707 refusals with the
same first diagnostic, `frontcontrols` 8 of 8, `fuzz_front` 1,500 of 1,500.

The first full `tirdiff` disagreed on six programs, all for one reason: an
interpolation inside an interpolation in an imported module, whose spans the
Rust parser does not move by the module's base. That is the Rust side's
defect, recorded with part one's entry for it in docs/known-issues.md;
`compiler/front` now reproduces it. No other disagreement was found.

**Speed.** Each program's whole `grasm tir` against its whole `front tir`
(`scripts/tirspeed.py`): processes, start-up included, alternating which
runs first program by program, five rounds, the median; lines are those of
every module a program loads. 2026-10-27, 12:55-13:06, load average 4.0-6.2:

| programs | lines | `grasm tir` | `front tir` | front / Rust time |
|---|---:|---:|---:|---:|
| `compiler/front`, `compiler/s3`, `tools/tirinterp`, `compiler/link`, `compiler/rt` | 55,665 | 7.3k lines/s | 3.3k lines/s | 2.19x |
| every tenth corpus file, the 167 both accept | 175,341 | 17.3k lines/s | 10.2k lines/s | 1.69x |

Most of the gap on packages is the capability ceilings: like the Rust
frontend, `compiler/front` checks every source file of every package whose
manifest states capabilities as a program of its own, and it checks more
slowly. With `ceiling.check` taken out of a copy, single runs: `compiler/front`
7.69 s to 1.90 s, `compiler/s3` 2.30 to 0.53, `compiler/link` 0.81 to 0.18
(`grasm tir`, ceilings included: 2.88, 0.88, 0.40). `tools/tirinterp`,
whose manifest states no capabilities, is 5.38 s either way against
`grasm tir`'s 3.22. The Rust side cannot be timed without its ceilings
without changing it, so nothing finer is claimed.

**What is left for part three:** the diagnostics. The Grasm checker refuses
where the Rust one refuses, but what it says -- the message, the span, the
help -- is compared with nothing, and many messages are shorter than the
Rust ones. Past TIR, nothing a command does on its own is ported (`grasm
check`'s output, `deps`, the lockfile). One known way the loader could
answer differently: it compares directories by their text, collapsing `.`
and `..`, where the Rust one asks the file system, so a symbolic link, or
one directory named both relatively and absolutely in one package graph,
could be two directories to it and one to Rust. No program in the
repository does either. (Part three made it ask the file system, §6.13.)

### 6.11 S5 part three: the runtime's state, the stateless families, fibers

Written 2026-10-27. Part two (§6.9) left the release walk for half the
payloads, the allocator, the output and panic path, maps, the stateless
families and the fibers in Rust. Of the runtime's 345 entry points 285 now
come from Grasm (58 at the end of part one), by the inventory's families
(§6.7):

| family | entry points | from Grasm | what stays Rust's, and why |
|---|---:|---:|---|
| values and counting | 22 | 22 | |
| number formatting and parsing | 19 | 19 | |
| output | 9 | 6 | `out_text`, `out_nl`, `out_to_stderr`: the buffer (below) |
| panics, exit, stack floor | 13 | 10 | `panic_msg`, `exit`, `report_main_err`: the way out (below) |
| math | 38 | 38 | |
| strings | 51 | 49 | `str_c_new`, `str_c_free`: C copies, with their own count |
| bytes | 21 | 20 | `append_bytes`: a file, journaled |
| lists and ranges | 52 | 51 | `list_dir`: a directory, journaled |
| maps and hashing | 15 | 15 | |
| structs, variants, tuples, results | 29 | 29 | |
| function values, effect handlers | 15 | 14 | `env`: an environment variable, journaled (the inventory's pattern files it here) |
| generational references | 6 | 6 | |
| args, files, clock, net, proc, C, raw memory | 42 | 0 | not in this part |
| tasks, channels, fibers, parallelism | 13 | 6 | `par_*`, `net_workers*`: the worker threads, not in this part |
| **all** | **345** | **285** | |

`compiler/rt` is 13,238 lines of Grasm in 31 modules. Every family that
moved has a gate holding it to the Rust runtime -- a program built twice,
on the Rust runtime and with the Grasm one interposed, run over seeds and
required to print and exit the same -- and planted faults `rtcontrols`
catches; the runtime it builds is checked against the corpus by `rtswap`.

**The corpus**, with the Grasm runtime interposed:

- `rtswap`: "Cranelift objects: identical: 648   failed: 0   timed out:
  0", "poisoned runtime: agreeing with no use after free: 648   timed out
  (not compared): 0" and "Grasm backend objects: in the subset: 648
  identical: 648   timed out: 0";
- `leakcheck` through `rtwith.sh`: "leak-free: 623   leaking: 0   not
  compiled: 0   external: 3   timed out: 0   crashed: 0   ended before
  the check: 25";
- `usecheck` through `rtwith.sh`: "clean: 648   use-after-free: 0   out of
  memory: 0   external: 3   declined: 0   timed out: 0";
- `rtcontrols`: "checks of planted faults: 65   caught: 65   missed: 0".

**Step one: the rest of the release walk.** Maps (the `hashbrown` table,
its control bytes and entries, and each `MapKey` with its inline or heap
text and tuple keys), structs (a `StructTy` and its kinds, offsets, element
descriptors and name; four fields in the box or more in a `Vec`), `Env`,
`Handler`, and lists of value structs with inline strings, lists and
nested value lists, walked iteratively as the Rust `dec_walk` walks them.
Every offset, tag and sentinel the walk and the families read is a
constant in `layout.gr` the probe measures from the Rust runtime and
`rtlayout` holds: "layout constants: 149   measured and equal: 148   also
stated by the Rust source: 52   held to the source alone: 1". The range's
three fields, held to the source alone before, are measured now by handing
the Rust runtime a box made by hand; the one left is the state a `OnceLock`
is in while its value is made, which nothing outside can see.
`fuzz_rtrelease` builds every kind nested in every other, with handlers and
value lists and regions: "seeds identical: 20 of 20 (1200 rounds, each build under MallocScribble)   leaks runs: 8   leaking: 0 (with workers: 16 blocks against the Rust build's 16, of sizes 16 128, 8 workers)   poisoned runtime: 5 of 5 seeds agreeing with no use after free".

**Step two: the allocator.** A box is 64 bytes from either side, and either
side may free a box the other made, so the two allocators keep one ledger:
each thread's block (`alloc.gr`, a runtime thread-local cell) holds its
pool of freed boxes, its live and made counts and its open regions; a
`pthread` key's destructor folds an ending thread's counts into process
cells and frees its pool. Only the sum of the two sides' live counts means
anything, so `live_allocations` is the sum, and the leak check hands this
side's balance to the Rust ledger -- a box made there for each one live
here, a box of nothing freed there for each one this side freed beyond what
it made -- before the Rust check counts. Whether the Rust build behind is
the poisoned one (`grasm_rt/poison`), whose freed boxes count `POISON`, is
asked once by handing it a box of nothing; then this side's frees keep
their boxes out of circulation too. Regions open on both sides at once
(`region.gr`), and this side's sweep finds every child outside the region
before it frees anything -- which the Rust sweep does not
(docs/known-issues.md, latent).

**Step three: output and the end of a program -- the texts moved, the
buffer did not.** The panic texts and variant names are static bytes
(`mem_static`), and `gen_text.py` and the module it wrote are gone;
showing a value (`display.gr`: `out_val`, `str_from_val`, interpolation,
`panic_value`) is Grasm's and iterative. The output buffer and the way out
of a panic stay the Rust runtime's: the Rust code still linked -- the
families below that have not moved -- panics through its own `rt_panic`,
which flushes its own stdout buffer before it writes the message and
exits, and a buffer on this side would be written after the message, or
lost. They move with the last Rust code that can panic.

**Step four: maps, hashed bit for bit.** `maps.gr` is the Rust runtime's
`MapTable`: `hashbrown` 0.15's table (groups of eight control bytes,
triangular probing, tombstones, growth at seven eighths, rehashing in
place) over the Rust runtime's `MapHasher` -- a keyed multiply-fold of the
128-bit product's halves, fed what `#[derive(Hash)]` feeds it. Each map
keeps the seed it was made with; this side draws its own once per process
from `getentropy`, as secret as the Rust one's, and a map either side made
is hashed with its own seed whichever side touches it. Iteration is in key
order ([E-MAP-ORDER]), so no output depends on the seed. `fuzz_rtmaps`
pins `getentropy` in the program, so both runtimes draw the same seed, and
compares every table's control bytes and entries after every operation:
"pinned seeds with every table identical: 30 of 30   unpinned seeds identical: 5 of 5   (1500 operations each; map names taken from Grasm: 13)".

**Step five: the stateless families**, each with its gate:

- equality and order (`equal.gr`): "seeds identical: 20 of 20 (6000 rounds, 198000 answers of == and cmp on each runtime)";
- showing values (`display.gr`): "seeds identical: 20 of 20 (2000 rounds, 2981282 bytes of values shown on each runtime)";
- structs, variants, tuples, results and value lists (`structs.gr`):
  "seeds identical: 20 of 20 (6000 rounds, 920501 bytes of what was read on each runtime; names of the family taken from Grasm: 25)";
- generational references (`refs.gr`, the heap in process cells behind a
  lock built from a compare-and-swap): "seeds identical: 20 of 20 (16000 rounds, 20 ending in the stale-reference panic, every other run leak-free)";
- function values, environments and effect handlers (`funcs.gr`), held
  through the gates of the families that make and call them
  (`fuzz_rtrelease`, `fuzz_rteq`, `fuzz_rtshow`) and the corpus;
- `bytes` and the builders (`bytes.gr`): "seeds identical: 20 of 20 (6000 rounds)   panics identical: 6 of 6   names of the family taken from Grasm: 22";
- lists and ranges (`lists.gr`): "seeds identical: 20 of 20 (4000 rounds, each leak-free)   panics identical: 6 of 6   names of the family taken from Grasm: 41";
- strings (`strings.gr`): "seeds identical: 20 of 20 (2000 rounds, each leak-free)   panics identical: 6 of 6   names of the family taken from Grasm: 53";
- upper and lower case (`case.gr`, over tables `scripts/rtcase.py` makes
  from the Rust runtime's own answers for every code point): "code points 0 to 10ffff identical: lowercase changes 1488, uppercase 1580, case-ignorable 2794, cased 4364";
- integers and floats read from text, `to_fixed`, integers held to a width,
  and `assert_eq` (`numparse.gr`; a float is read by `core`'s own
  arbitrary-precision decimal algorithm, correctly rounded):
  "seeds identical: 20 of 20 (6000 rounds, each leak-free)   panics identical: 6 of 6   names of the family taken from Grasm: 10".

**Step six: fibers.** `fiber.gr` is `grasm_fiber`: a stack mapped with a
guard page, kept for the next fiber of its size, entered and left with
`ctx_switch` and laid out with `ctx_init`; and the stack floor, a runtime
thread-local cell that a task's fiber swaps for its own. `tasks.gr` is the
Rust runtime's scheduler and channel table, the table in process cells
behind a lock `rtlint` holds every access to. The model of which fiber runs
is the interpreter's scheduler, a third implementation of [E-SCHEDULE]:
`fuzz_rttasks` runs random tasks over random channels, each task printing
what it does, compiled on both runtimes and under `grasm run`:
"seeds identical on both runtimes: 20 of 20 (1600 rounds, 18751 lines of schedule, each leak-free)   flat seeds as the interpreter schedules them: 20   panics identical: 6 of 6   names of the family taken from Grasm: 8". It found the interpreter losing a wake meant for an outer
block's task while a nested block runs (docs/known-issues.md, S1), so
nested blocks were held to the two runtimes only until it was fixed on
2026-10-28; they are held to the interpreter too now.

Pointer authentication, measured on this machine (an Apple M1 Pro,
`hw.optional.arm.FEAT_PAuth` 1, no FPAC): the programs are arm64, not
arm64e, and in an arm64 process the return-address keys are off --
`rtdialect`: "pointer authentication: off (pacibsp leaves the address as
it was)". Of the runtime object's 1,306 functions, 1,303 begin with
`pacibsp` -- all but `ctx_init`, `ctx_start` and `ctx_switch`, the leaves --
and a switch keeps each stack's signed return addresses with that stack: the
signing runs on both sides of every switch, and nothing here shows it
enforced. Nothing more is claimed.

**Differences from the Rust runtime.** Each found, and which side is
wrong:

- The region sweep's use after free (above): the Rust side; latent.
- A value nested deeper than the Rust runtime's own walks reach in 6 MiB
  of stack is shown and compared by this side, which walks iteratively: a
  struct nested 200,000 deep shows as 4,888,910 bytes and equals itself on
  this side and under `grasm run`, where the Rust runtime ends `panic: stack
  overflow: recursion too deep`. [E-RECURSION-DEPTH] bounds a program's
  recursion, not a runtime's, so neither side breaks a clause; the
  interpreter and this side agree. No program in the corpus or the fuzzers
  builds such a value.
- `sort_by` compares a scalar key list's elements as their boxes would
  compare without making the boxes, so `GRASM_ALLOC_STATS` counts fewer
  allocations on this side for such a sort; what is sorted is the same.
  Lists built by `split`, `words`, `lines` and `chars`, and a scalar list
  after `remove`, have the same elements and may have other capacities.
- Where the Rust runtime leaks (the losing `StructTy` of a race to make
  one) this side frees; where it copies a handler's names this side
  borrows them. Neither is visible to a program.

**What is left** for S5: the output buffer and the way out of a panic,
together with the families that still reach the Rust panic -- arguments,
files, clocks, the network, processes, C strings and raw memory (42 entry
points, with the journal that records and replays them) and the parallel
workers (`par_map`, `par_filter`, `net_workers`); then S5's exit (§6), the
corpus, `leakcheck`, `usecheck`, the fuzzers and the benchmarks with the
Grasm runtime linked in place of `libgrasm_rt_abi.a`, which needs all of
them.

**Time**, measured once, not benchmarked: `rtswap`'s first run on this tree
timed out one program, `casefolddiff` built by the Grasm backend, past its
20 s. Alone, at a load average of 3.4 to 4, it took 26.9 s on the Grasm
runtime and 3.5 s on the Rust one: almost all of it pushing scalars, a list
literal of computed values being rebuilt by a push per element each time it
is read, and each push reading eight layout constants through eight calls,
since a module-level `let` compiled by the Grasm backend is a call to a
function returning it. A long copy now goes to `memcpy`, and the push's
common case reads its constants from a block made once: 7.0 s against 3.8 s
(the Cranelift build of it, timed once with `rtswap` running beside it: 1.5 s
against 0.8 s). Every other read of a layout
constant in the runtime is still a call; nothing else has been timed.

### 6.12 S5 part four: the rest of the runtime, and S5's exit

Written 2026-10-28. All 345 of the runtime's entry points come from Grasm,
and a program is linked with the Grasm runtime alone: no
`libgrasm_rt_abi.a` on the link line, and nothing of the Rust runtime in the
binary (`compiler/link -interpose rt.o` with no archive after it binds every
`_grasm_<name>` the program names to `_grt_<name>`). The runtime calls none
of the Rust runtime's entry points; `scripts/lib/rtswap.sh` fails on one.

| family | entry points | from Grasm | moved in this part |
|---|---:|---:|---|
| values and counting | 22 | 22 | |
| number formatting and parsing | 19 | 19 | |
| output | 9 | 9 | `out_text`, `out_nl`, `out_to_stderr` (`io.gr`) |
| panics, exit, stack floor | 13 | 13 | `panic_msg`, `exit`, `report_main_err` (`io.gr`, `way.gr`) |
| math | 38 | 38 | |
| strings | 51 | 51 | `str_c_new`, `str_c_free` (`cmem.gr`) |
| bytes | 21 | 21 | `append_bytes` (`files.gr`) |
| lists and ranges | 52 | 52 | `list_dir` (`files.gr`) |
| maps and hashing | 15 | 15 | |
| structs, variants, tuples, results | 29 | 29 | |
| function values, effect handlers | 15 | 15 | `env` (`envs.gr`) |
| generational references | 6 | 6 | |
| args, files, clock, net, proc, C, raw memory | 42 | 42 | all (`files.gr`, `clock.gr`, `envs.gr`, `cmem.gr`, `net.gr`, `proc.gr`, `journal.gr`) |
| tasks, channels, fibers, parallelism | 13 | 13 | `par_*`, `net_workers*` (`par.gr`) |
| **all** | **345** | **345** | 60 |

`compiler/rt` is 18,983 lines of Grasm in 44 modules (13,238 in 31 before
this part); `families.txt` lists 345 names and no `state`, `backend` or
`uses` line.

**Step one: the output buffer and the ways out** (`io.gr`, `way.gr`).
Standard output is one buffer of 8 KiB for the process, behind a lock
built from a compare-and-swap, written out at each `print`'s newline, when
a write would not fit (a write of 8 KiB or more then goes straight out), and
on every way out; standard error is the same machinery written out after
every piece -- the Rust runtime's `grasm_sys::io` streams, chunk for chunk.
Which stream `print`'s pieces go to is a runtime thread-local cell. A panic
writes standard output out, writes `panic: <message>` to standard error in
one write and exits 101; `exit(code)` writes it out and exits with the
code's low eight bits; the error `main` returns is shown after `error:
unhandled error from main: ` and exits 1; and every way out ends in
`io.leave`, which writes out each stream no other thread holds -- as the
Rust runtime's `flush_all` skips a held one (F101) -- and calls the C
library's `exit`, found by `dlsym` because its name is the built-in
`exit`'s, so `atexit` work (the `leaks` tool's among it) still runs. A
message the program made is read as `String::from_utf8_lossy` reads it.
Panic texts stay static bytes. While steps two and three were done, each
Rust entry point still linked was entered through a wrapper that wrote this
side's standard output out first, since the Rust code flushed only its own
buffer before its panics and exits; by the end of step three none was left,
and neither is the wrapper.

**Step two: the system families and the journal.** Arguments, the
environment, a line of input and entropy (`envs.gr`); files and directories
(`files.gr`, over `fsys.gr`'s system calls); the clocks and sleeping
(`clock.gr`); C strings, `mem_alloc`, `mem_free` and the program's
thread-local block (`cmem.gr`); TCP and UDP with the handle table, its
lowest-free-first handles, sockets counted strong and weak as the Rust
runtime's `Arc`s, and `net_wait` on a `kqueue` per thread or `poll`
(`net.gr`); `proc_run` and `proc_pipeline`, nothing allocated between
`fork` and `exec` and both pipes drained by one `poll` loop (`proc.gr`).
Each answers with the same values and fails with the same `IoError` text --
`strerror`'s words for an error number, `NotFound(path)` where the Rust
runtime tells a missing file apart. `journal.gr` is `grasm_journal`: the
same `GRASMJRNL2` file, per thread and chosen on the thread's first ask, a
replay checked whole when it is opened, each journaled builtin either
answering its record or writing one, the clocks and `file_exists` and
`net_close` journaled as the Rust runtime journals them (a record of another
shape taken and the world read anyway), a C call refused under a replay,
and every way a replay stops worded the same. `open`'s mode and `fcntl`'s
argument are variadic in C, and arm64 macOS passes a variadic argument on
the stack: the runtime declares each with eight register parameters and
the variadic one ninth, which the backend now passes in the first stack
slot (below). What Rust behaviour this part found wrong is in
docs/known-issues.md ("Found on 2026-10-28"): it kept four of the five
defects so the runtimes stayed comparable, and all five were fixed in both
runtimes on 2026-10-31, with a sixth the fixes uncovered.

**Step three: the parallel workers** (`par.gr`): `par_map` and
`par_filter` timed on their first element and spread over the cores when
worth it, each worker on an 8 MiB stack taking chunks from the last to the
first and the calling thread taking them too; a `parallel` block's tasks a
thread each; `net_workers` a named thread each, or one after another under
a journal. What each worker allocates is its allocator's business, handed
to the process when the thread ends (`alloc.gr`'s thread-end call, which
also frees the read buffer, the `kqueue` and the program's thread-local
block a thread kept).

**Step four: S5's exit.** The leak check is this runtime's alone
(`leak.gr`); the allocator keeps one ledger; regions sweep only this
runtime's boxes; the poisoned build is this package with `build.gr` saying
so, built beside the ordinary one by every gate that links the poisoned
Rust runtime. Three changes outside `compiler/rt`, each in `compiler/mir`
or `compiler/aarch64` and none in Rust:

- `s3 --runtime` names the runtime's own calls of an entry point it defines
  by its `grt_` name -- the checks `/` and `%` make on their divisor -- so
  the runtime links with nothing behind it;
- a runtime's C function may take more than eight integer arguments, each
  past the eighth eight bytes wide, passed on the stack; a program's still
  takes eight of each class, so no program's object changes for it;
- a module-level constant whose value is a literal -- an integer or a
  float, negated or not, or a boolean -- is that literal where it is read,
  not a call of a function answering it ([T-CONST] re-evaluates a constant
  on every read, and a literal's evaluation gives the same value and does
  nothing else). That is the cost §6.11 named: in the runtime it removed
  2,225 of 7,470 calls and 26 KB of the object. It applies to programs too.

**The gates, on the final tree** (2026-10-28 to 2026-10-31, the machine at a
load average of 8 to 20 throughout, which none of these times):

- `rtswap`, the programs linked with the Grasm runtime alone: "programs
  with a main: 651   external set aside: 3"; "Cranelift objects:
  identical: 648   failed: 0   timed out: 0"; "runtime names the programs
  reference: 323 distinct, 323 taken from Grasm and 0 from Rust; per
  program, 36345 references bound to Grasm and 0 to Rust"; "programs that
  took nothing from Grasm (compared nothing): 0"; "poisoned runtime:
  agreeing with no use after free: 648"; "Grasm backend objects: in the
  subset: 648   identical: 648   timed out: 0".
- Through `rtwith.sh`, which first shows "the Grasm runtime: 345 names
  moved (...); it calls none of the Rust runtime's" and "examples/hello.gr
  took 15 runtime names from Grasm and none from Rust, linked with no Rust
  runtime archive": `leakcheck` "leak-free: 623   leaking: 0   not
  compiled: 0   external: 3   timed out: 0   crashed: 0   ended before the
  check: 25"; `usecheck` with the poisoned build "clean: 648
  use-after-free: 0   out of memory: 0   external: 3   declined: 0   timed
  out: 0"; `replaytest` "journaled: 44   unhooked: 0   no test program: 0"
  and "a native recording replayed elsewhere: identical on every engine"
  (the browser engine not checked); and the 22 generators that build
  natively, each with nothing disagreeing, leaking or failing (`fuzz_s3`
  "programs: 60   compared: 60   identical: 60", `fuzz_differential` and
  `fuzz_generics` and `fuzz_handlers` 500 each, `fuzz_operators` 176
  cells, `fuzz_panics` 104, `fuzz_exits` 85, `fuzz_modules` 72 with no
  holes, and the rest).
- The three new family gates: `fuzz_rtout` "runs identical, apart and
  together: 180 of 180 (20 seeds of 60 rounds, each ending all 9 ways;
  8094668 bytes together)"; `fuzz_rtsys` "files, input, environment,
  programs: seeds identical: 12 of 12 (720 rounds)", "journals recorded
  byte-identical: 12   replayed on the other runtime as recorded: 24",
  "network: seeds identical: 6 of 6 (360 rounds)", "hand-written journals
  identical: 200 of 200 (81 replayed whole, 119 refused)   names of the
  family taken from Grasm: 44"; `fuzz_rtpar` "seeds identical: 12 of 12
  (480 rounds, each leak-free; in the last seed 4 long enough to spread)
  panics identical: 4 of 4". `fuzz_rtrelease` with workers: "leaks runs: 8
  leaking: 0 (with workers: 0 blocks against the Rust build's 16, of sizes
  16 128, 8 workers)". Part three's eleven family gates, `rtlayout` (149
  constants measured and equal, 52 also stated by the Rust source),
  `rtlint` and `rtdialect` pass as before.
- `rtcontrols` "checks of planted faults: 78   caught: 78   missed:
  0"; `rtdialectcontrols` "controls: 5   caught: 5
  missed: 0"; `s3controls` "controls: 29   caught: 29   missed: 0".
- The backend's, for its three changes: `s3subset` "identical: 648
  mismatched: 0   timed out: 0   no TIR: 0"; `difftest` "identical: 648
  mismatched: 0   unsupported: 0   external: 3   timed out: 0"; `objtest`
  "objects: 648   reproducible: 648   literal loads on their lines: 108370
  with problems: 0" and "runtime objects: 2   reproducible: 2"; `ra_check`
  "programs: 648   stressed runs: 3240   identical: 3240   failed: 0", its
  control 43489 of 43489; `s3rc` 623 leak-checked, none leaking, 648 clean
  under poison; `inctest` 5 scenarios; `reprotest` "built twice: 160
  differ: 0"; `encodetest` 6416 of 6416; `tirtest`, `tirdiff`, `loaddiff`,
  `tokentest`, `asttest`, `fmttest`, `linttest` and `gatecheck` as before.

**The benchmarks.** `scripts/rtbench.py` runs `bench.sh`'s eight programs,
each linked twice from the same objects -- with the Rust runtime archive and
with the Grasm runtime alone -- checks each prints what `grasm run` prints,
and times them in alternation, refusing, as `bench.sh` does, above a load
average of 0.35 of the cores. Measured once, 2026-10-31, 21 rounds, the
median of each, on this 8-core machine (6 performance cores) at the load
shown, the serial rows at 2.4 to 2.6 and the parallel rows above the
threshold, so those two are not to be quoted:

| program | Rust runtime | Grasm runtime | ratio | load |
|---|---:|---:|---:|---|
| fib | 5.21 ms | 5.22 ms | 1.003 | 2.59 |
| sieve | 6.75 ms | 7.32 ms | 1.084 | 2.62 |
| matmul | 5.34 ms | 5.39 ms | 1.008 | 2.57 |
| wordfreq | 16.18 ms | 54.73 ms | 3.383 | 2.52 |
| tree | 19.78 ms | 36.88 ms | 1.864 | 2.52 to 2.40 |
| sort | 10.96 ms | 42.98 ms | 3.922 | 2.40 |
| parmap | 64.30 ms | 67.13 ms | 1.044 | 4.03 to 5.79 |
| parwords | 11.36 ms | 13.35 ms | 1.176 | 5.32 |

The geometric mean is 1.550: every program passes on the Grasm runtime, and
three are much slower on it. Programs that spend their time in compiled
code (`fib`, `matmul`) do not move. A profile under load put `sort`'s and
`tree`'s extra time in the runtime's list sort and struct type lookup,
which this part gave fast paths (`lists.gr`'s `msort_words`, `structs.gr`'s
`type_of`); at a load of 8 to 10 (numbers not to quote) they took `sort`
from about 5.9x to 4.0x and `tree` from 2.2x to 1.9x. `wordfreq`, which
leans on maps and strings, is not profiled here.

**Since, in the runtime's source** (2026-11-01; the backend unchanged).
Profiled with `sample` on each program scaled up and priced by A/B runs of
the two builds, a single machine at a load average of 4 to 15, so the times
are not to be quoted: `rtbench.py` on a quiet machine is what to quote.

- An integer interpolated into a string (`str_build_i64`) took a scratch
  buffer from `malloc` and gave it back every time; its digits are now
  written into the builder's room when there is enough for them, which is
  the Rust runtime's case too (its digits on its stack, `push_str` growing
  nothing), and `str_from_i64` writes them into the string's box.
- A map's hash called a function for each of its three multipliers, now
  literal constants; a probe compared the eight control bytes of a group
  one call each, and now matches all eight in one word, exactly, as
  `hashbrown`'s generic group does (`matching`, `empties`); a map's
  copy-on-write, the uncommon case, is a function apart, so the common case
  saves no registers.
- `sys.copy` under 64 bytes copied word by word and then byte by byte; it
  now copies two words that overlap, as the C library does.
- A list of integers, `u64`s or floats sorted itself by a merge sort
  (`msort_words`); words equal after `ordered_keys` are the same bits, so
  any sort that orders them is the same sort, and it is now a radix sort
  by bytes from the lowest (`rsort_words`), a pass skipped where every
  word has the same byte, by insertion at 32 or fewer.

`wordfreq` scaled to 40 million words went from 4.95 s to 3.33 s of user
time (the Rust runtime's build 1.32 s); `sort` of 20 million integers
takes 0.41 s against the Rust runtime's 0.42 s; `tree` did not move. What
is left of the gap on `wordfreq` and `tree` is mostly this backend's: every
small function a call, every register saved on entry whichever way a
function goes, constants and offsets not folded into instructions. The
list fuzz (`compiler/rt/probe/listfuzz.gr`) sorted lists of at most 40
integers, 12 floats and 8 `u64`s, which the radix sort would not reach, so
one round in eight now also sorts 33 to 232 of each; `rtcontrols` adds
`list-radix` (the sort stopped after its first byte) and `map-group-empty`
(a tombstone read as empty), both caught by their fuzz.

### 6.13 S2 part three: the diagnostics, and S2's exit

`compiler/front` now says what the Rust frontend says: `front check P` prints
what `grasm check P` prints -- stdout, stderr and the exit status, byte for
byte. That is every diagnostic of the loader and the parser, or else of the
checker and the effect pass, in Rust's order, each as `file:line:column:
error: message` with its source line quoted, the carets under it and its
help; the count line; then the capability ceilings' breaches with their
help; or ``ok: `P` has no errors``. Before this the Grasm checker refused where
the Rust one refused, in shorter words and with no help, as `error: <msg>
@<start>:<end>`. `compiler/front` grew from about 22,950 lines of Grasm to
24,500 (1,960 added, 410 removed):

| file | what changed |
|---|---|
| `main.gr` | `front check`, with the argument checks `grasm` makes; `render` (`Diagnostic::render`, its window on a long line included) and `render_all` (`render_diags`: each diagnostic in the file that owns its span) |
| `sema.gr`, `semcall.gr`, `semmeth.gr`, `sempat.gr`, `effcheck.gr` | a help beside every diagnostic that has one in Rust (about 170 sites), the messages that were shorter worded as Rust words them, the checker's and the effect pass's diagnostics sorted as `check_modules_in` sorts them |
| `loader.gr` | the load diagnostics with their spans and help; "cannot load module" with every place it looked (`tried`, `candidates_in_package`); manifests read and checked as `Manifest::parse` checks them and the graph resolved as `grasm_pkg::resolve` resolves it, in their words; directories compared by asking the file system |
| `rtoml.gr` | new: the Rust toolchain's own TOML reader, `grasm_pkg/src/toml.rs`, line for line |
| `ceiling.gr` | a breach worded as `CapabilityViolation` displays it, with `ceiling_help`'s help and where each effect is performed (`effect_sites_of_package`) |
| `parse.gr`, `tys.gr` | which parse diagnostics the Rust parser leaves unmoved by a module's base; a type displayed resolved only at its top |

**Decision for decision.** The order is Rust's: the loader's and the parser's
diagnostics in load order, unsorted, or else the checker's and the effect
pass's together, sorted by start and stably, so two at one start keep the
order they were made in. A diagnostic is made once per span and message, as
`push_diag` makes it, except where the Rust checker pushes past that (a move
in a loop, `own` and `inout` of one value in one call, the generic
constructors and empty literals settled at the end), where the Grasm checker
pushes too and prints the duplicate Rust prints. The rendering keeps Rust's
mix of units: the column counts bytes, the caret pad is that many spaces, the
carets are capped by the characters left on the quoted line, and a line past
120 characters is cut by characters around that byte column. Where Rust shows
a type resolved only at its top -- "`[_]` is not callable" -- so does the Grasm
checker. An I/O error is worded as Rust's `io::Error` displays it, `No such
file or directory (os error 2)`: the runtime's `IoError` carries the
system's message, and its number is looked up in this system's table.

**Manifests are read by the Rust toolchain's reader.** What `grasm check`
says about a manifest is what `grasm_pkg/src/toml.rs` says: it reads line by
line, takes a string, an integer, a boolean or a one-line array, refuses a
float, an inline table or an array across lines, and words a refusal `line
N: ...`. `packages/toml` reads TOML itself and answers otherwise on every one
of those, so the loader now reads manifests with `rtoml.gr`, a port of the
Rust reader, and checks them in `Manifest::parse`'s order and words: the
`[package]` table, its name and version, the capabilities and links, the
entry, `runtime`, each dependency's version and path, then the unknown keys
of `[package]`, keys above the first header, unknown tables, a dependency's
unknown keys, and last the name as an identifier. `packages/toml` stays the
reader stage S8 moves the toolchain to, held to the Rust reader and to
`tomllib` by `tomldiff`, and `front toml` still reads with it.

**A directory is compared by asking the file system.** Part two's loader
compared directories by their text, collapsing `.` and `..` (§6.10). It now
calls `realpath(3)` where the Rust toolchain calls `std::fs::canonicalize`:
the package graph's directories, `same_dir`, the registry's discovery and the
"resolves to two different directories" message, which prints the canonical
paths. That is an `extern fn`, so `compiler/front` declares `unsafe` in its
manifest; it passes a `bytes` buffer for the answer, as `packages/termios`
does, and needs no `mem`. A dependency reached by its real path and through a
symbolic link was two packages to part two's loader and one to Rust; it is
one to both now.

**The gates.**

- `diagdiff`: `front check` against `grasm check`, stdout, stderr and exit
  status byte for byte, over the conformance negatives, the programs inline
  in the Rust test suites, 67 package graphs `scripts/lib/diagcases.py`
  writes -- a broken manifest, dependency, import or ceiling each, nothing in
  the repository having one, and three that are right in an unusual way: a
  dependency through a symbolic link and by its real path, one named
  relatively and absolutely, an entry package through a link -- a `^` chain
  and an `else if` chain (as a value and as statements) of 6,000, 20,000 and
  50,000 terms each (`scripts/lib/chains.py`), with `front tir` against
  `grasm tir` by digest for the 6,000-term ones, 700 programs `fuzz_tir.py
  --emit` makes, and every corpus file and `.gr` file git tracks. Every negative and every case must give the same output, and
  floors on refusals, rendered diagnostics, help lines, unrendered errors,
  ceiling breaches and accepted programs keep a run that compared too little
  from passing.
- `fuzz_diag`: `fuzz_tir`'s ten generators, with one of its edits in more
  than half the programs (half of the package graphs), and `fuzz_front`'s
  generator for the parser, each program through both `check`s; the same
  bytes, or the run fails. Every engine must be compared, enough refusals,
  diagnostics, help lines and accepted programs seen, and the Grasm side's
  output read for every program counted as agreeing.
- `diagcontrols`: nine planted faults -- "unknown name" reworded, its span
  one byte late, its help dropped, the checker's and the effect pass's
  diagnostics left unsorted, a diagnostic added beside one, a ceiling
  breach's `performed: ... at ...` dropped, the manifest reader's words
  changed, the fuzzer taking every program as agreeing without running the
  Grasm side, the fuzzer skipping an engine.
- `tirspeed.py` times `check` as well as `tir` (`--command check`), and the
  generated programs `docs/compile-speed.md` measured (`--generated`).

**What they found, 2026-11-01**, 04:59 to 06:10, on this branch rebased on
`07c50893` with the main checkout's compiler as rebuilt at 04:56, a targeted
suite running beside them, the one-minute load average between 2.4 and 5.1:

| gate | result |
|---|---|
| `diagdiff` | 4,244 programs (66 conformance negatives, 829 from the Rust test suites, 67 package cases, 674 long chains and deep shapes, 700 from `fuzz_tir`, 1,908 from the corpus): 1,437 refused with the same output, all 66 negatives, all 67 cases and all 674 chains among the programs that agree; 2,807 accepted by both; 0 differ; 0 Rust panics; 114,791 rendered diagnostics, 36,163 help lines, 81 unrendered errors and 30 ceiling breaches compared; the two 6,000-term chains' TIR the same |
| `fuzz_diag` | 2,200 programs (the gate's size, seed 3): 2,200 the same output, 1,142 refused and 1,058 accepted, 7,565 diagnostics and 2,476 help lines compared; once at 22,000 (seed 101): 22,000 the same, 11,510 refused and 10,490 accepted, 68,533 diagnostics and 34,991 help lines -- where, before the checker walked its chains on a stack (below), 2 had differed |
| `diagcontrols` | 9 planted faults, 9 caught, each by its gate and for its reason |

Part one's and part two's gates, run again over the larger frontend:
`tokentest` and `asttest` 1,974 of 1,974 files identical (4,111,888 tokens;
every CST printed back), `parsediag` 701 refusals with the same first
diagnostic and 794 accepted by both, `fuzz_front` 1,500 of 1,500,
`frontcontrols` 8 of 8; `loaddiff` 1,671 of 1,737 programs loading the same
modules, the rest refused by both; `tomldiff` 1,446 files read alike by the
three readers; `tirdiff` 1,974 files, 1,907 the same TIR and 67 refused by
both, none disagreeing, 190 emitted twice and stable; `tirrefuse` 582 refused
by both and 313 the same TIR of 895; `fuzz_tir` 700 programs, 434 the same
TIR and 266 refused by both; `frontsmoke` 47 of 47 through `tirinterp` and
through s3 and grasm-link; `tircontrols` 8 of 8; `tirtest` 1,671 emitted,
no holes; `gatecheck`, `fmttest` (1,968 formatted, none not), `linttest`
(none broke) and the litter check (no new untracked file, no new top-level
entry) clean.

**What it found about the Rust frontend.** Seven defects, and one more fact
about one already recorded, each in docs/known-issues.md with a reproducer
and none fixed in Rust. The Grasm frontend reproduces the six that are
diagnostics, so the two stay comparable and are fixed together; it cannot
reproduce a hash order, and where the Rust checker crashes it answers:

- a `^` chain is not charged to [G-NEST-LIMIT]'s cap, and one of 100,000
  terms aborts `grasm check` with a stack overflow (S1);
- a caret is placed by bytes on a line of characters, under a pad of spaces
  beside a tab, and past its span on a long line (S4);
- "did you mean" lists its candidates in hash order, different on every run
  -- the one the Grasm frontend cannot reproduce: it lists them by alias, and
  nothing in the repository, the Rust tests or the fuzzers' output has two
  (S4);
- a type the checker knows is shown as `_` (S4);
- a function with a contract reports its declared row's breach twice (S4);
- "cannot check the capability ceiling" drops the reason it computed (S4);
- a `match` on a value whose type is an error reports its arms' own names as
  unknown (S4);
- and, already recorded, a nested interpolation's spans: part three found
  the parser's diagnostics there rendered against the entry file's first
  line.

**What it found about the Grasm frontend.** Before this part: one message
misspelt (`ownv` for `own`), about 25 worded shorter than Rust's, no help
anywhere, the ceilings' messages its own (`["io"]` for `` `io` ``), the
loader's "cannot load module" naming the qualified module and not where it
looked, the manifests read and worded otherwise, and a package through a
symbolic link refused. The large fuzz run found one more: the checker recursed
down a `^` chain and an `else if` chain as the Rust one does, and a native
Grasm thread stops a recursion 6 MiB down, so `front check` and `front tir`
panicked on a `^` chain of about 3,700 terms and an `else if` chain of a few
thousand that `grasm check` accepts; the nesting cap counts neither chain
(the parser iterates both, §6.8). Fixed without the cap or the stack:
`infer_expr` and `check_expr` give an arithmetic operator and an `if` to
`sema.spine`, which makes the same calls in the same order with the right
operand or the `else if` on a work stack and the rest of the node's work in a
frame; `emit.expr` does the same for TIR and `emit.write` prints from a stack;
and every other walk over an expression -- the effect rows and the effect
pass, the ceilings' sites, the checker's predicates on branches -- takes the
right operand and the `else if` in a loop. Both chains now check alike at
6,000, 20,000, 50,000 and 100,000 terms in fifteen positions (a binding, a
function's last expression, a `return`, an argument, an interpolation, a
fallible function's tail, a closure, a `while` condition, a `parallel` block,
a test, a constant, a `match` arm, floats, an annotation, a type error),
except a `^` chain of 100,000, which aborts `grasm check` and which `front
check` answers. One more, open: `front tir` printed nothing for an `else if`
chain of 20,000, whose TIR is 4.0 GB (docs/known-issues.md).

**Speed.** Each program's whole `grasm check` against its whole `front check`
(`scripts/tirspeed.py --command check`): processes, start-up included,
alternating which runs first, five rounds, the median; lines are those of
every module a program loads, counted from `grasm tir`'s file lines.
2026-11-01, 02:58-03:06, both niced, beside the main checkout's full suite
at a one-minute load average of 3.6 to 5.4 -- **not to be quoted**:

| programs | lines | `grasm check` | `front check` | front / Rust time |
|---|---:|---:|---:|---:|
| `compiler/front`, `compiler/s3`, `tools/tirinterp`, `compiler/link`, `compiler/rt` | 74,513 | 8.8k lines/s | 2.9k lines/s | 3.00x |
| every tenth corpus program, the 168 both accept with the same output | 205,855 | 22.4k lines/s | 9.3k lines/s | 2.40x |

The 365-400k lines/s quoted for the Rust frontend's `check` (§2, §11) is
`docs/compile-speed.md`'s measurement of 2026-10-07: `grasm check`, parse and
typecheck, a whole process, best of five, on one generated file at each of
four sizes (`benchmarks/compile/gen.py` 200, 1000, 3000 and 7000, a record
type, a sum type and three functions per unit), lines as the file's lines.
The same programs today (`tirspeed.py --generated`, best of five,
alternating, 02:57 at a load of 4.0 to 4.5, so these are not to be quoted
either):

| lines | `grasm check` | `front check` | front / Rust time |
|---:|---:|---:|---:|
| 5,052 | 15.8 ms, 320k lines/s | 87.7 ms, 58k lines/s | 5.55x |
| 25,052 | 55.2 ms, 454k lines/s | 415.6 ms, 60k lines/s | 7.53x |
| 75,052 | 174.7 ms, 430k lines/s | 1,266.2 ms, 59k lines/s | 7.25x |
| 175,052 | 449.2 ms, 390k lines/s | 3,022.7 ms, 58k lines/s | 6.73x |

So on that measure the Grasm frontend checks about 58-60k lines/s, linearly
in the size of the file, against the Rust frontend's 390-450k here; on the
corpus it takes 2.4 times the Rust frontend's time and on the compiler's own
packages 3.0 times, where most of the difference is the capability ceilings
(§6.10). Where the time goes on one large file was not measured. Walking
arithmetic and `if` on work stacks costs `front check` about 6% there: on the
175,052-line file, eleven alternating rounds at 06:11, load 2.6 to 2.9 (not
to be quoted either), the median went from 3,016 ms to 3,206 ms (best 2,983
to 3,186), `grasm check` 454 ms.

**S2's exit.** §6 asks for token streams identical to `grasm tokens`
(`tokentest`), AST dumps identical (`asttest`), TIR identical for every
corpus program (`tirdiff`), the same diagnostics at the same spans for every
conformance and `check_tests` case (`diagdiff`, which holds every Rust test
suite's programs, not only `check_tests`', and the whole output, not only the
spans), a fuzz gate feeding both frontends (`fuzz_front`, `fuzz_tir`,
`fuzz_diag`), and check speed measured against the Rust frontend's figure
(above). Each is met. What is left: the one Rust output the Grasm frontend
cannot reproduce (a hash order); `front tir`'s empty output for a TIR of 4 GB;
and everything past `check` a command does on its own -- `deps`, the
lockfile, `fmt`, `lint` -- which is S8's.

### 6.14 S6: self-compilation

Written 2026-11-01. The Grasm compiler, built by the Rust one, builds itself,
and what it builds builds the same bytes again: stage 3 is stage 2.

**What "the compiler" is at this stage.** Not the `grasm` command, whose
seventeen subcommands are S8, but the pipeline the gates already drive from a
source file to a running program -- four programs:

| program | what it does | Grasm it is compiled from, packages included |
|---|---|---:|
| `compiler/front` | source to TIR (`front tir`), and the frontend's other commands (`check`, `tokens`, `ast`, `cst`, `files`, `toml`) | 25,561 lines, 21 modules |
| `compiler/s3`, with `compiler/mir`, `compiler/aarch64` and `compiler/macho` | TIR to a Mach-O object; with `--runtime`, the runtime's | 17,242 lines, 21 modules |
| `compiler/link` | objects to a signed executable | 6,238 lines, 21 modules |
| `compiler/rt` | the runtime every program links, the object `s3 --runtime` makes | 19,121 lines, 44 modules |

68,162 lines in all. Those four, because together they take a program from
source to a running binary with nothing else on the way -- `front tir P.gr >
P.tir`, `s3 P.tir -o P.o`, `link -interpose rt.o P.o -dead_strip -o P` -- and
no fewer do. What strings them together is a shell function
(`scripts/lib/stages.sh`) where `grasm build` will be; that driver is S8's.
`tools/tirinterp`, the TIR interpreter (7,076 lines), is built beside them
the same way, because `frontsmoke` takes it, and is compared like them; it is
not part of the compiler. The largest program built natively before S3 was
about 15,000 lines in 19 packages (§2); `compiler/front` alone is 25,561.

**The stages** (`scripts/lib/stages.sh`):

- **Stage 1**: `front`, `s3`, `link` and `tirinterp`, each built by the Rust
  `grasm build --native --no-incremental` with `GRASM_LINKER=cc` --
  Cranelift, the Rust runtime, the system linker. It has no runtime of its
  own: its programs run on the Rust one.
- **Stage 2**: every program built by stage 1 alone. Stage 1's `front` makes
  each program's TIR, stage 1's `s3` its object and the runtime's -- and the
  poisoned runtime's, which the use-after-free runs link (`rt.poison`, from a
  copy of `compiler/rt` whose `build.gr` says so, as `scripts/lib/rtswap.sh`
  makes it) -- and stage 1's `link` the executable, with that runtime and
  nothing else: `-interpose rt.o` and no runtime archive on the line, so
  every runtime name a program uses binds to the Grasm runtime or the link
  fails. No Rust runtime, no `cc`, no `grasm`. Neither stage 1's nor stage
  2's `front`, `s3` and `link` import `exec`, `fork`, `spawn`, `system` or
  `popen` (`nm -u`): they cannot start a process, so nothing like `cc` can be
  on the path.
- **Stage 3**: the same with stage 2's programs.

**Step one: stage 2 exists.** It built at the first attempt, and so did stage
3: no TIR the Grasm frontend refused, no construct the backend lacked, no
symbol the linker could not resolve, no runtime entry missing. Nothing in the
compiler was changed for it and nothing in the programs it compiled, so there
was nothing of that kind to record. Why it went so smoothly was not measured;
the backend, the linker and the runtime had each been held to every corpus
program with a `main` (`s3subset`, `linktest`, `rtswap`), and `frontsmoke` had
taken 47 programs from `compiler/front` through `s3` and `grasm-link`.

**Step two: stage 2 is right.** Every gate that takes the Grasm toolchain's
binaries ran with stage 2's, through `scripts/withstage.sh` (below): stage 2's
`front` makes every TIR they make, its `s3` every object, its `link` every
link they make with `compiler/link`, and what the Grasm backend writes is
linked by stage 2's `link` with stage 2's runtime alone. What they compare
against is what it is on the stage-1 path: `grasm tir`, `grasm check`, `grasm
run`, the Cranelift build as it ships, ld-prime, `clang`. 2026-11-01, on this
branch, the kept stage 2 of `fixpoint` (whose digests are below), beside the
same gates on the stage-1 path and other work, at a one-minute load average
between 3 and 61:

| gate | what of stage 2 it runs | with stage 2 |
|---|---|---|
| `tokentest` | `front` | 1,974 files identical, 4,111,888 tokens |
| `asttest` | `front` | 1,974 ASTs identical, 1,974 CSTs printed back |
| `parsediag` | `front` | 1,495 programs: 701 the Rust parser refuses, the same first diagnostic for all 701; 794 accepted by both |
| `fuzz_front` | `front` | 1,500 of 1,500 agreed, 1,264 with diagnostics |
| `loaddiff` | `front` | 1,737 programs: 1,671 the same modules (5,403), 64 loaded and refused by both, 2 unloadable by both, none disagreeing |
| `tomldiff` | `front` | 1,446 manifests read alike by the three readers |
| `tirdiff` | `front` | 1,974 files: 1,907 the same TIR (6,593 modules, 452,610,978 bytes), 67 refused by both, none disagreeing; 190 emitted twice, none unstable |
| `tirrefuse` | `front` | 895 programs: 582 refused by both, 313 the same TIR, none disagreeing |
| `fuzz_tir` | `front` | 700 programs: 434 the same TIR, 266 refused by both, none disagreeing |
| `diagdiff` | `front` | 4,244 programs: 1,437 refused with the same output (the 66 conformance negatives among them), 2,807 accepted by both, none differing; 114,791 diagnostics and 36,163 help lines compared; the 67 package cases and the 674 long chains the same |
| `fuzz_diag` | `front` | 2,200 of 2,200 the same output (seed 3), 7,565 diagnostics and 2,476 help lines compared |
| `frontsmoke` | `front`, `tirinterp`, `s3`, `link`, `rt` | 47 of 47 through `tirinterp` and 47 of 47 compiled, linked on the runtime alone and run, each as `grasm run` |
| `encodetest` | `s3` | 106 forms, 6,416 encodings identical to `clang`'s |
| `s3subset` | `front`, `s3`, `link`, `rt` | 648 of 648 identical to the Cranelift build |
| `objtest` | `front`, `s3` | 648 objects and the 2 runtime objects reproducible, 108,370 literal loads on their lines, no problem |
| `linktest` | `link`, with `front` and `s3` for its Grasm-backend half | 651 programs from the Cranelift objects (SDL ones included) and 648 from the Grasm backend's, each running as its `cc` link does, and the 3 C fixtures: 2,604 links, each made twice and the same |
| `linkdiff` | `link`, `front`, `s3` | 168 of 168 the bytes the native build ships; tools and sections agree on all 168, the 3 fixtures and the whole runtime |
| `rtswap` | `link`, `rt`, `rt.poison`, `s3`, `front` | 648 of 648 identical on the Rust runtime and on stage 2's alone, all 323 runtime names from Grasm; 648 agreeing with no use after free on the poisoned builds; 648 of 648 of the Grasm backend's objects |
| `rtdialect` | `front`, `s3`, `link` | 2 of 2 runs, 5 of 5 checks |
| the 16 `fuzz_rt*` gates | `link`, `rt` (their probes are Cranelift builds) | every seed identical on both runtimes, in each of the 16 |
| `tirinterp` | `tirinterp`, `front` | 651 of 651 programs run as `grasm run` runs them, 1,671 TIRs read and printed back the same, 25 of 25 sandbox panics the same |
| `s3rc` | `front`, `s3` (with `S3_LINKER` unset, so its poisoned links are the Rust runtime's) | 623 leak-free and 648 clean under the poisoned runtime, no use after free |
| `leakcheck`, `usecheck`, through `scripts/rtwith.sh` | `link`, `rt`, `rt.poison` | 623 leak-free, none leaking (25 end before the check); 648 clean under stage 2's poisoned runtime |
| the 21 generators with the Grasm backend as an engine (`fuzz_builder` to `fuzz_tls`, and `fuzz_s3`) | `front`, `s3`, `link`, `rt` | all 21 pass. 17 passed at once; four lost a binary to SIGKILL at loads of 2 to 67 and passed when rerun -- in three a reference (20 Cranelift binaries in `fuzz_captures` at a load of 43, one each in `fuzz_generics` and `fuzz_handlers`), in one a binary stage 2 built (`fuzz_fallible`'s seed 311, which its rerun, over the same seeds, ran and agreed on); `fuzz_generics` lost one more Cranelift binary on its first rerun, whose program runs to the same output when built again, and passed on its second. On the first runs 3,410 programs were offered to the Grasm backend through `scripts/lib/s3engine.py` and 3,407 agreed, 3,288 leak-checked, the other 3 being the comparisons a SIGKILL ended; `fuzz_s3` 60 of 60 identical |

Each of them passed on the stage-1 path too: rerun on this branch (below),
but for `leakcheck`, `usecheck` and the 20 generators other than `fuzz_s3`,
which passed in the main checkout's run of the same morning on `07c50893`
(this branch less S2 part three's frontend, which they do not use, and the
`FRONT=` switch, which was unset). And `stagediff` (below) holds stage 1 to
stage 2 directly: **every artifact the two compilers make of the corpus is
the same bytes** -- the runtime and the poisoned runtime each makes; `front tir` of all 1,974 `.gr` files git tracks
and the corpus holds, 1,907 the same TIR and 67 refused with the same
diagnostics; and for all 760 programs with a `main` among them, 757 the same
object and the same executable, with the 3 that link SDL set aside. So stage 2
makes what stage 1 makes of the whole corpus.

**Which gates of `scripts/allgates.sh` ran with stage 2, and which cannot
yet.** The suite is 238 gates. With stage 2: the ones in the table
above, and `fixpoint` and `stagediff`, which build their own stages. The rest:

- *Takes a stage's binaries, not run with stage 2 here:* `ra_check`
  (`S3=`), for the time: 56 minutes on the stage-1 path, of which most is
  `s3` allocating under stress, so about three and a half hours on stage 2's.
  Its allocation checker is part of `s3` and ran on every function of every
  object stage 2 compiled above.
- *Cannot take one, by design:* the seven mutation controls (`frontcontrols`,
  `tircontrols`, `diagcontrols`, `s3controls`, `linkcontrols`, `rtcontrols`,
  `rtdialectcontrols`) each build their own broken copy of a component with
  `grasm build --native`. A control of the self-built compiler is a fault
  planted in its source and carried through the stages, which is what
  `fixpointcontrols` does.
- *Use the Rust toolchain only:* everything else. They build and run programs
  with `grasm build --native`, `grasm run`, `grasm check` or `grasm test` --
  `difftest`, `leakcheck` and `usecheck` as the suite runs them, `agreetest`,
  `nativegaps`, `reprotest`, `inctest`, `crosstest`, `wasmtest`, the package
  gates and every `*diff` oracle gate, `gold`, `depth`, `panics`,
  `northstar`, the generators' two other engines -- and the Grasm compiler
  has no such commands until S8 (`front check` is the one it has, and
  `diagdiff` holds it to `grasm check`); or they check `grasm tir`'s own
  output (`tirtest`), a generated source file (`tirccall`), the Rust runtime
  (`rtlayout`, which probes it), `compiler/rt`'s source through `grasm tir`
  (`rtlint`), the Rust code (`clippy`, `unittests`), the formatter, the
  linter and the grammar (`fmttest`, `linttest`, `gfmttest`, `grammar`), or
  the tree and the gates themselves (`gatecheck`, `findings`, `litter`,
  `sitefacts`, `site`).

**On the stage-1 path**, on this branch the same day, every S2 to S5 gate
and this stage's passed: `tokentest`, `asttest` (1,974 files each),
`parsediag` (701 and 794), `fuzz_front` (1,500), `tomldiff` (1,446),
`loaddiff` (1,671), `tirdiff` (1,907 the same TIR, 67 refused by both),
`tirrefuse`, `fuzz_tir`, `diagdiff` (4,244), `fuzz_diag` (2,200),
`frontsmoke` (47 and 47), `tirtest` (1,671 emitted, no holes), `tirinterp`
(651), `frontcontrols` 8 of 8, `tircontrols` 8 of 8, `diagcontrols` 9 of 9;
`s3subset` and `difftest` (648 each), `objtest` (648, 108,370 literal loads),
`encodetest` (106 forms), `ra_check` (648 programs in 3,240 stressed runs,
243,010 functions checked, 43,489 planted allocation errors caught),
`s3rc` (623 leak-free, 648 clean under poison), `fuzz_s3` (60),
`s3controls` 29 of 29, `inctest` (5 of 5), `reprotest` (160 built twice, none
differing); `linktest` (651, 648 and 3), `linkdiff` (168), `linkcontrols` 9 of
9; `rtswap` (648), `rtlayout` (148 of 149 constants measured and equal, one
held to the source), `rtlint`, `rtdialect`, `rtcontrols` 80 of 80,
`rtdialectcontrols` 5 of 5 and the 16 `fuzz_rt*` gates; `fixpoint` (48 of 48),
`gatecheck` (after the one `grep -q` on a log in `withstage.sh`, which it
refuses under `pipefail`, became `grep -c`), `fmttest` and `linttest`;
`fixpointcontrols` 6 of 6 (below).

**Step three: the fixed point.** Stage 3 is stage 2, byte for byte -- every
executable, every object, every TIR, both runtimes -- at the first attempt,
and each stage is the same bytes when built twice. Stage 3, 2026-11-01:

| artifact | bytes | SHA-256 |
|---|---:|---|
| `front` | 1,504,040 | `6bf45168eabd60477bd1120ae243185a84853dcea171c85a6a2b8837f988f5a9` |
| `s3` | 909,680 | `4fb60de7f6de7e8458ea5c029f475abab54bfffea0980d10aeac4cde95e9c481` |
| `link` | 467,144 | `aa5c203476cd9550757434302a4f07d9d1a9dd332eb0beac2902f8bf58a85819` |
| `tirinterp` | 608,168 | `b675cc84752c3c5cbea4855f70a47ba5ef46001a8439e2d1f6c73404e2678374` |
| `rt.o` | 595,200 | `34fdb81c7123e91dda996a1676297d72f6143a7f586622c7c51792ee0bc019a5` |
| `front.o` | 2,185,320 | `aeb5cef5a9b691f53b599ac1d718fa6773f8ec4a1b38d79743d10ababe406878` |
| `s3.o` | 1,365,504 | `e103f9eff2bd89baf314b86bc02307a38ed7cc0c27c1a2c63b399095a1297238` |
| `link.o` | 433,640 | `60eadc4cc9b1765949436ad9680aa7ac0f44f6367b3975d141d51377ed3f6bce` |
| `tirinterp.o` | 575,816 | `ed6b7d507a4ee30da586f5e04050166cfa05f7214386023e38799cfb54bf0826` |

`rt.poison.o` (598,168 bytes) is left out: it carries the path of the copy it
was built from (below), so its digest belongs to one run. The objects carry
the checkout's directory, so theirs hold only for a build from the one
directory they were measured in; the executables' hold from any directory. Stage 1's binaries, Cranelift's code on the Rust runtime, are
larger: `front` 2,083,208 bytes, `s3` 1,320,192, `link` 643,624, `tirinterp`
867,208. And stage 2's outputs on the corpus are stage 3's: `stagediff` with
the kept stage 2 and stage 3: 1,974 files' TIR or diagnostics, 757 objects
and 757 executables of the 760 programs with a `main`, all the same; and
each tool's process time within 0.3% (1.00x), as one binary's should be.

**What a stage's bytes depend on**, found by building and comparing; each is
why the gates build the way they do.

- An object carries the directory it was compiled in -- `s3` writes `$PWD` as
  DWARF's compile directory -- and its TIR the source paths it was given; so
  two stages' objects compare only when built from one directory, as every
  gate builds them, from the checkout's root. The executables carry neither:
  no path is in any of them (`strings`), and stage 2 built by the same stage 1
  from a copy of the sources at another path has the same TIR and the same
  four executables, and objects that differ in that one string.
- An executable carries its own file name: the ad-hoc signature's identifier
  is the name it was written as, as ld's is, so two links of the same inputs
  compare only under one name (`stagediff` writes both under one, as
  `reprotest` does).
- The poisoned runtime is built from a copy of `compiler/rt`, and its TIR and
  object carry the copy's path; every stage of one run shares one copy.
- Stage 2 does not depend on how stage 1 was built: a stage 1 linked by
  `grasm-link` instead of `cc`, and one built by the Rust compiler as rebuilt
  at `5ed6777f`, made the same stage 2 and stage 3 to the byte.
- No hash order, address, clock or uninitialised byte reached an artifact:
  every stage built twice is the same bytes, in each of four `fixpoint` runs,
  and so is everything stage 2 and stage 3 make of the corpus.
- `monotonic_nanos()` answers 0 at its first call in a process -- it counts
  "since an unnamed moment" ([L-LIB-CLOCK]), and the moment is that call --
  so a planted clock has to read `now_millis()`, as `fixpointcontrols` does.

**The gates.**

- `fixpoint`: builds stage 1, stage 2 twice by stage 1 and stage 3 twice by
  stage 2, and compares every artifact a stage holds -- each runtime's TIR
  and object, each executable's TIR, object and executable, sixteen -- three
  ways: the two stage 2s and the two stage 3s with each other, where a
  difference is a compiler that answers one input two ways
  (NONDETERMINISTIC, named by whose compiler it is, Cranelift's build or our
  own), and stage 3 with stage 2, where a difference from a deterministic
  stage 2 is NOT A FIXED POINT: a miscompile that only compiling the compiler
  shows. Objects are compared and not only executables, because an object
  holds a line table the executable does not. It holds stage 2's six TIRs,
  which stage 1's `front` made, to `grasm tir`'s; asserts the count compared
  (48, derived from the stages' lists) and that the four programs of the
  compiler are among them; and prints each build's cost and stage 3's sizes
  and SHA-256s. About eight minutes.
- `stagediff`: two compilers' output on the corpus, byte for byte -- stage 1
  against stage 2 by default, built there, or any two kept stages
  (`STAGE_A=`, `STAGE_B=`). The runtime and the poisoned runtime each makes
  (TIR and object); `front tir` of every corpus program and every `.gr` file
  git tracks -- exit status, TIR and diagnostics; for every program with a
  `main` both accept, `s3`'s answer and object; and the executable each
  compiler's `link` makes of its own object with its own runtime. Nothing is
  run; whether the programs are right is the other gates' question. Floors:
  1,800 files' TIR and the subset's 620 executables. 43 minutes for stage 1
  against stage 2 and 71 for stage 2 against stage 3, two programs at a time
  at loads of 4 to 21.
- `fixpointcontrols`: four faults planted in a copy of the compiler's
  sources, all caught, six checks in all -- `s3` writing the clock into every
  object (NONDETERMINISTIC, stage 1's compiler run twice); `s3` writing a
  program's functions in the order of a hash seeded by `random_bytes`, as a
  hash table seeded per process iterates (the same); `s3` compiling the
  function `main::file_line` with every 1 that is added made 2 -- a function
  only `s3` itself has, the one that finds a line for the line table (NOT A
  FIXED POINT, with nothing nondeterministic) -- with two more checks of that
  one: stage 1 with the fault compiles 62 of 62 sampled corpus programs to
  the bytes it does without it and stage 3's executables are stage 2's, so
  only the objects' line tables show it, and `stagediff` of stage 1 against
  stage 2 finds 20 of 20 sampled programs' objects differing; and the gate
  told to leave its last comparison out (compared 47 of 48).
- `scripts/withstage.sh <gate>` runs any gate with every binary of the Grasm
  toolchain it takes being a stage's -- stage 2 built there, or the stage
  `STAGE=` names: `FRONT`, `TIRINTERP`, `S3`, `LINK`, `RT_OBJ` and
  `RT_OBJ_POISON`, and as `S3_LINKER` and `LINKER` `scripts/lib/stage_link`,
  which links what the Grasm backend wrote with the stage's `link` and the
  stage's runtime alone. What the gate compares against is left as it is.
  First it takes `examples/hello.gr` through the stage and stops unless that
  runs as `grasm run` runs it with every runtime name from the stage's
  runtime.
- The switches the gates lacked: the S3 gates' TIR (`scripts/s3_collect.py`),
  `objtest`'s runtime objects, `rtdialect`'s fixture, the S5 gates' runtime
  build (`scripts/lib/rtswap.sh`), the generators' third engine
  (`scripts/lib/s3engine.py`) and `fuzz_s3` make their TIR with `$FRONT tir`
  when `FRONT=` names a frontend; `tirinterp` takes `TIRINTERP=` and `FRONT=`.

**Cost.** Each stage's build, every step timed as `fixpoint` times it (wall
time, `/usr/bin/time -l`'s peak memory): three rounds, the stages interleaved
(1, 2, 3; then 3, 2, 1; then 1, 2, 3), the median of each step, 2026-11-01
18:56-19:10 with nothing else of this work running and a one-minute load
average of 4.6 to 6.2 from other work on the machine -- so not to be quoted.
The three `fixpoint` runs of that day, at loads of 3 to 25, gave the same
totals to within 2%. Each program's steps added up, and the largest peak of
any of them:

| program | stage 1 (Rust `grasm build`) | stage 2 (by stage 1) | stage 3 (by stage 2) | stage 3 / stage 2 |
|---|---:|---:|---:|---:|
| `front` | 4.35 s, 126 MiB | 14.00 s, 176 MiB | 53.55 s, 169 MiB | 3.83x |
| `s3` | 1.44 s, 88 MiB | 6.05 s, 141 MiB | 24.69 s, 130 MiB | 4.08x |
| `link` | 0.66 s, 57 MiB | 2.07 s, 58 MiB | 7.65 s, 57 MiB | 3.70x |
| `tirinterp` | 3.51 s, 54 MiB | 6.52 s, 75 MiB | 20.91 s, 72 MiB | 3.21x |
| `rt` | -- | 9.73 s, 103 MiB | 37.23 s, 102 MiB | 3.83x |
| `rt.poison` | -- | 9.63 s, 104 MiB | 37.35 s, 103 MiB | 3.88x |
| **all**, median of the rounds' totals | **9.9 s**, 126 MiB | **48.2 s**, 176 MiB | **181.3 s**, 169 MiB | **3.76x** |

Stage 2 and stage 3 do the same work -- the same inputs, the same bytes out
-- so the last column is the price of our backend's code against Cranelift's
on the compiler's own work: by tool, `front tir` 3.75x (35.8 s against 134.4
s), `s3` 3.97x (11.0 s against 43.5 s), `link` 2.87x (1.2 s against 3.4 s).
On the corpus `stagediff` measured the same, two programs at a time at a
load of 3.9 rising to 21: `front tir` 3.66x (842 s of process time against
3,079 s), `s3` 2.41x (243 s against 586 s), `link` 3.12x (63 s against 197
s); stage 2 against stage 3 there, the same binary twice, 1.00x. Where the gap comes from was not measured. The four
executables take the Rust compiler 9.9 s, stage 1 28.6 s (2.9x) and stage 2
106.8 s (10.8x); the two runtimes, which the Rust compiler does not build,
19.4 s and 74.6 s more. Peak memory stays under 180 MiB at every stage. The
binaries' sizes are in step three's table.

**S6's exit.** §6 asks for stage 2 and stage 3 bit-identical, and every gate
passing with stage 2 as the compiler. The first is met, held by `fixpoint`
and its controls. The second is met for every gate that can take the compiler
but `ra_check` -- each one in the table -- and stage 2 is stage 1, artifact
for artifact, on the whole corpus. What is left is the rest of the suite: the gates that build,
run, check or test through the `grasm` command, whose subcommands are S8; the
mutation controls, which build their own broken components with Rust's `grasm
build`; and `ra_check`, not yet run with stage 2's `s3`. Found and not fixed,
in docs/known-issues.md: an object names the directory it was compiled in, so
the fixed point holds for builds from one directory (the executables from
any). Until S8 the Rust `grasm` stays stage 0 and the driver of everything
the Grasm compiler does not yet do.

### 6.15 S3 part six: the mid-end

Written 2026-11-01. §9's risk table answers "our backend's code is slower
than Cranelift's" with "the release allocator and mid-end close the gap
before S9". This part is that mid-end's first five steps, taken in the order
a profile and the disassembly put them, each bounded before it was built
where a bound could be had, and each held to the S3 to S6 gates with its own
controls in `s3controls`. Everything is in `compiler/mir` and
`compiler/aarch64`; nothing in the language, the runtimes or the Rust
toolchain changed. Every measurement below was taken on this machine at a
one-minute load average of 2.4 to 5.7 from other work, above `bench.sh`'s
0.35 of the cores: none is to be quoted.

**Step 0: what the passes before them cost.** SSA construction and both
register allocators computed block liveness as a dense array of blocks by
variables iterated to a fixed point, and SSA construction found a block
parameter's incoming edges by scanning every block. Liveness is now found a
variable at a time, walking back from its upward-exposed reads through the
predecessors until a block that writes it, and a parameter's incoming edges
are read from a list made once; the release allocator's loop depths are
summed from differences. The results are the same -- the same parameters in
the same order, the same intervals and segments, the objects the same bytes
-- and only the cost changes: `compiler/s3`'s TIR compiled with step 1's
inline paths in 20.7 s before this and 4.7 s after (master's driver, without
them, 3.4 s, and 2.1 s with this step alone).

**Step 1: list elements, lengths and struct fields, inline.** An element of
a list of integers, `u64`s, floats, `bool`s or references is read, written,
pushed and updated where the code is, as the Cranelift build reads and pushes
one (`scalar_list_get`, `scalar_list_push`): a load or a store when the list
is a `ListScalar` of the element's kind (its tag and the kind byte after it,
one halfword) held in the `ScalarData` that kind uses -- words, or bytes for
`bool`s -- the index is inside it, and, for a store or a push, nothing else
holds the list (and, for a push, the buffer has room); in every other case the
runtime's call, which copies a shared list, grows a full one, reads a tuple
or a list of boxes, and panics on an index out of range with its own
message. `xs[i] += x` tests once for the read and the write. A list's length
is read the same way -- `len`, and the length every `for` over a list and
every `map`, `filter` and `fold` takes -- and so is a field of a struct: a
struct's fields are in the box for four or fewer and in a vector for more,
the same for every box of its type, so one test of the tag and the layout
reaches the field's word. A store into a scalar field of a struct nothing
else holds (`p.n += 1`) is a store of its bits, as the Cranelift build does
it. A list or a struct a local holds, read by an expression that changes no
local -- literals, local reads, operators and element reads -- is read
borrowed, with no `inc` before and `dec` after: the local's reference keeps it
alive throughout, as the Cranelift build's borrowed reads rely on. Each test
is a branch of its own; every test computed first and combined into one
branch, as `scalar_list_ok` does, measured 12% slower on `sieve` and 16% on
`matmul`. The offsets and tags are the runtime's, in
`compiler/mir/src/layout.gr`, the same names and values as
`compiler/rt/src/layout.gr`, and `rtlayout` now holds every one of them to
that file and so to its probe of the Rust runtime ("the backend's: 28, all
layout.gr's: 28"). A function past 4,096 blocks takes the calls instead, and
a list literal, which runs once, keeps its pushes as calls: a corpus program
of 133,000 instructions, most of them one literal, took over a minute to
compile when its pushes were inline.

**Step 2: constant folding and immediate forms.** `compiler/mir/src/fold.gr`
computes every integer operation whose operands are constants with Grasm's
own integer operations, which [E-ARITH-INT] and [T-INT-FIXED] define as the
machine instructions behave (wrapping `+ - *`, shift counts modulo 64);
division and remainder by zero, `MIN / -1`, an unsigned division of a word
with its top bit set, and every operation on doubles are left as they are. It
turns commutative operations and comparisons round so that a constant is
operand b, where the rules take immediates from; turns a branch on a
constant into a jump and drops what that leaves unreachable (a constant
divisor's zero test, `while true`); and runs a block that ends in a jump to a
block nothing else enters into it. New rules: `lsl`, `lsr` and `asr` by a
constant count (`ubfm` and `sbfm`); `and`, `orr` and `eor` with a bitmask
immediate, the encoder working out `N:immr:imms` from the 64-bit value as
LLVM's `processLogicalImmediate` does and refusing one that is not a rotated
run of ones repeated; a product with a power of two as a left shift and an
unsigned quotient by one as a right shift. The rule table's own check now
computes every immediate rule against its register form on edge words and
counts -- the mask decoded from the bits the encoder writes, not the value it
was given -- and `encodetest` holds the six new forms to `clang`, the bitmask
cases made from every element size, run and rotation by the architecture's
own decoding (`logimm_value`): 112 forms, 6,992 encodings.

**Step 3: addressing modes.** A load or a store whose address is an `add` of
a constant the access can take as its unsigned scaled offset -- not negative,
a multiple of the bytes it moves, at most 4,095 of them -- is `ldr`/`str` at
that offset from the `add`'s base, in the 64-bit, byte, halfword, word and
`d` forms, and one at a frame slot's address is at an offset from `sp`; the
`add` is not emitted when nothing else reads it. The rule table checks that a
rule absorbing an address is a load or a store whose instruction moves the
bytes its offset was admitted for. Register-offset forms (`ldr x, [x, x, lsl
#3]`) are not built: on `matmul`'s inner loop they would remove two
instructions of about fifty per element read.

**Step 4: inlining.** `compiler/mir/src/inline.gr`, once every function is
built, replaces a call of a function of at most 32 MIR instructions and
terminators by a copy of its body, bottom-up over the call graph's strongly
connected components (Tarjan's, iterative, each component after every one it
calls), so a callee's own small calls are part of it first. Never inlined: a
function on a cycle of calls, one that reads the stack pointer (so every
thread's stack floor, [E-RECURSION-DEPTH], is taken in the frame it was), one
that never returns; and nothing is inlined into a function C calls -- the
program's `main` entry, an `export fn`'s, a function address's -- so what C
calls is the wrapper it was. The copy's parameters are the call's arguments,
its values are renumbered, its frame slots are a region past the caller's own
that every copy shares (each copy's are in use only while it runs), a copy
that returns at one place answers that place's value, and every instruction
keeps its source offset, so the line table maps a copy to the callee's lines.
What runs is the same instructions in the same order: the same counts, calls
and panics; only the call, the callee's frame and the moves of the calling
convention go. A function nothing calls any more and whose address nothing
takes is no longer compiled. Before inlining, a call through a function value
the same function made a moment before from a function's address --
`xs.map(|x| ..)`, the closure written as the argument -- becomes `inc` of the
environment the value was made with and a direct call (`grasm_fn_code`
answers that address, `grasm_fn_env` that environment retained), so the
closure is inlined too; after it, an `inc` and a `dec` of one value with only
register arithmetic between them cancel (`compiler/mir/src/tidy.gr`).
`objtest`'s sample of instructions whose line is known now takes a literal
written once and loaded more than once, once per copy, when no other name in
the program is that text (a field's, a variant's or a C function's name, which
the compiler loads where the field is read or the function called, is the
same string in the pool), and holds every copy to the literal's own line:
137,906 loads on their lines, 31,366 of them of literals loaded more than
once. `s3 --inline=N`
sets the bound, 0 none. The bound was chosen from a sweep before it was
fixed: the runtime's object built at each bound, `wordfreq` and `tree` timed
on it, two sessions of 9 rounds. At 0, 24, 32, 48 and 64, `wordfreq` took
37.8, 29.0, 26.7, 26.1 and 25.8 ms and `tree` 37.6, 32.2, 32.5, 32.1 and 31.5
ms (8 and 16, in the other session against 38.2 ms at 0: 31.2 and 30.1, and
`tree` 35.3 and 32.8 against 37.7), for an object of 50,377, 53,506, 53,594,
64,609 and 73,963 words (47,057 and 51,371 at 8 and 16): 32 takes most of
what 64 does for a seventh of its growth.
Inlining into a function C calls as well -- the runtime's entry points'
wrappers -- measured 3% on `front tir` and is not done; whether "no `export
fn`'s C entry changed" allows it is the owner's to say.

**Step 5: shrink-wrapping.** `compiler/aarch64/src/emit.gr`: the frame -- the
frame record, the callee-saved registers the body writes, the stack area --
is set up at the start of the nearest block that dominates every instruction
needing it (a call, a spill or a reload, a frame slot, a stack argument, a
callee-saved register), moved up the dominator tree until it is in no loop
and dominates every return it can reach, and on entry at worst. A return on a
path that never set it up is `retab` alone: `pacibsp` stays on entry in
every function, signing the return address, which every path authenticates.
A leaf that touches no callee-saved register has no frame at all. The plan is
linear in the blocks, a preorder of the dominator tree answering each
dominance in two comparisons, with at most eight moves up the tree. Of the
runtime's 1,037 functions 57 run with no frame and 33 set theirs up past the
entry; a function whose parameters live across a call is given callee-saved
registers for them from its first instruction, so its entry needs the frame,
and splitting those live ranges is the allocator's work, not done here.

**What holds it.** Thirteen new controls in `s3controls`, each a fault in
the step's own code that the gate named catches for its reason:
`list-stride` (an inline element read and written at four bytes an element),
`list-cow` (an inline store with no test that nothing else holds the list),
`field-slot` (a field read from the word after its own), `fold-sub` (folding
`a - b` as `a + b`), `branch-fold` (a branch on a constant going the other
way), `addr-off` (an absorbed offset one element out), `csr-slot` (each pair
of callee-saved registers restored from the other's slot) and `shrink-ret` (a
return past the entry taking the frameless way out whether or not its path set
up the frame), all by `s3subset`; `imm-bit` (a bitmask immediate's run one
longer) by `encodetest`; `layout-backend` (a `Vec`'s length where its capacity
is, in the backend's copy of the layouts) by `rtlayout`; `direct-env` (a
closure called directly handed its environment released, not retained) and
`rc-cancel` (a pair cancelled across a call) by `s3rc`; `inline-line` (a
copy's instructions mapped to the call's line) by `objtest`. `s3 --stats`
prints the mid-end's counts on a second line, as `ra_check` reads the first by
its fields' positions. The constant folding left a fixture function
with no return (its `while true` has no way out once the branch is folded),
so `rtdialect` now holds every compiled function to `pacibsp` on entry and no
plain `ret`, counted against the functions s3 says it compiled rather than a
floor of 30 that inlining went under, and `rtdialectcontrols` gains
`frameless-ret` (a frameless return left a plain `ret`). On the last step's
tree: `s3subset` and `difftest` 648 of 648 identical; `objtest` 648 objects
reproducible; `ra_check` 3,240 stressed runs identical and 34,625 of 34,625
planted errors caught; `s3rc` no program leaking, none using freed memory
under poison; `fuzz_s3` 60 of 60; `fixpoint` 48 of 48 artifacts the same,
stage 3 being stage 2, and `stagediff` 757 executables the same;
`fixpointcontrols` 6 of 6, `s3controls` 42 of 42, `rtdialectcontrols` 6 of 6,
`rtcontrols` 80 of 80; `encodetest` 6,992 encodings as `clang`'s;
`linktest`, `linkdiff`, `rtswap`, `rtlayout`, the runtime's fuzzers and the
generators that build natively, `leakcheck` and `usecheck` through `rtwith`
(623 leak-free, 648 clean) all pass.

**The ceilings, measured before building.** Step 1: `sieve` and `matmul`
built with every scalar element read an unchecked load and every write an
unchecked store, the list read borrowed (wrong for a shared or short list,
right for these two), ran at 0.447 and 0.169 of master's backend's time;
the built step, its tests in place, at 0.545 and 0.319 in the same session.
Testing an updated element once rather than twice was bounded by removing a
write's tests altogether: `matmul` at 0.816 of its time with them; built, it
ran at 0.817. Step 4's bound was the sweep above, taken before the bound was
fixed. Steps 2, 3 and 5 were not bounded before they were built; what they
removed is in the sizes below.

**What it measures**, 2026-11-01 in the evening. The benchmarks, against the
Cranelift build (`scripts/bench.sh`'s eight programs, master's backend and
this one built from the same TIR, linked with the Rust runtime, 21 rounds
interleaving the three binaries, medians; `s3bench.py`'s own run the same
evening gave a geometric mean of 2.079):

| program | Cranelift | master's backend | this backend | before / after, over Cranelift | after / before |
|---|---:|---:|---:|---|---:|
| fib | 5.69 ms | 8.23 ms | 8.25 ms | 1.445 / 1.448 | 1.002 |
| sieve | 7.05 ms | 46.68 ms | 14.14 ms | 6.617 / 2.005 | 0.303 |
| matmul | 5.82 ms | 71.22 ms | 14.65 ms | 12.230 / 2.516 | 0.206 |
| wordfreq | 16.48 ms | 50.67 ms | 49.99 ms | 3.074 / 3.033 | 0.987 |
| tree | 20.05 ms | 37.73 ms | 27.71 ms | 1.882 / 1.383 | 0.735 |
| sort | 10.62 ms | 13.63 ms | 11.21 ms | 1.283 / 1.056 | 0.823 |
| parmap | 60.16 ms | 120.18 ms | 120.76 ms | 1.998 / 2.007 | 1.005 |
| parwords | 11.23 ms | 65.63 ms | 53.89 ms | 5.842 / 4.797 | 0.821 |
| geometric mean | | | | 3.17 / 2.05 | 0.647 |

The Grasm runtime compiled by each backend against the Rust runtime, the
programs Cranelift's (`rtbench.py`'s shape, the two runtime objects and the
Rust archive in one run, 21 rounds; `rtbench.py`'s own run gave 1.133):

| program | Rust runtime | Grasm runtime by master's backend | by this one | after / before |
|---|---:|---:|---:|---:|
| fib | 5.37 ms | 5.33 ms | 5.48 ms | 1.028 |
| sieve | 6.68 ms | 7.23 ms | 7.28 ms | 1.007 |
| matmul | 5.55 ms | 5.57 ms | 5.40 ms | 0.970 |
| wordfreq | 16.00 ms | 38.64 ms | 25.25 ms | 0.654 |
| tree | 19.98 ms | 37.78 ms | 31.80 ms | 0.842 |
| sort | 10.70 ms | 10.91 ms | 9.90 ms | 0.907 |
| parmap | 61.23 ms | 62.05 ms | 61.34 ms | 0.989 |
| parwords | 11.55 ms | 12.46 ms | 11.13 ms | 0.893 |
| geometric mean over the Rust runtime | | 1.237 | 1.117 | 0.903 |

The compiler building itself, as §6.14 measured it: each tree's stage 1
(Rust), stage 2 (by stage 1) and stage 3 (by stage 2), every step timed, three
rounds interleaving the stages and the two trees -- master and this one,
each compiling its own sources -- medians, loads 2.9 to 4.8:

| | stage 1 | stage 2 | stage 3 | stage 3 / stage 2 |
|---|---:|---:|---:|---:|
| master | 9.6 s | 48.2 s | 182.9 s | 3.80x |
| this part | 9.8 s | 53.9 s | 97.4 s | 1.81x |

Stage 3 takes 0.53 of master's time; stage 2 takes 12% longer, the price of
the passes and the larger code, which the Cranelift-built stage 1 pays. By
tool, stage 3 over stage 2 -- our backend's code against Cranelift's on the
same work -- `front tir` 2.05x (36.3 s against 74.6 s; master 3.75x), `s3`
1.29x (16.0 s against 20.6 s; 3.97x), `link` 1.39x (1.4 s against 2.0 s;
2.87x). Stage 3's executables: `front` 1,820,856 bytes, `s3` 1,362,576, `link`
572,136, `tirinterp` 677,384 (master's 1,504,040, 909,680, 467,144, 608,168;
stage 1's, Cranelift's, 2,083,208, 1,320,192, 643,624, 867,208).

Code size, step by step: the 648 programs of the subset and the runtime,
`__text` words (and the runtime's object in bytes):

| | corpus | runtime |
|---|---:|---:|
| master | 11,201,063 | 53,707 (595,200 bytes) |
| step 0 | 11,201,063 | 53,707 |
| step 1 | 13,222,957 | 53,707 |
| step 2 | 13,061,830 | 51,857 |
| step 3 | 12,352,108 | 50,043 |
| step 4 | 12,739,708 | 52,891 |
| step 5 | 12,737,217 | 52,614 (555,152 bytes) |

The corpus's code grew 13.7% (its objects 5.2%), most of it the inline paths'
tests and their calls kept for the cases that fail them; the runtime, which
holds no Grasm lists, shrank 2.0% (6.7% in bytes), its 1,739 functions now
1,037.

**What is left.** Where the gap to Cranelift is now: `wordfreq` (3.0x) and
`parwords` (4.8x) spend their time in reference counting, string building and
map calls that the Cranelift build elides or makes inline -- the Perceus-style
pass of §4, not built here, and a division by a constant, which Cranelift
makes a multiplication; `front tir` at 2.05x of Cranelift's time spends much of it
in `inc` and `dec` (a profile's leaves put a third of its samples there, and
leaves overstate; it was not priced by removing them). Not built: register-offset
addressing, live-range splitting (which shrink-wrapping needs to leave most
non-leaf fast paths frameless), inlining into C entries, the SCCP, GVN and LICM
of §4's mid-end.

### 6.16 S7 part one: an x86-64 backend, Mach-O, under Rosetta

Written 2026-11-02, before the code past the encoder, as the design to build
to. S7 is "x86-64 Mach-O (crosstest becomes self-hosted), then Linux ELF on
x86-64 and arm64". This part is the first half: `compiler/s3` compiling the
corpus subset for x86-64 macOS, the runtime in Grasm compiled for it, both
linked into executables that run on this arm64 machine under Rosetta 2
(`arch -x86_64 ./prog`) and print and exit as the Cranelift x86-64 build and
the interpreter do -- what `crosstest` holds the Cranelift build to today.
Linux is part two, and waits on a decision of the owner's: where its
binaries run.

**What runs it.** Rosetta here offers an x86-64 with SSE through SSE4.2,
CMOV, CMPXCHG16B and AES, and nothing past them: no AVX (the
`hw.optional.avx*` flags are 0 under `arch -x86_64`), no POPCNT, LZCNT or
BMI, and an empty leaf-7 feature list, so no CET; pages are 4 KiB to an
x86-64 process. So the backend targets x86-64 with SSE4.1 -- `roundsd` for
`floor` and `ceil` is the one instruction past SSE2 it uses -- and counts
bits with the instructions every x86-64 has (`bsr` and `cmov` for leading
zeros), as Cranelift does without its optional features. x86-64 has no
counterpart of pointer authentication here: CET's shadow stack and
indirect-branch tracking are not offered under Rosetta, so no `endbr64` is
emitted and no return address is protected. Nothing replaces `pacibsp` on
this target, as nothing does in the Cranelift x86-64 build or in clang's
default for `x86_64-apple-macos`; the arm64 target keeps its signing
unchanged (the two targets share no frame code; below).

**Choosing the target.** `s3 --target x86_64` (the default stays `aarch64`,
so no command line that works today changes meaning). One driver, one MIR,
two instruction sets: `compiler/mir` stays target-neutral -- nothing in it
names a register, an instruction or a convention -- and everything from VCode
down is chosen by the target. TIR is target-neutral already (both targets are
64-bit little-endian with the same layouts, which `rtlayout` holds to the
runtime's), so `grasm tir` makes the same TIR for both and the S3 gates' TIR
collection serves both. MIR has one limit worth naming: a program's C
function takes eight integer and eight double arguments at most
(`extern_ok_for`), which AAPCS64 passes in registers and System V passes six
integers of in registers and the other two on the stack -- both conventions
the targets' own selection implements, so MIR does not change.

**What becomes shared, and what each target owns.** `compiler/aarch64` mixes
code that is about AArch64 with code that is about VCode. The second kind
moves into a package of its own, `compiler/vcode`, parameterised by a
*target description* -- a record of tables, not functions, as principle 5
asks, built once by each target:

| shared (`compiler/vcode`) | per target (`compiler/aarch64`, `compiler/x86_64`) |
|---|---|
| `VCode` itself (`vcode.gr`) | `mach.gr`: opcodes, encoder, assembly text |
| linear scan and its spill rewrite (`regalloc.gr`) | `rules.gr`: the selection table and its own checks (the condition maps against the flags, the immediate forms against the operations) |
| the release allocator (`backtrack.gr`) | `isel.gr`'s convention half: argument and result registers, stack arguments, how a call and a return are written |
| the allocation checker (`check.gr`) | `emit.gr`'s target half: prologue, epilogue, branch encoding and relaxation, relocations |
| the planted faults of `ra_check` (`plant`) | `leaves.gr`: the runtime dialect's context switch and thread-local getters |
| the frame's placement (`frame_plan`: dominators, shrink-wrapping) | `enctest.gr`: the encoder's cases |
| block arguments as parallel moves (`par_moves`) | the target description |
| the rule engine's plan: which rule each instruction takes, what is absorbed, what is left unread (`isel.plan`, `choose`, `expand`), with the guards and immediates a target defines | |

The target description is what the allocators and the checker read where
they now call `mach.*` and `regalloc.*`'s AArch64 tables -- about twenty-five
places, counted: per opcode, the four fields' roles (written, read, both) and
files (general or floating-point), and whether it is a call; the registers a
call reads (its argument registers, by file) and writes (its result's) and
clobbers; which registers are callee-saved, which are never allocated (sp and
x29, x30 and x18 on AArch64; rsp and rbp on x86-64) and which are scratch for
reloads (x15..x17 and d29..d31; r10, r11 and xmm14, xmm15); the orders the
allocators try registers in, normally and under `--max-regs`; the move,
load and store opcodes a rewrite inserts. Operand numbering is already
common: general registers below 64, floating-point from 64 (`R_D0`, `R_X0`),
virtual from 128 (`VR`). The refactor is a step of its own, and its gate is
that it changes nothing: every object the subset's programs and the runtime
compile to is the same bytes from the refactored driver as from master's (the
check step two ran for its own change, below), and `fixpoint` holds. The
allocator is parameterised, not forked.

Two things about x86-64 the shared code must allow, both of which the
AArch64 forms already exercise:

- *Two-address arithmetic.* `add %rm, %rd` reads and writes `rd`. Field 0 is
  `ROLE_USEDEF`, as `movk`'s and `casal`'s are, and a rule writes `mov a,
  res; add b, res`. The release allocator's move hints can make such a move
  one from a register to itself, which emission drops.
- *Registers an instruction does not name.* `idiv` reads and writes rax and
  rdx, `cqo` writes rdx from rax, a shift by a register counts in cl,
  `cmpxchg` compares with rax. Each is a field of the VCode instruction, fixed
  to its one register (`mach.fixed_reg`), with a role: the allocator sees a
  physical register's range exactly as it sees a call's argument registers
  today, and keeps every value live across it out of that register; the
  checker checks the read like any other; the encoder refuses any other
  register there. Field 3 (`ra`) gains roles other than "read" for this:
  `idiv`'s remainder is read and written there.

The flags are not modelled as a register. A comparison and its reader
(`setcc`, `jcc`, `cmovcc`) are adjacent in one rule's template, and the only
instructions anything inserts between two instructions -- a reload, a spill,
a block argument's move, a constant -- are `mov`s, which do not touch the
flags; `P_MOVIMM` becomes `movl`, `movq` or `movabsq`, never `xor`. The rule
table's own check will refuse a template that puts an instruction which sets
the flags between a setter and its reader.

**The convention: System V AMD64, as Apple uses it.** Integer and pointer
arguments in rdi, rsi, rdx, rcx, r8, r9 and doubles in xmm0..xmm7, counted
apart; the rest on the stack, eight bytes each in argument order, the first
at the lowest address, which is `0(%rsp)` at the call and `16(%rbp)` in the
callee; the result in rax or xmm0. A call clobbers rax, rcx, rdx, rsi, rdi,
r8..r11 and every xmm register; rbx, rbp and r12..r15 are callee-saved, and
so are the control bits of MXCSR and the x87 control word, which nothing
compiled changes. The stack is 16-byte aligned at every call, so on entry
`rsp` is 8 past a multiple of 16. A variadic call puts in al an upper bound
on the xmm registers it used. Narrow arguments: clang extends 8- and
16-bit arguments to 32 bits at the caller, which the System V document does
not promise, so the Grasm C entry keeps re-extending each narrow argument from
its own width (it does so on AArch64 already) and a narrow C result is read
at its width -- the same code on both targets. The 128-byte red zone below
`rsp` is not used. The hidden stack-floor argument of a recursive function,
the `inout` slots, the split result's word, and the closure convention are
MIR's and do not change: they are ordinary arguments.

**The frame and unwinding.** Kept as on AArch64 but for the return address,
which `call` pushes:

    [args in] [return address] [rbp] [rbx, r12..r15 as written] [pad] [spill] [slots] [args out] <- rsp

`pushq %rbp; movq %rsp, %rbp`, then a `pushq` of each callee-saved register
the body writes, then `subq $n, %rsp` with `n` keeping `rsp` 16-aligned; the
epilogue undoes it and `retq`. The frame-pointer chain is kept, so debuggers,
`sample` and the panic path walk the stack as they do now. Shrink-wrapping is
the shared `frame_plan`: a path that never sets the frame up returns with a
bare `retq`, its return address never having left the stack. Spill slots and
outgoing arguments are reached from `rsp` with a 32-bit displacement, so the
32,760-byte limit the AArch64 frame has from its 12-bit offsets does not
apply. Unwinding: the backend writes no unwind information on either target
-- no `__compact_unwind`, no `__eh_frame` -- because nothing unwinds: a panic
prints and exits (101), and no C function the runtime or a program calls
unwinds through its caller. A debugger needs only the frame-pointer chain, and
dwarfdump's line table is the same `dwarf.gr` with one relocation type
renamed (`X86_64_RELOC_UNSIGNED` is 0, as `ARM64_RELOC_UNSIGNED` is).

**Selection, MIR operation by operation.** Every form named here is in the
encoder now and held to the assembler (below); the table is what step three
writes as rules.

| MIR | x86-64 |
|---|---|
| `iconst` | `movl $k, %r32` (0..2^32-1, the upper half cleared), `movq $k, %r` (a sign-extended 32-bit k), else `movabsq` |
| `add sub and or xor` | `mov a, res` then the operation with b; with b a sign-extended 32-bit constant, `op $k, res` (the encoder picks the 8-bit or 32-bit immediate, or the short `rax` form, as the assembler does) |
| `mul` | `imulq b, res`; by a constant, `imulq $k, a, res`; by a power of two, `shlq $k` |
| `sdiv srem` | a into rax, then `cqo; idivq b`, the quotient in rax or the remainder in rdx; when b is -1 the quotient is `negq` of a and the remainder 0, since `idiv` faults on `MIN / -1` where [E-ARITH-INT] answers MIN -- a pseudo-instruction expanded at emission into `cmpq $-1, b; jne; ...`, as Cranelift's x86-64 backend expands `srem` (`CheckedSRemSeq`) |
| `udiv urem` | a into rax, `movl $0, %edx`, `divq b`; by a power of two, `shrq $k` |
| `shl sshr ushr rotr` | b into rcx, `shlq/sarq/shrq/rorq %cl, res`; by a constant, the immediate form; x86-64 counts modulo 64 as Grasm does |
| `neg not` | `negq`, `notq` after the move |
| `sext8/16/32`, `uext8/16/32` | `movsbq`, `movswq`, `movslq`; `movzbl`, `movzwl`, `movl` |
| `icmp` | `cmpq b, a` (or `$k`), `setcc t`, `movzbl t, res` |
| `straddr dataaddr fnaddr celladdr` | `leaq sym(%rip), res` (X86_64_RELOC_SIGNED) |
| `gotaddr` | `movq sym@GOTPCREL(%rip), res` (X86_64_RELOC_GOT_LOAD) |
| `getsp`, `slotaddr` | `movq %rsp, res`; `leaq off(%rsp), res` |
| loads and stores, every width | `movq`, `movzbl`, `movsbq`, `movzwl`, `movswq`, `movl`, `movslq` from `disp(%base)`; `movq`, `movb`, `movw`, `movl` to it; an absorbed `add` of any 32-bit constant is the displacement |
| `fconst` | its bits by `P_MOVIMM` into a general register, then `movq` across |
| `fadd fsub fmul fdiv` | `movaps a, res` then `addsd`/`subsd`/`mulsd`/`divsd b, res` |
| `fneg fabs` | the sign bit's mask made as a constant and moved across, then `xorpd`/`andpd` |
| `fsqrt ffloor fceil` | `sqrtsd`; `roundsd $9` and `$10` |
| `fcmp` | `ucomisd`, with the operands swapped for `<` and `<=` so that unordered reads false through `a`/`ae` (CF and ZF set); `==` is `setnp` and `sete` and'ed, `!=` `setp` or'ed with `setne` |
| `scvtf`, `fcvtzs`, `fbits`, `ffrombits` | `cvtsi2sdq`; `cvttsd2si` with the saturation and NaN-to-0 [L-LIB-AS-I64] asks for (`cvttsd2si` answers MIN for both), a pseudo-instruction like `sdiv`'s, as Cranelift's `CvtFloatToSintSeq` is; `movq` out and in |
| `ucvtf`, `fcvtzu` | the unsigned conversions x86-64 lacks, by the sequences Cranelift uses (`CvtUint64ToFloatSeq`, `CvtFloatToUintSeq`): halve with the low bit kept and double for a word with its top bit set; subtract 2^63 for a double at or past it -- pseudo-instructions too |
| `clz rbit bswap*` | `bsrq`, `cmovzq` of -1 for a zero word, and 63 less the index; a bit reversal by `bswapq` and three mask-and-shift swaps; `bswapq`, `bswapl`, `rolw $8` then `movzwl` |
| `aload*` | the plain load: x86-64 is total-store-ordered, and every sequentially consistent store below locks, so a load is sequentially consistent, as LLVM and Cranelift make it |
| `astore*` | `xchg`, which locks |
| `aadd*` | `lock xadd`, which answers the old value; a narrow one widened by `movzbl`/`movzwl`/`movl` after, as AArch64's `w` forms clear the upper bits |
| `acas*` | the expected value into rax, `lock cmpxchg new, (a)`, the old value out of rax, widened the same way |
| `brif` | a fused comparison as `cmpq` and `jcc`; a fused `fcmp` as `ucomisd` and one or two `jcc`s (`==` jumps to the else block on parity first); otherwise `testq a, a; jne` |
| `jump`, `trap` | `jmp`; `ud2` |
| `call callrt callind` | arguments by the convention, `callq sym` (X86_64_RELOC_BRANCH) or `callq *%r` |
| `inc dec` | calls of the runtime, as on AArch64 |

Branches are relaxed at emission: every block branch starts with an 8-bit
displacement and is widened to 32 bits until no displacement overflows, a
fixpoint that only grows, so it ends. A call or a RIP-relative form always
carries its relocation, with its four bytes zero, as the assembler leaves
them.

**The object.** `compiler/macho` gains a CPU type (`CPU_TYPE_X86_64`,
subtype 3) and byte-addressed code in place of words; its relocations are
x86-64's: `BRANCH`, `SIGNED`, `GOT_LOAD` and `TLV`, all PC-relative and four
bytes, at the displacement that ends the instruction (`mach.reloc_kind`),
and `UNSIGNED` (8 bytes) for the descriptors in `__data`, the thread-local
descriptors and the DWARF addresses, as on arm64. `__text` is 16-byte
aligned and each function starts at a multiple of 16, as clang places them.
The line table is `dwarf.gr`'s, its rows at byte offsets.

**The runtime.** `compiler/rt` is target-neutral Grasm but for four things,
found by reading every `extern fn` and every constant in it:

1. *The dialect's leaves* (`compiler/aarch64/src/leaves.gr`) get x86-64
   counterparts in `compiler/x86_64`. A context is rbx, rbp, r12..r15, rsp,
   MXCSR and the x87 control word (64 bytes, within the 176 `fiber.gr`
   reserves). `ctx_switch(from: rdi, to: rsi)` stores them at `from`, loads
   them from `to` and `retq`s: the return address it returns through is the
   one on the stack it switched to. `ctx_init(ctx, top, entry, arg)` aligns
   `top` down to 16, stores `_grt.ctx_start`'s address at `top - 8` and that
   as the saved `rsp`, entry and argument as rbx and r12, rbp zero (so a
   backtrace ends there), and the control words as they are; the first switch
   to it `retq`s into `_grt.ctx_start` with `rsp` 16-aligned, which moves r12
   to rdi, `callq *%rbx` and `ud2`s if that returns. The thread-local getter
   is `pushq %rbp; movq %rsp, %rbp; movq var@TLVP(%rip), %rdi; callq
   *(%rdi); popq %rbp; retq`, the address in rax. Every form they need is in
   the encoder (`stmxcsr`, `ldmxcsr`, `fnstcw`, `fldcw`, `call-m`, `load-tlv`
   among them).
2. *Variadic C functions.* `open`'s mode and `fcntl`'s argument are variadic
   in C, and the runtime declares each with eight register parameters before
   it so that AAPCS64 puts it in the first stack slot, where Apple's arm64
   `va_arg` reads it (`fsys.gr`). System V passes a variadic argument as a
   fixed one, in the next register, so on x86-64 that declaration would leave
   the mode in rdx as 0 and pass the real one on the stack, where `open` never
   looks -- a file created with mode 0, the defect docs/binary-size.md
   records for the Rust runtime's arm64 build. The language cannot say
   "variadic" (the systems subset left it out, §5.2) and the frontend is not
   to change, so the x86-64 target keeps a table of the two C functions the
   runtime calls that C declares variadic -- `open` and `fcntl`, with two
   fixed parameters each -- and passes such a call's first two arguments and
   its last as the first three, the six between dropped, with `movl $0,
   %eax` for al. It is the fact a C compiler would read from `<fcntl.h>`,
   kept where the convention is, and it applies only to `s3 --runtime`.
3. *`$INODE64`.* On x86-64 the 64-bit-inode `opendir` and `readdir` are
   `_opendir$INODE64` and `_readdir$INODE64` (plain `_readdir` returns the
   old 32-bit-inode `struct dirent`); on arm64 there is only the plain name.
   The x86-64 target names those two as the C library does, and then
   `files.gr`'s `DIRENT_NAME` of 21 is right on both, as the struct is the
   same.
4. *Nothing else.* The page size (16 KiB on arm64, 4 KiB under Rosetta) only
   rounds a fiber stack's guard up; the error numbers, `open` flags,
   `kevent`, `sockaddr`, `pthread` sizes and `dlsym`'s `RTLD_DEFAULT` are the
   same on both; maps hash and probe eight control bytes at a time in plain
   Grasm on both (the Rust runtime's x86-64 build probes sixteen with SSE2,
   which changes no program's output: iteration is in sorted order).

`s3 --runtime --target x86_64` makes `rt.o` for x86-64; `rtswap`'s shape
holds it to the Rust runtime's x86-64 build (`target/x86_64-apple-darwin`),
which `crosstest` already builds.

**The linker.** `compiler/link` refuses x86_64 by name today (§6.6). Step
four teaches it the x86-64 half of what it does for arm64, measured against
ld-prime's output for x86-64 as the arm64 half was: the five relocation types
the objects carry -- `BRANCH` to the function or to a 6-byte `__stubs` entry
(`jmp *slot(%rip)`), `SIGNED`, `GOT_LOAD` (with ld's relaxation of a GOT load
of something defined here to `leaq`), `TLV`, `UNSIGNED` -- 4 KiB pages, the
`__PAGEZERO` and segment layout ld gives x86-64, chained fixups in the
format ld-prime writes for x86-64 (read off its output, as the arm64 half's
`DYLD_CHAINED_PTR_64_OFFSET` was), and whatever signature ld writes for
x86-64 (none is required to run there). Until then the system linker links
x86-64 objects, as it linked the arm64 backend's during S3, and that is
recorded as the step's gap.

**Gates.** Built with this note: `encodetest_x64` and `x64controls`
(below). To come, each with its controls in `x64controls`:

- `xs3subset`: every program of `s3subset`'s subset compiled by `s3 --target
  x86_64`, linked, run under Rosetta, and held to the Cranelift x86-64 build
  (`grasm build --native --target x86_64-apple-darwin`, `crosstest`'s
  reference) and so to the interpreter -- stdout, exit status and stderr --
  with the subset's floor. This is crosstest with the Grasm backend as the
  compiler, which is what "crosstest becomes self-hosted" asks.
- `objtest` for x86-64 objects: `otool`, `nm`, `dwarfdump --verify` and the
  linker accept every one; relocations against instruction classes
  (`scripts/x64_macho.py`, which reads x86-64 objects as `s3_macho.py` reads
  arm64 ones); each object compiled twice the same bytes.
- `ra_check --target x86_64`: the allocation checker over every function
  under register pressure, with its planted faults.
- `xrtswap`: the corpus on the x86-64 Grasm runtime, then on it alone.
- The existing gates all still pass, and AArch64 output is byte-identical to
  master's: every object of the subset and the runtime's, compiled by the
  branch's driver and by master's from the same TIR in the same directory,
  and `fixpoint`.

**Decisions** (raised with this note, 2026-11-02; (1) and (3) were taken by
the coordinating session within the owner's standing rules and the owner may
reverse them; (2) is the owner's and is open). (1)
The runtime's two variadic C functions on x86-64 go through a per-target
table, as above; no `...` in `extern fn`, since the language is frozen and the
frontend does not change. (2) Linux, part two, stays with the owner -- where
its binaries run -- and does not start. (3) SSE4.1 is the floor: `roundsd`
for `floor` and `ceil`. Every Intel Mac has it, Rosetta provides it, and it
is part of the x86-64-v2 baseline. On a machine without it -- x86-64-v1, which
no Mac is -- a program reaching `floor` or `ceil` stops with an illegal
instruction (SIGILL) the first time it runs one; it never computes a wrong
answer, since the processor refuses the instruction rather than reading it
as another.

**Step two: the encoder** (2026-11-02). `compiler/x86_64` (`mach.gr`,
`enctest.gr`): 102 instruction forms -- every one the table above, the frame,
the calling convention, the allocator's spills and the runtime's leaves use --
each with its roles, its register files, the registers it fixes, its
encoding and its AT&T text. Where the assembler chooses between encodings the
encoder chooses as it does: an 8-bit immediate or displacement when the value
fits, the short `rax` forms of the arithmetic group, `d1` for a shift by one,
no displacement for 0 but from rbp or r13, a SIB byte for rsp and r12, the
REX prefix an 8-bit spl, bpl, sil or dil needs, and a 16-bit locked
instruction's operand-size prefix before `lock`. `s3 --encode-test <p>
--target x86_64` writes 5,558 cases -- every general and xmm register in every
field, immediates, displacements and branch offsets at both ends of each width
and between, all sixteen conditions, every base register -- and
`scripts/encodetest_x64.sh` assembles them with `as -arch x86_64` and
compares, case by case and in total length, the bytes and, for the 51 that
name a symbol, the relocation the assembler wrote: its type, its place, its
symbol. `x64controls` plants nine faults in a copy of the target and requires
the gate to fail on each for its reason: `add`'s opcode one bit off, REX.R
written as REX.W, 128 taken for an 8-bit immediate, no SIB byte for rsp and
r12, no displacement for 0 from rbp and r13, `lock` before the operand-size
prefix, a short branch's displacement one long, a RIP-relative address
relocated as a GOT load, and a form the target claims to emit with no cases.

On the step's tree (2026-11-02, a one-minute load of 3 to 5): `encodetest_x64`
"forms: 102   cases: 5558   bytes: 28464   relocated: 51   identical: 5558
different: 0"; `x64controls` "controls: 9   caught: 9   missed: 0";
`encodetest` 112 forms, 6,992 encodings identical to `clang`'s, as before.
Nothing on the AArch64 path changed, and that was checked rather than
assumed: every program of the subset, 648 of 648, and the runtime (555,152
bytes) compile from the same TIR in the same directory to the same object
bytes with this driver as with master's; `fixpoint` 48 of 48 artifacts
identical, stage 3 being stage 2; and, since the new package's files are
among the `.gr` files git tracks, `tokentest` and `asttest` 1,981 of 1,981
files, `tirdiff` 1,914 the same TIR and 67 refused by both, none disagreeing,
`diagdiff` 4,251 programs, none differing, and `fmttest`, `linttest` and
`gatecheck` pass. Not run: the rest of the suite, `stagediff` and
`s3controls` among them, which this step's change does not reach.

**Step three, part one: the shared package** (2026-11-02). `compiler/vcode`
holds what the table above calls shared but the rule engine's plan, which
stays in each target's `isel.gr` for now: `vcode.gr`, `regalloc.gr`,
`backtrack.gr` and `check.gr` moved out of `compiler/aarch64`, `frameplan.gr`
(the shrink-wrapping plan, out of `emit.gr`) and `moves.gr` (`mov` and
`par_moves`, out of `isel.gr`), and `target.gr`, the description. A
description is a record of tables: per opcode the four fields' roles and
files, whether it is a call, its form's name; per physical register whether
it is one a value lives in, whether a call changes or keeps it, whether the
body may write it, whether touching it needs the frame, its name; the
argument and result registers, the registers fixed on entry (the frame
pointer), the allocation, stress and scratch orders by class, the move, load
and store a rewrite inserts and the move's source field, the stack pointer,
the direct call, the largest frame. `compiler/aarch64/src/desc.gr` builds
AArch64's from `mach.gr` and AAPCS64's lists, the answers the allocators
asked before. The only generalisations the code needed were these: a call
through a register reads it if its role says so (was: if it is `blr`); every
field may be written and read as its role says (field 3 had only "read");
the entry's argument registers and the call's clobbers come from the
description rather than from register numbers 0..7 and 0..17.

It changed nothing AArch64 emits. From the same TIR in the same directory,
the branch's driver and master's made the same bytes for all 648 programs of
the subset and for the runtime (555,152 bytes), and for every eighth program
the same objects and `--stats` lines under linear scan, with 2 and with 0
registers under both allocators, and the same counts under each of the three
planted allocation faults: 567 more compilations, all the same. `s3controls`'
`allocator` and `backtrack` controls name the files where they now are, and
each, run alone against its moved file, was caught (`ra_check` and
`fuzz_s3`, "register allocation check failed"). On the moved tree: `ra_check`
"programs: 648   stressed runs: 3240   identical: 3240   failed: 0" with
34,625, 2,061 and 3,308 planted errors of its three kinds caught; `fixpoint`
48 of 48 identical; `encodetest` 6,992 and `encodetest_x64` 5,558
encodings as the assemblers'; `tokentest`, `asttest` and `tirdiff` over the
1,986 tracked files; `fmttest`, `linttest` and `gatecheck`.

**Step three: selection, allocation and the object** (2026-11-02). `s3
--target x86_64` compiles a program for x86-64 macOS. `compiler/x86_64`
holds what is x86-64's and nothing else:

- `rules.gr`, the rule table, 133 rules over the same MIR operations as
  AArch64's, by the selection table above. Two-address arithmetic is a move
  then the operation; the registers an instruction does not name are fixed
  selectors (`S_RAX`, `S_RCX`, `S_RDX`, `S_RSP`), held by the table's check
  to the register the form fixes; a temporary a pseudo-instruction reads, and
  rdx before a division, is made first by a constant, so that no operand is
  handed a register the sequence writes before it reads that operand. The
  table's own checks, which `encodetest_x64` runs: coverage and shadowing as
  AArch64's; every operand of its field's file; no instruction that sets the
  flags between a comparison and the `setcc`, `jcc` or `cmov` reading them;
  the integer conditions against the flags `cmp` sets, on words at the edges
  of both orders; each comparison of doubles -- `fcmp` and the fused branch,
  all six conditions -- evaluated from the table's own templates (the
  `ucomisd` operands' order, the conditions, how two combine) by IEEE 754 on
  both zeros, the infinities, the extremes, a subnormal and a NaN; every
  immediate form against its operation.
- `isel.gr`: selection by the table, System V's convention, and the
  runtime's two variadic C functions passed as C declares them (decision
  (1)).
- `emit.gr`: the frame; branch relaxation, every block branch short until out
  of reach, to a fixpoint; and four pseudo-instructions expanded with short
  branches of their own -- signed division and remainder, -1 answering as
  [E-ARITH-INT] asks; an unsigned word to a double; a double to a signed and
  to an unsigned word, saturating, NaN to 0 ([L-LIB-AS-I64]).
- `desc.gr`, the target description; `leaves.gr`, the dialect's leaves.

`compiler/macho` writes an x86-64 object when asked. The driver pads each
function to 16 bytes with `int3`, and in the runtime names `opendir` and
`readdir` `$INODE64` (a program's keep their plain names, as the Cranelift
build's do). Nothing AArch64 emits changed: the same 648 objects, the
runtime's and 567 variant compilations as master's, byte for byte.

**Step four: the runtime and the linker.** `s3 --runtime --target x86_64`
compiles `compiler/rt` (1,037 functions, 224,207 bytes of code) with the
leaves above. `compiler/link` links x86-64, as its header says: the objects
decide the architecture; pages are 4 KiB; a stub is `jmp *slot(%rip)`;
x86-64's relocations are mapped to kinds of the linker's own as they are
read and applied as 32-bit displacements, a GOT or thread-local load of
something defined here made a `leaq` (the Cranelift build's
`X86_64_RELOC_GOT` of its own data too); everything else is the arm64 code,
and an arm64 link is the same bytes as before (`linkdiff`, `linktest`). So a
program goes from TIR to a running x86-64 executable with nothing but the
Grasm toolchain's programs: `s3 --target x86_64`, `link -interpose rt.o`, and
the runtime `s3` compiled. What the linker did not do for x86-64 in this part
is match ld's bytes; the exit items below do.

Decisions taken by the session implementing this part, which the owner may
reverse: the linker signs an x86-64 executable as it signs an arm64 one,
where ld signs none (neither runs differently under Rosetta; reversed by the
same session for the exit items below, so that the bytes can be ld's); functions are
padded with `int3` where clang pads with `nop`s (nothing runs the padding);
`$INODE64` only in the runtime; `objtest_x64`'s floor of literal loads is
60,000, since x86-64 loads a literal's address with one `leaq` where arm64
uses two instructions, each counted (68,953 against 137,906); and
`ra_check_x64` counts apart, rather than failing, a program in which no double
is given a register -- System V preserves no xmm register, so a double living
across a call is in memory, and the control that puts a double in a general
register has nothing to plant there.

**Step five: the gates** (on the final tree, 2026-11-02 and 3, a one-minute
load of 2 to 5):

- `xs3subset`, crosstest with the Grasm backend as the compiler: "subset: 648
  in, 0 out, 3 external, of 651 programs with a main"; "identical: 648
  mismatched: 0   timed out: 0   no TIR: 0" -- each the Cranelift x86-64
  build's stdout, exit status and stderr, and the interpreter's.
- `objtest_x64`: "objects: 648   reproducible: 648   literal loads on their
  lines: 68953 (of literals loaded more than once, as inlined copies are:
  15683)   with problems: 0" -- otool, nm, `dwarfdump --verify`, the
  relocations against the instructions they end, a clean link.
- `xrtswap`: "Cranelift objects: identical: 648   poisoned runtime, agreeing
  with no use after free: 648"; "Grasm backend objects: identical: 648
  linked twice, the same bytes: 648"; "runtime names the programs reference:
  323 distinct, all taken from Grasm; per link, 64136 references"; the
  runtime "345 names moved ...; it calls none of the Rust runtime's".
- `ra_check_x64`: "programs: 648   stressed runs: 3240   identical: 3240
  failed: 0"; 179,600 functions checked, every program spilling with no
  registers; planted errors caught 34,500 of 34,500, among doubles 1,667 of
  1,667, a double in a general register 2,809 of 2,809 (5 programs where no
  double is given a register, counted apart).
- `encodetest_x64`: 133 rules, no problems; 102 forms, 5,558 encodings as
  the assembler's, 51 relocations as its.
- `x64controls`: "controls: 22   caught: 22   missed: 0" -- the encoder's
  nine; the rule table's integer condition map (caught by its check, and with
  the check skipped by `xs3subset`'s output), the operands of `<` on doubles
  not swapped, a flag setter between `bsr` and its `cmov`; the epilogue's pops
  in the pushes' order, rcx handed to values living across calls, a reload's
  scratch register handed out (the allocation checker); a relocation a byte
  short, the line table a byte late, the clock in every object
  (`objtest_x64`); a stub through the wrong GOT slot, a GOT or thread-local
  load not made a `leaq`, and the context switch restoring `rsp` from the
  wrong word (`xrtswap`). Each control builds the driver and the linker from
  its own mutated copy. Two of the first run's were found vacuous and
  replaced: removing rcx from what a call changes is no fault, since no value
  that crosses a call is ever offered it, and a new fiber's stack one word
  lower runs correctly under Rosetta.
- Unchanged for arm64: `linkdiff` 168 of 168 byte-identical to the shipped
  build, tools and sections agreeing, the 3 C fixtures and the whole runtime;
  `linktest` 651 Cranelift and 648 Grasm-backend programs, 2,604 links each
  made twice the same; `fixpoint` 48 of 48; `encodetest`; the identity of
  every subset object and the runtime's with master's.

**S7's exit items that need no decision of the owner's** (2026-11-03, asked
for by the coordinator once part one was merged):

*linkdiff for x86-64.* `scripts/linkdiff_x64.sh` holds `compiler/link` to
ld-prime on the same x86-64 objects, as `linkdiff` does on arm64's: every
fourth program of the subset and every one with an `export fn` or a
thread-local cell, linked twice each -- the Cranelift x86-64 object and the
Grasm backend's, each with the Rust runtime built for x86_64-apple-darwin,
`-dead_strip` -- byte for byte with `-strip -tool_version` against
`cc -arch x86_64` and `strip -no_uuid`, and unstripped by `dyld_info -fixups
-exports`, `nm -m`, `otool -l` and every section and __LINKEDIT blob
(`scripts/link_sections.py`); then the three C fixtures compiled with `-arch
x86_64`, symbol tables included, and the whole x86-64 runtime, its 306
members after a C `main` without `-dead_strip`. What it compares away is
what `linkdiff` compares away for arm64: the UUID, ld's tool entry, the
symbol table's size and what follows it (ld's debug map, §6.6). Nothing is
signed on either side.

Matching it took nine changes to the linker, each held by a control:

- A function whose compact entry defers to DWARF is
  `UNWIND_X86_64_MODE_DWARF`, 0x04000000, on x86-64 (arm64's is
  0x03000000), so the Rust runtime's `__eh_frame` and `__gcc_except_tab` were
  being dropped.
- An x86-64 FDE written by LLVM carries its function's and its LSDA's
  address as displacements with no relocation; the linker now reads the CIE's
  augmentation ("zPLR") for the fields' widths, finds the LSDA by its
  address, and rewrites both fields where the records land. A CIE's
  personality is an `X86_64_RELOC_GOT`.
- `X86_64_RELOC_GOT` is not a load ld may make a `leaq` (that is
  `X86_64_RELOC_GOT_LOAD`'s): the Cranelift build's loads of its own data,
  which it marks so, and the Rust runtime's `addq sym@GOTPCREL(%rip)` reach a
  local target through a GOT slot, rebased, as ld's do. Part one had made the
  loads `leaq`s, which ran and was not ld's image.
- Section-relative `SIGNED` and `BRANCH` relocations, which the Rust runtime
  has and the Grasm backend does not write, resolve by the displacement the
  field holds.
- ld signs no x86-64 executable, and now neither does this linker. Part one
  had signed them (my decision then); I reversed it, so that the bytes can be
  ld's -- Rosetta runs either.
- The thread-local template's sections all take the largest alignment among
  them, on both architectures (measured on arm64 too, with a 32-aligned
  `__thread_bss`).
- The chained fixups describe a segment's pages up to the last with a fixup,
  not all of them.
- A label at the very end of an input section (the runtime's
  `.Lfunc_end...` of `__rust_probestack`) is an empty atom of ld's: it has a
  function start and an unwind entry of its own, unless it ends `__text`, and
  `-dead_strip` removes it, symbol and all.
- ld leaves 48 bytes after an x86-64 image's load commands, not 32: room for
  an `LC_CODE_SIGNATURE` it does not write (measured with `cc -arch x86_64`
  over every residue: never less than 48, and 40 is refused). And `strip`
  sizes a stripped x86-64 image's __LINKEDIT in 16 KiB pages where ld uses
  4 KiB.

None changes an arm64 link: `linkdiff`, `linktest` and `fixpoint` below.

*The fuzzers on x86-64.* `scripts/lib/s3engine.py` takes `S3_TARGET=x86_64`:
the driver compiles with `--target x86_64`, the object is linked by `cc -arch
x86_64` (or `S3_LINKER`) with the x86-64 Rust runtime, and the reference is
built by the engine from the same source -- the Cranelift build for
x86_64-apple-darwin, linked by `cc -arch x86_64` -- rather than the
generator's arm64 binary, so that both sides share an architecture and its
libm (the ulps that differ between the two, as the demos found). Both run as
`arch -x86_64 <binary>` with twice the generator's time limit. A run of a
binary written over in place and ended by SIGKILL is run once more from a
fresh inode, as for arm64: `scripts/lib/inplace.py` now looks through `arch`
to the binary. `fuzz_s3` takes the same switch, and now records its own
in-place writes so that its runs get the kill-rerun too. `scripts/fuzz_x64.sh`
runs `fuzz_s3` and every generator that loads the engine with the switch,
counted from the scripts (20), each required to pass and to print the x86-64
engine's line with something agreed and nothing failed.

*Benchmarks under Rosetta.* `scripts/s3bench.py` takes the same switch: the
Grasm x86-64 backend's binaries against the Cranelift x86-64 build's, timed
in alternation, the load printed before and after each program. Under
Rosetta both are translated, so the figures measure the translation as much
as either code generator, and the script says they are not to be quoted.

**The gates** (on the final tree, 2026-11-03, a one-minute load of 2 to 6
from other work on the machine):

- `linkdiff_x64`: "sampled: 165 of 648 programs, 330 links   byte-identical
  to ld and strip: 330   tools and sections agree: 330   C fixtures agree: 3
  of 3   whole runtime (306 members) agrees: 1   (ld 1115.7.3)".
- `fuzz_x64`: "fuzzers: 21 of 21 (fuzz_s3 and 20 generators on the s3
  engine)   passed: 21   programs agreed: 3470" -- fuzz_s3's 60 programs
  each also run with three registers, and every generator's engine line with
  nothing declined and nothing failed. It took 78 minutes.
- `s3bench`, `S3_TARGET=x86_64`, 21 rounds, a load of 2.4 to 4.5 on 8 cores
  -- not to be quoted: the Grasm backend's code over Cranelift's, fib 1.119,
  sieve 1.381, matmul 1.747, wordfreq 2.716, tree 1.139, sort 0.997, parmap
  1.492, parwords 2.001; geometric mean 1.494.
- `x64controls`: "controls: 32   caught: 32   missed: 0" -- the 22 before;
  x-pop-order's backend also through `fuzz_x64` ("FAILED: fuzz_s3"); and the
  nine linker faults above, each caught by `linkdiff_x64`: signing (the load
  commands), 32 bytes of room (a program's bytes), every page in the chained
  starts, the empty atom's function start, the GOT reference made a `leaq`,
  arm64's DWARF mode, the template's alignments (the tlv fixture), the LSDA
  field four bytes off, strip's page size (the stripped bytes). The signing
  control first missed: it changed one of the three places that decide
  whether there is a signature, so the link failed rather than differed;
  signing is now one predicate.
- Unchanged for arm64: `linkdiff` "sampled: 168   byte-identical to the
  shipped build: 168   tools and sections agree: 168   C fixtures agree: 3 of
  3   whole runtime (401 members) agrees: 1"; `linktest` 651 Cranelift and 648
  Grasm-backend objects identical, 2,604 links each made twice the same;
  `fixpoint` 48 of 48 identical; `linkcontrols` 9 of 9; `s3controls` 42 of
  42 (with the coordinator's fp-class repair, cec5512e).
- `xrtswap` with the linker that no longer signs x86-64: 648, 648 and 648,
  linked twice the same 648.

**What is left for S7's exit.** Crosstest is `xs3subset`, self-hosted; the
fuzzers, ld's bytes and the benchmarks are above. What remains is a Linux CI
run, which is part two and the owner's decision (2).

### 6.17 S8 part one: a `grasm` driver in Grasm, for what Grasm can already do

Written 2026-11-03, with the driver's argument handling, before its gate
(`clidiff`) and before it is built by itself -- as the design to build to.
S8 is "every `grasm` subcommand, the LSP, DAP, packages, journal and
replay, `doc` and `api`, and the wasm32 backend". Part one is the
subcommands whose machinery is already Grasm, put behind one command that
answers as the Rust `grasm` answers: the same reading of its arguments, the
same messages on the same streams, the same exit statuses and files. The
Rust toolchain stays stage 0 and the oracle; nothing in it changed.

**One program, `compiler/grasm`.** The package is named for the command it
is, and sits beside `front`, `s3` and `link` because it is the compiler's
driver, the thing §6.14 left to a shell function (`scripts/lib/stages.sh`)
"where `grasm build` will be". It is one executable that imports the four
programs' packages -- `compiler/front`, `compiler/s3` (with `mir`,
`aarch64`, `x86_64`, `macho`, `vcode`), `compiler/link` and
`tools/tirinterp` -- and calls them in its own process. The alternative was
the stage's executables side by side, started by the driver; it was not
taken (by the S8 agent), for four reasons:

- `grasm run` cannot be done that way faithfully. `proc_run` captures a
  child's output and gives it no input ([L-LIB-PROC]), so a program run
  through it could not read stdin and would print nothing until it ended.
- Nothing has to start a process. The S6 programs import no `exec`, `fork`,
  `spawn`, `system` or `popen`; a driver that starts them would be the one
  part of the compiler that can, and the one that could start the Rust
  `grasm`. In one program the TIR and the object pass in memory: no
  temporary files, no `$TMPDIR`, no process identifiers.
- A release ships one file and its runtime, as the Rust one ships `grasm`
  beside `libgrasm_rt_abi.a`.
- The stages stay what §6.14 made them: four programs and a runtime, each
  built and compared as now, and the driver one more program built and
  compared the same way.

What it costs: the four share one namespace of types ([T-MODULE-NS]), and
one name clashed -- `Prog` in `compiler/front` and in `tools/tirinterp` --
so the interpreter's is now `TirProg`. The executable is 4.0 MB (stage 2);
§6.14's stage-2 `front`, `s3`, `link` and `tirinterp` were 3.5 MB together. Building it is
one large compilation: 50 s and 521 MB at peak by stage 1, 82 s and 589 MB
by stage 2 (once each, 2026-11-03, at a one-minute load of 2 to 7 from other
work; not to be quoted) -- the Rust compiler takes 50 s for it too, where
it takes 10 s for the four apart.

What the packages gave the driver, none of it changing what their own
commands do:

| package | change |
|---|---|
| `compiler/front` | `cmd_check`, `cmd_tokens`, `cmd_ast`, `read_source` and the writers public; `checked`, `load_and_check`'s path that `check`, `tir`, `run` and `build` share, split out of `cmd_check`; `read_source` words a failure as Rust's `io::Error` (`io_error_text`), where it printed the runtime's `NotFound(...)` -- `tokentest` and `asttest` read only files that exist, so no gate saw it; the manifest's `links` kept, and `declared_links`, the Rust command's walk of the package graph for them; `rtoml` keeps the entries of an array of tables, which the loader never reads and `grasm toml` prints |
| `compiler/s3` | `compile_program`: the object `s3 P.tir -o P.o` makes, as bytes; `compile` reads the file and calls it |
| `compiler/link` | `parse_args` and `run` public; an input may be handed over in memory (`mem_path`, `mem_data`) |
| `tools/tirinterp` | `Prog` renamed `TirProg`; `run_tree`, `run` on TIR already read |

**The subcommands of part one**, and whose code each runs:

| subcommand | what runs it |
|---|---|
| `version`, `--version` | the driver: `grasm 1.1.0`, the workspace's version as a constant `clidiff` holds to Rust's |
| `tokens`, `ast` | `front`'s `cmd_tokens` and `cmd_ast`, which `tokentest` and `asttest` hold to Rust's |
| `toml` | `rtoml`, the port of the Rust reader the loader reads manifests with, printed as `grasm toml` prints -- not `packages/toml`, which reads TOML itself and answers otherwise on a float, an inline table or an array across lines (§6.13); one reader is S8's to leave, not part one's |
| `check` | `front`'s `cmd_check`, which `diagdiff` holds to Rust's |
| `tir` | `front`'s check path, then its TIR. Rust exposes it, and it is what `grasm tir` prints: every diagnostic rendered, where `front tir` prints them unrendered for the gates |
| `run` | the check path, the runtime refusal, then `tirinterp` on the TIR in this process |
| `build --native` | the check path, then `s3` and `link` in this process, with the runtime object beside the driver (below) |
| `links` | `front`'s loader: the system libraries the packages declare, as the Rust command collects them for its link line. Not in the brief's list; added (taken by the S8 agent) because `build` needs the walk anyway and `difftest` and two other gates ask `grasm links` which programs call C |

**`run`: the TIR interpreter, in the process.** Both ways were measured:
`front tir` and `tirinterp`, against `front tir`, `s3`, `link` and running
the executable, stage 1's binaries, on `examples/hello.gr` and the nine
`benchmarks/grasm` programs, each once, 2026-11-03 at 11:12, a one-minute
load of 1.8 to 3.8 (not to be quoted):

| program | `front tir` | `tirinterp` | `s3` + `link` | the executable | Rust `grasm run` |
|---|---:|---:|---:|---:|---:|
| hello | 0.01 s | 0.46 s | 0.52 s | 0.45 s | 0.03 s |
| fib | 0.01 s | 1.46 s | 0.05 s | 0.95 s | 1.73 s |
| sort | 0.01 s | 1.21 s | 0.05 s | 0.47 s | 0.41 s |
| wordfreq | 0.01 s | 0.90 s | 0.06 s | 0.56 s | 0.52 s |
| tree | 0.01 s | 1.94 s | 0.05 s | 0.53 s | 1.54 s |
| parwords | 0.01 s | 2.71 s | 0.06 s | 0.56 s | 1.96 s |
| sieve | 0.01 s | 4.22 s | 0.05 s | 0.49 s | 2.82 s |
| matmul | 0.02 s | 7.23 s | 0.06 s | 0.57 s | 6.00 s |
| mandelbrot | 0.01 s | 24.19 s | 0.06 s | 0.49 s | 12.74 s |
| parmap | 0.03 s | 112.31 s | 0.06 s | 0.89 s | 53.80 s |

Every output was the same. The first run of a freshly written executable on
macOS costs about 0.45 s whatever it does (the `hello` row's `tirinterp`,
and every executable's), so the compiled path never costs less than half a
second and is far faster on any program that computes: 0.55 s against 24 s
for `mandelbrot`. It is still the interpreter (taken by the S8 agent), for
what `grasm run` is: the
program's stdin, stdout and stderr are the command's own, as they are under
the Rust `grasm run`, which `proc_run` cannot give a child; nothing starts a
process; and it is the engine the Rust command runs, so a runtime package
is refused in the same words, "the interpreter" -- the TIR interpreter
cannot run the runtime dialect either. The TIR interpreter takes 0.8 to 2.9
times the Rust interpreter's time on the nine benchmarks (2.1 times on
`parmap`). The
panic's stack trace is the one difference [E-PANIC-TRACE] allows between
engines, and the TIR interpreter, a native build, prints none; `clidiff`
strips it as `tirinterp.sh` does.

**`build --native`.** The Rust command's checks, in its order and words:
the operand (the first argument not a flag, if it is the first or ends in
`.gr`), `--link`'s values then the packages' `links`, the output (`-o`, or
the source's stem), not a regular file, its directory missing, the source
itself, a `.gr` name, `--target` without `--native`; then the check path,
no `main`, a runtime package. Then the program's TIR to `s3`'s
`compile_program` -- the object a stage's `s3` makes, release allocator,
inlining at 32 -- and the object to `link` with the runtime object,
`-interpose rt.o <object> <libraries> -dead_strip -exported_symbol _<e>...
-strip -o <out>`, as a stage links its programs (§6.14) and with `-strip`
as the Rust command links with `grasm-link`. `GRASM_KEEP_SYMBOLS` leaves
the symbols, as there. A link that fails is reported as the Rust command
reports it: the C functions no library defines, or the linker's words less
the lines naming the object. Where it differs -- each taken by the S8
agent, accepted by the coordinator, and the owner's to reverse:

- **The success line names the backend.** `built `out` natively (N bytes,
  Grasm)` where Rust says `Cranelift`: the size differs anyway, since the
  code is another compiler's, and the line would otherwise say something
  false about what built the file. The usage keeps "compile to machine code
  (Cranelift)" to the byte, because the brief holds `--help` byte for byte;
  it becomes false the day the Grasm driver ships, and is the owner's to
  reword in both.
- **`--no-incremental` is accepted and changes nothing.** The Grasm backend
  compiles a whole program into one object; there is no module cache. So
  without the flag the Rust command writes `.grasm-cache/` beside the
  output for a program of more than one module and this one writes
  nothing: a file the Rust command writes that this one does not.
- **`--target` takes x86-64 and arm64 macOS only** (`x86_64-apple-darwin`,
  `aarch64-` or `arm64-apple-darwin`, `-macos...`), and writes the object
  `s3 --target x86_64` writes, under the Rust command's name and message
  (`-o x.o`, else `<out>.o`; "emitted ... (link it with a ... toolchain +
  libgrasm_rt_abi.a)" -- true of the Grasm backend's objects too, which
  `xs3subset` links with that archive). Any other triple, which Cranelift
  takes for ELF, is "error: unsupported target `t`: the Grasm backend writes
  Mach-O objects for x86_64 and aarch64 macOS only", exit 1, where the
  Rust command emits an object; Linux is S7 part two.
- **The runtime object is `rt.o` beside the driver**, or in `../lib/grasm/`,
  as the Rust command looks for `libgrasm_rt_abi.a` beside itself and in
  `../lib`; `rt.o` because that is its name in a stage's directory. It
  reads neither `GRASM_RUNTIME` nor `GRASM_LINKER`, which name the Rust
  runtime's archive and `cc`. Missing, it says where it looked, exit 1.
- **The runtime is built by the driver**: `grasm build compiler/rt/src/main.gr
  --native -o rt.o` writes the object `s3 --runtime` writes (`.o` added
  to any other `-o`, as for `--target`), and says "built `rt.o`, the
  runtime `rt` (N bytes, Grasm)". The Rust command refuses that line ("no
  `main` function"), and its refusal of a runtime package names the Grasm
  backend as the one that takes it. Without this a driver could not make
  the runtime its own builds link, and stage 2 could not be built by stage
  1's driver alone; with it, it is a line the two commands answer
  differently, and `clidiff` names it as one.

**How it finds its parts.** Everything but the runtime is compiled into it;
the runtime object is found beside the executable, whose path it asks of
`_NSGetExecutablePath(3)` (what Rust's `current_exe` asks) and resolves
with `realpath(3)`. It never looks for the Rust `grasm`, `grasm-link`, `cc`
or any other program, and its package calls neither `proc_run` nor
`proc_pipeline`: the capabilities `net`, `proc`, `mem` and `async` in its
manifest are the interpreter's, performed for the program `run` runs, as
the Rust `grasm run` performs them.

**Every other subcommand.** The command line is read first, for every
subcommand, known or not, exactly as the Rust command reads it -- a second
operand where one is taken, a flag the subcommand lacks, a value missing,
`--` ending it -- so a malformed line is refused in Rust's words whatever
it names. Then:

- no subcommand: the usage on stderr, exit 1; `help`, `--help`, `-h`: the
  usage, exit 0; an unknown one: "error: unknown command `x`", a blank
  line, the usage, exit 1 -- all Rust's, byte for byte;
- `test`, `bench`, `doc`, `api`, `fmt`, `lint`, `deps`, `publish`, `add`,
  `lsp`, `dap` and `journal` -- the subcommands part one does not cover:
  "error: `grasm <cmd>` is not in the Grasm toolchain yet" and a help line
  naming the ones it runs and this section, on stderr, exit 1 (the status
  the Rust command gives a command it does not know);
- `build` without `--native`, and `build --source`: after every check
  above, "error: `grasm build` without `--native` is not in the Grasm
  toolchain yet" (or `--source`'s), with the same help, exit 1.

Never a fallback: nothing is handed to the Rust toolchain or any other
program. The wording and the status were taken by the S8 agent, because the
brief asks for an exact message and none exists in Rust for "not here"; the
other reading -- answering as Rust answers a command it does not know --
would print a usage that lists the very command it just called unknown.

**The gate, `clidiff`** (step three): every part-one subcommand through
both commands from one directory -- the corpus and the conformance
negatives for `check`, `tir`, `tokens`, `ast` and `run`; the manifests for
`toml`; argument lines for every subcommand, the unknown and the
not-covered ones included -- with stdout, stderr and the exit status the
same bytes, normalised only in the success line of `build` and the stack
trace `run` may leave out. `build --native` is compared by what its
executables do, as `rtswap` and `linktest` compare, and by building twice
for the same bytes. Counts asserted. Controls: a message reworded, an exit
status changed, a flag misread, a subcommand routed elsewhere.

**Self-hosting the driver** (step four): stage 1 is the driver built by
the Rust `grasm`, with the `rt.o` stage 1's `front` and `s3` make; stage 2
is the driver and `rt.o` built by stage 1's driver alone (`grasm build
compiler/grasm/src/main.gr --native --no-incremental` and the runtime line
above); stage 3 the same by stage 2. A first try, 2026-11-03, before the
gate: stage 2 built at the first attempt, and stage 3 is stage 2 to the
byte, the driver (4,025,208 bytes) and `rt.o` (555,152 bytes) alike; the
`rt.o` stage 1's driver makes is the one stage 1's `front` and `s3` make.
`fixpoint` is extended to it, and `clidiff` run with stage 2's driver.

**What the gates found** (2026-11-03 and 04, this branch on `0fed9ffd`, the
main checkout's compiler, a one-minute load of 2 to 6 from other work):

- `clidiff`, with the command stage 1 is (built by the Rust `grasm`) and,
  run again, with stage 2's (`fixpoint`'s kept stage, built by stage 1's
  command alone), both passing with the same lines:
  "lines: 144   the same: 124   not here, as §6.17 says: 16   the runtime
  built: 1   a target refused: 3"; "corpus: 1737 programs, 8685 commands
  (tokens, ast, check, tir, links)   the same: 8685, 138 of them
  refusals"; "toml: 1473 files   the same: 1473, 19 of them refused by
  both"; "run: 651 programs with a main   the same: 651   timed out: 0";
  "build: 648 programs with a main (3 link a system library)   the same
  output: 648, 0 of them refusals   run alike: 648   built twice the same:
  648"; "target: 65 programs for x86_64-apple-darwin   the same output, an
  x86-64 object made twice the same: 65"; "the Grasm command's package
  calls proc_run or proc_pipeline: 0 time(s)". About 1 hour 40 minutes a
  run. The first run failed on its own floor (200 refusals, where the corpus
  has 138 -- the S8 agent's guess, now 120) with nothing differing; the trial before it
  found four command lines tagged wrongly in the cases file (`build
  --native --source`, `test --cases -1`, two targets Cranelift refuses in
  its own words), each now tagged for what the Grasm command says. No line
  of the corpus met the `IoFailed` wording the engines disagree on
  (docs/known-issues.md).
- `clicontrols`: "controls: 7   caught: 7   missed: 0" -- "takes one file"
  reworded, the usage after an unknown command exiting 2, `--link` read as
  a flag, `check` handed to the Rust `grasm` by `proc_run` (caught by the
  count; and, with the count skipped, by the sandbox, where the routed
  output would otherwise have been the same bytes), the success line naming
  Cranelift, `build` compiling for x86-64.
- `fixpoint`, extended: "programs: 6 and the grasm command   artifacts per
  stage: 16 and 2   compared: 54 ...   identical: 54   differ: 0"; "the
  command's build the stage pipeline's (its rt.o and the 4 programs): 5 of
  5". The command's stage 3 is its stage 2: `grasm` 4,025,208 bytes,
  SHA-256 `8272e454f6c85013b32d9c7ff677e538d197324505b69db6a81d49da1123e483`,
  and `rt.o` 555,152 bytes, the compiler's stage-3 `rt.o` to the byte. Its
  stage 2 took 58.7 s and 550 MB at peak, its stage 3 100.8 s and 577 MB
  (not to be quoted: one run, loads of 2 to 4). `fixpointcontrols`, its
  copies now holding the command's sources: "controls: 6   caught: 6
  missed: 0", 61 minutes.
- Over the packages the command changed (`front`, `s3`, `link`,
  `tirinterp`): `tokentest` 1,993 of 1,993 files identical, `asttest` 1,993
  CSTs printed back, `tirdiff` 1,926 the same TIR and 67 refused by both of
  1,993, `tirrefuse` 582 and 313 of 895, `diagdiff` 1,437 refused alike and
  2,826 accepted, none differing, `s3subset` 648 of 648, `objtest` 648
  objects and 2 runtimes reproducible, `linktest` 651 and 648 and the 3
  fixtures, 2,604 links, `linkdiff` 168 of 168 byte-identical, `rtswap` 648
  on each path with 323 of 323 names from Grasm, `tirinterp` 651 of 651 and
  25 panics, `stagediff` 758 executables the same of stage 1 and stage 2;
  `tomldiff` 1,448 of 1,448 and `frontsmoke` 47 of 47.

**What is left for S8.** Part one covers nine subcommands. The rest -- `test`,
`bench`, `doc`, `api`, `fmt`, `lint`, `deps`, `publish`, `add`, `lsp`,
`dap`, `journal`, `build` without `--native` and `--source` -- with the LSP
on the query engine, DAP, packages, journal and replay, the wasm32 backend,
one TOML reader, and the decisions above that the owner reverses. The
gates that run through the `grasm` command (§6.14's last list) can now
take the Grasm command for the subcommands it has; none was pointed at it
here.

### 6.18 S7 part two: Linux ELF on arm64 and x86-64

Written 2026-11-04, before any code, as the design to build to. S7's second
half is "Linux ELF on x86-64 and arm64 with a system-call layer and an ELF
linker", and its exit "crosstest, and a Linux CI run". Part one (§6.16) left
it waiting on the owner's decision of where Linux binaries run. **The
owner's decision (2026-11-03): in OrbStack on the development Mac**, in the
owner's existing `ubuntu` machine (arm64, Ubuntu 25.10, glibc 2.42, kernel
6.17), used to run and inspect binaries only, with x86-64 binaries run
through OrbStack's Rosetta support. Everything else here was taken by the
agent implementing this part unless it says otherwise, and the owner may
reverse any of it.

**The targets.** Two triples, `aarch64-linux-gnu` and `x86_64-linux-gnu`
(the `-unknown-` spellings Rust and Cranelift use mean the same):
little-endian, LP64, glibc, the platform's System V ABIs -- AAPCS64 and the
System V AMD64 ABI. `s3 --target aarch64-linux` and `s3 --target
x86_64-linux` select them; `aarch64` and `x86_64` keep meaning macOS, so no
command line that works today changes meaning. An operating system is a
property of the target next to its instruction set: MIR stays target-neutral
and TIR stays the same for every target, as part one found for x86-64.

**What the machine is, measured** (2026-11-04, a one-minute load of about 4
on the Mac): Rosetta is registered with `binfmt_misc` for x86-64 ELF and
runs it -- a 156-byte static x86-64 executable written by hand, making two
system calls, printed and exited 0 -- but the machine holds no x86-64 C
library: no `/lib64/ld-linux-x86-64.so.2`, no `/lib/x86_64-linux-gnu`. Its
own toolchain is arm64's: gcc 15.2, GNU ld 2.45 (arm64 emulations only),
readelf and objdump, glibc's `libc.a`, `libm.a` and `crt1.o`; no lld. Its
CPU flags offer no pointer authentication (`paca`, `pacg` absent: the VM
hides it), and its pages are 4 KiB. The Mac's own directories are mounted in
it at the same paths, so a program built on the Mac runs there in place, and
starting a command in it costs about 28 ms. The Mac has LLVM 21.1.8's
`llvm-readelf`, `llvm-objdump`, `llvm-dwarfdump` and `llvm-mc` (Homebrew's
`llvm@21`), which read and disassemble ELF for both instruction sets, and
Rust's standard library for `x86_64-unknown-linux-gnu` but not for
`aarch64-unknown-linux-gnu`.

**Linking: dynamic, against glibc.** A Grasm program on Linux is a
position-independent executable that names `libc.so.6` and `libm.so.6` and
the dynamic loader, with every import bound at load (`BIND_NOW`, read-only
after relocation), and the runtime calls the C library for its system
calls, as the Darwin runtime calls libSystem. Measured, three ways to say
"hi" on arm64 in the machine, gcc 15.2 at `-O2`, 500 runs each through the
shell's fork and exec:

| | size | per run |
|---|---:|---:|
| dynamic, glibc | 70,408 bytes | 324 µs |
| static, glibc's `libc.a` | 777,632 bytes | 285 µs |
| static, no C library (two system calls) | 1,576 bytes | 200 µs |

The dynamic loader costs about a tenth of a millisecond a run; macOS charges
450 ms for a new executable's first run. What decides it is what the runtime
takes from the C library: 95 distinct C functions, of which 24 are libm's
(`sin`, `pow`, `exp`, `log`, ...), and `getaddrinfo` (the resolver, with
`/etc/hosts`, `resolv.conf` and NSS behind it), `strerror` (whose words are
the error texts programs print), the threads (`pthread_create`, `join`, keys
with destructors), `malloc` and its family, and `dlsym` for the C `exit`
that runs `atexit` work. None of these is a system call. Without a C library
the runtime would need a libm -- §4's correctly rounded one, not yet written,
and differing in the last place from glibc's, which the Cranelift build the
corpus is held to uses -- a resolver, threads over `clone` and futexes, and
an allocator over `mmap`; and a program's own `extern fn` and `links`
(11 corpus files declare C functions) would have nothing to call. Static
linking of glibc's archives was the third way: it needs `IRELATIVE`
relocations for the string functions glibc picks per processor and every
thread-local model, glibc warns against static `getaddrinfo` and `dlopen`, and the
machine has no x86-64 archive either. So §1's last line stands as the
finish line -- Linux binaries *can* be fully static, with no C library --
and is reached after the Grasm libm, not in this part. The cost of the
choice is the one the measurement above shows: an x86-64 binary needs an
x86-64 glibc to run in the machine, which it does not have (decision (2)
below).

The glibc a program needs: the linker binds each import to the version
glibc's own `.gnu.version_d` gives as the default (as ld does), and the
oldest glibc that can run the program is the newest of those versions.
`__libc_start_main@@GLIBC_2.34` sets the floor at 2.34 (Ubuntu 22.04,
Debian 12, RHEL 9 and later); a gate prints the floor of every link and
fails above 2.34.

**The start.** No `crt1.o`, `crti.o` or `crtn.o`: those are in glibc's
development package, which a user's machine need not have. The linker writes
`_start` itself, as Go's and Zig's do: clear the frame pointer, pass `main`,
`argc`, `argv`, no initializer and no finalizer (glibc 2.34 and later run the
executable's `.init_array` themselves), the loader's finalizer and the stack's
end to `__libc_start_main`, and trap if it returns. For comparing with ld the
linker writes the same `_start` as an object (`link -start-object`), and ld
is given that object in place of glibc's start files.

**The object** (step two). `compiler/macho`'s object description (`MoObj`:
the code, functions, relocations, data, process cells, thread-local
variables and the line table) is already format-neutral; a new package,
`compiler/elf`, writes it as a relocatable ELF64 (`ET_REL`) for `EM_AARCH64`
or `EM_X86_64`, as clang writes one with `-ffunction-sections
-fdata-sections`:

- a section per function (`.text.<name>`), per constant (`.rodata.<name>`),
  per writable datum (`.data.<name>`) and per process cell (`.bss.<name>`),
  and C strings in `.rodata.str1.1` (`SHF_MERGE | SHF_STRINGS`), so that
  `--gc-sections` removes what Mach-O's dead-stripping removes, function by
  function -- the Cranelift build's objects have one `.text`, as now;
- the runtime's thread-local variables in `.tbss.<name>` (`SHF_TLS`,
  `STT_TLS` symbols), with no descriptors: ELF needs none;
- `.rela.*` for each section that has relocations (ELF on both instruction
  sets uses explicit addends); `.note.GNU-stack`, empty, so the stack is not
  executable; `.symtab` with the locals first; `.strtab`, `.shstrtab`; and
  the line table's four sections (`.debug_abbrev`, `.debug_info`,
  `.debug_str`, `.debug_line`), DWARF 4 as on Mach-O, but with the
  relocations ELF needs and Mach-O does not: each function's sequence starts
  at its own section's address, and each offset from one debug section into
  another is relocated (ld concatenates them across objects, where Mach-O's
  debug map leaves them in place);
- symbols without Mach-O's leading underscore; nothing that depends on time,
  paths other than the compile directory, or hash order.

Relocations, by what the backend asks for:

| use | aarch64 | x86-64 |
|---|---|---|
| a call | `R_AARCH64_CALL26` | `R_X86_64_PLT32`, -4 |
| a datum's address | `ADR_PREL_PG_HI21` + `ADD_ABS_LO12_NC` | `R_X86_64_PC32`, -4 (`leaq`) |
| a load through the GOT | `ADR_GOT_PAGE` + `LD64_GOT_LO12_NC` | `R_X86_64_REX_GOTPCRELX`, -4 |
| a pointer in data | `R_AARCH64_ABS64` | `R_X86_64_64` |
| a thread-local variable | `TLSLE_ADD_TPREL_HI12` + `_LO12_NC` | `R_X86_64_TPOFF32` |
| DWARF: an address, an offset | `ABS64`, `ABS32` | `R_X86_64_64`, `R_X86_64_32` |

`objtest_elf` holds every object of the subset, for both instruction sets,
to the system's readers: `readelf -a -W` in the machine (binutils 2.45) and
`llvm-readelf`, `llvm-objdump -d -r` and `llvm-dwarfdump --verify` on the
Mac; every relocation against the instruction it patches (a script, as
`s3_macho.py` and `x64_macho.py` do for Mach-O); the line table's rows
against the literals they load, as `objtest` counts them; GNU ld accepting
every arm64 object, and the cross ld every x86-64 one (decision (2)); and each object compiled twice the same bytes. Its controls, in a new
`elfcontrols`: a relocation's addend off by four, a section's flags without
`SHF_ALLOC`, a symbol's binding local where it must be global, the locals
not first, an offset into `.debug_abbrev` left unrelocated, the clock in an
object.

**The backend** (step three). Selection and allocation are the same for both
operating systems; what differs is at the edges:

- *Names.* No leading underscore; `main` is `main`.
- *Variadic C functions.* Linux's AAPCS64 passes a variadic argument as a
  fixed one, in the next register -- Apple's arm64 puts it on the stack --
  so `open`'s mode and `fcntl`'s argument go as on x86-64: the per-target
  table of part one's decision (1) applies to both Linux targets, and the
  runtime's declarations do not change.
- *Thread-local variables.* The runtime is linked into the executable, so
  the local-exec model reaches its variables with no call: on arm64 `mrs x0,
  tpidr_el0` and two `add`s with the `TLSLE` relocations, on x86-64 `movq
  %fs:0, %rax` and `leaq v@tpoff(%rax), %rax`. These are the getters'
  bodies (`leaves.gr`); the context switch is the same leaf on both
  operating systems.
- *Pointer authentication on arm64.* Darwin's frames sign the return address
  with `pacibsp` and return with `retab`. `retab` is not in the hint space:
  on a processor without pointer authentication -- ARMv8.0 to 8.2, the
  Graviton2 and Cortex-A76 among them -- it is undefined and the program
  dies of SIGILL. (The machine runs it, measured: the processor has
  authentication though the VM's flags hide it.) Linux arm64 frames sign
  with `paciasp` and return with `autiasp` then `ret`, both hints: a no-op
  where there is no authentication, a check where the kernel enables it --
  what gcc's `-mbranch-protection=pac-ret` emits, with gcc's key (A). No
  branch-target identification: it guards a page only when every object in
  the executable is marked for it, and neither the Cranelift build's
  objects nor the Rust runtime's are; it is left until every input is this
  backend's. Darwin's frames do not change.
- *Darwin x86-64's `$INODE64`* is Darwin's alone; the per-target table of C
  names gains Linux's one rename, `__error` to `__errno_location` (the same
  signature: the address of the calling thread's error number).

Darwin's output must not change: every object of the subset and the
runtime's, from the same TIR in the same directory, the same bytes from the
branch's driver as from master's, for both Darwin targets, as part one
checked; and `fixpoint`.

**The runtime** (step three). `compiler/rt` assumes Darwin in constants and
in a few functions, found by reading every `extern fn` and every constant:

| what | Darwin | Linux (glibc, both instruction sets) |
|---|---|---|
| error number's address | `__error` | `__errno_location` |
| `EAGAIN`, `EILSEQ` | 35, 92 | 11, 84 |
| `O_APPEND`, `O_CREAT`, `O_TRUNC`, `O_NONBLOCK` | 8, 512, 1024, 4 | 1024, 64, 512, 2048 |
| `CLOCK_MONOTONIC` (also a literal 6 in `par.gr`) | 6 | 1 |
| `_SC_NPROCESSORS_ONLN` | 58 | 84 |
| `mmap`'s private anonymous flags (a literal in `fiber.gr`) | 4098 | 34 |
| `dlsym`'s `RTLD_DEFAULT` | -2 | 0 |
| a `dirent`'s name | at 21 | at 19 |
| `SOL_SOCKET`, `SO_REUSEADDR`, `AF_INET6` | 65535, 4, 30 | 1, 2, 10 |
| a `sockaddr`'s family | a byte at 1, after a length byte | 16 bits at 0 |
| `addrinfo`'s `ai_canonname`, `ai_addr` | at 24, 32 | at 32, 24 |
| `poll`'s count | 32 bits | 64 bits (`nfds_t`) |
| `pthread_setname_np` | the name | the thread and the name |
| `net_wait` | a `kqueue` per thread, else `poll` | `poll` (as the Rust runtime on Linux) |
| `open`, `fcntl`, variadic | ninth argument, on the stack | in the next register (the backend's table) |

The rest -- the other error numbers, `open`'s read and write flags, `fcntl`'s
commands, `poll`'s events, signals, the `pthread` calls' arguments, `getentropy`
-- is the same, and a gate checks each constant against the C library
(`elfrtlayout`: a C probe compiled in the machine prints each one, as
`rtlayout` probes the Rust runtime's layouts). Each difference is also what
the Rust runtime does under its `cfg(not(target_os = "macos"))` branches,
which is the oracle's behaviour.

How the runtime selects: the language is frozen, has no conditional
compilation, and the Rust frontend, which makes the runtime's TIR in stage
0, does not change. Two ways were measured on the runtime's object,
2026-11-04: moving a literal constant (`O_CREAT`) from `fsys.gr` into
another module left every instruction the same -- a literal is that literal
where it is read (§6.12) -- and changed only the line-table rows of the
loads it moved (599 bytes of `__debug_line`); putting one function's body
under `if build.LINUX { ... } else { ... }` with `LINUX` a literal `false`,
as `build.POISON` selects the poisoned build, changed register allocation in
the code around it. So the selection is by module, not by branch:
`compiler/rt/src/os.gr` holds every constant and every function in the table
above, for Darwin, and `compiler/rt/os/linux.gr` is the same module for
Linux, with the same public names. The Linux runtime is the package with
`src/os.gr` replaced by it: the gates build its TIR from a copy so replaced
(as `rtswap.sh` builds the poisoned runtime from a copy with `build.gr`
changed), and the Grasm `grasm` command's loader takes the replacement when
it builds the runtime for a Linux triple. Darwin's runtime object then
differs from master's only where the code moved into `os.gr` (its line
rows, and the order of the functions that moved), each difference listed
when step three is measured, and its behaviour is held by `rtswap`,
`rtcontrols` and the family fuzzers as before. A control that plants a fault
into code that moves is moved with it, in the same commit.

`s3 --runtime --target aarch64-linux` and `--target x86_64-linux` make the
two Linux runtimes, held by `elfrtswap` to the Rust runtime built for the
same triple.

**The linker** (step four). `compiler/link` reads the objects' format and
links ELF in modules of its own beside the Mach-O ones, sharing the archive
reader, symbol resolution and the reachability walk; an ELF link has
sections where Mach-O has atoms, and dead code goes by section
(`--gc-sections`), as GNU ld removes it. It writes what GNU ld 2.45 writes
for `-pie -z now -z relro --hash-style=gnu --gc-sections --build-id=none
-nostdlib` and the same inputs, read off ld's output as the Mach-O half was
read off ld-prime's: the program headers (`PT_PHDR`, `PT_INTERP`, the
`PT_LOAD`s with ld's pages -- on x86-64 the code in a segment of its own,
as ld's `-z separate-code` default is there, and each instruction set's
maximum page size as ld's defaults set it, measured at step four),
`PT_DYNAMIC`, `PT_TLS`, `PT_GNU_RELRO`, `PT_GNU_STACK`
not executable; `.interp` (`/lib/ld-linux-aarch64.so.1`,
`/lib64/ld-linux-x86-64.so.2`); `.dynsym`, `.dynstr`, `.gnu.hash`,
`.gnu.version` and `.gnu.version_r`; `.rela.dyn` (the `RELATIVE`
relocations, counted first) and `.rela.plt` (`JUMP_SLOT`), the PLT and the
GOT; `.init_array`, which the Rust runtime's archive may carry; a
thread-local segment laid out as each architecture's variant puts it
relative to the thread pointer; and `.eh_frame` with `.eh_frame_hdr` when
an input has unwind tables (the Rust runtime does; this backend writes none,
§6.16). The libraries are read as ld reads them: `libc.so.6` and `libm.so.6`
from a system root (`-sysroot`, the machine's own root through OrbStack's
mount on the Mac), their dynamic symbols and version definitions. The
output's sections are stripped of symbols for `grasm build --native`, as ld's
`-s` leaves them.

`linkdiff_elf` links every fourth program of the subset, for each
instruction set, from the Grasm backend's objects and from the Cranelift
build's, with the Grasm runtime and with the Rust runtime for the triple, and
requires the bytes to be ld's, or every difference named, explained and
counted by the gate; then `readelf -a -W`'s full report, `objdump -d`'s
disassembly and every section's bytes against ld's link of the same inputs.
`elfcontrols` plants the linker's faults: a `CALL26` one instruction off, a
`JUMP_SLOT` bound to the wrong import, a version requirement naming the wrong
version, a `RELATIVE` relocation missing, the TLS offset of the wrong
variant, `PT_GNU_RELRO` covering the `.got.plt`.

**The oracle, crosstest's.** `crosstest` holds the Cranelift build for a
target to the interpreter. For Linux its reference is the Cranelift build
for the triple -- the object `grasm build --native --target <triple>`
already emits (Rust's command emits an object, not an executable, for a
cross target) -- linked with the Rust runtime built for the triple by the
machine's own toolchain (gcc for arm64; an x86-64 ld for x86-64, decision
(2)), and run in the machine. The Grasm backend's program, linked by
`compiler/link` with the Grasm runtime for the triple, must print, exit and
write to stderr exactly as that reference does, and as the interpreter does
on the Mac -- where the two references disagree (glibc's libm and Apple's
differ in the last place on some arguments, as the demos found for arm64
and x86-64 macOS; glibc's `strerror` words some errors differently), the
gate names the program and the reason and counts it apart. That is
`elfsubset`, run for both triples. The other corpus gates for Linux are
`elfrtswap` (above), `elfrepro` (every link and object twice, the same
bytes) and `objtest_elf`.

**The `grasm` command in Grasm** (step five). `build --native --target`
accepts `aarch64-linux-gnu`, `x86_64-linux-gnu` and their `-unknown-`
spellings and writes the object `s3 --target <arch>-linux` writes, under
the Rust command's name and message, as it does for x86-64 macOS (§6.17);
`build compiler/rt/src/main.gr --native --target <linux>` builds that
runtime's object. Linking a Linux executable from the Mac is the linker's
(`link -sysroot`); a `grasm` command that runs on Linux, finding itself
through `/proc/self/exe` where it now asks `_NSGetExecutablePath`, is S8's.
`clidiff` gains the Linux triples' lines; `clicontrols` a control that
refuses them.

**Gates that need the machine** skip when OrbStack or the machine is not
there: they say why on their last line and exit with 77 (automake's "skipped"),
and `allgates.sh` prints such a gate as `NOT RUN`, counts it apart from the
passed and the failed, and its last line says how many did not run -- never
"every gate passed" while one did not. S7's exit asks for "a Linux CI
run"; here that is these gates, run in the machine by `allgates.sh` on the
Mac. A hosted CI service is not part of this part, and nothing is pushed.

**Decisions** (raised with this note, 2026-11-04). (2) and (3) are the
owner's, taken 2026-11-04. The others were taken by the agent implementing
this part and accepted by the coordinating session the same day; the owner
may reverse any of them.

1. Dynamic linking against glibc, the runtime calling the C library, a
   position-independent executable bound at load, glibc 2.34 or later,
   measured above. Static linking with no C library needs the Grasm libm
   first.
2. *The owner's:* x86-64 programs get their C library, and the reference
   and `linkdiff_elf` their x86-64 ld, from Ubuntu's arm64 cross packages
   -- `libc6-amd64-cross`, `libc6-dev-amd64-cross`,
   `linux-libc-dev-amd64-cross`, `libgcc-s1-amd64-cross` (the unwinder the
   Rust runtime's archive names) and `binutils-x86-64-linux-gnu`, about
   7.5 MB to download -- fetched with `apt-get download` and unpacked with
   `dpkg-deb -x` into the scratch directory on the Mac. Nothing is installed
   in the machine; an x86-64 program is started through the cross loader,
   `<root>/lib64/ld-linux-x86-64.so.2 --library-path <root>/lib ./prog`.
   (Installing them with apt, or multiarch `libc6:amd64`, were the other
   ways offered.)
3. *The owner's:* Rust's standard library for `aarch64-unknown-linux-gnu`
   is added on the Mac (`rustup target add`, about 150 MB), so that the
   arm64 reference and `elfrtswap` link the Rust runtime built for that
   triple, as x86-64's do.
4. Pointer authentication on Linux arm64 by hints (`paciasp`, `autiasp`,
   `ret`), no branch-target identification.
5. The runtime selects its operating system by module (`os.gr` and
   `os/linux.gr`), not by branch, measured above.
6. A section per function and per datum, so `--gc-sections` is
   dead-stripping.
7. The linker writes `_start`; no C start files.
8. Everything is built and linked on the Mac and run in the machine from the
   Mac's scratch directory through OrbStack's mount: nothing is written to
   the machine's own filesystem (within the owner's decision of where Linux
   runs).
9. A gate that needs the machine and cannot reach it exits 77, and
   `allgates.sh` counts it as not run.

**Step two: the object** (2026-11-04). `compiler/elf` writes the backend's
object description as a relocatable ELF64, as above, and `s3 --target
aarch64-linux` or `--target x86_64-linux` selects it. `compiler/macho/src/
dwarf.gr` gained `sections_elf`, which shares its line program with the
Mach-O writer (`line_table`), so that Mach-O's debug sections stay the bytes
they were. The writer maps every relocation kind the backend has; of
these, the low-twelve-bits types for loads and the GOT pair on arm64, and
`GOTPCREL` without a REX prefix on x86-64, are in no object of the corpus or
the runtime: every page-offset relocation the arm64 backend writes is on an
`add`, and nothing in the subset takes a C function's address through the
GOT. `scripts/elf_obj.py` reads the objects as `s3_macho.py` reads Mach-O:
the layout, every relocation against the instruction it patches (x86-64's
instruction starts from `llvm-objdump -d`), the struct descriptors' pointers,
every DWARF address a function's and every offset into another debug section
relocated (decoded from the abbreviations), no `retab` or `pacibsp` in a
Linux arm64 object, and the line table decoded with its relocations -- every
function's sequence starts at its own section's address 0, so a tool's
listing cannot say which function a row is in.

**Step three: the backend and the runtime** (2026-11-04). `s3 --target
aarch64-linux`, `x86_64-linux`, and `--runtime` with either:

- the arm64 frame signs with `paciasp` and returns with `autiasp; ret`
  (two encoder forms, `mrs xd, tpidr_el0` a third, each held to clang by
  `encodetest`); x86-64's frame is macOS's;
- the runtime's thread-local getters are the local-exec sequences above
  (x86-64 gained `movq %fs:0` and `leaq disp32(%r)`, held to the assembler by
  `encodetest_x64`), relocated `R_AARCH64_TLSLE_ADD_TPREL_HI12`/`_LO12_NC`
  and `R_X86_64_TPOFF32`;
- `open` and `fcntl` go through the variadic table on arm64 Linux too
  (`compiler/aarch64/src/isel.gr`, empty on macOS), and `__error` is
  `__errno_location`.

`compiler/rt/src/os.gr` holds Darwin's numbers and its three calls that
differ (`kq_new`, `kq_event`, `name_thread`); `compiler/rt/os/linux.gr` is
Linux's. Two more changes to the shared source: `pthread_key_create`'s slot
is zeroed (a key is 64 bits on macOS and 32 on Linux, and is read as a word),
and `par.gr`'s literal 6 and `fiber.gr`'s 4098 are `os` constants. Darwin's
runtime object from the branch's source is master's but for three
instructions in 53,693: the zeroing's `mov x1, #1`, and two `mov xN, #0`
two places later, where the `kevent` call now goes through `os.kq_event`
and is inlined with its arguments made in another order; with the line rows
of the moved constants. `linux_rt_build` (`scripts/lib/linux.sh`) builds the
Linux runtime from a copy of the package with `os/linux.gr` in place.

Until the linker links ELF (step four), the Linux gates link the Grasm
backend's programs with GNU ld 2.45: `compiler/link`'s `_start` (`link
-start-object`), the program, the Grasm runtime, glibc's `libm.so.6` and
`libc.so.6`, `-pie -z now -z relro --hash-style=gnu --gc-sections
--build-id=none`, and a `--defsym` binding each `grasm_<name>` to the
runtime's `grt_<name>`, as `link -interpose` binds them. Every program runs in
the machine with a /tmp of its own (`scripts/lib/isolate.py`: a user and
mount namespace mapping the machine's user to itself, the gate's directory
mounted on /tmp): fifteen of the subset's programs name paths under /tmp,
and the machine's /tmp is the owner's. The first full run of `elfsubset`,
before that, left 48 files and directories of those programs in the
machine's /tmp, and ran two programs' copies at once on one directory:
`packages/lsm-demo` differed on both triples for that reason alone, and
agreed once each run had its own /tmp.

What the Linux runs found: `spec/conformance/T-EXTERN-POINTER.gr` declares
`memcmp`, whose C result is an `int`, as returning `i64`, and so reads the
half of the register C leaves unspecified -- glibc leaves it zero under a
negative result, Apple's libSystem sign-extends -- so on Linux both builds
print `memcmp orders: false true` where the interpreter on the Mac prints
`true true`. It is the program's assumption, not a backend's: it is listed
in `scripts/elfsubset_known.txt` for both triples.

**The gates** (2026-11-04, the branch's final tree, a one-minute load on the
Mac of 3 to 5 from other work, 15 to 18 GB of swap in use of 16 to 19.5 GB, the
disk 8.6 to 12 GB free):

- `objtest_elf`: "aarch64-linux: objects: 648   reproducible:
  648   linked by ld: 648   relocations: 3322951   literal loads on their
  lines: 137906 (of literals loaded more than once: 31366)", "the runtime:
  built twice the same, read, relocated and linked (ld -r) without a
  problem: 1 of 1   relocations: 6386"; "x86_64-linux: objects: 648
  reproducible: 648   linked by ld: 648   relocations: 3083326   literal
  loads on their lines: 68953 (of literals loaded more than once: 15683)",
  the runtime 1 of 1, 5355 relocations; "with problems: 0" (LLVM 21.1.8's
  readers, GNU readelf and ld 2.45). The literal loads are the Mach-O
  gates' counts to the literal. 25 minutes.
- `elfsubset`: "aarch64-linux: subset: 648 in, 0 out, 3 external, of 651
  programs with a main", "identical: 647 mismatched: 0 timed out: 0
  agreeing with the Cranelift build and not the interpreter, as listed: 1",
  and the same for x86_64-linux; GNU ld 2.45, glibc 2.42. 36 minutes.
- `elfrtswap`: "the Grasm runtime for aarch64-linux x86_64-linux: 345
  names moved, exported as families.txt lists; it calls none of the Rust
  runtime's"; for each triple "programs with a main: 648   external set
  aside: 3", "Cranelift objects: identical: 648   poisoned runtime, agreeing
  with no use after free: 648", "Grasm backend objects: in the subset: 648
  identical: 648", "runtime names the programs reference: 342 distinct, all
  taken from Grasm; per program, 221616 references bound to Grasm" (a
  Cranelift ELF object names every runtime function it may call, used or
  not, so these count names, not the bindings `-interpose_log` counts on
  macOS). 20 minutes.
- `elfcontrols`: "controls: 17   caught: 17   missed: 0" -- the object's ten
  (an addend off by four, a function's section without SHF_ALLOC, `main`
  local, the locals' boundary one off, an `add` relocated as a load, the TLS
  high half relocated as the low, `.debug_abbrev`'s offset unrelocated, the
  line sequences four bytes late, no `.note.GNU-stack`, the clock in the
  object; each by `objtest_elf`) and step three's seven: `retab` in a Linux
  frame (`objtest_elf`: this machine's processor runs it, so no run shows
  it), `__error` kept (no link), the TLS getter from 0, the variadic table
  off on arm64 (`fcntl` reads 0, no descriptor is closed on `exec`, and
  `L-LIB-PROC` runs past the limit), Darwin's `O_CREAT` (each by
  `elfsubset`), Darwin's `dirent` and `SOL_SOCKET` (by `elfrtswap`, the
  Cranelift build with each runtime). The first `elf-ldst`, a load's
  relocation scaled wrongly, was vacuous -- no object relocates a load's low
  bits -- and was replaced by an `add` relocated as a load; the first
  `elf-variadic` aimed at `open`'s mode, which no gate here can see: the
  programs' /tmp is on the Mac's disk, and OrbStack's mount of it does not
  hold a file's owner to the file's mode.
- Darwin unchanged: from the same TIR in the same directory, master's
  driver and the branch's made the same bytes for all 1,302 objects of the
  corpus both compile for `aarch64` and `x86_64` (651 programs) and for the
  runtime on both; `encodetest` 115 forms, 7,025 encodings as clang's
  (`paciasp`, `autiasp`, `mrs-tpidr` new); `encodetest_x64` 104 forms, 5,638
  as the assembler's (`load-fs0`, `lea-tpoff` new); `rtswap` on macOS with
  the branch's runtime "Cranelift objects: identical: 648 failed: 0", "323
  distinct, 323 taken from Grasm", "poisoned runtime: agreeing with no use
  after free: 648", "Grasm backend objects: in the subset: 648 identical:
  648", as before; `fixpoint` "compared: 54 ... identical: 54 differ: 0",
  "stage 2's TIR the same as grasm tir's: 6 of 6" (the new `compiler/elf`,
  `os.gr` and `elfstart.gr` among what stage 1's frontend read), stage 3's
  `rt.o` 555,224 bytes; `fmttest`, `linttest` and `gatecheck` pass.

Not run on this tree: the rest of the suite -- among them `tokentest`,
`asttest`, `tirdiff` and `diagdiff` over the new files (`os/linux.gr`, which
no stage reads, among them), `clidiff` (the `grasm` command's one change is
passing `false` for Linux), `linktest` and `linkdiff` (the linker's one
change is `-start-object`), `xrtswap` (the x86-64 macOS runtime, whose source
changed as the arm64 one did), `rtcontrols`, `s3controls`, `x64controls` and
the fuzzers.

**Step four: the linker** (2026-11-04). `compiler/link` links ELF when its
first object is ELF: `elfmain.gr` reads the command line (`-o`, `-e`,
`-dead_strip`, `-strip`, `-sysroot`, `-L`, `-l`, `-nostdlib`, `-nostart`,
`-interpose`), `elfin.gr` the inputs (relocatable objects, GNU archives,
shared libraries' dynamic symbols, versions, `DT_SONAME` and `DT_NEEDED`),
`elfresolve.gr` resolves symbols in command-line order (archive members
loaded pass after pass, `-interpose`'s binding of each undefined
`grasm_<name>` to the runtime's `grt_<name>`, imports bound to the default
version glibc's `.gnu.version_d` gives) and marks what `--gc-sections`
keeps, `elfmerge.gr` merges string sections as ld does (`bfd/merge.c`:
first-met order, tails merged), and `elflink.gr` lays out and writes the
executable. The inputs are its own `_start` (`elfstart.gr`), the
command line's, then `libm.so.6` and `libc.so.6` from the system root
(`-sysroot`): for arm64 the machine's own root, read on the Mac through
OrbStack's mount of it (`~/OrbStack/<machine>`, `GRASM_A64_ROOT` in the
gates); for x86-64 `GRASM_X64_ROOT`.

For every program of the subset, from either backend's object, with the
Grasm runtime, its output is GNU ld 2.45's byte for byte, ld given `-pie -z
now -z relro --hash-style=gnu --gc-sections --build-id=none -s` and the
same files. That took ld's own algorithms, read from its source, where a simpler rule would
have made a working executable with other bytes:

- the dynamic symbols' order is the order of ld's symbol hash table
  (`bfd_hash_hash` of `name@@VERSION`, the table's size grown from 4051 as
  ld grows it from the count of names it has entered: each object's globals,
  each `--defsym`, each library's versioned spellings), later-entered first
  within a bucket; the PLT's and GOT's order is that traversal's;
- `.gnu.version_r` lists each library's versions newest-first as ld
  prepends them, `.dynstr` the names in binding order, then the libraries'
  and the versions', with ld's suffix merging (`elf-strtab.c`);
- `.dynamic`'s tags in ld's order, with its five spare entries
  (`DT_RELACOUNT` in the first);
- the RELRO layout of `lang_size_relro_segment_1` (the segment ends on a
  maximum-page boundary: 64 KiB on arm64, 4 KiB on x86-64), x86-64's
  separate-code pages for text and read-only data, its empty `.eh_frame`
  when there is a PLT, its multi-byte nops between sections;
- a merged section keeps `SHF_MERGE` only when every input had the same
  flags and entity size (`ldlang.c`); a section symbol's addend into one is
  mapped through the merge;
- on arm64 a section symbol for `.text` and one for `.data` (or the first
  writable section) in `.dynsym` when `.rela.dyn` is not empty, as
  `elfNN_aarch64_size_dynamic_sections` adds them;
- the GOT's local slots per object in symbol order, then the globals' in
  traversal order, their `RELATIVE` relocations first and counted; on
  x86-64, `GOTPCREL` loads of a symbol defined here relaxed to `lea`, and an
  import both called and loaded called through `.plt.got` (ld's non-lazy
  PLT) and hashed in `.gnu.hash`;
- the thread-local offsets of each architecture's variant: arm64's
  `TPREL` from the thread pointer past its 16-byte block (aligned to the
  segment), x86-64's `TPOFF` below it.

Relocations taken: arm64's `ABS64` (a `RELATIVE` in loaded data), `ABS32`,
`PREL32`, the page and low-twelve-bit pairs (`ADR_PREL_PG_HI21` with `ADD`
and every load size, `ADR_GOT_PAGE` with `LD64_GOT_LO12_NC`), `CALL26` and
`JUMP26` (through the PLT for an import), `TLSLE_ADD_TPREL_HI12` and
`_LO12_NC`; x86-64's `64`, `32`, `32S`, `PC32`, `PLT32`, the `GOTPCREL`
family and `TPOFF32`. Any other is refused by name, as is a 64-bit pointer
to an imported symbol (a copy relocation's case) and a branch out of range.

What it does not link yet, and refuses rather than write wrong: an
executable with a symbol table or debug sections (`-strip` is required: ld's
unstripped `.symtab` order and the debug sections' relocations, tombstones
for discarded code included, are left); unwind tables (`.eh_frame`,
`.gcc_except_table`) and section groups (COMDAT) -- which is the Rust
runtime's archive, so the Cranelift build's reference links stay gcc's and
the cross ld's; and exported symbols (`-exported_symbol`). Neither backend
writes unwind tables or groups, and `grasm build --native` strips.

**The glibc floor, measured.** The floor above is wrong for 16 of the 648
programs: glibc 2.42's libm gives `fmod` a default version of 2.38 and
`hypot` one of 2.35, and ld binds a call to the default. The Grasm runtime's
float `%` calls `fmod`, so a program that takes a float's remainder (13 of
them) needs glibc 2.38; one that calls `hypot` and not `fmod` (3; 5 call
`hypot`) needs 2.35; the runtime and every other program need 2.34. The
reference, gcc's link of the Cranelift object with the Rust runtime, needs
2.35 for `hypot` as well, and 2.34 for the others: its `%` does not call
`fmod` (checked on `packages/numeric2-demo` and `packages/timezone-demo`,
arm64). `scripts/glibc_floor.txt` lists the two functions with their
versions and why, and the floor check (`linux_glibc_floor` in
`scripts/lib/linux.sh`, run on every link `linkdiff_elf` and `elfsubset`
make) fails on any import above 2.34 not listed with its version, and, on a
whole `elfsubset` run, on a listed function no link needed. The effective
floor is therefore 2.38 for a program that uses a float `%`.

**Step five: the `grasm` command** (2026-11-04). `build --native --target`
takes `aarch64-linux-gnu`, `x86_64-linux-gnu` and their `-unknown-`
spellings and writes the object `s3 --target <arch>-linux` writes, under the
Rust command's message; any other Linux triple (musl, no ABI) is refused in
the Grasm command's words, as other targets are. A runtime package built for
a Linux triple is refused: its Linux object is compiled from a copy with
`os/linux.gr` as `src/os.gr` (`linux_rt_build`), which the loader does not
make. `clidiff`'s lines gain the four spellings (each the Rust command's
output) and the two refusals, and its `target` section builds every tenth
program for `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` as
well as `x86_64-apple-darwin`, each object checked for its format and
machine and made twice the same; `clicontrols` gains `linux-refused` (the
Linux triples refused as before) and `linux-macho` (a Linux target given a
Mach-O object).

**Decisions in steps four and five** (2026-11-04), taken by the agent
implementing this part and accepted by the coordinating session on
2026-11-05, with the refusal of a runtime package's Linux object (step
five); the owner may reverse any of them:

10. The floor is 2.34 but for the functions `scripts/glibc_floor.txt`
    lists (`fmod`, 2.38; `hypot`, 2.35), bound as ld binds them. Keeping
    2.34 for every program would take binding those two to their older
    versions -- the backend naming the import `fmod@GLIBC_2.17` (x86-64:
    `@GLIBC_2.2.5`) as a `.symver` does, which ld honours and this linker
    would have to -- or a float remainder that does not call `fmod`, or
    linking against an older glibc. The coordinator ruled out binding the
    old versions, which would bring back glibc's error handling for `fmod`
    from before 2.38.
11. Only a stripped executable is written (`-strip` required; `grasm build
    --native` strips): ld's unstripped symbol table order and the debug
    sections' relocations, tombstones for discarded code among them, are
    left.
12. The Rust runtime's archive is refused (unwind tables, section groups),
    so the references stay linked by gcc and the cross ld; `linkdiff_elf`
    checks the refusal, where §6.18's plan had it link that archive too.
13. `-exported_symbol` is refused for ELF; no program here exports one on
    Linux.
14. `linkdiff_elf` holds the linker to ld's bytes; equal bytes make the
    plan's `readelf -a`, `objdump -d` and per-section comparisons the same,
    so they run only on a difference (`scripts/elfcmp.py` names it part by
    part).
15. The arm64 C library is the machine's own, read on the Mac through
    OrbStack's mount of its root, so the linker and ld read the same files;
    a gate that cannot read it does not run (exit 77).
16. The Linux gates link the Grasm backend's programs with `-strip`; what a
    program prints does not depend on its symbol table.

**The gates, steps four and five** (2026-11-04 and 5, on the branch's tree
as committed -- `linkdiff_elf`'s second link of each and the comments of
`scripts/glibc_floor.txt` came after `clidiff` and the regression gates ran,
which read neither; a one-minute load on the Mac of 3 to 5 from other work,
15.5 to 18 GB of swap in use until it fell to 5.5 GB near midnight, the disk
6.6 to 22 GB free):
- `linkdiff_elf`: for each triple "programs: 162 (every fourth of the
  subset) linked by compiler/link: 324 twice the same: 324 GNU ld's bytes:
  324 different: 0 newest glibc required: 2.38 (floor 2.34; above it, as
  listed: fmod 2.38 in 8 links, hypot 2.35 in 4 links) refused as not linked
  yet: 2 of 2", against GNU ld 2.45 on arm64 and the x86-64 cross ld 2.45. 6
  minutes. Before it, a run over every program of the subset, both triples,
  from each backend's objects (1,296 links each), found one link not ld's:
  the Grasm backend's x86-64 object of `spec/conformance/L-LIB-CALL-C.gr`,
  whose `strlen` is both called and loaded -- such an import belongs in
  `.plt.got` and `.gnu.hash`, and `.rela.dyn`'s GOT relocations go by place.
  With that fixed, all 2,592, run again, were ld's bytes (a run of a scratch
  harness, not a gate).
- `elfsubset`, linked by `compiler/link`: for each triple "subset: 648 in, 0
  out, 3 external, of 651 programs with a main", "identical: 647 mismatched:
  0 timed out: 0 agreeing with the Cranelift build and not the interpreter,
  as listed: 1", "linked by compiler/link; the newest glibc a link requires:
  2.38 (floor 2.34; above it, as listed: fmod 2.38 in 13 links, hypot 2.35
  in 5 links)"; the references by GNU ld 2.45, run on glibc 2.42. 36
  minutes. Its first run with the switch, before the list, agreed on every
  program as before and failed the floor on those 16.
- `elfrtswap`, linked by `compiler/link`: for each triple "programs with a
  main: 648 external set aside: 3", "Cranelift objects: identical: 648
  poisoned runtime, agreeing with no use after free: 648", "Grasm backend
  objects: in the subset: 648 identical: 648", "runtime names the programs
  reference: 342 distinct, all taken from Grasm". 20 minutes.
- `elfcontrols`: "controls: 24 caught: 24 missed: 0" -- step two's and
  three's seventeen as before (`elf-errno` caught now by `compiler/link`'s
  "undefined reference to"), the linker's six, each by `linkdiff_elf`
  against ld's bytes (`link-call26` and `link-tls` in `.text`, `link-slot`
  in `.rela.plt`, `link-version` in `.gnu.version`, `link-relative` in
  `.rela.dyn`, `link-relro` in `PT_GNU_RELRO`), and `floor-list`. The first
  `link-relative` was vacuous on `hello`, whose link has no pointer in
  loaded data; it runs on `examples/ranges.gr`. 3 minutes.
- `clidiff`: "lines: 149 the same: 128 not here, as §6.17 says: 16 the
  runtime built: 1 a target refused: 4", "corpus: 1737 programs, 8685
  commands ... the same: 8685, 138 of them refusals", "toml: 1474 files the
  same: 1474", "run: 651 programs with a main the same: 651", "build: 648
  programs with a main (3 link a system library) the same output: 648 ...
  run alike: 648 built twice the same: 648", "target: 65 programs, each for
  x86_64-apple-darwin, x86_64-unknown-linux-gnu and
  aarch64-unknown-linux-gnu the same output, the triple's object made twice
  the same: 195 of 195"; no `proc_run`. 101 minutes.
- `clicontrols`: "controls: 9 caught: 9 missed: 0", `linux-refused` and
  `linux-macho` among them. 26 minutes.
- The regression gates step three left unrun, one at a time under `nice`:
  `tokentest` "files: 2003 identical: 2003 mismatched: 0"; `asttest` "files:
  2003 AST identical: 2003 AST mismatched: 0", "CST printed back: 2003 not:
  0"; `tirdiff` "files: 2003 TIR identical: 1936 both refuse: 67 disagree: 0
  holes: 0", "emitted twice: 193 unstable: 0"; `diagdiff` "programs: 4273
  ... refused with the same output: 1437 ... accepted by both: 2836 differ:
  0 Rust panics: 0"; `linktest` "Cranelift objects: identical: 651 failed:
  0", "Grasm backend objects: in the subset: 648 identical: 648 failed: 0",
  "C fixtures: identical: 3 of 3", "links by compiler/link (each made twice,
  compared): 2604"; `linkdiff` "sampled: 168 byte-identical to the shipped
  build: 168 tools and sections agree: 168 C fixtures agree: 3 of 3 whole
  runtime (401 members) agrees: 1"; `xrtswap` "programs: 648", "Cranelift
  objects: identical: 648 poisoned runtime, agreeing with no use after free:
  648", "Grasm backend objects: identical: 648 linked twice, the same bytes:
  648", "failed: 0 timed out: 0" (123 minutes); `rtcontrols` "checks of
  planted faults: 80 caught: 80 missed: 0"; `s3controls` "controls: 42
  caught: 42 missed: 0" (97 minutes); `x64controls` "controls: 32 caught: 32
  missed: 0" (53 minutes).
- `fmttest` "formatted: 1997 unformatted: 0", `linttest` "linted: 1998 ...
  broke: 0" and `gatecheck` pass.

Not run on this tree: `objtest_elf` (the ELF writer is step three's,
unchanged), `fixpoint` (the `grasm` command and the linker changed, and
`clidiff` held the command built by the Rust one), `linkcontrols` and
`linkdiff_x64` (the Mach-O linker's source is unchanged; `main.gr` sends an
ELF input to the ELF linker before it reads its arguments as Mach-O's, and
`linktest`, `linkdiff` and `x64controls` ran through it), `rtswap`,
`rtlayout`, `s3subset`, `xs3subset`, the fuzzers and the rest of the suite;
no whole `allgates.sh`. The runs of the gates above that run corpus
programs left a `target/` in the worktree (programs' `target/demo-out`, the
conformance programs' directories, and a poisoned runtime one of
`s3controls`' gates builds into `target/s3rc-poison`), removed after. That
last was `s3rc`, which took a poisoned runtime by `S3_POISON_RT` alone and
built one in the checkout when given `RT_POISON`, the name the other
runtime gates take it by; it now takes either.

**What is left for S7's exit.** The Linux gates -- `objtest_elf`,
`elfsubset`, `elfrtswap`, `linkdiff_elf` and `elfcontrols` -- run in the
machine by a whole `allgates.sh`, S7's "Linux CI run". Of this part's plan:
the floor decision (10) to confirm or reverse; `elfrtlayout` and `elfrepro`
of §8 are not gates of their own (`objtest_elf` makes each object twice,
`linkdiff_elf` each link); and the linker's unstripped output, unwind
tables, section groups and exports (decisions 11 to 13), which the Rust
runtime's archive would need. A runtime package's Linux object from `grasm
build`, and a `grasm` command that runs on Linux, are S8's.

## 8. Gates this adds

`tirdiff` (TIR from both frontends), `tirinterp` (TIR interpreter against
the Rust interpreter), `lexdiff`, `parsediff`, `diagdiff`, `encdiff`
(encodings against `llvm-mc`), `objcheck` (objects through `otool`, `nm`,
`dwarfdump`, `ld`), `linktest` (the corpus linked by our linker runs as
it does linked by `cc`), `linkdiff` (our linker against ld-prime) and
`linkcontrols` (§6.6), `rtswap`
(corpus on the Grasm runtime), `rtlayout` (the layouts both runtimes
share), `fuzz_rtfmt` (number formatting against the Rust runtime) and
`rtcontrols` (§6.7), `rtdialect` and `rtdialectcontrols` (the runtime
dialect as the Grasm backend lowers it, driven from C), `rtlint` (no plain
access to a process cell outside its lock) and `fuzz_rtrelease` (the release
walk against the Rust runtime's) (§6.9), `loaddiff` (the module graph
against the Rust loader's), `tomldiff` (every manifest read by
`packages/toml`, the Rust reader and `tomllib`), `tirrefuse` (what the Rust
frontend refuses, the Grasm one refuses), `fuzz_tir`, `frontsmoke` (front to
binary through Grasm alone) and `tircontrols` (§6.10), `fixpoint` (stage 2
= stage 3), `stagediff` (two stages' output on the corpus) and
`fixpointcontrols` (§6.14), `encodetest_x64` (the x86-64 encoder against
`as -arch x86_64`), `xs3subset` (crosstest with the Grasm backend),
`objtest_x64`, `ra_check_x64`, `xrtswap` (the x86-64 runtime and linker in
Grasm), `linkdiff_x64` (the linker against ld's x86-64 output), `fuzz_x64`
(the fuzzers on the x86-64 backend) and `x64controls` (§6.16), `clidiff`
(the `grasm` command in Grasm against the Rust one) and `clicontrols`
(§6.17), `objtest_elf` (ELF objects against the system's readers),
`elfsubset` (crosstest for the Linux triples, in the OrbStack machine),
`elfrtswap`, `elfrtlayout`, `elfrepro`, `linkdiff_elf` (the linker against
GNU ld) and `elfcontrols` (§6.18), `ddc`,
`selfperf` (compile speed and benchmark budgets), and the existing fuzzers
pointed at each new component. Every new gate ships with controls showing
it fails on a planted fault.

## 9. Size, risk, and what is not promised

**Size.** By component, from the Rust sizes and the reference compilers:
frontend 25-35k lines of Grasm, interpreter 8-12k, middle and two backends
45-65k, linker and object writers 10-15k, runtime 12-18k, tools 10-15k:
roughly **110-160k lines**. Zig's self-hosted compiler was about 250k
lines of Zig when its C++ predecessor was retired in 2022.

**Time.** This is many months of work, not weeks. The stages make progress
visible and keep the tree releasable throughout, so the date is not what
the plan depends on.

**Risks, each with its answer:**

| Risk | Answer |
|---|---|
| The compiler in Grasm is slow (refcounted trees are Grasm's slowest path) | data-oriented design (§3.5); Perceus RC (§4); `selfperf` budgets from S2 on |
| A ten-times-larger program exposes Rust-compiler bugs | expected and wanted: each one is fixed in the Rust compiler with a test while it is still the reference |
| Deep recursion in parser and checker | iterative algorithms where depth is input-controlled; the `depth` gate |
| Our backend's code is slower than Cranelift's | measured from S3; the release allocator and mid-end close the gap before S9 (§6.15's first steps: 3.17 to 2.05 times Cranelift's time on `bench.sh`'s programs, under load), and S9 does not ship a regression |
| Apple's linker conventions (ld-prime layouts, chained fixups, signatures) | the existing reader packages are checked against `dyld_info`, `codesign` and `nm`; `linkdiff` compares with ld-prime on every object |
| Two compilers to keep in step | the language freeze (§3.4) |
| macOS changes underneath | libSystem only; no private interfaces |

**Not promised:** peak performance beyond Cranelift-class at S9; Windows;
any target beyond the four in S7.

## 10. Decisions

Decided 2026-10-20:

1. **Bootstrap:** previous-release binaries, with the wasm seed of §7 added
   once the wasm32 backend exists; diverse double-compiling against the
   archived Rust implementation.
2. **Math:** our own correctly rounded libm, written in Grasm.
3. **Linker:** our own; `cc` and ld leave the native build at S4.
4. **Language freeze** for the length of the migration, apart from the
   systems subset (§5).
5. **Linux** in S7.

## 11. Where it stands (2026-11-01)

Measured, not estimated; each line is a gate's result on master.

- **S0 done.** TIR is specified (`docs/tir.md`) and `grasm tir` emits it
  for 1,671 corpus files with no holes. The TIR interpreter in Grasm
  (`tools/tirinterp`) prints and exits as `grasm run` does on all 651
  corpus programs with a `main`, none skipped, and gives the same panic
  on 25 sandbox-misuse programs.
- **S1 done except the context switch.** `u64`, bit operations, the
  builders, raw memory and atomics, function addresses, `call_c`,
  `export fn`, thread-local cells and tuple map keys, each with spec
  clauses, both engines, conformance tests, a fuzz gate and controls
  (§5.1-§5.4). The context switch is deferred to S3/S5 (§5.5).
- **S3 in progress.** `compiler/` holds an aarch64 backend in Grasm --
  MIR with explicit `inc`/`dec`, rule-based selection, linear scan with a
  checker, an encoder checked against `clang -c`, a Mach-O object writer
  -- and every corpus program with a `main` that does not link a system
  library builds with it: 648 of 651, each printing and exiting exactly as
  the Cranelift build does; the three that link SDL are set aside, as
  `difftest` sets them aside. Part one had 37 (scalars, control flow,
  calls, printing); part two, 289, added heap values with explicit
  reference counting, fallible functions, the built-in methods, arguments
  on the stack and places passed `inout` (§6.1); part three, 474, added
  `f64` as a second register class, closures and function values, and
  generics by monomorphization (§6.2); part four added references, ranges
  held as values, effects, tasks, raw memory and atomics, C functions,
  function addresses and `export fn`, and the rest (§6.3). All of them run
  with nothing live at exit (those that reach the leak check; a few end by
  design before it) and no use after free under the poisoned runtime.
  fib(35) was about 1.9x slower than Cranelift's code at part one. Part
  five (§6.4): the twenty generators that build native programs hold the
  Grasm backend to the Cranelift build on every program the Cranelift
  build accepts -- at their default sizes, every program each compared
  agreed, after two gaps they found were closed (`^` at a width,
  `str.parse_u64_base`; docs/known-issues.md) -- and every object carries
  a DWARF line table: `dwarfdump --verify` finds no error in any of the
  648, and 108,370 instructions that load a literal written once map to
  that literal's file and line. The release allocator (§6.5), backtracking
  by spill weight, is the driver's default, with linear scan kept for fast
  builds. Benchmarks against the Cranelift build (`scripts/s3bench.py`:
  `scripts/bench.sh`'s eight programs, 21 rounds interleaving the three
  binaries, medians), one session on 2026-10-26 at 04:26: the Grasm
  backend's time over Cranelift's, linear scan / release allocator --
  fib 1.497 / 1.460, sieve 6.875 / 6.771, matmul 12.869 / 12.847,
  wordfreq 3.168 / 3.161, tree 1.957 / 1.954, sort 1.292 / 1.291,
  parmap 2.060 / 2.058, parwords 6.271 / 5.402; geometric mean 3.289 /
  3.209; the release allocator over linear scan 0.976 (parwords 0.861,
  the rest 0.975-0.999). The load average was 2.70 when it started and
  rose to 2.81-3.37 from fib to sort and 5.0-6.4 over the two parallel
  rows, which swing by about 10% under load: these figures are as taken,
  not as `bench.sh` would publish them, and want repeating on a quiet
  machine. Why the gap is what it is per program was not measured. Part six
  (§6.15), the mid-end: scalar list elements, lengths and struct fields read
  and written inline, constant folding and immediate forms, loads and stores
  at an offset, inlining, and the frame set up where it is first needed. In
  one evening's runs interleaving master's backend and this one, at loads of
  2.4 to 5.7 (not to quote), the eight programs' geometric mean over
  Cranelift went from 3.17 to 2.05 (`sieve` 6.62 to 2.01, `matmul` 12.23 to
  2.52, `tree` 1.88 to 1.38), the Grasm runtime's over the Rust runtime's
  from 1.24 to 1.12, and the compiler's build of itself by stage 2 from
  182.9 s to 97.4 s: our backend's code 1.81 times Cranelift's on that work,
  from 3.80.
- **S4 done.** `compiler/link` (§6.6, about 4,400 lines of Grasm) links
  the native build on arm64 macOS, and `cc` is left as `GRASM_LINKER=cc`
  for one release. `linktest`: all 651 corpus programs with a `main` --
  the three SDL ones included -- linked from the Cranelift backend's objects
  and the 648 of the Grasm backend's subset from its own, with the three C
  fixtures: 1,302 links, each made twice and the two compared, every one
  running exactly as its `cc`-linked build and accepted by `codesign -v
  --strict`. `linkdiff`: 168 sampled programs are the very
  bytes the native build shipped through `cc` and `strip -no_uuid` (given
  ld's tool entry), and their unstripped links, the three C fixtures and the
  whole runtime agree with ld-prime's through `dyld_info`, `nm`, `otool -l`
  and section by section. `linkcontrols` catches all six planted faults.
  The whole gate suite but `s3controls` (which S4 does not touch) ran with
  `grasm-link` linking every native build: 187 of its 193 gates passed at
  once. Of the other six, two were this work's own mistakes, since fixed
  (`fmttest`, an unformatted file; `gold`, `fresh.sh` run from
  `ProjectGold/`); two lacked a Python environment a fresh checkout does not
  have and pass with it (`rmcptest`, `xxhashtest`); and two lost programs to
  SIGKILL and pass rerun -- seven of `fuzz_mapkeys`'s binaries refused by
  AppleSystemPolicy while the machine's network changed (the same seed then
  passed, and six alternating runs with `grasm-link` and with `cc` saw no
  refusal), and one of `difftest`'s.
- **S5 part one done** (§6.7). The Rust runtime's 345 entry points are
  inventoried by family, measured from the archive and the source, and the
  corpus names 323 of them. `compiler/rt` is the runtime in Grasm, built by
  the Grasm backend (`s3 --runtime`) and linked in front of the Rust one by
  `compiler/link -interpose`: 58 entry points moved -- math (35), number
  formatting with Rust's shortest floats by Dragon4 (12), the panics'
  messages (4), strings read (5) and reference counting (2) -- over 40
  layout constants held to a probe of the Rust runtime. The box allocator,
  the release walk, the output buffers and the panic path stay the Rust
  runtime's state until the last Rust code that uses them moves. `rtswap`:
  all 648 corpus programs that link no system library print and exit
  identically on both runtimes, from both backends' objects and against the
  poisoned runtime; `fuzz_rtfmt` 4,000,000 doubles and `fuzz_rtmath`
  16,500,000 math calls identical; `rtcontrols` caught all ten checks of
  nine planted faults. One difference between the runtimes was found, on the
  Grasm side, and fixed (`min`/`max` of a signalling NaN).
- **S2 part one done** (§6.8): the lexer and the parser in Grasm
  (`compiler/front`, about 8,500 lines of Grasm), with a lossless CST and the AST
  built from it, held to the Rust frontend through `grasm tokens` and the new
  `grasm ast`. `tokentest`: all 1,888 `.gr` files git tracks, 3,797,541
  tokens, every stream identical, stdout, stderr and exit status. `asttest`:
  the same 1,888 files, every AST dump identical -- the five the Rust parser
  rejects included, after recovery, with all six of their diagnostics -- and
  every CST printed back to its source byte for byte. `parsediag`: 1,480
  programs (the 65 conformance negatives, 815 programs read out of the Rust
  test suites' string literals, 600 from the fuzzer's generator): 706 the
  Rust parser rejects, each with the same first diagnostic from both; 770
  accepted by both; 4 on which the Rust lexer panics, the recorded panic.
  `fuzz_front`: 1,500 mutated, deep, literal, Unicode and random programs,
  1,484 agreeing on tokens, AST and every diagnostic, 1,248 of them with
  diagnostics, 16 the recorded Rust panic, none declined, none disagreeing.
  `frontcontrols` caught all seven planted faults (a token's start and its
  end one byte off, `while` lexed as a name, `+` and `*` swapped, an error
  node dropped from the CST, an expected-token diagnostic one byte off, the
  fuzzer skipping). Five defects of the Rust frontend were found and
  recorded in docs/known-issues.md, none fixed in Rust: a lexer panic
  (S1), a block whose value is a map refused (S3), two wrong diagnostics
  (S4) and exponential time on interpolations nested in blocks. Speed, on
  the corpus (1,736 files, 450,038 lines), read into memory, best of three
  per run, median of five runs interleaving the two, 2026-10-27 at 02:11 with
  the load average 3.1-5.4:

  | stage | Rust frontend | Grasm frontend | Grasm / Rust time |
  |---|---:|---:|---:|
  | lex | 4,219k lines/s (107 ms) | 804k lines/s (560 ms) | 5.2x |
  | lex, parse to the CST | -- | 193k lines/s (2,334 ms) | |
  | lex, parse, AST | 1,395k lines/s (323 ms) | 113k lines/s (3,977 ms) | 12.3x |

  The Rust side is `grasm_syntax` built at `opt-level=3` with one codegen
  unit, timed by a harness (not the `grasm` binary); the Grasm side is
  `front bench` built `--native`. The Rust frontend's `check`, quoted at
  365-400k lines/s, includes the checker; the Grasm frontend has none yet.
  Where the time goes was measured once: field writes (§6.8) were most of
  it until the hot state moved into whole variables, and the AST builder,
  which still writes through the AST's fields, is 40% of what is left.
- **S2 part two done** (§6.10): the loader, the checker and TIR emission in
  Grasm (about 14,000 more lines in `compiler/front`), held to `grasm tir`.
  `tirdiff`: all 1,925 `.gr` files git tracks, 1,858 the same TIR byte for
  byte and 67 refused by both, none disagreeing; `loaddiff`: 1,671 of 1,737
  programs load the same modules and the rest are refused by both; `tomldiff`:
  1,444 manifests and cases read alike by `packages/toml`, the Rust reader and
  `tomllib`; `tirrefuse`: 889 programs, 582 refused by both and 307 the same
  TIR; `fuzz_tir`: 20,000 generated programs and near-misses with no
  disagreement; `frontsmoke`: 47 programs from `compiler/front` through
  `tirinterp` and through s3 and grasm-link, each as `grasm run`;
  `tircontrols` 8 of 8. Two defects of the Rust native backend found and
  recorded (S1, S3), and one more fact about a Rust parser defect already
  recorded (S4). `front tir` takes 1.7-2.2 times `grasm tir`'s time; the
  checker's diagnostics are part three.
- **S5 part two, steps one to three done, step four begun** (§6.9): the
  runtime dialect of §5.6 -- process cells, static bytes, the runtime's own
  thread-local cells and the context switch -- usable only in a package
  whose manifest says `runtime = true`, refused by name everywhere else and
  by both other engines, in the Rust frontend and in compiler/front alike;
  lowered by the Grasm backend (`s3 --runtime`) to `__bss` symbols,
  `__cstring` bytes, Mach-O thread-local variables and leaves that switch
  contexts without signing anything; and guarded by `rtlint`, which refuses
  a plain access to a process cell outside a function holding its lock. The
  release walk is Grasm's for strings, lists, tuples, scalar lists, the
  builders, variants, function values and bytes; the other payloads' walk,
  the allocator, the output buffers, the panic path, maps and fibers are
  not moved yet. `rtswap`: all 648 corpus programs that link no system
  library print and exit identically on both runtimes, from both backends'
  objects and against the poisoned runtime; `rtdialect` drives every part
  of the dialect from C, linked by `compiler/link` and by `cc`;
  `fuzz_rtrelease` 20 seeds and 1,200 rounds identical, nothing leaking;
  `rtcontrols` 13 of 13, `rtdialectcontrols` 5 of 5 and `rtlint`'s 2 of 2
  planted faults caught. One fact bounds the switch's test: this machine's
  arm64 processes run with the return-address keys off.
- **S5 part three: steps one, two, four, five and six done, step three in
  part** (§6.11): 285 of the 345 entry points come from Grasm -- every
  family but output and the way out of a panic, which the Rust code still
  linked panics through; arguments, files, clocks, the network, processes,
  C strings and raw memory; and the parallel workers -- with the release
  walk for every payload, the allocator as a second allocator keeping one
  ledger with the Rust one, maps hashed bit for bit, and fibers, the stack
  floor and channels on `ctx_switch`. `rtswap`: all 648 corpus programs
  that link no system library identical on both runtimes from both
  backends' objects, and against the poisoned runtime with no use after
  free; `leakcheck` through `rtwith.sh` 623 leak-free, none leaking (25 end
  before the check); `usecheck` through it 648 clean; eleven new gates, one
  per family (`fuzz_rtmaps` to `fuzz_rttasks`), identical on every seed,
  `fuzz_rtcase` on every code point and `fuzz_rttasks` against the
  interpreter's scheduler too; `rtcontrols` 65 of 65 planted faults caught.
  Two defects found and recorded: the interpreter lost a wake for an outer
  `concurrent` block's task while a nested block ran (S1, fixed 2026-10-28
  after the stage), and the Rust runtime's region sweep frees a box before it has read
  it (latent: no compiled program sweeps a region). Measured once: one
  corpus program built by the Grasm backend took 7.0 s on this runtime
  against 3.8 s on the Rust one.
- **S5 part four** (§6.12): all 345 entry points come from Grasm -- the
  output buffer and every way out, arguments, files, clocks, the network,
  processes, C strings, raw memory and the journal they go through, and the
  parallel workers -- and programs link with the Grasm runtime alone, no
  `libgrasm_rt_abi.a` on the line. `rtswap`: all 648 corpus programs that
  link no system library identical on both runtimes from both backends'
  objects, 323 of 323 runtime names taken from Grasm, and with no use after
  free under the poisoned build; through `rtwith.sh`, `leakcheck` 623
  leak-free and none leaking (25 end before the check), `usecheck` 648
  clean, `replaytest` 44 journaled builtins hooked, and the 22 generators
  that build natively with nothing disagreeing; three new gates
  (`fuzz_rtout`, `fuzz_rtsys`, `fuzz_rtpar`) identical on every seed,
  `fuzz_rtrelease` leak-free with workers; `rtcontrols` 78 of 78 planted
  faults caught. Five Rust runtime defects found and recorded in
  docs/known-issues.md, fixed in both runtimes on 2026-10-31 together with
  a sixth that fixing them uncovered, and the backend gained three changes
  (§6.12). The
  benchmarks, measured once at a load of 2.4 to 2.6 on 8 cores: every
  program passes, `fib`, `sieve` and `matmul` within 9% of the Rust
  runtime, `tree` 1.86x, `wordfreq` 3.38x and `sort` 3.92x slower.
- **S2 part three done, and S2's exit criteria met** (§6.13): `front check`
  prints what `grasm check` prints, byte for byte -- every diagnostic with its
  line, column, quoted line, carets and help, in Rust's order, the count
  line, the ceilings' breaches with where each effect happens, or `ok` --
  with the manifests read by a port of the Rust toolchain's own TOML reader,
  directories compared by `realpath`, and the uncapped `^` and `else if`
  chains walked on work stacks. `diagdiff`: 4,244 programs (the conformance
  negatives, the Rust test suites' programs, 67 package cases, 674 long
  chains and deep shapes, 700 fuzzer programs, the corpus), 1,437 refused and
  2,807 accepted with the same output, none differing; `fuzz_diag` 2,200 of
  2,200 the same, and at 22,000 (seed 101) 22,000 of 22,000; `diagcontrols`
  9 of 9. Both chains check alike at 6,000 to 100,000 terms but for the `^`
  chain of 100,000, which aborts `grasm check` (a recorded Rust defect) and
  which `front check` answers. Seven defects of the Rust frontend found and
  recorded, none fixed in Rust, the six that are diagnostics reproduced by the
  Grasm frontend; part one's and part two's gates pass over the larger
  frontend. `front check` takes 2.4 times `grasm check`'s time on the corpus
  and 3.0 times on the compiler's packages, and checks about 55-60k lines/s
  on the generated files that measured the Rust frontend's 365-400k (390-450k
  today), measured under load and not to be quoted. Open: `front tir`
  prints nothing for a TIR of 4 GB.
- **S6 done but for the gates that need S8** (§6.14): the Grasm compiler --
  `compiler/front`, `compiler/s3`, `compiler/link` and the runtime
  `compiler/rt`, 68,162 lines -- built by the Rust one (stage 1) builds
  itself (stage 2) with no Rust runtime, no `cc` and no `grasm` on the way,
  and stage 2 builds a stage 3 that is the same bytes: every executable,
  object, TIR and runtime, at the first attempt, with nothing in the compiler
  changed for it. `fixpoint` holds stage 3 to stage 2 and each stage to
  itself built twice, 48 artifacts; `stagediff` holds stage 2's output on
  the corpus to stage 1's and stage 3's to stage 2's -- 1,974 files' TIR or
  diagnostics, 757 objects and 757 executables, the same bytes -- and
  `fixpointcontrols` catches its four planted faults (six checks). Every gate
  that takes the Grasm toolchain's binaries but `ra_check` passed with stage
  2's (`scripts/withstage.sh`): the S2 frontend gates, the S3 backend gates,
  the S4 linker gates, the S5 runtime gates with `leakcheck` and `usecheck`,
  `tirinterp` and the 21 generators with the Grasm backend as an engine. The
  gates that build, run, check or test through the `grasm` command cannot take
  it until S8. One thing found and not fixed: an object names the directory it
  was compiled in. Building the compiler takes 48 s by stage 1 and 181 s by
  stage 2 -- our backend's code 3.8 times Cranelift's on the compiler's own
  work -- measured under load and not to be quoted.
- **S7 part one done** (§6.16): an x86-64 Mach-O target, run under Rosetta.
  The allocators, the checker and the frame plan are shared with arm64
  (`compiler/vcode`, arm64's output unchanged byte for byte);
  `compiler/x86_64` selects, allocates and encodes for x86-64 (its encoder
  held to `as -arch x86_64` on 5,558 cases); the runtime compiles for it and
  `compiler/link` links it. All 648 programs of the subset compiled for
  x86-64 print and exit as the Cranelift x86-64 build and the interpreter do
  (`xs3subset`, crosstest self-hosted), and as the Rust runtime's builds do
  when linked by `compiler/link` with the Grasm runtime alone, the Grasm
  backend's own objects included (`xrtswap`); `objtest_x64` and
  `ra_check_x64` hold the objects and the allocation as arm64's are held.
  Since, S7's exit items that need no decision: `compiler/link` makes ld's
  x86-64 bytes (`linkdiff_x64`, 330 links byte for byte), `fuzz_s3` and the
  20 generators on the s3 engine check the x86-64 backend under Rosetta
  (`fuzz_x64`), and `s3bench` times it against Cranelift's x86-64 code there,
  figures not to be quoted. Left: Linux, part two.
- **S8 part one done** (§6.17): `compiler/grasm`, the `grasm` command in
  Grasm for `version`, `tokens`, `ast`, `toml`, `check`, `tir`, `run` (the
  TIR interpreter in the process), `build --native` (front, s3 and link in
  the process, with the runtime object beside it) and `links`, its command
  line read as the Rust one reads it and every other subcommand saying it is
  not here. `clidiff`: 144 command lines, 8,685 corpus commands, 1,473 TOML
  files, 651 runs, 648 native builds run alike and reproducible, 65 x86-64
  objects -- the same as the Rust command's, with stage 1's command and with
  stage 2's, the command executing nothing but itself; `clicontrols` 7 of
  7. Built by itself, its stage 3 is its stage 2 (`fixpoint`, 54 of 54), and
  its build of the compiler's programs is the stage pipeline's to the byte.
- **S7 part two** (Linux ELF, §6.18): the design, with the owner's decisions;
  the ELF object writer and the backend and runtime for arm64 and x86-64
  Linux, the corpus agreeing with the Cranelift build for each triple and
  with the interpreter, linked by GNU ld until `compiler/link` links ELF.
- **Not started:** the rest of S8, and S9.

S7 and S8 come next. S8 is also what S6's exit leaves: the `grasm` command's
subcommands in Grasm, so that the gates which build, run, check and test
through it can take the self-built compiler too.

## References

- Andrew Kelley, "Goodbye to the C++ Implementation of Zig" (2022): two
  compilers maintained in parallel, and the `zig1.wasm` bootstrap.
- go.dev/doc/install/source: Go 1.N bootstraps from 1.(N-2) rounded down
  to even; the compiler written in C ended with Go 1.4.
- David A. Wheeler, "Fully Countering Trusting Trust through Diverse
  Double-Compiling" (2009).
- Reinking, Xie, de Moura, Leijen, "Perceus: Garbage Free Reference
  Counting with Reuse" (PLDI 2021).
- Chris Fallin, Cranelift's ægraph mid-end (2023), ISLE, and regalloc2 with
  its checker.
- Poletto and Sarkar, "Linear Scan Register Allocation" (TOPLAS 1999).
- The CORE-MATH project: correctly rounded mathematical functions.
- mold and Wild: parallel and incremental linking.
