# Where the time goes

Grasm compiles through Cranelift and runs on a reference-counted runtime. This
file says what has been measured, what was tried and rejected, and what is left
— so the next person to open a profiler starts from evidence rather than from
the same three guesses.

`benchmarks/README.md` has the cross-language table and the rules for taking
it. This is the other half: why the numbers are what they are.

## The standing gap

Measured 2026-09-22 by `scripts/bench.sh` — wall clock, best of three,
checksums verified against the interpreter first.

| program | c | rust | go | nim | zig | grasm | vs rust |
|---|---:|---:|---:|---:|---:|---:|---:|
| `wordfreq` | 30.70 | 25.44 | 17.72 | 21.89 | 9.46 | **18.61** | **0.73x** |
| `sort` | 36.58 | 11.65 | 50.27 | – | – | **10.20** | **0.88x** |
| `fib` | 5.32 | 5.66 | 6.04 | 5.60 | 5.59 | **5.35** | **0.95x** |
| `parwords` | 7.87 | 11.38 | 10.37 | – | – | 11.27 | 0.99x |
| `sieve` | 7.10 | 7.04 | 8.05 | 9.08 | 7.39 | 7.60 | 1.08x |
| `parmap` | 61.05 | 51.60 | 90.20 | – | – | 69.90 | 1.35x † |
| `matmul` | 5.24 | 5.16 | 8.21 | 9.31 | 5.59 | 9.60 | 1.86x |
| `tree` | 20.62 | 9.86 | 21.00 | – | – | 25.49 | 2.59x |

† **`parmap` is contaminated.** The machine's load average reached 6.66 by the
end of this run, and a `par*` row is the most load-sensitive thing here: the
same binary measured 55.48 ms an hour earlier. Read it as ~1.07x, and read
`G2` as about 0.86 rather than the 0.836 printed below.

`G2` — the geometric mean of rust/native — printed **0.836x** against a target
of 0.95x, from 0.816x the previous day and 0.62x the day before that. With the
2.28 ms process floor subtracted from both sides, 0.794x.

**Four rows now beat Rust.** `wordfreq` by 37% and over C by 65%, after the
byte-keyed map took 400,000 allocations down to 3. `sort` by 14%, and by three
and a half times over C. `fib` by 6% — and by 4% over Zig, which is the first
row here to beat Zig outright. `parwords` is 1% ahead, which is inside what a
`par*` row resolves, so parity rather than a win.

`sieve` is within 8% of Rust and 3% of Zig. What is left is two rows:

* `tree` — 2.59x, and **2.245x of that is the representation**: the same
  program in Rust with `Vec<Box<Node>>`, which allocates 787,300 times exactly
  as this does, runs 22.46 ms against 10.00 for `Vec<Node>`. Grasm's own
  overhead over Rust doing the identical work is 1.22x. Storing nodes inline
  is the only thing that reaches this row.
* `matmul` — 1.86x. All instruction quality inside one loop, with no runtime
  call anywhere in its profile. Bounds checks are worth 1.081x and a vectoriser
  1.21x; the remaining 2.36x against Rust's *scalar* kernel is not explained by
  checks, index arithmetic, instruction count or unrolling, and naming it needs
  hardware counters.

**Arithmetic, calls, sorting, text and multi-core throughput are at or near
parity, and text is now ahead of Rust and of C. Allocation is not, and neither
is vectorised arithmetic.**

### Since that table, 2026-09-23 to 26

Interleaved A/B against the previous commit each time, not a new `bench.sh`
table — the machine sat between load 6 and 160 for these three days, and the
table above stands until a quiet one replaces it.

| change | row | before | after | |
| --- | --- | --- | --- | --- |
| walk a field without counts (`62244d3`) | `tree` | 26.44 | 25.20 | 1.045x |
| tail takes its last-read locals (`2c82398`) | `tree` | 25.01 | 24.34 | 1.03x |
| first push onto `[]` skips copy-on-write (`5ce58a6`) | `tree` | 24.26 | 23.58 | 1.03x |
| rotate a simple `while` (`520f481`) | `sieve` | 7.56 | 6.46 | 1.17x |

**`sieve` is ahead of Rust and Zig, narrowly.** Forty-one interleaved runs at
load 17: Grasm 6.95 ms, Rust 7.10, Zig 7.45. A 2% lead on a loaded machine is
not a result to lean on; it is the first time the row has been ahead at all.

**`tree` is 1.12x faster and still about 2.3x behind.** `total` now makes no
reference-count call — `for k in n.kids` reads the list in place when the body
never mentions `n`, and the one-level splice of `total(k)` binds `k` borrowed —
and `build` makes none either, since `kids: kids` takes the local rather than
copying it. What is left is the representation, as above: a box per node.

**Rotating `for i in a..b` as well: noise on every row, dropped.** The loops
where the extra jump would show are the unrolled ones, and those only reach the
header for the remainder.

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

**A parameter the callee only reads needs no reference count.** Passing a
collection to a function cost a retain at the call and a release at the
callee's exit — an atomic pair, 10.1 ns, on a call that may do nothing but
read one element. A parameter is *lent* instead of given when every mention of
it in the callee's body is the receiver of an index or a field read
(`param_is_borrowed`); anything else — returning it, storing it, handing it
on, calling a method on it, capturing it in a closure, rebinding it — keeps the
owning convention. Deny by default, so an unrecognised shape is owned.
Measured on the mechanism: passing a collection **9.78 ns to 1.28**, the same
as passing an `i64`; a call that also indexes it 9.66 to 2.84. On the
benchmarks it is worth 1.07x on `tree` and 1.03x on `parwords`, and nothing on
the ones that do not pass collections around. Done, 2026-09-20.

Two defects on the way, both of which say something about the change. The
first: a borrowed parameter must be skipped on *every* release path, and the
`return` path is a separate one — with it missed, a program released a
reference it never took, and `leakcheck` passed all 497 programs, because a
premature free is not a leak. That is what `usecheck.sh` was built for. The
second: a convention is a contract between two sides, so every place a
function can be *reached* has to know about it. A function used as a value is
reached through a wrapper, since the closure ABI has one shape for every
function — and the wrapper forwarded the caller's reference to a callee that
no longer released it. Two programs leaked, one of them 170,202 allocations.
The wrapper is where the two conventions meet and is now where the release
happens; the hidden stack-floor argument had already needed exactly this.

**Every element the same value is a fill, not a loop.** `(0..=limit).map(|_i|
true)` is how a program asks this language for a buffer — there is no
`vec![x; n]` to write instead — and it was lowered to a `for` loop pushing two
million times. That cost **4.03 ms of the sieve benchmark's 10.25**, measured
by timing the allocation on its own; the same bytes take 0.2 ms to fill, which
is what `vec![v; n]` compiles to: a `memset` for a byte list and a store loop
for a word list. `grasm_list_fill_scalar` does it in one call. Only a *literal*
body qualifies — a call or an index runs once per element and can see how many
times that was — and the bounds are type-checked before anything is emitted, so
falling back to the loop cannot compute `(f()..g())` twice. Worth **1.42x on
`sieve`**, 10.48 ms to 7.40 against Rust's 5.8. Done, 2026-09-20.

**A small function is spliced into its caller.** Cranelift has no inliner, so
every call a compiled program makes is a real call: arguments into registers,
a stack-guard check if the function can reach itself, a return. Bounding it
first by expanding one level of `fib`'s recursion by hand in the source said
1.24x; doing it in the compiler, where the splice is an ordinary block and the
backend's own constant folding sees through it, is worth **1.66x on `fib`**
(11.50 ms to 6.91) and nothing measurable on the benchmarks that do not make
many calls.

The body is spliced as a block: two `let`s per argument — one to a hidden name
in the caller's scope, one from there to the parameter's own name, so
`f(x, x)` with a parameter also called `x` cannot read the binding it just
made — then the callee's statements, with a trailing `return e` rewritten to
`e`. Everything after that is the backend compiling an ordinary block, which
is what makes this cheap to get right: scopes, releases and types are already
its job.

Deny by default. No generics, no effects, no `! E`, no `inout`, a declared
return type, a tail that is an expression, and a body of at most 48 AST nodes
containing no `return` other than that tail, no closure, no `with`, no
`parallel`, no `spawn`, no `region`, no `try` and no `fail` — every one of
those either leaves the caller or is shaped by the frame it was written in.
The call site is told the *declared* return type rather than the body's:
`fn overlap(..) -> Option[Span]` ending in `None` infers `Option[Any]`, and a
`match` on that reads the payload as `Any`.

Two levels of splicing is *slower* — `fib` 6.70 ms to 8.86 — because a body
with two calls to itself grows fourfold at the second level, and the register
pressure costs more than the calls it removes. One level.

The defect worth recording: a spliced body keeps its own names, and two of
this backend's optimisations are keyed by name rather than by binding — a list
filled in a loop is pinned under the name it is written as. `tnet_encode`'s
`var out`, spliced into a loop whose own `out` was pinned, wrote the callee's
bytes into the caller's list. It produced the right answer for a list and the
wrong one for a dictionary, which is why it took a differential run over 497
programs to see. Ordinary shadowing is fine; a body whose names are pinned or
are a loop's append target at the call site is called instead.

The other one: a body that *leaves* rather than produces a value — `fn boom()
-> i64 { panic("mid") }` — puts the trap in the caller's own block when it is
spliced, where a call to it would have returned to a live one. The rest of
`print("{boom()}")` was then written into a block nothing reaches and nothing
terminated, and Cranelift refused the function. A block opened inside an
expression that control left now gets its terminator at the end of the
function, which costs a reachable program nothing: a filled block already has
one. Done, 2026-09-20.

**A constant power-of-two divisor is shifts, not a divide.** `acc = (acc *
1103515245 + 12345) % 2147483648` — the standard LCG, and the inner loop of
both `parmap` and `sort` — compiled to `sdiv` plus `msub`, with a
divide-by-zero branch testing a constant that cannot be zero, on the
dependency chain of a loop that runs 160 million times. Bounded first by
masking the same arithmetic by hand in the source, which is exact for a
non-negative accumulator: `parmap` **911 ms of processor time to 269**, a
ceiling of 3.4x. The compiler cannot assume non-negative, so it emits the
truncating sequence — bias by `2^k - 1` when the dividend is negative, then
shift or mask — which is four instructions more than the hand-written mask and
still no divide. Worth **1.70x on `parmap`** and 1.06x on `sort`. Done,
2026-09-20.

