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

**S7's Linux CI run, 2026-11-13.** The owner's `ubuntu` machine was gone by
then; on the owner's word a scratch machine, `grasm-linux` (Ubuntu 25.10,
arm64, glibc 2.42, gcc 15.2, GNU ld 2.45 -- the versions this part was built
against, with `gcc`, `binutils` and `libc6-dev` installed), ran the five
Linux gates through `allgates.sh` with `GRASM_LINUX_MACHINE=grasm-linux` and
`GRASM_X64_ROOT` set, and was deleted after. All five passed:
`objtest_elf`; `elfsubset` -- on each triple 647 of 648 identical, 0
mismatched, 0 timed out, the one listed (`T-EXTERN-POINTER`, §6.18);
`elfrtswap` -- on each triple 648 of 648 from the Grasm backend's objects;
`linkdiff_elf` -- 324 links per triple, each made twice the same and GNU
ld's bytes, newest glibc required 2.38 as listed; and `elfcontrols`, 24 of
24 caught. The rest of the suite passed on master the week before but for
three controls S7 had broken (`linkcontrols`, `rtdialectcontrols`,
`elfcontrols`, fixed in "Three controls S7 left behind run again") and gates
whose oracles had gone (Go's module cache, again), each passing on its
rerun. With `crosstest` (x86-64 Mach-O, part one), that is S7's exit.

### 6.19 S8 part two: `fmt`, `lint`, `test` and `bench` in the `grasm` command

Written 2026-11-05, before any code, as the design to build to. Part one
(§6.17) left twelve subcommands answering "not in the Grasm toolchain yet".
This part takes four of them -- `fmt`, `lint`, `test` and `bench` -- into
`compiler/grasm`, held to the Rust command as part one's are: the same
reading of the command line, the same stdout and stderr, the same exit
status, the same files written. The Rust toolchain stays stage 0 and the
oracle and is not changed; where its behaviour is wrong it is reproduced and
recorded in docs/known-issues.md. The decisions below are numbered (1)
to (5); each was taken by the agent implementing this part and accepted by
the coordinator on 2026-11-05, and the owner may reverse any of it.

**What the Rust arms do** (`crates/grasm/src/main.rs`; the command line is
read first by the check every subcommand shares, which part one already
reproduces as `prevalidate`):

- `fmt <file> [--check]`. The operand is the first argument after `fmt`
  that does not start with `--` -- anywhere, after a `--` too, and a lone
  `-` or `-x` is an operand here, unlike every other subcommand's
  `first_operand`; `--check` likewise counts anywhere. The file is read as
  UTF-8 (`cannot read`), parsed, and refused if the parser has any
  diagnostic ("does not parse, so there is nothing to format" and the first
  three messages, indented). Then `grasm_syntax::fmt::format_src`: a line
  at a time, re-indented by a bracket-depth state machine over characters
  (strings, interpolations, raw strings, nested block comments, `|` and `.`
  continuations, contract clauses, a comment taking the level of the next
  code line) and re-spaced by `respace`, which splits the line into words,
  operators and opaque pieces by bytes and rejoins them by rule (unary
  signs, closure pipes, four kinds of brace, keywords before a parenthesis,
  aligned trailing comments kept). `--check` says `ok: ... is formatted`
  (exit 0) or that it is not (stderr, exit 1); without it the file is
  rewritten only if it changed (`formatted` or `already formatted`).
- `lint <file> [--json] [--only RULE] [--rules]`. `--rules` anywhere prints
  the six rules (`{name:18} {why}`) without reading a file. Otherwise the
  file is read and parsed as for `fmt` ("nothing to lint"), `--only` is
  checked against the rules, and `lint.rs` walks the AST: scopes as the
  language nests them, a name used only when read, `_` and the parser's
  U+0001 discards exempt; unused bindings, parameters and imports,
  shadowing (naming the line of the hidden one), functions over 60
  statements, literal shift counts outside 0..63. One input is not the
  tree's: the imports a file uses only through a type or variant they
  declare, found by loading the program (`imports_used_for_types`; a load
  that fails gives none). Findings are sorted by line and byte column,
  stably, and printed as `file:line:col: rule: message` and a count, or as
  a JSON array; exit 1 when there is a finding.
- `test <file> [--coverage] [--contracts] [--seed N] [--cases N] [--filter
  T] [--schedules N] [--schedule-seed S]`. The operand by `first_operand`,
  then the options in order (`--seed`, `--cases` -- at least 1 --,
  `--schedules`, `--schedule-seed`, each worded "`--x` expects a number,
  got `v`"; Rust's unsigned parse takes a leading `+`), then the program
  loaded, checked and refused if it is a runtime package, as `run` does.
  The Rust interpreter (`grasm_interp::test_program_with`) then runs, in one
  interpreter whose state carries from test to test: every `test` of every
  module in load order, labelled `module::name` when the program has more
  than one module, filtered by substring; a unit test once, reporting its
  panic, error or `exit` as the failure, and with `--schedules` again under
  N seeded task orders if it ran a `concurrent` block; a property test over
  `--cases` inputs generated by a splitmix64 stream from `--seed` (default
  `0x5eed`) by the *written* type of each parameter, the first ten cases
  walking boundary lists, a failure shrunk greedily within a budget of 2,000
  tries and reported with the counterexample and a replay line; with
  `--contracts`, each function with a body and an `ensures` as a property of
  its own parameters, inputs its `requires` refuses discarded; then every
  fenced example in a `///` comment, the examples of one module compiled
  together into a synthesized program (the owner's source inlined, its
  imports as the loader resolved them, each example a `test`), checked, and
  run, or one at a time if the group does not compile. Then `N passed, M
  failed`, the coverage report with `--coverage` (statement lines reached
  against statement lines, per file, `{:.1}%`, uncovered lines as ranges),
  exit 1 if anything failed.
- `bench <file> [--filter T]`. Loaded as `test` loads; every `bench` of
  every module (labelled as tests are, filtered by the bare name), each run
  once to warm up -- a panic or an error is the result, `FAILED: ...` -- then
  in batches of 16 until 200 ms or 20,000,000 iterations; one line each, the
  name padded to the longest name's length in bytes (padded in characters,
  as `format!` pads), the time per iteration in ns, µs or ms right-aligned in
  12, and the count; "no benchmarks found ..." when there are none; exit 1 if
  one failed.

**What Grasm already has.** `tools/gfmt` is `format_src`'s re-indenting half
(the state machine), held to `grasm fmt` by `gfmttest` -- over corpus files
that are already formatted, so the gate cannot see that it lacks `respace`
altogether, splits lines on `\n` alone where Rust also takes a `\r` before
it, and trims by `trim_start`. `compiler/front` has the lexer, the parser
with Rust's diagnostics (`tokentest`, `asttest`), the AST with its doc
comments, the loader, the checker and TIR emission; TIR already carries
every `test` (name, parameters with their resolved types, body) and `bench`
(docs/tir.md §4). `tools/tirinterp` runs a program's `main` from its TIR as
`grasm run` does, and is linked into the command; it reads no `test` or
`bench`, and it ends the process on the program's panic with the host's own
`panic`, so nothing it runs can fail without ending everything.

**What has to be written**, and where:

- `fmt`: `tools/gfmt` becomes the whole formatter -- `respace`,
  `split_pieces`, `scan_string`, `brace_kind`, `needs_space` ported, the
  line split and the trims done as Rust does them (`split_inclusive('\n')`,
  a `\r` stripped before a `\n`, Unicode whitespace trimmed by `trim_start`
  and, in the comment look-ahead, `trim`) -- with `format_src` public, so
  the command imports it as it imports `tirinterp`, and `gfmt` stays a
  program of its own. `fmt` in `compiler/grasm` reads, parses with
  `front`'s parser for the refusal, formats, compares and writes.
- `lint`: `compiler/grasm/src/lint.gr`, a port of `lint.rs` over `front`'s
  AST (`asttest` holds that AST to Rust's, node for node), with the type
  imports from `front`'s loader, the source of the entry given in memory as
  `load_from_disk_in` gives it.
- `test` and `bench`: `compiler/grasm/src/testing.gr` -- the options, the
  runner, the generator, the shrinker, the counterexample's rendering
  (strings by `front`'s `rdebug`, Rust's `{:?}`; other values as the
  interpreter prints them), coverage and the bench loop on
  `monotonic_nanos` -- over an interface `tirinterp` gains: tests and
  benches read from TIR into its program, and a call that runs a test's or
  bench's body, or a function, in a context kept from one call to the next,
  and reports how it ended.
- Doc examples: the synthesized program written as `example_group_program`
  writes it, then loaded by a `front` loader entry that takes the modules'
  sources from memory with the import resolution of the program it came
  from (a port of `load_with_resolved_imports`), checked, emitted and run by
  the same runner; one at a time, with "FAILED to compile: <first
  diagnostic>", when the group does not check.
- Coverage: the Rust interpreter records the start of each statement it
  evaluates. TIR gives a `let` no span, but a TIR block should line up
  statement for statement with the AST block of the same span (docs/tir.md
  §1: no desugaring of TIR's own), so the runner maps each TIR block's
  children to the AST's statement starts and counts a statement when its
  block reaches it, and refuses a program where a block does not line up
  rather than guess; the executable lines are the AST's, as
  `executable_lines` collects them. Whether every block lines up is to be
  measured on the corpus, not assumed.

**How `test` and `bench` run programs: the TIR interpreter, in this
process.** Decision (1), taken by the agent and accepted by the
coordinator, for three reasons. It is the engine the Rust
`grasm test` and `grasm bench` run on, and §6.17 put `run` on it for the
same reason: a test's prints share the command's stdout with the report
lines, in order, and a runtime package is refused in the same words. A
native build would need a process per test -- one cannot catch a native
program's panic, and the Rust runner reports a panicking test and goes on --
and the command starts no process (clidiff's sandbox). And every
`--seed`/`--cases` replay line and every shrunk counterexample depends on
running the same test body many times within one command, cheaply: the
first run of a freshly written executable alone costs about 0.45 s on macOS.
What it costs: the TIR interpreter takes 0.8 to 2.9 times the Rust
interpreter's time (§6.17), and `bench`'s numbers become the TIR
interpreter's.

So, decision (2), **a panic in `tirinterp` becomes a way out of an
evaluation, not the end of the process** (taken by the agent, accepted by
the coordinator on the condition that `run` stays the same bytes). Beside `break`, `continue`, `return` and an error, a
context gains two more reasons to unwind: a panic, carrying its message, and
an `exit`, carrying its status. Every panic the interpreter words for the
program (about 120 sites in `eval`, `builtins`, `rawmem` and `cabi`) and
`exit` go through one function that, under `run`, does exactly what it does
now -- the host's `panic` or `exit`, the same bytes -- and under `test` and
`bench` sets the reason and unwinds; `finish` and the loops pass it through,
as they pass nothing else but an error. Where the interpreter leaves a
program's fault to the host's own operation (a `substr` out of range, an
empty fill), the host would still end the process, so each of the Rust
interpreter's panic sites (163 in `interp.rs`, 12 in `tasks.rs`) is checked
against the TIR interpreter's and the missing ones are tested for before
the operation, in its words. Decision (3): two places stay the host's,
and are the one difference between the commands this part allows
(docs/known-issues.md records both): a panic inside a task of
a `concurrent` block (the host's scheduler, which the interpreter's tasks
run on, cannot abort a task's siblings as [E-TASK-PANIC] requires, so
`test` ends there with `panic: ...` and 101, where the Rust command reports
the test failed and goes on), and the host's own stack running out. A
third of the same cause was found after the decision, and accepted with it
by the coordinator (2026-11-05): a panic the runtime's channel operations raise (a send on
or a close of a closed channel, a channel operation outside every task that
would wait), since the interpreter's channels are the host's too.

**`--schedules` and `--schedule-seed`** (decision (4)). The Rust interpreter
has its own scheduler, and `--schedules N` reruns a unit test that ran a
`concurrent` block under N task orders drawn from seeds; the TIR
interpreter's tasks are the host's, scheduled by the runtime by
[E-SCHEDULE]'s fixed rule, with no seed to give. When no test starts a task
the flags change nothing, and the Grasm command answers as Rust's does. When
one does and the Rust command would act on the flag -- `--schedule-seed`
given, or `--schedules N` above 0 and the test passed -- the Grasm command
says "error: `grasm test --schedules` is not in the Grasm toolchain yet"
(or `--schedule-seed`'s) in place of that test's report line, with part
one's help, exit 1 -- taken by the agent, accepted by the coordinator because it refuses
rather than answering otherwise in silence, the owner's to reverse; the
alternative is a scheduler of the interpreter's own, which means evaluating
without the host's stack (every task a state machine), a rewrite of the
interpreter's task model.

**How the output is held to Rust's, byte for byte** (decision (5), taken
by the agent and accepted by the coordinator). `clidiff` gains four
sections, each both commands on the same input from the same directory, the
counts asserted:

- `fmt`: every `.gr` file git tracks, each copied into an empty directory
  of each command's own and formatted there (`fmt F`, then `fmt --check F`
  before and after): stdout, stderr, exit status and the file left behind,
  byte for byte. The corpus is formatted (`fmttest`), so this alone would
  test `already formatted` 2,000 times, as `gfmttest` does; so each file is
  also put through three rewrites `scripts/lib/fmtmangle.py` makes,
  deterministic -- every line's indentation removed, every run of spaces
  doubled, every space next to punctuation removed -- and those formatted by
  both. A rewrite that stops the file parsing is compared as a refusal.
- `lint`: every `.gr` file git tracks, `lint F` and `lint F --json`; every
  tenth with `--only R` for each of the six rules and once for a rule that
  does not exist; `--rules`. Findings, their order, their JSON, the count,
  the status.
- `test`: every corpus program, `test P` (a program with no test still
  prints `0 passed, 0 failed`, after any doc examples of the modules it
  loads); the programs with a property test also with `--seed 7 --cases 37`
  and a `--filter`; the ones with a contract with `--contracts`, under the
  run limit (`T-CONTRACT-ENSURES`'s `good_sqrt` counts to the square root
  of `i64::MAX` for one generated input, and runs for hours under both
  commands); `--coverage` on every tenth and every one with a test. Output,
  status. And, so that the panic path is exercised by more than the few
  programs that have failing tests: every corpus program with a `main`,
  copied with `test "main" { main() }` appended -- the whole corpus copied
  into the work directory, so imports resolve as they do in the tree --
  whose test fails exactly where `run` panics or exits.
- `bench`: every corpus program. The time and the count are not anyone's
  to reproduce, so each line is compared with the two numbers taken out:
  the name and its padding, the `FAILED: ...` lines and the `no benchmarks
  found` line byte for byte, the order, the status, stderr; and each timing
  line must be the format exactly -- the time a number with one decimal
  under 1,000 ns and two above, `ns/op`, `µs/op` or `ms/op` as its size
  says, right-aligned in 12, the count a positive multiple of 16 no larger
  than 20,000,000. Run with `--filter` too, which must drop the same
  benchmarks.

The command-line cases (`scripts/lib/clicases.txt`) lose their `elsewhere`
tags for the four and gain the lines that reach each arm's own checks (the
operand anywhere, `--check` after `--`, `--rules` anywhere, `--cases 0`,
`--seed +5`, `--seed x`, `--only nosuch`, a file that does not parse, one
that does not exist, one not UTF-8). `clicontrols` gains one control per
subcommand at least: a finding dropped (lint), a format change (fmt), a
test's result inverted (test), a bench filter ignored (bench).

**Self-hosting.** The command grows by the formatter, the linter and the
runner; `fixpoint` builds it as before and must still find stage 3 its
stage 2, and `clidiff` runs again with stage 2's command. `tirinterp`
changes too, so `tirinterp`, `frontsmoke` and `clidiff`'s `run` section
hold its `run` to what it was.

**Copied from Rust, and recorded as Rust's defect.** `bench` pads each name
to the longest name's length in bytes, counted by `format!` in characters,
so a non-ASCII name widens the column by its extra bytes; the Grasm command
pads the same way (docs/known-issues.md, with a reproducer).

**Steps two to four, as built** (2026-11-05, while the main checkout's
full suite was running, so with light checks only; the corpus-wide gates are
step five's and were not run).

- `fmt`: `tools/gfmt/src/fmt.gr` is the whole formatter, `respace` with it,
  and `gfmt` and the command both import it. `compiler/grasm/src/fmtcmd.gr`
  is the arm. `scripts/lib/fmtmangle.py` writes four rewrites of a file:
  dedented, spaces doubled, spaces beside punctuation removed, and CR LF --
  four, not the three planned, because a CR before a newline is the one
  thing `gfmt` got wrong. Held to the Rust command on 28 tracked files and
  their rewrites (137 formatted alike, 3 refused by Rust), 10 hand-written
  edge files, and 18 command lines; no difference.
- `lint`: `compiler/grasm/src/lint.gr`. Held on 79 tracked files, plain
  and `--json` (51 findings), six files written to have findings (101:
  every rule, and imports used only through a type, a missing import
  among them), and 19 command lines; no difference.
- `test` and `bench`: `compiler/grasm/src/testing.gr`, on the changed
  `tools/tirinterp` (`eval.fault`, `eval.exit_with`, `run_body`, tests and
  benches loaded) and `front`'s `load_sources`. Held on the 42 corpus files
  with a test the Rust command finishes, plain and with `--coverage` (43
  with a test, less `T-CONTRACT-ENSURES`), every documented example
  (`examples/documented.gr`, and three files written for doc examples: one
  in an imported module, one that does not compile, a `text` fence, `# `
  lines, prints), the flags (`--seed` at `u64`'s maximum and past it, `+5`,
  `--cases 0`, `--contracts` on `T-CONTRACT-REQUIRES` and `-PURE`), 44
  sampled corpus programs under `test` and `bench`, and the 25 sandbox
  panics of `tools/tirinterp/panics`, 14 conformance panics and 38 sampled
  programs, each with `test "main" { main() }` appended. Every one agreed
  but those this section allows: `--schedules` and `--schedule-seed` on a
  test that ran tasks, a panic inside a `concurrent` task, and the two
  channel panics (`E-CHANNEL`, `E-DEADLOCK`) accepted above. `exit(-1)` inside
  a test first said `exit(-1)` where Rust says `exit(255)`: the reference
  carries the low eight bits, and so does `exit_with` now. `run` was held on
  57 programs, 39 of them panicking, after the change; the same bytes.
- The gates: `clidiff` gains the four sections (`fmt`, `lint`, `test`,
  `bench`), each with counts that must add up and floors on a whole run,
  and `scripts/lib/benchnorm.py`; with `FILES=` the `fmt` and `lint`
  sections take its list rather than every tracked file. On 12 programs:
  "fmt: 12 files, each and four rewrites, 120 commands the same: 120
  rewritten alike: 44 refused alike: 5", "lint: 12 files, 31 commands (7
  with --only) the same: 31 findings: 2", "test: 12 programs, 23 commands
  (3 with a property test, 1 with a contract, 4 with --coverage) the same:
  23, 4 of them with a failing test the host's panic: 0", "test, main as a
  test: 11 programs the same: 10, 2 of them failing the host's panic: 1"
  (`E-CHANNEL`), "bench: 12 programs, 18 commands (3 with a benchmark, each
  filtered twice) the same: 18 timing lines in the format: 6".
  `clicontrols` gains five controls, and `CONTROLS=` to run some: on their
  own, "controls: 5 caught: 5 missed: 0" -- a space before a comma (fmt), the
  shadowing finding dropped (lint), a passing unit test reported failed
  (test), the runner's trap taken away (test), `--filter` ignored (bench).
  The floors on fmt's rewrites, lint's findings, failing tests and mains
  as tests are estimates until a whole run measures them.

**Step five: the gates** (2026-11-05 to 13, this branch's tree; the
first runs beside the main checkout's full suite, at loads of 3 to 17, so
their times are not to be quoted; then rebased on `112239a9`, whose three
commits touch only other gates' scripts, and the stage-1 gates rerun):

- `fixpoint`, on the rebased tree: "programs: 6 and the grasm command
  artifacts per stage: 16 and 2 compared: 54 ... identical: 54 differ: 0";
  "stage 2's TIR the same as grasm tir's: 6 of 6"; "the command's build the
  stage pipeline's (its rt.o and the 4 programs): 5 of 5". The command's
  stage 3 is its stage 2: `grasm` 4,322,680 bytes (4,025,208 at §6.17),
  SHA-256 `2071d6931e8ba31c2d5e6ee4d579b4a5b354f542f10fb3f9a97a0aa148123d85`,
  and `rt.o` 555,224 bytes, the compiler's stage-3 `rt.o` to the byte. The
  same lines and digests on the tree before the rebase.
- `clidiff`, with the command stage 1 is, and again with stage 2's
  (`fixpoint`'s kept stage, before the rebase), the same lines both times:
  "lines: 195 the same: 179 not here, as §6.17 says: 11 the runtime built: 1
  a target refused: 4"; "corpus: 1737 programs, 8685 commands ... the same:
  8685, 138 of them refusals"; "toml: 1474 files the same: 1474, 19 of them
  refused by both"; "run: 651 programs with a main the same: 651 timed out:
  0"; "fmt: 2007 files, each and four rewrites, 20070 commands the same:
  20070 rewritten alike: 7638 refused alike: 385"; "lint: 2007 files, 5414
  commands (1400 with --only) the same: 5414 findings: 1001"; "test: 1737
  programs, 1961 commands (6 with a property test, 3 with a contract, 209
  with --coverage) the same: 1960, 12 of them with a failing test the host's
  panic: 0 both timed out, --contracts: 1" (`T-CONTRACT-ENSURES`); "test,
  main as a test: 651 programs the same: 649, 13 of them failing the host's
  panic: 2" (`E-CHANNEL`, `E-DEADLOCK`, as accepted above); "bench: 1737
  programs, 1743 commands (3 with a benchmark, each filtered twice) the
  same: 1743 timing lines in the format: 6"; "build: 648 programs with a
  main (3 link a system library) the same output: 648 ... run alike: 648
  built twice the same: 648"; "target: 65 programs ... 195 of 195"; no
  `proc_run`. 281 minutes with stage 1's command, of which an hour was
  `T-CONTRACT-ENSURES` under `--contracts` running into the 30-minute limit
  under each command; `--contracts` now has two minutes, as the stage-2
  run had (246 minutes). The floors were then set a margin below these
  counts (the agent's, at the coordinator's request): rewrites 7,000,
  refusals 300, findings 900, failing tests 10, mains failing as tests 11,
  the host's panics at most 4, timing lines 6.
- `clicontrols`: "controls: 14 caught: 14 missed: 0" -- part one's nine and
  this part's five. 47 minutes.
- `tirinterp` "identical: 651 mismatched: 0 timed out: 0 skipped: 0",
  "round trip: 1671 identical, 0 not", "panics: 25 identical, 0 not";
  `frontsmoke` "programs: 47 through tirinterp as grasm run: 47 compiled by
  s3, linked by grasm-link, as grasm run: 47"; `tirdiff` "files: 2007 TIR
  identical: 1940 both refuse: 67 disagree: 0 holes: 0", "emitted twice:
  194 unstable: 0"; `loaddiff` "programs: 1737 same modules: 1671 ...
  disagree: 0"; `diagdiff` "programs: 4277 ... refused with the same output:
  1437 (conformance: 66) accepted by both: 2840 differ: 0 Rust panics: 0";
  `tircontrols` "controls: 8 caught: 8 missed: 0" -- all before the rebase.
- On the rebased tree: `fmttest` "formatted: 2001 unformatted: 0 does not
  parse: 5 exempt: 1"; `gfmttest` "checked: 1533 disagreeing: 0"; `linttest`
  "linted: 2002 findings: 1001 does not parse: 5 broke: 0"; `gatecheck` "gate
  helpers checked: 587 undefined: 0 ..."; the litter check (the files git
  does not track, and the checkout's top level, against a list taken before
  the first gate) "new untracked files: 0". The conformance programs the
  gates run left a `target/` in the worktree (`demo-out`,
  `grasm_bytes_conformance`, `grasm_fs_conformance`), which git ignores and
  which was removed.

Not run on this branch: the rest of `allgates.sh`. The gates of §6.14's
last list that run through the `grasm` command still take the Rust one.

**What is left for S8 after this part.** `doc`, `api`, `deps`, `publish`,
`add`, `lsp`, `dap`, `journal`, and `build` without `--native` and with
`--source`; `test --schedules` and `--schedule-seed` for a test that starts
tasks, which need a scheduler of the TIR interpreter's own (decision (4));
a panic inside a `concurrent` task under `test`, and one the runtime's
channel operations raise, for the same reason, and the interpreter's own
stack (decision (3)); the LSP on the query engine,
DAP, packages, journal and replay, the wasm32 backend and one TOML reader,
as §6.17 left them.

### 6.20 S8 part three: `doc`, `api`, and `build` without `--native`

Written 2026-11-13, before any code, as the design to build to. Parts one
and two (§6.17, §6.19) left eight subcommands answering "not in the Grasm
toolchain yet", and `build` without `--native` and with `--source`. This
part takes `doc`, `api` and those two forms of `build` into
`compiler/grasm`, held to the Rust command as the others are: the same
reading of the command line (`prevalidate` already reads all four), the
same stdout and stderr, the same exit status, the same files written. The
Rust toolchain stays stage 0 and the oracle and is not changed; where its
behaviour is wrong it is reproduced and recorded in docs/known-issues.md.
The decisions are numbered (1) to (4); each was taken by the agent
implementing this part and accepted by the coordinator on 2026-11-13, and
the owner may reverse any of it.

**What the Rust arms do** (`crates/grasm/src/main.rs`, `grasm_sema::docs`,
`grasm_sema::loader::to_source_bundle`):

- `doc <file> [-o out.md]`. The operand is `first_operand` after `doc`
  (`-o`'s value skipped, `--` ends the search, a lone `-` is an operand);
  none is "usage: grasm doc <file.gr> [-o out.md]", exit 1. The program is
  loaded and checked as `check` does (`load_and_check`, which every arm
  here shares, and which part one reproduces as `front.checked`). Then
  `docs::collect`: for each module in load order, its declarations in
  order -- in the entry module (the first) every `fn`, `type`, `let` and
  `extern fn`, in the others only the `pub` ones; every `effect` in any
  module; never a test, bench, import, interface, `impl`, thread- or
  process-local. The declarations are the AST's, so the parser's
  desugarings are documented as it leaves them (an `impl`'s methods as
  `name@Type`, an actor's state type and handle, a contract's wrapper and
  clauses). Each item is a kind, a name, its signature written back from
  the AST without a body (`fn_signature`, `type_signature`,
  `effect_signature`, `const_signature`, `extern_signature`, all over
  `type_text`), and the `///` text filed under the declaration's start.
  `render_markdown`: `# <title>`, then per module with an item `## Module
  `m``, per item `### `kind` name`, the signature in a fence, and the doc
  text with the `# ` setup lines of its fenced blocks dropped
  (`without_hidden_lines`: `str::lines`, `trim_start` by Unicode
  whitespace, rejoined with `\n`, so a trailing newline and any `\r` go).
  The title is the name in the manifest of the package the file is in:
  `enclosing_package` walks up from `Path::parent` of the operand -- for a
  bare `x.gr` that is the empty path, so the working directory's
  `grasm.toml` -- to the first directory whose `grasm.toml` is a regular
  file, and takes `Manifest::load` of it; a manifest that does not load
  ends the walk with no name. Without a name, the file's stem; without a
  stem, `API`. With `-o` (its value from the first `-o` anywhere after
  `doc`, after a `--` too) the text is written there and `wrote <out> (N
  item(s))` printed; a write that fails is `error: cannot write <out>:
  <io error>` -- the one message in the command that does not quote its
  path in backticks -- exit 1. Without `-o` the text goes to stdout as it
  is (`print!`: it ends in one newline, and no other is added).
- `api <file> [--against F]`. The operand is the argument after `api`
  alone, unless it starts with `--` (not `first_operand`: `api --against
  b x.gr` is the usage, `usage: grasm api <file.gr> [--against
  <saved.api>]`). Loaded and checked as above. `api_surface`: for every
  module whose name's first dotted segment is not a dependency package's,
  each `pub fn`, `pub extern fn` and `pub let`, and every `type` and
  `effect` (one namespace program-wide, [T-MODULE-NS]), as `module \t name
  \t signature`. A `pub let`'s signature is `pub let N: T` with `T` the
  checker's type of it as `TypeDisplay` shows the type resolved when the
  constant was collected (`Analysis::const_types`), else the written one;
  in every signature but a type's own, each word that names a type alias
  without generics is replaced by the alias's written target, eight rounds
  at most (`expand_aliases`: a word is a run of alphanumeric characters and
  `_`). The lines are sorted, duplicates dropped, each ends in a newline,
  and without `--against` they are the output as they are (`print!`;
  nothing at all for a program with no surface). `--against`'s value is
  the argument after its first occurrence anywhere on the line, after a
  `--` too. That file is read as UTF-8 (`error: cannot read `F`: <io
  error>`); a baseline in which no line splits into exactly three by tab,
  while the surface is not empty, is refused in two lines naming it.
  `report_api_change`: both sides read line by line (`str::lines`), a
  line of at least three tab-separated fields keyed `module::name` (the
  last of a key wins); `REMOVED  k` for each key gone, `CHANGED  k` with
  `    was  ` and `    now  ` lines for each signature that differs,
  `ADDED    k` for each new one, each group in key order; a blank line; and
  `breaking: R removed, C changed, A added — this is a MAJOR release` (exit
  1), `compatible: A added — this is a MINOR release` (exit 0) or `no
  change to the public surface — this is a PATCH release` (exit 0).
- `build <file> [-o out]` without `--native`. Every check part one
  reproduces, in order (§6.17: the operand, the links, the output, not a
  regular file, its directory, the source itself, a `.gr` name, `--target`
  without `--native`), then the check path, `no `main` function`, and a
  runtime package refused naming "the interpreter". Then `--source` if it
  is anywhere on the line, whether or not `--native` is (below). Otherwise
  `write_bundle`: the Rust command's own executable (`current_exe`, read
  whole, any bundle already on it cut off) followed by a payload, the
  payload's length as a little-endian `u64`, and the eight bytes
  `GRASMBDL`. The payload is the program's modules in load order (each a
  `u32` length and the name, a `u32` length and the source), the
  dependency packages' names in byte order, every module's imports as
  (module, alias, the module it resolved to) sorted, and the entry file's
  name without its directory -- every count and length a little-endian
  `u32`. Written in place by `fs::write`, given mode 0755; `built `out` (N
  bytes, M module(s))`, or `error: <io error>` and exit 1.
  Every start of the Rust `grasm` reads its own executable whole first, and
  if it ends in `GRASMBDL` it is that program and never reads its command
  line: the modules are loaded from the payload (`load_with_resolved_
  imports`, the entry `main` shown under the recorded file name, the
  imports as recorded, the dependency set), checked (the loader's and the
  checker's diagnostics, rendered as `check` renders them; no capability
  ceiling), and run by the interpreter with every argument as the
  program's, the exit status its low eight bits, a panic's stack trace on
  stderr.
- `build <file> --source [-o out]`: the same checks, then
  `to_source_bundle` -- each module in load order as a line `//!module
  <name>`, then its source, then a newline if it lacks one -- written to
  `-o`'s path, or, if that is empty, to the operand with every trailing
  `.gr` taken off and `.bundle.gr` added; `wrote `dest`: M module(s), N
  bytes`, or `error: cannot write `dest`: <io error>`. The loader reads such
  a file back as the program (`load_from_source_bundle`), which `front`'s
  `load_bundle` already ports.

**What Grasm already has.** `front.checked` is `load_and_check` with
Rust's diagnostics (`diagdiff`); front's AST is Rust's node for node, its
`///` map included (`asttest`); its loader has the module order, each
module's resolved imports and the dependency set (`loaddiff`), and
`load_manifest`, `Manifest::load` in Rust's words (`tomldiff`, and the
loader's own manifest errors in `diagdiff`); its checker keeps each
constant's type as resolved when collected, in `Ck.consts`, and `tys`
writes a type as `TypeDisplay` does (the checker's diagnostics depend on
it). `compiler/grasm/src/testing.gr` already ports `str::lines`,
`decl_name` and the doc-comment walk for `grasm test`'s doc examples, and
`front`'s `load_sources` is `load_with_resolved_imports` (§6.19), which
the bundle's start needs. `tools/gfmt/src/fmt.gr` has Rust's
`char::is_whitespace` as bytes. The command finds its own path
(`_NSGetExecutablePath(3)`, part one) and has the TIR interpreter linked in.
What it lacks: the signature writer and the Markdown renderer, the API
surface, alias expansion and the comparison, the payload writer and
reader, and the start that checks for a payload.

**What has to be written**, and where:

- `compiler/grasm/src/docgen.gr`: `type_text` and the signature writers
  over front's AST; `collect` and `render_markdown`; `cmd_doc`. Trims by
  gfmt's whitespace rule rather than the language's `trim_start`, whose
  set is not held to Rust's anywhere for non-ASCII spaces. Rust's rule
  takes a fence or a hidden line behind any amount of any Unicode
  whitespace -- a no-break space, an em space, five spaces -- where
  CommonMark takes at most three spaces (checked 2026-11-13 with `grasm
  doc`); it is copied, and recorded in docs/known-issues.md.
- `compiler/grasm/src/api.gr`: `api_surface` (with `expand_aliases`: a
  word is ASCII alphanumerics and `_` -- exact, because identifiers are
  ASCII and the only other characters a signature can hold, the parser's
  `\u{1}` and `§` in desugared names, are not alphanumeric),
  `report_api_change`, `cmd_api`.
- `compiler/grasm/src/bundle.gr`: the payload written and read, `--source`,
  and the bundled program's start: called first in `main`, it asks the
  size of its own executable and reads the last sixteen bytes
  (`read_bytes_at`), and only when they end in the magic reads the
  payload, loads the modules from memory (`load_sources`, given the entry's
  shown name), checks them, emits TIR and runs it with `tirinterp.run_tree`
  and every argument. Rust's reads the whole 11 MB file on every start;
  reading the tail answers the same question.
- `compiler/front`: `load_sources` takes the entry's shown name (now fixed
  as `<doc example>`); a public check of an already loaded program that
  renders its diagnostics as `checked` does (the bundle's start); a public
  `enclosing_package` name (the walk, `load_manifest`); `tys` writes a type
  without resolving it again (`const_types` is the type as resolved at
  collection, and a variable bound afterwards still shows as `_`).
- `build.gr`: the "not in the Grasm toolchain yet" refusals for `--source`
  and for `build` without `--native` replaced by the two; `main.gr`: `doc`
  and `api` out of `elsewhere`, into the dispatch and the help line.

**The bundle: the Grasm command and Rust's payload** (decision (1)). The
bundle `build` writes without `--native` is the toolchain's own executable
-- the interpreter -- carrying the program. The Grasm command is, since
part one, an executable with the TIR interpreter compiled in, so its
bundle is the same construction: its own executable, read through
`_NSGetExecutablePath(3)`, followed by the payload in the Rust command's
format, byte for byte; and the Grasm command, started as such a file, runs
the program on the TIR interpreter, the engine its `grasm run` runs. For
three reasons. Nothing starts a process, and nothing but the command's own
bytes goes into the file, so no Rust code is on the path. The payload being
Rust's to the byte, the gate compares the written file exactly beyond the
executable at its front, which it checks is the command itself -- where any
other payload could be compared only by what the bundle does when run. And
a bundle runs as `grasm run` runs, which is what it is for: stdin, stdout
and stderr the program's own, a runtime package refused in the
interpreter's words, `--target` refused because "a bundled build carries
the interpreter, which is this machine's" -- all still true. Not taken: a
payload of the program's TIR, which would start without loading and
checking again but could not be compared to Rust's except by running; and
a native executable through `s3` and `link`, which is `--native`, would
need `rt.o` where the Rust bundle needs nothing beside it, and would make
those two refusals' words false. What it costs: a bundle is about 4.3 MB of
command and the program's sources, where Rust's is about 11.5 MB; the
success line's size is the Grasm command's, so it differs, as `--native`'s
does. The executable before the payload keeps the linker's ad hoc
signature, which covers the code alone. A payload appended after a signed
arm64 executable breaks strict validation: `codesign -v` says "main
executable failed strict validation" of a Rust bundle, as it will of a
Grasm one. The bundle still runs, started from a shell (tried 2026-11-13:
a Rust bundle, and a Grasm-linked `tirinterp` with bytes appended), and
writing a new bundle over an old one in place, as `fs::write` does, is safe,
because the signed part is the same bytes. Where Gatekeeper applies,
`spctl --assess --type execute` rejects a Rust bundle -- and a Grasm-linked
executable with nothing appended alike, since an ad hoc signature is not a
notarized one -- so a bundle downloaded with the quarantine attribute would
not open from Finder; launching a quarantined bundle was not tried. A
hardened runtime cannot be given to one: `codesign -s - -f --options
runtime` on a Rust bundle refuses ("main executable failed strict
validation") and leaves the linker's signature, flags `adhoc,linker-signed`,
and the copy still runs. Neither is new with the Grasm command; both are
the bundle's as the Rust command makes it.

Decision (2): **the bundled program's stack trace.** The bundle prints
what `grasm run` under the Grasm command prints, which leaves out a
panic's stack trace ([E-PANIC-TRACE]); `clidiff` strips it from the Rust
bundle's stderr as it does from `run`'s, and that is the one difference in
what the two bundles do.

Decision (3): **`--source` with `--native` on a runtime package.** Part one
made `build <runtime package> --native` write the runtime's object; it
answered before `--source`, so `build <runtime package> --native --source`
wrote an object where Rust says `no `main` function` (no case tried
it). `--source` is read first, so that line now
answers as Rust's does; `--native` without `--source` still builds the
runtime.

**How the output is held to Rust's** (decision (4)). `clidiff` gains four
sections, each both commands on the same input from the same directory,
counts asserted:

- `doc`: every corpus program, `doc P` -- stdout, stderr, status; every
  tenth, and every package's entry, also `doc P -o out.md` in a directory of
  each command's own: the line, the status, the file's bytes.
- `api`: every corpus program, `api P`; where that succeeds with a
  surface, `api P --against` that surface (a PATCH under both) and four
  baselines rewritten from it -- CR LF line ends, every tab a space, empty,
  a byte that is not UTF-8 before it; and, from a copy
  of the corpus, three changed copies against baselines the Rust command
  makes: `pub fn api_probe() -> i64` appended (against the original's
  surface, an addition), the original against the appended copy's surface
  (a removal), and `pub fn api_probe(n: i64) -> i64` appended against the
  first copy's surface (a change). Report, status, stderr.
- `bundle`: every corpus program with a `main`, `build P -o prog`: the
  output with the size taken out, stderr, status; the Grasm bundle must be
  the command's own executable followed by exactly the bytes that follow
  the Rust command's executable in the Rust bundle; both run, stdin closed,
  with arguments, and the same stdout and status, stderr the same less the
  Rust trace; the Grasm build made twice, the same bytes.
- `source`: every corpus program, `build P --source -o x.bundle.gr`: the
  output, the status and the file, byte for byte; every tenth file written
  also run by both commands.

The command-line cases lose the `elsewhere` tags of `doc`, `api` and the
two `build`s, and gain lines that reach each arm's own checks: `doc` with
no operand, with `-o` before the file, after `--`, into a directory that
does not exist; `api` with the operand after `--against`, `--against`
after `--`, a baseline that is missing, empty, not UTF-8, with CR LF line
ends, and one with no tab-separated line; `build` without `--native` into
a directory that does not exist, over a directory, on a program with no
`main` and on a runtime package; `--source` with `-o ""` and with no `-o`.
`clicontrols` gains a control for each: a doc line dropped (the doc text
of each item), an API change missed (a changed signature not reported), a
wrong bundle (the entry's name with its directory), and `--source`
writing the modules last first. (`--source` without the newline it adds
after a module was the plan; no corpus file lacks a final newline, so it
could not be caught.)

**Self-hosting.** The command grows by the renderer, the surface and the
bundle; `fixpoint` must still find its stage 3 its stage 2, and `clidiff`
runs again with stage 2's command, its bundles then carrying stage 2.

**Steps two to four, as built** (2026-11-13, while the main checkout ran
five Linux gates, so with light checks only; the corpus-wide gates are
step five's and were not run).

- `doc`: `compiler/grasm/src/docgen.gr`, the signature writers over front's
  AST, `collect`, `render_markdown` and the arm; `front`'s loader gains
  `enclosing_package_name`; gfmt's `ws_len` is public for the trim.
- `api`: `compiler/grasm/src/api.gr`; `front`'s `tys` gains
  `show_written`, a type written as it stands.
- `build` without `--native`, and `--source`: `compiler/grasm/src/bundle.gr`;
  `front` gains `checked_loaded` and `load_sources_shown` (the entry's
  shown name, `<doc example>` for `load_sources`); `build.gr` hands the two
  forms to it, and reads `--source` before building a runtime package.
  `main` asks `bundle.run_if_bundle` first. The executable's path moved
  from `build.gr` to `bundle.gr` (`own_exe`), which both use.
- `clidiff` gains the four sections (`doc`, `api`, `bundle`, `source`), with
  counts that must add up; their floors wait for a whole run. `--source`
  with an empty `-o` writes beside the source -- through the `lines`
  section's links that is the checkout itself, which a first trial did (a
  stray `examples/hello.bundle.gr`, removed) -- so that case, and the one
  without `-o`, are tried on a copy in the `source` section instead.
  `clicases.txt` goes from 195 cases to 237: four `elsewhere` lines of
  these subcommands dropped, a fifth kept without its tag, 46 added.
  `clicontrols` gains four controls: `doc-dropped` (each doc comment's
  last line dropped), `api-missed` (a changed signature not reported),
  `bundle-entry` (the entry named with its directory) and `source-order`
  (the modules written last first).
- Light checks, the command built by the Rust `grasm` (4,902,008 bytes; its
  `rt.o` 555,224 bytes, part two's): `clidiff` with `ONLY=lines,doc,api,
  bundle,source` on 21 programs (13 chosen for a `pub let`, an alias, an
  effect, an `impl`, an actor, a contract, an `extern fn`, a package with
  dependencies; and `hello`, `documented`, `testing_tour`, `fib`, two
  packages, a refused program, `E-PANIC-TRACE`) -- "lines: 237 the same:
  226 not here, as §6.17 says: 6 the runtime built: 1 a target refused:
  4"; "doc: 21 programs, 27 commands (6 written with -o, 379 items) the
  same: 27, 2 of them refusals"; "api: 21 programs, 125 commands (13 with a
  surface) the same: 125, 2 of them refusals against its own surface, a
  patch: 13 against a rewritten baseline: 52 an item added: 13, removed:
  13, changed: 13"; "bundle: 15 programs with a main the same output: 15
  ... the command and Rust's payload: 15 run alike: 15 built twice the
  same: 15"; "source: 21 programs the same output and file: 21, 6 of them
  refusals run from the file alike: 2 written beside the source alike: 2
  of 2". `clicontrols` with `CONTROLS=` the four: "controls: 4 caught: 4
  missed: 0". `grasm test` of the file in docs/known-issues.md's entry
  agrees under both commands (the TIR interpreter's `trim_start` takes the
  no-break space too). Starting the command costs the tail read only: ten
  `grasm version` in 0.026 s.

**Step five: the gates** (2026-11-13 and 14, this branch's tree, beside the
main checkout's Linux gates in the OrbStack machine, at one-minute loads of
4 to 32, so no time here is to be quoted).

- `clidiff` with the command stage 1 is, in three runs. The first whole run
  was stopped in its `api` section when free disk fell to 4.6 GB (swap had
  grown to 18 GB under the Linux gates; the run's own work directory held
  26 MB); its ten finished sections, every one the same: "lines: 237 the
  same: 226 not here, as §6.17 says: 6 the runtime built: 1 a target
  refused: 4"; "corpus: 1737 programs, 8685 commands ... the same: 8685,
  138 of them refusals"; "toml: 1474 files the same: 1474, 19 of them
  refused by both"; "run: 651 programs with a main the same: 651 timed
  out: 0"; "fmt: 2010 files, each and four rewrites, 20100 commands the
  same: 20100 rewritten alike: 7649 refused alike: 386"; "lint: 2010
  files, 5427 commands (1407 with --only) the same: 5427 findings: 1001";
  "test: 1737 programs, 1961 commands (6 with a property test, 3 with a
  contract, 209 with --coverage) the same: 1960, 12 of them with a failing
  test the host's panic: 0 both timed out, --contracts: 1"; "test, main as
  a test: 651 programs the same: 649, 13 of them failing the host's panic:
  2"; "bench: 1737 programs, 1743 commands ... the same: 1743 timing lines
  in the format: 6"; "doc: 1737 programs, 3168 commands (1426 written with
  -o, 66515 items) the same: 3168, 71 of them refusals". The second, `ONLY=
  api,bundle,source,build,target`: "api: 1737 programs, 10793 commands
  (1132 with a surface) the same: 10793, 66 of them refusals against its
  own surface, a patch: 1132 against a rewritten baseline: 4528 an item
  added: 1132, removed: 1132, changed: 1132"; it was stopped in `bundle`,
  which ran each bundle with arguments from the checkout: a program that
  takes an argument for a path to write wrote `a/`, `b c/` and a file
  `--` into the worktree (removed; the main checkout gained nothing), and
  `projects/intermediate/03_kv_store.gr`, given the directory `a` as its
  store, met the `IoFailed` wording the engines disagree on
  (docs/known-issues.md, open S1, §6.17). So a bundle now runs as `run`
  runs, from the checkout with no argument, and then with arguments from
  an empty directory of each command's own, whose files are compared; and
  a run differing only in that wording is counted as such, never hidden.
  The third, `ONLY=bundle,source,build,target`, 159 minutes: "bundle: 651
  programs with a main the same output: 651, 0 of them refusals the
  command and Rust's payload: 651 run alike, plain and with arguments: 651
  (0 run(s) differing only in the IoFailed wording) built twice the same:
  651"; "source: 1737 programs the same output and file: 1737, 1086 of them
  refusals run from the file alike: 66 written beside the source alike: 2
  of 2"; "build: 648 programs with a main (3 link a system library) the
  same output: 648, 0 of them refusals run alike: 648 built twice the same:
  648"; "target: 65 programs ... 195 of 195".
- The floors of the four sections, set from those runs a margin below
  (the agent's, at the coordinator's request): `doc` 1,300 documents
  written, 60,000 items, 60 refusals (first run); `api` 1,050 surfaces and
  1,050 each of items added, removed and changed, 55 refusals (second
  run); `bundle` 640 run alike and none in the `IoFailed` wording (third
  run); `source` 1,000 refusals and 60 run from the file (third run).
- `clicontrols`: "controls: 18 caught: 18 missed: 0", 76 minutes.
- `fixpoint`: "programs: 6 and the grasm command artifacts per stage: 16
  and 2 compared: 54 ... identical: 54 differ: 0"; "stage 2's TIR the same
  as grasm tir's: 6 of 6"; "the command's build the stage pipeline's (its
  rt.o and the 4 programs): 5 of 5". The command's stage 3 is its stage 2:
  `grasm` 4,372,312 bytes (4,322,680 at §6.19), SHA-256
  `a6b22601d1c528557baec54a253c61c8f5a8f36761553514e6c4d3138f9c757a`, and
  `rt.o` 555,224 bytes, the compiler's stage-3 `rt.o` to the byte. 23
  minutes.
- `clidiff` with stage 2's command (`fixpoint`'s kept stage, built by stage
  1's command alone), the whole run with every floor in force, passing, 7
  hours 25 minutes: the same lines as the stage-1 runs, section for
  section -- "lines: 237 the same: 226 not here, as §6.17 says: 6 the
  runtime built: 1 a target refused: 4"; "corpus: 1737 programs, 8685
  commands ... the same: 8685, 138 of them refusals"; "toml: 1474 files the
  same: 1474, 19 of them refused by both"; "run: 651 programs with a main
  the same: 651 timed out: 0"; "fmt: 2010 files, ... 20100 commands the
  same: 20100 rewritten alike: 7649 refused alike: 386"; "lint: 2010 files,
  5427 commands (1407 with --only) the same: 5427 findings: 1001"; "test:
  1737 programs, 1961 commands ... the same: 1960, 12 of them with a
  failing test the host's panic: 0 both timed out, --contracts: 1"; "test,
  main as a test: 651 programs the same: 649, 13 of them failing the
  host's panic: 2"; "bench: ... the same: 1743 timing lines in the format:
  6"; "doc: 1737 programs, 3168 commands (1426 written with -o, 66515
  items) the same: 3168, 71 of them refusals"; "api: 1737 programs, 10793
  commands (1132 with a surface) the same: 10793, 66 of them refusals ...
  a patch: 1132 against a rewritten baseline: 4528 an item added: 1132,
  removed: 1132, changed: 1132"; "bundle: 651 programs with a main the same
  output: 651, 0 of them refusals the command and Rust's payload: 651 run
  alike, plain and with arguments: 651 (0 run(s) differing only in the
  IoFailed wording) built twice the same: 651"; "source: 1737 programs the
  same output and file: 1737, 1086 of them refusals run from the file
  alike: 66 written beside the source alike: 2 of 2"; "build: 648 ... run
  alike: 648 built twice the same: 648"; "target: 65 programs ... 195 of
  195"; "the Grasm command's package calls proc_run or proc_pipeline: 0
  time(s)". Its bundles carry stage 2 itself.
- `gatecheck` "gate helpers checked: 587 undefined: 0 corpus copied: 0 grep
  -q under pipefail: 0 gates not executable: 0"; `fmttest` "formatted: 2004
  unformatted: 0 does not parse: 5 exempt: 1"; `linttest` "linted: 2005
  findings: 1001 does not parse: 5 broke: 0"; `gfmttest` "checked: 1533
  disagreeing: 0". `fmttest` and `linttest` read `GRASM_BIN`, not `GRASM`:
  run first without it, each built a Rust `grasm` into the worktree's
  `target/` with cargo and used that; the `target/` was removed and both
  were run again with `GRASM_BIN` naming stage 0, with the same lines.
- The litter check, against lists taken before the first gate: the
  worktree's files git does not track and its top level, "new untracked
  files: 0", new top-level entries 0; the main checkout's, the same, 0 and
  0. The `run` and `bundle` sections' conformance programs and demos leave
  a git-ignored `target/` (`demo-out`, `grasm_bytes_conformance`,
  `grasm_fs_conformance`) in the worktree each time, as §6.19 found; it was
  removed after each run.

**What is left for S8 after this part.** `deps`, `publish`, `add`, `lsp`,
`dap` and `journal`; `test --schedules` and `--schedule-seed` for a test
that starts tasks, a panic inside a `concurrent` task under `test`, one the
runtime's channel operations raise, and the interpreter's own stack, as
§6.19 left them; the `IoFailed` wording the engines disagree on, which a
bundle shows as `run` does; the LSP on the query engine, DAP, packages,
journal and replay, the wasm32 backend and one TOML reader. The gates of
§6.14's last list that run through the `grasm` command still take the Rust
one.

### 6.21 S8 part four: `deps`, `add`, `publish`, and one TOML reader

Written 2026-11-14, before any code, as the design to build to. Parts one to
three (§6.17, §6.19, §6.20) left `deps`, `add`, `publish`, `lsp`, `dap` and
`journal` answering "not in the Grasm toolchain yet", and two TOML readers
in the Grasm toolchain where S8 is to leave one (§6, S8). This part takes the
three package subcommands into `compiler/grasm`, held to the Rust command as
the others are -- the same reading of the command line (`prevalidate`
already reads all three), stdout, stderr, exit status, and every file
written, the lockfile and a registry's contents among them -- and settles
the reader. The Rust toolchain stays stage 0 and the oracle and is not
changed; where its behaviour is wrong it is reproduced and recorded in
docs/known-issues.md.

**What a registry is, and what touches the network.** Nothing.
`grasm_pkg` depends on `grasm_sema` and `grasm_syntax` alone and opens no
socket; a registry is a directory, `<root>/<name>/<version>/grasm.toml` and
the package's sources beside it ("deliberately a directory rather than a
service"). Its root is `--registry <dir>` (publish only), else
`GRASM_REGISTRY`, else the nearest `.grasm/registry` found walking up from
the package's canonical directory (`Registry::discover`). So the gates need
no network and no mirror: they use registries in scratch, made by `publish`
itself and by `scripts/lib/regcases.py` (to be written), and a copy of the
tracked `examples/registry/.grasm/registry`; they name the root with
`--registry` or `GRASM_REGISTRY`, and assert before they run that no
`.grasm/registry` exists above their work directory and that
`GRASM_REGISTRY` is unset in the environment they inherit (on this machine
neither exists, checked 2026-11-14), so the walk cannot find a registry the
gate did not make.

**What the Rust arms do** (`crates/grasm/src/main.rs` `cmd_deps`,
`cmd_add`, `cmd_publish`; `crates/grasm_pkg`):

- `deps [dir] [--locked]`. The directory is `first_operand` (else `.`),
  `--locked` anywhere. `resolve(dir)`: the root's manifest
  (`Manifest::load`, its errors prefixed with the tidied path), minimal
  version selection over the registry (each name gets the lowest version it
  holds at or above the highest floor anyone states, then every requirement
  checked, a conflict naming each requirer), then a depth-first walk giving
  the packages dependencies first, refusing a cycle, a missing manifest, a
  dependency whose manifest names another package, a version a requirement
  refuses, and one name reached at two directories. Then on stdout `<root>
  <version> — N package(s) in the graph`, and per package `  {name:<16}
  performs [..]  permits [..]` -- what its own sources perform
  (`effects_of_package`: the entry and every other `.gr` file under the
  package, each loaded and checked as a program; a failure is `  {name:<16}
  CANNOT ANALYZE`, worded by `first_failure`, "(reached through a
  dependency)" when the failing file is a dependency's) together with what
  it performs in the root's whole program (`effects_by_package`), and the
  ceiling or `(unrestricted)`. Then, in this order, each a reason to stop
  with exit 1 and the lockfile untouched: packages that could not be
  analysed (`error: cannot analyze ...` and a two-line help), capability
  names that are no effect (`check_capability_names`, the nearest real name
  by edit distance within `max(2, len/3)` as "did you mean"), and ceilings
  exceeded (`CapabilityViolation` and `ceiling_help`'s `performed: ... at
  ...`), the last line `error: N package(s) ...` from `main`. Then the
  lock: per package its name, version, SHA-256 over `grasm.toml\0<manifest>\0`
  and every `.gr` file under the package as `<relative path>\0<bytes>\0`,
  sorted as `PathBuf` sorts (by component, not by byte: `a/b.gr` before
  `a-b.gr`), and its ceiling; rendered as `[[package]]` tables under a
  two-line header, strings quoted by `toml::quote`. Against an existing
  `grasm.lock` (read by the same TOML reader, `parse_lock`; a file it
  refuses is one change, "unreadable"): per package in the graph's order,
  not in the lockfile, or its version, its contents (the two checksums'
  first twelve bytes) and its capabilities changed; then each package the
  lockfile has and the graph does not. `--locked`: no lockfile is an
  error; any change is `error: <change>` per change, a help, and `N
  change(s) since grasm.lock`; none is `grasm.lock is up to date` with
  nothing written. Without it each change is `  note: <change>` and the
  lockfile is written whole (`fs::write`, so its mode is the umask's) and
  `wrote <dir>/grasm.lock` printed (`./grasm.lock` for `.`).
- `add <name> <version> [dir]`. Name and version are the two arguments
  after `add` whatever they are (`grasm add --registry r x` takes
  `--registry` as the name); the directory is the third unless it starts
  with `--`. `--registry` passes the command-line check and is never read:
  `add` touches no registry. `VersionReq::parse` (trimmed; `=` exact, `^` or bare
  compatible), the name an identifier, the manifest read
  (`cannot read <dir>/grasm.toml: <io error>`) and parsed -- its errors
  *without* the path prefix `Manifest::load` adds -- and a dependency of
  that name refused ("already a dependency, at ^1.0.0" or "by path"). Then
  the text appended as written: a newline if the file lacks one, a blank
  line, `[dependencies.<name>]`, `version = "<req as displayed>"`
  (`+01.2.3` becomes `^1.2.3`). Prints `added `<name>` <req> to <package>`
  and `  run `grasm deps` to resolve it`.
- `publish [dir] [--registry R]`. The registry first (missing: "no
  registry found" and how to make one). Then `Manifest::load`, the version,
  the entry file existing, `capabilities` present (publish is the one place
  it is required), the package within its own ceiling (`effects_of_package`
  against it, "refusing to publish a package that exceeds its own
  ceiling"), no `path` dependency, and the version not already there
  ("a published version is immutable -- publish x.y.(z+1) instead"). Then
  `create_dir_all(<root>/<name>/<version>)`, the manifest and every `.gr`
  file under the package copied into it with their relative paths
  (`fs::copy`, which keeps each file's permission bits), and `published
  <name> <version> — N file(s) to <dir>`.

**What Grasm already has.** `front`'s loader reads and checks a manifest as
`Manifest::load` does (`load_manifest`, held by `diagdiff`'s 67 package
cases), and resolves the graph as `resolve` does, the registry with it
(`resolve_graph`, `select_versions`, `visit`, `discover_registry`,
`registry_versions`); `ceiling.gr` has `effects_of_package`,
`effects_by_package` and `effect_sites_of_package` for the capability
ceilings `check` enforces, and `source_files`; `crypto_hash.sha256_of`
(which `compiler/link` already uses for code signatures) is SHA-256;
`bundle.gr` calls `chmod(2)` and `build.gr` `stat(2)`.

**What has to be written**, and where:

- `compiler/grasm/src/pkg.gr`: `cmd_deps`, `cmd_add`, `cmd_publish`; the
  lockfile (`build_lock`, `render_lock`, `parse_lock`, `diff_lock` and the
  changes' wording); the checksum; `effect_vocabulary`,
  `check_capability_names` and the edit distance over characters; the copy
  into a registry, each file's mode carried over by `stat` and `chmod`, as
  `fs::copy` carries it.
- `compiler/front`: `ceiling.gr`'s three functions public, and
  `effects_of_package`'s reason worded as `first_failure` words it (`check`
  prints none of it, so it was never needed: §6.13); `source_files` sorted
  by component and its walk deduplicated by canonical directory, as
  `collect_sources` is (it now uses the path as written, which a symbolic
  link loop would defeat); `Manifest` and `resolve_graph` public to the
  command.
- `main.gr`: the three out of `elsewhere`, into the dispatch and the help
  line. `lsp`, `dap` and `journal` stay there.

What will differ, before a line is written: nothing intended. The two places
to measure rather than assume are the ones where `deps` runs code the
command has not run before on the whole corpus -- `effects_of_package` on
every package in a graph (the `check` path calls it only for packages with
a ceiling) and `effects_by_package` on a root that does not check (the
Rust one analyses the parsed program; the Grasm one needs the checker's
rows) -- and the time: the Rust `deps compiler/grasm`, 19 packages, took
1 minute 49 seconds (once, 2026-11-14, nothing else running; it loads and
checks every source file of every package as a program of its own).

**The TOML decision, measured.** Three readers of manifests are in the Grasm
toolchain today: `rtoml.gr`, the port of `grasm_pkg::toml` the loader reads
manifests with and `grasm toml` prints; the loader's own line-by-line
questions (a dependency's path, the entry, `runtime`), ports of the Rust
loader's (`loader.rs` reads a manifest a second way, line by line); and
`packages/toml`, which reads TOML itself and serves `front toml`. Both
candidates were run, with the Rust reader (`grasm toml`) and `tomllib`
(`scripts/lib/tomldump.py`), over every `.toml` file git tracks, the 102
manifests `diagcases.py` writes, and 1,500 manifests from a new generator,
`scripts/lib/tomlfuzz.py` (seed 1): a third in the dialect the Rust reader
documents, a third valid TOML outside it (floats, inline tables, arrays
across lines, literal and multi-line strings, `\u` escapes, hex and `_`
integers, dates, dotted and quoted keys and headers), a third broken.
`=` is the same bytes; `~` the same with a refusal's words taken out, as
`tomldiff` compares.

| input | files | Rust ~ tomllib | rtoml = Rust | rtoml ~ tomllib | packages/toml ~ tomllib | packages/toml ~ Rust |
|---|---:|---:|---:|---:|---:|---:|
| tracked `.toml` | 1,474 | 1,455 | 1,474 | 1,455 | 1,473 | 1,456 |
| `diagcases.py` manifests | 102 | 100 | 102 | 100 | 97 | 97 |
| fuzzed, dialect | 500 | 478 | 500 | 478 | 434 | 455 |
| fuzzed, valid TOML | 500 | 229 | 500 | 229 | 215 | 186 |
| fuzzed, broken | 500 | 344 | 500 | 344 | 278 | 386 |
| fuzzed, all | 1,500 | 1,051 | 1,500 | 1,051 | 927 | 1,027 |

What it says:

- `rtoml.gr` is the Rust reader on every file, refusals' words included
  (3,076 of 3,076). It agrees with `tomllib` exactly where the Rust reader
  does.
- `packages/toml` is no TOML reader to hold anything to: on the fuzzed
  manifests it accepts 388 files `tomllib` refuses (a key or a table twice,
  an array item that is a bare word, `[[bin]`), refuses 92 it accepts
  (55 arrays across lines, 31 multi-line strings, five `\b`, `\f` or `\U`
  escapes, an inline table across lines) and reads 93 otherwise. It agrees on the tracked
  manifests only because they are tame.
- **The Rust reader is not TOML either, and that is the decision.** It
  disagrees with `tomllib` on 449 of the 1,500 fuzzed manifests: it
  refuses 216 that are valid TOML (206 outside its dialect, by design --
  "anything outside that is a clear error" -- and 10 strings with an
  escaped quote, below), and it misreads 233 -- 156 that are not TOML it
  accepts, 77 that are TOML it reads to other values. Each class,
  reproduced 2026-11-14: `k = "A"` is `A`, not `A` (no `\u`,
  `\U`, `\b`, `\f` or `\r`); `k = "a\"#b"` is `a\` -- the escaped quote
  ends the string for the comment cut, so a valid string is silently
  truncated -- and `k = ["a\", \"b"]` is refused for the same reason;
  `k = "a" "b"` is `a" "b`; `a key = 1` is a key `a key`; `k = 007` is 7;
  `[dependencies.]` is a table; `[a."b.c"]` is the table `a."b.c"` and
  `'lit' = 1` the key `'lit'`; `k = """x"""` is `""x""`. In the
  repository only the 19 `Cargo.toml` files fall outside (valid TOML, an
  inline table or an array across lines, refused); among `diagdiff`'s cases,
  `dep-empty-name` (`[dependencies.]`, accepted, then refused as an unknown
  table) and `toml-float`.

So no reader can be both what S8 asks of the one that stays -- "every
manifest in the repository, and fuzzed ones, read by it and by `tomllib`,
agreeing" -- and what every gate that runs the Grasm command asks of it --
the Rust command's bytes, `grasm toml`, `check`'s manifest diagnostics,
`deps` -- because on almost a third of the fuzzed manifests those two answers
differ. One of them gives way on exactly those files:

- **(A) `rtoml.gr` promoted into `packages/toml`.** The package's reader
  becomes the Rust reader, line for line, with an accessor API for a
  program's configuration over it; `rtoml.gr` and the reader `packages/toml`
  has now are deleted; the loader, `grasm toml`, `front toml` and the
  lockfile read through it. Every gate holds Rust's bytes as now, and
  `diagdiff`, `loaddiff` and `clidiff`'s `toml` are unchanged by
  construction. `tomldiff` keeps the tracked manifests three ways (all
  agree), and over the fuzzed ones holds the reader to the Rust reader byte
  for byte, refusals included, and to `tomllib` wherever the Rust reader
  agrees with it; the rest it counts by class -- refused valid TOML,
  accepted non-TOML, read otherwise -- and the misreadings go into
  docs/known-issues.md as the Rust reader's, reproduced, to be fixed in the
  one reader when the oracle is gone (S9), as §6.13 does with the checker's.
  Costs: `packages/toml` stops reading floats, inline tables, arrays across
  lines, literal strings, dates and `\u` escapes -- its only users are
  `front toml` and `packages/toml-demo`, whose program is rewritten to the
  dialect -- and S8's sentence holds on the fuzzed manifests only where the
  Rust reader is right.
- **(B) `packages/toml`, made a TOML reader, in the loader.** It is
  rewritten to agree with `tomllib` on every fuzzed file, and the manifest
  layer refuses what the dialect leaves out in the Rust reader's words
  (`line N: cannot parse value `...``, `unterminated array`), from each
  value's line and written text. S8's sentence holds to the letter. Costs:
  where the Rust reader misreads (233 of the 1,500), the Grasm command
  answers as TOML does and not as Rust does, which the owner's rule --
  reproduce Rust's wrong behaviour and record it -- forbids unless waived
  for manifests; `diagdiff`'s `dep-empty-name` changes; and reproducing
  the Rust reader's *refusal* of a valid file exactly needs its line
  algorithm (the `"a\"#b"` case is a refusal of a valid file decided by
  the comment cut), so either some refusals' words differ too or that
  algorithm survives as a check, which is a second reader by another name.

**Decided: (A)**, by the owner, 2026-11-14 (recommended by the agent).
`tomldiff` holds the one reader to the Rust reader byte for byte on every
file and to `tomllib` wherever the Rust reader agrees with `tomllib`, and
counts the rest by how they disagree and by a recorded cause
(`scripts/lib/tomlclass.py`), asserting the counts at the default size and
seed so that a new way to disagree fails the gate; each of the Rust
reader's misreadings is in docs/known-issues.md with its reproducer and
severity, to be fixed at the S9 cutover. The loader's line-by-line
questions go through the one reader, and a program whose answer changes is
reported. The measurements above are the generator as committed with the
reader (one key in ten set twice in the dialect's third, where the first
draft set every fourth or so twice and saw fewer files read).

The agent's reasons, as given before the decision: It is the only one
that keeps the owner's rule that the Rust toolchain's wrong behaviour is
reproduced while it is the oracle, and with it every gate unchanged; and
the thing (B) would gain -- a reader that agrees with `tomllib` -- is worth
having only once nothing holds the toolchain to a reader that does not,
which is S9: (A) leaves one reader whose every departure from TOML is
counted by `tomldiff` and recorded, to be fixed in one place on the day the
Rust reader goes.

One more reader goes under either: the loader's line-by-line questions read
a manifest as the Rust loader does, which is not as `grasm_pkg::toml` does
(a `#` inside a quoted path cuts it; no escape is undone; the first `entry =`
anywhere is the entry, an `[[bin]]` entry included). Routed through the one
reader, they answer as the Rust command's other reader answers: the same on
every manifest both readers read alike, and on the rest the Rust toolchain
disagrees with itself. To be measured by `loaddiff` and `diagdiff`, not
assumed; if one of their programs moves, it is reported, not hidden.

**How the output is held to Rust's.** `clidiff` gains three sections, each
both commands on the same input from directories of their own, the counts
asserted and floors set from a first whole run:

- `deps`: every package in the repository, in a copy of the checkout's
  packages (a lockfile is written into each): `deps P` with no lockfile,
  again over the lockfile it wrote, `deps --locked P`, then with the
  lockfile edited each way `diff_lock` names -- a package's checksum, its
  version, its capabilities (to none and to `[]`), an entry removed, one
  added, a file the reader refuses -- and with a source byte changed, each
  plain and `--locked`. stdout, stderr, status and the lockfile's bytes.
  The tracked lockfiles (`apps/apigateway`, `examples/jsonlib`,
  `examples/registry`, `examples/workspace`) as they are.
- `add`: in copies of a sample of manifests and of `regcases.py`'s: a new
  dependency, one already there by version and by path, a name that is not
  an identifier and a keyword, versions bare, `^`, `=`, `+01.2.3` and
  malformed, a missing manifest, a manifest that does not parse, one with no
  final newline, one with CR LF. The output and the manifest's bytes.
- `publish`: every package in the copy, into an empty registry by
  `--registry`, by `GRASM_REGISTRY` and by a `.grasm/registry` above it,
  then again (immutable); and `regcases.py`'s packages for each refusal (no
  capabilities, over its ceiling, a path dependency, no entry file, a
  version not `MAJOR.MINOR.PATCH`, no registry). The output, and the two
  registries' trees: every path, every file's bytes and mode.
- Resolution against a registry: `regcases.py` writes package graphs over
  registries in scratch -- the lowest version above the highest floor, a
  conflict, a version missing, a name missing, a dependency of a registry
  package, exact requirements, `0.0.x` and `0.x` ranges, a version
  directory that is not a version, a `README` beside the versions -- and
  `deps` runs over each, so the registry's reading is held too, not only
  `publish`'s writing.

`tomldiff` gains the fuzzed manifests (`tomlfuzz.py`, its counts asserted)
as the decision says. `clicases.txt` loses the `elsewhere` tags of the three
and gains lines reaching each arm's checks. `clicontrols` gains four: a
lockfile's entries written in another order (`deps`), a package dropped
from the graph without a word (`deps`), `publish` writing into
`<root>/<version>/<name>`, and for the reader, a key set twice keeping the
first value (under (A); under (B) an escape decoded otherwise).

**Self-hosting.** The command grows by the lockfile and the registry;
`fixpoint` must still find its stage 3 its stage 2, and `clidiff` runs again
with stage 2's command.

**Steps two to four, as built** (2026-11-14).

- **One reader** (step two). `packages/toml` is `rtoml.gr`'s reader, the
  Rust toolchain's line for line, with an accessor API over it: tables and
  arrays of tables by index (`table`, `get`, `subtables`, `entries`,
  `entry_get`), values by dotted path (`text_of`, `int_of`, `bool_of`,
  `texts_of`, `keys_under`), `render_value`, and `quote` as the lockfile
  writes; version 2.0.0, its API being new. `rtoml.gr` is deleted, and the
  reader `packages/toml` had with it. The loader, the Grasm command's `grasm
  toml` (now `front.cmd_toml`, which `front toml` runs too) and the lockfile
  read through it; so do the loader's line-by-line questions -- a
  dependency's path, the entry, `runtime` -- which asked the manifest's text
  as the Rust loader does (the coordinator accepted the plan). No program of
  `loaddiff`'s or `diagdiff`'s moved (below); the one program known to
  answer otherwise is the reproducer in docs/known-issues.md, a dependency's
  path with a `#` in it, which the Rust loader cuts and the Grasm one does
  not. `packages/toml-demo` is rewritten to the dialect: its first document
  had a float (`ratio = 1.5`), an underscored integer (`big = 1_000_000`), a
  literal string (`entry = 'src/main.gr'`), a date, a nested array and an
  inline table, each now refused -- it shows them refused, with the Rust
  reader's words -- and the escape `ü`, which the reader keeps as
  written. `tomldiff` is as decided above; `tomlclass.py` names the causes;
  the counts at the default size and seed are in `tomldiff.sh` and
  `tomlclass.expected`. The two controls that planted into the old readers
  plant into `packages/toml`: `diagcontrols`' `manifest-words` and
  `tircontrols`' `toml-entries`.
- **`deps`, `add`, `publish`** (step three): `compiler/grasm/src/pkg.gr`,
  about 640 lines, as the design above says. `front`'s ceilings gained
  `whole_program_effects`, made `effects_of_package`,
  `effects_by_package` and `effect_sites_of_package` public, word a
  package that cannot be analysed as `first_failure` does, and walk a
  package's sources as `source_files` does -- each real directory once,
  sorted by component -- a walk the ceilings `check` enforces now share.
  The command depends on `packages/toml` and `crypto_hash` (the
  lockfile's SHA-256).
- **The gates** (step four): `clidiff`'s four sections; `clicases.txt`
  without the `elsewhere` tags of the three and with 24 new lines (261
  cases); `regcases.py` (24 cases, 129 commands, 14 lockfile edits from
  `lockedit.py` between them); `clicontrols`' four controls, as above.

**What the gates found** (2026-11-14 and 15, this branch's tree, nothing
else running on the machine; the stage-1 command built by the Rust
`grasm`):

- `tomldiff`: "manifests: 1450 read alike by all three: 1450 (9599 keys)
  refused by all three: 0 disagree: 0"; "fuzzed: 1500 (seed 1) the Rust
  reader's bytes: 1500 read as tomllib reads them: 1051, 577 of them
  refused by both"; "where the Rust reader is not tomllib: refuses-valid
  216 accepts-invalid 156 reads-otherwise 77 unexplained: 0", the causes'
  counts the ones `tomlclass.expected` records.
- `loaddiff` "programs: 1737 same modules: 1671 (5403 modules; 321 with a
  package's own import) loaded, then refused: 64 unloadable by both: 2
  disagree: 0"; `diagdiff` "programs: 4280 ... refused with the same
  output: 1437 (conformance: 66) accepted by both: 2843 differ: 0 Rust
  panics: 0", 114,791 diagnostics and 36,163 help lines compared, the 67
  package cases the same. Both are §6.19's counts, but 2,843 accepted where
  §6.19 had 2,840 (the checkout holds three more `.gr` files): routing the
  loader's questions through the one reader moved no program.
- `diagcontrols` "controls: 9 caught: 9 missed: 0" (`manifest-words` now
  planted in `packages/toml`); `tircontrols` "controls: 8 caught: 8 missed:
  0" (`toml-entries` likewise).
- `clicontrols` "controls: 22 caught: 22 missed: 0", 92 minutes: §6.20's
  eighteen and `lock-order`, `dep-dropped` (each "registry
  lowest-at-floor: grasm deps app ... the files differ"), `publish-where`
  ("registry publish: grasm publish pkg --registry reg") and `toml-first`
  ("registry lockfile: grasm deps app --locked -- exit 1 (Rust) and 0
  (Grasm)").
- `fixpoint`: "programs: 6 and the grasm command artifacts per stage: 16
  and 2 compared: 54 ... identical: 54 differ: 0"; "stage 2's TIR the same
  as grasm tir's: 6 of 6"; "the command's build the stage pipeline's (its
  rt.o and the 4 programs): 5 of 5". The command's stage 3 is its stage 2:
  `grasm` 4,405,384 bytes (4,372,312 at §6.20), SHA-256
  `84c5a7aa6b6e4b57cdd1e3d05a2e1af01ea71eb1a9e6728d9a1677c4139a8cb2`, and
  `rt.o` 555,224 bytes. 24 minutes.
- `clidiff` with the command stage 1 is: the first whole run passed every
  section up to `publish` with no difference, and was stopped in
  `publish` after 11 hours, 500 packages in: it compared every byte of the
  growing registry after each package, which is quadratic. The section now
  holds each publish to the registry's paths and modes and to the bytes of
  what it wrote, and the whole registry's bytes once at the end; run again
  alone (`ONLY=publish`), 14 minutes: "publish: 1448 packages the same,
  registries and all: 1448, 635 of them published". The floors of the four
  new sections were then set a margin below these runs (the agent's):
  `registry` 120 commands and 60 refusals, `deps` 1,400 packages, lockfiles
  written and refusals, `add` 1,400, `publish` 600 published.
- `clidiff` with stage 2's command (`fixpoint`'s, built by stage 1's
  command alone), the whole run with every floor in force, passing, 559
  minutes: "lines: 261 the same: 253 not here, as §6.17 says: 3 the runtime
  built: 1 a target refused: 4"; "corpus: 1737 programs, 8685 commands ...
  the same: 8685, 138 of them refusals"; "toml: 1474 files the same: 1474,
  19 of them refused by both"; "run: 651 ... the same: 651 timed out: 0";
  "fmt: 2010 files, ... 20100 commands the same: 20100 rewritten alike:
  7648 refused alike: 387"; "lint: 2010 files, 5427 commands (1407 with
  --only) the same: 5427 findings: 993"; "test: 1737 programs, 1961
  commands ... the same: 1960, 12 of them with a failing test the host's
  panic: 0 both timed out, --contracts: 1"; "test, main as a test: 651
  programs the same: 649, 13 of them failing the host's panic: 2"; "bench:
  ... the same: 1743 timing lines in the format: 6"; "doc: 1737 programs,
  3168 commands (1426 written with -o, 66543 items) the same: 3168, 71 of
  them refusals"; "api: ... 10793 commands (1132 with a surface) the same:
  10793 ..."; "bundle: 651 ... the command and Rust's payload: 651 run
  alike, plain and with arguments: 651 ... built twice the same: 651";
  "source: 1737 programs the same output and file: 1737, 1086 of them
  refusals ..."; "build: 648 ... run alike: 648 built twice the same: 648";
  "target: 65 programs ... 195 of 195"; "registry: 129 commands (93 deps,
  23 add, 13 publish; 14 lockfile edits between them) the same, files and
  all: 129, 75 of them refusals"; "deps: 1448 packages, 4344 commands the
  same, lockfiles and all: 4344, 1447 of them refusals lockfiles written:
  1448"; "add: 1448 packages the same, manifests and all: 1448, 0 of them
  refusals"; "publish: 1448 packages the same, registries and all: 1448,
  635 of them published"; "the Grasm command's package calls proc_run or
  proc_pipeline: 0 time(s)". The stage-1 run's sections gave the same lines.
- `gatecheck` "gate helpers checked: 587 undefined: 0 ..."; `fmttest`
  "formatted: 2004 unformatted: 0 does not parse: 5 exempt: 1"; `gfmttest`
  "checked: 1533 disagreeing: 0"; `linttest` "linted: 2005 findings: 993
  does not parse: 5 broke: 0" (993 where §6.20 had 1,001: the deleted
  `rtoml.gr` and the old reader had findings).
- The litter check, against lists taken before the first gate: the
  worktree's and the main checkout's files git does not track, and their
  top levels: no new untracked file and no new top-level entry in either.
  Each gate's git-ignored `target/` was removed after it ran, and the
  `__pycache__` directories the controls' Python fuzzers leave under
  `scripts/` were removed at the end.
- Time: the Rust `deps compiler/grasm` took 1 minute 48 seconds and the
  Grasm one 3 minutes 38 seconds (once each, nothing else running, not to
  be quoted): every source file of each of 19 packages loaded and checked
  as a program of its own.

Not run on this branch: the rest of `allgates.sh`.

**What is left for S8 after this part.** `lsp`, `dap` and `journal`; the
rest as §6.20 left it -- `test --schedules` and `--schedule-seed` for a
test that starts tasks, a panic inside a `concurrent` task under `test`,
one the runtime's channel operations raise, the interpreter's own stack,
the `IoFailed` wording; the LSP on the query engine, DAP, journal and
replay, the wasm32 backend. The Rust TOML reader's misreadings, kept in the
one reader until S9 (docs/known-issues.md). Three tracked lockfiles
(`apps/apigateway`, `examples/jsonlib/app`, `examples/workspace/app`) are
stale as `examples/registry/app`'s was, and were left as they are.

### 6.22 S8 part five: `lsp`, `dap` and `journal`, with replay

Written 2026-11-15, before any code, as the design to build to. Parts one to
four (§6.17, §6.19-§6.21) left `lsp`, `dap` and `journal` answering "not in
the Grasm toolchain yet". This part takes the three into `compiler/grasm`,
held to the Rust command byte for byte: the same reading of the command line
(`prevalidate` already reads all three), the same bytes on stdout and stderr
in the same order, the same exit status, the same files written -- for the
two servers, every byte of every response to a fixed transcript of requests;
for the journal, every byte of the file a recording writes and of what a
replay prints. The Rust toolchain stays stage 0 and the oracle and is not
changed; where its behaviour is wrong it is reproduced and recorded in
docs/known-issues.md. The decisions are numbered (1) to (9); each was
proposed by the agent implementing this part and accepted by the
coordinator on 2026-11-15, with the conditions written under it, and the
owner may reverse any of it.

**No network, no sockets.** Neither Rust server can open a socket:
`grasm lsp` and `grasm dap` take no option and speak `Content-Length` framed
JSON on stdin and stdout only (`crates/grasm/src/main.rs`; the command line
check gives `lsp` and `dap` no flag). The Grasm servers do the same and
nothing more.

#### What the Rust side does

**The journal** is two things. *Recording and replay* is not a subcommand:
`GRASM_RECORD=<path>` or `GRASM_REPLAY=<path>` in the environment
(`crates/grasm_journal`; docs/replay.md), read once per thread, the first
time the thread asks, so under `grasm run` (and `test`, `bench`, `dap`,
which run the same interpreter) the first journaled call or C call decides
the mode. The Rust interpreter's hooks (`crates/grasm_interp/src/journal.rs`
and 60 call sites) cover the 40 builtins `builtins.rs` marks `Recorded` --
the file, clock, environment, argument, stdin, entropy, socket and process
surface -- each gated after its arguments are evaluated and before its
effect, and recorded after it; `net_close` records `()`, `file_exists` a
`bool`, and under replay the first C call stops the run. Values cross as
`Wire` (`GRASMJRNL2`, then per record a `u32` name length, the name, a `u32`
value length, the value: tag 0-8 for unit, `i64`, `f64` bits, bool, text,
bytes, list, tuple, variant with its type name), `Ok` and `Err` as variants
of type `!`. Each record is written to the file as it happens. The file is
created only when the first question is asked, so a program that reaches
nothing leaves no file, and a replay of a file that does not exist goes
unnoticed by a program that asks nothing (both checked 2026-11-15). Words
and status, all exit 7: `grasm: cannot record to <p>: <e>`,
`grasm: cannot replay <p>: <e>`, the journal "ran out at call N (`name`)",
"at call N the recording says `a` and this run asked for `b`", "this run
calls the C function `f`". A replay that ends with records unread says
nothing: `check_exhausted` exists and nothing calls it, though
docs/replay.md says a replay "says so" -- a defect of the Rust toolchain,
reproduced and recorded. *`grasm journal <file> [--full]`* prints a
recording: the operand is `args.get(1)` unless it starts with `--` (so
`journal --full j` is the usage, exit 1); the file read whole and parsed
(`grasm: cannot read <p>: <e>`, exit 1, `<e>` in `grasm_sys`'s words -- "No
such file or directory", "Is a directory", "not a Grasm journal", "journal
ends mid-record", a value's own errors); then `<p> — N record(s), B bytes`
(B from `metadata`, 0 if that fails), and either "The program read nothing
from outside itself." or one line per record -- two spaces, `{i:>places}`,
two, `{name:<width}`, two, the value -- (width in bytes of the longest name,
padded in characters, as `bench` pads), and a tally of calls per builtin in
byte order. A value is rendered within a budget of 48 characters, or none
with `--full`: text quoted with `\n`, `\r`, `\t` and `"` escaped and cut
after the budget with "… N more char(s)"; bytes as a count and `budget/3`
hex pairs; lists and tuples by their first three items, each with a third of
the budget; a variant by its name and fields; a float as Rust's `{}` writes
it; names and text decoded lossily.

**`lsp`** (`crates/grasm_lsp`, 708 lines and a 334-line JSON reader and
writer of its own). One loop: read a message (header lines to a blank one,
`Content-Length:` parsed as `usize` -- an unparsable value forgets an
earlier one -- the body read exactly and decoded lossily; a read error ends
the server with `lsp error: <e>`, exit 1; end of input ends it with 0); a
body that is not JSON is skipped; otherwise the method is handled and, if
the message had an `id` and the handler a result,
`{"id","jsonrpc","result"}` is written, then any notification. `initialize`
(the capabilities and `grasm-lsp` `1.1.0`), `shutdown` (null),
`textDocument/didOpen`, `didChange` (the last change's text: full sync) and
`didSave` (each a `publishDiagnostics`), `hover`, `definition`,
`completion`, `documentSymbol`; `exit` ends the loop only after `shutdown`;
every other method gets no answer at all. Diagnostics: for a `file://` URI,
the path after the prefix loaded with the buffer as the entry
(`load_from_disk_in`, no registry packages), the loader's diagnostics or
else the checker's, only those that start before the end of the first file,
each `message` (and `"\nhelp: " + help`), severity 1, source `grasm`, its
range in 0-based lines and *byte* columns; for any other URI, or a load that
fails, `analyze` -- the buffer parsed and, if it parsed, checked as one
module named `main` with no imports. Hover, definition, completion and
symbols parse the buffer alone and look only at its own declarations: a
function's signature, a type's body, an effect's operation count, a
variant's fields, with the `///` text under a rule; a builtin's one-line
help from a fixed table of eleven; completion's items are the file's
declarations, 27 keywords and those eleven builtins. A position is a line
and a character taken as a *byte* offset into the line, clamped to it. JSON
objects are written with their keys in byte order (`BTreeMap`), numbers as
`f64` (an integral one as an integer, saturating; any other as Rust's `{}`),
strings with `"`, `\`, `\n`, `\r`, `\t` escaped and other controls as
`\u00xx`; the reader takes `\u` four characters at a time (a surrogate is
U+FFFD, so a pair is two of them) and reports every failure in words of its
own.

**`dap`** (`crates/grasm_dap`, 879 lines, its own copy of the JSON code).
Phase one, before the program runs: requests answered in order with `seq`
counting every message read; `initialize` (eight capabilities, then an
`initialized` event), `launch` (the program loaded from disk and checked --
the loader's and the checker's diagnostics *together*, unlike `check` -- the
first one's message as "`p` has errors: m", or "cannot load `p`: e"; with
`"journal": "<path>"` that recording installed first, as a replay, "cannot
replay <path>: e" -- a failed launch is a failed response and a `terminated`
event, exit 0), `setBreakpoints` (held, every line reported verified),
`threads`, `configurationDone` (ends the phase; before a `launch`, "dap
error: configurationDone arrived before launch", exit 1), `disconnect` and
`terminate` (exit 0), anything else answered with a null body. A framing or
JSON error here is `dap error: <e>`, exit 1. Phase two runs `main` in the
reference interpreter with a hook before every statement: it stops when the
resume mode says (stop on entry steps in; a breakpoint is a file and a line;
`next` stops at the same depth or shallower, `stepOut` shallower), sends
`stopped` (reason `breakpoint` or `step`), and serves requests until one
resumes: `stackTrace` (frames innermost first, each `module::function`, the
innermost at the statement running, each outer one at the statement that was
running in it when the call was made, path and file name, line and byte
column), `scopes` (one, `Locals`, readable only for frame 0), `variables`
(every name in the innermost frame's scopes, the innermost binding of each,
sorted by name; the value as the interpreter prints it, a string as Rust's
`{:?}`; a type name of the value's kind), `evaluate` (a name looked up,
nothing more; else "no variable named `x`"), `setBreakpoints` (replaces that
file's), `threads`, `continue`, `next`, `stepIn`, `stepOut`, `disconnect`
and `terminate` (which stop the stopping, not the program: it runs on to its
end); end of input or a bad frame in this phase ends the stopping the same
way. The program's stdout becomes `output` events a line at a time (lossy
UTF-8), its stderr is the adapter's stderr, a panic adds one `stderr` output
event with the stack trace, and the run ends with `exited` (the status) and
`terminated`; exit 0. Tasks of a `concurrent` block run in interpreters of
their own with no debugger, so the Rust adapter never stops inside one.

#### What Rust answers on the corpus

Measured 2026-11-15 with the main checkout's `grasm` (the release of
2026-11-01), from the worktree's root, stdin `/dev/null` for runs, each
program once, at a one-minute load of about 5 from other work.

- **The journal**: the 651 corpus programs with a `main` (`clidiff`'s list),
  each run once under `GRASM_RECORD` with a 120-second limit (`clidiff`
  gives 1,800): 4 ran past it (`examples/parallel_map.gr`,
  `packages/b29-demo`, `crypto_hash-demo`, `password-demo`). Of the 647, 467
  left no file -- they asked the journal nothing -- 6 left a 10-byte file
  holding only the magic (they call C and nothing journaled: `termios-demo`
  and five `T-EXTERN*`/`L-LIB-CALL-C` conformance programs), 140 recorded
  `args` and nothing else, and 38 recorded more: 6,262 records and 1.7 MB in
  all, of 39 of the 40 journaled builtins (all but `udp_recv_from`), from
  `read_bytes_at` 3,317 times (`lsm-demo`) to `now_millis`, `input`,
  `sleep_millis` and `udp_send` once each. Exit statuses: 620 zero, 18
  panics (101), 8 one, 1 `exit(255)`. Each recording replayed by Rust: 176
  printed and exited as recorded; the 8 others stop with exit 7 at their
  first C call (`isatty`, `SDL_Init`, `call_c`, `strlen`, `sqrt`, `abs`), as
  designed. Recorded a second time, 175 of the 184 files were the same
  bytes; the 9 others differ only in the values of `now_millis`,
  `monotonic_millis`, `monotonic_nanos`, `random_bytes`, `net_port` and
  `tcp_peer` -- the builtins whose answer depends on when and where, and the
  only ones a comparison of two recordings may normalise.
- **`lsp`**: every tracked `.gr` file (2,010), one scripted session each
  (`initialize`, `didOpen`, `documentSymbol`, `hover` and `definition` at
  each `fn`/`type`/`effect` name and at 40 identifiers spread over the file,
  `completion`, a `didChange` adding a function with a type error and two
  hovers on it, a `didChange` dropping the last `}`, `didSave`, `shutdown`,
  `exit`): every session exited 0; 253,306 messages and 42 MB of responses,
  39,861 hovers with a result and 36,459 definitions, 2,348 diagnostics (the
  type-error probe in 2,009 files; in the one source bundle,
  `ProjectGold/36-markdown-editor/bundle.gr`, the probe is in its last
  module, past the first file, and is dropped). 277 seconds of the Rust
  server's time in all; the longest, 8.9 s, are `compiler/grasm`'s files,
  each change reloading the whole command. Every diagnostic of the unclosed
  `{` was dropped (below).
- **`dap`**: the 651 programs, each with a session that stops on entry and
  steps in 200 times, asking `threads`, `stackTrace`, `scopes` and
  `variables` at each stop, then ends its input; every fourth (211) also
  with a session of 100 stops cycling `next`, `stepIn`, `next`, `stepOut`,
  `stepIn`, `stepIn`, `next`, and one with a breakpoint on every third line
  and 50 `continue`s -- each session limited to 60 s (300 s for the first 17
  programs; the session costs a whole run, since the program runs on after
  the input ends). 1,073 sessions, 445,262 messages, 72,996 stops, 281,094
  variables shown -- `i64` 112,046, `list` 54,628, `str` 52,167, `struct`
  21,771, `f64` 11,660, `bytes` 11,248, `bool` 9,021, `map` 2,870, `variant`
  1,734, `ref` 1,514, `tuple` 797, `fn` 759, `ByteBuilder` 298, `StrBuilder`
  216, `result` 206, `()` 109, `range` 50, and no `u64` at all; frames up to
  101 deep; 42 sessions ending in a panic with its stack trace, 13 with
  status 1, one with 255. 3.3 GB of responses, 2.4 GB of it one session:
  `sieve`'s list of a million booleans rendered whole in the pane at each of
  200 stops. Seven sessions ran past the limit (`b29-demo`,
  `crypto_hash-demo`, `password-demo`), three launches failed
  (`examples/registry/app`, whose registry package the adapter's loader does
  not resolve, "cannot load module"), and one aborted: `E-PANIC-TRACE`'s
  deliberate unbounded recursion overflows the Rust adapter's own stack --
  "thread '<unknown>' has overflowed its stack", SIGABRT, exit 134 -- where
  `grasm run` reports the panic, because `serve` does not run the
  interpreter on the deep stack `run` uses (a Rust defect, recorded). The
  Rust adapter's time: 982 s for the 651 stepping sessions, 1,237 s for the
  422 others, `examples/parallel_map.gr` alone 488 s.

#### What Grasm has, and what has to be written

What exists: `front`'s loader, parser and checker with Rust's diagnostics in
Rust's order and words (`diagdiff`, which compares the rendered diagnostics,
and the renderer keeps the checker's order), its AST node for node with the
`///` map (`asttest`, `doc`), `rdebug` (Rust's `{:?}` of a string), gfmt's
`char::is_whitespace`; `tirinterp`, which `run` runs in this process, with
`fault`/`exit_with` unwinding and the statement-to-AST map `testing.gr`
built for `--coverage` (`stmt_offsets`: each TIR block's children lined up
with the AST block of the same span); and the Grasm runtime's own journal
(`compiler/rt/src/journal.gr`), which records and replays native programs
and is held to the Rust runtime's by `jfuzz` and `rtswap`. What does not:
any JSON, any framing, a loader entry given in memory, a one-module check,
any journal in the interpreter, any debugger hook.

- **A JSON reader and writer and the framing**, ported from
  `grasm_lsp/src/json.rs` (identical in `grasm_dap` but for a comment) --
  not `packages/json`, whose errors, number writing, key order and escapes
  are its own -- with Rust's `str::parse::<f64>`, `usize`'s parse (a leading
  `+`), `char::is_whitespace`, `trim_end`, `read_line`'s UTF-8 check
  ("stream did not contain valid UTF-8"), `read_exact`'s "failed to fill
  whole buffer", `String::from_utf8`'s error words and `from_utf8_lossy`,
  and Rust's `{}` for a float (the language's `to_str` less a `.0`, as
  `float_text` adds it). In `tools/tirinterp/src/json.gr`, since the adapter
  runs inside the interpreter (below); the command's LSP imports it from
  there. Decision (8).
- **Standard input and output as bytes.** `input()` cannot read bytes, see
  the end of input, or keep a `\r`, and `print` cannot write a message
  without a newline after it, so the servers read fd 0 with `read(2)` and
  write fd 1 with `write(2)` through `extern fn`, as the command already
  calls `stat(2)` and `chmod(2)`, with a buffer of their own (decision (9)).
  Nothing else in the command writes to stdout while a server runs.
- **`lsp`**: `compiler/grasm/src/lsp.gr` -- the loop, the handlers, the
  signature writers (`render_fn_signature`, `render_type_decl`, `type_text`:
  the LSP's own forms, not `doc`'s), `word_at`, the two fixed tables -- over
  two additions to `front`: `load_with`'s entry text given in memory
  (`load_from_disk_in`) and a one-module check (`analyze`).
- **The journal**: `tools/tirinterp/src/journal.gr` -- `Wire`, its encoding
  and decoding with every error in Rust's words, the lazy mode
  (`GRASM_RECORD`, `GRASM_REPLAY`, chosen at the first question, the file
  created then), `gate`, `put`, `refuse_foreign`, `install` (for the
  adapter's `journal` argument) and `render`; the 40 builtins and the C call
  hooked in `builtins.gr`, `cabi.gr` and `ccall.gr` as the Rust interpreter
  hooks them -- after the arguments, before the effect -- with each
  builtin's answer converted to and from `Wire` in the shapes the Rust
  interpreter gives (an `IoError` variant of type `IoError`, `Option`, `!`,
  tuples). Decision (1). `compiler/grasm/src/journalcmd.gr` is
  `grasm journal`.
- **The runtime's exemption**: decision (2) below.
- **`dap`**: `tools/tirinterp/src/dap.gr` -- both phases, the session, the
  hook's stopping rule, the responses -- and the interpreter's debugger
  hooks, decisions (3) to (7). The command's `dap` arm loads and checks the
  program as the Rust one does (`front.load` with both diagnostic lists),
  builds the TIR and the statement map, and hands over.

#### The decisions

**(1) The program's journal is the TIR interpreter's.** The Rust interpreter
records and replays with hooks of its own over the shared format, and so
does `tirinterp`: it is the engine `run`, `test`, `bench` and now `dap` run,
its builtins are its own implementations, and only it knows which host
operations are the program's. Writing each record to the file as it happens,
as Rust does, so a run that panics or exits leaves the records before it.
The alternative -- letting the host runtime's journal record the interpreted
program, since `tirinterp`'s builtins call the host's -- was rejected: the
host makes calls the program did not (the interpreter's own reads), and the
program makes calls the host never sees (`args()`, answered from the command
line it was given).

**(2) A Grasm executable may be exempt from its own runtime's journal, and
the command is.** Measured 2026-11-15: under `GRASM_RECORD`, the Grasm
command (part four's, on the Rust runtime) wrote eleven records for a
program that makes two -- its own `file_size` and `read_bytes_at` of itself
(the bundle check), `args`, five `file_exists` and the source's `read_bytes`
-- and a native program built by the Grasm command records the same way on
the Grasm runtime. Under `GRASM_REPLAY` it would be worse: the command's own
first call would take the program's first record, and its first C call
(`stat`) would stop the run. No program can opt out today: the mode is read
from the environment at the first journaled call or C call, both of which
the command makes before it knows anything, and every C call checks it. So
`compiler/rt`'s journal (a runtime package; the language freeze does not
cover it) learns one thing: at the first question, before reading the
environment, it looks for an exported symbol `unjournaled_host` in the main
executable (`dlsym(RTLD_MAIN_ONLY)`) and, if it is there, the executable's
journal is off for good -- no record, no replay, no refusal of C calls. The
command defines it with `export fn` (not `grasm_...`: the checker keeps
names beginning `grasm_` for the runtime and the compiler). Tried
2026-11-15: a program exporting it and asking `dlsym(-5, ...)` finds it when
built by the Rust command; built by the Grasm command it does not link at
all -- "cannot export _unjournaled_host: it is not defined" -- because the
Grasm backend leaves out an `export fn` that no Grasm code calls, and the
link then refuses the export. That is a defect of the Grasm backend (an
exported function is a root), found here; no corpus program met it
(`clidiff`'s `build` builds all 648 alike). Fixed first, in its own commit (the coordinator's condition): every
`export fn` is a root of the program in `compiler/mir`, and `linktest`
builds `scripts/fixtures/link/export_uncalled.gr` with both backends and
runs both -- the function must be in the executable and answer 42 through
`dlsym` (with the old rule, "compiler/link failed: cannot export
_fixture_uncalled: it is not defined"). No corpus
program defines it, so `rtswap`, `jfuzz` and `replaytest` see nothing
change; Linux ELF executables are static and have no `dlsym`, and the
command does not run there yet, so the ELF runtime keeps today's rule until
it needs another way. What it costs: **the Rust runtime has no such rule**
and is not to be changed, so a command linked with it -- stage 1, as
`clidiff` and `fixpoint` build it -- journals itself under either variable.
The `journal` section of the gate therefore runs on commands linked with the
Grasm runtime: stage 2, and a stage 1 linked as `rtswap` links a Cranelift
object, `compiler/link -interpose rt.o`; every other section runs on stage 1
as built today. The alternatives: change the Rust runtime (refused by the
brief); give the program a builtin (a language change, frozen); name the
exemption in another environment variable (the person replaying sets
`GRASM_REPLAY`, not that); let the gate run the section on stage 2 alone
(weaker: stage 1 is the build of the command by the reference compiler).
A link flag that sets the bit was the less magic one and is not as cheap:
the command is built by `grasm build --native` (stage 1 by the Rust
command, stages 2 and 3 by the Grasm one, a release by whichever ships),
and a flag that only the Grasm command reads is a line the two commands
answer differently, while one the Rust command would have to pass cannot
be added; the exported name travels with the source and every build of it.

The symbol is a switch for debugging and replay, **not a security
boundary** (the coordinator's condition, written here and beside the check
in `compiler/rt`): any program can export it and keep its own effects out
of the journal. That is acceptable because the journal is not an audit log
-- it records what a cooperating program read, so a failure can be made to
happen again, and a program that wants to hide its effects can already do
so by calling C, which no journal records.

**(3) The adapter runs in the TIR interpreter, through a hook that does
nothing unless a debugger is attached.** As §6.17 and §6.19 put `run`,
`test` and `bench` there: the program's stdout is the adapter's to frame,
and the stops must happen in this process. The interpreter gains a debugger
state in its context (`cx.dbg`, off for `run`, `test` and `bench`): before
each block child that `stmt_offsets` maps to an AST statement, the
statement's start becomes the current position and the stopping rule is
asked; a stop serves the protocol from inside the evaluation, as the Rust
adapter serves it from inside its hook. A block whose children do not line
up with the AST's (none is expected; coverage relies on the same map, and
the count is measured at step three) refuses the launch rather than stepping
wrongly. `run` stays the same bytes: every change is behind `cx.dbg` or
`cx.jrnl`, and `tirinterp`, `frontsmoke` and `clidiff`'s `run` section hold
it.

**(4) The reference's frames and scopes, kept beside the interpreter's.**
The interpreter's frames are slot arrays; the reference's are a call stack
of `(module::name, call site)` and, per frame, a stack of name-to-value
scopes. Under the debugger the interpreter keeps the second shape as the
reference builds it: a frame pushed for every call of a declared function
(its name as TIR writes it, `module::name`, as the reference's
`module::function`; the call site the current statement when the call was
made -- the reference's `cur_span`, which a closure's statements also move
and only a declared function's return restores), a scope pushed for every
block, function body, closure call, `for` iteration and `match` arm, and a
binding recorded at each `let`, `let (..)`, parameter, loop variable and
pattern binding by its var id, its value read from the slot at the stop. A
closure's call scope is the reference's: the names its body mentions
(`mentioned_names`, ported over front's AST) bound where it was made, by
value, and its parameters. Two things the interpreter does for speed would
show through and are turned off under the debugger: moving a value out of a
slot at its last use (the reference keeps the binding, so the pane shows it)
and any slot reuse that would make one name show another's value. A `ref`
shows as `ref#slot.generation`, so the interpreter's slab hands out slots
and generations in the reference's order (1,514 `ref` values in the measured
panes). Which value kinds the pane names (`i64`, `u64`, `str`, `list`,
`map`, `struct`, `variant`, `result`, `fn`, `range`, ...) is decided by the
reference's representation, not the static type: `let x: u8 = 5` is an `i64`
there, and a `u64` is `u64` only where the reference made a `U64`; the rules
are taken from `grasm_interp` and measured on the corpus's transcripts (no
variable of type `u64` in them).

**(5) The program's stdin under the adapter is the protocol.** The Rust
adapter reads requests through a `BufReader` over `stdin()`, and the
program's `input()` reads the same process stdin through its own lock, so
what a program that reads stdin sees under `dap` depends on two buffers'
sizes. The Grasm adapter answers `input()` from the protocol stream the same
way only by accident, so the three corpus programs that call `input()`
(`packages/bindgen-demo`, `L-LIB-IO`, `T-EFFECT-BUILTIN`) are left out of
the sessions, and the difference is recorded, not designed.

**(6) `concurrent` tasks do not stop, as in the reference.** The Rust
interpreter runs a `concurrent` block's tasks in task interpreters with no
debugger; the TIR interpreter runs them as host tasks on copies of its
context, which carry the debugger switched off. A task's prints reach the
adapter as the reference's do (collected, then written to the program's
output when the reference writes them), measured on the four corpus programs
with a `concurrent` block in their own source (`E-ACTOR`, `E-CONCURRENT`,
`E-DEADLOCK`, `E-SCHEDULE`). `parallel` blocks, `par_map` and `net_workers`
run in order on the one interpreter in both, and stop as any statement does.

**(7) A panic and an exit under the adapter unwind.** `cx.trap`, which §6.19
gave `test` and `bench`, is on under the adapter too, so a panic becomes
`exited` 101 with the stack trace event, built from the frames of (4) at the
statement where the panic first left a statement, as the reference captures
it; an `exit(n)` is `exited` n. A recursion that runs out of stack is the
one thing that cannot match: under `run` the reference stops at its own
depth count and the interpreter at its host's stack, so the traces differ in
length, and under `dap` the Rust adapter does not even get that far -- it
overflows its own stack and aborts (exit 134, measured on the one corpus
program that does this, `E-PANIC-TRACE`). An abort is not a behaviour to
copy; such programs are left out of the sessions, and the Grasm adapter
reports the panic as `run` would.

**(8) One JSON port, in `tirinterp`.** Above.

**(9) The servers use `read(2)` and `write(2)`.** Above.

#### What the interpreter must change, and `run`

In `tools/tirinterp`: the journal module and a gate-and-record around each
of the 40 builtins and the C call (`cx.jrnl`, chosen at the first question;
off and never asked under any command that sets neither variable, which is
every gate but the new one); the debugger state and the hook in
`block_prefix`, the reference's frames and scopes kept beside its own,
last-use moves turned into copies, the program's prints routed to the
adapter, all behind `cx.dbg`; `cx.trap` on under the adapter. Under `run`,
`test` and `bench` with neither variable set nothing of this is reached but
two flag tests per statement and per builtin, so `run` stays the same bytes,
held by `tirinterp`, `frontsmoke` and `clidiff`'s `run` section, and its
speed is measured against §6.17's table on the nine benchmarks before and
after. `test` and `bench` record and replay too, as Rust's do, since the
hooks are the interpreter's; the gate exercises `run` and the adapter's
launch argument, and `test` under a recording is sampled (the programs whose
tests make a journaled call).

**Order** (the brief's): the journal and replay first (step two), with the
runtime's exemption and the backend's `export fn` fix, the JSON port and
`grasm journal`; then the adapter (step three); then the language server
(step four); then the gates and controls (step five). Each step's checks are
run before the next starts.

#### How each is held to Rust

`clidiff` gains three sections, both commands from the worktree's root on
the same input, every byte compared, the counts asserted and a floor on a
whole run; `scripts/lib/clicases.txt` loses the `elsewhere` tags of the
three and gains the lines that reach each arm's own checks (`journal`
without a file, with `--full` first, with two files, on a missing file, a
directory, a file that is not a journal, a truncated one; `lsp x`, `dap x`,
`lsp --x`).

- **`journal`**: every corpus program with a `main`, run by each command
  under `GRASM_RECORD` into a file of its own: stdout, stderr (the trace
  stripped, as in `run`) and status the same, the same file or none, and the
  two files the same bytes once the values of the builtins that answer
  differently from run to run (`now_millis`, `monotonic_millis`,
  `monotonic_nanos`, `random_bytes`, `net_port`, `tcp_peer`: what recording
  twice with Rust found) are replaced by a placeholder of the same shape.
  Then each command replays *Rust's* recording, and Rust replays the Grasm
  command's: the same bytes as the recording run printed. `grasm journal`
  and `grasm journal --full` by both commands on both recordings. And for
  every program whose recording holds a record, five journals
  `scripts/lib/jmangle.py` writes from it, each replayed and printed by
  both: one record short (ran out), the first record renamed (diverged), cut
  mid-record, the magic broken, and every value of the first text record
  grown past the budget; plus the 38 programs whose recording holds more
  than `args`, built natively by the Grasm command, recorded by the
  executable and replayed by both commands' `run` (a native recording is the
  case docs/replay.md exists for). Cost, estimated: one more `run` section
  for the recordings (the Rust half took 22 minutes here, with four programs
  cut at 120 s), and a few minutes for the 184 programs' replays and the 38
  programs' mangled journals and native builds.
- **`dap`**: the measured sessions, transcripts made by
  `scripts/lib/dapscript.py` (to be written): every corpus program with a
  `main` but those decisions (5) and (7) leave out stops on entry and steps
  in 200 times; every fourth also cycles `next`, `stepIn`, `next`, `stepOut`
  100 times and runs to a breakpoint on every third line 50 times -- at each
  stop `threads`, `stackTrace`, `scopes` and `variables` -- then its input
  ends, the program runs to its end unsupervised, and its output events and
  exit are compared too. The two streams are compared as they arrive, by
  length and SHA-256, never stored: one is 2.4 GB. A session the Rust
  adapter does not finish in 60 s is counted and not compared (seven of
  1,073 here), and the two extra sessions are skipped for a program whose
  stepping session took Rust more than 10 s (counted). And a fixed set:
  launch failures (the conformance programs that must not compile, every
  tenth; a missing file; the registry app; a journal that is missing and one
  that is not a journal), `configurationDone` before `launch`, `evaluate` of
  a name that exists and one that does not, `scopes` of an outer frame,
  `variables` of reference 0, breakpoints replaced in phase two,
  `disconnect` while stopped, framing and JSON errors in each phase (every
  error message of the reader), and the `journal` launch argument on the
  recordings above. Cost, estimated: the Rust half about 22 minutes on the
  measured times; the Grasm half is the TIR interpreter with the hook, 0.8
  to 2.9 times Rust's time on `run` (§6.17), so the section about an hour to
  an hour and a half.
- **`lsp`**: one scripted session per tracked `.gr` file -- `initialize`,
  `didOpen`, `documentSymbol`, `hover` and `definition` at every
  declaration's name and at 40 identifiers spread over the file,
  `completion`, a `didChange` adding a function with a type error and a
  hover on it, a `didChange` that no longer parses (dropping the last `}`),
  `didSave`, `shutdown`, `exit` -- and a fixed set: an `untitled:` URI (the
  one-module check), a `file://` path that does not exist, a request before
  `didOpen`, positions past a line's end and past the last line, a line with
  non-ASCII text, a method nobody handles with and without an `id`, ids of
  every JSON type, a body that is not JSON, a header with no
  `Content-Length`, one with an unparsable length after a good one, invalid
  UTF-8 in a header and in a body, a body cut short, `exit` before
  `shutdown`, and JSON strings with every escape (a surrogate pair among
  them) and numbers in every form. Cost, estimated: 277 s of the Rust
  server's time, the Grasm server's to be measured at step four (front
  checks each change three times per file, as Rust does), and about ten
  minutes of transcript writing -- under half an hour.

`clicontrols` gains a control for each: a journal record dropped (a
builtin's hook skipped), the exemption removed (the command records itself),
a response field dropped (`allThreadsStopped`), a step that lands on the
wrong line (a statement's start one line on), a variable's binding not
shadowed, a diagnostic missed (the end-of-file filter off by one the other
way), a hover signature reworded. `fixpoint` must still find stage 3 the
same as stage 2, and the stage-2 command passes the whole `clidiff`.

**Copied from Rust, and recorded as its defects** (docs/known-issues.md,
each with a reproducer): a replay that leaves records unread says nothing,
where docs/replay.md says it does; the LSP drops every diagnostic that
starts at the end of the buffer -- an unclosed `{` at the end of a file
shows no error at all -- because it keeps those that start *before* the
first file's end; positions and ranges are byte offsets where the protocol
says UTF-16 code units; the escape pair `\uD83D\uDE00` reads as two U+FFFD;
an unparsable `Content-Length` after a good one forgets the good one (LSP)
or ends the adapter (DAP); `terminate` and `disconnect` do not stop the
program; the adapter overflows its own stack on a deep recursion and aborts,
where `run` reports the panic; and it loads a program without the registry
packages `run` resolves, so a program with a registry dependency cannot be
debugged ("cannot load module"); and a breakpoint set by a path spelled
other than the launch's -- an absolute path, as editors send, for a program
launched by a relative one -- is answered `verified` and never stops
(checked 2026-11-15).

**Not copied: the Rust adapter runs a program without the checker's facts**
(docs/known-issues.md, S2 by the coordinator, 2026-11-15). Found at step
three: `grasm_dap::serve` makes its interpreter with `Interp::new_program`
and never hands it the checker's facts that `grasm run` hands it -- the
literal a `[f64]` stores, an integer's width, raw memory, `call_c`,
interface calls -- so under `grasm dap` a program computes other values than
under `grasm run`, with no error. The Grasm adapter runs it as `grasm run`
does. Nor copied, found by the journal section: the Rust interpreter never
gives a closed socket's handle out again, where the Rust runtime and the
Grasm one take the lowest free (docs/known-issues.md, S3 by the coordinator,
2026-11-17), so `grasm run` and `--native` number sockets otherwise after a
`net_close`; the Grasm command's `run` numbers them as `--native` does.

**Steps two to five, as built** (2026-11-15 to 19; the main checkout's
compiler; other programs' swap held the disk between 2.6 and 10 GB, the
gates were stopped at the disk guard six times -- once by the coordinator's
hand, five times by the guard -- and run again from the step, or later the
section, they stopped in; loads of 3 to 10, so no time here is to be
quoted).

- **The backend's `export fn`** (decision (2)'s condition), first and in its
  own commit (`6427283a`): every `export fn` is a root of the program
  (`compiler/mir/src/prog.gr`), and `linktest` builds
  `scripts/fixtures/link/export_uncalled.gr` with both backends, links each
  and runs it: "Grasm fixtures (s3): as expected: 1 of 1" (with the old rule,
  "compiler/link failed: cannot export _fixture_uncalled: it is not
  defined"). On that commit `s3subset` "identical: 648 mismatched: 0",
  `objtest` "objects: 648 reproducible: 648 ... with problems: 0",
  `linktest` "Cranelift objects: identical: 651 ... Grasm backend objects: in
  the subset: 648 identical: 648 ... C fixtures: identical: 3 of 3", and on
  `5c17b142` `s3controls` "controls: 42 caught: 42 missed: 0".
- **The journal** (`tools/tirinterp/src/journal.gr`, `builtins.journaled`,
  the refusal in `cabi.gr`, `compiler/grasm/src/journalcmd.gr`). The mode is
  off when neither variable is set and is otherwise chosen at the first
  question, as the reference chooses it; under a journal `journal.mark`
  turns the calls of the forty recorded built-ins into a node of their own,
  so a run with no journal tests for one nowhere. The journal -- mode, file,
  records, cursor -- travels with the slab the tasks of a `concurrent` block
  hand on, so they share one in the order they call: the first build gave
  each task its own cursor, and a replay whose tasks reached outside the
  program diverged at call 2, found by the check the coordinator asked for;
  `scripts/fixtures/journal/tasks.gr` is in the gate since. `compiler/rt`
  looks for `unjournaled_host` through `os.RTLD_MAIN` (RTLD_MAIN_ONLY on
  Darwin; RTLD_DEFAULT on Linux, which has none).
- **The language server** (`compiler/grasm/src/lsp.gr`, over
  `tools/tirinterp/src/json.gr` and `rpc.gr`, and `front`'s `load_text` and
  `single_module`) agreed on every session from its first build.
- **The debug adapter** (`compiler/grasm/src/dap.gr`,
  `tools/tirinterp/src/debug.gr`, the variables' names and closures' reads
  loaded from TIR in `load.gr`). `debug.rewrite` turns the program's nodes
  into the debugger's before `main` runs -- blocks, `match`es, `let`s,
  closures, `inout` calls and `print`s become opcodes of their own, and every
  function body, `for` body and closure body is put under a node that keeps
  the reference's frames and scopes -- so a run without the debugger tests
  for it nowhere; a task of a `concurrent` block runs the nodes as they
  were, but for its prints. What the corpus taught it: an `inout` argument
  must keep its value while the other arguments run (a stop inside one of
  them showed `()`) and still be moved before the call (a copy made every
  write in the callee copy the whole value: `scheme-demo` stepped past its
  limit); a function with contracts is a wrapper and a body whose blocks
  share one span, so a TIR block is paired with the AST block of its span,
  its number of statements and their kinds; each debugger node is a call of
  its own, and a debugged call's body moves to a fresh stack halfway between
  `body`'s hops (with the handlers inline, `scheme-demo`'s step session
  overflowed a fiber); and the debugger's lists are taken out of the
  context, changed and put back, never changed in place -- a method call on
  a field of a struct finds the field by name at run time, the debugger's
  41 fields are the last of `Cx`'s 87, and under the stage-2 command
  `structs.field_named` and `memcmp` were 60% of a step session's samples.
- **The timing of `run`** (the owner's rule, through the coordinator,
  2026-11-17: twelve rounds of the seven serial benchmarks, master, the
  build and the build with every `cx.dbg` test made false interleaved; under
  1.01 of master's geometric mean, the gap accepted, else the op-code
  rewrite). The first build, with the debugger's state saved in each call's
  locals, was 4.6 to 5.4% slower on four of the nine benchmarks; with it in
  the context, 1.0168 of master (the tests made false: 1.0117), so the
  rewrite was made: 1.0036. On the final tree, 0.9969 of master (fib 1.0036,
  sort 0.9877, wordfreq 0.9903, tree 0.9992, sieve 1.0022, matmul 0.9936,
  mandelbrot 1.0016), output byte-identical.
- **The coordinator's decisions on the gates**, the owner's to reverse: a
  session whose Rust adapter prints or ends otherwise than `grasm run` holds
  the Grasm adapter to `grasm run` (`scripts/lib/dapran.py`, 2026-11-15); one
  that differs only in the variables pane is a counted, printed exception
  (`dapmask.py`, 2026-11-15), now none, since a corpus session that differs
  is run again by both adapters launched on one recording of the program by
  the Rust command (the agent's, 2026-11-17: `L-LIB-RANDOM`'s rejection
  sampling stopped at a breakpoint as often as it drew again); a session
  ended by the host's channel panic (`E-CHANNEL`, `E-DEADLOCK`) is counted
  as §6.19's decision (3) counts it under `test` (2026-11-16); a journal
  that differs only in socket handles after the run's first `net_close`, as
  one renumbering, is counted (`jnorm.py`, 2026-11-17), with the control
  `net-handle`; and a session the Grasm adapter does not end in its limit is
  run once more given the program's own run time -- that command's plain
  `grasm run` of it, timed in the gate -- on top of the limit, must then
  match, and is counted on a line of its own and held to at most three, with
  the control `dap-hang` for a session that never ends (2026-11-19). Why:
  most of a heavy program's session is the program running to its end once
  the input stops. The four step sessions the stage-2 dap section first
  timed out on, alone (Rust; stage 1 before and after the lists were taken
  out; stage 2 before and after):

  | session | Rust | s1 | s1 after | s2 | s2 after |
  |---|---|---|---|---|---|
  | `benchmarks/grasm/parmap.gr` | 56.7 s | 168.5 s | 151.5 s | 279.0 s | 240.5 s |
  | `packages/accounts-demo` | 55.5 s | 163.9 s | 99.5 s | 316.2 s | 161.5 s |
  | `packages/observe-demo` | 55.9 s | 154.8 s | 91.2 s | 300.6 s | 146.7 s |
  | `packages/postgres-demo` | 44.2 s | 126.1 s | 73.1 s | 242.9 s | 118.4 s |

  every stream the same bytes as Rust's. `parmap`'s plain `run` takes 55.0 s
  (Rust), 118.3 s (stage 1) and 156.7 s (stage 2): the stage-2 adapter adds
  about 84 s to it, the Rust adapter about 2 s.
- **Why `grasm-j` is slow.** The journal section runs on the stage-1 command
  relinked with the Grasm runtime (decision (2)), and that build takes about
  25 minutes on `examples/parallel_map.gr`: `sample` puts 87% of its time in
  the Grasm runtime's struct-field and reference-count entry points
  (`grt_struct_get_at` alone 21%), which the Cranelift-compiled interpreter
  calls on every read of its context.
- **The gates.** The final tree is `9f12e7b1`; each line below names the commit it ran on.
  - On `cc4dd645` (the tree but the gatecheck fix and the floors):
    `tirinterp` "identical: 651 mismatched: 0 timed out: 0 skipped: 0",
    "round trip: 1671 identical, 0 not", "panics: 25 identical, 0 not";
    `clidiff` `dap` with stage 1, "dap: 1002 sessions compared the same:
    960, 51 of them fixed the Rust adapter running it otherwise than grasm
    run: 25 differing in the variables pane alone: 0 the same under one
    recording: 13 the host's channel panic: 4 reading stdin, left out: 3
    past 60s for Rust: 6 the Rust adapter's own stack: 1 long, not stepped
    again: 14", "given the program's own run time: 0"; with stage 2, "dap:
    1004 sessions compared the same: 961, 51 of them fixed ... the same under
    one recording: 13 the host's channel panic: 4 ... past 60s for Rust: 4
    ... long, not stepped again: 14", "given the program's own run time: 1
    (slowest: benchmarks/grasm/parmap.gr step 245 s, run 159 s)" (the count
    of sessions moves with how many Rust sessions pass 60 s and 10 s); `run`
    with stage 2, "run: 651 programs with a main the same: 651 timed out:
    0"; `clicontrols` "controls: 30 caught: 30 missed: 0" (§6.21's 22 and
    eight: `journal-lost`, `exempt-gone`, `dap-field`, `dap-line`,
    `lsp-diag`, `lsp-hover`, `net-handle`, `dap-hang`); `fmttest`
    "formatted: 2013 unformatted: 0 does not parse: 5 exempt: 1";
    `gfmttest` "checked: 1537 disagreeing: 0"; `linttest` "linted: 2014
    findings: 994 does not parse: 5 broke: 0".
  - On `9f12e7b1`: `gatecheck` "gate helpers checked: 587 undefined: 0
    corpus copied: 0 grep -q under pipefail: 0 gates not executable: 0" (on
    `cc4dd645` it found one, the recording's first line grepped under
    pipefail in the dap section's replay path; `c12deea3` reads it into a
    variable, the same condition, checked by the dap section on the eight
    programs that take that path with stage 1: "the same under one
    recording: 9", the rest the same); the litter check, against lists
    taken before the first gate: no new untracked or ignored file and no new
    top-level entry in the worktree, the gate worktree or the main checkout.
  - On `086d58f3`: `fixpoint` "programs: 6 and the grasm command artifacts
    per stage: 16 and 2 compared: 54 ... identical: 54 differ: 0", "stage
    2's TIR the same as grasm tir's: 6 of 6", "the command's build the stage
    pipeline's (its rt.o and the 4 programs): 5 of 5"; the command's stage 2
    is 4,616,696 bytes, SHA-256
    `8c0296fef7fedaaaccc10ae89445e79b987ed948beb619bb7588ed49df851fb4`;
    `clicontrols` with `CONTROLS=dap-field,dap-line` "controls: 2 caught: 2
    missed: 0".
  - On `8e4b8915`, and not run again (the diff since is `debug.gr`, the
    dap section's retry rule, the gatecheck fix and the floors; the
    coordinator's decision): the `journal` section with stage 1 and the
    Grasm-runtime command, "journal: 652 programs recorded the same: 652,
    185 of them with a journal (7 differing only in the IoFailed wording, 1
    only in socket handles after a net_close) replays alike: 370 printed
    alike: 370 mangled journals alike: 330 native recordings replayed alike:
    45 of 45", and with stage 2 the same line; with stage 2, `deps` "1448
    packages, 4344 commands the same, lockfiles and all: 4344, 1447 of them
    refusals lockfiles written: 1448", `add` "1448 packages the same,
    manifests and all: 1448, 0 of them refusals", `publish` "1448 packages
    the same, registries and all: 1448, 635 of them published", `lsp` "2036
    sessions the same: 2036, 17 of them fixed"; and the sections before them
    -- `lines` to `registry` -- in a whole run the guard stopped in `deps`,
    whose log the next start wrote over: their lines are the coordinator's,
    copied from the screen while it ran (`lines: 272 the same: 267`, `corpus
    ... the same: 8685`, `toml ... the same: 1474`, `run ... the same: 651`,
    `fmt ... the same: 20190`, `lint ... the same: 5445`, `test ... the same:
    1960 ... both timed out, --contracts: 1`, `test, main as a test ... the
    same: 649, 13 of them failing the host's panic: 2`, `bench ... the same:
    1743`, `doc ... the same: 3168`, `api ... the same: 10793`, `bundle ...
    the same output: 651`, `source ... the same output and file: 1737`,
    `build ... the same output: 648`, `target ... 195 of 195`, `registry ...
    the same, files and all: 129`), not a file the gate wrote.
  - The floors of the three sections (`9f12e7b1`, the agent's), a margin
    below these runs: `journal` 640 programs, 170 journals, 340 replays and
    prints, 300 mangled, 40 native, at most 10 in the IoFailed wording and 2
    in socket handles; `dap` 950 sessions, 45 fixed, at most 30 run otherwise
    by the Rust adapter, 20 under one recording, 4 host panics, 2 in the
    variables pane alone, and 3 given their program's run time; `lsp` 1,900
    sessions, 15 fixed. They hold only on a whole run, and no whole run has
    been made with them.

Not run on this branch: the rest of `allgates.sh`, `fuzz_rtsys`, and a
whole `clidiff` run of one command on one tree.

**What is left for S8 after this part.** In the backend, `lower.gr`'s
`take_child` finds a struct field by name for every method call that
changes a field in place (`grasm_struct_take`, `grasm_struct_get`): a
by-index take where the field's position is known would remove what the
profile of a stage-2 step session put at 50% (`field_named`) and 10%
(`memcmp`), and 21% (`memcmp`) under stage 1 -- left, by the coordinator's
decision, as a backend item. The debugger's own cost per statement on
stage 2, about 84 s over parmap's 157 s run where the Rust adapter's is
about 2 s. The Grasm-runtime command's struct-field and reference-count
entry points (`grasm-j`, above). The rest as §6.21 left it -- `test
--schedules` and `--schedule-seed` for a test that starts tasks, a panic
inside a `concurrent` task under `test`, one the runtime's channel
operations raise, the interpreter's own stack, the `IoFailed` wording; the
LSP on the query engine and the wasm32 backend; the Rust TOML reader's
misreadings, kept until S9; and the Rust defects this part copied or
declined (docs/known-issues.md).

### 6.23 S8 part six: `test --schedules`, a panic in a task, the interpreter's own stack, the `IoFailed` wording, and the by-index field take

Written 2026-11-19, before any code, as the design to build to. Parts two to
five (§6.19-§6.22) left five things in the `grasm` command in Grasm
answering otherwise than the Rust one, or not at all: `test --schedules N`
and `--schedule-seed S` for a test that starts tasks (refused, decision (4)
of §6.19); a panic inside a `concurrent` task under `test`, and one the
runtime's channel operations raise (the host's panic, decision (3)); the
TIR interpreter's own stack (the host's); the `IoFailed` text (the
runtime's, not the Rust interpreter's); and, from §6.22, `lower.gr`'s
`take_child`, which finds a field by name where its position is known. The
Rust toolchain stays stage 0 and the oracle and is not changed; where its
behaviour is wrong it is reproduced and recorded in docs/known-issues.md,
or, where it aborts, recorded and not copied. The decisions are numbered
(1) to (8); each is proposed by the agent implementing this part, and is
the coordinator's to accept and the owner's to reverse. Where there is a
choice the alternatives are written under the decision. The coordinator's
answers (2026-11-19) are under "The coordinator's decisions" below.

#### What was measured

Measured 2026-11-19 with the main checkout's `grasm` (stage 0, the release
of 2026-11-01), the command built from this branch's base by it (stage 1)
and part five's stage 2 (`fixpoint`'s, the same sources), from the
worktree's root, at a load of 3 to 6 from other work.

**Tests that start tasks.** None of the corpus's 43 programs with a
`test` has one that runs a `concurrent` block: the Rust command with
`--schedules 1` reports `(1 schedules)` on none, and no file with a `test`
mentions `concurrent` or a channel. Five corpus programs use tasks or
channels at all, all conformance programs with a `main`: `E-ACTOR`,
`E-CHANNEL`, `E-CONCURRENT`, `E-DEADLOCK` and `E-SCHEDULE`, which
`clidiff`'s mains-as-tests run (`test "main" { main() }` appended). Under
the Rust command three pass -- `ok (3 schedules)` with `--schedules 3`,
`ok` with `--schedule-seed 5` -- and two fail on every schedule, `E-CHANNEL`
("send on a closed channel") and `E-DEADLOCK` ("deadlock: ..."); under the
Grasm command the three say "`grasm test --schedules` is not in the Grasm
toolchain yet" (exit 1) and the two end on the host's panic (101). None of
the five prints differently under any of the seeds 1 to 20, so the corpus
alone cannot tell a scheduler that honours the seed from one that ignores
it. `compiler/rttasks/taskfuzz.gr` (`fuzz_rttasks`' program: random tasks
over random channels, each printing what it does) can: wrapped as a test of
ten rounds, each of three fuzz seeds prints otherwise under every one of
the schedule seeds 1 to 10, and `--schedules 20` takes the Rust command 1.0
to 1.5 s. Under `run` the two commands already agree on it: 5 fuzz seeds,
each plain and in five of its panic modes, the same bytes and status.

**Panics in tasks and channels.** No corpus program panics inside a task.
The runtime's channel panics in the corpus are `E-CHANNEL`'s (a send on a
closed channel, outside every task) and `E-DEADLOCK`'s (every task
waiting): they are `clidiff`'s "host's panic: 2" under `test` and its 4
under `dap`. Found while measuring: a channel made inside a task cannot be
used by a sibling under the Grasm command -- each task's context has its
own copy of the channel list -- so a program in which a task makes a
channel and hands it to another ends "panic: not a channel" (101), where
the Rust command runs it. No corpus program does that.

