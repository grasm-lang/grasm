# North-star goals, measured

MASTERPLAN §2.3 sets ten measurable goals and §19.3 adds an eleventh. Four of
them had never been measured — not missed, *unmeasured*, which is worse: a
number nobody takes cannot be argued with or improved, and G3 was found to be
violated only by accident, when an evaluator happened to time a chain of field
accesses and found the checker taking exponential time on it.

`scripts/northstar.sh` takes the measurements that can be taken. It prints a
scorecard rather than failing on a miss, because three of these are missed today
and a gate that is red on purpose is a gate people learn to ignore. It *does*
fail when a measurement cannot be taken at all, which is how a metric quietly
stops being collected.

Figures below were taken on 2026-08-17 on an idle machine. Load matters: the
same build measured 478k LOC/s at load average 9 and 571k at load average 3, so
the scorecard takes the best of five runs and subtracts the process floor.

| # | Goal | Target | Measured | |
|---|---|---|---|---|
| G1 | Time for a Go/TS dev to ship a real service | ≤ 1 week | — | **unmeasurable without users** |
| G2 | Geomean vs Rust | ≥ 0.95× | 4 of 7 programs at or above; `wordfreq` 2.5× behind | **partly met** |
| G3 | Frontend throughput | ≥ 500k LOC/s | **571k** | **met** |
| G3 | Incremental edit-to-diagnostic | ≤ 50 ms | — | **not built** — there is no query engine |
| G4 | Safe-subset guarantees | 100% | 204 programs identical across three engines, 204 leak-free, row and FFI soundness fuzzers clean | **strongest area** |
| G5 | Hello-world static binary | ≤ 300 KB | 328 KB | **missed by 9%** |
| G6 | Structured, uncolored concurrency | 1M+ tasks/node | `net_wait` gives concurrent I/O on one thread ([L-LIB-NET]); `spawn` is still single-core ([E-NOT-PARALLEL]) and `par_map` still refuses every effect | **partly met** |
| G7 | Capability-sandboxed dependencies | 100% | the supply-chain benchmark's tamper is refused as `performs [io] permits []` | **met, for what exists** |
| G8 | Production failure → deterministic replay | ≥ 99% | — | **not built** |
| G9 | C ABI import overhead | zero-cost | `extern fn` takes `i64`, `f64`, `str` and `bytes` ([T-EXTERN-POINTER]), gated by `unsafe` and charged `io`; a pointer may not be *returned* | **met** |
| G10 | Adoption | ≥ 500 companies | zero — nobody outside this repository has run a Grasm program | **not started** |
| G11 | Tokens vs Python | ≤ 1.00× | **1.04×** | **4% over** |
| G11 | Tokens vs Go | ≤ 0.60× | **0.61×** | **2% over** |

## What the numbers say

**G3 is met: 571k LOC/s against 500k.** It got there in four steps, all of them
found by measuring rather than guessing:

1. Three superlinear defects fixed — a field chain that was exponential, a
   method chain that was exponential, a nested tuple literal that was cubic.
   Before those, an ordinary program with a long projection chain did not finish
   at all, and no throughput figure meant anything.
2. Memoizing inference for the forms that cannot hold a closure. Added for the
   pathological chains, it turned out to be **2.2× on ordinary code too** — 257
   ms to 115 ms on a 47,000-line program.
3. Not freeing the syntax tree at exit. A third of `grasm check` was
   `drop_in_place` walking the tree node by node to release it, immediately
   before the process ended and the operating system reclaimed the whole address
   space in one step.
4. A non-cryptographic hasher for the tables keyed by span. The default hasher
   resists an attacker choosing collisions; the keys here are byte offsets in a
   file the compiler is already reading.

Two things were nearly shipped and caught. Exiting the process outright to skip
*all* destructors is simpler than step 3 and collapsed every program's own exit
code to 1, because an `ExitCode` cannot be read back. And the first version of
this scorecard shelled out for each timestamp, putting 25 ms of helper startup
*inside* the interval — a measurement of itself.

The second half of G3 — the ≤50 ms incremental figure, which is what the goal is
really about — still has no measurement, because the "compiler as a queryable
incremental database" the executive summary describes was never built. What
exists is a batch compiler with an object cache.

**G11 reads 1.04× of Python and 0.61× of Go**, against targets of 1.00× and
0.60×. Two separate things got it there, and only one of them is an improvement.

