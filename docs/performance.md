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