**Recursion.** The Rust interpreter stops a recursion at 100,000 calls or
when its own stack passes a 448 MiB budget (`STACK_BUDGET`, on a 512 MiB
thread), whichever comes first, and the stack comes first for every shape
measured: the deepest call that still ran, by shape --

| recursion | Rust | stage 1 | stage 2 |
|---|---:|---:|---:|
| `1 + depth(n - 1)` | 41,941 (41,943 under `test`) | 99,998 | 99,998 |
| `d(n - 1)` as a statement | 36,653 | 99,998 | 99,998 |
| `[n - 1].map(down)` | 42,426 | 99,998 | 99,998 |
| `f(d(n - 1, f))`, `f` a closure | 29,775 | 99,998 | 99,998 |
| `1 + depth(n - 1)` inside a `go` | 41,941 | 8,733 | 15,724 |

So outside a task the Grasm command runs every recursion the Rust one runs,
and runs on where Rust refuses, up to the count; inside a task it stops
below [E-RECURSION-DEPTH]'s guaranteed 10,000 on stage 1, at a depth that
differs between stages (each task is a host task with the host's 6.5 MiB
stack, and the interpreter's hop to a fresh stack every 500 calls is off
inside a task), with the host's panic -- the same words, so `run` agrees,
but `test` ends there.

