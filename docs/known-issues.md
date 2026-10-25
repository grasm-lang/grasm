# Known issues

Every defect that is known and not fixed, with its severity from
`docs/release-scope.md`, the clause it touches, and a reproducer small enough
to paste. S1 and S2 block a release; S3 and S4 do not, and this file is what
makes shipping them honest rather than quiet.

**Open S1: none. Open S2: none. Open S3: none. Open S4: none.**

## Found on 2026-10-25 by the Grasm backend (S3, part four)

**`+=` on a string field or element, twice, was a use after free in the
native build -- *closed, S1*.** Found by `fuzz_s3`'s part-four generator, the
Cranelift build being its reference; reduced by hand:

```
type Leaf = | L(n: i64)

type Cell = { tag: str }

fn bump(c: ref Cell) {
    c.tag += "+"
}

fn main() {
    let c = ref(Cell{ tag: "xyz" })
    bump(c)
    bump(c)
    let more = [ref(L(8)), ref(L(9))]
    print("{c.tag} {more.len()}")
}
```

`grasm run` prints `xyz++ 2`; so does the S3 backend's build, with nothing
live at exit and nothing touched after its release under the poisoned
runtime. The native build exits with SIGSEGV (139) and prints nothing, or
with SIGBUS in a larger program. Each of these makes it go away: one `bump`
instead of two; `c.tag = c.tag + "+"` instead of `+=`; an `i64` field `+=`
instead of the string; references to integers instead of to a sum's
variants afterwards. So it is the compound string assignment to a place
behind a reference (the place path of `eval_stmt`'s `Stmt::Assign`, through
`read_place` and `write_place` in `crates/grasm_native/src/lib.rs`) that
leaves the heap inconsistent, and a later allocation that trips on it. No
corpus program has the shape: `difftest` and the S3 gates agree on all of
them.

Closed on 2026-10-25. The reference was not the cause: under the poisoned
runtime, `c.tag += "+"` twice on a `var` struct, `b.c.tag` on a nested field
and `xs[0]` on a list element failed the same way. `read_place` hands the
compound assignment the place's current value, and for a boxed type it
released the count the accessor had given it -- `unbox` passes a boxed value
through, so the box it released *was* the value it returned. `+` then
consumed that borrowed string and the write released it again as the old
field, so the second `+=` on a heap string released it twice; the first one
was over a literal, which no count frees. The value is now handed over with
its count. `a_compound_string_assignment_releases_the_old_string_once` in
`crates/grasm/tests/cli_tests.rs` builds the program above (and the other
shapes) natively and requires the interpreter's output with nothing live at
exit; with the release put back, its binary dies with SIGSEGV.

**Interpolating a `ref` into a string was an internal compiler error in the
native build -- *closed, S3*.** `grasm run` writes a reference as the slot it
names (`ref#0.0`), and so does `print` of one in the native build
(`grasm_out_ref`), but a reference interpolated into a string that is built
-- anything other than the direct argument of `print` -- reaches
`emit_pieces` in `crates/grasm_native/src/lib.rs`, whose match over the
piece's type has no arm for `VType::Ref` and ends in `unreachable!()`:

```
fn main() {
    let r = ref(5)
    let s = "at {r}"
    print(s)
}
```

`grasm run` prints `at ref#0.0`; `grasm build --native` stops with "the
backend panicked while generating code ... entered unreachable code at
crates/grasm_native/src/lib.rs:13596:30". No corpus program has the shape (a
program that has it does not build natively, so `difftest` would show it).
The S3 backend writes the piece as a boxed reference displays
(`grasm_box_ref`, then `grasm_str_from_val`) and prints `at ref#0.0`, as the
interpreter does; with no Cranelift build to compare against, no gate
compares that. The fix belongs in `emit_pieces`: a `VType::Ref` arm that
boxes the word with `grasm_box_ref` and writes it as the boxed arm does.

Closed on 2026-10-25 with that arm. `print(r)` of a bare reference -- the
argument itself, not an interpolation -- reached the same `unreachable!()`
in `lower_print` and now writes it with `grasm_out_ref`. The CLI test above
covers both.

**TIR does not record which system libraries a program links.** A program
whose package graph declares `links` (the three SDL programs: `sdl-demo`,
`pcview-demo`, `pcedit-demo`) can only be linked where the library is, and
`scripts/sdltest.sh` is the gate that builds them. The S3 gates ask the
compiler (`grasm links`) and set those programs aside, counted, as
`difftest` does; nothing in the TIR text says it, so a consumer of TIR alone
-- the S2 frontend's output included -- cannot tell. Not a defect in any
engine; written down so that S4's linker, which has to put the library on
the link line, knows to ask for it.

## Found on 2026-10-22 by the Grasm backend (S3, part three)

