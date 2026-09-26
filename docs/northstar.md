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
| G2 | Geomean vs Rust | ≥ 0.95× | **0.46×**, and **0.39×** with the process floor subtracted — 0 of 8 programs at or above. `scripts/bench.sh` computes both from the numbers it prints; the figure used to be worked out by hand and could only be retyped, not reproduced | **missed** — `tree` at 5.0× still carries it; `parwords` 2.6×, `sort` 1.9×, the rest 1.5–3.0×. Moved from 0.45× on 2026-09-10 when the allocation path stopped resolving three thread-locals per value |
| G3 | Frontend throughput | ≥ 500k LOC/s | **535–547k** across consecutive runs on one machine | **met** |
| G3 | Incremental edit-to-diagnostic | ≤ 50 ms | **p50 21 ms, p95 40 ms** on the largest program in the repository (`packages/unicodenorm`, 3,489 lines) and the largest that imports, measured through `grasm lsp` by `scripts/lsp_latency.py` at load 17 on 2026-09-27. There is still no query engine: the server re-checks the whole program on every change, and that is fast enough. It was recorded as *not built*, which described an implementation rather than measuring the goal | **met at this repository's scale, not at the plan's.** MASTERPLAN §1 frames this as sub-50 ms at 1M+ lines, and a whole-program re-check at ~20 ms per 3.5k lines is nowhere near that; reaching it needs the query engine |
| G4 | Safe-subset guarantees | 100% | **223** programs identical across three engines, **223** leak-free, 27 agreeing on panic messages, **44** whole programs, 184 identical in the browser build, 28 builtins refusing under an empty journal, nine fuzzers clean — and the gap list the compiled backend refuses is down from 17 shapes to 1, which is polymorphic recursion and is permanent | **strongest area** |
| G5 | Hello-world static binary | ≤ 300 KB | 350 KB on 2026-09-27, up from 328 | **missed by 17%**. About 135 KB of `__text` is Rust's own backtrace symbolizer (`gimli`, `addr2line`, `rustc_demangle`), reached from std's default panic hook, which any panic path links. Removing it needs `-Z build-std` with `panic_immediate_abort` — nightly only — or a `no_std` runtime |
| G6 | Structured, uncolored concurrency | 1M+ tasks/node | `net_wait` gives concurrent I/O on one thread ([L-LIB-NET]); `par_map`/`par_filter` are multi-core over **boxed** values too — 3.68× on 8 cores over strings — but still refuse every effect, and `spawn` is still single-core ([E-NOT-PARALLEL]) | **partly met** |
| G7 | Capability-sandboxed dependencies | 100% | the supply-chain benchmark's tamper is refused as `performs [io] permits []`; **and it was not met between `bytes` shipping and 2026-08-24** — `read_bytes`/`write_bytes` were left off the list that charges `io`, so a `capabilities = []` package could read and write any file (F35) | **met now, and the claim was wrong before** |
| G8 | Production failure → deterministic replay | ≥ 99% | `GRASM_RECORD` writes a journal of everything a run read from outside itself; `GRASM_REPLAY` hands it back and the run reaches outside itself for **nothing** — not the reads and not the writes. All **28** journaled builtins on **all four engines** — interpreter, bundle, native and the browser build: the whole `io`, `net` and `proc` surface bar `print`/`eprint`, which are repeated on purpose. A recording made by the native build replays byte-identically under `grasm run` *and inside a wasm module that has no sockets, no processes and no filesystem* — TCP session and child process included. `scripts/replaytest.sh` runs each builtin under an empty journal and requires it to refuse. Not covered: recording in a browser (wasm32 has no filesystem to write to — replaying there works, and is checked) and worker threads (the journal is per-thread; `par_map` refuses effects, so nothing in the corpus can reach it) | **met for a single-threaded native or bundled run; the ≥99% number is unmeasured — nobody has run a Grasm program in production** |
| G9 | C ABI import overhead | zero-cost | `extern fn` takes `i64`, `f64`, `str` and `bytes` ([T-EXTERN-POINTER]), gated by `unsafe` and charged `io`; a pointer may not be *returned* | **met** |
| G10 | Adoption | ≥ 500 companies | zero — nobody outside this repository has run a Grasm program | **not started** |
| G11 | Tokens vs Python | ≤ 1.00× | **1.04×** | **4% over** |
| G11 | Tokens vs Go | ≤ 0.60× | **0.61×** | **2% over** |

### G2 moved because the measurement was wrong, not the compiler