**The ruler was wrong.** The first measurement counted tokens with a regular
expression, which cannot compare these three languages fairly: Python's grammar
has `INDENT`, `DEDENT` and `NEWLINE` tokens and Go's lexer inserts semicolons,
so a regex over punctuation gives both of those away free and charges Grasm for
every brace. Braces were 55% of the apparent gap. Counted by each language's own
lexer — `grasm tokens`, Python's `tokenize`, Go's `go/scanner` — the same
programs read 1.04× rather than 1.22×. **The language did not get shorter; the
measurement got honest.**

**The language did also get shorter**, by 1.41× → 1.22× on the *unchanged* ruler,
from two things. A range gained the list combinators ([L-LIB-RANGE-OPS]), so
building a list from a count stopped needing `var xs: [i64] = []`, a `for`, a
`push` and two closing braces. And three of the six benchmark programs were
accumulating by hand where `.sum()` or `.values()` already existed — not a
language cost, and not what the benchmark README promises. One rewrite went the
wrong way and was reverted: replacing `sort`'s push loop with a
map-then-overwrite made it eleven tokens longer, and Python writes that one as a
push loop too.

Per program, by each language's own lexer:

| | fib | sieve | matmul | wordfreq | sort | parmap |
|---|---|---|---|---|---|---|
| vs Python | 1.12× | 1.13× | 1.10× | **0.98×** | 1.13× | **0.79×** |
| vs Go | 0.93× | 0.85× | 0.74× | **0.66×** | 0.70× | **0.29×** |

`parmap` is the number to read: **0.29× of Go and 0.79× of Python** — a
compiled, statically safe, GC-free language at four fifths of Python's length on
the concurrency program. That is the thesis holding where it is tested hardest.

**The remaining 4% is 19 tokens across six programs, and all of it is the entry
point.** `fn main() { … }` costs six tokens — `fn`, the name, two parentheses,
two braces — and six programs pay it: 36 tokens. Grasm's actual code is 521
against Python's 538, or **0.97×**.

That figure is *not* the one reported above, and deliberately. A Python
benchmark is a script and has no entry point; a compiled systems language names
where a program starts. Comparing the two without the entry point is comparing
different things — but so is quietly dropping it because the number comes out
better, which is the move this file exists to refuse. The goal is missed by 4%,
the 4% is entirely `fn main`, and both sentences are true.

Allowing top-level statements as an implicit `main` would close it and is an
exact rewrite, so it would cost no kernel form. It is not done, because it gives
a language two ways to write a program to win a metric by 19 tokens, and
principle 5 of the master plan is "one way to do it".

**G5 is missed by 9%**, down from 46%. The binary went 437 KB → 328 KB by
stripping the symbol table, which nothing reads at run time: a compiled program
carries no reflection and a `--native` build prints no stack trace at all
([E-PANIC-TRACE]). Two other levers were tried and reverted, both measured:

* `panic = "abort"` saved **1 KB**. The 39 KB of unwinding tables in a compiled
  program come from Rust's *precompiled* `std`, which the setting cannot reach.
* `opt-level = "z"` on the runtime made the binary **4 KB bigger**. Optimising
  the runtime for size blocks the inlining that lets link-time optimisation
  delete the code around it — the archive shrinks and the binary grows.

The remaining 28 KB is `std`: `core::fmt`'s formatting machinery, the allocator,
and those unwinding tables. Reaching 300 KB needs `-Z build-std` (nightly, so
not available to a stable toolchain) or a `no_std` runtime, which means
reimplementing float formatting, allocation and threads. That is a real project
with a known shape, not an open question — and it is the only thing between here
and the goal.

**G2's one real gap is the value representation, and it is not a missing
optimisation.** `wordfreq` is 2.5× behind Rust; every other program is within
1.2× of the fastest. Three candidate causes were checked and none of them was
it: the map is already a `HashMap` with sorting deferred to iteration, so
[E-MAP-ORDER] costs nothing on insert; rewriting the program's string
construction to one interpolation instead of a concatenation changed the time by
nothing; and replacing the `RefCell` borrow that every allocation makes with a
plain counter measured **zero** on a benchmark that allocates 800,000 times.

**Fixed, 2026-08-18: `(a..b).map(f)` is one loop, not two passes.** `sieve` 52 ms
to 39 ms, and `range.map` on two million elements 20 ms to 7 ms — faster than the
hand-written `push` loop it replaces.

A range combinator was lowered by first turning the range into a list and then
running the list method over it: two passes and an allocation the size of the
input. Decomposing the sieve showed the flags cost more to *build* than the
sieving cost to run — 20 ms against 5 ms of reads and 3 ms of writes.

When the function is a closure written at the call site, the loop now binds its
parameter and its body becomes the loop body, so there is no intermediate list
and no call at all. A named function takes the ordinary path, where the call has
to happen anyway.