**`return` inside a closure passed to `map` returns from the enclosing
function in the native build -- *closed, S1*.** The Cranelift build compiles
`xs.map(|x| ..)`, `filter`, `fold`, `sort_by` and the searches by inlining
the closure's body into a loop in the calling function, and a `return` in
that body then leaves the *calling* function, with the closure's value as
that function's result. `grasm run`, the TIR interpreter and the S3 backend
(which lifts the closure into a function of its own) all return from the
closure. Measured on the shapes below with one `return` in the closure:
`map` and `fold` answer wrongly, `sort_by` exits with SIGSEGV (139), and
`filter` is refused ("codegen bug ... Verifier errors").

```
fn early(xs: [i64]) -> [str] {
    xs.map(|x| {
        if x > 2 {
            return "big"
        }
        "small {x}"
    })
}

fn main() {
    print(early([1, 2, 3, 4]))
}
```

`grasm run` prints `[small 1, small 2, big, big]`; the native binary prints
`big` -- a `str` returned from a function declared `-> [str]`, so the value
that comes back is not even of the declared type. No corpus program has the
shape. Twelve closures in three corpus programs (`numeric3-demo`,
`tui-demo`, `web-demo`) contain a `return`, but each is bound to a name or
returned rather than passed to one of those methods, so the Cranelift build
compiles it as a function of its own; all three programs are in the S3
subset and agree. Fixed: a closure literal whose body has a `return` of
its own is now bound to a local before the call, so the backend compiles it
as a function of its own and calls through it, as it already did for a
closure bound to a name. A CLI test runs all six shapes (`map`, `fold`,
`filter`, `sort_by`, `any`, `find`) in both engines; against the old backend
it fails, because `filter` does not build.

**An operator is written with the wrong operand type when an integer literal
on its left meets an `f64` -- TIR, no effect on any engine -- *closed*.** [E-NUM-PROMOTE]
makes the literal a double, and TIR writes it as a `float` node, but the
`binary` node keeps the literal's type as its operand type: `3 + f` is
`(binary f64 add i64 (float ..) (local f64 ..))`, and `5 == 5.0` is `eq
i64`. docs/tir.md §6.5 says the operator is written "with the operand type
that decides its meaning". Five nodes in two corpus programs
(`E-NUM-PROMOTE`, `L-LIB-TO-STR`). The TIR interpreter decides by the values
it holds and is unaffected; the S3 backend decides by the operands' own types
when both are `f64` (`lower_binary`). Fixed: the emitter writes `f64` as
the operand type when the left operand is a promoted literal, negated or
not.

## Found on 2026-10-22 by the Grasm backend (S3)

Writing a second native backend means deciding every case the first one
decided, and two of its decisions turned out to be wrong. Both are in the
Cranelift build; the interpreter and the S3 backend agree with each other.

**A bare `Err` pattern meant different things in the two engines -- *closed,
S1*.** In `match r { Err -> ..., Ok(v) -> ... }` the interpreter (and TIR)
read `Err` as a binding that matches anything, so an `Ok` takes the first
arm; the native build reads it as a variant test, so an `Ok` takes the
second. The checker accepts the program. Reproducer:

```
fn f(x: i64) -> i64 ! str {
    if x < 0 {
        fail("negative")
    }
    return x
}

fn main() {
    let r: i64 ! str = f(5)
    match r {
        Err -> print("first arm: the bare Err matched an Ok")
        Ok(v) -> print("second arm: ok {v}")
    }
}
```

