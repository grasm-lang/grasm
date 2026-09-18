# Where the time goes

Grasm compiles through Cranelift and runs on a reference-counted runtime. This
file says what has been measured, what was tried and rejected, and what is left
— so the next person to open a profiler starts from evidence rather than from
the same three guesses.

`benchmarks/README.md` has the cross-language table and the rules for taking
it. This is the other half: why the numbers are what they are.

## The standing gap

Against Rust on the same programs, with the 2 ms harness floor subtracted from
both sides, Grasm's compiled backend is roughly:

| program | ratio | what it is |
|---|---:|---|
| `sort` | 1.9x | comparison-heavy work over a large list |
| `parmap` | 2.1x | multi-core throughput over integers |
| `sieve` | 2.6x | list indexing in a tight loop |
| `matmul` | 2.7x | nested indexing over a flat list |
| `fib` | 2.7x | function call overhead |
| `parwords` | 3.3x | multi-core throughput over text |
| `tree` | ~4.9x* | allocating small linked structures and walking them |

\* `tree`'s cell is an estimate: the table in `benchmarks/README.md` records
5.9x and predates the thread-local work below, which took 1.19x off it on a
machine that was not idle. That row is due a re-measure.

`sort` beats C and Go outright. The three worst rows are the three that
allocate, which is the shape of the whole problem: **arithmetic and indexing
are close, allocation is not.**

## What has been measured

**A thread-local is a call on macOS.** `sample` on `tree` put `_tlv_get_addr`
at the top of the profile with about a third of the samples: `alloc` reached
three separate thread-locals and releasing a value reached two. Consolidating
them into one structure — and replacing a `RefCell` borrow with a counter —
was worth 1.19x on `tree` and 1.10x on `wordfreq`, and nothing at all on the
programs that do not allocate. Done, 2026-09-10.

**The stack guard costs 7%, not 70%.** A function that can reach itself checks
the remaining stack on entry ([E-RECURSION-DEPTH]). Measured by
short-circuiting the check and timing the two builds alternately: 7% on `fib`,
3% on `tree`, nothing on the loop programs. An older note claimed 70%; that
predated the free-box pool. Not worth a guard-page redesign at 7%.

**A region is 10% slower, not faster.** Everything allocated inside one is
arena-owned and swept at the closing brace, which sounds like it should win.
It does not: every allocation is pushed onto the region's list so the sweep can
find it, and the sweep then frees values one at a time exactly as the refcount
would have. Measured on `tree`: 50.7 ms against 45.6, and the same 5.2 MB peak,
because refcounting already freed each round.

**Things that were already fast** and do not need looking at again: scalar list
indexing is pinned and inlined before a loop with an inline bounds check
(`pin_loop_lists`); `bool` lists are one byte an element; a list of four or
fewer elements never touches the heap; a map key of fifteen characters or fewer
allocates nothing; freed boxes go back to a per-thread pool.

## Measured and rejected, 2026-09-18

**Non-atomic reference counts while a program is single-threaded: worth zero.**
In isolation an atomic retain/release pair costs 10.1 ns on this machine
against 0.33 ns for an ordinary one — thirty-one times, most of it the
release/acquire ordering rather than the read-modify-write. That is a real
measurement and it is irrelevant: threading here comes only from `par_map` and
`par_filter`, so the counts *can* be plain integers until one of them runs, and
making them so changed nothing. Forcing every count back to atomic in the same
build changed nothing either — `tree` 45 ms both ways, `wordfreq` 39 ms both
ways. The atomic costs ten nanoseconds when a loop does nothing else and
disappears into the work around it when the program is doing anything at all.
Reverted; the branch and the `as_ptr` write were complexity for no gain.

**A per-thread scratch buffer for the free walk: worth zero.** `dec` allocated
its worklist per call and grew it from nothing. Keeping one per thread for its
capacity measured nothing on `tree`, which frees six hundred thousand values.

**What the allocation counter says.** `GRASM_ALLOC_STATS=1` reports every
allocation a program made. The benchmark set splits cleanly in two:

| program | allocations | what that is |
|---|---:|---|
| `tree` | 1,180,960 | exactly two per node — the struct box and its list of children |
| `wordfreq` | 800,003 | exactly two per iteration — the interpolated number and the joined string |
| `sieve` | 1 | |
| `sort` | 1 | |
| `matmul` | 3 | |
| `fib` | 0 | |

So the two slow rows are slow because of *how many* values they allocate, and
the four fast-ish rows allocate nothing at all — their gap is entirely code
generation. Those are two different problems and neither is fixed by making
allocation cheaper at the margin.

## What the other implementations do, and which of it applies here