**The interpreter's other recursions.** Walks over a value the program's
depth does not count: for a variant list nested `n` deep, the Rust
interpreter compares (`==`) and prints (`"{l}"`) one 1,000,000 deep, and at
3,000,000 its `==` overflows its own stack and aborts ("thread '<unknown>'
has overflowed its stack", SIGABRT, exit 134); stage 1 compares 20,000 and
not 40,000, prints 15,000 and not 20,000; stage 2 does both at 20,000 and
neither at 40,000 -- `value.show`, `equal`, `compare` and `key_text` in
`tools/tirinterp` recurse. Dropping a list 3,000,000 deep ends well under the
Rust command and stage 1. And expressions nested deep without a call: a `^` chain of 10,000 or
50,000 terms and an `else if` chain of 10,000 run under the Rust command;
under the Grasm command a `^` chain of 8,000 runs and one of 10,000 ends on
the host's stack (the loader's `expr` and `ev` recurse per node), and an
`else if` chain of 50,000 ends "a builder would need more memory than one
can hold", because the TIR text the command writes for itself to read back
indents every level: 200 MB for the 10,000-term `^` chain and 1.8 GB for
10,000 `else if`s (`grasm tir` writes the same bytes under both commands).
`+` chains and parentheses are refused by both parsers ("code is nested
too deeply").

**The `IoFailed` wording.** Every corpus program with a `main` recorded
by the Rust command (`GRASM_RECORD`, 120 s each; the four programs §6.22
names ran past it): 11 journals hold an `IoFailed`, and recorded again by the stage-2
command (the Grasm runtime, exempt from its own journal) 7 of them differ
only there -- `clidiff`'s "7 differing only in the IoFailed wording" -- with
the same output and status. What differs, by builtin: `list_dir` on a file
(`fs-demo` 76 times, `lsm-demo` 4, `walkdir-demo` 2), `write_bytes` into a
missing directory (`sysfile-demo`), `make_dir_new` under a missing one
(`tempdir-demo` 8), `tcp_connect` refused (`L-LIB-NET`): each Rust's
`<message> (os error N)` -- `Not a directory (os error 20)` -- and the
Grasm command's message alone;
`proc_run` and `proc_pipeline` of a missing program (`L-LIB-PROC`):
`no-such-program-anywhere: No such file or directory (os error 2)` against
the same without the number; and `tcp_connect` to `no.such.host.invalid`
(`L-LIB-NET`): Rust's `failed to lookup address information: nodename nor
servname provided, or not known` (`std`'s words around `gai_strerror`)
against `No such file or directory`, because both runtimes report a failed
`getaddrinfo` as `ENOENT` and drop its own code (`grasm_sys`, `net.gr`). The
other 4 (`tlsdiff`, `L-LIB-NET-PEER`, `L-LIB-RANDOM`, `L-LIB-UDP`) record
socket errors both runtimes already word as `std` does. No `run` of the
corpus prints an `IoFailed` (`clidiff`'s `run` and `bundle` sections). The
system's messages are distinct for every error number on this Mac (1 to
107).

#### What the Rust side does

`crates/grasm_interp/src/tasks.rs`. Each task of a `concurrent` block is an
interpreter of its own (depth 0, empty call stack) on a fiber with the
interpreter thread's budget (`TASK_STACK`); they share the channel table,
one output buffer drained after every resume, and the slab. The block runs
in rounds: a round is whatever is runnable at its start; in order, each
slot not yet finished is marked running, cleared from the waiting flags,
started (a fiber made at its first turn) or resumed, and its output
drained; a task that ends with an error or a panic ends the round and the
block (`failed`), its siblings abandoned with their fibers; a task that gave
way without joining a queue is runnable again, else it waits for a wake.
After the round the wakes meant for this block are taken (others are left
for the block around it); none runnable and some alive is
`deadlock: every task is waiting on a channel, and nothing left can send or
receive`. A channel keeps two queues, waiting for room and for a value;
`wake_one` passes over stale entries; close wakes every waiter, room side
first; a rendezvous send returns once its value is taken (`sent`/`taken`
tickets); `chan_select` joins every queue. A channel operation that must
wait outside every task panics with the deadlock text and "-- a channel
operation outside every `go` cannot wait for one; move it into a task".
With `--schedules`, `Chans::schedule` holds a SplitMix64 state, and each
round of two or more is permuted by a Fisher-Yates shuffle drawing from it
(`round_order`): the queue decides who is in a round, the seed their order;
one state per run of a test, shared by nested blocks.

`test_program_with` (`grasm_interp/src/lib.rs`): for each unit test, the
schedule set to `--schedule-seed` (or none) for one run; then, if that run
passed, no `--schedule-seed` was given and the run started a block
(`blocks_run` grew), the same test run again `--schedules` times in the same
interpreter, the `k`th under `schedule_seed(--seed, k)` (a SplitMix64 step
of `seed ^ (k + 1) * 0x9e3779b97f4a7c15`), stopping at the first failure:
`test "t" ... FAILED: <m>\n  under schedule k of N\n  replay:
--schedule-seed <s>`, or `test "t" ... ok (N schedules)`. Each run prints
what the test prints. Property tests, contracts and doc examples run with no
schedule. A panic, error or `exit` inside a task is that test's failure, in
the task's words, and the next test runs.

#### What Grasm has, and what has to be written

`tools/tirinterp` runs each program task as a task of the host -- a host
`concurrent` block around the program's -- on a copy of the context, with
the program's channels the host's channels, so the host runtime's scheduler
(a third implementation of [E-SCHEDULE], §6.11) picks who runs, and the
interpreter cannot give it a seed, abort a task's siblings, or see a
deadlock before the host does. The slab, raw memory's sandbox and the
journal are handed from task to task through a host channel of capacity one
(the baton). `fault` and `exit_with` unwind under `test`, `bench` and `dap`
everywhere but inside a task. `testing.gr` counts blocks (`blocks_run`) and
refuses the flags where Rust would act on them.

**(1) A scheduler of the interpreter's own, a port of `tasks.rs`, with the
host's tasks as stacks only.** Proposed. The seeded orders are a property of
the reference's own scheduler -- its rounds, and a shuffle of each round of
two or more drawn from one state in a fixed order -- which [E-SCHEDULE]
does not describe ("N other orders, each named by a seed"). The TIR
interpreter cannot give the same orders without doing what `tasks.rs` does,
step for step; that is the decision the brief asks to be stopped for. The
design: `tools/tirinterp/src/tasks.gr` holds the reference's state --
channels (buffer, capacity, closed, two wait queues, tickets), the ready
list, the per-block waiting flags, the running slot, block ids, the
schedule state -- in the slab that is handed with each turn, so every task
sees one table. A program block becomes one host block of host tasks: a
scheduler, which runs the reference's loop and `round_order`, and one host
task per program task, each waiting on a host channel of its own for its
turn (which carries the slab) and answering on one back to the scheduler
when it ends or gives way. Only the task holding a turn can run, so the
host's own order decides nothing; a host channel operation never waits
where the reference would not, so the host never sees a deadlock. The
channel builtins become the reference's, with its words; `fault` and
`exit_with` unwind inside a task under a trap, and the task's outcome ends
the block as the reference's does, the scheduler sending every suspended
sibling a turn that tells it to unwind without running another statement,
so the host block ends. Under `run` nothing visible changes -- the default
order is [E-SCHEDULE]'s, which the host already gives (`fuzz_rttasks`
holds all three to it) -- and a panic stays the host's.

What it buys: `--schedules` and `--schedule-seed` as Rust gives them (the
runner's loop and lines are a dozen lines of `testing.gr` on top);
[E-TASK-PANIC] under `test`, `bench` and `dap`; the channel panics and the
deadlock as test failures; channels shared by every task. What it costs:
every turn is two host channel operations and two host switches more than
today, to be measured on `E-SCHEDULE`, `fuzz_rttasks`' rounds and the
`run` timing of §6.22; an estimated 500 lines in `tools/tirinterp`; and the TIR
interpreter's task model becomes the reference's, so a change to one is a
change to both (the Rust interpreter is frozen until S9).

The alternatives: **(1b)** keep the host's scheduler and decision (4) of
§6.19 -- `--schedules` stays refused -- and do item four in part inside it:
a task's panic unwinds its task and leaves a flag in the slab that every
sibling obeys when it next takes the slab (only one runs at a time and they
switch only at channel operations, so none runs another statement); a
closed channel checked by the interpreter before the host's operation; the
channels moved into the slab. A deadlock stays the host's to see (it alone
knows who is waiting), so `E-DEADLOCK` keeps ending `test` on the host's
panic. **(1c)** seeded orders of the interpreter's own design: refused, as
nothing could hold them to Rust's replay lines.

**(2) Inside a task, the interpreter hops to spare host stacks.** Proposed,
with (1); not possible without it. Today a hop is a nested host block, and
a host task in a nested block cannot wait on its siblings outside it (the
host reports a deadlock), so the hop is off inside a task. Under (1) a
program block's host block also holds spare host tasks, each waiting for a
body to run; a task at its 500th call hands the evaluation to a spare in the
same block and waits on it, and a give-way from inside a spare goes to the
scheduler as from the task itself (its turns are sent to the deepest
stack it is running on). So a task recurses as deep as the main program,
to the reference's count of 100,000 -- counted from the task's start, as the
reference's task interpreter starts at depth 0. The number of spares is
fixed when the block starts: enough for the count, `(100,000 - depth) /
500`, at most 200, each costing a host task that starts, waits and ends
(its stack mapped, not touched, and kept by the host's pool for the next).
To be measured; if it costs too much on `fuzz_rttasks`, fewer, with the
in-task bound then set to what they hold and recorded. The alternatives:
**(2b)** a fixed count inside a task, 500 calls (what one host stack is
known to hold of the costliest shape), worded as the reference words it --
the same on both stages, but below the guaranteed 10,000; **(2c)** leave it
to the host, as today.

**(3) The count stays 100,000; the window below it is recorded, not
copied.** Proposed. The Rust interpreter refuses past its stack, between
29,775 and 42,426 calls for the shapes measured; the bound depends on its
frames' sizes, so reproducing it would mean modelling a Rust build's stack
use per construct. [E-RECURSION-DEPTH] makes the bound the
implementation's, from 10,000 up; the Grasm command keeps the reference's
own count, runs everything the Rust one runs, and refuses past 100,000 in
the same words and status. A recursion between Rust's stack bound and
100,000 calls runs under the Grasm command and is refused under the Rust
one; recorded in docs/known-issues.md as a difference the clause allows,
with the measured bounds. The alternative, a lower count, would refuse
programs the Rust command runs.

**(4) The interpreter's walks over values and expressions run off the host
stack.** Proposed. `value.show`, `equal`, `compare` and `key_text` become
loops over an explicit stack, so no value the Rust interpreter walks is too
deep for them; where Rust aborts (an `==` 3,000,000 deep), the Grasm command
answers, and the abort is recorded as a Rust defect, not copied (§6.22's
decision (7) did the same for the adapter's). The loader's `expr` and `ev`
count nested nodes as `body` counts calls and hop to a fresh host stack
every 500 levels of either (inside a task, to a spare). The alternative is
to leave these to the host and record them.

**(5) The TIR the command reads back is written without indentation.**
Proposed, with (4), and left out if the coordinator prefers: it is
`compiler/front`'s printer, not the interpreter. `run`, `test`, `bench` and
`dap` print the program's TIR and read it back (`build.tir_to_run`,
`sexp.read`); the printer indents each nested node, so the text grows with
the square of the nesting -- 1.8 GB for an `else if` chain of 10,000 -- and
an `else if` chain of 50,000, which the Rust command runs, cannot be run.
The printer gains an unindented form for the command's own use; `grasm tir`
and `front tir` keep today's bytes (`tirdiff`). The alternative: record it
(docs/known-issues.md already has `front tir` printing nothing for a TIR of
4 GB, open, S1).

**(6) The `IoFailed` text is the Rust interpreter's.** Proposed. `io_err`
(`tools/tirinterp/src/builtins.gr`) is the one place a host `IoError`
becomes the program's, and it words an `IoFailed` as the Rust interpreter's
`std` does: a text that is the system's message for an error number N
(`strerror(N)`, N from 1 to the system's last), or ends with `": "` and that
message (`proc_run`'s), gains ` (os error N)`; other texts stay as they
are. And `tcp_connect` (and every builtin that resolves a name) answering
the `ENOENT` text, which only a failed lookup gives there, is `failed to
lookup address information: <gai_strerror(code)>`. The code is the one
thing the runtimes threw away; two ways to it: **(6a)** ask `getaddrinfo`
for the same name again, through `extern fn`, only on that failure (the
name the program already asked for, so no new host is asked, and the
command's own call, which no journal sees); or **(6b)** take `EAI_NONAME`,
the code for a name that does not exist, and accept another text on any
other lookup failure. (6a) is proposed, being exact. The conversion runs
before the journal records the answer, so a recording is Rust's bytes. The
native build keeps the runtime's text: the S1 entry (the Rust interpreter and
the Rust runtime disagree) stays open for the specification to settle, and
the Grasm `run` now sides with the Rust `run`. Recorded as a Rust runtime
defect: a native program cannot tell an unknown host from a missing file.
On Linux, where the command does not run yet, a static executable has no
`strerror`; it will need the table then.

**(7) The by-index field take.** `take_child` (`compiler/mir/src/lower.gr`)
calls `grasm_struct_take(c, name)` and, when that answers null,
`grasm_struct_get(c, name)`: both find the field by comparing names
(`field_named`, `memcmp`), and `mut_method` calls it for every struct on the
path to a receiver changed in place, which the profile of a stage-2 step
session put at 60% of its samples (§6.22). The field's position is the
path's key (`kv`), as `read_child` and `write_child` already use it. Neither
runtime has a take by position, and the Rust runtime is not to change, so
the take is lowered as `struct_scalar_update` lowers a scalar store: when
`c` is laid out as a struct of its type's field count (`expect_struct`) and
nothing else holds it (`unique`), the field's word is loaded from its slot
and `grasm_box_bool(0)` -- the interned `false` both runtimes' takes leave --
stored in its place; otherwise `grasm_struct_get_at(c, k)`, which both
runtimes have. A field its type does not box takes that slow path too, as
the runtimes' take answers null for one. No other call in the backend finds
a field by name.
Held by `s3subset`, `objtest`, `fixpoint` (stage 3 still stage 2) and
`tirinterp`; measured before and after on the stage-2 command -- a `dap`
step session of `benchmarks/grasm/parmap.gr` and the whole `run` section --
interleaved, as a ratio.

**(8) What `--schedules` reports where Rust's would not run.** Nothing new:
property tests, contracts and doc examples run with no schedule, as in the
reference, and the flags are read as today (`parse_opts`).

#### The coordinator's decisions

Taken by the coordinator on 2026-11-19, on the note above; the owner may
reverse any of them.

- (1): (1a), the port of the reference's seeded scheduler -- a deterministic
  algorithm whose output (the replay lines) the gate must hold. On the
  conditions that `run` stays the same bytes on the whole corpus; that the
  two host switches more per turn are measured, A/B interleaved, on
  `taskfuzz`, `fuzz_rttasks` and the task and `par_map` benchmarks, the
  ratios reported, and the work stopped and reported if `run` of a
  task-heavy program is more than 10% slower on stage 1; and that the
  channel made in a task and handed on is fixed by it, with a fixture in the
  `schedules` section and an entry in docs/known-issues.md, found and fixed.
- (2): (2a), with spare host stacks made when a hop needs one and reused,
  not 200 made at the start of every block, unless making them on demand is
  measured to cost more; peak memory measured on `fuzz_rttasks` and
  `parmap` before and after; inside a task both stages must reach the
  guaranteed 10,000 calls, the count outside a task being the aim and not
  the requirement; stopped and reported above 10% on a task-heavy row.
- (3): accepted; the window recorded in docs/known-issues.md and here as a
  difference [E-RECURSION-DEPTH] allows.
- (4): accepted; the Rust interpreter's abort on an `==` 3,000,000 deep
  recorded as its defect, not copied.
- (5): accepted, since TIR text that grows with the square of the nesting
  is a defect and not a limit to record: `grasm tir` stays the same bytes,
  every reader of the command's own TIR is held by a round-trip gate or a
  check in the `stack` section, `fixpoint` holds, and whether an `else if`
  chain of 50,000 then runs is reported, with the reason if it does not.
- (6): the " (os error N)" text accepted for the six cases; **both (6a) and
  (6b) refused** -- a second lookup is a network query the program did not
  make, whose answer may differ and which breaks the rule of no network
  under the gates and under replay, and `EAI_NONAME` is a guess. A failed
  lookup stays the one counted exception, at most 1, recorded in
  docs/known-issues.md as the runtime ABI dropping `getaddrinfo`'s code
  (both runtimes), to be fixed in the Grasm runtime at or after S9. The
  floors: that case at most 1, every other `IoFailed` difference 0.
- (7): accepted; stage 2's `parmap` step session and the whole `run`
  section measured before and after, interleaved, as ratios.
- (8): accepted. The gate plan below accepted, its five controls added to
  `clicontrols`' count.

The coordinator's second decision (2026-11-19), on the stop the agent made
at step three under (1)'s condition. Measured on stage 1, the old command
against the new, three interleaved rounds, at a load of 4 to 5 from the
step-two gates -- median seconds of `grasm run`:

| program | old | new | new/old |
|---|---:|---:|---:|
| `pingpong` (the agent's: 200,000 rendezvous messages between two tasks, 200,000 into one buffered channel from eight) | 1.055 | 1.727 | 1.6375 |
| `blocks` (the agent's: 2,000 `concurrent` blocks of one task) | 0.032 | 0.067 | 2.0691 |
| `taskfuzz.gr -- 1 40` | 0.111 | 0.113 | 1.0231 |
| `taskfuzz.gr -- 2 40 flat` | 0.091 | 0.095 | 1.0440 |
| `taskfuzz.gr -- 3 40` | 0.109 | 0.112 | 1.0262 |
| `E-SCHEDULE` | 0.028 | 0.027 | 0.9853 |
| `E-ACTOR` | 0.029 | 0.028 | 0.9721 |
| `benchmarks/grasm/parmap.gr` | 120.276 | 119.847 | 0.9964 |
| `benchmarks/grasm/parwords.gr` | 2.955 | 2.845 | 0.9630 |

- (a) accepted: the port buys what the host's scheduler could not give --
  the reference's seeded orders, a task's panic as a test's failure,
  channels shared between tasks -- and the real task programs are within
  2 to 4%, `parmap` and `parwords` unchanged. The cost on the two
  switch-bound programs the agent wrote is a known cost, written here; they
  stay out of the gates.
- (b) accepted: spare host stacks only for a program that may recurse, the
  analysis conservative -- any cycle in the call graph, any call through a
  function or method value or anything the loader cannot resolve counts as
  possibly recursive; the `stack` section with a recursion through a
  function value and one through an interface's method, and a control that
  plants a too-optimistic analysis; the spare count for a program that may
  recurse chosen by measurement (21, 50, 100, 200 on both stages, against
  `taskfuzz`, the `E-` programs, `parmap` and `parwords`, interleaved): the
  largest that keeps every one within 10% of the old command on both
  stages; an in-task bound below the reference's recorded in
  docs/known-issues.md as a difference [E-RECURSION-DEPTH] allows.
- (c) not now: stage 2 measured on the same table first, stopped again only
  if a real task program is more than 10% slower there than on the old
  stage 2; the remaining cost per switch goes into "What is left for S8".

**The spare count, by measurement** (2026-11-19, the agent's, under (b)'s
rule). Each count built as stage 1 (by the Rust command) and stage 2 (by
that stage 1); three interleaved rounds against the old command of the same
stage, the order of the five commands rotated round to round; median
seconds of `grasm run`, and each count's ratio to the old command:

| stage 1 | old | 21 | 50 | 100 | 200 |
|---|---:|---:|---:|---:|---:|
| `taskfuzz.gr -- 1 40` | 0.126 | 1.0668 | 1.1429 | 1.1848 | 1.4423 |
| `taskfuzz.gr -- 2 40 flat` | 0.093 | 1.0039 | 1.0222 | 1.1780 | 1.6151 |
| `taskfuzz.gr -- 3 40` | 0.112 | 0.9766 | 1.0325 | 1.1701 | 1.5177 |
| `E-ACTOR` | 0.029 | 1.0616 | 0.9734 | 0.9844 | 0.9830 |
| `E-CONCURRENT` | 0.029 | 0.9918 | 1.0826 | 1.0771 | 0.9269 |
| `E-SCHEDULE` | 0.032 | 0.9412 | 0.8640 | 0.8963 | 1.0242 |
| `E-CHANNEL` | 0.031 | 1.0057 | 0.9702 | 1.0075 | 1.0347 |
| `E-DEADLOCK` | 0.029 | 1.0886 | 0.9507 | 0.9614 | 0.9207 |
| `parwords` | 2.954 | 0.9704 | 1.0611 | 0.9624 | 0.9710 |
| `parmap` | 118.906 | 1.0066 | 1.0014 | 0.9925 | 1.0231 |
| `pingpong` (the agent's) | 1.080 | 1.5878 | 1.6011 | 1.6487 | 1.6537 |
| `blocks` (the agent's) | 0.033 | 1.5056 | 1.4046 | 1.3330 | 1.5696 |

| stage 2 | old | 21 | 50 | 100 | 200 |
|---|---:|---:|---:|---:|---:|
| `taskfuzz.gr -- 1 40` | 0.129 | 1.0253 | 1.0543 | 1.3747 | 1.6525 |
| `taskfuzz.gr -- 2 40 flat` | 0.112 | 0.9605 | 0.9457 | 1.0453 | 1.3389 |
| `taskfuzz.gr -- 3 40` | 0.122 | 0.9891 | 1.0400 | 1.1680 | 1.5293 |
| `E-ACTOR` | 0.031 | 0.9560 | 0.9590 | 0.9604 | 1.0225 |
| `E-CONCURRENT` | 0.030 | 0.9552 | 1.3546 | 1.0094 | 1.0453 |
| `E-SCHEDULE` | 0.032 | 0.9576 | 0.9144 | 1.0847 | 1.0392 |
| `E-CHANNEL` | 0.032 | 0.9488 | 0.9632 | 1.0067 | 0.9185 |
| `E-DEADLOCK` | 0.034 | 0.8938 | 0.8662 | 0.8490 | 0.8527 |
| `parwords` | 4.308 | 0.9131 | 0.9313 | 0.9217 | 0.9181 |
| `parmap` | 171.673 | 0.8575 | 0.8538 | 0.9261 | 0.9176 |
| `pingpong` (the agent's) | 1.469 | 1.0718 | 1.0772 | 1.0808 | 1.0835 |
| `blocks` (the agent's) | 0.038 | 1.1221 | 1.1816 | 1.2045 | 1.1622 |

21 is the count: 50 puts `taskfuzz.gr -- 1 40` at 1.14 on stage 1. The
`E-` programs take a few tens of milliseconds, most of it the process's
start, and their ratios move by 10% from round to round either way (50's
1.35 on `E-CONCURRENT`, stage 2, among them); none of them recurses, so no
count gives them a spare. With 21, a task recurses to 10,999 calls on both
stages (500 on its own stack and 500 on each spare), below the Rust
interpreter's 41,941: recorded in docs/known-issues.md as a difference the
clause allows. On stage 2 no real task program is slower than on the old
stage 2 by more than 10% -- (c)'s condition -- and the two switch-bound
programs cost 1.07 and 1.12 of the old stage 2, where stage 1 pays 1.59 and
1.51: the remaining cost per switch is mostly the stage-1 build's search for
fields by name, which the Grasm backend's take by position (step two)
removes. `parmap` on stage 2 is 0.86 of the old stage 2 here, the field take
among the changes. The `blocks` program has no recursion, and so no spares;
its cost is the handing on of the shared state at a block's start and end.

#### How each is held to Rust

`clidiff` gains a section, `schedules`, both commands on the same input, the
counts asserted, a floor on a whole run:

- the five corpus programs with tasks, each with `test "main" { main() }`
  appended: `--schedules 20`, and `--schedule-seed` with five seeds,
  among them 0 and `u64`'s maximum;
- `taskfuzz.gr` wrapped as tests (`scripts/fixtures/sched/`): ten fuzz
  seeds of ten rounds each, plain, with `--schedules 20` and with three
  `--schedule-seed`s; its panic modes as tests of their own (a deadlock, a
  send and a close on a closed channel, a select of none, a recursion that
  never ends, each inside and outside a task);
- fixtures written for what the corpus lacks: a test that fails only under
  some orders (two senders racing to a buffered channel and an `assert` on
  who came first), so the `under schedule k of N` and `replay:` lines are
  compared and replayed; a panic, an error and an `exit` inside a task,
  with tests after them; a task that makes a channel and hands it on;
  nested blocks with waits across them; `--schedules 0`; a filtered run.

Cost, estimated from the Rust half (about a second a command): 150
commands, 5 to 10 minutes. `test`'s mains-as-tests and `dap`'s host panics
are then asserted zero, not counted (today 2 and 4). A section `stack`:
fixtures at the depths above, each by both commands under `run` and as a
test -- inside a task and outside, a value walked 1,000,000 deep, `^` and
`else if` chains of 10,000 and 50,000 -- with the window of (3) counted and
printed, never hidden; a few minutes. `journal`'s and `bundle`'s `IoFailed`
counts asserted zero, and a fixture that prints an `IoFailed` in `run`.
`scripts/lib/clicases.txt` gains the flags' edge cases (`--schedules 0`,
`--schedule-seed +5`, `u64`'s maximum and past it).

`clicontrols` gains: `sched-seed` (the scheduler ignores the seed: caught by
the fuzz tests' seeded orders), `task-panic-pass` (a task's panic reported
as the task ending well), `stack-frame` (the count one call higher: caught
by a fixture that recurses to exactly the count, refused by both),
`task-stack` (no spares: caught by a 30,000-deep recursion in a task),
`iofailed-old` (the runtime's text back). `fixpoint` must still find stage
3 the same as stage 2, `tirinterp`, `frontsmoke` and `clidiff`'s `run`
section hold `run`, and `run`'s time is measured against master as §6.22
measured it (twelve interleaved rounds of the seven serial benchmarks,
under 1.01 of master's geometric mean or the cause found).

**Order**: step two, the field take, alone and measured; then the
scheduler (steps three and four together, since (1) is both); then the
stack; then the wording; then the gates.

#### Steps two to seven, as built

(2026-11-19 and 20, this branch, the main checkout's compiler; loads of 3
to 6 from other work, so no time here is to be quoted but the interleaved
ratios; who decided what is said where it was decided.)

- **The by-index field take** (`0e04d168`, decision (7)): `take_field_at`
  in `compiler/mir/src/lower.gr`. On that commit `s3subset` "identical:
  648 mismatched: 0 timed out: 0 no TIR: 0" and `objtest` "objects: 648
  reproducible: 648 ... with problems: 0" (its `xs3subset` was stopped by
  hand when the agent stopped at step three, and run again on the final
  tree). Measured on stage 2, the base's against this commit's, three
  interleaved rounds: a `dap` step session of `benchmarks/grasm/parmap.gr`
  (`scripts/lib/dapscript.py step`), median 237.8 s against 228.0 s, 0.959,
  the same answer stream; and the whole `run` section, the corpus's 651
  programs with a `main` run by each, the two alternating which goes first
  program by program: 3,130.9 s against 2,542.8 s, 0.812, every output
  and status the same (`password-demo` 349 s to 132 s, `observe-demo` 391
  s to 234 s, `examples/parallel_map.gr` 403 s to 373 s).
- **The scheduler** (`tools/tirinterp/src/tasks.gr`, decision (1a)): the
  reference's rounds and seeded shuffle, its channel table, its rules for
  waiting and waking, `run_tasks`' ending. One change from the design,
  found while measuring: there is no scheduler task. The reference's loop is
  a state per open block in the shared table, and the task that gives way
  or ends runs it on and hands the next task its turn directly, one host
  switch a turn where a scheduler task made two. The table is taken out of
  the context for the length of each channel operation, and the context's
  new fields are its last: a build of the reference's compiler finds a field
  changed in place by its name, and a first build with them in the middle of
  the context spent four times the old command's share of samples in
  `memcmp` (and pushed every field `run` reads further down its search).
  `grasm test --schedules` and `--schedule-seed` run as the reference's
  runner does. The channel made in a task and handed on runs
  (`scripts/fixtures/sched/handoff.gr`).
- **The stack** (decisions (2) of the second, (3), (4), (5)): `value.show`
  and `value.equal` recurse on the host's stack as before to 64 levels and
  below that walk with stacks of their own (`d636ea31`; the first build
  walked every value so, which made printing a list of a million booleans
  about four times slower and timed out the `dap` section's step session of
  `sieve` -- found by the final stage-1 run; three interleaved rounds, the base's
  recursive walk, `71f47f6e`'s walk with its own stack all the way and
  `d636ea31`'s, median seconds: printing a list of a million booleans five
  times 1.596, 3.980 and 1.409, the `dap` step session of `sieve` 70.2,
  276.7 and 72.4, every output the same; a value nested a million deep
  still compares and prints) (`compare` and
  `key_text` recurse only through tuples, which the program's text bounds);
  the loader reads an expression on a fresh host stack every 400 levels and
  puts an operator node at such a level under an `OP_DEEP` node, which the
  evaluator runs on a fresh stack; `emit_flat` in `compiler/front` writes the
  TIR the command reads back unindented (`run`, `test`, `bench`, `dap` and a
  bundle's run), read by `sexp.read` as before -- `grasm tir`, `front tir`
  and the native build's TIR are the same bytes; and a block of a program
  that may recurse (`load.may_recurse`, conservative as the coordinator
  asked) has 21 spare host stacks (by measurement, above); a task recurses to
  10,999 calls on both stages. Peak memory (`/usr/bin/time -l`'s maximum
  resident set, in KiB): `taskfuzz.gr -- 1 40` 2,288 old and 2,352 new on
  stage 1, 2,128 and 2,160 on stage 2; `parmap` 44,384 and 46,960 on stage 1, 45,280 and 46,816 on stage 2. The `^` chain and the `else if`
  chain of 50,000 now run, and so do the values a million deep.
- **The `IoFailed` text** (decision (6), as the coordinator decided it):
  `os_words` in `tools/tirinterp/src/builtins.gr`, by `strerror_r` through
  `extern fn`.
- **The gates** (step seven, decisions as the coordinator accepted them):
  `clidiff` gains `schedules` and `stack`; the `test` section's and the
  `dap` section's host panics are gone, asserted by every comparison
  (nothing is allowed for them any more); the `journal` section allows a
  failed lookup's `IoFailed` text alone, at most once (`jnorm.py` exits 2 for
  it and 1 for any other); `scripts/lib/clicases.txt` gains the schedule
  flags' edges; `clicontrols` gains six controls (the coordinator's five and
  the one for the recursion analysis), its floor 36.

- **The timing of `run`** (§6.22's rule, the owner's): twelve interleaved
  rounds of the seven serial benchmarks, the base's stage 1 against this
  tree's, median ratios fib 0.970, sort 0.976, wordfreq 0.956, tree 1.027,
  sieve 0.995, matmul 0.955, mandelbrot 0.995 -- a geometric mean of 0.982,
  outputs the same.

- **The final runs** (2026-11-21 and 22, by the agent's resumable runner,
  `nice -n 10`, one gate at a time, no network; other apps held swap at 15
  to 18 GB throughout). On `c97650a7`, the final tree but for the control
  fixed below: `s3subset` "identical: 648 mismatched: 0 timed out: 0 no
  TIR: 0"; `objtest` "objects: 648 reproducible: 648 ... with problems:
  0"; `tirinterp` "identical: 651 mismatched: 0 timed out: 0 skipped: 0",
  "round trip: 1671 identical, 0 not", "panics: 25 identical, 0 not";
  `frontsmoke` "programs: 47 through tirinterp as grasm run: 47 compiled by
  s3, linked by grasm-link, as grasm run: 47 outside s3's subset: 0";
  `fixpoint` "identical: 54 differ: 0", "stage 2's TIR the same as grasm
  tir's: 6 of 6", "the command's build the stage pipeline's (its rt.o and
  the 4 programs): 5 of 5" (stage 2's `grasm` 4,732,408 bytes). The stage-1
  command, `stack`: "stack: 13 programs, 26 commands the same: 22 past the
  Rust interpreter's stack, within the count: 2 past a task's spare stacks:
  2"; `dap`: "dap: 1004 sessions compared the same: 966, 51 of them fixed
  ... the Rust adapter's own stack: 1 long, not stepped again: 14", "given
  the program's own run time: 0" (run alone, so without the `journal`
  section's recordings to launch: 45 fewer fixed sessions than a whole run).
  The whole `clidiff` with fixpoint's stage-2 command, 873 minutes, exit 0,
  every floor held: among its lines "run: 651 programs with a main the
  same: 651 timed out: 0", "test: 1737 programs, 1961 commands ... the
  same: 1960, 12 of them with a failing test both timed out, --contracts:
  1", "test, main as a test: 651 programs the same: 651, 15 of them
  failing", "schedules: 20 programs, 121 commands the same: 121, 24 of
  them with a failing test, 16 with a test held under every schedule tried
  a failing schedule's seed replayed alike: 1", the `stack` line as on
  stage 1, "bundle: 651 ... run alike, plain and with arguments: 651 (0
  run(s) differing only in the IoFailed wording) built twice the same:
  651", "journal: 652 programs recorded the same: 652, 185 of them with a
  journal (1 differing only in a failed lookup's IoFailed text, 1 only in
  socket handles after a net_close) ... native recordings replayed alike:
  45 of 45", "dap: 1043 sessions compared the same: 1001, 96 of them fixed
  ... the Rust adapter's own stack: 1 long, not stepped again: 18", "given
  the program's own run time: 1 (slowest: packages/gc-demo/src/main.gr step
  351 s, run 358 s)" and "lsp: 2050 sessions the same: 2050, 17 of them
  fixed". `fmttest` "formatted: 2027 unformatted: 0 does not parse: 5
  exempt: 1"; `gfmttest` "checked: 1538 disagreeing: 0"; `linttest`
  "linted: 2028 findings: 996 does not parse: 5 broke: 0". `clicontrols`
  "controls: 36 caught: 35 missed: 1": the six new controls caught, and
  `test-inverted` could not be planted, because the schedules loop had moved
  the line it changes; `38f44f71` plants it in the result's plain branch,
  and run alone there it is caught: "controls: 1 caught: 1 missed: 0"
  (the whole run, and so the floor of 36, was not repeated). `gatecheck` on `38f44f71`: "gate helpers checked: 587 undefined: 0
  corpus copied: 0 grep -q under pipefail: 0 gates not executable: 0".
  On stage 1, the sections `lines` to `bundle` and then `journal` passed
  on `71f47f6e` (`lines` "lines: 280 the same: 275 ...", `run` 651 the
  same, the `test` and `schedules` lines as above), and so did `xs3subset`,
  run again after it was stopped by hand at step three: "identical: 648
  mismatched: 0 timed out: 0 no TIR: 0". Those stage-1 runs came before
  the value walks' fix, which changes the code that prints and compares
  every value; on the final tree those sections ran on stage 2, above. The first final
  run on `71f47f6e` was hard-stopped twice by other apps' swap (free disk
  3,123 MB and 3,191 MB, 2026-11-20) and resumed per section, and stopped by
  hand when `dap` timed out on `sieve` (the walks' fix). Not run: the rest
  of `allgates.sh`, the Linux ELF gates.

**What is left for S8 after this part.** The scheduler's cost per switch:
on stage 1 the two switch-bound programs the agent wrote take 1.59 and 1.51
times the old command's time, on stage 2 1.07 and 1.12 (above) -- next to
the debugger's own cost per statement on stage 2 (about 84 s over parmap's
run, §6.22). A task's recursion bound of 10,999 calls, below the Rust
interpreter's (docs/known-issues.md). The failed lookup's `IoFailed` text,
the runtime ABI's, for the Grasm runtime at or after S9. The Grasm-runtime
command's struct-field and reference-count entry points (`grasm-j`,
§6.22). The LSP on the query engine and the wasm32 backend and its
playground. The Rust TOML reader's misreadings, kept until S9, and the Rust
defects this part and the ones before it copied or declined
(docs/known-issues.md).

### 6.24 S8 part seven: the wasm32 backend from MIR, and the playground built by it

Written 2026-11-22, before any code, as the design to build to. The plan
names this work three times: §4's architecture table ("a wasm32 backend from
MIR | replaces the Rust-built playground and gives a portable bootstrap
seed (§7)"), S8 ("the wasm32 backend with the playground built by it") and
§10's first decision ("previous-release binaries, with the wasm seed of §7
added once the wasm32 backend exists"). There is no §7 in this document
today: "Bootstrap and trust" was §7 from the plan's first commit
(`4df66f10`) until `449f2676` (2026-11-03, S7's exit items), which removed it
with no word in its message; §4 and §10 still point at it, and its last text
is `449f2676^`'s. The Rust toolchain stays stage 0 and the oracle and is not
changed; where its behaviour is wrong it is reproduced and recorded in
docs/known-issues.md, or, where it traps, recorded and not copied. The
decisions are numbered (1) to (12); each is proposed by the agent
implementing this part, and is the coordinator's to accept and the owner's
to reverse. Where there is a choice the alternatives are written under the
decision.

#### What was measured

Measured 2026-11-22 on this branch's base (`5ae2ad5f`) with the main
checkout's `grasm` (stage 0, the release of 2026-11-01), its wasm module
(`target/wasm32-unknown-unknown/release/grasm_wasm.wasm`, built at 08:44
that day from the crates' last commit, `0509ace4`, and the same bytes as
`ProjectGold/web/grasm.wasm`), part six's stage-1 and stage-2 binaries
(`fixpoint`'s, the same compiler sources), and `node` v25.2.1 (V8
14.1.146.11), the engine `wasmtest` runs. Every wasm module below that is
not the Rust one was assembled byte by byte by the measuring scripts, in
scratch; nothing was installed and nothing reached the network.

**The Rust playground.** `crates/grasm_wasm` (316 lines) is not a code
generator: it is the Rust interpreter -- `grasm_syntax`, `grasm_sema`,
`grasm_interp` and the journal decoder -- compiled by rustc 1.99.1 for
`wasm32-unknown-unknown`, with four hand-written exports around
`run_src_capturing_fault_with_stdin`. The module is 1,939,255 bytes
(564,375 under `gzip -9`): 1,713 functions, 1,592,298 bytes of code, 95,544
of data and a 248,326-byte name section. Its target features, as rustc
records them: bulk memory, multi-value, mutable globals, non-trapping
float-to-int, reference types, sign extension (and the encoding variants
`bulk-memory-opt`, `call-indirect-overlong`). Exports, with their types:
`memory` (18 pages at least), `grasm_alloc (i32) -> i32`, `grasm_free (i32,
i32)`, `grasm_run (i32, i32) -> i32`, `grasm_run_with_stdin` and
`grasm_run_with_journal (i32, i32, i32, i32) -> i32`. Imports: three,
`env.grasm_now_millis () -> f64`, `env.grasm_monotonic_millis () -> f64`,
`env.grasm_sleep_millis (f64)`; `dlsym`, which every embedder still supplies,
is no longer imported. The answer is one buffer of little-endian `u32`s --
its total length, the exit code, stdout's length and bytes, the diagnostic
text's length and bytes -- which the caller hands back to `grasm_free`.
V8 compiles it lazily: `WebAssembly.compile` takes 2.8 ms (median of seven).
Its embedders, the places that instantiate it: `scripts/wasm/run.mjs`
(wasmtest's and replaytest's), `site/playground.js`, `docs/wasm.md` and
four pages under `ProjectGold/` (`web/`, `33-`, `36-`, `41-`). The site
serves `site/grasm.wasm`, which is the module of 2026-10-27, not this one:
`site/` is regenerated at a release, and `scripts/site.py --check` lists it
among the files that are behind.

What it does with what it cannot do, measured on probes, each also run by
`grasm run`:

| program | `grasm run` | the Rust module |
|---|---|---|
| `par_map` of four | `[1, 4, 9, 16]` | the same, on one core |
| `parallel` with `spawn` (`E-PARALLEL-JOIN`, `E-SPAWN-SHARED`) | runs | the same bytes, on one core |
| `concurrent` (`E-CONCURRENT`) | runs | `panic: \`concurrent\` needs a platform that can switch stacks, which this one cannot`, 101 |
| `read_file`, `tcp_connect` | reads; `Connection refused (os error 61)` | `Err(IoFailed(operation not supported on this platform))` |
| `proc_run("echo", ...)` | runs | `IoFailed(echo: operation not supported on this platform)` |
| `random_bytes(4)` | 4 bytes | `IoFailed(cannot open /dev/urandom: operation not supported on this platform)` |
| `eprint` then a panic | both lines, then the stack trace | stdout only; the frame's text is the panic's line without the trace; the `eprint` line is lost |
| `1 + depth(n - 1)` | to 41,941 (§6.23) | to 173; at 174 a trap, `RuntimeError: memory access out of bounds`, no output, `run.mjs` exits 1 |

The last row is the Rust module's stack: it lives in linear memory, 1 MiB
by rustc's default for this target, and the interpreter's frames fill it at
174 calls -- a trap, which [E-RECURSION-DEPTH] says a program never meets, at a
depth 58 times below the 10,000 it guarantees. `wasmtest` sets
`E-RECURSION-DEPTH`, `E-PANIC-TRACE` and `scheme-demo` aside for it. The
Rust interpreter keeps a map as a `BTreeMap`, so the module hashes nothing
and has no seed.

**`wasmtest` today** (the main checkout's module, every program it runs
compared with `grasm run`, 2026-11-22,
38 minutes): "identical: 508   mismatched: 0   skipped: 143" of the 651
programs with a `main`. Set aside, by reason: a filesystem 82, threads
(`par_map`, `par_filter`, `spawn` named in the code) 24, sockets 21, a C
function 8, stack switching (`concurrent`) 4, the stack's depth 3, and
processes 1. The gate compares standard output and the status, not
standard error.

**The interpreters' speed, as a playground would meet it** (three rounds
interleaving the three, the order rotated, median seconds of a whole run, at
a load of 4.4 to 5.1 from other work, so ratios only; every output the
same): the Rust module under node, and stage 2's `grasm run` (the Grasm
interpreter built by the Grasm backend, natively), each over the Rust `grasm
run` --

