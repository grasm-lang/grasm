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

**Stability nobody can observe costs half the list and a call per compare.**
`xs.sort()` on a scalar list ran Rust's *stable* sort through a closure that
re-branched on the element kind for every comparison; a stable sort also takes
a scratch buffer of half the input. But every comparator here is a *total*
order on the stored word — integers compare as words, floats by `total_cmp`,
which separates -0.0 from 0.0 and orders NaNs by their bits — so two elements
that compare equal are the same 64 bits, and no program can tell which of them
came first. Switching to `sort_unstable` (pdqsort, no scratch, and for
integers no comparator at all) was worth **1.90x on `sort`**: 19 ms to 10,
against Rust's 11. The first benchmark row where Grasm is not behind.
Done, 2026-09-18.

**An empty list is one list.** `[]` allocated a fresh box every time, and the
programs that build a tree or a table of lists allocate a lot of them. There
is now one immortal empty boxed list and a five-entry table of immortal empty
scalar lists, handed out by `grasm_list_new` and friends. Safe without a
special case anywhere else because every list mutator already goes through
`cow_list`, which copies anything whose refcount is not exactly 1 — and an
immortal refcount never is. Worth 393,660 fewer allocations on `tree`
(1,180,960 to 787,300) and 1.05x. Done, 2026-09-18.

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

**Hoisting `i * n` out of the inner loop is worth nothing on this machine.**
`c[i * n + j] += aik * b[k * n + j]` inside `for j in 0..n` recomputes two
products per element, and Cranelift has no pass that moves them out. A
loop-invariant pass was written — canonical spellings of `+`/`-`/`*` over names
and literals, a conservative writes analysis that gives up on any body
containing a call, a closure or a thread, and the computation emitted in the
block before the loop, which is sound without a guard because none of those
three operators can trap. It hoisted exactly what it should: `(i * n)` out of
two loops and `(k * n)` out of one. Measured against the same compiler with the
pass switched off: matmul 1.011x, sieve 1.012x, parwords 1.028x, sort 0.970x,
tree 0.997x — a wash.

The reason is in the disassembly. AArch64 has `madd`, and Cranelift's lowering
pattern-matches `iadd(imul(a, b), c)` into one instruction, so `i * n + j`
costs a single `madd` whether or not the multiply was moved. Hoisting replaces
one instruction with one instruction. The pass was removed.

What the inner loop is actually carrying is the *bounds checks*: of eighteen
instructions, four are two `cmp`/`b.hs` pairs and three more are register
shuffles setting up their operands. Removing those needs the loop compiled
twice — once with the checks and once without, chosen by a single test in the
preheader that the whole range fits — which is the next thing to try, and worth
about forty percent of the loop rather than nothing.

**The map's hash was most of the gap to Zig, and it was a choice.** Timed
head to head by processor time, `wordfreq` takes Grasm 31.4 ms, Rust 25.2 and
Zig 8.5 — and the three programs are the same program. Rust's `HashMap`
defaults to SipHash-1-3, a keyed MAC at about half a byte per cycle; Zig's
defaults to Wyhash. Grasm was inheriting Rust's default and therefore Rust's
bill.

A map now hashes with a multiply-fold — wyhash's mixing step, a full 64x64 to
128 product with its halves xored together — seeded once per process from the
operating system through the same entropy `std` uses. Short inline strings also
compare as whole fifteen-byte arrays rather than through `memcmp`, which is
sound because `StrData::of` starts from zeroes, so the bytes past the text are
zero in both. Together: **wordfreq 1.12x, parwords 1.08x**, and SipHash went
from 108 samples of the profile to 26.

**What this gives up, stated plainly.** SipHash is proved against an attacker
who can watch a table's behaviour and search for colliding keys against the
live seed. The replacement is not. It keeps the defence that matters in
practice — the seed is secret and per-process, so collisions cannot be computed
ahead of time — and it is the same trade Zig's standard library makes. A
program holding a map keyed by hostile input, which cares about the adaptive
attack, should not rely on the default. Nothing here can make a program answer
*wrongly*: a hash picks a bucket and `Eq` decides what is in it. A bad hash is
slow, not incorrect — which is why the guard is four distribution tests in
`grasm_rt`, checked to fail when the mixer is weakened.

**Hand-rolled small copies are slower than `memmove`.** With SipHash gone,
`_platform_memmove` was 22.5% of `wordfreq` — copying seven-byte words. The
obvious fix, two overlapping fixed-width moves instead of a call, made it
*worse*: wordfreq 1.15x slower and parwords 1.23x slower. Apple's `memmove` is
well tuned and the branches on the length mispredict. Reverted.

**The stack check is most of `fib`, and it is two calls, not one.** Every
function that can reach itself calls `grasm_stack_check` on entry
([E-RECURSION-DEPTH] requires a panic and exit 101 rather than a SIGSEGV), and
that function then reads `LOCAL` — which on macOS is a second call, through
`_tlv_get_addr`. `fib(30)` makes about 2.7 million calls and pays for both
every time.

Measured in a standalone Rust program rather than by weakening the compiler's
own check, on the same `fib(30)`:

| stack check | fib(30) |
| --- | --- |
| a call that reads a thread-local — what is emitted today | 7.52 ms |
| a call, with the floor already in hand | 4.96 ms |
| an inline compare against a floor in a register | 3.20 ms |
| no check at all | 2.53 ms |

So the check costs 5.0 ms as built and 0.67 ms inlined. Grasm's `fib` is
9.65 ms against Rust's 3.98 by processor time — a gap of 5.67 — and about 4.3
of that is the two calls. An older note in this file said the guard cost 7%;
that measurement short-circuited the *body* of the check and left both calls in
place, which is why it saw so little.

