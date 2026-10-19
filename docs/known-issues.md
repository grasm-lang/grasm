# Known issues

Every defect that is known and not fixed, with its severity from
`docs/release-scope.md`, the clause it touches, and a reproducer small enough
to paste. S1 and S2 block a release; S3 and S4 do not, and this file is what
makes shipping them honest rather than quiet.

**Open S1: none. Open S2: none. Open S3: none. Open S4: none.**

**One gate was red, and it was right to be: the specification is over the
budget it binds itself to.** `unittests` failed on
`the_kernel_budgets_do_not_regress` until 2026-10-19 (below); the spec is
still twenty pages over its commitment. That is not one of the severities
above -- nothing is wrong with the implementation -- but
`docs/release-scope.md` asks for every gate green, so it stays here until the
spec is within budget or the commitment is re-stated.

    the specification is ~71 pages against a §19.2 budget of 50. It has grown
    past the point where the gap is worth recording rather than closing.

MASTERPLAN §19.2 binds Grasm-Core to a formal spec of 50 pages, and
`spec/core.md` explains why that one is binding rather than aspirational: the
claim that Core fits in one context window is the thing no competitor can copy
quickly, and it is worth nothing unless the split is real and checked. The test
allows up to 70 pages -- 3,550 lines at the 50-lines-a-page conversion
`spec/core.md` states -- and the tripwire is there to say when recording the
gap stops being enough.

**It crossed on 2026-10-12, at `03b8aad`**, from 3,549 lines to 3,556: the
per-channel wait queue rewrote [E-SCHEDULE] and added seven lines. Counted at
each commit in the range rather than guessed.

Three of those seven are gone now, because they were performance rationale in a
normative document and `docs/task-scaling.md` already makes the same
O(runnable) point. That leaves 3,553 lines, still 71 pages, **and the gate is
still red** -- which is the honest outcome. The remaining four lines are the
wake rules themselves, and they are normative: a send wakes the longest-waiting
sender, a receive the longest-waiting receiver, closing wakes every waiter.

**What it would take to close, and why it is not done here.** 71 to 50 is
twenty-one pages, and `spec/core.md`'s own §"What would have to move" already
names the seven forms that would take Core from 37 constructs to 30 and what
each costs. That is a design decision about what Grasm-Core *is*, not an
editing job, and it belongs to whoever owns that decision. What must not happen
is the alternative: `spec/core.md` says it in as many words --

> A number that is met by redefining what counts is not a number.

so the tripwire is not being raised to 71, and the metric is not being
re-scoped to exclude `spec/README.md` (59 lines, which would buy exactly the
page needed). Either the spec gets smaller or the commitment gets re-stated
deliberately.

**Why it sat unseen for three days.** The gate that reports it is `unittests`,
and from 2026-10-12 that suite did not fail -- it **hung**, on F101's panic
deadlock, which landed the same day. Two defects from one day, and the first
one hid the second.

**Back to 70 pages on 2026-10-19, still 20 over.** `spec/core.md` carried the
two paragraphs below -- how the page and interpreter figures moved, with
dates -- which are a record, not a rule, and moved here. That took the spec
from 3,561 lines to 3,546, which is 70 pages, the tripwire's ceiling, so
`unittests` is green again; the ceiling was not raised and nothing was
excluded from the count. The 50-page commitment is as far off as before, and
the next normative clause will turn the gate red again unless prose pays for
it, as `spec/core.md` says.

> The page figure said ~70 while the gate computed 71, which is the drift
> the table exists to make visible, caught on 2026-10-15. It crossed at
> `03b8aad` on 2026-10-12 when [E-SCHEDULE] was rewritten, and
> `crates/grasm_spec/tests/core.rs` failed from then: its ceiling is 70 pages,
> because past that the gap is meant to be closed rather than recorded.
>
> The interpreter grew 6,009 → 6,667 for readiness and datagrams
> ([L-LIB-NET], [L-LIB-UDP]), then → 9046 for the sender's address, the file
> calls a crash-safe store needs ([L-LIB-BYTES]). Each is movement *away*
> from the 5,000 target, recorded rather than absorbed quietly, which is what
> the test forces. Each was honest -- the reference engine implements every
> builtin -- and that is the point: **three times running, the library has
> been the thing that grew**, the last two demanded by a program that hit a
> wall. What would shrink it is moving builtins out of the engine into Grasm,
> which needs a standard library written in the language.