| program | Rust `grasm run` | stage 2 `grasm run` | the Rust module |
|---|---:|---:|---:|
| `fib` | 1.786 | 0.889 | 1.631 |
| `sieve` | 3.045 | 1.713 | 1.488 |
| `matmul` | 6.510 | 1.442 | 1.491 |
| `sort` | 0.413 | 4.250 | 2.003 |
| `tree` | 1.594 | 1.243 | 1.604 |
| `wordfreq` | 0.547 | 1.926 | 1.563 |
| `examples/hello.gr` | 0.014 | 0.951 | 7.007 |
| `examples/effects.gr` | 0.013 | 0.976 | 9.630 |

The wasm build of the Rust interpreter costs 1.5 to 2.0 times its native
build; the Grasm interpreter is already up to 4.25 times the Rust one
natively, on `sort`. If wasm costs it what it costs Rust's, the Grasm-built
playground will be slower than the Rust module on such programs -- part
eight's to measure, not this part's to promise. A program that does nothing
takes 0.10 s under node, most of it node's start and the module's
compilation.

**The Rust command and `--target wasm32`.** `grasm build --native --target
wasm32` (or `wasm32-unknown-unknown`) answers "error: unsupported target
`wasm32`: Support for this target has not been implemented yet", exit 1:
no command builds a program for wasm today, although the site's front page
(`scripts/site_pages.py`) shows "Wasm `--target wasm32`" beside the native
build.