Restricted to powers of two, and the restriction is measured rather than
cautious. Dropping *just* the zero check for any constant divisor — a branch
on a value that cannot be zero, two instructions gone — made `tree` 1.05x
**slower**, twice over. Removing a block changes what the register allocator
does with the ones around it, and a divide the core can hide behind is not
worth paying for that. So a divisor that is not a power of two keeps the
general path, guard and all.

**An interpolation appended to a string never becomes a string.** `acc = acc
+ "{i % 7}"` is how a program builds text here, and it built a one-character
string in its own box four million times in `parwords`. Decomposed by
substitution: the benchmark is 120 ms of processor time, 15 of which is
everything except that loop; replacing the interpolation with a literal saves
63 ms and dropping the append saves 23. **The box for the piece cost more than
the appending did.** `grasm_str_append_i64`, `_f64`, `_bool` and `_text` take
the left side over and write the piece's bytes straight into its buffer, so
`acc = acc + "{x}"` and `acc += "{x}"` allocate nothing at all when `acc` is
the only holder of its string. Worth **2.05x on `parwords`** — 112 ms of
processor time to 55, which is wall-clock parity with Rust. Done, 2026-09-20.

The aliasing is what makes this need care rather than only a new entry point:
the right-hand side can *be* the left-hand side. `s = s + s`, `s = s + "{s}"`
and `s = s + "{s}{s.len()}q"` all have to answer what a rebuild would, and
they do because a second reference makes the count two and the in-place path
needs exactly one. E-STR-CONCAT pins all three.

**A counted loop runs four iterations at a time.** Cranelift does not unroll,
so `for j in 0..n` pays its compare, increment and branch once per element —
four instructions of bookkeeping behind thirteen of work in `matmul`'s inner
loop. The loop now runs four copies of its body while four iterations are
left, then the ordinary loop for the remainder. Worth **1.21x on `sieve`**,
1.04x on `sort`, 1.03x on `matmul` and nothing elsewhere.

Two details decided by measurement rather than by argument:

*The copies use `base + k`, not one more than the copy before,* so the four
addresses do not depend on each other. Writing each index back into the loop
variable is what keeps `continue` right: it jumps to the remainder loop's
step, which increments from wherever the body left off, so the same iterations
run in the same order and simply stop being unrolled. `break` needs nothing;
it goes to the exit it always did.

*`matmul` did not reproduce its probe, and that is not explained.* Unrolled by
four in the source it measured 1.37x; unrolled by the compiler it measures
1.03x, with chained increments and with independent ones alike. The compiled
version differs from the hand-written one in emitting a remainder loop as
well, which doubles the code in a three-deep nest, and in keeping the
loop-invariant `end >= i64::MIN + 4` test as a materialised boolean the outer
loop re-branches on. Either could account for it; neither has been measured
apart. The win recorded here is `sieve`'s.

*Bodies that contain a call are not unrolled.* `tree`'s inner loop is
`kids.push(build(...))` three times over: the call dwarfs the loop's own
bookkeeping, so there is nothing to win, and four copies of the body cost
1.05x in instruction cache. With the rule in place `tree` is 1.02x faster
instead. Done, 2026-09-20.

Deciding whether four more iterations fit is written `i <= end - 4`, guarded
by `end >= i64::MIN + 4`, rather than `i + 4 <= end`: the second form wraps
when `i` is within four of `i64::MAX` and would unroll four iterations where
two remain. E-LOOP-RANGE walks every trip count from zero to twice the group,
both range forms, both ends of the number line, and `break` and `continue` at
every position within a group.

**A one-byte append does not go through `memcpy`.** `String::push_str` copies
through `memcpy` however short the piece is, and appending one digit at a time
— which is what building text in a loop does — spends more in the call than in
the byte. A piece of four bytes or fewer is written directly, and `i64_digits`
answers a single digit without its loop or its two divisions. Worth **1.037x**
on the string-building workload, reproduced on two independent runs.

Measuring it is the part worth recording. On `parwords` itself the same
comparison gave 1.12x faster, 1.05x faster, 1.00x, 1.02x faster and 1.11x
*slower* — the same two binaries, interleaved, best of nine to eleven
processor-time runs each. A `par*` row contends for cores with whatever else
is on the machine, and contention inflates total processor time, so the metric
that survives load on the single-threaded rows does not survive it here. The
number above comes from a *serial proxy*: the same program with `par_map`
replaced by `map`. Nothing smaller than about 15% can be measured on a `par*`
row at all. Done, 2026-09-20.

Two things that looked like they should help and did not, measured the same
way: marking `push_str` and `str_append_text` `#[inline]` made the serial
proxy *slower*, and so did writing the short append through `set_len` and raw
pointer stores rather than a `push` loop.

**The release walk remembers only what died.** Taking a value apart pushed
every child onto a pending list and decided what to do with it when it came
back off. Two thirds of `tree`'s nodes hold nothing but an interned integer
and the shared empty list, and neither of those is ever freed — so the list
was mostly things that were going to be looked at and dropped. The reference
is released where the child is *read* instead, and only a child that reached
zero is remembered. Every value on the pending list is now known to be dead,
which also takes a branch out of the loop. Worth **1.12x on `tree`**
(30.4 ms of processor time to 27.2) and nothing measurable elsewhere, which
is what it should be: no other benchmark tears down a graph. Done,
2026-09-20.

**The stack guard reads the stack pointer instead of a scratch slot's
address.** A function that can reach itself compares its current stack
position against a floor ([E-RECURSION-DEPTH]). It got that position by
allocating a one-word stack slot and taking its address — and a slot costs a
`sub sp` in the prologue and an `add sp` in the epilogue of *every* frame that
has one. Cranelift's `get_stack_pointer` answers the same question with no
slot at all.

Two instructions of fifty, and worth **1.19x on `fib`** (5.61 ms of processor
time to 4.71). That is the opposite of everything else on this page, where
removing four instructions from a thirteen-instruction loop measured nothing:
these two sit in the prologue and epilogue, and every load and store in the
frame waits on the stack pointer they move. Done, 2026-09-21.

**Appending to a string it alone holds is written out, not reached through
three calls.** `acc += "{x}"` went through `grasm_str_append_i64` to
`i64_digits`, then `str_append_text`, then `RtStr::push_str` — four frames to
put one byte on the end of a buffer. `append_fast` makes the same four tests
the general path makes (the reference count, the payload, the storage, and
whether a character index was ever built) and then writes the bytes.

Worth **1.62x** on the string-building workload, 34.6 ms of processor time to
21.3, and 1.22x on `parwords` itself. Done, 2026-09-21.

It was bounded first, by an unsound probe that skipped the tests as well as
the calls: 1.59x. The sound version measured 1.63x — *the tests were never the
cost*. That is worth remembering the next time a fast path looks like it needs
to drop a check to be fast; this one dropped none.

The same helper serves `grasm_str_append_text` and `grasm_str_append`, so a
literal piece and a string piece take it too. Aliasing is ruled out the way
the general path already ruled it out: two references to one box cannot both
be the only one, so `s = s + s` never reaches a path that writes in place.

**Pushing onto a list it alone holds is written out too.** Every push called
`cow_list` to ask whether this was the only holder — a question whose answer
is two instructions, in a function large enough not to inline because of its
clone path. The same treatment as `append_fast`, applied to
`grasm_list_push` and `grasm_list_push_scalar`. Worth 1.03x on `tree` and
nothing elsewhere, and the reason it is small is that the loops which push
hardest already hold their buffer open through `grasm_list_append_begin`;
only the paths that cannot be pinned — a recursive `build` — were paying.
Done, 2026-09-21.

Asked rustc to inline the three small helpers the profile shows as frames of
their own — `inc`, `interned_int`, `interned_at` — and `tree` measured 1.02x,
slightly *worse* than the push change alone. That is the second time an
`#[inline]` request has measured negative where writing the path out measured
positive. The difference is not the inlining: writing it out lets the specific
case skip work the general function still has to be able to do.

**A string a loop only appends to has its buffer held open.** `acc += "{x}"`
was a call into the runtime per character, and after the fast-path work above
that call was a single flat frame at 42% of the string-building workload. The
same bargain lists have had since `grasm_list_append_begin`: the buffer, its
length and its capacity are taken once per loop and held in registers, so an
append is a capacity compare and a store — one store for a one-digit integer,
constant stores for a short literal — with a leaf call only for a longer
number or a longer piece of text.

Bounded first, by building a runtime whose append does nothing at all:
**1.81x**. Built, it is **1.41x** — 21.8 ms of processor time to 15.5 — and
1.45x on `parwords`. Done, 2026-09-21.

The rule that makes it safe is the one lists already follow: while the buffer
is open the string's own length is stale, so only a name the loop does
nothing to but append to itself is ever pinned, and a body that can `break`,
`return` or `fail` is not pinned at all.

That rule has a second half which the first version missed. A pinned string's
*every* append has to go through the buffer: one that fell back to the older
path would write into the value while the length said otherwise, and the
length written back at the end of the loop would discard it. `acc += "{i > 1}-"`
has a `bool` piece, which the inline writer does not take — and that loop
printed nothing where the interpreter printed `false-false-true-`. So the
decision belongs at the open site rather than per statement:
`str_append_rhs` collects the right-hand side of every self-append to a
candidate, and the string is pinned only if all of them are shapes the buffer
takes.