This row used to read "4 of 7 programs at or above" Rust. That was an artifact.
`bench.sh` was adding a constant 21 ms to every cell (ProjectGold F27), and a
constant added to every measurement compresses every ratio toward 1 — so a
5 ms program and a 10 ms program read as 26 and 31, which is "at or above" if
you squint. With the timer fixed, **no program is at or above Rust** and the
geometric mean is 2.49× behind, or 2.92× once the harness's own 2 ms floor is
taken out of both sides.

Nothing got slower. Every performance change this session moved a number the
right way — `wordfreq` 117 → 53 ms, `tree` 175 → 97, `sieve` 17 → 15. The
scorecard was simply reporting a flattering error, and it reported it here
because it read it from the benchmark.

That is the third place the same 21 ms turned up: two published tables and this
row. A wrong measurement does not stay where it was made.

Two smaller rows were stale in the same way — recorded once and never
re-read. G3 said 571k LOC/s where `scripts/northstar.sh` printed 547k and then 535k on
the very next run — so it is recorded as the range it is rather than as
whichever number was on screen when somebody wrote the row. And
G5 said 328 KB, which is what the script prints, against the 329 KB a manual
`ls` gives for the same binary. Both now say what the gate says, because a
scorecard that disagrees with its own gate is the same defect as a gate that
does not run.

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
optimisation.** As of the table above, the rows sit between 1.5× and 3.0×
behind Rust with `tree` at 5.0×, so the geomean is mostly one row's problem
rather than a spread. Three candidate causes were checked on `wordfreq` and
none of them was it: the map is already a `HashMap` with sorting deferred to
iteration, so [E-MAP-ORDER] costs nothing on insert; rewriting the program's
string construction to one interpolation instead of a concatenation changed the
time by nothing; and replacing the `RefCell` borrow that every allocation makes
with a plain counter measured **zero** on a benchmark that allocates 800,000
times.

**Fixed, 2026-09-10: the allocation path resolved three thread-locals.** The
`RefCell` above really was worth nothing on its own — what it was sitting next
to was. `sample` on `tree` put `_tlv_get_addr` at the top of the profile with
about a third of the samples: on macOS a thread-local is a call, and `alloc`
reached three of them while releasing a value reached two. One structure now.
`tree` 49 ms to 42, `wordfreq` 44 to 38, `parwords` 32 to 23, and G2 from 0.45×
to 0.46× — which is the honest size of it. A third of one program's profile
buys a hundredth of the geomean, because five of the eight rows barely allocate.
`docs/performance.md` has the rest of the ledger.

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

**The representation change, done.** A struct value was the box plus a vector of
`(name, pointer)` pairs — and the names are the same for every value of a type,
so they were being repeated once per value. They now live once in a `StructTy`
the value points at, and the field *values* live inline in the box itself when
there are four or fewer, which is most structs. Four is the largest run that
keeps `RtVal` at 64 bytes, the budget every other value is already fitted to.

A two-field node went from two allocations to one. `tree` **100.3 to 83.0 ms**,
and 175 to 83 across the whole thread — 8.3x C down to 4.0x.

The intermediate step measured *worse*: the type table alone, with the field
vector still there, was 103.2 ms against 100.3, because a thread-local lookup
per construction cost more than the narrower vector saved. A one-entry cache in
front of it — a loop building one kind of node asks for the same type every
time — brought it to 98.4, and only removing the vector made it pay. Worth
recording as a case where the enabling step had to be measured *and kept*
despite being a regression on its own.

**The type descriptor, done.** The next step named above: the compiler now
emits one descriptor per struct type into the object file's data section — the
type name, the field count, and a pointer and length per field name, with a
cache word the runtime fills in the first time a value of that type is built.
Building a value reads that word and is done. Nothing is interned at run time,
so `STRUCT_TYPES`, the hash of the type name, and the one-entry cache the
previous commit had to add in front of it are all gone.

That also collapsed construction to a single call. A struct was three calls for
two fields — make the box, then name and push each field — and every push
validated a name as UTF-8, took a `RefCell` and compared lengths to work out
whether the type still needed naming. The field values now go into a stack slot
and one `grasm_struct_make` builds the whole value. `StructTy` holds no
`RefCell` any more, which is also what makes it safe to share across the
threads `spawn` starts; the descriptor cache is process-wide, and two threads
racing to fill it both build one, one wins the exchange, and nothing can tell,
because struct comparison goes by type *name* and never by the address of the
type.