`grasm run` printed the first arm, the native binary the second.
[G-PAT-BIND] already decides it: a bare name that is a variant carrying
fields is refused, because it is a payload left off far more often than a
binding. The checker applied that to declared sum types only; `Ok` and `Err`
over a fallible value are refused now too ("variant `Err` has 1 field(s) —
bind them: `Err(..)`"), pinned by a check test that fails against the old
checker.

**A `() ! E` function whose body ends in a value leaked that value natively --
*closed, S1*.** The checker accepts a non-unit tail in a function returning
`() ! E` and discards it; the native build never releases it.

```
fn step(n: i64) -> () ! str {
    if n < 0 {
        fail("negative")
    }
    "a value nobody asked for {n}"
}

fn main() {
    for i in 0..100000 {
        step(i) ?? ()
    }
    print("done")
}
```

Built natively and run with `GRASM_LEAK_CHECK=1`: `leak: 100000
allocation(s) still live at exit`; with the tail replaced by `let _unused =
n`, `nothing live at exit`. `leakcheck` never saw it because no corpus
program has this shape. The native backend now releases the discarded value
before returning the unit `Ok`; a CLI test builds this program and requires
a clean leak check, and fails against the old backend.


## Found on 2026-10-19 by an outside review

**The published release cannot build natively -- *open*.** v1.0.0 is two bare
binaries and `SHA256SUMS`. `grasm run` works from the download; `grasm build
--native` stops with "cannot find the Grasm runtime", because
`libgrasm_rt_abi.a` is not in it, and the message told the user to keep a
`libgrasm_rt.a` beside `grasm` that no release has shipped. Reproducer:
`scripts/verify_release.sh --tag v1.0.0 -R grasm-lang/grasm` (exits 1: "v1.0.0
has no .tar.gz archives"). `scripts/package.sh` now makes an archive with the
runtime and BUILD-INFO and refuses to checksum one that cannot build a program
from an empty directory; `.github/workflows/release.yml` runs it, publishes,
and downloads the result again to check it. None of that has produced a
published release yet, so this stays open until one is.

**CI has never run -- *open*.** The workflows are written; the public
repository holds them in `ci/github-workflows/`, because the import was pushed
without GitHub's `workflow` scope. Every gate result in this repository comes
from one arm64 Mac. No Linux binary has been linked or run anywhere.

**Replay called C functions for real -- S1, *closed*.** Under `GRASM_REPLAY`
an `extern fn` ran live: `unsafe { getpid() }` recorded 18839 and replayed
18847, exit 0, in both engines. A recording holds nothing a C function did, so
both engines now stop at the first C call of a replay, with exit 7 and the
function's name -- a direct call, a method call and a function value alike.
`scripts/replaytest.sh` checks all three on both engines.

**`leakcheck` counted crashes and early exits as leak-free -- *closed*.** Only
exit 7 was a leak and only a timeout was set aside; everything else, a segfault
included, was "clean". The runtime now prints `leak check: nothing live at
exit` when the check runs and finds nothing, and only that counts as clean. A
death by signal is reported as a crash and fails. A program that ends before
`main` returns is named; 25 do so on purpose and are listed in the script, and
a new one fails. Those 25 had been counted leak-free without the check running;
none of the 604 crashed.

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

**Paid for again the same day.** [T-INT-FIXED], its three sub-clauses and
[L-LIB-INT-CONV] added 30 lines, which would have made the spec 3,576 lines --
71 pages, over the tripwire. The history below, sentences of record inside
seven normative paragraphs, came out to pay for them: 3,546 lines, 70 pages,
where the day started. The rule in each paragraph stayed; only the account of
what went wrong before it was written moved here.

> From [T-EXTERN-POINTER]:
>
> Without this the C boundary carried scalars only, which excluded `open`, `read`,
> `write`, `system`, `getenv`, `socket` — every libc function taking a pointer,
> which is nearly all of the useful ones. What could be called was `abs`, `sqrt`,
> `pow` and their kind, so "C ABI import overhead: zero-cost" was true and
> described arithmetic.
>
> From [L-LIB-TO-STR]:
>
> `"1.0"`. The interpreter used to hand the job to its host language, which drops
> the point on a whole value, so it answered `"1"` where the compiled backend
> answered `"1.0"` — and the documented idiom `xs.map(|v| v.to_str()).join(", ")`
> printed a `[f64]` as a list of integers, which [L-DISPLAY] exists to prevent.
>
> From [L-LIB-STR] character counts:
>
> proportional to the slice it returns), whatever the text. They
> count *characters*, so each used to decode the string from the start:
> `while i < s.len()` was quadratic in two ways at once, and 400 000 characters
> took 6.7 seconds where the identical loop over a list took 0.21. A string
> never changes once built, so its character count is computed once and kept.
>
> From [L-LIB-STR] character index:
>
> is asked for by position. Fixing only the ASCII half left the same 6.3 seconds
> waiting behind a single two-byte character, and building the index for
> `code_at` while leaving `substr` to walk from the start left a `substr(i, 1)`
> scan quadratic — 39 seconds over 262 144 characters — next to a `code_at` scan
> of the same string costing 0.11.
>
> From [E-CONST]:
>
> This is a promise about cost, not only a permission: rebuilding per mention
> made every *read* of an n-element constant O(n), so a loop of k reads over a
> 4 000-entry dispatch table took 632 ms where the same loop over a local copy
> took 1 ms. A constant is the shape this clause recommends for a table read in
> a loop, and reading one is O(1).
>
> From [E-PAR-FAIL]:
>
> run such a closure sequentially, and the native backend does: its parallel
> workers carry one machine word of result and no error channel, so a `fail`
> inside one used to leave the error's address in the result slot —
> `[1, 2, 3].par_map(|x| if x == 2 { fail(e) } else { x })` printed a raw pointer
> as its middle element and exited 0.
>
> From [T-OK-ERR-RETURN]:
>
> The wrapping happens wherever the branch lands, not only on the way out of a
> function. A fallible function re-wraps its whole body, which masked a gap
> everywhere except the positions that do not — a `let` annotation, a struct
> field, a list element — where the interpreter stored the *raw* value:
> `let a: i64 ! E = if true { 1 } else { Ok(2) }` held `1` rather than `Ok(1)`,
> so it printed as `1`, compared unequal to `Ok(1)`, and matched no arm of a
> `match` over it. A native build, which wraps by produced type, answered
> correctly — the normative engine was the wrong one, at exit 0 with nothing on
> stderr.

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

**Paid for again on 2026-10-20, by [T-U64].** The `u64` clause and its library
rows took the spec to 3,561 lines, 71 pages. The history below came out of
[T-FIELD-CALL] and [T-OK-ERR-RETURN] -- accounts of what each rule's absence
did, inside the rule -- and the spec is 3,542 lines, 70 pages, with every
rule still where it was. Nothing was excluded from the count and the ceiling
did not move.

> From [T-FIELD-CALL]:
>
> field: `n.run(5)`. Building the value and reading the field back both worked
> from the start and the call did not, so the only spelling that ran was
> `let f = n.run` followed by `f(5)` — which nothing pointed to, while the error
> suggested declaring a free `fn run(Node, ...)`, a different function that would
> not call the field at all.
>
> function in the first position; the error used to be "no method `0`", with
> help suggesting a free `fn 0(...)`, which is not a name that can be written.
>
> From [T-OK-ERR-RETURN], on annotations:
>
> same two shapes per branch, and demanding the whole type of every branch made
> `let v: f64 ! E = if c { 3.0 } else { Ok(4.5) }` a type mismatch while the same
> expression without the annotation was an unhandled failure whose help says to
> annotate it. Two diagnostics pointing at each other is no way to write the
> program at all.
>
> On `return`:
>
> `return if c { 3 } else { Ok(4) }` is the two shapes in one statement. Only the
> tail expression was taught this, so `return Ok(1)` was "this expression can
> fail but `i64` is required" — a message about the wrong problem, in the shape
> early exits are written in, and the workaround was to rewrite every one of them
> as `fail`. Deciding by *how the value was written* rather than by what it
> turned out to be left the same hole one level out: an `if` whose branches
> merged to the whole `T ! E` was wrapped a second time on the way out, and the
> caller printed a heap address.
>
> On mixing the halves:
>
> Taking the type from whichever branch came first instead meant
> `if n == 1 { 10 } else { Ok(n * 3) }` was typed `i64`: the `Ok` branch's value
> travelled as a machine word and was wrapped a second time on the way out, so a
> caller that matched `Ok(v)` printed a heap address where the number was.

**And again for [L-LIB-BITS-WIDTH].** Its six rows and two paragraphs came to
15 lines; the history below, from [L-LIB-SORT-BY], [L-LIB-TUPLE],
[G-STRUCT-NONEMPTY] and [G-MATCH-ARM-EXPR], paid for them. Each rule is where
it was.

> From [L-LIB-SORT-BY]:
>
> ([L-LIB-TUPLE]). This clause said "must be `i64`, `f64` or `str`" while tuple
> keys worked in all three engines — two clauses of one document disagreeing, and
> the one that was wrong was the one a reader would find first.
>
> mutates and `rev` answers with a new list, so the clause's earlier wording —
> `xs.rev()` on its own line — left the list ascending and said nothing.
>
> From [L-LIB-TUPLE]:
>
> error. `min` and `max` used to spell out the types they accepted instead of
> asking whether the element was ordered, so `xs.enumerate().min()` was refused
> on a list `sort` orders happily.
>
> From [G-STRUCT-NONEMPTY]:
>
> would be a type with no way to write a value of it. The rules above used to
> make both the field list and the literal's contents optional, so `type E = { }`
> was declared, accepted, and unbuildable, and `E{}` failed with "expected end of
> line after statement".
>
> From [G-MATCH-ARM-EXPR]:
>
> 0..2 { … } }`, not `1 -> for i in 0..2 { … }`. The rule used to read
> `( expression | statement )`, which the parser had never accepted — a grammar
> that promises more than the implementation gives is a worse guide than no
> grammar, because a reader trusts it and gets a parse error with no idea which
> of the two is wrong.

**And for [L-LIB-BUILDER].** The builders' clause, table and the reason they
are types of their own came to 25 lines. These paid for them, each an account
of a defect inside the rule that closed it; the rules stay.

> From [T-MOVE-LOOP]:
>
> rejected too. It was refused in the loop and accepted in the closure, which
> are the same program written two ways.
>
> From [T-PARAM-INOUT-PLACES]:
>
> And an `inout` parameter crosses a module boundary like any other. Both were
> compiled-backend gaps, and both mattered more than they sound — `inout` is the
> answer [T-CAPTURE-WRITE] gives for threading mutable state, so its stopping at
> a file edge reshaped a public API.
>
> From [T-MUTATE-PLACE]:
>
> `mk().pop()` are not: there is nowhere for the mutation to land, and they were
> accepted and then panicked at run time, one of them printing a
> compiler-internal `Span { start: .., end: .. }` into the terminal. Writing to
> a `let` binding was already a compile error, so the temporary was the one
> receiver nothing checked.
>
> From [T-MODULE-NS]:
>
> Clock` declares `Clock` and `ClockHandler`. Both were free to collide with a
> type, and both names stayed usable afterwards. [...] A *type alias* is a type
> declaration like any other, and it was the one that escaped both this rule and
> [T-TYPE-NO-PUB]: aliases resolve a pass after struct and sum bodies, so
> nothing had registered the first `type Id = i64` when the second was
> collected, and two declarations of one name checked clean with the later one
> silently winning.
>
> From [G-TYPE-NOT-QUALIFIED]:
>
> qualifier would carry no information — and the grammar used to allow one,
> which meant the only way to find out was the error.
>
> From [E-IF-VALUE]:
>
> otherwise, so a branch ending in a `with`, `for` or `while` statement sailed
> past the demand entirely: `if c { 7 } else { with D handled_by h { n } }` in an
> `-> i64` function checked clean, `grasm run` printed `()` where the integer
> belonged and a compiled build printed `0` — while the same `with` as the
> *whole* body was refused.
>
> From [E-ENTRY]:
>
> means what that clause says. Only the return type was refused for a while, so
> `fn main(x: i64)` checked clean and then panicked at run time with
> `unknown name \`x\`` — a compiler-internal message arriving after the point
> where types are supposed to be settled.
>
> From [E-IF-FALLIBLE-MERGE]:
>
> the ok half, so a branch that is only `Err` cannot decide the type by itself —
> taking it anyway made `xs.map(|x| if p { Err(e) } else { Ok(x) })` produce a
> list whose elements were later read as the wrong thing.
>
> From [E-LIST-CONCAT], replaced by a pointer to [L-LIB-BUILDER]:
>
> reason: strings are immutable, so each `s = s + x` copies everything so far.
> 400 000 appends take about 2.4 seconds. There is no string builder; collect the
> pieces in a list and `join` them, which is linear.

