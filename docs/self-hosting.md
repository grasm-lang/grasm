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
The manifest stays `grasm.toml`: self-hosting changes who reads it, not its
format (decided 2026-10-26; a manifest in Grasm's own syntax, as Zig's
`.zon`, would cost every TOML-aware tool and a migration of every package,
and the language is frozen until S9). Its reader becomes `packages/toml` in
place of `crates/grasm_pkg/src/toml.rs`, with a gate the package lacks
today: every manifest in the repository, and fuzzed ones, parsed by both
and by Python's `tomllib`, agreeing.

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
`dwarfdump`, `ld`), `linktest` (the corpus linked by our linker runs as
it does linked by `cc`), `linkdiff` (our linker against ld-prime) and
`linkcontrols` (§6.6), `rtswap`
(corpus on the Grasm runtime), `rtlayout` (the layouts both runtimes
share), `fuzz_rtfmt` (number formatting against the Rust runtime) and
`rtcontrols` (§6.7), `fixpoint` (stage 2 = stage 3), `ddc`,
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

## 11. Where it stands (2026-10-26)

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
  machine. Why the gap is what it is per program was not measured.
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
- **Not started:** S2 (the frontend in Grasm), S6 (self-compilation),
  S7-S9; S5 past part one.

With S4 done, S6 waits on S3's last steps and on S2 and S5, the frontend
and the runtime in Grasm; they come next, one stage at a time.

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