So the floor is now carried rather than looked up: a function that can reach
itself takes it as a hidden first argument, fetched once where the recursion is
entered, and the check is a compare and a branch the compiler emits in place.

**And it was worth a sixth of what the table above predicted.** fib 1.06x,
tree 1.03x, everything else unchanged — not the 1.8x the 7.52-to-3.20 row
suggests. Two reasons, both visible in the disassembly. Removing the call is
not free: the floor has to survive the recursive calls, which costs a third
callee-saved register to save and restore and a `mov` before each call, so
`fib` went from 27 instructions to 35 even as two calls disappeared. And a
perfectly predicted direct call to a tiny leaf function is largely hidden by an
out-of-order core, which a standalone microbenchmark written around
`#[inline(never)]` does not model.

The lesson is about the method rather than the change: a microbenchmark of a
mechanism in isolation bounds the cost of that mechanism, not the win from
removing it, because removing it changes everything around it. The change is
kept — it is faster, and it is the plumbing a full context parameter would
need — but the four milliseconds were not there to collect.

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

## Ceilings, measured before building — 2026-09-18

Three times in one session an idea that looked worth twenty to forty percent by
counting instructions turned out to be worth nothing. So each of these was
*bounded first*, in the real compiler on the real benchmarks, before any of it
was written. Five were abandoned on the strength of one measurement each.

| idea | how it was bounded | ceiling | verdict |
| --- | --- | --- | --- |
| eliminate bounds checks | compiled the corpus with the check removed outright | matmul 1.00x, sieve 1.13x, sort 1.03x | not built |
| carry the thread state instead of looking it up | timed a `thread_local!` access against a `&Local`, times the measured access count | ~1.06x on the allocating rows | not built |
| unboxed scalar map values | timed `counts.set(k, counts.get(k) + 1)` against a bare `counts.has(k)` | 1.00x | not built |
| strength-reduce `%` by a constant | timed 20 million `i % 8` against 20 million `i + 1` | 0.31 ns per division, ~1.01x | not built |
| hoist loop-invariant arithmetic | built it, then A/B'd against itself with the pass off | 1.01x | built and removed |

The reason is the same in every row: a modern out-of-order core hides
independent, perfectly-predicted work. A bounds check is a compare and a branch
that never mispredicts; a `sdiv` whose result nothing waits on retires behind
other work; a direct call to a tiny leaf function overlaps with its caller.
Counting instructions in a disassembly measures none of that. **The only
number worth acting on is a measured one, and the cheapest way to get it is
usually to break the thing rather than to build its replacement.**

**Releasing a value with children called `malloc`.** `dec_in` walks a value's
children with a work list, and that list was a `Vec` built fresh per call.
`Vec::new()` does not allocate, but the first push does — so releasing *any*
value that had children cost a `malloc` and a matching `free`. In a loop that
makes and drops a two-field struct, `RawVec` growth and the allocator were
three quarters of the profile.

The work list now lives in the stack frame — four slots, which is what
`StructFields::Inline` and `ListData::Inline` already hold — and spills to the
heap only for a value big enough to need it, at which point it moves everything
across and stays there, so a deep walk does not pay to refill the frame at
every step.

Making and dropping a small value: **49 ns to 28.9**, a list of one boxed
element **40.4 to 22.8**. On the benchmark suite: nothing, within noise on every
row. The suite has no program shaped like "make a small record, read it, drop
it", which is the shape most code in the package corpus is made of — so this is
recorded as a win on a shape rather than a win on a number.

Two details that were measured rather than assumed. The inline array is
`MaybeUninit`: zeroing it cost more than it saved, because the frame is built on
*every* release including the overwhelming majority that push nothing, and
`wordfreq` was three percent slower until that was fixed. And sixteen slots were
worse than four for the same reason.

## Where the remaining gap actually is

Timed head to head by processor time, and split by compiling cut-down versions
of the same program:

**`wordfreq`, 28 ms against Zig's 8.5.** Building the key costs 15 ms and the
map costs 10.5. Inside the key: making *one* string box costs **26 ns** — the
same whether it is built by one call to `grasm_str_concat` or four calls
through the string builder, which is how it is known that the calls are not the
cost. The box is: a pool pop, sixty-four bytes written, two thread-local
reaches, a `memmove` for six bytes, and drop glue on the way out. Zig's
equivalent writes into a stack buffer and allocates nothing. Removing the box
for a string that is only ever *borrowed* — built, used as a map key, dropped —
is worth the whole 10.4 ms, and it is escape analysis, which is a real
subsystem rather than a peephole.

**`tree`, 37.9 ms against Rust's 8.8.** Rust's `Vec<Node>` stores its nodes
inline and contiguous: 196,820 allocations for the whole run against our
787,300 scattered sixty-four-byte boxes. Not refcounting — layout. Closing it
means structs stored inline inside lists, which needs a notion of a borrowed
element, because `for k in n.kids` must not box one to look at it.

**`matmul`, 8.6 ms against Rust's 3.9.** Rust's inner loop is vectorised —
`umull.2d` over `q` registers, visible in its disassembly. Cranelift has no
auto-vectoriser. The bounds checks, which looked like the answer, are worth
nothing.

**`fib`, 8.9 ms against Rust's 4.0.** No allocation, no checks left to remove:
this is instruction selection and register allocation. Cranelift emits
`mov x6, x19; sub x1, x6, #1` where one instruction would do, and spills a
register it did not need to.

**`sieve`, 10.3 ms against Rust's 5.9.** Bounds checks are 1.13x of it; the
rest is not yet accounted for.

None of these five is a peephole. Four are subsystems.

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