`tree` **83.7 to 79.3 ms**, measured A/B interleaved, best of thirty each,
against a binary built from the previous commit. The machine was not idle, so
the ratio is what stands — 0.948 — rather than the absolute pair; the old
binary's 83.7 matching the 83.0 recorded on an idle machine says the load was
not distorting much.

**The child list, half of it.** A list was a `Vec`, so `[a, b, c]` cost the box
the value needs anyway *plus* a second allocation for three pointers. Four or
fewer elements now live in the box itself — the same bound as `StructFields`,
for the same reason. `tree`'s nodes have three children, so every interior
node's child list now fits: **77.0 to 73.8 ms**, A/B interleaved, best of
thirty each, about 197k allocations gone from the run.

`ListData` derefs to a slice, so every read the runtime already did — index,
iterate, sort, first, last — is unchanged and untouched. Only push, pop and
remove needed writing. A `Vec` that already exists is kept as it is: it has
paid for its allocation, and copying it into the box to save an allocation that
already happened would be work for nothing.

What is still left is the *box*: Rust stores a node inline in its parent's
vector, so a child costs no separate cell at all, where Grasm still allocates
one boxed value per node. Closing that needs unboxed struct storage in lists —
what `ListScalar` does for integers, done for a struct type — which is a
larger change than either of the two above.

**The allocator was the cost, not the copying.** Taking `wordfreq` apart said
so: `s + t` in a loop, six bytes of text, cost 45 ns an iteration, which is a
`malloc` and a `free` and almost nothing else. With a literal on one side it
was 78 ns, because the literal was boxed and freed too.

Every `RtVal` is the same 64 bytes, so a freed one is exactly what the next
allocation wants. Freed boxes now go on a thread-local chain — threaded through
the `rc` field of a box whose payload has already been dropped — and an
allocation pops one instead of asking the system.

    tree       73.8 -> 63.1 ms
    wordfreq   54.7 -> 49.2 ms
    s + "lit"  40.3 -> 30.2 ms per 400k

The cap is what decides whether this pays. At 8192 boxes `tree` got **worse** —
82.7 against 73.8 — because its working set is about 60,000 boxes, so nearly
every free overflowed the cap and paid the allocator anyway *plus* the pool's
own bookkeeping. At 65536 it recycles the whole tree; at 262144 nothing further
happens. Four megabytes held, and the first measurement of the idea was a
regression caused entirely by a constant.

**Building a string was quadratic, and it did not have to be.** Writing the
`parwords` benchmark five times over is what found it: C came out at 8 ms
against Grasm's 70, and the reason was not codegen. `acc = acc + piece` copied
the whole accumulator every step, so a string built in a loop cost time
proportional to the square of its length, while C — which makes you write the
buffer yourself — was linear.

`grasm_str_append` takes ownership of its left operand instead of borrowing it,
so when the count is exactly 1 the text is appended in place and the same box
comes back. A count of 1 is also what rules out `s = s + s`: two references to
one box cannot both be the only one.

The compiler hands over the variable's own reference for `acc += piece` and for
`acc = acc + piece`, which are the same statement written two ways, and for the
intermediate inside an interpolation — `"{a}{b}{c}"` built one string per piece
before this and builds one string now.

    parwords   69.7 -> 31.7 ms

The benchmark's other four implementations were rewritten at the same time, and
against this project's interest: Go was using `acc += piece`, which is
quadratic in Go too and which no Go programmer would write — it now uses
`strings.Builder`. Rust was allocating a `String` per digit and counting
characters with an O(n) scan. A benchmark Grasm wins because the other
implementations were written badly is worth nothing.

**A string literal allocated on every occurrence.** `"hello".len()` in a loop
cost 24 ns a time; the same text bound to a `let` cost nothing. Every mention of
a literal was building a fresh box for text that cannot change.

One shared `IMMORTAL` value per distinct text now, cached in a slot the backend
emits — a load and a branch, and the runtime call happens the first time each
literal is reached and never again. `Box::into_raw` rather than `alloc`, for the
same reason the small-integer table uses it: `alloc` counts a live value and
something never freed would be reported as a leak at exit.

    two million `"hello".len()`      57.9 -> 18.1 ms

Sharing it is safe because of the one place a `str` is mutated in place:
`grasm_str_append` does that only when the count is exactly 1, and an immortal's
never is — so appending to a literal copies it. Which is not a detail, it is the
whole thing: without it, the first `s += "!"` in a program would rewrite every
occurrence of that literal in it. `E-VALUE-SEMANTICS.gr` checks exactly that,
because a shared literal is an implementation choice that has to stay invisible.