This needed the same help from the checker as the map fusion below: the
accumulator needs a written element type, and the closure body's type is not
knowable at the point the loop is synthesised, because the loop variable it
mentions is not bound yet.

**Fixed, 2026-08-18: `m.get(k) ?? d` no longer builds an `Option`.** 149 ms to
116 ms on `wordfreq`, and the gap to the fastest implementation narrows from
4.6x to 3.6x.

`grasm_map_get` returned `Some(v)` — an `RtVal` *and* a `Vec` for its one field —
which `??` unwrapped and freed on the very next instruction. Measured over two
million lookups, that wrapper cost ~85 ns each, three times the lookup it wrapped.
The runtime now answers with a null pointer when the key is absent and the
backend branches on that, so nothing is allocated.

The branch is kept deliberately. `??` is lazy, and a fused form that evaluated its
default eagerly would change what `m.get(k) ?? fail(e)` means — which
`spec/conformance/E-COALESCE-DIVERGE.gr` caught when a first version did exactly
that.

Getting it to fire needed the checker's help. The backend's type probe is
structural, so for `var counts = map()` it sees `map()` and learns nothing; the
value type comes from later use. The checker now publishes it, resolved *after*
inference finishes rather than at the call site, where it is still a variable.
Without that the optimisation fired on annotated maps and declined on inferred
ones, which is the common case: 146 ms against 113 ms for the same program.

**Measured decomposition, 2026-08-18.** The claim above — that what is left is
string boxing — is only half right, and the half it gets wrong matters. Splitting
the benchmark into its parts and timing each back to back:

| part | time |
|---|---|
| whole benchmark | 123–148 ms |
| building the strings, no map at all | ~58 ms |
| the map work, with only 8 distinct keys | ~74 ms |
| interpolation alone | ~21 ms |
| the loop and arithmetic alone | ~1 ms |

**The map is at least as expensive as the strings**, on a map that never exceeds
eight entries. At 800,000 operations that is ~90 ns each, and most of it is
hashing — which is a *deliberate* cost, for the reason recorded below: map keys
are user data and the default hasher is what stops a program reading untrusted
input from being hash-flooded. It is not a defect to be optimised away.

**One attempt, measured and reverted.** `MapKey::of` calls `to_string()` on the
key, so `set` allocates a `String` purely to look something up. Making `RtStr`
hold `Arc<str>` so that becomes a refcount bump did work — map operations fell
from ~74 ms to ~68 ms. It also made string *creation* 32% slower (58 → 77 ms),
because `Arc::from(String)` allocates a second buffer and copies the bytes into
it. This benchmark creates more strings than it looks up, so the trade was a net
loss and the change was reverted.

Note also that the `get` path was already allocation-free: `with_key` borrows a
thread-local scratch `String`. The optimisation was only ever available on `set`.

**Small-string optimisation, measured and kept — but only half of it.** Every
string was a refcounted box holding a `String`: two allocations for text that is
usually a word. `RtStr` now stores up to fifteen bytes inline. Fifteen because
that is exactly the size of the `String` it replaces, so `RtStr` stays 40 bytes
and every boxed value in the runtime stays 64; a larger buffer would make every
integer and bool pay for it.

The inline buffer *alone* made things **7% slower** (298 → 321 ms on a loop that
builds two million short strings). Storage was never the cost. Every constructor
still built a `String` first and then copied it inline, so the change added a
copy and a branch to a path that already allocated.

The win is in the constructors, not the storage. `RtStr::of_i64` writes decimal
digits straight into the inline buffer, and `RtStr::of` takes borrowed text, so
interpolation, `trim`, `substring` and the character methods stop allocating a
`String` that exists only to be handed over one line later. With those, the same
loop runs in **238 ms (−20%)** and wordfreq in **73 ms, down from 81**.

The lesson is the same one the `i64_text` attempt taught: *the digits were never
the cost, the allocation was* — and an optimisation that changes where a value
is stored buys nothing until the code that produces the value stops allocating.

**Short map keys, measured and kept.** With inline storage in place, the same
trick applies to the key: `MapKey::of` called `to_string()` on every key on every
`set`, including a key already in the table — four hundred thousand allocations
for eight distinct one-byte keys. `MapKey::Str` now holds the inline
representation, so a short key allocates nothing, and the probe buffer is only
needed for long ones.

Measured on one machine, minutes apart: the map half of wordfreq 44.6 → 41.5 ms,
and wordfreq itself 73 → 70 ms. Small, and positive: comparing and hashing now
go through a match instead of a `String` deref, and that costs less than the
allocation it replaces.