Closed on 2026-09-22: a second `var` of a name already bound in the same block
leaked what the first binding owned. `FnCx::bind` inserted into the innermost
scope map and dropped whatever entry was there, and `pop_scope_with_decs`
releases each name's *current* variable — so the shadowed one was simply lost.
It took an append to bite, which is why it went unseen: a bare string literal
is immortal and has no reference to lose, so `var t = "xy"` twice over is
clean while `var t = "xy"; t = t + "a"; var t = "zw"` exits 7 under
`GRASM_LEAK_CHECK=1`. `bind` now releases at the point of replacement, guarded
on the variable actually differing — `rebind` refines a type on the same one
and must not release — on the old type being boxed and not lent, and on the
block being live, since a call after a terminator is a verifier error.
[E-STR-CONCAT] carries the repro. Found while writing a conformance case that
reused a name the file already had, and present before that day's changes.


Closed on 2026-09-21: `fuzz_exits` disagreed on four programs in eighty-five,
and each was the same leak. A tuple literal of two or three elements is built
in one call, and the guard on that path asked `body_can_fail` — whether an
element could `fail`. An element can also leave by `return`, `break`,
`continue` or `try`, and when it did, the elements already built were
abandoned with nowhere to release them from. `elem_can_leave` asks the whole
question now, and T-TUPLE carries all four spellings; it exits 7 under
`GRASM_LEAK_CHECK=1` on the compiler before the fix.

Closed on 2026-09-21: `gitsigndiff` differed on eight cases in a hundred and
five, and four of them produced the *same* wrong hash — which is not what
eight independent errors look like. `\ No newline at end of file` is a note
about the line above it, not a line of the patch, and git's `patch-id` passes
over it. `packages/patchid` fell through to its state machine, where after a
hunk's lines are consumed the next line must begin `diff ` — so a marker there
ended the walk, and every diff touching a file without a trailing newline
hashed as its own first hunk. The failing commits shared a first file, hence
one hash for four inputs. The marker is skipped now, and the gate reports
105/105.

Closed on 2026-09-21: `grasm_sema`'s thousand-deep projection tests overflowed
the stack in a *debug* build and aborted the test binary. `analyze` recurses
once per level of nesting, and how deep a program may go is decided by the
checker's own "code is nested too deeply" limit — not by whichever stack the
caller happened to have. `grasm` establishes that by running its front end on
a 256 MB thread; a test calling `analyze` directly got cargo's thread instead.
The test helpers now take the same stack the compiler takes, and
`cargo test --workspace` passes in both profiles.

Closed on 2026-09-20, and the entry it replaces was wrong twice before it was
right. Two programs — `packages/aes-demo` and `apps/broker` — released a value
twice and segfaulted natively while running correctly under the interpreter.
Neither was a code-generation defect. Both were **objects compiled by an
earlier build of the compiler and reused by a later one**.

`build_native_incremental` caches one object per module in `.grasm-cache`
beside the output, and recompiles a module only when its key is missing. The
key was the module's name, its source, and the program's whole signature
surface — and nothing about the compiler doing the compiling. So a session
spent changing code generation and rebuilding left every cached object valid,
and each new binary linked yesterday's code.

Everything that made it look like a program bug follows from that:

* *The output path mattered.* The cache lives beside the output, so `-o /tmp/a`
  and `-o /tmp/sub/a` are different caches with different staleness.
* *Padding the source fixed it.* It changes the key, so the module recompiles.
* *The same function emitted two releases in one program and three in another.*
  One of those was read from a stale object.
* *`difftest` never saw it.* It builds into a fresh directory per run, so its
  cache is always cold — which is also why the gate was right and the bug was
  real at the same time.
* *Instrumenting the backend printed nothing.* Nothing was being compiled.

The fix is `compiler_fingerprint()` in the cache key: the version, and the
executable's size and modification time, which cost one `stat` and change for
a user who upgrades as surely as for a developer who rebuilds. With it, a
compiler rebuild invalidates the cache; without it, a released upgrade would
silently mix old objects into new binaries. With a cold cache both programs run
clean, `usecheck` reports no use-after-free, and the previously failing
difftest slice is 71 of 71.

**`leakcheck` is red: `E-VALUE-SEMANTICS.gr` holds 16 allocations at exit.**
Found on 2026-10-15 and present at `HEAD` before that day's work, so it is not
new that day -- but it was not written down, and the site's gate table said
"0 leaks", which is the only reason it was not. The ratchet is at zero by
design: 27 programs leaked the day the gate was written and none did
afterwards, so one leak fails it.