**A struct field read was eight times a local read, and did not need to be.**
Taking `tree` apart again: build and free is 33.5 ms of it and the *walk* is
28.8, so half the program is reading two fields per node. Pricing that directly
— three million reads of an `i64` field against three million reads of a local —
said **12 ns against 1.5**.

Four runtime calls were behind it. `p.a` fetched the field and incremented it,
released the receiver, unboxed, then released the field.

Two of the four cancel exactly and the other two are one call:

* **The field's increment and decrement.** They exist so a boxed field handed
  out can outlive the read. A scalar's *bits* cannot outlive anything — they are
  copied on the spot — so `grasm_struct_scalar_at` returns them and never
  touches the count.
* **The receiver's.** `p.a` evaluated `p`, which increments, and released it
  after. The variable holds a reference for the whole of the read, so the count
  cannot reach zero during it. That is the argument [E-BORROWED-READ] already
  makes for a read-only method, applied where it had not been — and only for a
  plain identifier, because anything else produces a temporary this expression
  owns.

        three million i64 field reads    40.5 -> 9.6 ms   (12 ns -> 1.7 ns)
        tree                             62.3 -> 50.5 ms

The second half was worth four times the first, which is the wrong way round
from how they were found: the fused call is the clever one and the borrow is the
obvious one.

**G8: record and replay, and why the effect system made it cheap.** Knowing
which calls are non-deterministic is normally the hard part of building a
record-and-replay tool. Here it was already written down — a builtin either
declares an effect row or it cannot read anything — so the journal is just
"every call that has a row, in order, and what it answered".

    $ GRASM_RECORD=j ./program      # 86 bytes for a clock, 4 bytes of entropy
    $ GRASM_REPLAY=j ./program      # identical output, twice, on any machine

**Divergence is an error rather than a guess.** On replay the recorded call name
is checked against the call made. A different program against the same journal
says so and stops:

    at call 1 the recording says `now_millis` and this run asked for `env`

The alternative — hand back the next record anyway — produces a run that looks
like a replay and is not, and a tool whose whole promise is *this is what
happened* must not be able to lie about it. A program that reads *more* than the
recording holds is caught the same way.

Writes are recorded and not replayed against the world: replaying a
`write_file` would damage a disk to reproduce a bug. `print` and `eprint` are
the exception and are repeated for real — a replay whose output you cannot see
is not worth running.

**The coverage is now the whole surface, and is checked rather than asserted.**
Twenty-eight builtins — the filesystem, the clock, the environment, stdin,
entropy, eleven socket operations, both ways of running another program —
journaled on the interpreter and on both compiled engines. A recording made by
the native build replays byte-identically under `grasm run`, including a TCP
session and a child process, which is the case that matters: the failure happens
in the fast engine and the debugger is in the other one.

The check is behavioural rather than a grep for the hook. Under an *empty*
journal a hooked call has nothing to hand back, so it must stop with exit 7; an
unhooked call reaches the world and the program finishes. `scripts/replaytest.sh`
runs all 28 that way on both engines, and two `cargo test` gates keep the list
honest — one refuses a builtin with an effect row that says nothing about
replay, the other refuses a journaled builtin with no program in the script.
All three were confirmed by breaking them on purpose and watching them fail.

And a demonstration that a replay really does not touch the world: record a
program that writes a file, delete the file, replay it. The output is identical
and the file is not there.

**The browser engine replays too.** It cannot record — wasm32 has no filesystem
to write a journal to — but a recording made by the native build replays inside
a wasm module byte-identically, TCP socket and child process included, in a
module that has neither. `scripts/replaytest.sh` runs that comparison every time.

**The debugger takes a journal.** `"journal": "production.jrnl"` in `launch.json`
and the breakpoints, stepping and variables all work as they did — on the
recorded run. Stop after `now_millis()` and the variable holds the time the
failure happened, not the time you looked. An unreadable journal fails the
launch rather than starting a live run, because a session that quietly ran
against the real world while you believed you were watching a replay would be
worse than an error.

**And a journal does not have to have been recorded.** Written by hand it is a
script of what the world will say — the disk full, the socket refusing,
`read_file` answering something no file contains — and the program cannot tell,
because the effect system says those are the only places the world reaches it.
The failure paths become reachable without a broken machine to reach them from.
The same property, used in the other direction.