**And for raw memory and atomics, 2026-10-21.** [T-ADDR], [T-MEM-UNSAFE],
[L-LIB-MEM], [L-LIB-ATOMIC] and [E-MEM-UNCHECKED], their stability entries
and `mem` in [T-EFFECT-RESERVED] came to 70 lines. They were paid for
by restating [T-FN-TYPE-ROW-CARRIED] and [T-FN-VALUE-ROW] as their rules alone:
every rule in them is still there, and the accounts of what each one's absence
did are here instead. 3,543 lines, 70 pages; nothing excluded from the count
and the ceiling unmoved.

**And for function addresses and `export fn`, 2026-10-21.** [T-FN-ADDR],
[T-EXPORT] and [L-LIB-CALL-C], `addr` in [T-EXTERN-POINTER] and [T-EXTERN],
`call_c` in [E-MEM-UNCHECKED] and their stability entries came to 32 lines.
They were paid for by restating [T-EXTERN-POINTER], [T-EXTERN-VALUE],
[T-UNSAFE-FFI], [T-UNSAFE-UFCS] and [T-EXTERN] as their rules alone, the
accounts of what each one's absence did moved to `docs/spec-history.md`: 29
lines saved.
3,546 lines, 70 pages.

**And for thread-local cells, 2026-10-21.** [T-TLS], `mem_tls` in
[L-LIB-MEM]'s table, the declaration's rewrite in `spec/core.md` and its
stability entry came to 11 lines. They were paid
for by restating [T-EFFECT-OP-RESULT], [T-REGION-FOREIGN] and
[T-MAP-KEY-GENERIC] as their rules alone, the history moved to
`docs/spec-history.md`: 14 lines saved. 3,543 lines, 70 pages.

