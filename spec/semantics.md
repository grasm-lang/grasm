# Evaluation Semantics

The tree-walking interpreter in `grasm_interp` is the **reference
implementation**: where the native backend and the interpreter disagree about
what a program *computes*, the interpreter is right and the backend has a bug.
`scripts/difftest.sh` enforces this on every program in the repository.

Two differences are not that, and are stated where they arise rather than left
to be discovered:

- **Resource bounds.** How deep recursion may go before it stops is
  implementation-defined above the guaranteed minimum, and differs between the
  two ([E-RECURSION-DEPTH]). Both stop by panicking; they stop in different
  places.
- **What the backend can compile at all.** `grasm build --native` rejects a
  few shapes the interpreter runs, always by name and always at build time —
  never by miscompiling. Each refusal says what it cannot do and tells you to
  use `grasm run`. This is a smaller language than the one specified here, and
  the gap is a defect being closed rather than a design.

## 1. Values

**[E-VALUE-SEMANTICS]** All values have value semantics. Assigning, passing,
or storing a value never creates an alias: mutating one binding cannot be
observed through another.

**[E-COW]** Value semantics are implemented with copy-on-write, so a copy costs
a reference-count bump and an actual copy happens only on mutation of a shared
value. This is an optimization and never observable.

## 2. Evaluation order

**[E-ORDER-LTR]** Subexpressions evaluate left to right. Function arguments
evaluate left to right before the call.

**[E-SHORTCIRCUIT]** `and` does not evaluate its right operand when the left is
`false`; `or` does not when the left is `true`.

**[E-CONST]** A module-level `let` is a constant: one value for the whole
program, not assignable, and **its initializer may not perform effects**. It
may call functions and name them as values — a dispatch table
(`let OPS: Map[str, fn(A) -> B] = {…}`) is what constants are most wanted for
— and may use constants declared before it. It
is evaluated wherever the name is used, so an effect in it would happen once
per mention — which would make how often something happens a property of how
many times a name appears. That is why the restriction exists rather than
being a limitation to work around: a value that has to be computed once, with
effects, is a function.

**[E-CONST-REF]** A constant's value may not take a reference. Because it is
evaluated at every mention, each mention would allocate a different slot: the
same constant printed as two different references on one line, releasing it
gave up a slot nothing else would ever name, and a loop reading it allocated
one per iteration. This follows from the clause above and was worth saying
out loud, since it silently destroys the one thing a reference is for.

**[E-CLOSURE-CAPTURE]** A closure captures the values it uses **at the moment
it is created**, not the bindings. Changing a captured variable afterwards is
not visible inside, and a closure may not write to one at all — that is a
compile error ([T-CAPTURE-WRITE]), because the write would change a copy the
caller can never see.

This is the same value semantics every other binding has ([E-COW]), applied to
a closure's environment. It has one consequence worth stating outright, because
it is the shape of program people reach for first: **an effect handler cannot
hold mutable state.** A handler's operations are closures, so a `Store` whose
`write` updates a captured list updates a copy, and the next `read` does not
see it. Handlers are for supplying values and for accepting output; state that
has to change belongs in the caller, threaded through `inout` or returned.

Note the contrast with **[E-SPAWN-SHARED]**: a `spawn` body *does* see and
mutate the enclosing scope. Two block-shaped constructs, two rules.

A capture is a value, and for a value that lives on the heap that means
holding a reference to it. The compiled backend stored the bare pointer, so
the binding still looked uniquely owned and a later `xs.push(v)` mutated the
very list the handler had captured — `[E-COW]` copies on mutation of a
*shared* value, and this made a shared value look unshared. Of forty
handler-capture shapes, thirty-two disagreed between the engines: a mutating
method returned the new value, a reassignment freed what the handler still
pointed at and the next call read a raw word as the wrong type, and returning
the captured value crashed with nothing on stderr.