And `grasm journal <file>` prints a recording back out — every call in order and
what it answered. That is not a convenience: a journal is a file of framed bytes,
and evidence nobody can examine is not evidence. It is also what found **F37**,
where the compiled runtime answered `write_file` with `Ok(0)` and the interpreter
with `Ok(())` — a difference no direct run could see, because a compiled build
prints unit from the *type*, and one that a cross-engine replay printed as `0`.

What is left: **recording** in a browser, and **worker threads**, because the
journal is per-thread. `par_map` and `par_filter`
refuse an effectful function, so no program that exists can reach the second one
— but the honest statement is that the guarantee is single-threaded today.

The ≥99% target is about crashes *in production*, and G10 is zero. That number
cannot be claimed until somebody outside this repository runs a Grasm program.

**Two of these four were worth nothing on the benchmark set, and that is the
honest way round to say it.** After the literal cache and the borrowed field
path, the published table moved by at most 1 ms in any cell — `parmap` 104 to
103, `parwords` 32 to 31, and the rest identical.

Both are real: 57.9 ms to 18.1 on the literal loop, 23.8 to 9.1 on the chain,
measured the same way as the two that *did* move the table. What they are not is
*visible here*, because these eight programs do not put a string literal in a
hot loop and do not have a nested struct. A benchmark set is a sample of program
shapes, and a win outside the sample is invisible rather than absent.

The temptation is to publish the microbenchmark and let a reader assume it
compounded. It did not, and the table above is what the table above says.

**And every step of a field path.** `o.i.v` fetched `i` and incremented it, read
`v`, then released `i` — a pair that exists only to protect a value the *root*
already protects. Every intermediate in a path rooted at a live local is
reachable from that local, so none of them can be freed during the read.

    two million `o.i.v`      23.8 -> 9.1 ms

Only a chain of field names. An index or a call in the middle can run code that
frees what an earlier step borrowed, so those keep the owning path — and
`T-STRUCT.gr` checks that one answers the same as the other, along with the
three ways this could stop being invisible: a chain ending in a boxed field has
to hand out an owned value or it dangles when the root changes; a write through
one copy of a shared value must not be visible through the other, even though
the borrow hands both the same interior pointer; and the plain scalar read has
to still be right.

**And the same borrow on an index.** `xs[i]` evaluated `xs`, which increments,
and released it after — the identical cancelling pair, in the identical shape.
There was already a `pin` path that avoided it inside a `for` loop, and nothing
outside one.

    three million `ps[i].a` reads      42.9 -> 27.8 ms

Two places found by the same sentence, which is the argument for writing the
sentence down: *a read through a plain local borrows it, because the variable
holds a reference for the whole of the read.* [E-BORROWED-READ] said it about
methods. It is true of every read.

**The price of atomic reference counting, measured.** "Parallelism needs
atomic refcounting, and atomic refcounting is expensive" is the sentence that
kept `par_map` restricted to machine words — the workers share nothing because
sharing would race on a `Cell<i64>`. Nobody here had priced it.

    tree       63.3 -> 66.0 ms   (+4.3%)
    wordfreq   46.7 -> 45.7 ms   (-2.1%)
    sort       19.4 -> 19.3 ms
    sieve      15.4 -> 15.5 ms

About half a percent on the geometric mean. `tree` pays because it is the most
reference-count-heavy program here and nothing else in it is slow; everything
else does not move, on a machine whose uncontended atomics are one instruction.

Taken, along with `Rc<Vec<u8>>` becoming `Arc<Vec<u8>>` — the one payload left
that could not cross a thread, and exactly the kind of value a parallel worker
would be handed. Single-threaded behaviour is identical: 218 identical, 218
leak-free, 27 agreeing, 31 ProjectGold, clippy 0, `cargo test` green.

**And then `par_map` and `par_filter` took it.** With the count atomic, a
worker touching a boxed value is safe, so the fallback to sequential for
anything that is not a machine word is gone. Elements, captures and results may
all be boxed; a pointer crosses a thread as eight bytes exactly as a word does.

    8,000 strings, ~200 concatenations each, 8 cores
    map      128.5 ms
    par_map   34.9 ms      3.68x

The calling convention is what makes the counting come out right: the lifted
body **borrows what it is handed and owns what it returns**. It increments the
element and each boxed capture on entry, so the releases it makes on the way
out balance, and the caller holds the input list for the whole call. That is
the traffic that made the count have to be atomic in the first place.