Read against Rust's `hashbrown`, Zig's `std.hash_map`, Swift's SIL optimizer,
Lean 4's *Counting Immutable Beans*, Koka's Perceus, Nim's ARC, Go's escape
analysis and Cranelift's own documentation. Four of those findings changed
what is worth doing here, and two of them changed it by saying **no**.

**The map is already the good one.** `wordfreq` is the worst row and the
obvious suspect was the hash table, but this runtime's maps are Rust's
`HashMap`, which is `hashbrown` — the SwissTable design: one control byte per
slot holding seven bits of the hash, sixteen slots matched at a time, a
triangular probe, 87.5% load. Independent measurement puts that at 6.1x a
chaining map on string keys. There is nothing to take; we have it.

What we do *not* have is a fast hash: the default hasher is SipHash-1-3, which
SMHasher3 measures at 0.61 bytes/cycle against rapidhash's 8.82, and foldhash's
own suite puts a word-count workload — ten thousand inserts, ninety per cent
duplicates, which is exactly `wordfreq` — at 35.36 ns/op under SipHash against
14.77 under a folded-multiply hash. Our profile says hashing is 16% of
`wordfreq`, so the ceiling here is about 12%, not 2.4x. **It is a trade, not a
free win**: SipHash is keyed and flooding-resistant, and a language whose maps
hold parsed input should not give that up quietly. PolymurHash is the option
that keeps a proof — almost-universal, 21 cycles for 49 bytes — and is what to
reach for if this is done at all.

**Reference counting is not the problem it is in those languages.** Lean
measures 52% for making its counts non-atomic and Koka 5–59%; both were
tempting until the same experiment here measured zero, twice (above). The
reason is density: their RC operations are everywhere and ours are sparse.
The advice that follows from their papers — biased reference counting — is
therefore the wrong move, and this file records that so it is not tried again.

**What does apply is that every count is a call.** The backend emits
`call grasm_val_inc` at 189 sites, and Cranelift cannot inline it: its mid-end
optimises pure instructions and keeps calls in a side-effect skeleton, and it
has no inliner at all. Nothing in the runtime is an `ir::Function`, so even a
future Cranelift inliner would not reach it. Emitting the fast path as IR —
load the count, compare against the two sentinels, add, store, and branch to
the runtime only when it reaches zero — is the one large win available without
building anything new.

**Everything else needs an intermediate representation we do not have.**
Borrow inference (Lean's fixpoint: borrowed by default, owned when consumed),
drop-guided reuse (the ICFP'22 correction to Lean's algorithm, which otherwise
grows peak memory without bound), stack promotion (Go's signed-indirection
flow graph plus an `outlives` test, with Swift's trick of giving a promoted
object an immortal count so the existing operations become no-ops) — all of
them run over a mid-level IR. This backend lowers the typed AST straight to
Cranelift, so there is nowhere to put them. That is the real prerequisite for
closing the `tree` row, and it is a project rather than a patch.

## What is left, and what each would take

**A bump arena for regions.** The feature exists and currently costs what it
should save. Laying values down consecutively in a chunk and releasing the
chunk whole would remove both the per-allocation list push and the per-value
free. Contained to the runtime; the sweep still has to drop each payload, so
the win is the bookkeeping rather than the drops. Would make regions the
answer to allocation-heavy code that the language already claims they are.

**Escape analysis.** `tree` builds a `Node` per call and drops it a few
instructions later. A value that provably does not escape its function could
live on the stack with no box and no refcount. This is the one that closes the
worst row, and it is a compiler project rather than a patch: it needs an
alias analysis the front end does not have.

**Inlined reference counts.** The backend emits a call for every retain and
release, at more than a hundred sites. Inlining the atomic would remove a call each
time. It needs `#[repr(C)]` on `RtVal` so generated code may know where the
count is, which is fine, and it needs Cranelift's atomics, which are
sequentially consistent. The runtime's counts are `Relaxed` on the way up and
`Release`/`Acquire` on the way down, so an inlined version could easily be
*slower* on ARM and would be a threading defect if the ordering were wrong.
Measure before writing.

**A faster hash.** Both this runtime and the Rust comparison use SipHash, so
the map rows are a fair fight. Swapping in a non-cryptographic hash would win
several per cent and give up hash-flooding resistance, which is a real
property for a language whose maps hold parsed input. Not a free win; it is a
trade, and it should be argued rather than slipped in.

**The optimiser itself.** Cranelift does not unroll, does not vectorise, and
picks instructions less well than LLVM. Part of every ratio above is that, and
no amount of runtime work touches it.

## What will not be claimed

That Grasm matches Rust. It does not, on any row here except `sort`, and the
rows where it is closest are the ones doing the least allocation. The numbers
are in `benchmarks/README.md` with the method beside them; anybody can rerun
them on an idle machine.