**[E-COALESCE-DIVERGE]** The right-hand side of `??` may leave instead of
producing a value — `m.get(k) ?? fail(e)` is the shortest way to turn absence
into a failure, and it is the shape a fallible function's body most often has.
`fail` is an expression; `return` is a statement and cannot go there
([T-DIVERGE]).

**[E-COALESCE-LAZY]** `a ?? b` evaluates `b` only when `a` is `None`.

## 3. Arithmetic

**[E-ARITH-INT]** `+ - *` on `i64` wrap on overflow (two's complement), and
so does `/`: `i64::MIN / -1` has no representable result and yields `i64::MIN`
rather than trapping. `i64::MIN % -1` is `0`, as `x % -1` is for every `x`.

**[E-DIV-ZERO]** Integer `/` or `%` by zero panics.

**[E-POW]** `^` is exponentiation, not the bitwise xor it is in the C family.
`2 ^ 10` is `1024`. It works on `f64` as well as `i64` — `2.0 ^ 0.5` is a
square root — and a float exponent may be fractional or negative.

**[E-POW-NEG]** A negative *integer* exponent panics: `2 ^ -1` has no `i64`
value and silently returning `0` would be worse than stopping. Write it in
floats, where `2.0 ^ -1.0` is `0.5`.

**[E-RECURSION-DEPTH]** Recursion is bounded. **At least 10,000 nested calls
are guaranteed to work.** Past an implementation's own bound the program
panics with a stack-overflow message and exit 101 — it does not abort, crash,
or corrupt the process. This holds for compiled programs too: a function that
can reach itself checks the remaining stack on entry, because nothing in the
generated code would otherwise notice, and the failure was SIGSEGV with an
empty stderr. The bound above the guaranteed minimum is
implementation-defined and differs between the two engines, so a program that
recurses deeper than 10,000 should not assume it will run everywhere. The
*message* does not differ: both engines say `stack overflow: recursion too
deep`, and neither names its own bound, because a number printed there reads
like a limit worth coding against and is not one.

**[E-MATCH-BUILTIN-ARMS]** A match yields a value whenever its arms do,
whatever the arms are made of — a builtin call is not a special case. The
compiled backend types a match by looking at its arms, and where it cannot, it
**refuses**; it does not fall back to treating the match as a statement. Those
two used to be the same answer, so a value match whose arms it did not
understand compiled to no value at all and the enclosing function returned
whatever was in the register. A wrong answer with nothing reported is the worst
outcome available, and strictly worse than the refusal that replaced it.

**[E-PANIC-TRACE]** A panic *may* be followed by a stack trace on stderr, and
whether one appears depends on how the program was run, not on what it did.
`grasm run` prints one, and so does a bundled executable, which is this
interpreter with the program attached. A `--native` build does not: the
compiled code keeps no frame table, and paying for one on every call to
improve a message that only prints as the process dies is the wrong trade. The
panic line itself, and the exit status, are identical in all three. This is the
one place the three ways of running a program differ in what reaches stderr,
which is why `scripts/agreetest.sh` strips the trace block and compares
everything else exactly.

**[E-RECURSION-PARALLEL]** The bound above holds inside `par_map` and
`par_filter` too. A worker runs on its own stack, and it must be big enough for
the same guarantee — a worker on the platform's default stack computed its
floor below the end of it, so the guard could never fire and deep recursion in
a parallel closure took SIGBUS with an empty stderr, which is the exact outcome
[E-RECURSION-DEPTH] rules out. Small inputs run sequentially and were
unaffected, so this only appeared past a few hundred elements.

**[E-ARITH-FLOAT]** Float arithmetic follows IEEE-754, including infinities and
NaN. Division by zero yields an infinity rather than panicking.

**[E-NUM-PROMOTE]** An integer *literal* meeting an `f64` becomes one, on
either side and for arithmetic and comparison alike: `3 * f`, `f * 3`,
`3 < f` and `f > 3` all work. A non-literal `i64` does not convert on its own
([T-INT-TO-FLOAT]) — `n > f` is an error that says to write `n.as_f64()`.

The rule used to be stated as "mixing promotes", which was not what the
implementation did, and comparisons had a further asymmetry nothing recorded:
`5.0 == 5` was accepted and `5 == 5.0` was not.

**[E-STR-CONCAT]** `+` on two `str` values concatenates them.

**[E-STR-COMPARE]** `< <= > >=` on `str` compare lexicographically by code
point.

**[E-EQ-STRUCTURAL]** `==` compares structurally and recursively; two values of
the same shape and contents are equal.

## 4. Collections

A float that is not equal to itself is the one exception, and it is inherited
rather than chosen: `NaN == NaN` is false, so a struct, list, tuple or
`Option` containing one is not equal to itself either, and `contains`,
`assert_eq` and de-duplication all fail on it. `cmp` answers `0` for the same
pair, because `[L-LIB-SORT-NAN]` needs a total order to sort by. Both are
right for what they are for, and they are one method apart.

**[E-INDEX-BOUNDS]** Indexing a list out of bounds panics with the index and
the length. `list.get(i)` returns `Option` instead of panicking.

**[E-LIST-CONCAT]** `xs + ys` is a new list holding `xs`'s elements then
`ys`'s. Neither operand is changed. `+` on lists never means element-wise
addition.

**[E-MAP-LIT]** `{k: v, ...}` builds a map. Entries are evaluated left to
right, each key before its own value, and a repeated key keeps the last value
written. `{}` is the empty map: an empty block has no use as an expression and
is spelled `()`. Anything else in braces is a block, because a block cannot
carry a `:` where the first entry's colon would sit. The braces that belong to
`if`, `for`, `while`, `match` and function bodies are always blocks — they are
not in expression position.

**[E-MAP-ORDER]** Map iteration (`keys`, `values`, printing) is in sorted key
order — `false` before `true`, then integers ascending, then strings
lexicographically — so program output is deterministic.

**[E-STR-CHARS]** String length and indexing operate on Unicode characters,
not bytes.

**[E-PAD-CYCLE]** `pad_start` / `pad_end` widen a string to a character count
using a fill that repeats cyclically and is cut to the exact width. A string
already at or over the width is returned unchanged; an empty fill panics.

**[E-SEARCH-SHORTCIRCUIT]** `any` and `find` stop at the first element whose
predicate holds; `all` stops at the first that fails. Elements after the
deciding one are never passed to the predicate.

**[E-MINMAX-OPTION]** `min` / `max` return `Option`: `None` on an empty list,
so an empty list is not a panic and not a sentinel value.

## 5. Control flow

**[E-IF-DIVERGE]** A branch that leaves — through `fail`, `panic`, `return`,
`break` or `continue` — produces no value, so an `if` used as a value is worth
whatever its *other* branch produced. `if c { fail(e) } else { v }` is `v`'s
type, and an `else if` chain of failures is the type of the one arm that does
produce something.

**[E-IF-VALUE]** `if` is an expression when both branches yield the same type;
otherwise it is a statement of type `()`.

**[E-MATCH-ORDER]** Match arms are tried top to bottom; the first whose pattern
matches and whose guard passes is taken.

**[E-LOOP-RANGE]** `for i in a..b` iterates `a` up to but excluding `b`;
`a..=b` includes `b`. An empty range runs zero times.

**[E-LOOP-RANGE-MAX]** The bounds are `i64` values, `i64::MIN` and `i64::MAX`
included. `mx..=mx` runs once. Neither engine managed that: the interpreter
built the inclusive form as `a..(b + 1)`, which wraps to `i64::MIN` at the top
of the range and iterated over nothing, and the compiled backend tested
`i <= end` after incrementing, so `i` wrapped and the loop never ended. An
empty loop and an endless one, from the same program, from an overflow neither
reported.

**[E-LOOP-LIST]** `for x in xs` iterates a list's elements directly, in order.
The loop variable is the element, not its index. Iterating an empty list runs
zero times.

## 6. Errors and panics

**[E-ERR-PROPAGATE]** `try` on `Err(e)` returns `Err(e)` from the enclosing
function immediately, skipping the rest of its body.

**[E-PANIC-EXIT]** A panic prints `panic: <message>` **on stderr** and exits
with status 101. It is a program bug, not a recoverable condition. It is not
the program's output either: on stdout it lands in the middle of whatever is
reading the program, which is the one place it does damage and the one place
nobody looks.

**[E-MAIN-ERR]** An error reaching `main` prints
`error: unhandled error from main: <value>` on stderr and exits with status 1.

Which is to say: stdout is what a program *produces*, stderr is what went
wrong with it, and the two never mix.

**[E-PRINT-ATOMIC]** `print` writes a whole line or none of it. Every
interpolated expression is evaluated first, so a panic partway along a line
leaves nothing of that line on stdout — which is what makes the rule above hold
for the common case of panicking inside an interpolation. The same order means
a `print` reached from inside an interpolation completes *before* the line that
contains it, rather than landing in the middle of it.

## 7. Effects

**[E-EFFECT-NO-INOUT]** An effect operation may not take an `inout`
parameter. A handler is reached through a function value, so the write lands in
a copy the caller never sees: the handler reports success and the caller reads
back an unchanged value. It typechecked and every engine agreed on the wrong
answer, which is the one shape differential testing cannot see. Return the new
value from the operation instead.

**[E-HANDLER-INNERMOST]** An effect operation dispatches to the innermost
enclosing handler for that effect.

**[E-HANDLER-SCOPE]** A handler is installed for the dynamic extent of its
`with` block, including calls made from inside it.

**[E-HANDLER-OWN-ROW]** A handler's *own* effects are counted where the handler
is **installed**, not where its operations are called. So a `B` handler whose
body performs `A` needs `A` handled around the `with B` block, even if every
call to `B.b()` happens inside an `A` block further in.

This is narrower than the dynamic extent above, and it is a limit of the
analysis rather than a design choice: effect rows are computed per function
from the call graph, and which handler is installed at a given moment is a
property of the run. Stating it is better than leaving a reader to infer the
dynamic rule and be refused — the message they get names the enclosing
function, which is the one place the fix does not go.

Counted where it is installed means *counted*, including when the handler was
built somewhere else. A handler type says nothing about what its operations
do — that is what lets `LogHandler{ info: |m| print(m) }` be written at all —
so installing one that arrived as a parameter charges what *any* handler for
that effect does, anywhere in the program. Over-approximate, and the safe
direction: it was previously charged only where the handler was *constructed*,
so `fn run(h: LogHandler) / {}` installed a printing handler and the
declared-empty row held.

**[E-HANDLER-MISSING]** A user effect that reaches `main` or a test without a
handler is rejected *at compile time*, so the runtime's missing-handler panic is
a defensive backstop that well-typed programs never reach.

## 8. Concurrency

Keeping that true takes one restriction. A closure written inside a `with`
has the effect discharged *there*, because there is where the handler is — so
storing that closure somewhere declared before the block began carries the
discharge out with it, and calling it afterwards has nowhere to send the
effect. Both spellings are refused: assigning to an outer binding, and
pushing into an outer container.

**[E-PARALLEL-JOIN]** A `parallel` block does not exit until every task spawned
inside it has completed.

**[E-SPAWN-ORDER]** Tasks run to completion in the order they were spawned.
This is deterministic: the same program always produces the same output.

**[E-SPAWN-SHARED]** A task body sees the enclosing scope, which is how tasks
share results. This is the opposite of how a closure body behaves — see
**[E-CLOSURE-CAPTURE]** — and the two are worth reading together, because
`spawn { … }` and `|x| { … }` look alike and do not capture alike.

**[E-TASK-PANIC]** A panicking task aborts the whole scope; tasks spawned after
it do not run.

**[E-NOT-PARALLEL]** `parallel`/`spawn` tasks are *not* executed on multiple
cores. That scope provides structure and determinism, not speedup — tasks share
the enclosing scope, and running them concurrently would race. Multi-core
execution of *data* is provided by `par_map` instead.

**[E-PAR-FILTER-ORDER]** `xs.par_filter(f)` tests elements concurrently and
keeps the survivors in **input order**. The predicate parallelises; the
compaction that follows does not, so the result does not depend on how many
cores ran it.

**[E-PAR-MAP-ORDER]** `xs.par_map(f)` applies `f` to every element, possibly on
several cores, and returns the results in **input order**. The result does not
depend on how many cores ran it — including one — so a `par_map` is
interchangeable with a `map` in every observable way.

Any list, however it was built. A compiled backend may store a list of
scalars as raw words, chosen from the static type — and a runtime producer
that builds a boxed list for the same type must not change what the program
computes. `m.values().par_map(f)` built and then panicked `par_map on a
non-scalar list` where `grasm run` answered, which is a miscompile rather than
the refusal §11 promises.

**[E-PAR-FAIL]** A `par_map` or `par_filter` whose closure fails produces the
failure, exactly as the `map` or `filter` written in its place would. Because
the result never depends on how many cores ran it, an implementation is free to
run such a closure sequentially, and the native backend does: its parallel
workers carry one machine word of result and no error channel, so a `fail`
inside one used to leave the error's address in the result slot —
`[1, 2, 3].par_map(|x| if x == 2 { fail(e) } else { x })` printed a raw pointer
as its middle element and exited 0.

## 9. Ownership at runtime

**[E-INOUT-FAIL]** A function that fails still returns, so the `inout` writes
it made before failing are visible to the caller. Discarding them would make
"what the function did" depend on how it ended, and a `push` before a `fail`
would silently unhappen.

**[E-INOUT-WRITEBACK]** Mutations to an `inout` parameter are visible to the
caller when the function returns.

**[E-BORROWED-READ]** A read-only method called on a plain variable does not
change that value's reference count. The variable holds a reference for the
whole of the call, so the count cannot reach zero during it, and the usual
+1/-1 pair is traffic with no effect.

This is an observable property rather than an optimization detail: it is why
`xs.len()` in a loop costs one runtime call instead of three, and it is the
first step of the larger job — proving where a value's last use is, so the
release can be placed there instead of counted for.

**[E-OWN-RUNTIME]** `own` has no runtime effect beyond an ordinary pass; the
guarantee it provides is entirely static.

## 10. Testing

**[E-TEST-PROPERTY]** A property test runs its body once per generated case.
It passes only if every case holds; the first failing case ends the run.

**[E-TEST-SHRINK]** A failing property is reported with a *shrunk*
counterexample: the runner repeatedly substitutes smaller inputs and keeps any
that still fail, so the reported values are minimal rather than whatever the
generator happened to produce. Smaller means simpler as well as shorter — a
string shrinks its characters toward `a` once it can no longer lose any, since
`s = "🙂/🙂"` is length-minimal and tells a reader nothing about the property
that rejected it.

**[E-TEST-SEED]** Generation is driven entirely by a seed. The same seed and
case count reproduce the same run exactly, and a failure reports the seed that
found it.

**[E-BENCH-WARMUP]** A benchmark body runs once before timing starts. A body
that panics is reported as a failure instead of a measurement.

## 11. Program startup

**[E-ENTRY]** Execution begins at `main` in the entry module. A program
without `main` is an error.

**[E-ARGS]** `args()` returns the arguments the program was given, excluding
the program's own name. How they are given depends on how the program is run,
and the `--` in `grasm run prog.gr -- a b` belongs to the toolchain rather
than to the program: it separates the toolchain's arguments from the
program's. A compiled binary has no such separator, so `./prog a b` passes the
same two arguments. In every mode `args()` sees `[a, b]`, and a literal `--`
among a program's arguments is an ordinary argument.