One counter needed moving. `LIVE` — what the leak checker reads — is
thread-local, so that allocating stays a load and a store rather than an
atomic. That is right until a value crosses a thread: a worker builds a string
and the main thread frees it, so one counter goes up and the other goes down,
and the check would report a leak of minus four. A worker folds its balance
into a global `ESCAPED` once, as it ends, and the exit check adds the two. No
cost on the allocation path.

`[E-PAR-MAP-ORDER]` and `[E-PAR-FILTER-ORDER]` needed no change, which is the
point of how they are written: they promise that the answer does not depend on
how many cores ran it. A sequential fallback honoured that promise and so does
this. Their conformance programs now carry the boxed cases.

**A copy-on-write clone was losing its inline storage.** `cow_list` collected
the elements into a `Vec` and wrapped it, so a copied three-element list came
back on the heap even though three elements fit in the box — and then allocated
again on the next push. Built by pushing instead. Worth nothing on `tree`,
which never copies a list, and worth an allocation each to every program that
does.

**The empty list, interned and reverted.** Two thirds of a tree's nodes are
leaves, and a leaf's child list is empty, so a shared immortal empty list would
have removed a third of the run's boxes. Copy-on-write makes it safe — every
in-place operation on an empty list is a no-op, a panic, or goes through
`cow_list`, which copies anything whose count is not exactly 1.

It measured **67.1 ms against 63.3**, and 62.4 once the clone above was fixed —
1.4%, for a shared mutable value. The reason is that it moves the cost rather
than removing it: a leaf saves one box, and every *interior* node now pays a
full `cow_list` clone on its first `push` where it used to push in place. The
tree has 9,841 interior nodes to 19,683 leaves, so most of the saving is spent
again immediately.

**`dec`'s worklist, measured and reverted.** `dec` walks a structure
iteratively with a `Vec` worklist, and that `Vec` is made fresh on every call —
which reads like an allocation per value freed, on the path whose whole job is
to give memory back. Keeping it in a thread-local and handing it back with its
capacity measured **74.6 ms against 63.1** on `tree`, an 18% *regression*.

The reasoning was wrong about how often the allocation happens. One `dec` call
frees an entire structure: the tree's 29,524 nodes are released by the single
call at `let t` going out of scope, and the worklist is reused within that walk.
So there were twenty allocations to save across the whole run, and the price was
two thread-local accesses and two `RefCell` borrows on *every* `dec` — including
the great majority that only decrement a count and return. A saving spread over
the wrong denominator.

**The layout argument is now checked.**

    const _: () = assert!(std::mem::size_of::<RtVal>() == 64);

Fifteen inline bytes of text, four inline struct fields, four inline list
elements: each of those numbers was chosen by measuring this struct and
stopping where it stopped growing, and that reasoning lived only in prose. A
fifth inline field would have widened every boxed integer in the program and
nothing would have said so.

**An S1 the fuzzer found on the way.** `fuzz_generics` reported 58 of 200
programs disagreeing — present before this work, not caused by it. An `inout`
argument naming a *place* (`bump(b.n)`) passed the field's **boxed** value where
the callee expects a pointer to a slot holding its own type, so `n += 1`
incremented the address of a box and wrote that back into the field. Reading the
field then said `unbox: not an i64` on a program `grasm run` answered — a
disagreement, not a refusal. Unboxed on the way in and boxed on the way back
now, which is what a place holds. `spec/conformance/E-INOUT-PLACE-WRITEBACK.gr`
covers it across a nested field, a list element, `str`, `f64` and `bool`,
because the corpus had no program that passed a scalar field as `inout`.

**The same defect in the interpreter, measured and reverted.** `Value::Struct`
holds `name: Rc<String>` and `fields: Rc<Vec<(String, Value)>>`, so building one
allocates a `String` per field name and a `String` *and* an `Rc` for the type
name — exactly the waste that cost the compiled runtime a third of `tree`.
Interning both as `Rc<str>` from a table made construction refcount bumps
instead: **57 ms against 56**, on the same program, with all 218 difftest
programs still identical.

So the lesson did not transfer. In the compiled engine the allocation was most
of what building a struct cost; in the tree-walker it is lost among the
dispatch, the `Value` clones and the `Rc` traffic that interpreting anything
costs. Fifteen edits to the *normative* engine for nothing measurable is churn
in the one component every gate trusts, so it is reverted — and recorded,
because "the runtime had this exact problem" is a compelling argument that
happens to be wrong here.

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