**And for tuple map keys, 2026-10-21.** Widening [T-MAP-KEY], [E-MAP-ORDER]
and [L-LIB-MAP] came to one line net, inside the slack: 3,544 lines, 70
pages.

> From [T-FN-TYPE-ROW-CARRIED], as it read:
>
> This needed saying because it was not true. The row lived only in the syntax
> someone wrote and never in the type, so `fn(i64) -> i64` and
> `fn(i64) -> i64 / {io}` were the same type here and nothing could compare them;
> one hand-written check covered direct call arguments and nothing else. Every
> other route dropped the row, which made each of them a way to hand a printing
> function to a `/ {}` row, to `par_map` past [T-PAR-MAP-PURE], to the
> missing-handler backstop [E-HANDLER-MISSING] says well-typed programs never
> reach, and to a package declaring `capabilities = []`.
>
> That is the rule for a value *arriving* at a written type. Calling one that
> arrived is the other half, and it was only carried by `let`. `let f = fs[0]`
> charged `io` and `for f in fs` charged nothing — the same program, one keyword
> apart — and so did a `match` payload, a tuple destructuring and a closure
> parameter. Each of those is a `for` loop away from a printing function running
> inside a `/ {}` function, on `par_map`'s workers, and in a package declaring
> `capabilities = []`. A call through a local that no binder accounts for is
> charged every row written in the *enclosing function's own signature* at a
> position with no field name — its list elements, its tuple components, its
> `Option` payloads. Scoped to that signature rather than to the program: the
> wider pool charged `io` to a function whose own element type declares no
> effects, on the strength of an unrelated declaration elsewhere in the file.
>
> And the pool is reached for only where nothing is known about the part. Where
> every binding the expression reads has a written type, *those* types say what
> came out: a tuple component, a `??` between two known carriers, a block around
> one. Each of those drew the pool anyway, so a `/ {}` body performing nothing
> was refused on the strength of an unrelated parameter — including a parameter
> whose own row is charged exactly, at every mention of it. Where both a field
> name and the binding's own type are known, the answer is their
> **intersection**: each over-approximates the true row, so the intersection
> does too, and it is the tighter of the two.
>
> An **empty** carried row is a fact, not the absence of one. A parameter written
> `fs: [fn(i64) -> i64]` says its elements perform nothing, and reading one out
> hands that on — but only non-empty rows were recorded, so `let f = fs[0]`
> looked unaccounted and drew the whole pool: a sibling parameter whose own
> *named field* holds an `io` function refused a `/ {}` body that performs
> nothing, which is the shape this clause already records as fixed, one scope
> level in.
>
> Reading a part out of a binding hands on *that* binding's sources, and so does
> copying one whole: `let f = step` is `step`'s row, not the pool. A plain named
> parameter is not a position "with no field name", but the copy fell through to
> the fallback anyway, so an unrelated `log: fn(str) / {io}` in the same
> signature charged `io` to a call through a pure `step` — while `xs.map(step)`
> written directly was accepted. The same program one `let` apart, again.
>
> A `type` name counts as the type it stands for, here as everywhere
> ([T-TYPE-ALIAS]). This is inferred from written syntax rather than from
> resolved types, and a row behind a name was not there: `f: Cb` for
> `type Cb = fn(i64) -> i64 / {io}` recorded an *empty* row, which then shadowed
> every fallback. One `type` line laundered `io` past all four of the guarantees
> above.