```
grasm build spec/conformance/E-VALUE-SEMANTICS.gr --native -o /tmp/evs
GRASM_LEAK_CHECK=1 /tmp/evs        # exit 7, 16 allocations
```

The program uses no `bytes`; it pushes into lists, structs and a nested place
(`bag.items.push(7)`), which is where `write_place` is already recorded above
as leaking intermediates. That is the first place to look, and the three
repairs tried against it are described in `scripts/leakcheck.sh` -- each was
wrong in a different way, which is why this is open rather than quick.

`scripts/usecheck.sh` and the `poison` feature stay. They are what turned an
intermittent segfault into a named, repeatable `use after free`, and the gate
`leakcheck` cannot replace.

F92 was open here and is fixed: a loop body is a block in statement position,
so its last expression is handed to nobody, and neither `while` nor `for`
released it.  See `ProjectGold/FINDINGS.md`.

Closed on 2026-08-17, all four found by writing a calculator in the language
rather than by a test: a closure in a `match` arm could not see the arm's own
pattern binding (S3 — a correct program had no spelling); the `--native` refusal
for a closure inside `Some(...)` in an arm stated a false reason and offered a
fix the source already contained; `&&` and `||` were reported as "unexpected
character `&`" with no mention of `and` and `or`; and a range's "no method" help
proposed `fn count(range, ...)`, a signature nobody can write.

Closed on 2026-08-16: the effect-row fallback pool (and the mechanism behind
it), writes through a `ref` under `--native`, returning a closure under
`--native`, and the UFCS help text.

This file is not empty because nothing is wrong — it is empty because what was
wrong is listed above as closed. The next round of evaluation is what refills
it.

## Representation limits

These are not defects. Nothing here is broken against a clause, and no gate is
red because of them -- they are things the language does not have yet, and the
reason a program can be correct here and still not be the right program to
write here. They are collected because the first person outside this repository
to port a real application hit every one of them in a week, and the list is
more useful than the sum of the clauses that imply it.

**One integer width, one float width.** `spec/types.md` [T-INT-ALIAS]: `i8`,
`i16`, `i32`, `u8`, `u16`, `u32`, `u64`, `usize` and `int` are aliases for
`i64`, and `f32` and `float` are aliases for `f64`. Distinct fixed-width types
are designed and unimplemented. There is no unsigned arithmetic, no wrapping
`u8`/`u16`/`u32`, and a `u32` hash wants masking by hand. Code that packs
binary32 writes the bit layout itself.

**No mutable byte buffer.** *Closed on 2026-10-15 by [L-LIB-BYTES-WRITE].*
`bytes_filled(n, byte)` makes a buffer and `set`, `fill` and `copy_from` write
into it, under both engines, with the same panics from each. Measured on 24
million bytes, each written and then read: **25 MB** of resident memory and
0.37 s, against **196 MB** and 0.79 s for the same loop over a `[i64]`. So the
eightfold memory difference this entry used to predict is gone, and the byte
buffer is also about twice as fast, because a byte of cache is a byte.

What a write costs, and why there is no `push`: a `bytes` may be a *window* on
a larger allocation, since `slice` does not copy, so every write first checks
that nothing else can see the buffer and copies it when something can
([E-COW]). The check is one atomic refcount test and two comparisons, which is
what the timing above includes. Growing a buffer would move the allocation out
from under every window on it, so the length is fixed and `concat` is how a
program grows one.

What remains is the integer width above, not the buffer: a `[f64]` of samples
is still eight bytes a sample, and packing binary32 still means writing the bit
layout by hand.

**No SIMD.** No intrinsics and no autovectoriser. `docs/performance.md`
records the one measurement taken against this: the flat struct-list sum is
scalar in both Grasm and Rust, so vectorisation is not what the geomean gap is
made of -- but it is also not available when a program wants it.

**No shared mutable state across threads.** `par_map` and `par_filter` require
an empty effect row and will not reach a `ref` from an enclosing scope, and a
task cannot be spawned per element of a list. A worker pool over a shared
tile cache -- `rayon` plus an `Arc<Tile>` -- has no direct expression. This is
the ordering guarantee and the replay guarantee being paid for, not an
oversight, but the cost is real and belongs here.