It is the same shape as the `Arc<str>` attempt that was reverted, and it works
where that failed for a specific reason — `Arc::from(String)` allocated a
*second* buffer, so the key got cheaper and creating a string got 32% dearer.
Inline storage adds no allocation to either side.

Correctness mattered more than the 4%. `MapKey` derives `Hash`, `Eq` and `Ord`,
and [E-MAP-ORDER] rests on the derived `Ord` being lexicographic, so `StrData`
implements all four **by content**. Deriving them would have made `Inline("a")`
and `Heap("a")` different keys — the same text reached two ways landing in two
slots, which is a wrong answer rather than a slow one.

**Half of every recursive call is the guard that makes deep recursion safe.**

[E-RECURSION-DEPTH] requires that recursing too deeply *panics* with exit 101
and "does not abort, crash, or corrupt the process". The backend implements it
by calling `grasm_stack_check` on entry to every function that can recurse.

Measured on `fib(32)`, which is nothing but calls and additions:

| | |
|---|---:|
| with the check | 22.9 ms |
| call kept, check's body emptied | 20.4 ms |
| no call at all | 11.6 ms |
| the same program in C | 8.4 ms |

**The call is 8.8 ms of that and the check itself is 2.4 ms.** An opaque call at
function entry forces the register allocator to spill everything live across
it — in `fib` that is the argument — so the cost is not the work the callee
does, it is that a callee exists. Grasm is 3.3x C on this program with the
guard and about 1.4x without it, which makes this the largest single
performance item in the language for recursive code.

**And then the scale turned out to be an artifact of `fib`.** The same
measurement on a realistic recursive workload — building a 3-ary tree nine deep
and walking it, thirty times, which allocates and reads fields on every call —
costs **1.9%** (316.8 ms with the guard, 310.9 ms without). Not 49%.

`fib`'s body is one comparison and one addition, so a fixed per-call cost *is*
the program. Nothing anybody writes looks like that. The guard is a per-call
constant, and its share of a program is set by how little the program does per
call — which makes `fib` the maximum, not the typical.

That reordering matters more than the number did. On 49% a `SIGSEGV` handler
looks worth the platform-specific unsafe code and the risk of misreporting a
genuine null dereference as a stack overflow. On 1.9% it plainly is not, and
the honest conclusion is that this is a microbenchmark artifact with a real but
small tail — worth knowing, not worth trading a correctness guarantee for.

Three ways out are recorded anyway, because the finding is still true and the
next person to measure `fib` will find it again:

* **Cranelift's own `ir::Function::stack_limit`** emits the comparison in the
  prologue with no call at all — and **traps**. A trap is `brk` on aarch64,
  which is the crash the clause exists to prevent. It is the right mechanism
  with the wrong failure behaviour.
* **A `SIGSEGV` handler on an alternate stack**, turning the guard page into
  the required message and exit status, which is what Rust itself does. That
  deletes the per-call cost entirely and moves a safety guarantee into an
  async-signal-safe handler, which is a much larger change than it looks.
* **Inlining the comparison** needs the per-thread floor readable from
  generated code. A plain global is wrong the moment a `par_map` worker
  recurses on its own stack, and Cranelift has no portable thread-local read.

Recorded rather than rushed: the guard is a *correctness* guarantee, and
trading it for 8.8 ms of a microbenchmark — 1.9% of anything real — is how a
language gets a footnote saying deep recursion sometimes segfaults.

Two cheaper things were tried and measured before the analysis above.
Colocating the call changed nothing (22.9 to 22.6 ms), and making it take no
argument at all — the callee's own frame standing in for the stack pointer, so
no scratch slot and no argument setup — measured 23.0 ms and was reverted.
Between them they establish that the cost is the *clobber*: an opaque call at
entry makes the register allocator spill everything live across it, and nothing
about the call's shape changes that.

**Runtime calls are colocated now, which was measured and kept.** The runtime is
a static archive linked into the same binary, but every call to it was declared
`Import` and went through the PLT. Marking them colocated makes each a
PC-relative branch: `sieve` 17 to 15.3 ms, `sort` 20 to 19.2, `wordfreq` 54 to
53.7. It does *not* help `fib` at all — 22.9 to 22.6 — which is the evidence
that the indirection was never the cost there, the clobber was.

**The benchmark suite had no allocation-shaped program, and adding one found
the worst row in the language.**

`fib`, `sieve` and `matmul` measure arithmetic and indexing; `wordfreq`
measures strings and a map; `sort` a list; `parmap` threads. None of them
measures what a parser, an interpreter or a compiler actually spends its time
on — building small linked structures and walking them. So `tree` was written
in five languages: build a 3-ary tree nine deep, sum it, drop it, twenty times.
All five print `294954900`.