> From [T-FN-VALUE-ROW], as it read:
>
> *And for every name, not only a function's.* A binding spelled like an imported
> module made `binding.field` take the *module function's* row: an `io` field
> looked pure, so a printing function reached `par_map` and a `capabilities = []`
> package; and a pure field was charged an unreachable private helper's `io`, so
> a package that performs nothing failed to build in its consumer. A parameter
> spelled like an **effect** demanded a handler for an effect the body never
> performs. Both directions of one defect, and both are the rule below applied
> to a name that is not a function's.
>
> *And only when it is a function.* Scope decides at a **call** as well as at a
> mention: the two arms disagreed, so a *parameter* spelled like a private helper
> was charged that helper's row — a `capabilities = []` package broke in every
> consumer, and `par_map`'s purity proof failed, over one parameter's spelling.
> A local of that name is a different thing:
> a parameter spelled like a private helper charged that helper's row, which
> broke a `capabilities = []` package in every consumer and failed a `par_map`
> purity proof over a closure parameter's spelling. Scope decides, here as
> everywhere else.
>
> *However it is named.* `lib.shout` is the same function `shout` would be and
> carries the same row, and it did not: only the bare spelling was charged, so
> one `import` emptied the row and with it every rule that rests on it —
> `/ {}`, [T-PAR-MAP-PURE], [E-CONST]'s constant table, [T-REGION-FOREIGN],
> [E-HANDLER-MISSING]'s "well-typed programs never reach it", and a package
> declaring `capabilities = []`, which `grasm deps` then certified as performing
> nothing while it printed. A capability ceiling one `import` away from
> meaningless is not a ceiling.
>
> This is deliberately conservative: storing an effectful function and never
> calling it is refused, and the way out is to declare the row. The alternative
> was worse. A built-in higher-order method's function parameter is
> unconstrained by design, since `xs.map(|x| print(x))` is ordinary code — so a
> *named* effectful function passed to one had no row to check against, and
> `[1, 2].map(shout)` inside a `/ {}` function typechecked and printed. One hop
> further, `let fs = [uses]` and then `fs.map(call)`, went past `par_map`'s own
> purity check as well: the effectful function is never called in the source,
> only mentioned, and something else calls it. That reached through `/ {}`,
> [T-PAR-MAP-PURE], the [E-HANDLER-MISSING] backstop, and a package's
> `capabilities = []` ceiling.
>
> That covers a named function. A function value that arrives as a *parameter*
> carries its row on its written type instead, and mentioning one is the same
> permission as mentioning a name: `xs.map(f)` where `f: fn(i64) -> i64 / {io}`
> performs `io`, exactly as `f(x)` does. Only the direct call was charged, so
> `map`, `fold`, `filter`, `any`, `all`, `find` and `sort_by` each carried an
> effect past a declared row. `par_map` was checked, which is what made the hole
> look closed.
>
> A declared C function has no body to infer a row from and what it does is
> unknowable, so its row is `{io, unsafe}` — and that travels with the value.
> Without it, pushing `sqrt` into a `[fn(f64) -> f64 / {io}]` inside an `unsafe`
> block was accepted, and calling it back out ran libc from a function declaring
> nothing at all.

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