**No mutable globals, statics or thread-locals.** A handler cannot hold state
between invocations. A fresh-id counter, a lazily built transform cache and a
thread-local "current colour space" all have to be restructured -- the state
moves onto the value it belongs to and the context is threaded explicitly.
This is what makes a recording replayable, and it is also a rewrite of any
code that assumed otherwise.

**No pointer identity.** No address-keyed memo table and no weak map, so a
cache over a graph needs keys the program invents and maintains.

**No file handles.** Whole-file reads and writes plus ranged access --
`read_bytes_at`, `append_bytes`, `file_size`, `sync_file`, `rename_file`.
There is no reader object, no `seek`, no `mmap` and no streaming encoder.
Random-access container parsing works, by offset arithmetic rather than by a
cursor.

**No string builder.** `str` is immutable, so `s = s + x` in a loop is
quadratic; the external port measured roughly 2.4 s for 400,000 appends.
Collect into a list and `join`.

**A union error cannot be narrowed.** `(A|B)` can be raised, propagated and
printed, but not destructured back into its arms. The way to branch on which
failure occurred is to declare one sum type and match it.

**The C boundary takes four types.** `extern fn` exists and the C ABI import
is zero-cost, but `[T-EXTERN-POINTER]` allows a parameter to be `i64`, `f64`,
`str` or `bytes` and nothing else. No struct by value, no callback, no opaque
pointer handed back and forth. That is enough for `cos` and for a syscall and
not enough for most real C libraries, which hand you a `struct` or want a
function pointer. A review called this "hard isolation from the C ecosystem",
which overstates it -- the boundary exists and declaring it is how a package
admits to crossing it -- but the narrow type set is the real limit behind that
complaint.

**No widget toolkit. A window, as of 2026-10-15.** The windowing half of this
entry is closed: `packages/sdl` binds SDL2 and `packages/sdlevent` decodes its
events, with `scripts/sdltest.sh` creating a real window, renderer and
streaming texture under `SDL_VIDEODRIVER=dummy` and measuring **0.4 ms a
frame** for generate-upload-clear-copy-present at 256x256 RGBA. The entry
claimed there was "no FFI shaped to bind one", and that was the part worth
retracting -- handles as `i64` and out-params as `bytes` were enough, and no
builtin was added.

What is still missing is everything above the window: no buttons, sliders,
text fields, menus or layout, and no glyph rasteriser, so text means binding
SDL2_ttf or writing one over the font tables that already exist (`sfnt`,
`glyf`, `cmap`, `fontmetrics`). Nothing addresses the GPU in any package, and
the wasm build ships the interpreter without reach into the DOM. A library
that passes structures by value or wants a callback -- raylib, Cocoa through
`objc_msgSend` -- still cannot cross the boundary at all.

## Nothing open

The six defects that were open on 2026-08-17 are closed:

* an incremental `--native` rebuild shipped a **stale imported constant** — the
  cache keyed a module's object on its own source and on the program's
  *signature* surface, and a signature omits a constant's value. Now keyed on a
  fingerprint of everything the object read, which is the rule rustc's
  incremental engine uses.
* an **effect operation's result** was not row-checked, so a handler could hand
  a printing function through an empty-row result and a `capabilities = []`
  package reached `io`. `check_expr` returned early for a closure after
  checking the closure's own row.
* the **registry** was found only when the working directory was exactly the one
  holding `.grasm/` — the walk up was by `parent()` from a relative path, which
  has no parents.
* [L-LIB-SORT-BY] said a sort key "must be `i64`, `f64` or `str`" while tuple
  keys worked; [T-REF-WRITE] said the compiled backend refuses writes through a
  reference, a day after it stopped doing so.
* a **deeply nested tuple literal** cost O(n³) to check — 1000 levels took 7.8
  seconds, now 31 ms.

## S3 — the checker did not enforce [T-GENERIC-TYPE] — *closed*

`let r = Good(1)` for `type Res[T, E]` passed `grasm check`, ran under `grasm
run` and was refused by `--native`. The clause said the annotation was the
language's rule and one engine of three enforced it.

Closed on 2026-08-25 by teaching the checker, which is what the clause asked
for. The check waits for inference to finish, so a parameter a *later* line
fixes is still determined: `let a = Good(1)` followed by a call wanting
`Res[i64, str]` compiles everywhere. The backend reads the checker's answer
rather than inferring from the constructor's arguments alone, which is what it
had been doing and why it refused a program the checker had settled.