**A power-of-two remainder is two masks on separate branches, not a bias in a
row.** `parmap`'s kernel is 160 million iterations of an LCG — `acc = (acc * A
+ C) % 2^31` — which is a dependency chain, so what decides it is *latency*,
not instruction count. Counted against Rust's disassembly: ours was `madd` →
`sshr` → `ushr` → `iadd` → `band` → `isub`, eight cycles; LLVM's is `madd` →
`negs` → `and` → `csneg`, six. Measured 2.50 ns and 1.89 ns an iteration at
3.2 GHz — 8.0 and 6.0 cycles, the one time all session a cycle count and a
measurement agreed exactly, and they agree because a serial chain is
predictable where issue-bound throughput is not.

The shape is the difference: biasing then masking is four operations in a row,
where `x >= 0 ? (x & m) : -((-x) & m)` puts the two masks on separate
branches. Emitting that form takes the chain from eight cycles to **seven**,
worth **1.13x on `parmap`**, confirmed on the benchmark and on a serial proxy.
The missing seventh cycle is a `csneg` — Cranelift has no rule fusing
`select(c, ineg(a), b)`, so it emits `neg` and `csel` where LLVM emits one
instruction. Done, 2026-09-21.

`i64::MIN` needs no special case: negating it wraps back to itself, and its
low bits are zero for every power of two, so both halves answer zero — which
is what `i64::MIN % 2^k` is.

Nothing else moves, and that was checked rather than assumed. `parwords`
printed 1.29x *slower* in the same run — the `par*` noise — and its serial
proxy says 1.004x. `sieve` printed 1.13x faster and re-measures at 1.00x.
Neither program contains a power-of-two remainder.

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

### Ceilings measured, 2026-09-20

**Lending *every* boxed parameter: 1.01x, which is nothing.** The analysis that
decides which parameters are lent is deliberately narrow — index and field
reads only — and the obvious next step was to widen it to the zero-argument
reads (`xs.len()`, `s.trim()`) that `borrowed_read` already compiles without
touching a refcount. Bounded first by forcing *every* boxed non-`inout`
parameter to be lent, which is unsound but compiles and runs: `fib` 1.01x,
`sieve` 1.01x, `matmul` 1.01x slower, `tree` 1.01x, `sort` 1.01x, `wordfreq`
1.01x slower. The narrow analysis had already taken what there was to take.
One build instead of a day.

**Removing every bounds check: 1.11x on `sieve`, nothing anywhere else.**
Measured the same way, by compiling with the check stripped out of
`pinned_address`: `sieve` 10.41 ms to 9.34, `sort` 1.03x, and `matmul`,
`tree`, `fib` and `wordfreq` all 1.00x. Even a perfect bounds-check
elimination leaves `sieve` at 9.34 against Rust's 5.8, so it is not what the
gap is made of. This is the third measurement to say the same thing about
checks an out-of-order core can predict.

What the stripped build *did* show is where the rest goes. Without the check
the inner loop is eight instructions where the ideal is four, and the four
extra are all Cranelift's: two rematerialising the loop bound
(`mov`/`movk` of the same constant, every iteration), one materialising zero
into a register instead of using `wzr`, and one unconditional branch because
the body is laid out after the exit. With the check *in*, regalloc adds five
more register-to-register moves shuttling values across block boundaries.
That is the shape of the remaining gap on the loop benchmarks: not the
runtime, and not the checks.

### More ceilings, same day

**Marking the panic blocks cold: 1.00x.** `set_cold_block` on the
out-of-bounds, divide-by-zero and stack-guard blocks does what it says — the
sieve's inner loop went from thirteen instructions to ten, three of the five
register-to-register moves gone — and changed no benchmark by more than noise,
and no binary's size by a single byte. Reverted. Three instructions in ten,
removed, measuring nothing is the clearest statement yet of what an
out-of-order core does with work that never mispredicts.

**Lending the loop element: 1.03x on `tree`, nothing elsewhere.** `for k in
n.kids` reads each element with a `+1` and releases it at the end of the
iteration, which is the same reference-count pair `param_is_borrowed` removes
one level up — and the list holds every element alive for the length of the
loop, so the same argument applies. Built as far as a forced-on probe
(`grasm_list_get_ref`, the element bound as a borrowed var) and measured
before the analysis that would make it safe: `tree` 1.03x, everything else
1.00x. Reverted. It agrees with the older finding that making the counts
non-atomic is worth nothing: the reference counting is not where the time is.

**Where `tree`'s time actually is, profiled.** `sample` on 400 rounds, with
symbols kept (`GRASM_KEEP_SYMBOLS=1`): `dec_walk` 25%, the rest of the release
path (`drop_glue`, `grasm_val_dec`) 11%, reading fields and elements
(`grasm_list_get`, `grasm_struct_get_at`, `grasm_list_len`) 13%, allocating
(`alloc_in`, `grasm_struct_make`, `grasm_list_new`, push) 15%, `_tlv_get_addr`
5%, `cow_list` 5%, and the compiled program's own `build` and `total` 15%.
Freeing and allocating are 40% of the benchmark, and they are 40% because of
how many boxes there are: 39,365 a round against Rust's 9,841 `Vec`s, because
every `Node` is its own box where Rust stores it inline in its parent's
vector. Removing the struct box — inline aggregate storage in a list — is the
only lever on that 40%, it is worth about 1.4x by this decomposition, and it
would still leave `tree` more than twice Rust's time.

**What an inliner would be worth: 1.24x per level on `fib`.** Cranelift has no
inliner and neither do we, so `fib(30)`'s 1.6 million calls each pay argument
setup, a stack-guard check and a return. Bounded without writing one, by
expanding one level of the recursion by hand in the source: 11.15 ms to 8.97
on the same loaded machine, measured alternately. That is the largest number
any ceiling probe has produced this session apart from the fill, it compounds
with a second level, and unlike the others it applies to every program with
small functions rather than to one benchmark. It is also the only remaining
item on this list that is squarely ours rather than Cranelift's.

### `matmul`, taken apart — 2026-09-20

Four probes, and the one that measured was not the one that looked obvious.

**Vectorising is worth 1.21x, not 2x.** Rust's inner loop uses `umull.2d`; the
easy assumption is that SIMD is the whole gap. Measured by building the *Rust*
program with LLVM's vectorisers off (`-C llvm-args=--vectorize-loops=false
--vectorize-slp=false`): 4.83 ms against 3.99 vectorised. So an auto-vectoriser
— by a long way the most work of anything on this list — would close a fifth of
a gap that is 1.9x wide. Not built.

**Index strength reduction: 1.00x.** `c[i * n + j] += aik * b[k * n + j]`
recomputes two `madd`s per element where an induction variable would increment
by one. Written that way by hand in the source: 8.78 ms to 8.90, *slower*.
The increments cost what the multiplies did, and two more loop-carried
variables cost a little more than that.

**Bounds checks: 1.01x**, measured the same day as `sieve`'s 1.11x by
compiling with the check stripped out.

**Unrolling is worth 1.37x in the source**, and it was the only one that
measured — though the compiler doing it reached only 1.03x on this benchmark;
see below. Unrolled by four in the source: 8.75 ms to 6.37. Cranelift does not unroll and neither
do we; a loop that is four instructions of bookkeeping and thirteen of work
pays a quarter of itself for the bookkeeping, and unrolling also puts more
memory operations in flight, which is what this loop is actually short of —
removing one indexed *load* from the body saved 25% where removing four ALU
instructions saved nothing.

### `matmul` is not a codegen gap at all — 2026-09-21

Everything above probes the *code* `matmul` runs. None of it probed the data.
A Grasm list is `Vec<*mut RtVal>`: eight bytes of pointer per element, each
pointing at a separately allocated 64-byte value. Rust's is `Vec<i64>`.

So give Rust our layout and see what rustc can do with it. Same program, same
algorithm, elements changed to a 64-byte `#[repr(C)]` struct holding an atomic
refcount and the value, one allocation each, reached through a pointer in a
`Vec`. Kernel only, timed inside the process with `Instant`, best of five, and
both print `47998000`:

| `matmul` kernel | |
| --- | --- |
| Rust, `Vec<i64>` | 2.31 ms |
| Rust, 64-byte boxed elements | 6.61 ms |
| Grasm (9.29 ms whole program less a 2.54 ms setup-only build) | **6.75 ms** |

**Grasm's kernel is within 2% of what rustc produces from the same layout.**
Not close to it — level with it, with rustc free to use every optimisation it
has, including the unrolling and vectorisation we lack. The 1.87x on this row
is the representation, all of it, and no amount of backend work reaches it:
the 1.21x vectoriser and the 1.37x unroller above would move a number that is
already at parity with the best another compiler manages on these bytes.

**Withdrawn the same day.** The premise was wrong: `matmul`'s lists are not
boxed. A one-shot probe in `grasm_list_get_scalar` never fired, and a profile
of a 600-iteration build put every sample inside a 288-byte range of the
program's own text with no runtime call in it at all. Disassembled, the inner
loop is `ldr x6, [x24, x0, lsl #3]` against a packed `ListScalar`, already
unrolled by four. Rust with boxed elements landing within 2% of it is a
coincidence of two different costs, not a match.