**`leakcheck` was red on `E-VALUE-SEMANTICS.gr` (16 allocations at exit) --
*closed*.** Found on 2026-10-15. On 2026-10-19 the program prints `leak check:
nothing live at exit` and the gate reports 0 leaking of 579 checked; the commit
that fixed it was not recorded here.

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

**One float width, no packed layout.** *Narrowed on 2026-10-19 by
[T-INT-FIXED], and on 2026-10-20 by [T-U64].* `i8 i16 i32 u8 u16 u32 u64` are
types of their own that wrap at their width in both engines, checked against
an exact oracle by `scripts/fuzz_intwidth.py`; a `u32` hash no longer wants
masking by hand, and a 64-bit hash (FNV, xxhash, SipHash) prints, compares and
divides as the unsigned number it is. What is still missing: `usize` and `f32`
are refused rather than implemented, so code that packs binary32 still writes
the bit layout itself. Every integer is held in 64 bits in a list or a struct
-- a `[u8]` is not a byte array, and there is no C struct layout -- and
`bytes` indexing still answers an `i64`.

One edge of `u64` is a limit rather than a defect. A `match` pattern on a
`u64` names 0 to 2^63 − 1: the parser folds a pattern's minus sign into its
literal, so `-1` and `0xFFFF_FFFF_FFFF_FFFF` arrive as the same number and
cannot be told apart; a value from 2^63 up is matched in a guard, `n if n ==
0xFFFF_FFFF_FFFF_FFFF`, and both spellings in a pattern are refused rather than
read either way.

Closed on 2026-10-20, found while writing `u64`'s `^`: `i64`'s `^` clamped its
exponent at 4294967295 in both engines (`wrapping_pow` takes a `u32`), so
`3 ^ 4294967296` printed 830436559519656619, the answer for `3 ^ 4294967295`,
where [E-ARITH-INT] wrapping gives 2491309678558969857. The two engines agreed,
so no agreement gate could see it. Both now multiply over every bit of the
exponent, and `scripts/fuzz_intwidth.py` draws exponents past 2^32.

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
from under every window on it, so the length is fixed. A buffer that grows is
a `ByteBuilder` ([L-LIB-BUILDER]), below.

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

**No string builder.** *Closed on 2026-10-21 by [L-LIB-BUILDER].* `str` is
immutable, so `s = s + x` in a loop is quadratic under `grasm run`; the
external port measured roughly 2.4 s for 400,000 appends. `StrBuilder` and
`ByteBuilder` append in amortized constant time in both engines and stay
values -- a copy is a copy, before and after an append, which
`scripts/fuzz_builder.py` checks against a Python model through variables,
struct fields, list elements and `inout` parameters. Measured with
`scripts/builder_bench.py --sizes 100000,200000,400000,800000 --reps 2`
(best of two, on a loaded machine; growth per doubling is the number that
survives load):

    run           100000      200000      400000      800000   growth per doubling
      str  s = s + x          0.505s      1.311s      4.964s     30.907s   2.6x 3.8x 6.2x
      str  StrBuilder         0.041s      0.071s      0.138s      0.266s   1.8x 1.9x 1.9x
      bytes  b = b.concat(x)  0.394s      0.965s      2.556s     10.110s   2.5x 2.6x 4.0x
      bytes  ByteBuilder      0.045s      0.064s      0.120s      0.246s   1.4x 1.9x 2.0x

    native        100000      200000      400000      800000   growth per doubling
      str  s = s + x          0.005s      0.006s      0.006s      0.007s   1.3x 1.0x 1.2x
      str  StrBuilder         0.004s      0.005s      0.007s      0.011s   1.2x 1.3x 1.6x
      bytes  b = b.concat(x)  0.408s      0.996s      2.483s      9.865s   2.4x 2.5x 4.0x
      bytes  ByteBuilder      0.004s      0.005s      0.006s      0.008s   1.1x 1.2x 1.4x