**The engine.** Each feature by a module of a few bytes, validated,
instantiated and run: accepted, and computing the right answer, are sign
extension, non-trapping float-to-int (NaN to 0 and saturation, as
[L-LIB-AS-I64] asks of a conversion), multi-value, bulk memory, reference
types, tail calls (a `return_call` a million deep), exceptions in both
encodings (the legacy `try`/`catch` and the standard `try_table` with
`exnref`), threads (a shared memory and an atomic add), SIMD, memory64,
extended constants, and JS promise integration (`WebAssembly.Suspending`
and `promising`). Behind flags that are off: core stack switching
(`--experimental-wasm-wasmfx`, `--experimental-wasm-stack-switching`) and
growable JSPI stacks. A browser page cannot set a flag.

- *The native stack.* A function of one `i64` argument and four `i64`
  locals recurses 10,405 deep under node's default stack (98,304 with
  `--stack-size=8000`, which a page cannot ask for either), about 94 bytes
  a frame.
- *A stack overflow cannot be caught by wasm.* An unbounded recursion inside
  a `try_table` with `catch_all` is not caught: JS sees `RangeError: Maximum
  call stack size exceeded`, and the instance can be called again after.
- *Address width*, the price of a word held as `i64` (a loop doing nothing
  but loads, five interleaved rounds, medians): `i32` addresses 0.547 ns a
  load; `i64` words wrapped to `i32` at each access, the 64-bit layout in a
  32-bit memory, 0.631 (1.154); memory64, 0.533 (0.976).
- *Mathematics.* 20,000 seeded arguments per function, three libms against
  each other, bit for bit. macOS's (what `grasm run` and the native builds
  call) against V8's `Math`: sin 886 differ, cos 916, tan 8,293, exp 1,958,
  log 56, log2 27, log10 605, asin 1,676, acos 3,412, atan 0, sinh 5,267,
  cosh 5,012, tanh 1,133, cbrt 2,198, pow 0, atan2 3,567, hypot 6,631. The
  Rust module's own (Rust's `libm`, a port of musl's, which rustc links into
  a `wasm32-unknown-unknown` program; measured by a four-function probe
  rustc built for wasm32 in scratch): against macOS sin 883, tan 8,324, exp
  1,874, hypot 0; against V8 sin 279, tan 233, exp 11, hypot 6,690. Three
  libms, each differing from the other two. `wasmtest` agrees today because
  no corpus program prints a transcendental answer to its last bit.
- *WASI*: `node:wasi` (preview 1, 46 functions) is built in, and prints an
  `ExperimentalWarning` to stderr when loaded.
- *Tools*: no `wasm-tools`, `wabt`, `binaryen` or `wasmtime` on the machine.
  Homebrew's LLVM 21.1.8, which `elfrtswap` and `objtest_elf` already use,
  has the `wasm32` target in `llvm-mc` and reads wasm in `llvm-objdump`: an
  assembler to hold an encoder to and a second decoder, but no validator.
  musl's mathematics is on the machine as source, as Zig 0.16 ships it
  (`lib/libc/musl/src/math` and its ports in `lib/compiler_rt` and
  `lib/std/math`).

**MIR, as the backend would meet it.** A measuring program (in scratch, on
`compiler/mir`) built every corpus program's MIR as `s3` builds it --
`mir.build` with inlining at 32 -- and walked it:

| | the 651 programs with a `main` | runtime | `front` | `tirinterp` | `s3` | `link` | the command |
|---|---:|---:|---:|---:|---:|---:|---:|
| functions | 36,107 | 1,037 | 731 | 394 | 640 | 211 | 2,096 |
| blocks | 1,382,292 | 10,623 | 54,563 | 20,175 | 71,498 | 33,097 | 176,187 |
| irreducible functions | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| most values in one function | not taken | 2,094 | 20,062 | not taken | 128,755 | 273,516 | not taken |
| most blocks in one function | | 215 | 1,101 | 816 | 2,944 | 4,726 | 2,944 |