What the disassembly does show is the per-element instruction count:

    madd x0, x14, x4, x15     index i*n+j
    cmp  x0, x27 / b.hs       bounds check
    ldr  x6, [x24, x0, lsl #3]
    madd x0, x28, x5, x15     index again, other array
    cmp  x0, x1  / b.hs       bounds check
    ldr  x7, [x3, x0, lsl #3]
    madd x6, x2, x6, x7
    str  x6, [x3, x0, lsl #3]
    mov  x5, #0xc8            n, rematerialised per element

Eleven instructions where Rust's scalar loop is four or five, because LLVM
hoists the check and strength-reduces the index to a pointer walk. The working
set is three 320KB arrays, so this is cache-resident and issue-bound, and
instruction count is the thing.

That sat badly against the 1.01x measured for bounds checks and 1.00x for
strength reduction above, so both were re-measured.

**Bounds checks are worth 1.081x whole-program on `matmul`, not 1.01x.**
Measured by gating the check in `pinned_address` on an environment variable
read at codegen time, so the generated program pays nothing for the test:
9.88 ms to 9.14 ms, same answer, and 1.029x on `sieve`. On the kernel alone
(less the 2.54 ms setup) that is 7.34 ms to 6.60 ms, 1.11x. The old number was
wrong.

**Recomputing the index is free, and that part of the old number stands.**
Rust's scalar kernel with `n` behind a `black_box`, so the index must be
recomputed rather than walked: 2751 us against 2797 us for the ordinary scalar
build. Independent multiplies cost nothing on an out-of-order core.

**And instruction count is not the bottleneck.** Removing the checks takes the
body from thirteen instructions per element to nine -- 1.44x fewer -- and buys
1.11x. Against Rust's scalar kernel (2797 us, seven instructions per element
including its loop overhead, unrolled not at all) Grasm's checkless kernel is
still 2.36x slower at 6.60 ms. Neither the checks, nor the index arithmetic,
nor the raw instruction count accounts for that, so what does is still open.
Grasm's IPC across the loop works out near 3.1 against Rust's 5.7, which is
where to look next -- the body carries two conditional branches per element and
splits into a trap block for each, and branch throughput is a candidate the
instruction count does not capture.

`sieve` shares the layout and does not share the problem: its elements are
booleans, which are interned, so it pays eight bytes of pointer but never
allocates, and it already runs at 1.08x.

### The unroller is a trade, measured four ways — 2026-09-21

`matmul` is unrolled by four and Rust's scalar loop is not unrolled at all and
is faster, so the unroller was gated behind an environment variable read at
codegen time and every row measured both ways. Same answers throughout, best
of eleven:

| | unrolled | not | |
| --- | --- | --- | --- |
| `sieve` | 7.57 ms | 8.75 ms | unrolling helps 13.4% |
| `sort` | 10.33 ms | 10.69 ms | helps 3.3% |
| `matmul` | 9.83 ms | 9.75 ms | hurts 0.8% |
| `fib` | 5.72 ms | 5.55 ms | hurts 3.0% |

Net positive and worth keeping, but it is not free on every shape: the two rows
it costs are the two whose bodies are already dense with work, where four
copies buy no bookkeeping back and cost register pressure. Turning it off for
`matmul` recovers 0.8%, which is noise; for `fib`, 3.0%, which is not, though
`fib` is 1.03x of Rust and the whole row is nearly parity already.

No rule change made. Picking the cases apart on four benchmarks would be
fitting the rule to them.

### What one box actually costs, re-measured — 2026-09-21

The 24.8 ns for making and freeing one struct box is now **19.91 ns**, measured
the same way: 5,000,000 iterations of a two-field struct whose fields are
interned integers, against an identical loop without the struct, best of nine.
The number moved, so anything reasoned from the old one is worth re-checking.

Profiling that loop names where it goes, and it is not where the comment in
`alloc_in` says:

| leaf | samples | share of non-dyld |
| --- | --- | --- |
| `_tlv_get_addr` | 441 | ~28% |
| `_platform_memmove` + its stub | 295 | ~19% |
| everything in the program's own text | ~874 | rest |

**`_tlv_get_addr` is 28%, not the 15% on record.** Two thread-local lookups
per iteration -- one to allocate, one to release -- at roughly 2.75 ns each.
That is ~5.5 ns of the 19.91, and it is the largest single item in the cost of
every allocation this runtime makes.

**The memmove is a red herring, twice over.** `grasm_struct_make` builds an
`RtPayload` and passes it by value into `alloc` and again into `alloc_in`, a
56-byte enum moved twice before it lands in the box. Two fixes were tried:
writing the fields through `addr_of_mut!` instead of `ptr::write`ing a whole
`RtVal`, and `#[inline]` on both allocator functions. Neither moved the
microbenchmark (19.91 -> 19.67 -> 19.69 ns), the memmove leaf stayed at 210
samples, and on the real rows both together measured *worse*:

| | base | changed | |
| --- | --- | --- | --- |
| `tree` | 28.03 ms | 28.45 ms | 0.985x |
| `wordfreq` | 25.52 ms | 25.48 ms | 1.001x |
| `sort` | 10.39 ms | 10.42 ms | 0.997x |

Both reverted. A leaf high in a sample histogram is not the same as a leaf on
the critical path, which is the lesson worth keeping.

**The thread-local is not on the critical path either — built and reverted.**
The soundness question settled cleanly: there is no continuation capture and no
stack switching in this runtime, threads only ever run a closure to completion,
so a frame belongs to one thread and may hold its state pointer. So it was
built: `grasm_local_ptr`, a `grasm_struct_make_in` taking the pointer, and a
`local` on `FnCx` fetched in a block that dominates the body.

In the box loop that removes one of the two lookups per iteration, and should
have been worth ~14%. It was worth **1%** (19.91 ns to 19.72). Every real row
got worse, `fib` worst at 7.22 ms against 5.50, which is the per-call lookup
this crude version adds to functions that allocate nothing.

So a leaf worth 28% of samples is worth 1% of the clock. That is the same
mistake as the memmove above, made twice in one session on the same evidence:
**`sample`'s leaf histogram attributes cost to cheap leaf calls that the
out-of-order core is already hiding.** The way to price something here is to
remove it and time the result, never to read its share off a profile.

**Attributed by removal instead — 2026-09-22.** Three differentials, each one
building the program without some part of the work and timing the result.

*Refcounting and freeing are not it.* The same loop written inside a `region`,
where nothing is reference counted and the whole set is freed in one sweep at
the closing brace, costs **19.91 ns/box against 19.34** for the ordinary
reference-counted version. Tier 1 buys nothing here, which says the cost is in
making the box rather than in owning or releasing it.

*Allocation is about half.* With `grasm_struct_make` changed to build one
struct once and hand it back with a reference each time -- so the argument
array, the call, the field read and the release all still happen, and only the
allocation does not -- the loop falls from 19.41 ns to **9.44 ns**. So the
allocation itself, take-box through payload-write through give-back, is ~9.9 ns.

*A scalar field read is 1.98 ns.* Differencing `acc += n.v` against
`acc += n.v + n.w`: 19.41 ns against 21.39. That is one runtime call and its
body, ~7 cycles, which also prices what a runtime call costs here.

Inside the allocation, by the same method -- build without the part, time it:

| part of one box | ns | |
| --- | --- | --- |
| allocation | ~9.9 | of which drop glue 1.59, counters 0.67 |
| one scalar field read | 1.98 | |
| the rest -- argument array, two more calls, release, loop | ~7.5 | |

The drop glue is the one with something to do about it. `give_box_in` calls
`drop_in_place` on the whole `RtPayload`, and for much the commonest payload --
a struct whose parts are a `&'static StructTy` and four raw pointers in a fixed
array -- there is nothing to drop at all: the pointees are released by
`dec_walk`, not by this. Removing the call entirely is worth 1.59 ns (19.41 to
17.82); saying so with a match, which still costs the discriminant test,
recovers 0.48 of it. **`tree` 1.010x, `wordfreq` and `sort` unchanged.** A
`needs_drop` assertion holds the reasoning, so a payload that later owns
something fails the build rather than leaking.

The counters are 0.67 ns and stay: `allocated` exists for the leak report, and
1.7% is not worth changing what a gate prints.

**Interleave the A and B runs.** Measured back to back, this change read as
`tree` 1.036x and `wordfreq` 0.982x; alternating the two binaries and taking
25 of each put them at 1.010x and 1.001x. Sequential runs drift about 2% here,
which is larger than most of what is being measured.

So the target is the ~9.9 ns of allocation, and the reach of any fix is limited
by what else a box costs: even a free allocation leaves 9.44 ns standing. For
`tree`, which makes 1.33 boxes a node against Rust's 0.33, a cheaper box is not
the answer -- a fourth as many boxes is.

### Three more ceilings, 2026-09-21

**Unrolling `sieve`'s inner loop: 1.00x.** `while j <= limit { flags[j] = false;
j += i }` has an invariant step, so it can be unrolled by four with three
extra index additions and a remainder loop. Written that way in the source:
6.66 ms against 6.70. That loop scatters byte writes across two megabytes and
waits on memory, not on issue slots, which is why removing a quarter of its
bookkeeping changes nothing.

**`matmul` has nothing left to find by profiling.** 250 of 452 samples are in
the compiled function itself with no runtime call anywhere — the whole 1.8x is
instruction quality. That agrees with the four probes already on file and
means the row needs a vectoriser, which the Rust-with-vectorisers-off
measurement values at 1.21x.

**A `tree` with no boxes at all is *slower*.** The profile decomposition says
allocation and release are about a third of `tree` and that inline aggregate
storage — every `Node` stored in its parent's list rather than in a box of its
own — would take three quarters of that, worth roughly 1.4x. A flat rewrite
bounds it: the same program, same answer, nodes held in parallel scalar lists
with an explicit stack and not one per-node box, runs **1.35x slower** (36.1 ms
against 26.6). It is not a clean bound, because the explicit stack is work the
recursive version does not do and `push_at` is a call the recursion is not
making. But it is the only probe available, it does not support the
decomposition's 1.4x, and an older note in this file claiming a flat rewrite
reached 18.7 ms does not reproduce on today's compiler.

That is the reason the representation change has not been started: every
prediction made from a profile decomposition this session has been wrong by
between six and twenty times, and this one has no measurement behind it.

### `tree` has been probed five ways, 2026-09-21

One of the five landed. The rest are here so the next reader does not spend
the day rediscovering them.

| probe | measured |
|---|---:|
| the release walk remembers only what died | **1.12x — landed** |
| lending the loop element (`for k in n.kids`) | 1.03x |
| moving a local into a struct field instead of copying it | 1.009x |
| `% 1000` replaced by a power of two, so no `sdiv` at all | 0.995x |
| a flat rewrite with no per-node box at all | 0.74x |

The last three are the interesting ones. Removing a constant division from the
hot path of every node measured nothing, which agrees with the older finding
that a divide nothing waits on retires behind other work. Moving rather than
copying a field removes two calls and two atomic read-modify-writes per node
and measured nine parts in a thousand. And the representation change that the
profile decomposition valued at 1.4x is, in the only form that can be built
today, a third *slower*.

`tree` is 2.7x Rust and none of the obvious explanations survives measurement.

### Every remaining row now has a measured ceiling, 2026-09-21

Re-measured at the end, on the current compiler, because most of these
numbers were taken before the day's changes shrank the totals they were a
share of.

| row | vs rust | the lever | its ceiling |
|---|---:|---|---:|
| `parwords` | 1.06x | — | no cause identified |
| `fib` | 1.10x | the accumulator loop LLVM builds from `f(a) + f(b)` | 1.06x |
| `sieve` | 1.15x | removing every bounds check | 1.03x |
| `parmap` | 1.22x | — | not measurable below 15% on that row |
| `matmul` | 1.76x | auto-vectorisation | 1.21x |
| `tree` | 2.79x | inline aggregate storage | 0.74x — *slower* |

Bounds checks were worth 1.11x on `sieve` when that was measured; they are
worth 1.03x now, because the constant-map-to-fill change took four
milliseconds out of the total they were a share of. That is the general shape
of this table: each lever got smaller as the program around it got faster.

The accumulator transform is the only one left with a real mechanism and no
implementation. `fib(n-1) + fib(n-2)` has its second call in tail position
modulo the addition, so it can become a loop with an accumulator — which is
what Rust's disassembly shows LLVM doing, and why its frame count is half
ours. Written out by hand in Grasm source it measures 1.059x, which would
leave `fib` about 4% behind Rust rather than 10%. It is a real AST transform
for six per cent of one benchmark, and it is recorded rather than built.

### `wordfreq`'s `memmove`, chased twice and not caught — 2026-09-21

`_platform_memmove` is the largest single item in `wordfreq`'s profile at 38
samples — ahead of the map bump at 22 and four times the hash at 9 — and the
words it copies are five to seven bytes. Two ways of removing it, both
measured, both *slower*:

**A short-piece fast append in the builder, 0.987x.** `grasm_str_build_text`,
`_str`, `_i64`, `_bool` and `_f64` all append through `String::push_str`,
which calls `memcpy` at any length. Writing a byte loop for pieces of sixteen
bytes or fewer — the shape that was worth 1.62x in `append_fast` — measured
`wordfreq` 1.03x slower and `parwords` 1.04x slower. The difference from
`append_fast` is that there the win came from collapsing three frames; here
there is one frame either way, and a byte loop is not faster than `memcpy` at
five bytes.

**Raising `RtStr::push_str`'s threshold from four bytes to sixteen, 0.97x.**
The four was chosen when the case in hand was a single digit, which looked
like a number fitted to one example. It is not: on the serial proxy the
sixteen-byte version measured 0.981x and 0.967x on two runs. The `parwords`
row said 1.07x *faster* in the same session, which is what that row does —
below fifteen per cent it cannot be read.

So the `memmove` is somewhere else: the map key, or the copy into inline
storage when a builder finishes. Recorded rather than pursued, because the
profile has now pointed at this twice and been wrong both times about what
removing it would be worth.

### Two more hypotheses closed, 2026-09-21

**The reference count's memory ordering costs nothing: 1.02x *slower*.** An
older note here says non-atomic counts would be worth zero, and reasons it
from a microbenchmark: an atomic retain/release pair costs 10.1 ns against
0.33 for an ordinary one, "most of it the release/acquire ordering". That
reasoning is now checked by building it. Dropping the `Release` on the
decrement and the `Acquire` fence before the free — keeping atomicity, losing
only the ordering — measured `tree` 1.02x slower, `sort` 1.02x slower,
`sieve` 1.02x slower and `wordfreq` 1.03x faster. Noise in both directions,
and no win hiding behind the ordering.

**Process start is not where the small rows lose.** Every row carries the cost
of starting a process, and at five to eight milliseconds that is a third of
some of them. Measured with a program that prints one word: C 2.19 ms, Rust
2.34, **Grasm 2.21**, and `/usr/bin/true` 1.77. Grasm starts *faster* than
Rust and level with C, so there is nothing to win — and it means the
compute gaps are slightly wider than the raw ratios say, which is why the
floor-subtracted G2 (0.72x) is the more honest of the two figures.

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
| defeat Cranelift's constant rematerialization | timed the same loop with a literal bound against a register-held one | 0.998x | not built |
| drop the inclusive-range overflow guard | timed `0..=N` against `0..N+1` | the guarded form is *faster* | not built |
| reserve a counted loop's push capacity up front | added the hint, A/B'd it | 0.99x-1.01x | built and removed |
| tune Cranelift's own settings | swept alias analysis, opt_level, frame pointers, unwind info, regalloc | all within noise; `single_pass` regalloc is 3-4x *worse* | nothing to change |
| pin a loop's appends to a list of boxed values | timed a pinned scalar push against an unpinned boxed one | 3.08 ns against 2.01 — worth 0.6 ms of `tree`'s 27.6 | not built |

The reason is the same in every row: a modern out-of-order core hides
independent, perfectly-predicted work — *while there are spare issue slots*.
That qualification matters and took a while to find. In a six-instruction loop
the two that rebuild a constant are free, measured. In the twenty-instruction
loop that fills a list they are not, because the loop is issue-bound. Neither
"instructions are free" nor "instructions cost" is the rule; the rule is
whether the loop has slack, and only measurement says. A bounds check is a compare and a branch
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

**A loop over a list asked how long it was on every iteration.** `for x in xs`
lowered to an index loop whose header called `grasm_list_len`, so walking a
thousand elements made a thousand calls to be told the same number. The length
cannot change: the loop iterates a temp it owns, which user code has no name
for, and every list mutator goes through `cow_list`, which copies rather than
writes anything whose refcount is not exactly 1 — so `for x in xs { xs.push(y) }`
grows `xs` and leaves what the loop is walking alone, which is checked against
the interpreter.

Asked once, in the block before the loop: iterating a scalar list went from
3.29 ns an element to **2.34**, a list of strings 6.81 to 6.46, and a list of
structs with a field read 8.78 to 7.82. **tree 1.04x, parwords 1.06x.**

**What a walk still pays.** `for k in n.kids` in `tree` makes five runtime
calls per child — `grasm_list_get`, `grasm_val_inc`, the recursive call,
`grasm_val_dec`, and two more for the field reads that reach `n.value` and
`n.kids`. A struct field read is a call: measured at **1.95 ns** against 0.70 ns
for a plain local, because the field is a pointer to a box and the accessor has
to match on the payload. Inlining it would mean the generated code depending on
where Rust puts an enum's fields, which is not a promise Rust makes — so the
route to it is an explicit layout, not a peephole.

**Releasing a value asked the thread where its pool was before finding out
whether it needed one.** `dec` opened with `LOCAL.with(...)`, and only inside
that did it look at the refcount. But most releases free nothing: a borrow
taken for the length of a call, an element a loop bound and dropped, a value
somebody else still holds. Each of those is one atomic decrement and nothing
more — no pool, no work list, no thread state — and every one of them was
paying for a call through `_tlv_get_addr` first. A profile of a loop over a
list of strings put that call at **twenty-eight percent**.

The decrement happens first now, and `LOCAL` is reached only when the count
actually reaches zero. Iterating a list of strings went from 6.46 ns an element
to **4.61**, a list of structs with a field read from 7.82 to **5.87**, and on
the benchmarks **wordfreq 1.21x and tree 1.12x** — the largest single change of
the session.

It is worth noticing why this one was found late. The cost was never in a
function that looked expensive; it was in the *order* of two cheap things, and
it only became visible after profiling a four-line loop rather than a
benchmark. The benchmarks are averages over programs that do many things at
once; a microbenchmark of one operation says which one.

**`"n{x}"` allocated twice to produce three characters.** The string builder
was used only when two or more *pieces were interpolated*, on the reasoning
that one interpolated value already costs one allocation and the builder could
only add calls around it. That is right for `"{x}"`, which is a single piece.
It is wrong for `"n{x}"`, which is two: folding makes a value for the number
and then joins it to the text, and the text is a literal, and a literal is
immortal — so the append cannot write into it and has to allocate again. Two
allocations, 47 ns, for three characters. The test is now on the number of
pieces rather than the number of interpolations.

String interpolation **47.3 ns to 27.3**, a map lookup by an interpolated key
**69.1 to 50.0**. Nothing on the benchmark suite, because the one benchmark
that interpolates does it as a single piece.

**A table of what each operation costs**, measured by putting one of them in a
loop and subtracting the empty loop — which is how the release-path find above
was made, after five iterations of profiling whole benchmarks missed it:

| operation | ns |
| --- | --- |
| index a scalar list | 0.31 |
| read a list's length | 1.00 |
| read a struct field | 2.02 |
| read a string's length | 2.89 |
| index a list of strings | 6.30 |
| interpolate `"n{x}"` | 27.3 |
| concatenate two strings | 29.3 |
| make a two-field struct | 32.8 |
| make a two-element tuple | 47.7 |
| look a key up in a map | 50.0 |

The outliers are the thing to read it for. A tuple cost more than a struct with
the same contents, because a tuple literal was built as a list and then sealed —
`list_new`, a push per element, and `tuple_seal`, four runtime calls where a
struct takes one. Two and three go straight to the runtime now
(`grasm_tuple2`/`grasm_tuple3`), which is where tuple literals stop being
common: **67.6 ns to 53.4 for a pair, 1.27x**.

Not for a literal whose elements can *fail* part-way through, though. The
general path holds the half-built tuple in a scope so a `fail` in a later
element releases the earlier ones, and that is worth four calls; where no
element can fail there is nothing to release.

And a field read costs a call at all, because a field is a pointer to a box and
the accessor matches on the payload to unwrap it. `.1` on a *tuple* costs four —
`list_get`, an inc, an unbox and a dec — which is the next outlier in the table.

**Releasing built a stack frame before finding out it had nothing to do.**
With the zero case inline, `grasm_val_dec` opened with a forty-eight byte frame
and four saved registers — because reaching `LOCAL` needs the link register —
even though the common path decrements and returns. Moved into a `#[cold]`
`#[inline(never)]` function, the frame is two instructions and LLVM shrink-wraps
the rest onto the branch that actually frees.

**1.17x iterating a list of structs, 1.10x a list of strings, 1.04x a map
lookup; 2% slower making a struct**, which is the cost of one extra call on the
path that does free. Kept because releasing without freeing is much the more
common of the two. Nothing on the benchmark suite.

`#[cold]` earns its place: without it, LLVM still splits the function but lays
the branch out evenly — 1.11x on the iteration instead of 1.17x.

**A program costs 1.67 ms before it runs.** Process startup — dyld, the
runtime's first thread-local touches, the interned-integer block — is that much
on this machine, and it is included in every absolute number here. Splitting a
benchmark into phases by timing cut-down versions *must* subtract it, and the
first split of `sieve` did not: it read as build 5.39, scan 4.04, strike 3.95,
which put the cost in the wrong place. Corrected: **build 3.56, scan 2.42,
strike 5.74**. The strike loop is the expensive part, not building the flags.

## What `tree` is made of, measured

Split by timing cut-down programs, with the 1.67 ms of process startup
subtracted:

| phase | ms | share |
| --- | --- | --- |
| build the trees and free them | 27.55 | 70% |
| walk them | 11.83 | 30% |

And inside the build, by profiling: the generated `build` is 52%, `dec_walk`
34%, the drop glue 8%.

The unit cost that explains it: **making and freeing one struct box is 24.8 ns**
— measured on a loop that does nothing else, with the field value small enough
to be an interned integer, so exactly one box. Of that, `_tlv_get_addr` is 17%:
the allocation reaches the thread's state and so does the free. A pinned scalar
push is 2.01 ns by comparison, and the boxed push that `kids.push(child)` does
costs 3.08 — so pinning appends to boxed lists, which looked like the obvious
next move, is worth about 0.6 ms of 27.6.

`tree` makes 1.33 boxes per node — one for the node, one for each interior
node's list of children. Rust's equivalent makes 0.33 allocations per node,
because `Vec<Node>` stores the nodes *inside* it. At 24.8 ns a box that
difference is the whole gap, and nothing smaller than changing it will close
it: not the pushes, not the field reads, not the thread-local, each of which is
worth a few percent.

**Lending a parameter instead of giving it: measured at 7.8x, written, and
reverted.** Passing a collection to a function costs 8.52 ns where passing an
integer costs 1.26 — measured on a loop calling a function that does nothing
with it. The caller adds a reference because reading a boxed name yields one,
and the callee releases it on the way out; neither is needed when the callee
only *looks* at the value, because the caller's own binding holds it open for
the length of the call.

Built as an inferred borrow: a `let` parameter qualifies when every mention of
its name is the receiver of an index or a field read, the flag lives on the
signature so the declaration and every call site read the same answer, and the
callee's release paths skip it. It worked — passing a collection fell to
1.26 ns, the same as an integer, a call that also indexes it 9.66 to 2.81, and
`tree` 1.08x.

**It also broke four programs in the package corpus, and it is reverted.** The
differential gate caught them; forcing the analysis off restores 59 of 59. The
hole is not yet found: the compiler reports *no* borrowed parameters in the
failing programs, which cannot both be true, so either the analysis fires
somewhere the trace does not cover or one of the call-site edits is not as
conditional as it reads. The measurement stands and the design is sound; the
implementation is not, and a wrong answer here frees a value the caller is
still holding.

Two things worth keeping from the attempt. The first version released a borrow
it never took on the `return` path — `emit_scope_decs_all`, a third release
path beyond the two obvious ones — and **`leakcheck` passed all 497 programs
with that bug present**, because a premature free is not a leak. Only running
the program caught it. And the conformance case for [T-PARAM-LET] now exercises
every shape the analysis has to decide — returned, stored, relayed, read,
handed a temporary — which is what a second attempt should be developed
against.

## `tree`'s allocations, counted exactly — 2026-09-22

`GRASM_ALLOC_STATS=1` on the benchmark reports **787,300 allocations**. The
tree is depth 9, branch 3, built 20 times: 29,524 nodes each, 9,841 of them
interior, so 590,480 nodes and 196,820 interior nodes in the run. The count is
exactly their sum:

    787,300 = 590,480 nodes + 196,820 kids-lists

So it is one box per node, one box per interior node's list of children, and
**nothing at all for a leaf's empty `[]`** — the empty list is already free,
which is worth knowing because two thirds of the nodes are leaves.

Rust's `Vec<Node>` stores nodes inside the vector, so it allocates once per
interior node and never per node: **196,820**. Grasm makes exactly **four
times** as many, and at ~19 ns a box that is ~15 ms of a 27.5 ms run.

It is also a working-set difference. `RtVal` is 64 bytes (`RtPayload` 56,
`StructFields` and `ListData` 40 each, measured), so the live tree is ~50 MB
against Rust's ~19 MB of 32-byte nodes stored inline.

**Shrinking the box does not work.** Getting `RtVal` to 32 bytes would need
every payload variant under 24, and `ListScalar` alone is 32 (a `kind` plus a
`Vec`). Dropping inline struct capacity from four fields to two reaches 48, a
quarter off, while pushing every three- and four-field struct onto the heap.

**Reducing the count is not a language change.** `[T-PARAM-OWN]` moves a value
and `[E-VALUE-SEMANTICS]` says reading a place copies, so a node pushed into
its parent's list is owned by that list and nothing may observe its identity.
Storing it inline is therefore permitted by the semantics already; that `[Node]`
holds boxes is an implementation choice in `ListData`. That is the change worth
costing, and it is the only one that reaches this row.

**And it is worth 2.245x, measured.** The same benchmark in Rust, written two
ways -- `Vec<Node>` as it ships, and `Vec<Box<Node>>`, which allocates 787,300
times exactly as Grasm does -- interleaved, 15 runs each, both printing
294954900:

| `tree` | min | median |
| --- | --- | --- |
| Rust, `Vec<Node>` | 10.00 ms | 10.46 ms |
| Rust, `Vec<Box<Node>>` | 22.46 ms | 23.08 ms |
| Grasm | 27.5 ms | |

Unlike the boxed-`matmul` probe further up, this model is exact: the allocation
counts match to the unit. So of `tree`'s 2.87x, **2.245x is the representation**
and Grasm's own overhead over Rust doing the identical work is **1.22x** --
reference counting every one of those boxes and still within a quarter of a
`Box::new` per node. Inline storage would put this row near 1.2x, which is the
largest single move available anywhere in this document.

**The prerequisite is a borrowed list read.** Inline storage only pays if
`for k in n.kids` can hand the body a pointer into the list rather than copy an
element out of it; otherwise the allocations move from the build to the walk.
`grasm_struct_borrow_at` already does exactly this for a field -- "sound only
while the struct holding it is alive, which the caller establishes" -- and
there is no list equivalent: every element read is a `grasm_list_get` that
returns a +1 reference.

**Built as a probe and priced: 1.029x min, 1.050x median on `tree`**, neutral
on `wordfreq` and `sort`. A `grasm_list_borrow_at` that returns the element
without a reference, with the loop variable marked in `borrowed_vars` so the
scope does not release it -- the mechanism lent parameters already use. It
neither leaks (exit 0 under `GRASM_LEAK_CHECK`) nor changes the allocation
count (787,300 either way), so what it saves is purely the +1/-1 per element.
The loop is entitled to it: the loop walks a temp it owns and every mutator
goes through `cow_list`, so nothing can shorten the element's life mid-walk.

**But the sound analysis refuses the shape that gains.** `param_is_borrowed`
treats a bare mention as disqualifying -- "a bare mention could go anywhere" --
and `tree`'s loop body is `sum = sum + total(k)`, where `k` is a bare mention
passed as an argument. Applying the existing rule unchanged would borrow
nothing here. Realising the 1.05x needs one extension: a bare mention is
allowed when it is an argument to a parameter already inferred lent, which is
information the backend has in `sig_borrowed` but `param_is_borrowed`, a free
function over a name and a block, cannot see.

That extension is the shared prerequisite: it is what makes the borrowed loop
read pay, and it is what inline storage needs so that `for k in n.kids` hands
out a pointer rather than moving the allocation from the build to the walk.

**Built. `tree` 1.052x min, 1.055x median**, `wordfreq` and `sort` unchanged,
every answer the same. `grasm_list_borrow_at` lends an element without touching
its count, the loop variable goes into `borrowed_vars` so the scope releases
nothing, and `loop_elem_is_borrowed` decides when that is allowed. The rule is
`param_is_borrowed` plus the one shape a loop has and a parameter cannot: a
bare mention that is an argument to a parameter the callee only reads, which
`FnSig::borrowed` already records. Parameter inference keeps the stricter rule,
because it runs before any body is compiled and has no signatures to consult.

Two things were checked rather than assumed, because borrowing changes who owns
the element and a wrong answer here frees a live value:

- **The element cannot be written through.** `k.v = k.v + 100` in a loop body
  is refused by the checker -- a loop variable is immutable. That matters
  because a borrowed element has a count of one and would be mutated in place,
  where a taken one has two and `cow_struct` copies. The observable difference
  cannot arise because the program cannot be written.
- **The same question for lent parameters, which landed earlier.** Also
  refused: "a parameter cannot be `var`". No latent bug there either.

The list cannot be shortened underneath the walk: the loop iterates a temp it
owns, so every mutator sees a count above one and `cow_list` copies.

## `wordfreq` taken apart, and it is not the hash — 2026-09-22

Against Zig rather than Rust, `wordfreq` is the worst row here: 9.69 ms to
24.81, where C is 31.07 and Rust 26.39. The standing note elsewhere in this
document blames the map hash and says a faster one would be worth "several per
cent". Both halves of that are wrong. **This runtime already uses a wyhash-style
mixing step**, not SipHash -- see `mod maphash` -- so there is no trade left to
make there, and the map is not where the time goes anyway.

Split three ways, interleaved, min of 15:

| | min | |
| --- | --- | --- |
| whole program | 24.61 ms | 400,003 allocations |
| the string half alone | 17.84 ms | 400,001 allocations |
| the map half alone, keys built once | 12.98 ms | 780 allocations |

The map does 400,000 `get`s and `set`s for 780 allocations, which is close to
free. The string half is one allocation per iteration -- the concatenation and
the interpolation are already fused into a single string, so that work is done.

Per iteration, measured against a bare loop of the same shape:

| | ns/iter | allocations |
| --- | --- | --- |
| `ns[i % 8]` on a list of integers | ~0.0 | none |
| `words[i % 8]` on a list of strings, plus `len()` | 8.9 | none |
| `"{i % 97}"` | 16.4 | one |
| `words[i % 8] + "{i % 97}"` | 37.0 | one |

**A scalar list index is free and a boxed one is 8.9 ns.** The scalar list is
pinned -- the loop holds its buffer open and the index compiles to a load --
while the boxed list pays `grasm_list_get` with its +1, a call for the read,
and `grasm_val_dec` on the way out, none of which the scalar path has. Nothing
is allocated either way, so all of it is call and reference-count traffic.

That is the next thing to build, and the shape matters. The measurement above
used `let w = words[i % 8]`, but `wordfreq` writes
`words[i % 8] + "{i % 97}"`: the index is a **temporary inside an expression**,
not a `let` initializer. A borrow attached to a `let` would not fire on this
row at all -- something worth noticing before building it rather than after.

What fires here is an index in *operand* position whose consumer provably keeps
nothing. `grasm_str_concat` reads both operands and returns a new string, as do
`grasm_str_eq` and the comparisons, and each is followed by a `grasm_val_dec`
of an operand that was incremented only to be released a moment later. So the
rule is narrower than pinning and does not need the list to be only-indexed
across a whole block: evaluate `xs[i]` borrowed where `xs` is a plain local and
the operand is consumed on the spot.

**Built and reverted: it helps the benchmark and hurts its own microbenchmark.**
`borrowed_index_operand` in `emit_concat_into`, lending the element to
`grasm_str_build_str` and skipping the release. Matched builds, interleaved,
15 runs each, same answers and the same 400,001 allocations throughout:

| | before | after | |
| --- | --- | --- | --- |
| `wordfreq` | 24.94 ms | 24.39 ms | 1.023x |
| `wf_str`, the same concatenation with `len()` instead of a map | 17.72 ms | 19.68 ms | **0.901x** |
| `sort`, `tree` | | | unchanged |

The two disagree about the identical change to the identical expression, which
means something is going on that the reasoning above does not cover. The
likeliest candidate is a second bounds check: `index_value` normalises the
index and `grasm_list_borrow_at` checks it again, where `list_get_elem` checked
once. That is a guess, and 10% against a 2.3% gain is not a trade to make on a
guess, so it is reverted.

What the pair does establish is that the reference counting on this operand is
worth about 2% of `wordfreq` -- real, but small next to the 16.4 ns the
formatting and its allocation cost. The string half is dominated by making the
string, not by reading the word that goes into it.

**And the allocation inside that is 12.67 ns, measured by removing it.** With
`grasm_str_build_end` changed to fill the buffer, hand it back, and return one
shared immortal string instead of allocating -- so the whole build runs and
only the box does not -- `wf_str` falls from 17.51 ms to 12.44 ms, **1.407x**,
with allocations going from 400,001 to 2. On `wordfreq` that is ~5.1 ms of
24.9, about **20%**, and it is the largest measured item on the row this
project is furthest behind Zig on.

What would collect it: `wordfreq` writes `counts.set(w, (counts.get(w) ?? 0) + 1)`
over 776 distinct keys and 400,000 iterations, so **399,224 of those 400,000
key strings are allocated only to be found already present and dropped**. A map
that could be probed by the builder's bytes -- hashing from the buffer and
materialising a key only on insert -- would allocate 776 times instead of
400,000. The buffer already exists and is already kept for its capacity.

**Built. `wordfreq` 25.35 ms to 19.17 ms, 1.322x**, with allocations going from
400,003 to **3**, `sort` and `tree` unchanged and every answer the same.

The route there is worth recording, because two thirds of the work was aimed at
dead code. `m.set(k, v)` has three lowerings and the first two patched were
both unreachable for this program: the counting idiom is already fused into one
`grasm_map_add_i64` that hashes once, which is why neither `map_set` site ever
saw it. The only waste left in that fused call was the key *string*, so what
was actually needed was `grasm_map_add_i64_bytes` and a rule that keeps the
text where it was written.

The rule: a `let` whose value is a string build and whose **every** mention is
the key argument of `get` or `set` never becomes a string. The text goes into a
builder, the table is probed from its bytes, and the builder returns to the
thread when the block closes. Three conditions hold it up:

- Only for a block that runs to its end -- `body_leaves_early` -- because a
  `break` past the close would strand the builder.
- Reading *through* the name disqualifies it, where the loan analysis allows
  exactly that: a loan still has a value behind it and this has none, so
  `w.len()` has nothing to read.
- The scan denies by default, so an expression nobody thought about refuses
  rather than quietly qualifying.

Against Rust's 26.39 ms this row now runs 0.73x. Against Zig's 9.69 it is
1.98x, and what remains is the 16.4 ns of formatting and copying per
iteration, which no allocation trick reaches.

**How far it reaches, counted properly: 0 of 186.** The first count of this
was wrong, and wrong in the flattering direction for the measurement rather
than the change: it built corpus files standalone, and 47 of the 83 it looked
at are library modules with no `main`, so they never reached code generation at
all. Every one of the 186 demo packages *does* build, and pulls its libraries
in with it. The rule fires on none of them. It fires on `wordfreq` and on the
conformance test written for it, both files this work touched.

Nor is the narrower reading the problem. A key written straight into the
argument -- `db.by_port.get("{port}/{protocol}")`, which `netdb` really does
write -- needs no analysis at all, and building that form and counting it the
same way gives **1 of 186**.

So this earns its keep on the benchmark it was built for and on one demo. It
stays, because the idiom it removes is the counting loop -- which is what
`wordfreq` is in the set to represent -- and because 400,000 allocations down
to 3 is not a trick that stops working on other programs shaped like it. But
the corpus says plainly that the shape is rare here, and that belongs in the
record next to the 1.322x.

The corpus does contain the idiom, just not in a shape that reaches it:

    seen.set("{pixels[i]},{pixels[i + 1]},{pixels[i + 2]}", true)
    db.by_port.get("{port}/{protocol}")
    c.entries.get(context + "\u{4}" + key)

That form was built this session, extended to the plain `grasm_map_get` that
`match m.get(k)` goes through, and reverted: one demo in 186 is not enough to
measure a change by, and code that nothing measures should not land.

So the honest position on this row: 1.322x is real and gated, and its
generality is measured rather than assumed -- and measured small.

(The single-interpolation form `"{i % 97}"` does not go through
`grasm_str_build_end` -- under the same probe its allocation count *rose*, so
its own path is elsewhere and this measurement says nothing about it.)

Written the other way round -- a single interpolation, `"{words[i % 8]}{i % 97}"`
-- costs the same 37.1 ns against 37.2, so the concatenation and the
interpolation already lower to one thing and there is no source-level win
hiding here.

## A chained append was quadratic — 2026-09-22

Found by asking the corpus instead of the benchmarks. Every one of the 186 demo
packages was built and run, and the slowest by a wide margin was `zlib-demo` at
**5.17 s**, with `_platform_memmove` holding ~85% of its samples.

The cause is general and nothing to do with zlib. The self-append fusion
matched `s = s + a` and not `s = s + a + b`, because the second parses as
`((s + a) + b)` and the match looked for the name directly on the left. What it
did not match fell through to building a whole new string each time round:

| | n=50,000 | n=100,000 |
| --- | --- | --- |
| `out = out + a` | 2.5 ms | 3.1 ms |
| `out = out + a + b` | 244.9 ms | 711.8 ms |
| `out = out + a + b`, after | 3.0 ms | 4.1 ms |

Linear against quadratic — **174x at n=100,000** — and `bytes.encode_hex`
writes exactly that shape, which is why hex-encoding 200 KB to compare two
buffers dominated the whole program.

    zlib-demo   5171 ms -> 65.2 ms   79x

Nothing else moved: `scheme-demo`, `x509-demo`, `statistics-demo`,
`encodings-demo` and `certs-demo` are all within 1%.

**Appending in place changes the name as it goes**, which is a hazard the
single-piece form does not have. `s = s + "-" + s` compiled to `ab-ab-` for
`ab-ab`, because the second piece read the name after the first had already
written to it. A chain of more than one piece that mentions the target now
falls back to the ordinary path; one piece stays safe, since it is read before
the append. No gate caught that -- difftest passed the broken build, because
nothing in 497 programs writes it and no generator makes it -- so it is pinned
in [E-STR-CONCAT] along with the chain, a three-piece mix, and a chain whose
head is a different name.

Two hypotheses died before any of this was built: hoisting `zlib`'s per-call
lookup tables to module constants changed nothing at all, and an `inout` list
push turned out not to copy (2.9 ms against 2.7 for 200,000 pushes).

**The list has the same trap, and nothing in the corpus steps in it.**
`xs = xs + [i]` allocates twice an iteration — the one-element literal and a
fresh concatenation — so each step copies everything written so far:

| n | | of n=10,000 | linear would be |
| --- | --- | --- | --- |
| 10,000 | 25.6 ms | 1.0x | 1x |
| 20,000 | 95.9 ms | 3.7x | 2x |
| 40,000 | 195.9 ms | 7.6x | 4x |
| 80,000 | 628.5 ms | 24.5x | 8x |

Per element that is 5.1 us against 0.13 for `xs.push(i)`, which is linear
(2.6 ms at 20,000 against 2.9 at 40,000). `m.set(k, v)` is fine too.

It is **not** fixed, and the reason is the measurement rather than the
difficulty: `x = x + [...]` appears in **0 of the corpus's source files**,
which all use `push`. The string version earned its fix by being what
`bytes.encode_hex` actually writes. This one would be a change nothing here
exercises, and the session already has one of those. It is written down so that
a program which does hit it is not a mystery.

A runtime-only fix would not reach it either: the backend reads the left
operand with a reference and releases it after the call, so the concatenation
never sees a uniquely-held list to append into. It needs the same self-append
shape the string form uses.

### Three more scaling traps, and why the obvious fix is wrong

Scaling is one measurement that survives a busy machine: a 4x ratio for a
doubled input does not become a 2x one under load. Doubling 20,000 to 40,000:

| | 20,000 | 40,000 | |
| --- | --- | --- | --- |
| `while i < xs.len()` on a list | 2.5 ms | 3.0 ms | 1.2x — fine |
| `while i < s.len()` on a string | 19.8 ms | 67.4 ms | **3.4x** |
| `s.substr(i, 1)` per character | 53.8 ms | 196.7 ms | **3.7x** |
| `s = "x" + s` | 6.6 ms | 21.9 ms | **3.3x** |

A list's `len` is O(1); a string's is not. `RtStr::char_len` runs `is_ascii`
over the whole text on every call — vectorised, but still linear — so
`while i < s.len()` scans the string once per character. **432 of the corpus's
source files write `while i < x.len()`**, which is the opposite of the list
trap above, though the ones checked are over lists and bytes rather than text.

**A cached count as a new field would be the wrong fix, and the runtime says
so.** `RtStr` is the largest `RtPayload` variant, so its size is charged to
every boxed value — every integer, every bool. A cached length and an index
were removed for exactly that reason, and the comment explaining it is still
there: 36 of `RtStr`'s 64 bytes were an index most strings never build.

**Fixed without paying that, by using the `OnceLock` the index already
occupies.** It is empty for ASCII text, which never needs offsets, so the count
goes there and costs no bytes at all. `byte_at` and `char_at` both return
before reading `offsets`, so an index carrying a count and an empty vector is
never misread. Below 256 bytes nothing changes: the scan is what the original
comment measured as cheaper than reaching the cache, and short text keeps it.

| `while i < s.len()` | before | after |
| --- | --- | --- |
| 20,000 | 19.8 ms | 2.4 ms |
| 40,000 | 67.4 ms | 2.9 ms |
| scaling | 3.4x, quadratic | **1.2x, linear** |

**23x at 40,000**, and matched builds show nothing paying for it: short-string
`len` over two million calls, a non-ASCII loop, `wordfreq`, `sort` and `tree`
are all inside noise with identical answers.

The hazard is a count outliving the text it described, and the index already
had it — three mutation sites reset the `OnceLock`, so the count inherits that
discipline. [E-STR-CHARS] pins the case that would catch a miss: text that is
ASCII until an `\u{03b1}` is appended to it, where a remembered one-byte-per-
character count would be wrong from that moment on.

**`s.substr(i, 1)` had the same defect, one function over.** Calling it
inherent above was wrong, and inferred from what UTF-8 makes hard rather than
from the code. `byte_at` returns the index directly for flat text, so this
should be constant — but `is_flat` was `self.s.as_str().is_ascii()`, reading
the whole string on every call. The index answers it without a scan: offsets
for text that needs them, none for text that does not, so an empty table *is*
the flat case.

| `s.substr(i, 1)` per character | before | after |
| --- | --- | --- |
| 20,000 | 53.8 ms | 3.4 ms |
| 40,000 | 196.7 ms | 3.6 ms |
| scaling | 3.7x | **1.1x** |

**55x at 40,000.** `char_at` is on the same path and gains the same way.
Matched builds put `wordfreq`, `sort`, `tree` and short-string `len` inside
noise with identical answers.

`s = "x" + s` remains quadratic and that one really is inherent: putting a
character in front of text means moving the text.

### What the two string fixes are worth on real programs

The byte-keyed map above helped 0 of 186 demos, so these were held to the same
question. Every demo whose runtime is far enough above the process floor to
resolve — 108 of them — was timed with the old binary and the new one
**interleaved**, run for run:

| | before | after | |
| --- | --- | --- | --- |
| `sql-demo` | 10.1 ms | 4.2 ms | 2.41x |
| `legacytext-demo` | 85.9 ms | 41.3 ms | 2.08x |
| `fonts-demo` | 17.9 ms | 8.6 ms | 2.08x |
| `geo2-demo` | 5.6 ms | 4.2 ms | 1.34x |
| `content-demo`, `encodings-demo`, `japan-demo` | | | 1.27x |
| `sevenbit-demo` | 83.6 ms | 66.9 ms | 1.25x |

All with identical output. Text handling is where the corpus actually spends
itself, and this is the first change this session to show that breadth.

**Two measurement traps caught on the way, both mine.** Diffing two corpus
sweeps taken at different times under load produced eight "movers" of 2.2-3x;
interleaved, three of the five checked had not moved at all (`http-demo` 0.96x,
`config2-demo` 0.97x, `compression2-demo` 1.00x). Two sweeps are not a
comparison, however many programs they cover. And seven reps on a 3 ms program
reported `cache-demo` at 0.60x and `audio-demo` at 0.73x; at twenty-one reps
they are 0.97x and **1.00x**. When min and median disagree, the answer is that
there are not enough runs yet.

**Two more callers of the same scan.** `index_of` turned its byte result into
a character index by counting the prefix, and flat text is indexed by byte, so
that walk was unnecessary — 358 uses across 144 corpus files. `pad` counted
characters where `char_len` now remembers them. Neither is a complexity fix,
since `find` is linear either way; they are constants: 1.03-1.04x min and
1.07-1.12x median on `index_of` over long text, and **1.00x on short mixed
text**, so nothing pays for the extra question.

**The case that cannot pay for itself costs about 5%.** A fresh 5,000-character
string asked its length exactly once — so the remembered count is never read
again — runs 0.97x min, 0.95x median, 0.99x mean over thirty-one runs. That is
the price of the 23x and the 55x, and it is worth it.

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

**Storing struct values inline in a list.** `tree` allocates 787,300 times
where Rust allocates 196,820 — one box per node plus one per interior node's
list of children, against `Vec<Node>` holding its nodes inside it. The ceiling
is measured, not guessed: the same program in Rust with `Vec<Box<Node>>`
allocates identically and runs 22.46 ms against 10.00, so **2.245x of that row
is the representation** and the 1.22x on top is ours. Nothing smaller reaches
it; a cheaper box does not, because even a free allocation leaves 9.44 ns of
the 19.41 standing.

`[T-PARAM-OWN]` moves and `[E-VALUE-SEMANTICS]` copies on read, so a node
pushed into its parent's list is owned by it and its identity is unobservable
— the semantics already permit this, and only `ListData` stands in the way.
What it needs first is a borrowed element read, so `for k in n.kids` hands out
a pointer rather than moving the allocation from the build to the walk; that
part is built and landed.

**But storing values inline is worth only ~10% on its own — measured before
building it.** The easy design is available: keep each list element as a real
`RtVal` in the list's buffer with `ARENA` for a count, so every existing reader
still gets a valid `*mut RtVal`. The catch is how `tree` fills its lists —
`kids.push(build(...))`, where `build` returns a fresh value. Pushing would copy
that value into the buffer and free its box, so **every node is still
allocated once**. Modelled in Rust, interleaved, 31 runs, all printing the same
checksum:

| `tree` storage | min | |
| --- | --- | --- |
| `Vec<Node>`, each node built in place | 10.06 ms | 1.00x |
| each node built in a `Box`, then moved into a `Vec<Node>` | 19.77 ms | 1.97x |
| `Vec<Box<Node>>` | 21.88 ms | 2.18x |

The middle row is what the easy design buys: 21.88 to 19.77, a tenth of the
gap. The walk's pointer-chasing was never the cost; the allocation per node is.

**And neither representation change is the lever — withdrawn the same hour.**
A fourth model keeps Grasm's shape exactly — every node its own object, reached
through an indirection, the way a box is — but allocates by bumping an index in
one arena and releases the whole tree at once:

| `tree` storage | min | |
| --- | --- | --- |
| `Vec<Node>`, built in place | 9.92 ms | 1.00x |
| **separate objects behind an indirection, bump-allocated, freed wholesale** | **9.41 ms** | **0.95x** |
| boxed, then moved into `Vec<Node>` | 19.59 ms | 1.97x |
| `Vec<Box<Node>>`, a `malloc` and a `free` per node | 22.07 ms | 2.22x |

Separate objects and the pointer-chasing between them cost *nothing* once
allocation is a bump and release is wholesale — that row is faster than
inline. So `tree`'s entire 2.2x is **the price of allocating and releasing each
object**, and the paragraph that stood here recommending destination-passing
was aimed at the wrong thing: placing values inline was never needed.

Grasm's box is 19.4 ns made and freed, and the removal measurements above say
where it goes — about 9.9 on the allocation side (pool, a 64-byte write, two
counters, drop glue) and about 7.5 on calls: `grasm_struct_make`, the release
through `grasm_val_dec` and `dec_walk`, each an out-of-line trip. The bump
arena's equivalent is an add. That makes the lever general rather than
`tree`-shaped — every allocation-heavy program pays the same 19.4 ns — and it
is the entry below on inlining reference counts, together with an inline
allocation fast path, that reaches it.

**Regions are not the other route, measured.** `tree` with each iteration's
work written inside a `region { }` — a probe, not a change to the benchmark,
whose Rust counterpart uses no arena — runs **0.98x** of the ordinary program:
26.27 ms against 25.85. A region already skips every reference count and frees
its values in one sweep, and gains nothing. Its sweep did have waste: a `Vec`
allocated per value to gather children to release, which an interned integer
triggered despite being `IMMORTAL`, and a thread-local lookup per value.
Removing both measured **1.01x**, noise, and was reverted.

So neither refcounting nor freeing is where a box's 19.4 ns goes, and a bump
arena would inherit the same cost: every box is made by an out-of-line call to
`grasm_struct_make`, filled through a 64-byte write, pushed by another call,
and released by a third. Rust's arena row is fast because constructing a node
there is a store in the caller. What would close this row is emitting the
allocation and release fast paths into generated code, so that making a node
is not a call at all.

**Escape analysis**, the other half of the same problem: a `Node` that
provably does not escape its function could live on the stack with no box at
all. A compiler project rather than a patch — it needs an alias analysis the
front end does not have.

**Inlined reference counts.** The backend emits a call for every retain and
release, at more than a hundred sites. Inlining the atomic would remove a call each
time. It needs `#[repr(C)]` on `RtVal` so generated code may know where the
count is, which is fine, and it needs Cranelift's atomics, which are
sequentially consistent. The runtime's counts are `Relaxed` on the way up and
`Release`/`Acquire` on the way down, so an inlined version could easily be
*slower* on ARM and would be a threading defect if the ordering were wrong.
Measure before writing.

**A faster hash — already taken, and this entry was wrong.** It read "both
this runtime and the Rust comparison use SipHash". They do not: `mod maphash`
is a wyhash-style mixing step, seeded per process, and has been for some time.
There is no per-cent left there to trade hash-flooding resistance for, and
`wordfreq` turned out not to be map-bound anyway — 400,000 `get`s and `set`s
cost 780 allocations, while the *keys* cost 400,000 until they stopped.

**`matmul`'s unexplained 2.36x.** Against Rust's *scalar* kernel, after bounds
checks (1.081x) and with the vectoriser (1.21x) set aside, what remains is not
explained by instruction count, index arithmetic, branch density or unrolling
— each measured and ruled out. Naming it needs hardware counters, and this
machine has Command Line Tools without Instruments, so `xctrace` is absent.

**Two quadratic traps, left in on purpose.** `xs = xs + [i]` copies the whole
list each step, and appears in **0 corpus source files** — they all use `push`.
`s = "x" + s` is quadratic and inherent, since putting a character in front of
text means moving the text. Both are written up above; neither is worth a
change nothing exercises.

**The optimiser itself.** Cranelift does not unroll, does not vectorise, and
picks instructions less well than LLVM. Part of every ratio above is that, and
no amount of runtime work touches it.

## What will not be claimed

That Grasm matches Rust. `G2` is below its 0.95x target; the rows that are
ahead — text, sorting, recursion, and now `sieve` narrowly — are the ones doing
the least allocation, and `tree` and `matmul` are not close. The numbers
are in `benchmarks/README.md` with the method beside them; anybody can rerun
them on an idle machine.