Linear doubles; quadratic quadruples. Both builders double in both engines,
where the interpreter's `s = s + x` and both engines' `bytes.concat` go
past 4x at 800,000 -- 116 times slower than the builder for text under
`grasm run`, 1,200 times for bytes natively. The compiled `s = s + x` was
already linear: it appends in place when the string is the only holder of
its buffer (docs/performance.md, "A chained append was quadratic"); that
rewrite has no interpreter counterpart and no `bytes` one, and a builder does
not depend on recognising the shape.

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

**Raw memory stops at the process's own allocations.** *Added on 2026-10-21
with [L-LIB-MEM] and [L-LIB-ATOMIC].* What is not there yet, each on purpose:

- *Closed in S1 part three:* an `extern fn` takes and returns `addr`
  ([T-EXTERN-POINTER]). Under `grasm run` an address C returns points into no
  allocation the sandbox made, so every access through it is refused, and an
  address the sandbox made cannot be handed to C at all: memory C owns, and C
  reading memory Grasm allocated, need a compiled build.
- There is no `mem_copy` or `mem_fill`; a loop of loads and stores does it.
  Nor is there a way to move bytes between raw memory and a `bytes`.
- `mem_size_of` and `mem_align_of` answer for layout types only. A struct has
  no layout a program may rely on, so a runtime written in Grasm writes its
  own offsets down.
- Atomics were observed from one thread until S1 part three. Threads C starts
  now run Grasm ([T-FN-ADDR]), and a CLI test and `scripts/fuzz_tls.py` add to
  one counter from up to six of them and get the exact total; the orderings
  weaker than sequential consistency are still untested, because a compiled
  build makes every ordering sequentially consistent.
- The interpreter's addresses are made up and its sandbox is bounded only by
  the host: an allocation larger than 2^40 bytes is refused by both engines,
  and one the host cannot meet panics, but a large one that the host *can*
  meet is held in full, as a compiled build's would be.

**C reaching Grasm is native only.** *Added on 2026-10-21 with [T-FN-ADDR],
[L-LIB-CALL-C] and [T-EXPORT].* What is not there yet, each on purpose:

- `grasm run` cannot make machine code for a Grasm function, so it cannot give
  C one: handing C a function's address (a `qsort` comparator, a signal
  handler) panics there and works compiled, and an `export fn` is not in the
  interpreter's process for `dlsym` to find. Those programs are tested by CLI
  tests and `scripts/fuzz_fnaddr.py`, natively. The interpreter does call C
  through an address C handed it, and Grasm through a Grasm address.
- No variadic C function (`printf`), no struct passed or returned by value, and
  no closure as a callback: a closure has an environment C cannot pass, so a
  callback that takes a `void *` context gets an `addr` instead.
- `grasm run` and the TIR interpreter call C only in the shapes
  `crates/grasm_interp/src/ffi.rs` enumerates: at most three arguments, and no
  mixed integer and `f64` shapes past two. A compiled build calls any shape.
- A callback C runs on a thread it made gets the stack floor the main thread
  would have, so deep recursion on a small C-made stack can fault rather than
  panic.
- Names beginning `grasm_` cannot be exported, so a runtime written in Grasm
  cannot yet define the symbols `libgrasm_rt_abi.a` does; S5 lifts that for the
  package that replaces it.
- Executables are `arm64`, whose code pointers are not signed. An `arm64e`
  target would have to sign in `mem_fn_addr` and authenticate in `call_c`.

**A tuple key costs a probe.** *Added on 2026-10-21 with tuple keys
([T-MAP-KEY]).* A compiled lookup builds the key's tuple and then a probe
key from its components, one allocation each, where a `str` probe reuses a
buffer: 2,000,000 lookups in a 1,000-entry map took 64 ns each with a `(i64,
i64)` key, 53 with the same two parts joined into a `str`, and 26 with them
packed into one `i64` (three interleaved runs each, all within a
nanosecond). A borrowed probe that hashes the components where they lie
would close most of it; S5 rewrites the map and is the place for it.

**Thread-local cells hold a word.** *Added on 2026-10-21 with [T-TLS].* What
is not there yet, each on purpose:

- A cell holds one layout type; a pool or a struct of per-thread state is an
  `addr` cell pointing at memory the thread allocates the first time it finds
  the cell zero.
- Natively a thread's block holds every cell of the program and is made the
  first time the thread asks for any, by a runtime call per `mem_tls`; S3 may
  lower it to the platform's own thread-local variables.
- `grasm run` has one thread and one of each cell. The TIR interpreter reads
  the declarations and the operation, and gives each cell an address as
  `grasm run` does, but the bytes behind it wait for the raw-memory sandbox
  another branch is adding there: until the two meet, a program using a cell
  fails `scripts/tirinterp.sh` as the other raw-memory programs do.

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
