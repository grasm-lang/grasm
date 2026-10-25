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
here so it is not lost. Return addresses *are* signed: Cranelift turns on
`sign_return_address` with the B key for every function on macOS
(`pacibsp`/`retab`), and a C entry is a function like any other.

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

## 7. Bootstrap and trust

- **During the migration:** stage 0 is the Rust-built `grasm`.
- **After the cutover:** stage 0 is a pinned previous release, its SHA-256
  in the repository. Policy as Go's: release 1.N builds with release
  1.(N-1) or later, so any commit builds from a downloadable binary.
- **Any platform, any commit (later):** a compressed wasm build of the
  compiler, committed and runnable by any WASI runtime, as Zig does with
  `zig1.wasm`. Zig then needs a C compiler to translate it. We do not,
  because our own wasm backend writes the seed and a WASI runtime (Node,
  Wasmtime, a browser) runs it.
- **Trusting trust:** diverse double-compiling against the archived Rust
  implementation, run at the cutover and at every release thereafter that
  the archive can still build.
- **Reproducibility:** two independent builders produce identical bytes;
  release archives list the commit, and their checksums are published.

## 8. Gates this adds

`tirdiff` (TIR from both frontends), `tirinterp` (TIR interpreter against
the Rust interpreter), `lexdiff`, `parsediff`, `diagdiff`, `encdiff`
(encodings against `llvm-mc`), `objcheck` (objects through `otool`, `nm`,
`dwarfdump`, `ld`), `linkdiff` (our linker against ld-prime), `rtswap`
(corpus on the Grasm runtime), `fixpoint` (stage 2 = stage 3), `ddc`,
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
| Our backend's code is slower than Cranelift's | measured from S3; the release allocator and mid-end close the gap before S9, and S9 does not ship a regression |
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

## 11. Where it stands (2026-10-22)

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
  -- on a subset: 474 of 651 corpus programs with a `main` build with it
  and print and exit exactly as the Cranelift build does (part one had 37:
  scalars, control flow, calls, printing; part two, 289, added heap values
  with explicit reference counting, fallible functions, 111 built-in
  methods over seven receiver families, 22 one-call built-ins, arguments on
  the stack and places passed `inout`, §6.1; part three added `f64` as a
  second register class, closures and function values with the
  higher-order list methods, and generics by monomorphization, §6.2: 357
  with doubles, 457 with function values, 474 with generics). All of them
  run with nothing live at exit (those that reach the leak check; a few end
  by design before it) and no use after free under the poisoned runtime.
  fib(35) was about 1.9x slower than Cranelift's code at part one. What
  still keeps programs out, by count of the 177 left (a program counts
  under each reason it has): references (72), and among the rest raw
  memory and `unsafe` (71), the network built-ins (52), effects (`with`
  50, effect operations 43), ranges held as values (`to_list` 49, a range
  local 21), parallel and task operations (`par_map` 16 and more). No
  program is kept out by doubles, function values or generics alone any
  more; they still appear in 9, 10 and 28 programs something else keeps
  out (a function type over a reference, generic code at a reference
  type).
- **Not started:** S2 (the frontend in Grasm), S4 (the linker), S5 (the
  runtime in Grasm), S6 (self-compilation), S7-S9.

Order from here follows the critical path S3 -> S4 -> S6, one stage at a
time; S2 and S5 follow.

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