Every function is reducible -- 35,132 retreating edges in the corpus, each
to a block that dominates its source -- so a structuring for reducible
graphs is enough, and an irreducible one can be refused as the backend's
own error. The operations wasm has no instruction for, by the programs that
hold them: the stack pointer for the recursion floor (`getsp`) in 175
programs, 1,219 functions taking the floor; a frame slot's address
(`slotaddr`, an `inout` argument's home) in 491, 153,293 times; a call
through a code address in 98 (530); a function's address in 208; a C
function's address in 2; atomics in 2 programs and 323 times in the
runtime; process cells in the runtime alone. The values: V8 refuses a
function with more than 50,000 locals, and `s3` and `link` each have one
with more than 128,000 values -- a local per value cannot build the
compiler, and would make every frame large. The programs call 296 of the
runtime's functions, and C functions of their own through `extern fn`: SDL
(the three programs set aside everywhere), and `abs`, `atoi`, `free`,
`getpid`, `ioctl`, `isatty`, `memcmp`, `pow`, `read`, `sqrt`, `strchr`,
`strlen`, `system`, `tcgetattr`, `tcsetattr`, `write`, in the programs
`wasmtest` sets aside as "calls a C function".

**The runtime in Grasm and its host.** `compiler/rt` (23,104 lines) calls
95 C functions: memory (`malloc`, `calloc`, `realloc`, `free`,
`posix_memalign`, `mmap`, `munmap`, `mprotect`, `memcpy`, `memset`,
`memcmp`, `memchr`, `strlen`, `sysconf`); the 24 of libm; clocks
(`clock_gettime`, `nanosleep`); `getentropy` (the maps' seed, drawn once,
and nothing else -- `random_bytes` reads `/dev/urandom`); `getenv`;
`write` and `read` on the standard streams; files and directories (12);
sockets, name lookup and readiness (18); processes (`fork`, `execvp`,
`waitpid`, `kill`, `pipe`, `dup2`, `_exit`); threads (`pthread_*`, 10, and
`sched_yield`); and `dlsym`, `strerror`, `__error`. Fibers switch with
`grt.ctx_switch`, a leaf each target's backend writes. Compiling it: `grasm
tir` 3.3 s, then stage 2's `s3 --runtime` 1.8 s. Peak memory of each
compiler program's build by stage 2 is under 210 MB (`fixpoint`'s
`cost.tsv`), far inside wasm32's 4 GiB.

#### What the plan asks, and what each half costs

What "a wasm32 backend from MIR" replaces is an interpreter compiled to
wasm by rustc, so the plan's row holds two things:

**(a) The backend**: any Grasm program in the subset compiled from MIR to a
wasm module, with the runtime compiled for wasm beside it. New code: a
target package beside `compiler/aarch64` and `compiler/x86_64` --
structuring, locals, selection, the module writer, its encoder's cases --
about 2,500 to 3,000 lines; the runtime's host on wasm, written in Grasm --
memory over `memory.grow`, the string functions, 21 functions of libm,
the refusals -- about 3,000, two thirds of it libm; the driver's target and
the gates. It needs no register allocator: wasm's locals are the virtual
registers, and the engine allocates.

**(b) The playground**: `compiler/front` and `tools/tirinterp` -- the Grasm
interpreter, which `grasm run` in Grasm already is -- compiled by (a) into a
module with the Rust module's exports. On top of (a): a package for the
exports and the frame, the interpreter taught to run where no stack can be
switched (it hops to fresh host stacks every 500 calls and runs a program's
tasks as host tasks, §6.23), the frontend reading the one source from
memory (`load_sources`, which the language server already uses), about 500
to 800 lines; and the comparisons with the Rust module.

(b) cannot exist without (a); (a) without (b) leaves the Rust module as the
playground. The plan's row implies both, and so does the brief.

**The seed** is (a) applied to the compiler: `compiler/grasm`'s build path
(the frontend, MIR, `s3` with its native targets, `link`) as one wasm
module, run by any WASI preview-1 runtime -- `node:wasi` here -- that reads
the compiler's sources and writes a native stage 1 for the machine it runs
on, whose stage 2 must be the release's stage 2 to the byte. It needs from
(a) the locals shared by liveness (the 273,516 values of `link`), a host
over WASI's files, arguments and clock instead of the playground's, and a
gate that is itself a bootstrap. It is S9's ("the bootstrap documented").

#### The decisions

**(1) Scope: (a) in this part, (b) in the next, the seed in S9.**
Proposed. This part builds the backend and the runtime for wasm and holds
every corpus program `wasmtest` can compare, compiled by it, to `grasm run`;
part eight builds the playground module with it and holds that to the Rust
module and to `grasm run`; the seed waits for S9. The alternatives:
**(1b)** (a) and (b) in one part, which doubles a part that is already the
size of S7's first; **(1c)** (b) alone, the interpreter's module built by a
backend tested on one program, refused -- a backend is trusted by the
corpus, not by one program.

**(2) The target: the core features of wasm 2.0 and exception handling,
nothing else.** Proposed: sign extension, non-trapping float-to-int, bulk
memory and mutable globals (each among the features rustc already enables
for the Rust module, so nothing is asked of an engine that it does not ask
today; MIR returns one value, so multi-value is not needed), and exception
handling in its standard encoding (`try_table`, `throw`, `exnref`) for one
thing only: an `exit` or a panic, deep in a program or in the interpreter,
unwinds to the export that started the run, which writes the frame and
returns. Not used: tail calls (no Grasm construct needs one), threads (a
shared memory needs a cross-origin isolated page, and `par_map` runs on one
core as in the Rust module), SIMD (a later vectorizer's), memory64 (decision
(3)), GC, JS promise integration and stack switching (decision (4)). The
alternatives: **(2b)** the legacy encoding of exceptions, which V8 now keeps
behind a flag that is on by default and the standard has replaced --
refused; **(2c)** no exceptions: `exit` and a panic call an import that
throws in JS, and the embedder catches it and asks an export for the frame
-- every embedder's glue changes, against decision (9), refused. What
engines other than this V8 accept could not be measured here (no network, no
other engine), and is the one fact this decision rests on that is not
measured.

**(3) Addresses: wasm32, with the runtime's 64-bit layout.** Proposed.
Every word stays eight bytes -- `RtVal`, the 40 layout constants `rtlayout`
holds, TIR -- and an address is an `i64` whose upper half is zero, wrapped
to `i32` at each load and store (an offset that fits folded into the
instruction's own). `compiler/rt` and `compiler/mir` do not change for
wasm. The cost is a wrap per access: 1.154 on a loop of nothing but loads.
The alternatives: **(3b)** memory64: no wraps (0.976 on the same loop),
the same layout, no change anywhere -- but it is wasm64, not the plan's
wasm32, and whether every browser runs it could not be checked here;
the owner's to choose, and the backend would differ only in the memory's
limits and the address type of each access. **(3c)** a 32-bit layout:
every layout constant, `rtlayout` and the runtime's arithmetic on words
change for one target -- refused.

**(4) Tasks: refused as the Rust module refuses them.** Proposed. A
`concurrent` block ends the program with the Rust module's words, `panic:
\`concurrent\` needs a platform that can switch stacks, which this one
cannot`, exit 101: the wasm backend writes the context switch's leaves
(`grt.ctx_switch`, `grt.ctx_init`), as each target does, as a panic with
that text, so `compiler/rt` does not change; whether anything is printed
between a block's start and its first task's start, where the two would
differ, is checked by a fixture. `par_map` and `par_filter` run on one core,
as in the Rust module: with one processor the runtime's `workers_for`
answers one worker and no thread is started. A `parallel` block's
`spawn` runs on one thread in the Rust module; what the Grasm runtime does
where no thread can start is measured on a fixture, and made the same, before
the gate runs. (`wasmtest` sets aside every program naming `par_map`,
`par_filter` or `spawn` as needing threads, which the Rust module runs; the
new gate keeps the rule, as the brief asks, and the comparison with the Rust
module in part eight covers them.) The
alternatives, each for a later part: **(4b)** an asyncify-style lowering in
our own backend, every function that may reach a switch saving its locals
to the shadow stack and returning, then re-entered -- tasks everywhere,
for code size and time to be measured; **(4c)** JS promise integration:
exports that return promises, an asynchronous ABI, and V8 alone among the
engines measured; **(4d)** core stack switching, once an engine ships it
unflagged.

**(5) The stack: a shadow stack in linear memory, and the recursion floor
on it.** Proposed. A frame slot (`slotaddr`) lives on a shadow stack in
linear memory, as rustc's own, its pointer a global; `getsp` reads that
pointer and the stack floor ([E-RECURSION-DEPTH]) is checked against it, so
a recursion too deep ends in the runtime's panic with its words and status
-- never a trap -- provided the shadow stack runs out before V8's own: a
function that takes the floor reserves a shadow frame at least as large as
the native frame it will cost (its locals' count, by a rule measured on the
`stack` fixtures under node's default stack), and the shadow stack's size
is chosen so that its floor comes first. The depth each shape then reaches
is measured, printed by the gate and recorded in docs/known-issues.md (the
Rust module: 173 calls and a trap). The
three programs `wasmtest` sets aside for depth stay aside, for the same
reason. The alternative: **(5b)** no floor, a RangeError as the Rust
module's trap -- refused, a trap is what the clause rules out.

**(6) The runtime's host on wasm: a C library in Grasm, inside the
module.** Proposed. `compiler/rt` is compiled for wasm unchanged but for
`os.gr`, replaced as `os/linux.gr` replaces it for Linux (§6.18), and its
`extern fn` calls are bound by the backend to a package of Grasm functions
of the same names, `compiler/rtwasm`, compiled into the same module -- a
name it lacks is a build error naming it, never an import. In it:

- *memory*: `malloc` and its family and `mmap`, `munmap`, `mprotect` over
  pages taken by `memory.grow` (never given back, as wasm cannot);
  `memcpy` and `memset` as `memory.copy` and `memory.fill`; `memcmp`,
  `memchr`, `strlen` as loops; `sysconf` answers one processor;
- *mathematics*: of the 24 functions of libm the runtime calls, `floor`,
  `ceil` and `sqrt` are instructions; the other 21 as musl writes them,
  ported to Grasm from the source on the machine, and held bit for bit to
  the Rust module's own (a probe module rustc builds in the gate's scratch,
  the playground's libm by construction), so the playground answers as it
  answers today.
  Decision 2 of §10 (a correctly rounded libm everywhere) is not this; this
  is the first libm in Grasm, and the one the playground already has;
- *the seed of the maps*: `getentropy` asks a fourth import,
  `env.grasm_entropy(ptr, len) -> i32`, which every embedder answers from
  `crypto.getRandomValues` (a browser's and node's); its failure is the
  runtime's existing refusal to run without a seed. No seed from a clock and
  no fixed seed: the maps stay resistant to flooding. Seven embedders gain
  that line, and `wasmtest`'s import check finds any that does not;
- *clocks*: the three imports the Rust module has, unchanged;
- *the standard streams*: a program's output goes to an import,
  `env.grasm_write(fd, ptr, len)`, for a program built by (a); the
  playground of part eight keeps it in memory for the frame instead;
  standard input is the lines the embedder hands over;
- *what wasm cannot do*: files, directories, sockets, name lookup,
  processes, threads and `dlsym` fail with one error number of the
  package's own, which `strerror` words "operation not supported on this
  platform", so every `IoFailed` the program sees is the Rust module's text
  (`random_bytes` included: the runtime already words its failure "cannot
  open /dev/urandom: " and the system's message, as the Rust module's
  reads); `pthread_create` fails; thread-local and process cells are plain
  linear memory, one thread having them all;
- *`exit` and the end of a panic*: `_exit` throws the exception of (2) with
  the status, which the module's export catches.

The alternatives: **(6b)** the C library as imports in JavaScript -- every
embedder carries it, and `Math`'s answers are V8's, which differ from both
the Rust module's and `grasm run`'s (above); refused. **(6c)** WASI for
the host from the start: the seed's host, but a browser has no WASI and
the playground's page would carry a shim; for S9.

**(7) One module per program, linked by the backend.** Proposed. `s3
--target wasm32` reads the program's TIR, the runtime's (`--runtime-tir`)
and `compiler/rtwasm`'s, builds their MIR, binds every runtime call and
`extern fn` by name, keeps what the exports reach and writes one module.
The runtime is compiled with every program: about 5 s each (3.3 s of `grasm
tir`, which a gate does once and reuses, and 1.8 s of MIR and code). The
alternative, **(7b)**, a relocatable wasm object for the runtime and a
wasm linker in `compiler/link`, waits until that time is measured to
matter.

**(8) The command: not in this part.** Proposed. Part seven adds `s3
--target wasm32` and the gates; `grasm build --native --target
wasm32-unknown-unknown` in the Grasm command comes with the playground in
part eight, which is built by it, with `clidiff` gaining a category as
`runtime` was (§6.17): the Grasm command writes a module where the Rust one
refuses the target. The alternative: the command now, with the wasm
runtime's TIR beside the command as `rt.o` is.

**(9) The playground's ABI, for part eight: the Rust module's, byte for
byte, but one import.** Proposed now so that part seven builds toward it:
the five exports with the Rust module's types, `memory` exported, the
frame's layout, the exit codes and the frame's text as the Rust module
gives them (a panic's line alone). One import more, `grasm_entropy`
(decision (6)). Two things the Rust module does that part eight must decide
on, recorded now as the Rust module's defects: an `eprint`'s text is lost,
and a recursion of 174 calls traps.

**(10) The structure, the locals, and what wasm lacks.** Proposed, as the
backend's design. Structured control from the reducible graph by the
dominator tree (Ramsey, "Beyond Relooper", ICFP 2022): a loop header opens a
`loop`, a block reached by two or more forward edges is placed just after
the end of a `block` that those edges leave by `br`, every other branch is a
`br` or `br_if` to its enclosing label, and an irreducible function is the
backend's own error. The dominator tree is the backend's own (the iterative
algorithm of Cooper, Harvey and Kennedy, as the measuring program used;
`compiler/mir` builds SSA without one). Locals shared by liveness, per class
(`i64`, `f64`), so that no function nears V8's 50,000 and frames stay small.
Block arguments as parallel copies, a cycle broken through a spare local.
What wasm lacks, by MIR operation: `sdiv` of the most negative number by -1
is tested first (wasm traps; [E-ARITH-INT] answers the number), as x86-64's
selection does; `rbit` and `bswap16/32/64` are shifts and masks;
`fcvtzs`/`fcvtzu` need nothing, the saturating instructions being exactly
what [L-LIB-AS-I64] asks; atomics as plain loads, stores and adds, one
thread being all there is; `fnaddr` and `gotaddr` as an index into the
module's table, `callind` as `call_indirect` with the call's type; strings,
descriptors and cells as data at fixed addresses. The functions are named in
the module's name section, so a trap's stack names Grasm functions.

**(11) Held by four checkers of every module.** Proposed: V8 validating
it whole (`new WebAssembly.Module`, which validates every function before
any runs); a validator written for the gate in Python, the specification's
validation algorithm for every instruction the backend emits, typing every
function body, independent of V8; `llvm-objdump` decoding it; and the
encoder's every form held to `llvm-mc -triple=wasm32` as `encodetest`
holds arm64's to `clang` and `encodetest_x64` x86-64's to `as`.

**(12) §7 restored.** Proposed: `449f2676^`'s "Bootstrap and trust" put
back as §7, word for word, with its "Any platform, any commit (later)"
bullet pointing here and to S9, so that §4's and §10's references point at
what they meant. The alternative: point them here.

#### The coordinator's decisions

Taken by the coordinator on 2026-11-22, on the note above; the owner may
reverse any of them.

- (1): accepted. Part seven builds (a) -- the backend, `compiler/rtwasm`
  and the runtime compiled for wasm; part eight builds (b), the
  playground; the seed is S9's.
- (2): accepted: the core features of wasm 2.0 and exception handling in
  its standard encoding (`try_table`, `throw`), used only to unwind an
  `exit` or a panic to the export.
- (3): (3a), wasm32 with the runtime's 64-bit layout, each address wrapped
  at its access; the 1.154 of the loop of loads recorded as measured.
  Memory64 (3b) declined for now, the plan saying wasm32, and left as a
  later option.
- (4): accepted as proposed; `spawn` measured on a fixture first and made
  the same as the Rust module's.
- (5): accepted. The aim is the clause's guaranteed 10,000 calls under
  node's default stack, never node's stack raised by a flag (a page cannot
  raise it); the depth reached measured, and if it falls short, recorded in
  docs/known-issues.md with the number as a difference the clause allows.
  A recursion too deep is always the panic, never a trap. (Corrected by the
  coordinator on review: a depth short of 10,000 is not a difference the
  clause allows -- it guarantees 10,000 -- so the shortfall is recorded as
  an open S3 defect of `compiler/wasm32`.)
- (6): accepted: `compiler/rtwasm` in Grasm, libm ported from musl and held
  bit for bit to the rustc-built probe of the Rust module's libm, and
  `env.grasm_entropy` with no fallback. What replaces the musl port in the
  end is the plan's own correctly rounded libm in Grasm (§4's Math row,
  §10's decision 2), which is not this part.
- (7): accepted, one module per program; its cost per program measured and
  reported.
- (8): accepted: `s3 --target wasm32` only, in this part.
- (9): accepted, for part eight; what becomes of an `eprint`'s text is part
  eight's to decide.
- (10): accepted.
- (11): accepted, on the condition that the Python validator refuses every
  bad module `wasmcontrols` plants, and that it cannot pass by checking
  nothing: the count of function bodies it checked is asserted.
- (12): accepted: §7 restored from `449f2676^` in a commit of its own,
  whose message says `449f2676` deleted it without a word; then §4's and
  §10's references checked to resolve.
- On the gates, one addition: `wasmtest` sets aside every program that
  names `par_map`, `par_filter` or `spawn`, which the Rust module runs;
  `wasmsubset` does not inherit that blind spot. Those programs are held
  too, to `grasm run` and, where the Rust module runs them, to the Rust
  module, their counts reported apart; the floor of 508 stays. Removing the
  blind spot from `wasmtest` itself, if done, is a gate change in a commit
  of its own, its floor moving with it.

#### How it is held

- **`wasmsubset`**, new: every corpus program `wasmtest` compares -- the 508
  of its run above, every one in `s3`'s subset of 648 -- compiled by `s3
  --target wasm32` from `grasm tir`, checked by the four checkers of (11),
  run by node through `scripts/wasm/runprog.mjs` (the imports of (6)), its
  stdout and status compared with `grasm run`'s (as `wasmtest` compares) and
  its stdout, stderr and status with the same program built by the Grasm
  backend for arm64 (the same runtime, so the same panic text), built twice
  and the two modules compared. The programs set aside are exactly
  `wasmtest`'s, for its reasons -- the rules moved into
  `scripts/lib/wasmskip.sh`, which both gates source, so they cannot drift
  -- the three SDL programs among its eight that call C; no new reason.
  Counts asserted, the floor the 508 compared today. Estimated at 30 to 50
  minutes (the runtime's TIR made once; about 3 s of compiling and a run per
  program), to be measured.
- **`encodetest_wasm`**: the encoder's forms against `llvm-mc`, a minute.
- **`fuzz_wasmmath`**: the 21 functions against the Rust module's libm, bit
  for bit, a million arguments each from seeded generators that reach
  every branch (the reductions of `sin` far from zero, `pow` near
  overflow, subnormals, NaNs and infinities).
- **`fuzz_wasm`**: `fuzz_s3` and the generators with the Grasm backend as an
  engine, pointed at the wasm target, as `fuzz_x64` points them at x86-64.
- **The stack**: fixtures recursing until refused, by shape, under node's
  default stack: each must end in the runtime's panic, never a RangeError,
  the depth reached printed.
- **Speed and size**, measured, not claimed: the serial benchmarks
  compiled to wasm run by node, against the same programs built by the Grasm
  backend for arm64 and against `grasm run`, interleaved, as ratios; the
  modules' sizes.
- **`wasmcontrols`**, each a planted fault the gates must catch: an
  instruction miscoded (`i64.sub` for `add` in one selection), a branch's
  label one level off in the structuring, a refused feature run anyway (the
  leaf of (4) switching nothing and returning), a host error worded
  otherwise than the Rust module's, a module built twice differently (the
  build's clock written into a custom section), a module the validators must
  refuse (a local's type changed), and the floor of (5) removed (a
  RangeError, not the panic). The frame's layout off by a field is part
  eight's control.
- **What must not move**: `s3subset`, `objtest`, `xs3subset`, `fixpoint`
  and `tirinterp` as today, and every arm64 and x86-64 object the subset
  and the runtime compile to the same bytes from this part's `s3` as from
  master's, as S7 held its refactor.

#### Where the first part ends

Part seven ends when (a) holds: `s3 --target wasm32`, `compiler/rtwasm`, the
runtime compiled for wasm, and every corpus program `wasmtest`'s rules let
run, compiled by it, agreeing with `grasm run` under node, with the gates and
controls above, and the native targets unchanged. Part eight is the
playground: the module of (b) with the ABI of (9), built by `grasm build
--native --target wasm32-unknown-unknown`, held to the Rust module on every
program and to `grasm run` on `wasmtest`'s set, its size and speed against
the Rust module's, and `site/` and `ProjectGold/` switched to it only on the
owner's word. The seed is S9's.

**What is left for S8 after this part, if it ends there**: the playground
(part eight); the LSP on the query engine; the debugger's cost per
statement; `grasm-j`; the scheduler's cost per switch; a task's recursion
bound of 10,999 calls (§6.23).

#### Step two, as built

(2026-11-22, this branch, the main checkout's compiler and node v25.2.1;
loads of 3 to 5 from other work, so no time here is to be quoted but the
interleaved ratios; who decided what is said where it was decided.)

- **What was built.** `compiler/wasm32` (2,404 lines): the structuring,
  locals shared by liveness, the selection, the module writer, the link of
  the three programs' MIR, the encoder's cases. `compiler/rtwasm` (2,514
  lines): the C library -- memory over `memory.grow` with a free list per
  size class, the standard streams and clocks through the host, the
  refusals, and libm (1,867 of the lines). `s3 --target wasm32
  --runtime-tir R --libc-tir L` (118 lines of the driver), and
  `scripts/wasm/runprog.mjs`, the runner. The module exports `memory`,
  `_args` (where the runner writes the argument vector) and `_start(argc)`;
  its imports are `env.grasm_write`, `grasm_entropy` and, where the program
  reads a clock, the three the Rust module has.
- **Found while building, each settled here.** The runtime ends a program
  through C's `exit`, which it looks up by name (`io.leave`'s
  `dlsym(RTLD_DEFAULT, "exit")`): the library's `dlsym` answers that one
  name with a table slot and every other with null. A failed name lookup is
  worded `ENOENT` by the runtime whatever the lookup answered
  (docs/known-issues.md), so `strerror` words every number as the refusal:
  every error here is one. The libm on the machine to port from was the
  `libm` crate 0.2.16, a copy of the code rustc links: its functions agree
  with the playground's bit for bit but for `hypot`, where 5 of 2,000 pairs
  differed by an ulp -- the playground's `hypot` is correctly rounded (20,000
  random pairs, every answer the exactly rounded one, by an exact BigInt
  square root), a newer libm's than the copy's -- so `hypot` here is written
  correctly rounded: a candidate from `sqrt`, each midpoint beside it
  compared with x^2 + y^2 exactly by a non-overlapping expansion (Shewchuk,
  1997), a tie to the even neighbour.
- **The stack, by measurement (decision (5)).** With no shadow frames, V8's
  frame for a function was about 8 bytes a parameter or local and 55 more
  (94.6, 157.6, 110 and 126 bytes for four shapes of 5, 13, 7 and 10 slots;
  10,404 calls of `1 + depth(n - 1)` before the engine's stack overflowed).
  A shadow frame of 6.5 times that left too little of the engine's stack
  for the panic itself (`plain` trapped at 10,081); 7.0 is the scale, every
  recursion fixture then ending in the panic. A recursion through a
  function value and the runtime's `map` (`closure`) still trapped while
  only the frames that take the floor or call through a value kept shadow
  frames -- the runtime's own frames are in every level -- so every frame
  that calls anything keeps one. The depths each shape then reaches under
  node's default stack, the floor met first and the panic printed:
  `mutual` 10,089, `plain` 9,368, `doubles` 8,030, `tree` 7,026, `locals`
  5,620, `closure` 3,390 -- below the clause's 10,000 for all but one,
  recorded in docs/known-issues.md as the coordinator's decision asked (the
  Rust module: 173 on `plain`, and a trap).
- **The link's cost (decision (7)),** three runs each: a module 0.54 to 0.62
  s for `hello`, `tree` and `asmcheck`, where their arm64 objects take 0.012
  to 0.09 s -- the runtime's and the C library's MIR made with every program
  -- and 3.1 s for `b28-demo` against 5.1 s for its object (no register
  allocation). Modules: `hello` 119,103 bytes, `tree` 90,414, `b28-demo`
  1,151,105.
- **Speed (decision (5)'s frames included),** three rounds interleaving the
  three, the order rotated, median seconds of a whole run, the serial
  benchmarks at ten times `bench.sh`'s sizes (`fib(35)`, a sieve to
  20,000,000, 400 by 400, 4,000,000 sorted and counted, 200 trees) so that
  node's start -- 0.07 to 0.08 s for `hello`'s module -- weighs less; at a
  load of 4.3, so ratios only, every output the same: the module under node
  over the program built for arm64 by the Grasm backend (stage 2's `grasm
  build --native`) and by Cranelift --

  | program | wasm | Grasm arm64 | Cranelift | wasm / Grasm | wasm / Cranelift |
  |---|---:|---:|---:|---:|---:|
  | `fib` | 0.146 | 0.075 | 0.051 | 1.951 | 2.842 |
  | `sieve` | 0.451 | 0.131 | 0.068 | 3.432 | 6.597 |
  | `matmul` | 0.417 | 0.106 | 0.032 | 3.945 | 12.963 |
  | `sort` | 0.273 | 0.094 | 0.110 | 2.895 | 2.489 |
  | `tree` | 0.559 | 0.534 | 0.177 | 1.047 | 3.160 |
  | `wordfreq` | 1.474 | 1.022 | 0.143 | 1.442 | 10.284 |

  Geometric means 2.206 over the Grasm backend's arm64 code and 5.195 over
  Cranelift's. Where the time goes was not measured: every address wrapped,
  every value a local read and written, no folding of an offset into its
  load, and node's start are the candidates, and none is promised here. At
  `bench.sh`'s own sizes the same modules take 0.088 to 0.217 s, against
  the Rust interpreter's 0.41 to 6.14 s for the same programs.
- **The native targets, unchanged.** Six programs' objects for arm64 and
  x86-64 and the runtime's for both, from part six's stage-1 `s3` and from
  this tree's: the same bytes, 14 of 14 (`s3subset`, `objtest`, `xs3subset`
  and `fixpoint` below hold the rest). The command built by Cranelift grows
  by the wasm target's code, which it never calls.
- **A tool's fact.** llvm-mc 21.1.8 crashes printing `try_table` with
  `-show-encoding`; `encodetest_wasm` leaves that form to the validators.
- **The gates, on `bb01d7c0`** (`nice -n 10`, one at a time):
  `encodetest_wasm` "cases: 234   forms: 91   agreeing: 234   disagreeing:
  0"; `fuzz_wasmmath` "4800000 calls compared with the Rust playground's
  libm", every seed "disagreeing: 0"; `wasmsubset`, 50 minutes: "651
  programs with a main, 121 set aside, 530 compared", "as grasm run: 530
  as the native build (stdout, stderr, status): 530   timed out: 0", "named
  par_map, par_filter or spawn and held: 22, the Rust module running them as
  grasm run does: 21, otherwise: 1" (`E-RECURSION-PARALLEL`, 8,000 calls deep
  in `par_map`, where the Rust module traps), "modules: 530   built twice the
  same: 530   V8 validates: 530 (refuses 0)   wasm_validate.py: 530 valid, 0
  invalid, 145560 bodies checked", "refusals: 4 of 4 as the Rust module's",
  "stack: 6 of 6 recursions past any depth end in the panic". The 508
  programs `wasmtest` compares are among the 530; set aside: a filesystem 83,
  sockets 22, a C function 8, stack switching 4, the stack's depth 3,
  processes 1 -- `wasmtest`'s 143 less the 22 held, two of its 24 thread
  programs needing a filesystem or a socket as well. `wasmcontrols`, 24
  minutes: "controls: 9   caught: 9   missed: 0" -- `add` selected as
  `i64.sub` (not as `grasm run`), a branch's label one level out and the
  `i64` locals declared `f64` (each refused by `wasm_validate.py`), the
  context switch's leaf returning (a `concurrent` block not refused as the
  Rust module refuses it), a refusal worded otherwise (the same), the clock
  in every module (built twice, not the same bytes), no shadow frames (a
  recursion past any depth not ending in the panic), a signed LEB128 ended
  early (`encodetest_wasm`), `sin`'s first coefficient one bit off
  (`fuzz_wasmmath`).

- **The final runs** (2026-11-22 and 23, on `2f8c9782`, the final tree but
  for this paragraph, by the agent's resumable runner, `nice -n 10`, one gate
  at a time, no network; other apps held swap at 15 to 17.4 GB; the Mac did
  not sleep). `encodetest_wasm`, `fuzz_wasmmath`, `wasmsubset` and
  `wasmcontrols` as on `bb01d7c0` above, line for line. `wasmtest`, its rules
  moved to `scripts/lib/wasmskip.sh`: "identical: 508   mismatched: 0
  skipped: 143", its list of what it set aside the same lines as the
  baseline's. `s3subset` "identical: 648   mismatched: 0   timed out: 0   no
  TIR: 0"; `objtest` "objects: 648   reproducible: 648 ... with problems:
  0"; `xs3subset` "identical: 648   mismatched: 0   timed out: 0   no TIR:
  0"; `fixpoint` "compared: 54 ... identical: 54   differ: 0", "stage 2's TIR
  the same as grasm tir's: 6 of 6", "the command's build the stage
  pipeline's (its rt.o and the 4 programs): 5 of 5". The stage-1 command
  built from this tree, the sections the new files reach: `lines` "lines: 280
  the same: 275 ... a target refused: 4", `fmt` "2054 files ... 20540
  commands   the same: 20540", `lint` "2054 files, 5543 commands ... the
  same: 5543". The whole `clidiff` with fixpoint's stage-2 command (its bytes
  not part six's: the command carries `s3`, which now links the wasm
  target), 753 minutes, exit 0, every floor held: among its lines "run: 651
  programs with a main   the same: 651   timed out: 0", "build: 648 programs
  ... run alike: 648   built twice the same: 648", "target: 65 programs ...
  195 of 195", "dap: 1049 sessions compared   the same: 1008", "given the
  program's own run time: 0", "lsp: 2071 sessions   the same: 2071".
  `clicontrols` "controls: 36   caught: 36   missed: 0"; `fmttest`
  "formatted: 2048   unformatted: 0   does not parse: 5   exempt: 1";
  `gfmttest` "checked: 1538   disagreeing: 0"; `linttest` "linted: 2049
  findings: 1165   does not parse: 5   broke: 0"; `gatecheck` "gate helpers
  checked: 603   undefined: 0   corpus copied: 0   grep -q under pipefail: 0
  gates not executable: 0". Not run: `tirinterp` (the interpreter did not
  change), the rest of `allgates.sh`, the Linux ELF gates. `wasmtest`'s own
  blind spot (the programs it sets aside for `par_map`, `par_filter` and
  `spawn`) is left as it is; `wasmsubset` holds them.

### 6.25 S8 part eight: the playground module built by the Grasm wasm32 backend

Written 2026-11-23, before any code, as the design to build to. Part seven
(§6.24) built the wasm32 backend from MIR and left the playground to this
part: decision (1b) of §6.24, the module of `compiler/front` and
`tools/tirinterp` compiled by that backend with the Rust module's exports,
and decision (9), its ABI -- the Rust module's, byte for byte, but the one
import for the maps' seed -- with two questions left open, what becomes of an
`eprint`'s text and of a recursion past 173 calls. The Rust toolchain stays
stage 0 and the oracle and is not changed; where its behaviour is wrong it
is reproduced and recorded in docs/known-issues.md, or, where it traps,
recorded and not copied. `site/` and the published playground are not
switched in this part: that is the owner's word alone. The decisions are
numbered (1) to (10); each is proposed by the agent implementing this part,
and is the coordinator's to accept and the owner's to reverse. Where there
is a choice the alternatives are written under the decision.

#### What was measured

Measured 2026-11-23 on this branch's base (`fec77d2a`) with the main
checkout's `grasm` (stage 0, the release of 2026-11-01) and its Rust module
(`grasm_wasm.wasm`, 1,939,255 bytes, the module §6.24 measured), `s3` built
from this tree by stage 0, and `node` v25.2.1 (V8 14.1), the engine every
wasm gate runs. Other apps held the load at 7 to 16 and swap at 17 to 18
GB, so no time below is to be quoted but the interleaved ratios. Everything
was built and run in scratch; the modules assembled byte by byte for the
engine's facts are part seven's builder, reused; nothing reached the network.

**The Rust module's five exports, as `crates/grasm_wasm` writes them.**
`grasm_alloc(len: i32) -> i32` reserves `len` bytes (a `Vec`'s buffer);
`grasm_free(ptr, len)` gives back one of those or an answer, and does
nothing for a null pointer or a length of 0; `grasm_run(ptr, len) -> i32`,
`grasm_run_with_stdin(ptr, len, in_ptr, in_len) -> i32` and
`grasm_run_with_journal(ptr, len, j_ptr, j_len) -> i32` run the UTF-8 source
at `ptr` and answer a pointer to one buffer of little-endian `u32`s -- its
total length, the exit code, the standard output's length and bytes, a
text's length and bytes -- which the caller frees with the total it read.
What goes in the text and the code, by case: diagnostics, each rendered
against the file name `<browser>` and joined by newlines, with no "N
error(s) found" line, code 1; a program without `main`, "error: no `main`
function", 1; a panic, its first line alone ("panic: index 5 out of bounds
(len 1)"), 101; `exit(n)`, nothing, `n` & 255; a journal that does not
install, "cannot replay: " and the decoder's words (among them "not a
Grasm journal", "journal ends mid-record", "trailing bytes after a recorded
value", "this run already has a journal"), code 7, and the program not
run. Nothing else is
refused: invalid UTF-8 is replaced, not refused; the standard input is
split on newlines after one trailing newline is dropped, and none given is
end of input; a source that starts with `//!module ` is a source bundle
(`grasm build --source`), every other a single file with no imports. An
`eprint`'s text is lost (docs/known-issues.md). Its imports are the three
clocks; `run.mjs` also supplies a `dlsym` the module no longer imports.

**Front and the interpreter, compiled by part seven's backend, today.** A
measuring package (scratch, `play`: `compiler/front` and `tools/tirinterp`,
and the dozen lines of `compiler/grasm`'s `run_bundle` -- the program
loaded from memory by `loader.load_sources_shown` or, for a bundle,
`loader.load_text`, checked by `front.checked_loaded`, its TIR printed flat
and read back, run by `tirinterp.run_tree` -- with the source as its first
argument) through `grasm tir` (33 s) and `s3 --target wasm32` with the
runtime's and `compiler/rtwasm`'s TIR. Refused, by name, two C functions
the C library lacks: "s3: wasm32: `realpath`, called from the program, is
defined by nothing linked" (`front`'s loader, `std::fs::canonicalize`) and,
once that was defined, `strerror_r` (`tirinterp`'s `os_words`, §6.23
decision (6)). With both added to a scratch copy of `compiler/rtwasm` as
refusals, nothing else: no irreducible function, no instruction, feature or
size the backend refuses. The module:

- 3,444,329 bytes (813,690 under `gzip -9`) -- 1.78 times the Rust
  module's bytes and 1.44 times its compressed size: 1,987 functions,
  3,267,216 bytes of code, 124,121 of data, a 50,385-byte name section;
- `s3`'s link 12.0 to 12.7 s (three builds), peak footprint 243 MB; the
  three builds the same bytes;
- V8 validates it (`new WebAssembly.Module`); `scripts/wasm_validate.py`:
  valid, 1,987 bodies and 1,761,033 instructions checked;
- `WebAssembly.compile` 10.7 to 16.4 ms (lazy, as for the Rust module's 2.8
  ms); a first run of `hello` 51 to 78 ms, the next ones in the same
  instance 2 to 7 ms;
- imports `env.grasm_write`, `grasm_entropy` and the three clocks; exports
  `memory`, `_start` and `_args` (part seven's program entry, not the five).

What it does, each probe also run by `grasm run` and the Rust module
(`run.mjs`):

| program | `grasm run` | the Rust module | the probe |
|---|---|---|---|
| `examples/hello.gr`, `exit(3)` | prints; 0 and 3 | the same | the same |
| a type error | the diagnostic and "1 error(s) found", 1 | the diagnostic at `<browser>`, no count line, 1 | the diagnostic at `<browser>` and the count line, 1 |
| no `main` | "error: no \`main\` function", 1 | the same | "tirinterp: <browser>: the program has no \`main\`", 3 (the probe skipped the command's check) |
| `eprint` then a panic | both lines, then the stack trace, 101 | stdout; text the panic's line, 101 | stdout; standard error both lines, no trace, 101 |
| `E-PARALLEL-JOIN`, `E-SPAWN-SHARED` (`spawn`) | `3`, `30` | the same | the same |
| a `concurrent` block, `E-CONCURRENT` | runs | "panic: \`concurrent\` needs a platform that can switch stacks, which this one cannot", 101 | the same bytes (the leaf of §6.24 (4)) |
| `scripts/fixtures/wasmrefuse/` (4) | -- | -- | stdout, stderr and status the Rust module's, 4 of 4 |
| `1 + depth(n - 1)`, n = 400 | 400 | traps at 174 | 400 |
| the same, n = 499 and up | runs | traps | the `concurrent` panic, 101 |

The last row is the interpreter's stack (§6.23): every 500 nested calls of
the program, `eval.body` hops to a fresh host stack by starting a host
`concurrent` block of one task, and on wasm no stack can be switched. With
the hop taken out (a scratch copy, `HOP` one billion), the interpreter
recurses on the engine's stack until the runtime's floor on the shadow stack
(§6.24 decision (5)) stops it, always with the panic "stack overflow:
recursion too deep" and 101, never a trap; the deepest call that ran, by
shape: `1 + depth(n - 1)` 1,315; `d(n - 1)` as a statement 1,775; through a
function value 537; through `[n - 1].map(down)` 334. Each call of the
program is several calls of the interpreter -- the floor of §6.24, 6 MiB of
shadow stack at its scale of 7.0, is about 900 KB of the engine's stack,
some 680 bytes a call of the program on `plain` -- so no single engine stack
of node's 984 KB holds 10,000 of them.

**The probe on `wasmtest`'s set** (every corpus program with a `main` that
`wasmtest` would compare, and the ones it sets aside only for naming
`par_map`, `par_filter` or `spawn`, each against `grasm run`, stdout and
status, and the latter against the Rust module too; source bundles for
programs with imports, as `wasmtest` runs them; 300 s each; 49 minutes,
then the 157 programs over 60,000 bytes again with the source on standard
input, 23 minutes): of the 508 programs `wasmtest` compares, 506 print and
exit as `grasm run` does; `text4-demo` ends at once in the `concurrent`
panic -- the loader's hop for an expression nested 400 deep, the same stack
-- and `gc-demo` ran past 300 s (30 s under `grasm run`). Of the 22 set
aside only for threads, 20 as `grasm run` and as the Rust module;
`examples/parallel_map.gr` ran past 300 s under both modules (178 s under
`grasm run`); `E-RECURSION-PARALLEL`, 8,000 calls deep inside `par_map`,
ends in the `concurrent` panic where the Rust module traps. So with no
switch the interpreter's own scheduler never runs on wasm: `par_map`,
`par_filter` and `spawn` run on one core with `grasm run`'s answers, as in
the Rust module, and the first `concurrent` block -- the program's, or a
hop's -- is refused in the Rust module's words. Standard error differed
from `grasm run`'s on 14 programs: by the stack trace alone on 12 (the
trace is the Rust interpreter's; the module's runtime, like a native
build's, prints none) and by the hop's panic on the two above. The
probe's node time was 26 minutes for the first 373 programs, `grasm run`'s
12.

**The engine, for switching stacks.** Part seven found core stack switching
behind flags and JS promise integration (JSPI: `WebAssembly.Suspending`,
`WebAssembly.promising`) accepted. Measured now, each by a module assembled
in scratch:

- *A promising call runs on a stack of its own, as large as node's default
  one.* A recursion of 94-byte frames (one argument, four locals) went
  1,000,000 deep when every 5,000th level called a Suspending import that
  awaited the same export through `promising` (200 stacks, 32 ms); one such
  stack held 10,000 levels and not 10,400, as the main stack does; without
  the hops 20,000 levels overflowed. No flag.
- *An exception crosses stacks.* A wasm exception thrown on the inner stack
  reached, through the Suspending import's `await`, a `try_table` on the
  outer one and was caught by its tag, the value intact.
- *A switch costs about 0.2 µs.* Two stacks handing control to each other
  through a Suspending import and a promise each resolves (a coroutine's
  switch built on JSPI): 195 to 198 ns a switch over 2,000,000 switches
  (the native switch's cost was not measured beside it).
- *The leanest frames.* Under node's default stack, one function of one
  `i64` argument recursing reaches 15,554 levels with no locals (63 bytes a
  level), 10,369 with four, 5,184 with sixteen; a level made of two
  functions with no locals at all, 7,777 (127 bytes a level).

What engines other than this V8 do with JSPI could not be measured here (no
network, no other engine).

**The speed, as a playground meets it** (five rounds, the two modules
alternating which goes first, median seconds of a whole run under node at a
load of 7 to 16 from other apps, every output the same; the probe's source
passed as an argument, the Rust module's read by `run.mjs`) --

| program | the Rust module | the probe | probe / Rust |
|---|---:|---:|---:|
| `fib` | 3.385 | 2.725 | 0.805 |
| `sieve` | 5.238 | 10.322 | 1.970 |
| `matmul` | 11.602 | 17.437 | 1.503 |
| `sort` | 0.909 | 3.230 | 3.552 |
| `tree` | 4.341 | 3.643 | 0.839 |
| `wordfreq` | 0.954 | 2.043 | 2.141 |
| `examples/hello.gr` | 0.105 | 0.163 | 1.560 |
| `examples/effects.gr` | 0.121 | 0.133 | 1.094 |

A geometric mean of 1.50: the Grasm interpreter built for wasm by the Grasm
backend takes half as long again as the Rust interpreter built for wasm by
rustc, faster on two of the eight. Where the time goes was not measured;
§6.23's natively measured `sort` (4.25 times the Rust interpreter) is the
same program's weakness here. Nothing is promised of it.

#### The decisions

**(1) The module: a package of its own, `tools/playground`, with the Rust
module's five exports.** Proposed. A package beside `tools/tirinterp`
whose `export fn`s are `grasm_alloc`, `grasm_free`, `grasm_run`,
`grasm_run_with_stdin` and `grasm_run_with_journal`, typed `i32` as the Rust
module's are, answering the same frame; inside, the path `grasm run` takes
from memory (`compiler/grasm`'s `run_bundle`, measured above): the source
loaded as one file named `<browser>`, or as a bundle; checked; refused as
`grasm run` refuses (no `main`, a runtime package); its TIR made, read back
and run by `tirinterp.run_tree`; for the journal export the bytes handed to
`journal.install` (§6.22), whose refusals are already the Rust decoder's
words. Estimated 250 to 350 lines. What it needs beneath it:

- *`compiler/wasm32`*: a module whose roots are a program's `export fn`s
  instead of `_start` and `_args`, each exported under its name, with `i32`
  parameters and results where the Grasm types are `i32` (wrapped and
  extended at the boundary, so a caller passes numbers, not `BigInt`s), each
  inside the `try_table` that catches the exit tag as `_start` does, so an
  `exit` or a panic deep in the interpreter ends the run with its status and
  not the instance; `s3 --target wasm32 --exports`. About 150 lines.
- *`compiler/rtwasm`*: `realpath` (refused, as every file operation is) and
  `strerror_r` (`strerror`'s words), the two names the build lacked; and a
  second form of the standard streams for a module built `--exports`,
  chosen when the module is linked as `os.gr` is chosen per target: what
  the program writes to standard output and standard error kept in memory
  for the frame, and standard input served from the lines the export was
  handed -- so that the playground imports neither `grasm_write` nor
  `grasm_read`. About 150 lines.

Its imports are the three clocks the Rust module has and
`env.grasm_entropy`, decision (6) of §6.24 -- and, under (2a), the switch.
So `run.mjs` cannot be left untouched: it gains the `grasm_entropy` line,
because the Grasm runtime hashes every map with a seed drawn from the
host's entropy and by the owner's rules has no fallback, where the Rust
interpreter keeps a map as a `BTreeMap` and hashes nothing. The frame, the
exports, their types and every text the Rust module gives are unchanged, so
`wasmtest` and `replaytest` run the new module through `run.mjs` as they
run the old one (`WASM=`). `wasmtest` itself cannot be pointed at it as it
stands -- it rebuilds the Rust module first, checks every embedder for every
import (`docs/wasm.md` and the four pages under `ProjectGold/` lack
`grasm_entropy`, and must until the owner switches them) and compares the
module with the committed `ProjectGold/web/grasm.wasm` -- so the new gate of
(9) holds the module on `wasmtest`'s set with `wasmtest`'s rules.

The alternatives: **(1b)** the whole `compiler/grasm` package compiled with
a playground entry. Measured: the command compiles today with five C names
added as refusals (`realpath`, `strerror_r`, `stat`, `chmod`,
`_NSGetExecutablePath`): 7,880,225 bytes (1,761,775 under `gzip -9`), 2.29
times the probe's, 3,066 functions, V8 and `wasm_validate.py` valid, a link
of 65 s at a peak of 672 MB -- `s3`, `link`, the TOML reader and SHA-256
carried into a page that runs none of them; refused (it is the seed's
starting point, S9's, not the playground's). **(1c)** exports with `i64`
signatures, as the backend's own: every embedder would pass `BigInt`s --
refused.

**(2) Switching stacks: JS promise integration, for the playground's link.**
Proposed, against §6.24's decision (2), which left JSPI out, and as its
alternative (4c), which it left for later. The interpreter needs a fresh
stack every 500 calls of the program and one per task (§6.23), and wasm
without a switch has neither: the probe stops every recursion at 499 calls.
The design: the backend writes the context switch's two leaves for a module
built `--exports` as calls of two functions of the C library, which keep a
fiber's shadow-stack pointer and entry in its context and call one more
import, `env.grasm_switch(from, to)`; the embedder supplies it as a
`WebAssembly.Suspending` function that starts the target fiber the first
time, by calling the module's export `_fiber(ctx)` through
`WebAssembly.promising` on a stack of its own, and after that resumes it by
resolving the promise it is waiting on; and the embedder calls the three
run exports through `WebAssembly.promising` and awaits them. About 150
lines of the backend and the C library, and 40 of JavaScript in
`scripts/wasm/stacks.mjs`, which `run.mjs` imports and a page copies. The
interpreter does not change: its hops and its scheduler run as they do
natively, on fresh engine stacks the size of node's default one.

What it buys: the interpreter's recursion to its own count (decision (3)),
and with it `text4-demo` and `E-RECURSION-PARALLEL`;
`concurrent` blocks run as `grasm run` runs them, the scheduler of §6.23
deciding the order, its seeds included -- the four programs `wasmtest`
sets aside for stack switching become comparable with `grasm run` (and
differ from the Rust module, which refuses them; counted apart). What it
costs: the calls become asynchronous -- `run.mjs`'s `run` and `replay`
await, and so must `site/playground.js` and the pages when the owner
switches them; about 0.2 µs and a promise a switch, nothing on a run that
never switches; a fiber's stack is linear memory, which wasm never gives
back (to be measured at the deepest recursion); and an engine without JSPI
cannot switch: there the embedder answers the switch with a plain function
and the leaf panics in the Rust module's words, so every recursion past 499
calls and every `concurrent` block ends in that panic -- unless (2b)'s mode
is built as well and chosen by the embedder through one more import
(`env.grasm_stacks() -> i32`, answered 1 with JSPI and 0 without), which the
coordinator may take or leave. The compiled programs' modules (`wasmsubset`)
keep §6.24's decision (4), the refusal, unless the coordinator moves them
too.

The alternatives: **(2b)** no switching: the interpreter gains a mode with
no host stacks, a field of its context the playground sets (no hop in
`body`, `deep_node` or the loader's reads 400 levels deep), so
that a recursion ends at the runtime's floor in the reference's words, and
`concurrent` is refused as the Rust module refuses it; synchronous, every
engine, `run.mjs` gaining only the entropy line; about 30 lines in
`tools/tirinterp`, held by `tirinterp` and `clidiff` to change nothing
natively. Its depths are the ones measured above -- 334 to 1,775 calls by
shape, short of 10,000, an open defect of the playground, though 2 to 10
times the Rust module's 173 and never a trap; whether `text4-demo`'s deep
expression then fits one engine stack is to be measured, and if not it is
a program `wasmtest` compares that the module cannot run. **(2c)** an
asyncify-style lowering in `compiler/wasm32` (every function that may reach
a switch saves
its live locals to the shadow stack and unwinds, and re-enters through a
dispatch): synchronous and engine-independent, tasks and depth for the
playground and for compiled programs alike; for the interpreter nearly every
function may reach a switch, so its size and speed would pay throughout --
not measured here, estimated at 1,500 to 2,500 lines with its gates, the
largest of the three. **(2d)** core stack switching: behind flags in this
node; a page cannot set one.

**(3) The interpreter's depth.** Under (2a): the count of 100,000 calls
(§6.23) the aim, the clause's 10,000 required, under node's default stack
and never a flag; a recursion past either ends in "stack overflow:
recursion too deep", 101, never a trap; inside a task 10,999, as natively;
the linear memory each depth takes measured and reported, and if 100,000
takes more than a page should ask for, the bound is stopped and reported to
the coordinator, not lowered quietly. Under (2b): the depths above, recorded
in docs/known-issues.md as an open S3 defect of the playground, the Rust
module's 174-call trap closed beside it.

**(4) Standard error: carried in the frame's text.** Proposed. The frame's
second text is everything the program wrote to standard error, in order --
`eprint`'s lines, then a panic's line or a refusal -- followed by
diagnostics as the Rust module renders them. Same layout, same exports:
`run.mjs` already writes that text to its standard error and
`playground.js` already shows it after the output, so neither changes for
this. For a program that writes nothing to standard error the text is the
Rust module's to the byte; for one that does, it is the Rust module's
preceded by those lines. The gates compare it with `grasm run`'s standard
error less the stack trace (which the module, like a native build, does not
print). The alternatives: **(4b)** the Rust module's loss copied -- refused,
a defect copied on purpose; **(4c)** a fourth field for standard error --
the layout changes and every embedder with it, refused.

**(5) The compiled modules' recursion depth: switching stacks closes it; not
in this part's first half.** Measured above: the engine's stack, not the
shadow stack, is what runs out (the shadow frames are sized to it), and a
level of two functions with no locals at all reaches 7,777 levels, so a
recursion through the runtime's `map` (`closure`, 3,390 today) cannot reach
10,000 on one engine stack however small its frames; `plain` (9,368) is
within 10% of its bare limit with no shadow frames (10,404, §6.24), below
which the panic itself needs room. A smaller shadow frame changes nothing
the engine counts; fewer locals live would raise `locals`, `doubles` and
`tree` but not past the two-frame limit; a tail call would help one shape
alone, `mutual`, the only fixture whose recursion is in tail position and
the one that already passes 10,000 (and §6.24's (2) left `return_call` out).
What closes it is a switch: with (2a), a function that takes the floor and
finds the shadow stack past its segment calls itself again through
`grasm_switch` on a fresh stack, which makes every compiled module's start
asynchronous (`runprog.mjs`) -- estimated 300 to 500 lines and the
`wasmstack` fixtures then held to 10,000 each. Proposed as a later step: at
the end of this part if the coordinator accepts (2a) and wants it here, or a
part of its own; the defect stays open in docs/known-issues.md until then.

**(6) Every run starts from the instance's first state.** Proposed. Three
runs in one instance agreed above, but a run that ends by `exit` or a panic
unwinds past every `free`, and the runtime's process cells -- the maps'
seed, the output buffers, the journal's state, the fiber pool -- outlive
it. So each run export restores the data segments (made passive and
re-initialised with `memory.init`), the heap, the shadow stack and the
process cells, and draws a new seed; the caller's buffers and the answer
live in an arena of their own that the reset leaves alone. The
alternative: **(6b)** a fresh instance per run, made by the embedder -- every
embedder changes; refused.

**(7) The Rust module's words wherever it has words.** Proposed: a
diagnostic at `<browser>` with no count line, "error: no \`main\` function"
and 1, "cannot replay: <why>" and 7, a panic's line, the exit code &
255, a bundle's diagnostics as the Rust module names them (measured on
fixtures before they are copied); where the module of (1) would say
otherwise -- the probe's "tirinterp: <browser>: the program has no
\`main\`" and 3 -- it is changed to the Rust module's. Where the Rust module
traps (a recursion of 174) nothing is copied.

**(8) How it is built.** Proposed: by `s3 --target wasm32 --exports` from
`grasm tir`, as part seven's gates build, in this part; `grasm build
--native --target wasm32-unknown-unknown` in the Grasm command (§6.24's
(8), with `clidiff` gaining the category in which the Grasm command writes a
module where the Rust one refuses the target, and the runtime's and the C
library's TIR carried beside the command as `rt.o` is) in the second half
(below).

**(9) How it is held.** A gate of its own, `playtest`, sibling of
`wasmtest` (sourcing `scripts/lib/wasmskip.sh`), in `scripts/allgates.sh`:

- the module built twice, the same bytes; V8 validating it and
  `wasm_validate.py`, its count of bodies asserted; its imports exactly the
  ones above, and `run.mjs` supplying each;
- `wasmtest`'s whole set through `run.mjs`, the Grasm module against the
  Rust module and `grasm run`: stdout and status the same as both on every
  program `wasmtest` compares (no new skip, the same reasons, the floor
  `wasmtest`'s 508), the thread programs held as `wasmsubset` holds them,
  the stack-switching ones (under (2a)) held to `grasm run` and counted
  apart, and the text of (4) held to `grasm run`'s standard error less its
  trace;
- `replaytest`'s journal entry with `WASM=` the Grasm module, and a
  journal that does not install, each as the Rust module answers;
- `wasmsubset`'s extra programs through it (`wasmrefuse` as the Rust
  module, `wasmstack` and the depth fixtures of `clidiff`'s `stack` section
  to the depth of (3), each past it the panic, the depth printed);
- the frame's every case on fixtures, the Grasm module against the Rust
  one: a diagnostic, no `main`, a panic, `exit(-1)` as 255, stdin with and
  without a trailing newline, none, invalid UTF-8, an empty program, a
  bundle;
- size and speed against the Rust module, interleaved, as ratios, printed.

Cost, from the measurement above: the probe ran 373 programs in 26 minutes
of node and `grasm run` ran them in 12, the slowest four (`parmap`,
`password-demo`, `crypto_hash-demo`, `examples/parallel_map.gr`) half of
that; with the Rust module's runs, the large programs and the fixtures,
estimated at 70 to 100 minutes, which the gate measures and prints. Nothing
in it grows faster than the corpus. One program sets the limit: `gc-demo`
ran past 300 s under the probe, more than ten times its 30 s under `grasm
run` -- measured again on the module of (1), and the gate's limit set so
that no program it compares times out, never a skip. `wasmcontrols` gains,
each a fault planted that the gate must catch: the frame's layout off by a
field, a refusal worded otherwise (`cannot replay` changed), a journal entry
lost (`grasm_run_with_journal` running without installing it), a module
built twice differently, the reset of (6) left out (a second run sees the
first's state), standard error dropped (4), and the hop's switch refused
(under (2a)) or the no-stack mode left off (under (2b)). `wasmtest`,
`wasmsubset`, `wasmcontrols`, `replaytest`, `s3subset`, `objtest`,
`fixpoint`, `tirinterp` hold.

**(10) What the site needs to switch, listed only.** Not done in this part:
`ProjectGold/web/grasm.wasm` (which `scripts/site.py` copies to
`site/grasm.wasm`) replaced by the Grasm module; `site/playground.js`, the
four pages under `ProjectGold/` and `docs/wasm.md` supplying
`grasm_entropy` (and, under (2a), loading the switch glue and awaiting the
run); `wasmtest`'s check of the committed module pointed at the Grasm build
or retired in favour of `playtest`; `scripts/site_pages.py`'s words ("The
interpreter, compiled for `wasm32`", the `grasm_wasm` row, the `cargo build
--target wasm32-unknown-unknown -p grasm_wasm` recipe, "Wasm `--target
wasm32`", which the Grasm command's build makes true after (8)); and
`crates/grasm_wasm` kept until S9 as the oracle.

#### The coordinator's decisions

Taken by the coordinator on 2026-11-23, on the note above; the owner may
reverse any of them.

- (1): accepted. `tools/playground`, five `export fn`s in the Rust module's
  `i32` types and frame, built by `s3 --target wasm32 --exports`; `realpath`
  and `strerror_r` added to `compiler/rtwasm` as refusals in the Rust
  module's words. `run.mjs` gains `grasm_entropy` (the platform's
  cryptographic generator, no fallback; an engine without it is refused,
  never seeded from a clock or a constant). The Rust module ignores imports
  it does not declare, so `run.mjs` stays one file for both modules, and
  `wasmtest`'s line is shown unchanged after the edit. (1b) and (1c)
  refused.
- (2): (2a), JSPI, for the playground's link, with (2b) as the fallback,
  chosen by the host by feature detection (`'Suspending' in WebAssembly`)
  and passed in through the `grasm_stacks` import. The coordinator's
  reason: JSPI is a standard (phase 4, April 2025) shipping in Chrome and
  Edge 137, Firefox 153 and Safari 27, so a current browser gets the
  clause's depth, and an older engine still runs every program
  synchronously. The fallback's depths are recorded in docs/known-issues.md
  as an open S3 defect of the no-stack mode, never as a difference the
  clause allows. That JSPI needs no flag on the pinned node is to be
  confirmed; if it needs one, the work stops and is reported.
- (3): accepted. Under JSPI the interpreter's own count of 100,000 calls is
  the aim and 10,000 is required on node's default stack with no flag; if
  memory makes 100,000 unreasonable, the work stops and is reported with the
  numbers, the count not lowered quietly.
- (4): accepted. Standard error goes in the frame's existing text field in
  the order the program wrote it, no layout change; `playtest` compares it
  with `grasm run`'s, the stack-trace lines handled as `clidiff` handles
  them (said where the gate is described); docs/known-issues.md's
  playground `eprint` entry says the Grasm module fixes it while the Rust
  module still loses it.
- (5): not in this part. The compiled modules' recursion depth is a later
  part of its own, since it changes how every compiled module starts; the
  S3 entry in docs/known-issues.md stays open.
- (6): accepted, with a control that runs two programs in one instance and
  catches state carried from one to the next.
- (7): the Rust module's words and exit codes everywhere it has them -- no
  diagnostic-count line it does not print, "error: no \`main\` function"
  and exit 1; any place that cannot be matched is stopped at and reported.
- (8): by `s3` in this part. The command's wasm target only if the Rust
  command has a behaviour to hold it to byte for byte; measured first.
  Measured 2026-11-23: the Rust `grasm build --native --target
  wasm32-unknown-unknown` (and `--target wasm32`) answers "error:
  unsupported target \`wasm32-unknown-unknown\`: Support for this target
  has not been implemented yet", exit 1, writing nothing, and without
  `--native` it refuses `--target` altogether. It has no behaviour to hold
  a wasm build to, so the second half is dropped, not invented.
- (9): accepted, with the controls listed and the reset control of (6). It
  runs both modes on the whole set if the two together stay at or under
  about 150 minutes; otherwise JSPI on the whole set and the fallback on a
  fixed, named sample, counts asserted. `gc-demo` and
  `examples/parallel_map.gr` each get a longer limit for that program alone,
  never a skip, their times on both modules stated; `text4-demo` and
  `E-RECURSION-PARALLEL` must match `grasm run` under JSPI.
- (10): the first half is (1), (2), (3), (4), (6), (7) and (9), built by
  `s3`; with (8) dropped there is no second half. The work stops and is
  reported after the first half's final runs. `site/`, the published
  playground and every embedder (`ProjectGold/` included) are neither
  switched nor edited.

The coordinator's second decision (2026-11-23), on the stop the agent made
at (3). Measured first, under JSPI with the 500-call hop of a command: the
statement and function-value shapes ran to 99,998 calls, but the shape
through `[n - 1].map(down)` was refused below 10,000 -- one engine stack
holds only 334 of its calls, fewer than the 500 between hops -- and with
the hop every 250 calls on wasm every shape reached 99,998 at a cost of
3,239 to 3,285 MiB of linear memory (331 to 336 MiB at 10,000), each hop
taking the runtime's 6.5 MiB fiber stack, which the wasm `mmap` cut as an
8 MiB block and zeroed whole; every 100 calls, 99,998 ran out of memory in
the runtime's placeholder panic "(message needs formatting)". The
coordinator's answer:

- (3): C and D. The count stays 100,000 on wasm, as under `grasm run`, so
  that a program's output does not depend on the engine; B (a lower count
  on wasm) refused, since the playground and `grasm run` would then
  disagree on a program between the two bounds; A (a hop every 250 calls)
  refused for its memory. C: on wasm only, the hop decided by the room the
  running engine stack has left (the playground's stack-space query), a
  command's 500-call hop unchanged and shown so by `tirinterp`, `fixpoint`
  and `clidiff`'s `stack` section; where a hop falls must change no output,
  the panic still at the reference's count in the reference's words, held
  with the four shapes and `scripts/fixtures/stack` against `grasm run`.
  D: exact-size blocks for fibers in the wasm `mmap`, only the pages
  `memory.grow` has just added left unwritten, every reused block zeroed,
  with a control that reuses one dirty.
- Out of memory: when `memory.grow` fails the run ends in a worded panic
  and exit 101, never the placeholder and never a trap; the placeholder's
  source found, reproduced natively if native shows it and recorded, fixed
  on wasm, and the work stopped before any change to `compiler/rt`; the
  case in `playtest` under a lowered memory cap.
- Measured and recorded: the four shapes at 10,000 and at 99,998 under
  JSPI, with linear and resident memory; 10,000 must run for every shape
  on node's default stack with no flag.
- `--export-prefix play_:grasm_` accepted. The language keeps the names
  beginning `grasm_` for the toolchain's own code ([T-EXPORT]: "names
  beginning `grasm_`, `g_`, ... are the runtime's and the compiler's"), so
  `tools/playground` cannot write `export fn grasm_run`; it writes
  `play_run`, and the backend exports it as `grasm_run`. The module's ABI
  -- the five names, their types, the frame -- is the Rust module's,
  unchanged. Likewise an `export fn` may not perform `async` ([T-FN-ADDR]),
  and running a program does, so a run export leaves its inputs with the C
  library and runs the program's `main` through it (`wasm_main`).

#### Where the first half ends

The first half: (1), (2), (3), (4), (6), (7), (9) -- the module built by
`s3`, held to the Rust module and `grasm run` -- in steps, each held before
the next: the backend's `--exports` and the C library's streams and reset,
on a fixture of the five exports; the switch of (2a) (or (2b)'s mode); the
package; the gate and its controls. The second half: the Grasm command's
`--target wasm32-unknown-unknown` with its `clidiff` category, and (5) if
the coordinator puts it here.

**What is left for S8 after this part, if it ends there**: the LSP on the
query engine; the debugger's cost per statement; `grasm-j`; the scheduler's
cost per switch (on wasm too, under (2a)); the in-task recursion bound of
10,999; the compiled modules' recursion depth, unless (5) is done here; and
the site's switch, the owner's.

#### The first half, as built

(2026-11-23 and 24, this branch, the main checkout's compiler and node
v25.2.1; loads of 6 to 16 from other apps, so no time here is to be quoted
but the interleaved ratios; who decided what is said where it was decided.)

- **What was built.** `compiler/wasm32` and `compiler/s3`: `--target wasm32
  --exports`, `--export-prefix FROM:TO` and `--max-memory MIB` (the module of
  `export fn`s, decision (1); a program's module is the same bytes as
  part seven's, six programs compared). `compiler/rtwasm`: the streams in
  `src/streams.gr`, the playground's in `play/streams.gr` (which imports
  nothing: a file outside `src/` cannot name the package's modules, as the
  capability check reads every file of a package); `realpath` and
  `strerror_r` refused in the Rust module's words; `stacks.gr`, the switch
  over JSPI and the room query; `reset.gr`, a run begun from the instance's
  first state; large mappings cut exact and zeroed but where `memory.grow`
  has just made them zero; out of memory, the panic "memory allocation of N
  bytes failed". `tools/tirinterp`: the three ways a run moves to a fresh
  stack (`STACKS_COUNT`, a command's, unchanged; `STACKS_ROOM`;
  `STACKS_NONE`), and 48 spare stacks for a task hopping by room.
  `compiler/front`: `diagnostics_text`. `tools/playground` (the five exports
  and the run path), `scripts/wasm/run.mjs` (both modules, JSPI by feature
  detection, `--session`), `scripts/playtest.sh` and nine controls in
  `scripts/wasmcontrols.sh`.
- **Found while building, each settled here.** The language keeps the names
  beginning `grasm_` for the toolchain ([T-EXPORT]) and an `export fn` may
  not perform `async` ([T-FN-ADDR]): the exports are written `play_*` and
  renamed by `--export-prefix`, and a run export runs the program's `main`
  through the C library (`wasm_main`), the coordinator accepting both. The
  Rust module reads a standard-input line ending `\r\n` with its `\r`, where
  `grasm run` drops it; the Grasm module reads as `grasm run` does
  (docs/known-issues.md, not copied). A task hopping by room spends its 21
  spare stacks well before a command's count of 10,999 calls on a costly
  shape (about 290 calls of `[n - 1].map(f)` to an engine stack): 48 spares
  for a task that hops by room, the count the command's.
- **The stack, by measurement (decisions (3) and C, D).** Under JSPI, with
  the hop by room, every shape runs 10,000 calls and 99,998 under node's
  default stack and no flag, and the 100,000th call is the count's panic,
  as under `grasm run`. Linear memory and resident set (the module, 3.5 MB,
  and node's own included in the latter), the program `main` calling the
  shape: `1 + depth(n - 1)` 65 MiB and 119 MB at 10,000, 589 MiB and 608 MB
  at 99,998; `d(n - 1)` as a statement 52 and 105, 452 and 474; through a
  function value 146 and 198, 1,396 and 1,346 (8 s); through `[n -
  1].map(down)` 227 and 270, 2,216 and 1,803 (28 s). Inside a task each
  shape runs 10,997 calls, the command's count, at 334 to 339 MiB (a
  block's 48 spare stacks, each 6.5 MiB of the runtime's, linear memory that
  is reserved and mostly untouched). With no stack switched: 1,314, 1,774,
  537 and 334 (docs/known-issues.md, open S3). Under a 256 MiB cap the
  deepest shape ends in "panic: memory allocation of 6848528 bytes failed",
  101, in half a second; the placeholder text the runtime gave before was
  found in `compiler/rt/src/fiber.gr`, copied from the Rust runtime, and not
  reproduced natively (docs/known-issues.md, latent; `compiler/rt` is not
  changed).
- **The `\r` of a line.** The Rust module reads a standard-input line ending
  `\r\n` with its `\r`; `grasm run` and the Grasm module drop it. The
  coordinator's decision (2026-11-24): the Grasm module drops `\r` as `grasm
  run` does; the Rust module's behaviour is the S4 defect recorded in
  docs/known-issues.md, not copied.
- **Slow programs, measured.** `gc-demo`: the Grasm module 371 s, the Rust
  module 40 s, `grasm run` 30 s; `examples/parallel_map.gr`: 563 s and 246
  s, `grasm run` 178 s. `playtest` gives these two 3,600 s each, every other
  run 600 s.
- **The final runs** (2026-11-23 and 24, on `ac4f5c5a`, by the agent's
  resumable runner, `nice -n 10`, one gate at a time, no network; other
  apps held swap at 17 to 19.7 GB, disk 8.2 to 10.7 GB free; the Mac did not
  sleep). `playtest`, 112 minutes, exit 0: "build: built twice the same: 1
  V8: valid   wasm_validate.py: 2036 bodies   link: 13s", the exports
  "grasm_alloc/1 grasm_free/2 grasm_run/2 grasm_run_with_journal/4
  grasm_run_with_stdin/4" as the Rust module's, the seven imports, "size:
  3496479 bytes (821266 under gzip -9), the Rust module's 1939255 (564375)";
  "frame: 16 cases, 2 modes each   as the Rust module: 27   as grasm run, by
  decision: 5"; "session: 18 runs in one instance   as a fresh instance:
  18"; "stack: 30 runs   as held: 28   past the Rust interpreter's stack,
  within the count: 1   past a task's spare stacks: 1"; "corpus: compared
  508   as grasm run: 508 (standard error too: 508)   as the Rust module:
  508   set aside: 114" (filesystem 83, sockets 22, a C function 8,
  processes 1: `wasmtest`'s reasons, none new), "named par_map, par_filter
  or spawn and held: 22, the Rust module running them as grasm run does:
  21", "stack switching, held under JSPI to grasm run: 4", "recursing past
  what the Rust module's stack holds, held under JSPI to grasm run: 3",
  "without JSPI, the sample (scripts/lib/playsample.txt): 56   as under JSPI
  (or refused as the Rust module refuses): 55   deeper than one engine
  stack, the panic: 1" (`E-RECURSION-PARALLEL`); "replay: identical on every
  engine"; "speed: geometric mean of this module's time over the Rust
  module's, three interleaved rounds: 1.451" (`fib` 0.865, `tree` 0.997,
  `hello` 1.092, `effects` 1.169, `matmul` 1.292, `wordfreq` 1.843, `sieve`
  1.982, `sort` 3.776). `wasmcontrols` "controls: 18   caught: 18   missed:
  0". `wasmsubset` "as grasm run: 530   as the native build (stdout,
  stderr, status): 530   timed out: 0", "modules: 530   built twice the
  same: 530   V8 validates: 530 (refuses 0)   wasm_validate.py: 530 valid,
  0 invalid, 147125 bodies checked", "refusals: 4 of 4 as the Rust
  module's", "stack: 6 of 6 recursions past any depth end in the panic".
  `wasmtest`, through the new `run.mjs`: "identical: 508   mismatched: 0
  skipped: 143", the line it gave before the edit. `replaytest` "journaled:
  44   unhooked: 0   no test program: 0", "a native recording replayed
  elsewhere: identical on every engine". `s3subset` "identical: 648
  mismatched: 0   timed out: 0   no TIR: 0"; `objtest` "objects: 648
  reproducible: 648 ... with problems: 0"; `tirinterp` "round trip: 1671
  identical, 0 not", "identical: 651   mismatched: 0   timed out: 0
  skipped: 0", "panics: 25 identical, 0 not"; `fixpoint` "compared: 54 ...
  identical: 54   differ: 0", "stage 2's TIR the same as grasm tir's: 6 of
  6", "the command's build the stage pipeline's (its rt.o and the 4
  programs): 5 of 5". The stage-1 command (5,228,984 bytes), the sections
  the change reaches: "lines: 280   the same: 275 ... a target refused: 4",
  "run: 651 programs with a main   the same: 651   timed out: 0", "fmt: 2070
  files ... 20700 commands   the same: 20700", "lint: 2070 files, 5589
  commands ...   the same: 5589", "schedules: 20 programs, 121 commands
  the same: 121 ...", "stack: 13 programs, 26 commands   the same: 22
  past the Rust interpreter's stack, within the count: 2   past a task's
  spare stacks: 2" -- the command's 500-call hop unchanged. The whole
  `clidiff` with fixpoint's stage-2 command, 910 minutes, exit 0, every
  floor held: among its lines "run: 651 programs with a main   the same:
  651   timed out: 0", "test: 1737 programs, 1961 commands ...   the same:
  1960, 12 of them with a failing test   both timed out, --contracts: 1",
  "schedules: 20 programs, 121 commands   the same: 121 ...", the `stack`
  line as on stage 1, "bundle: 651 ... run alike, plain and with
  arguments: 651 ... built twice the same: 651", "journal: 652 programs
  recorded   the same: 652 ... native recordings replayed alike: 45 of
  45", "dap: 1047 sessions compared   the same: 1005, 96 of them fixed ...",
  "given the program's own run time: 1 (slowest:
  packages/gc-demo/src/main.gr step 262 s, run 356 s)", "lsp: 2087 sessions
  the same: 2087, 17 of them fixed". `fmttest` "formatted: 2064
  unformatted: 0   does not parse: 5   exempt: 1"; `gfmttest` "checked:
  1539   disagreeing: 0"; `linttest` "linted: 2065   findings: 1175   does
  not parse: 5   broke: 0"; `gatecheck` "gate helpers checked: 609
  undefined: 0   corpus copied: 0   grep -q under pipefail: 0   gates not
  executable: 0".
- **`clicontrols`, missed by the final run and fixed after it.** On
  `ac4f5c5a` it said "controls: 36   caught: 35   missed: 1": the
  `task-stack` control could not be planted, because `7befd735` (48 spare
  stacks for a task hopping by room) had moved the line it plants into --
  `let nsp = if cx.p.may_recurse { SPARES } else { 0 }` in
  `tools/tirinterp/src/tasks.gr` became the function `spares` -- without
  updating the control in the same commit, as the owner's rule asks. Found
  by the final run, not before. `a6656bc9` plants the same fault into the
  code as it stands (`let nsp = spares(cx)` made `let nsp = 0`: a command's
  task with no spare stacks, which `scripts/fixtures/stack/task-limit.gr`
  shows), run alone first ("controls: 1   caught: 1   missed: 0", 11
  minutes) and then the whole `clicontrols` on `a6656bc9` (186 minutes,
  ended 2026-11-24 23:26): "controls: 36   caught: 36   missed: 0". That
  commit changes `scripts/clicontrols.sh` alone, and is the
  only one between `ac4f5c5a` and the final tree but for the documents, so
  every other gate's line above holds on the final tree. The agent's runner
  had also marked the failed step done (it marked a step whatever its
  status); it now marks only a step that exits 0.
- **Not run:** the rest of `allgates.sh`, `xs3subset` (the native targets'
  code did not change: `s3subset`, `objtest` and `fixpoint` hold it), the
  Linux ELF gates.

### 6.26 S8 part nine: a program compiled to wasm reaches 10,000 calls

Written 2026-11-24, before any code, as the design to build to.
[E-RECURSION-DEPTH] guarantees at least 10,000 nested calls and a panic past
an implementation's bound, never a trap. A module the Grasm backend writes
(`s3 --target wasm32`, §6.24) stops short on five of the six shapes of
`scripts/fixtures/wasmstack/` -- `mutual` 10,089, `plain` 9,368, `doubles`
8,030, `tree` 7,026, `locals` 5,620, `closure` 3,390 -- each in the clause's
panic, which is the open S3 entry of docs/known-issues.md. Part eight
measured why (§6.25 decision (5)): the engine's own stack runs out first,
the shadow stack being sized to it, and a level of two functions with no
locals at all reaches only 7,777 levels on one engine stack, so nothing but
a second engine stack closes it. This part does that. The Rust toolchain
stays stage 0 and the oracle and is not changed; `compiler/rt` and
`compiler/mir` are not changed either. The decisions are numbered (1) to
(8); each is proposed by the agent implementing this part, and is the
coordinator's to accept and the owner's to reverse. Where there is a choice
the alternatives are written under the decision.

#### What was measured

Measured 2026-11-24 on this branch's base (`f3856fe7`) with the main
checkout's `grasm` (stage 0) for `grasm tir`, `s3` built from this tree by
stage 0 in scratch, scratch copies of `compiler/wasm32` changed for
measuring only (said where used), and `node` v25.2.1 (V8 14.1.146.11), with
no flag. Other apps held the load at 6 to 15 and swap at 19.8 to 21.5 GB, so
no time below is to be quoted but the interleaved ratios. The engine's facts
come from modules assembled byte by byte by part eight's builder, reused;
nothing reached the network.

**A second engine stack, synchronously.** Part eight's switch
(`env.grasm_switch`, §6.25 decision (2)) is a `WebAssembly.Suspending`
import, so everything that may reach it runs inside `WebAssembly.promising`
and the run is asynchronous. A recursion needs less than a switch: a fresh
stack that runs to its end and comes back. Measured on a function `f(n) = 1
+ f(n - 1)` of four `i64` locals that every 5,000 levels calls a plain
import, which calls the module's export `g` through `promising` -- `g` runs
`f(n - 1)` and leaves the answer in an exported global the import reads
back:

- *It runs to its end before the import returns.* 10,000 levels on 2 extra
  stacks in 1.0 ms, 100,000 on 20 in 2.5 ms, 1,000,000 on 200 in 33.5 ms,
  every answer right, the caller -- `f` called plainly from JavaScript --
  never asynchronous and no `Suspending` anywhere. Without the hops, 20,000
  levels overflow.
- *Each such stack is a JSPI stack's size.* Levels one stack holds, by
  bisection, a fresh process per probe, for 0, 4 and 16 locals: node's main
  stack 15,608, 10,405, 5,202; a stack `promising` makes when called from
  JavaScript 15,227, 10,151, 5,075; one started by a synchronous import
  inside a wasm call the same, 15,227, 10,151, 5,075. A JSPI stack holds
  97.6% of what the main stack does.
- *A hop costs about 2.5 µs, either way.* 100,000 levels hopping every
  5,000 or every 100, synchronous and through `Suspending` as part eight's
  switch does, the four alternated, 30 rounds, medians, the difference over
  980 hops, two runs: synchronous 2.34 and 2.64 µs a hop, through
  `Suspending` 2.48 and 2.66.
- *Nesting has a limit, and the synchronous form fails silently at it.*
  Hopping every 100 levels, both forms fail at the 1,386th nested hop
  (138,601 levels) with `RangeError: Maximum call stack size exceeded`,
  as if each hop's JavaScript frames took about 710 bytes of node's main
  stack (its 984 KB over 1,386). The synchronous probe, whose import did not check
  that the inner stack had finished, then answered 138,601 for 1,000,000 --
  wrong output. Whatever starts a stack must check that it finished.
- *An exception is seen only where it is caught.* A wasm exception left
  uncaught on the started stack does not reach the import: `promising`
  answers a promise, which is rejected later. Caught by a `try_table` on
  that same stack, its value is in memory before the import returns.
- *A switch inside a synchronous hop breaks it.* A `Suspending` import
  called on a stack the synchronous import started parks that stack and the
  import returns before it has finished (the probe read -1, and the stack
  finished afterwards with 42). A synchronous hop must never meet a switch.
- *Memory.* 1,000,000 levels, 200 stacks live at the deepest: peak resident
  148 MB, against 48 MB for 9,000 levels with none -- about 0.5 MB a live
  stack. 1,000 runs of 100,000 levels (20 stacks each) in one instance:
  resident 50 MiB after 200 runs, 62 MiB after 1,000.
- *Without JSPI, a stack from another thread.* The module's memory shared,
  a worker per stack, a hop that writes `n` to the next worker's slot and
  waits (`Atomics.wait`) for its answer: 100,000 levels with 23 workers in
  1.05 to 1.24 ms a run, the 23 started in 79 to 113 ms; 209 workers 1,204
  ms; about 20 µs a hop; peak resident 213 MB with 23 workers, about 7 MB a
  worker. It works under node. In a browser it needs a shared memory, so a
  cross-origin isolated page, and `Atomics.wait` is refused on a page's
  main thread, so the module would have to run in a worker.

**The compiled modules today, and on a JSPI stack.** The six fixtures built
by `s3` from this tree, each run past any depth and bisected for the
deepest call that prints, by `scripts/wasm/runprog.mjs` and by a copy of it
that calls `_start` through `WebAssembly.promising` (so the whole program
runs on a JSPI stack): `closure` 3,390, `doubles` 8,030, `locals` 5,620,
`mutual` 10,089, `plain` 9,368, `tree` 7,026 both ways, past each the panic
"stack overflow: recursion too deep" and 101 both ways, never a trap. The
calibration of §6.24 (shadow frames seven times the engine frame) holds on
a JSPI stack, 2.4% smaller as it is. A recursion inside `par_map` (`[n].par_map(|x|
depth(x))`) reaches 9,362 and one inside a `parallel` block's `spawn`
9,368: on wasm both run on the calling engine stack (one processor, no
thread), as `plain` does. Of the three corpus programs `wasmtest` sets aside
for their depth, compiled to wasm today: `E-RECURSION-DEPTH` ends in the
panic, 101, where `grasm run` prints `10000` twice; `E-PANIC-TRACE`
(unbounded) ends in the panic and 101 as `grasm run` does; `scheme-demo`
prints and exits as `grasm run` does.

**One engine stack's share, if the shadow stack models it one to one.** A
scratch copy of `compiler/wasm32` with every shadow frame at 1.0 times the
engine frame's estimate (`8 * (parameters + locals) + 56` bytes, §6.24)
instead of 7.0, and a check on entry to every function that takes the
floor against a second global, set to 6 MiB / 7 (898,779 bytes) below the
shadow stack's top, that traps (`unreachable`) when passed: the deepest call
each shape reaches before that check, on node's stack and on a JSPI stack
alike, with frames rounded to 16 bytes as today -- `closure` 3,301,
`doubles` 8,022, `locals` 5,615, `mutual` 9,359, `plain` 9,359, `tree` 7,019
-- and rounded to 8 -- 3,401, 8,022, 5,615, 10,210, 9,359, 7,019, the
`par_map` shape 9,352 and the `spawn` shape 9,358. Past each, the check,
never a `RangeError`, on both stacks. Rounded to 8, four shapes hold
what they hold today less 0.1% and two more (`closure` 0.3%, `mutual`
1.2%); the rounding to 16 lost 7% on `mutual`, taking it below 10,000.
Whether the room left at the check holds the hop's own frames as well as
it holds the panic's is the gates' to show; the panic's room is §6.24's
calibration, which failed at a scale of 6.5 (8% more levels on `plain`).

**The cost on programs that never recurse deep.** The same entry check (9
bytes: two `global.get`, `i32.lt_u`, an `if` around `unreachable`) in every
function that takes the floor, never taken, against the module as it is;
and the module as it is with `_start` called through `promising` and
awaited, which is what an asynchronous start costs. Seven rounds, the three
alternated and the order rotated, median seconds of a whole run under node,
every output the same; `fib(40)` is 331 million calls of a function that
takes the floor:

| program | today | entry check | ratio | start through `promising` | ratio |
|---|---:|---:|---:|---:|---:|
| `fib(40)` | 0.634 | 0.645 | 1.017 | 0.635 | 1.001 |
| `sieve` | 0.459 | 0.459 | 1.000 | 0.460 | 1.002 |
| `matmul` | 0.438 | 0.434 | 0.993 | 0.426 | 0.974 |
| `sort` | 0.254 | 0.255 | 1.005 | 0.262 | 1.032 |
| `tree` | 0.580 | 0.570 | 0.983 | 0.572 | 0.988 |
| `wordfreq` | 1.488 | 1.480 | 0.995 | 1.488 | 1.000 |

Geometric means 0.999 and 0.999; an earlier nine-round run of `fib(40)` and
`tree` gave 0.994 and 0.980 for the check, 0.991 and 0.989 for the start.
Neither is distinguishable from the noise at these loads. Functions that
take the floor, by module: one in every module (the runtime's), `hello` 1,
`fib` 2, `b28-demo` 2, `tree` 3, `scheme-demo` 6.

#### What each way costs

| | (A) part eight's switch, the start asynchronous | (B) a synchronous hop | (C) a worker per stack |
|---|---|---|---|
| engines | JSPI | JSPI | shared memory and workers; no JSPI needed |
| the module's start | through `promising`, awaited by every embedder | unchanged | unchanged, but in a worker on a page |
| a hop | about 2.5 µs | about 2.5 µs | about 20 µs, a worker 4 to 5 ms to start and about 7 MB |
| a program that never recurses deep | 0.999 (start) and 0.999 (check), measured | 0.999 (check), measured | a shared memory in every module, not measured |
| size | about 90 bytes a function that takes the floor and 300 a module, estimated | the same | more: a shared memory needs a maximum, and every module the threads feature |
| lines, estimated | `compiler/wasm32` 180, `compiler/rtwasm` 80, `runprog.mjs` 50 (part eight's glue) | 150 to 200, 50, 15 | 500 to 800 with the page's worker |
| tasks in a compiled module later | ready: a hop is a switch | the hop moves onto the switch then | no |

The sizes are estimates from the functions counted above (a 120,255-byte
`hello` module would grow by about 0.4 KB, `scheme-demo`'s 243,924 by about
0.85 KB), measured when built. Three more ways were weighed and set aside:
**(D)** an asyncify-style lowering (§6.25's (2c)) unwinds the frames into
memory and rewinds them onto the same engine stack, so it adds switching
but not depth; **(E)** core stack switching is behind flags in this node,
and a page cannot set one; **(F)** smaller engine frames cannot pass 7,777
levels of two functions, and `closure` has several frames a level.

#### The decisions

**(1) The mechanism: a synchronous hop to a fresh engine stack, (B).**
Proposed. A function that takes the floor checks on entry, before it
reserves its shadow frame, whether that frame would pass the current
segment's floor -- a second global, `global 1`, beside the shadow stack's
pointer. If it would, it does not run here: it writes a hop record below
the shadow stack's pointer (its own thunk's table slot, its arguments, room
for its answer and a status), moves the pointer below it, and calls the C
library's `wasm_hop(record)` (`compiler/rtwasm/src/deep.gr`, new), which
sets the segment's floor one segment below the pointer and calls one new
import, `env.grasm_deeper(record) -> i32`. The embedder answers it by
calling the module's new export `_deep(record)` through
`WebAssembly.promising` -- a fresh engine stack, which runs to its end
before the import returns (measured above) -- and answering 1, or 0 when it
has no `promising`. `_deep`, written by the backend, calls the thunk inside
a `try_table` that catches the tag `_exit` throws and writes its status in
the record; the thunk, one per function that takes the floor, reads the
arguments, calls the function -- whose check now passes -- and writes its
answer. Back in `wasm_hop`: the floor restored; an `exit` or a panic in the
deeper stack thrown again here with its status, so it unwinds stack by
stack to `_start`; a record the deeper stack did not finish (a trap there)
ends in a trap, never in a made-up answer (the silent wrong output
measured above). The function then takes its answer from the record, moves
the pointer back and returns it. The function is re-entered from its first
instruction with the arguments it was given, before it has loaded an
`inout` argument or reserved anything, so nothing it does is done twice.
The shadow stack stays one contiguous stack across hops, so the runtime's
floor and its check, which MIR already emits, are untouched and remain the
bound (decision (2)); the segment's check is the backend's alone.

What it costs: the module's start, `runprog.mjs`'s and every embedder's
calls, stay synchronous; `runprog.mjs` gains the import (about 15 lines,
`GRASM_WASM_STACKS=0` standing in for an engine without JSPI as in
`run.mjs`); 2.3 to 2.6 µs a hop, and one hop per segment of depth (about
3,400 to 10,200 levels, decision (2)), so 10,000 levels of `closure` take
three hops, 7.5 µs; nothing measurable on a program that never hops (the
table above). One cost is new and is not in the native builds: a call made
over and over across a segment's edge -- a loop at the edge calling a
function that takes the floor -- pays a hop on every call, 2.5 µs where a
call costs a few nanoseconds. Go's segmented stacks met the same ("hot
split") and Go moved to contiguous, copied stacks in 1.3; a wasm module
cannot copy the engine's stack. It is accepted here, measured on a fixture
in step two and recorded, not hidden. The alternatives: **(1b)**, (A)
above: the same hop made with part eight's switch, which makes every
compiled module's start asynchronous and every embedder await it, for no
gain in speed or depth -- its one advantage, that a hop could then meet a
task's switch, has no use while compiled modules refuse tasks (§6.24
decision (4)), and if a later part gives them tasks through JSPI the hop
moves onto the switch there, the thunks and records unchanged; **(1c)**,
(C), the only way measured that works without JSPI, at the costs above and
a page in a worker -- not proposed now, and listed with decision (4).

**(2) The bound: the runtime's own, on a shadow stack that models the
engine stack one to one.** Proposed. Every shadow frame becomes 1.0 times
the engine frame's estimate (`floor_scale` 100, from 700), rounded to 8
bytes (from 16), and a segment is 6 MiB / 7 = 898,779 bytes of it -- the
engine stack §6.24's calibration already gave each stack, so each segment
holds what one stack holds today (measured above, to 0.1%). The runtime's
floor stays where `compiler/rt` puts it, 6 MiB below where a recursion
first asks, which is now seven segments: a program compiled to wasm may use
about 6 MiB of the engine's stack, as a native build may use 6 MiB of the
machine's. The depths that predicts, seven segments' worth, to be measured:
`closure` about 23,800, `locals` 39,300, `tree` 49,100, `doubles` 56,100,
`plain` 65,500, `mutual` 71,400 -- each at least twice the clause's 10,000.
Past it, the runtime's own check panics in its words on the last stack,
where §6.24's calibration (holding on a JSPI stack, measured) leaves room
for it. At most eight engine stacks are live, eight hops' frames on node's
main stack (about 6 KB of its 984 KB, against 1,386 hops' worth), about 4
MB resident by the measure above; the shadow stack's 8 MiB of linear memory is not enlarged.
The alternatives: **(2b)** the scale kept at 7.0 and the shadow stack made
seven times larger (about 50 MiB of linear memory from the start), the
runtime's floor asked once before `main` from 42 MiB down so that it lands
at the bottom -- more memory, the same depths; **(2c)** a count of hops as
the bound: the runtime's floor would have to move with each segment, which
only `compiler/rt` could do.

**(3) Which functions hop.** Proposed: exactly those that take the floor
(MIR's `floor`). Every unbounded recursion passes through one -- that is how
the clause's panic is guaranteed today -- and between two of them the
frames that do not take it are counted on the shadow stack and covered by
the margin §6.24 measured, as now. A thunk and a check for each: the 9
bytes of the check, a hop path of about 30 bytes and 5 an argument, a thunk
of about 20 and 5 an argument; one `_deep`, one import and `wasm_hop` a
module. Deterministic: the thunks in the functions' order, built twice the
same as every module is.

**(4) An engine without JSPI: the clause's panic, at today's depths, and
the entry kept open for it.** Proposed. `grasm_deeper` answers 0 and
`wasm_hop` ends the run in "stack overflow: recursion too deep" and 101 --
the clause's words, not a new refusal -- at the first segment's end: the
depths measured above with frames by 8 (`closure` 3,401 to `mutual`
10,210), the same as today's within 0.1%, so nothing gets worse there.
docs/known-issues.md's S3 entry is closed for an engine with JSPI with the
measured depths, and a narrower one stays open, S3, for an engine without
it, those depths in it -- a defect, not "a difference the clause allows".
What would close it is (C), a worker per stack, at the costs measured
above, or core stack switching once an engine ships it unflagged: neither
proposed in this part, both the coordinator's or the owner's to put in a
later one.

**(5) The playground: not changed, and its no-JSPI entry not closed by
this.** Proposed. A module built `--exports` keeps the scale of 7.0, frames
by 16 and no hop, so `tools/playground`'s module is the same bytes as
master's (checked once in the final runs) and `playtest` holds unchanged.
Under JSPI it already reaches 99,998 calls by the interpreter's own hops by
room (§6.25). The no-JSPI entry of part eight is not closed by this
mechanism, measured three ways: the hop needs `WebAssembly.promising`, half
of the same proposal as `Suspending`, so an engine without JSPI has neither
(the coordinator's own test for it, `'Suspending' in WebAssembly`, stands for
both); the interpreter's hops switch (`grasm_switch`, `Suspending`), and a
switch inside a synchronous hop returns to the import before the stack has
finished (measured above); and each call of the program is many of the
interpreter's frames, 334 to 1,774 calls to an engine stack. What would close
it: (C), or an interpreter whose calls of the program do not recurse on the
host's stack -- a rewrite of `tools/tirinterp`'s evaluator, not proposed.

**(6) Threads and tasks.** On wasm `par_map`, `par_filter` and a
`parallel` block's `spawn` run on one processor, on the calling engine
stack (measured: 9,362 and 9,368 levels, as `plain`), so the hop applies to
a recursion inside them as to any other, and [E-RECURSION-PARALLEL] holds as
[E-RECURSION-DEPTH] does. Proposed: two fixtures more in
`scripts/fixtures/wasmstack/`, `parmap.gr` and `spawn.gr`, held like the
six. Tasks stay refused in a compiled module (§6.24 decision (4)): a
`concurrent` block's leaf panics before any switch, so a hop never meets
one; a later part that gives compiled modules tasks through JSPI moves the
hop onto the switch, as (1b) says.

**(7) How it is held.**

- **`wasmsubset`'s `stack` line**: the eight shapes, each built and run
  past any depth (the panic, 101, never a trap) and bisected, with JSPI and
  with `GRASM_WASM_STACKS=0`; with JSPI each asserted at or past 10,000 and
  its depth printed; without, each the panic and its depth printed, held to
  at least today's numbers less 1% (no shape worse than before). About a
  minute.
- **`wasmsubset`'s corpus**: the three programs `wasmtest` sets aside for
  their depth (`E-RECURSION-DEPTH`, `E-PANIC-TRACE`, `scheme-demo`) held,
  as grasm run and as the native build, and counted apart as the thread
  programs are -- `scripts/lib/wasmskip.sh` gaining `WASM_SKIP_NO_DEPTH=1`
  beside `WASM_SKIP_NO_THREADS=1`. No new skip anywhere.
- **`wasmtest`** unchanged: it holds the Rust module, which still traps at
  174 calls, so its three depth skips stay; its line must be the line it
  gives today.
- **`playtest`** unchanged, and the playground's module the same bytes as
  master's.
- **`wasmcontrols`** gains, each run alone first and then in the whole: the
  hop dropped (the entry check falls through), so a shape traps or falls
  short of 10,000; the panic past the bound turned into a trap (`_deep`
  without its catch, so a panic in a deeper stack is lost); a module built
  twice differently (the build's clock in a thunk's name); and the refusal
  without JSPI worded otherwise than the clause. Each control's planted text
  checked against the tree before the final run, and `clicontrols`' and
  `wasmcontrols`' existing plants into `compiler/wasm32` grepped after every
  commit that moves its code.
- **What must not move**: `s3subset`, `objtest`, `fixpoint` (the native
  targets' code does not change); `replaytest`; the whole `clidiff` with
  stage 2, since the command carries `s3` (about 13 to 15 hours).

**(8) One part, in steps.** Proposed, each step held before the next:
the backend's check, thunks, `_deep` and the scale, `compiler/rtwasm`'s
`deep.gr`, `runprog.mjs`'s import and the two fixtures, held by
`wasmsubset`'s `stack` section and by `s3subset` and `objtest`; then the
three depth programs held and the controls; then the final runs and the
entry of docs/known-issues.md. The part ends when every shape reaches
10,000 under node's default stack with no flag, past the bound the panic
and 101, the S3 entry closed for engines with JSPI with the measured depths
and the narrower one open for engines without.

**What is left for S8 after this part, if it ends there**: the LSP on the
query engine; the debugger's cost per statement; `grasm-j`; the scheduler's
cost per switch; the in-task recursion bound of 10,999; a recursion below
10,000 on an engine without JSPI, compiled (decision (4)) and in the
playground (decision (5)); the site's switch, the owner's.

What engines other than this V8 do could not be measured here (no network,
no other engine): that a `promising` call runs synchronously until it
suspends is what the JSPI proposal specifies, and is the one fact this
design rests on that was measured on one engine only.

#### The coordinator's decisions

Taken by the coordinator on 2026-11-25, on the note above; the owner may
reverse any of them.

- (1): (B), the synchronous hop; the module's start stays synchronous and
  no embedder changes. On conditions: (a) no silent wrong answer, ever --
  after every hop a completion flag the deeper stack sets is checked, and a
  stack that did not finish ends the run in a trap, never in whatever the
  record's answer slot holds (the 138,601 measured above), although with
  seven segments nesting is at most seven; (b) a trap inside a deeper
  segment (`unreachable`, an access out of bounds) surfaces as a trap, as it
  does today, never as a normal exit or wrong output, held by a fixture;
  (c) a compiled module never reaches `grasm_switch` or anything
  `Suspending` from inside a hop -- where anything in it could, it is
  refused when the module is built or traps at the call, the choice and its
  reason said where built; (d) the one fact verified on V8 alone -- that a
  `promising` call runs synchronously until it suspends, as JSPI specifies
  -- recorded as a risk here and in docs/known-issues.md where relevant.
- (2): (2a): the runtime's floor of 6 MiB, the scale 1.0, frames rounded to
  8 bytes, seven segments, so that the bound is the native one. The margin
  between a segment (about 878 KB) and a JSPI stack (about 961 KB) measured
  and recorded, and held two ways: no `RangeError` on any shape, on either
  kind of stack, past the bound; and a control that catches frames sized
  too small, the engine overflowing before the floor.
- (3): accepted: hops only in functions that take the floor.
- (4): accepted. Without JSPI, the clause's panic at today's depths, no
  shape worse. The S3 entry split: closed for an engine with JSPI with the
  measured depths; a no-JSPI entry open, S3 (the coordinator's), worded as
  an open defect below the guaranteed 10,000, never as a difference the
  clause allows. Workers recorded as the way to close it, in a later part.
  The engines that ship JSPI named as §6.25 records them, no others.
- (5): accepted: the playground unchanged and its no-JSPI entry open; the
  gate that builds the playground's module asserts its bytes are master's.
- (6): accepted: tasks stay refused in compiled modules; the `par_map` and
  `spawn` fixtures join the `stack` line.
- (7): the gates and floors as proposed, and two controls more: frames
  sized too small (the scale lowered), so that a shape overflows the
  engine's stack before the floor, caught as a trap or `RangeError` and
  failing the gate; and the completion check dropped -- or, if no run can be
  made to reach it, why not and what holds it instead. Six new controls in
  all. The hot split measured on a fixture and its cost a call recorded as a
  ratio, with no claim.
- (8): one part, in the three steps proposed, each held before the next;
  the final runs as the brief says, the whole `clidiff` with stage 2 among
  them since the command carries `s3`; every control's planted text grepped
  against the tree before the final run. Only measured depths are recorded
  as reached; the figures predicted above are predictions.

#### The coordinator's decisions on step two, and the tiers

Taken by the coordinator on 2026-11-25, on the agent's reports; the owner
may reverse any of them.

- **Where the completion check lives: in the module.** The agent's
  decision, accepted by the coordinator: `wasm_hop` trusts only the status
  the deeper stack wrote in the record, and traps on anything but answered
  or ended, so no embedder's glue can leave the check out; the embedder's
  import only says whether it could start a stack.
- **Condition (c): refused when the module is built,** as the agent did it
  (the switch's imports in a program's module, the hop's in one built
  `--exports`): accepted.
- **The hot split** -- a call made across a segment's edge against one made
  beside it, measured by `wasmsubset` at 155 times (624 ns and 4.0 ns) -- is
  recorded as measured, a known cost, with no claim; in docs/known-issues.md
  only if a corpus program meets it, which is to be checked.
- **The tiers.** Asked to measure the margin under V8's other tiers, the
  agent found that the hop as first committed (`f806e96b`, written inline in
  the function's prologue, at the scale 1.0) let the engine's stack overflow
  before the floor under `--no-liftoff` (TurboFan compiling everything from
  the start): `mutual` needed 1.20, `plain`, `parmap` and `spawn` 1.10. The
  cause, shown by `--no-liftoff --no-wasm-inlining`, under which `mutual`
  panics: V8's wasm inlining, with the hop's code in the function, made a
  level larger than its estimate. The coordinator's principle: a trap is a
  worse failure than a shallower panic, so correctness under every tier
  comes before the depth without JSPI. Decided: (iv), a slow path TurboFan
  does not inline, tried first in scratch for about an hour; if it failed,
  (ii), the hop out of line, at the smallest scale leaving at least 5% under
  all three tiers; the depths without JSPI allowed to fall by what that
  costs (relaxing "no shape worse"), their floors the new depths less 1% and
  the open S3 entry saying plainly that no shape now reaches 10,000 without
  JSPI and why; a section of `wasmsubset` running each shape past any depth
  under `--liftoff-only` and `--no-liftoff`, the coordinator's word on those
  flags -- they change the compiler, never the stack, so they are allowed
  for that check of the panic alone, and every depth stays a flag-free
  one -- with a control only that section catches; and the margin stated
  as measured on node v25.2.1's V8 14.1 alone, nothing claimed for other
  engines, whose frame sizes and inlining could not be measured here.

What was measured. The smallest scale, in hundredths of the engine frame's
estimate, at which each shape run past any depth still ends in the panic
rather than a `RangeError` -- by bisection, each probe a module built and run
under node with no flag (default), `--liftoff-only` and `--no-liftoff`; with
JSPI and without, the same in every cell but one (`closure` under (iv) with
`--no-liftoff`, 102 and 101). Master's modules (no hop, `floor_scale` 700,
frames by 16) for comparison, in the same units:

| shape | master: default, Liftoff only, TurboFan only | the hop inline (`f806e96b`) | (iv) out of line through `call_indirect` | (ii) out of line, a direct call |
|---|---|---|---|---|
| `closure` | 637, 637, 637 | 92, 92, 92 | 92, 92, 102 | 92, 92, 83 |
| `doubles` | 659, 659, 559 | 94, 94, 94 | 109, 109, 94 | 94, 94, 94 |
| `locals` | 651, 651, 521 | 91, 91, 81 | 111, 111, 76 | 91, 91, 76 |
| `mutual` | 583, 583, 583 | 83, 83, 120 | 102, 102, 83 | 83, 83, 102 |
| `parmap` | 652, 652, 535 | 93, 93, 110 | 93, 93, 50 | 93, 93, 93 |
| `plain` | 652, 652, 535 | 93, 93, 110 | 93, 93, 77 | 93, 93, 93 |
| `spawn` | 652, 652, 535 | 93, 93, 110 | 93, 93, 77 | 93, 93, 93 |
| `tree` | 651, 651, 401 | 95, 95, 83 | 95, 95, 70 | 95, 95, 83 |

Master's margin is 5.9% at worst (`doubles`, 659 of 700). (iv), the slow
path reached through `call_indirect`, is not inlined under `--no-liftoff`
but costs room under the default tiering (`locals` 1.11, `doubles` 1.09):
the worst of the three at 1.11, so not taken. (ii), the slow path a direct
call, is taken: its worst is 1.02 (`mutual` under `--no-liftoff`), so the
scale is 1.08, which leaves 5.9% there and at least 13.7% everywhere else;
1.07 would have left 4.9%. The agent chose 1.08 over the coordinator's
1.07 for that reason, and the coordinator accepted it (2026-11-25): the
rule was the margin of 5%, which 1.08 holds. (iv) was tried first, as
decided, its table above, and failed that margin under the default tiering. The depths at 1.08, flag-free, with JSPI and
without: `closure` 21,841 and 3,117, `locals` 35,740 and 5,104, `tree`
43,685 and 6,239, `doubles` 52,421 and 7,487, `parmap` 60,481 and 8,632,
`plain` 60,488 and 8,639, `spawn` 60,488 and 8,638, `mutual` 65,528 and
9,359 -- without JSPI 7% below what the scale 1.0 gave (12% on `tree`), and
`mutual` below 10,000 for the first time. Warming the recursion first under
the default tiering (200,000 shallow calls before the deep one, so that
TurboFan's code runs it) needed less room, not more: 0.50 or below on
`plain` and `mutual`.

#### Step two, as built

(2026-11-25, this branch, the main checkout's compiler and node v25.2.1;
other apps held swap at 21 to 23.5 GB, disk at 6.1 to 7.5 GB free and the
load at 6 to 15, so no time here is to be quoted but the ratios; who
decided what is said where it was decided.)

- **What was built.** `compiler/wasm32`: in a program's module global 1,
  the running segment's floor, one segment (898,779 bytes) below the shadow
  stack's top; in every function that takes the floor, before its frame is
  reserved, the segment's check and, past it, a call of the function's slow
  path, out of line, which writes the hop record (`body.gr`'s `H_*`: the
  thunk's table slot, a status, an exit status, the answer, the arguments)
  below the shadow stack's pointer, calls the C library's `wasm_hop` and
  answers what the thunk left; a thunk per such function in the table after
  the program's own slots; the export `_deep(record)`, the thunk called
  inside a `try_table` that catches the exit tag and writes its status into
  the record; frames 1.08 times the engine frame's estimate, rounded to 8
  bytes. A module built `--exports` keeps 7.0, 16 and no hop.
  `compiler/rtwasm/src/deep.gr`: `wasm_hop`, which lowers the segment's
  floor, asks the host (`env.grasm_deeper`), restores the floor, and trusts
  only the record's status -- 1 answered, 2 an `exit` or a panic thrown
  again, anything else a trap -- and without a fresh stack ends in the
  runtime's own stack panic. `scripts/wasm/runprog.mjs` answers
  `grasm_deeper` through `WebAssembly.promising` (0 under
  `GRASM_WASM_STACKS=0`) and prints, as the process ends, what ended a
  deeper stack that trapped. The fixtures `wasmstack/parmap.gr`,
  `wasmstack/spawn.gr`, `wasmdeep/trap.gr` and `wasmdeep/split.gr`;
  `wasmsubset`'s `stack` section -- the eight shapes with JSPI and without,
  the tiers, the traps, the hot split -- and its holding of the three
  programs set aside for depth (`WASM_SKIP_NO_DEPTH=1` in
  `scripts/lib/wasmskip.sh`); seven controls in `scripts/wasmcontrols.sh`,
  `WASM_CONTROLS_ONLY=` and `WASM_SUBSET_ONLY=stack` to run them alone;
  `PLAYTEST_BASE_MODULE=` in `playtest`.
- **Condition (c), refused when built.** In a program's module the switch's
  imports (`grasm_switch`, `grasm_fiber`, `grasm_stacks`) are a build error
  naming the caller; in a module built `--exports` the hop's import and
  intrinsics are. A program that declares `extern fn __host_grasm_switch`
  and calls it is refused by name. The runtime's own context switch was
  already the panic of `sys.wasm_no_switch` in a program's module (§6.24
  decision (4)).
- **The trap fixture (condition (b)).** No Grasm program can trap, so
  `wasmdeep/trap.gr` declares two of the C library's intrinsics, which only
  the wasm link binds -- `__wasm_trap` (`unreachable`) and `__wasm_memcpy`
  to an address past the memory (out of bounds) -- and calls one 100 and
  30,000 calls deep, on the first engine stack and on the fourth. Each ends
  as a trap: exit 1, `RuntimeError`, no output; the deep ones also print the
  deeper stack's own error ("the deeper stack: RuntimeError: memory access
  out of bounds").
- **The playground, the same bytes as master's.** Built from master's tree
  (`f3856fe7`) by master's driver and from this branch's by its own, the
  two modules first differed by 40 bytes (3,496,479 and 3,496,519): the
  playground links all of `compiler/rtwasm`'s TIR, and the backend lays out
  every string of a linked program, reachable or not, so `deep.gr`'s panic
  words -- 35 bytes and 5 of alignment -- were inserted at address 103,513,
  and 804 constants in 277 of the 2,036 bodies moved by 35 or 40 with them:
  no function added, no other byte of code changed, no code path. Found by
  the coordinator from the size. `deep.gr` now ends a hop that cannot start
  in the runtime's own stack panic (`grasm_panic_stack`, the same words) and
  holds no string, and the two modules are the same bytes; `playtest`
  compares its module with master's own (`PLAYTEST_BASE_MODULE=`), not with
  another driver's build of this branch's sources, which is what its first
  form did and why it said "the same bytes" of the 40-byte difference.
- **The gates, on `ce40fbc5`** (`nice -n 10`, one at a time, by the agent's
  resumable runner). `wasmsubset`, 50 minutes: "651 programs with a main,
  118 set aside, 533 compared", "as grasm run: 533   as the native build
  (stdout, stderr, status): 533   timed out: 0", "named par_map, par_filter
  or spawn and held: 22, the Rust module running them as grasm run does:
  21, otherwise: 1", "set aside by wasmtest for their depth and held here:
  3", "modules: 533   built twice the same: 533   V8 validates: 533
  (refuses 0)   wasm_validate.py: 533 valid, 0 invalid, 151975 bodies
  checked", "refusals: 4 of 4", "stack: 8 of 8 shapes held", "deepest with
  JSPI: closure 21841 doubles 52421 locals 35740 mutual 65528 parmap 60481
  plain 60488 spawn 60488 tree 43685", "deepest without JSPI (open S3):
  closure 3117 doubles 7487 locals 5104 mutual 9359 parmap 8632 plain 8639
  spawn 8638 tree 6239", "past any depth under --liftoff-only and
  --no-liftoff, the panic: 16 of 16", "a trap in a deeper stack ended as a
  trap: 4 of 4", "the hot split: 516 ns a call across an edge, 4.2 ns beside
  it: 122 times" (155 and 122 on two runs). `s3subset` "identical: 648
  mismatched: 0   timed out: 0   no TIR: 0"; `objtest` "objects: 648
  reproducible: 648 ... with problems: 0". The new controls, each a run,
  `WASM_CONTROLS_ONLY`: "controls: 8   caught: 8   missed: 0" -- `w-floor`,
  `w-hop`, `w-small` and `w-deep-catch` by "past any depth", `w-tier` by
  "under --no-liftoff, past any depth" and by nothing flag-free, `w-done` by
  "a trap in a deeper stack", `w-seg-nondet` by "not the same bytes", and
  the refusal without JSPI by "without JSPI, past any depth" (then planted
  as other words; now `w-deep-panic`, a trap where the panic is, after the
  change above).

## 7. Bootstrap and trust

- **During the migration:** stage 0 is the Rust-built `grasm`. S6 names
  the stages after it (§6.14): stage 1 is the Grasm compiler --
  `compiler/front`, `compiler/s3`, `compiler/link` and the runtime
  `compiler/rt` -- built by stage 0 with Cranelift, the Rust runtime and the
  system linker; stage 2 is that compiler built by stage 1 alone, on its own
  runtime; stage 3 is built by stage 2, and `fixpoint` holds it to stage 2's
  bytes. What stage 0 is did not change: it is the Rust `grasm` until the
  cutover, and builds nothing of stage 2 but stage 1.
- **After the cutover:** stage 0 is a pinned previous release, its SHA-256
  in the repository. Policy as Go's: release 1.N builds with release
  1.(N-1) or later, so any commit builds from a downloadable binary.
- **Any platform, any commit (later):** a compressed wasm build of the
  compiler, committed and runnable by any WASI runtime, as Zig does with
  `zig1.wasm`. Zig then needs a C compiler to translate it. We do not,
  because our own wasm backend writes the seed and a WASI runtime (Node,
  Wasmtime, a browser) runs it. What the seed is, and why it is S9's,
  is §6.24.
- **Trusting trust:** diverse double-compiling against the archived Rust
  implementation, run at the cutover and at every release thereafter that
  the archive can still build.
- **Reproducibility:** two independent builders produce identical bytes;
  release archives list the commit, and their checksums are published.

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
against the Rust loader's), `tomldiff` (every manifest, and fuzzed ones, read
by `packages/toml`, the Rust reader and `tomllib`, §6.21), `tirrefuse` (what the Rust
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
GNU ld) and `elfcontrols` (§6.18), `wasmsubset` (the corpus compiled to wasm
against `grasm run`, the native build and the Rust playground's module, every
module validated twice), `encodetest_wasm` (the wasm encoder against
`llvm-mc`), `fuzz_wasmmath` (the wasm C library's libm against the
playground's) and `wasmcontrols` (§6.24), `playtest` (the playground in
Grasm against the Rust playground's module and `grasm run`, with and
without JS promise integration, §6.25), `ddc`,
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
- **S7 done** (Linux ELF, §6.18): the ELF object writer, the backend and
  runtime for arm64 and x86-64 Linux, and `compiler/link` linking ELF
  against glibc to GNU ld's bytes; the corpus agreeing with the Cranelift
  build for each triple and with the interpreter. Its exit, `crosstest` and
  a Linux CI run, held on 2026-11-13: the five Linux gates passed in a
  scratch OrbStack machine.
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