On a machine too busy to publish absolute numbers, the ratios were stark:
Rust 9.9 ms, Go 20.3, C 21.1, **Grasm 175.1** — eight times C and eighteen
times Rust, where the worst existing row was 2.4x.

The first cause was already solved elsewhere in the same file. `RtPayload::
Variant` holds its names as `&'static str`, with a comment saying that copying
them "made `Some(x)` cost two heap allocations before the payload".
`RtPayload::Struct` still held a `String` for the type name *and for every
field name* — so a two-field node allocated four times to say words the binary
already contained, before storing anything. The names come from the compiled
binary's read-only data and live as long as the program, exactly as a variant's
do.

**175.1 to 132.6 ms**, same checksum. Then reading a field turned out to be
worse than building one.

`grasm_struct_get` took the field name as a pointer and length and called
`take_str` on it — **a heap allocation on every field read**, to hold a name
the binary already contained. Borrowing instead took a loop reading one field
three million times from 100 ms to 48 ms.

What was left was the walk. Reading the *last* of six fields took 153 ms where
the first took 100, which is what a linear scan looks like from outside. But
the order is not arbitrary: the backend already sorts a struct literal into
declaration order before it stores anything, which is why `P{ b: 2, a: 1 }` and
`P{ a: 1, b: 2 }` print identically and compare equal. So the position of a
field is known at compile time, and a read is a load.

    reading the first of six fields    100.4 ms -> 35.1 ms
    reading the last of six fields     152.7 ms -> 35.2 ms

Flat, which is the point. `tree` is **175.1 to 97.1 ms**, and 4.8x C rather
than 8.3x.

Writing a field was then the slowest thing left — three million writes took
157 ms against 66 ms for six million reads, because the write still walked the
names a read no longer did. Indexed the same way: **157.3 to 85.3 ms**.

**One attempt, measured and reverted.** The compiler knows how many fields a
struct has, so `grasm_struct_new` could size the vector once instead of pushing
into an empty one and reallocating on the way. It measured 98.5 ms against
97.1, and `alloconly` 137.0 against 132.6 — no better, and inside the noise
either way. Reverted, and recorded here so the next person does not spend the
afternoon on it.

What is left is representation rather than waste: Rust stores child nodes
inline in the parent's vector and allocates about once per node, where Grasm
allocates the box, the field vector, the child list and its storage. Closing
that means changing what a struct *is*, not what its accessors do — which is
where this thread stops until there is a design for it.

**The counting idiom hashed twice, and now hashes once.**

```grasm
counts.set(w, (counts.get(w) ?? 0) + 1)
```

Written out, that is two lookups of the same key: one to read and one to store.
The C program this benchmark is measured against hashes once and increments the
slot in place, so Grasm was paying a cost the comparison was not — and every
language answers this idiom with one lookup (`m[k]++` in Go,
`*m.entry(k).or_insert(0) += 1` in Rust, `Counter` in Python).

The backend recognises the shape and calls one runtime operation that reads,
adds and writes back under a single hash. Measured on one machine minutes
apart: **wordfreq 70 → 53.7 ms**, with the same checksum — a fifth of the whole
program.

The recogniser is deliberately narrow, and every restriction buys a guarantee
that fusing cannot change what the program does: the map and the key must be
spelled the same on both sides, the key must be an expression whose evaluation
nothing can notice happening once rather than twice, and the default and the
addend must both be integer literals — a computed default would lose `??`'s
laziness, which is a defect this repository has already had once. Two different
maps, two different keys, a non-integer value type and a copy of the map
watching are each checked in `scripts/fuzz_fusions.py`.

**The hasher is untouched.** The saving is one hash, not a cheaper one. Map keys
are user data, and Rust's default hasher is what stops a program that reads
untrusted input from being hash-flooded — the C implementation uses FNV-1a and
has no such protection, so Grasm is still paying more per hash on purpose.

**G1 and G10 cannot be moved from inside this repository.** No human has run a
Grasm program. Every signal the project has is model-generated, and the two
goals that measure whether the language is *good to use* are the two with no
data at all.

## What this changes

The scorecard says the effort has gone almost entirely into G4 and G7 — safety
and capability soundness — which are the two strongest columns and were never
the differentiator. G3, G5 and G11 are all *measurable, close, and unattended*;
G1 and G10 need someone outside; G6, G8 and G10 measure features that do not
exist.

Re-run it with `./scripts/northstar.sh`.