## S3 — over-strict, effect-row inference

### The fallback pool — *removed*

[T-FN-TYPE-ROW-CARRIED] (`experimental`). Closed on 2026-08-16, and then the
mechanism behind it was deleted. The type checker publishes the row of every
function-valued expression it types and of every call site whose callee is not
an expression of its own; the effect pass reads that. The syntactic
reconstruction — a map of name to sources, a pool of every function value a
body mentions, the rows a signature writes at nameless positions, and the
per-binder special cases that fed them — is gone, 511 lines of it.

There is no fallback pool to be over-strict any more, so the shape recorded
here (a `match` payload charged a sibling parameter's row) cannot recur.

### `--native` — *no gaps in the repository's programs*

Every program in `spec/conformance/`, `examples/`, `projects/` and
`benchmarks/` compiles natively: **223 identical across all three engines, 0
unsupported, 0 leaking.**

The §11 table listed seventeen shapes the backend refused and now lists two,
neither of which is a defect: a generic sum constructor that leaves a type
parameter undetermined, which is the language's rule ([T-GENERIC-TYPE]) rather
than one engine's, and polymorphic recursion, which is what monomorphization
is. `scripts/nativegaps.sh` proves the list matches the compiler in both
directions, over 55 cases, comparing output, exit code and leaks.

Closed on 2026-08-16: writes through a `ref` (including around a cycle) and
returning a closure. Closed on 2026-08-24 and 25: the prelude's `IoError`
variants, a range held as a value, `map` with a function returning nothing,
`==` on two bare `ref`s, a call through a non-name callee, a `fold` seeded with
an unannotated `map()`, a closure whose parameter types nothing local fixes, a
type parameter only in a function result, every way of using an effect handler
as a value, a UFCS call of a generic free function, generic `type` declarations
for both structs and sums, and `fail` inside a fallible closure passed to `map`. Closed on 2026-08-25:
interpolating a unit whose type was inferred rather than annotated (F45), and
`return` of an `if` whose every branch fails (F46) — the second a Cranelift
verifier failure, so a defect rather than a gap. Closed on 2026-08-26: a write
through a `ref` to a sum, which [T-REF-VALUE] always described and the
implementation excluded along with the structs (F48), and a handler operation
capturing a unit, which panicked inside Cranelift rather than refusing
anything (F50), and a closure declared inside another closure, which was
refused for every representation and had never been written down as a gap
(F51), and an empty `map()` as an element of a list literal, which was the
one position that asked the probe rather than the lowering (F52), and a range reaching a method from any shape that produces one rather
than only from a name -- which also gave the range type the spelling `range`,
so one can be a parameter, a result or a field (F53); and `fold`,
`unwrap_or`, `expect` and `parse_int_base` where the backend has to name a
call's result type rather than a receiver's (F54). And a write *inside* a referent, `r.value[0] = v`, refused
at two sites one level apart while reading the same place always compiled
(F56). Closed on 2026-08-27: a handler left installed when a `fail` or
`return` leaves its `with` block, which answered from the wrong handler and
then from a dead frame (F59), and the `return` itself, whose following
statement was emitted past the terminator (F60), and the same for `break`
and `continue`, which leave the block without leaving the function (F61); and the handler's
captures, which every one of those four exits kept (F62); and a `region`, a
plain block or an `unsafe` block that was left early, whose following
statement was emitted past the terminator (F63); and a `parallel` block's
task list, kept by every early exit, whose statement had the same fault
(F64); and a list, map, tuple or struct literal under construction,
abandoned when an element or field left the function (F65); and reading
`.value` through a captured `ref`, which [T-REF-CAPTURE] is the clause for
(F67).

## S4 — diagnostics

### `x.f()` does not reach a library free function — *closed*

Closed on 2026-08-16 by narrowing the clause rather than widening the
implementation. [L-UFCS] now says the language's own free functions are not
methods of any type, and `"hi".print()` is refused with help that names the
spelling that works — `"hi" |> print`, which was always there and is charged
the same row a call is. The old help offered UFCS for `print`, which is the
call it had just refused.

## `ProjectGold/30-async-chat` crashed under the native backend — *not the compiler*

Recorded by `scripts/allgates.sh`'s `gold` gate on 2026-10-07: the interpreter
ran it to completion (21 lines); the native build stopped after three and
exited with a varying signal -- SIGSEGV, SIGBUS, SIGTRAP, and once `panic:
expected a list`. A signal that changes between runs of the same binary is
memory corruption rather than a logic error. It failed three times out of
three, and the compiler built from the commit before that day's compile-speed
fix failed identically, so it was not a regression from that fix.

**It no longer reproduces.** On 2026-10-15 the program built native and ran
**eight times out of eight** at exit 0, byte-identical to `expected.txt`, and
the `gold` gate reports **44 agreeing, 0 failing, 0 blocked** -- every project
it has, interpreter against bundle against native. `ProjectGold/30-async-chat/`
has not been touched since 2026-08-18, so what changed is the compiler, not
the program.

**No commit fixed it, because no commit broke it.** The bisect was run on
2026-10-15 and stopped at its first step, which is the step that tests the
endpoint rather than assuming it: the compiler built from `411812f`, the last
commit before the failing gate run, builds this program and runs it **16
times out of 16** clean. Whatever happened on 2026-10-07 was not a property
of that compiler, so there is no fixing commit to name and the five audit
fixes of 2026-10-12 are not what closed it.

Two hypotheses survive, and the evidence separates them:

* **CPU load alone: ruled out.** HEAD runs it 16 of 16 clean at load average
  70-112 on eight cores, and the 2026-10-07 commit does the same.
* **Memory pressure, or something about running inside the full suite:
  untested.** The failure was seen while `allgates.sh` was running the whole
  corpus, in the week a commit message records the machine reaching load 146
  and another records seven fuzzer failures that were "the machine again".
  Under swap thrash macOS can deliver SIGBUS on a page-in and SIGSEGV where a
  stack cannot grow, which fits a signal that changes between runs better
  than a code-generation defect does. Nothing here reproduces that state on
  purpose, so this stays a hypothesis.

The one condition not retested is the original one: this program running
*inside* a full gate suite rather than alone.

Two things in the original entry were wrong, and both pointed the next reader
away from the defect:

* It said the program "combines sockets with `concurrent`". The word
  `concurrent` does not appear in `main.gr` and never has. The program is
  explicitly **single-threaded** -- "one thread, three clients" -- and
  multiplexes with `net_wait`, because `spawn` carries no `net` capability.
* It therefore named the wrong place to look. The socket layer under
  `net_wait` is the place; threads are not involved.

An intermittent memory corruption that disappears without a diagnosis is not
the same as one that was fixed. Until a bisect names the commit, the bug class
cannot be called gone -- only this reproducer can.

## `bytes.to_str()` is lossy without saying so

`bytes_of([65, 200, 201, 66]).to_str().to_bytes()` answers
`[65, 239, 191, 189, 239, 191, 189, 66]`: each byte that is not valid UTF-8
on its own becomes U+FFFD, the replacement character. The round trip is not
one.

What makes it a defect rather than a documented conversion is that nothing
says so at the time. The `str` reports `len()` of 4 -- the number of bytes
that went in -- so a caller measuring it sees no loss. The three-byte
replacements only appear on the way back out.

`read_file` refuses a file that is not valid UTF-8, which is the right
answer for something that must produce a `str`. `bytes.to_str()` accepts the
same bytes and quietly mangles them, so the two disagree about whether a
`str` is UTF-8.

Found through `gitsigndiff`: `packages/patchid` must hash the bytes git
hashed, and `git show` of a commit adding a file with no NUL bytes in its
first few thousand treats that file as text and emits its bytes as added
lines. Four diffs out of a hundred and five in that corpus contain such
bytes, and the patch-id came out wrong for exactly those four -- not because
`canonical` is wrong, but because the diff it was given was no longer the
diff git read. An ASCII diff of the same shape agrees to the character.

Two ways out, and it is a language decision rather than a local fix:

* `to_str()` refuses invalid UTF-8, matching `read_file`, and a caller
  wanting raw bytes keeps them as `bytes`;
* or the round trip is made exact, which means a `str` may hold bytes that
  are not text, and every operation on it has to say what it does with
  them.

Until that is settled, a Grasm program cannot compute a patch-id over a
diff that is not valid UTF-8, and `packages/pidtest` therefore uses
`read_file`, which refuses one, rather than `read_bytes().to_str()`, which
would read it and hash the wrong bytes. A refusal is the honest answer; the
alternative was a confident wrong id.

`scripts/patchid_corpus.py` keeps such diffs out of the corpus and counts
them, and `gitsign_diff.py` asserts the count against a limit, so the
limitation cannot grow quietly.
