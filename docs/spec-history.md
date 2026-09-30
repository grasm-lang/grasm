# How the specification got here

Paragraphs moved out of `spec/` on 2026-09-28 because they record what went
wrong rather than state a rule. `crates/grasm_spec/tests/core.rs` holds the
specification to a page budget (MASTERPLAN §19.2), and a defect's history is
worth keeping but is not the language. Each is filed under the clause it sat
beside; the rule itself is unchanged in `spec/`.

## spec/types.md, after [T-ALIAS-NOT-RECURSIVE]

`type A = [A]` was accepted at the declaration and then walked recursively by
every pass that reads a written type: the process aborted with a Rust stack
overflow, exit 134 and no diagnostic at all — and a dependency carrying the
line took its consumer's toolchain down with it, after `grasm deps` had
printed a partial success line. [G-NEST-LIMIT]: exceeding a cap is a
diagnostic, never a crash.

## spec/types.md, after [T-MATCH-NESTED]

This used to not be checked at all: a payload pattern was recorded as covering
its whole variant, so `match o { Wrap(A) -> … Plain -> … }` compiled and
panicked on `Wrap(B)`. The same held for an integer payload, an `Option`
payload and an `Ok`/`Err` payload. That is precisely the runtime surprise the
clause above exists to prevent.

## spec/types.md, after [T-EFFECT-RESERVED]

`unsafe` is a keyword, so it never parsed. The other two are not, so
`effect io { … }` did — and the `with io handled_by …` it then made available
discharged the *builtin* `io` for everything in the block. A function
declaring `/ {}` printed, `par_map` ran a printing function on several cores
past [T-PAR-MAP-PURE], and a dependency declaring `capabilities = []` read and
wrote files while `grasm deps` certified it as performing nothing. A
capability ceiling one declaration away from meaningless is not a ceiling; a
package may not write away the word that constrains it.

## spec/types.md, after [T-FN-TYPE-ROW]

It was not:
only the shape was compared when a closure met a written function type, so
`Box{ f: shout }` was refused and `Box{ f: |n| shout(n) }` was accepted — the
same function, one closure apart. Every consequence followed from that one
gap: a `/ {}` function printed, `[T-PAR-MAP-PURE]`'s compile-time proof
passed, a C function reached libc from a declared-empty row, a closure
carrying a handled effect left its `with` block inside a list, and a package
declaring `capabilities = []` exported one.

## spec/types.md, after [T-PAR-MAP-PURE]

Reading one was allowed on
the reasoning that a read shares nothing; it shares the reference **count**,
which a read through a reference takes and which is not atomic. The day the
native backend stopped refusing writes through a reference, a program made only
of reads — `[1, 2].par_map(|x| x + acc.n)` — died with SIGTRAP, panicked inside
the runtime with `field access on non-struct`, or printed a different wrong
answer on each run at exit 0. The clause below had named the shape and said it
was "not a race today"; it became one exactly as predicted, and the fix is the
one the prediction implies.

## spec/semantics.md, after [E-NUM-PROMOTE]

The rule used to be stated as "mixing promotes", which was not what the
implementation did, and comparisons had a further asymmetry nothing recorded:
`5.0 == 5` was accepted and `5 == 5.0` was not.

## spec/semantics.md, after [E-IF-VALUE]

The second half was missing. A mismatched `if` became `()` wherever it stood,
and `()` was accepted as an initializer without checking — so
`let a: () = if c { 1 } else { 2.5 }` typechecked and left `a` holding an
`i64` at run time: it printed as `1`, compared unequal to `()`, and could be
passed to a `fn f(u: ())`. `()` has one value.

## spec/core.md

`try` used to be the seventh entry here and is now in the Full table, because
the rewrite turned out to exist and to be exact:
`try e` is `match e { Ok(v) -> v  Err(er) -> return Err(er) }`. That was checked
by writing a program both ways and running it, on the ok branch and the failing
one, rather than by reasoning about it — which is how the other six will have to
be settled too.

## spec/types.md

Each of those
used to report a type error about a program this clause calls well typed, and
the last was the sharpest: `fn f() -> f64 { 3 }` was refused while
`fn f() -> f64 { return 3 }` was accepted and printed `3.0`. In a `match` the
first arm decided for all of them, so `let x: f64 = match n { 1 -> 2  _ -> 3.5 }`
complained that the *second* arm produced `f64`, with help that said to "use a
literal, which does adopt the other side" — which is what was written.

## spec/semantics.md

It did not produce NaN,
so a property over `f64` never saw the one value `[L-LIB-SORT-NAN]` and
`[E-EQ-STRUCTURAL]`'s NaN paragraph exist for — six thousand cases across two
seeds found everything else and never that.

## spec/types.md

The clause used to say that anything without an
annotation is rejected, which was not true of `grasm run` and was true of
`grasm build --native` — so the
one thing it said, it said about a case where the engines differed, and a
reader consulting it about a program `check` had just accepted would have been
told the opposite. Both now accept it: the checker resolves an empty literal
against the whole body, and it hands the answer to the backend, which has
nothing to read the type off otherwise.

## spec/types.md

Without it the obvious spelling
was refused and so was the reference that answers it, leaving `inout` on an
effect operation as the apparent last option — which typechecked and silently
discarded the write ([E-EFFECT-NO-INOUT]). A feature with no working spelling
sends people somewhere worse.

## spec/library.md

it used to offer UFCS for `print`, which is the call it had just refused.

## spec/types.md

`let g =
fs[0]` fell through to the pool of every function value in the function, so an
unrelated closure elsewhere charged its effect to a call through `g` — while
the same program written `for g in fs` was accepted, which is the pair this
clause exists to close, back in the other direction. Where nothing is known
about the part, the fallback is this function's own signature and nothing
wider: reaching for every nameless row in the *program* meant an **unused**
`type Loud = fn(i64) -> i64 / {io}` — in this file or in a dependency — refused
a `/ {}` function whose own tuple component declares no effects.

## spec/types.md

"The same way" is about the *body* too: a named function's is checked against
its declared return, and a closure's was merely inferred. So the expected type
never reached the literals in its tail — `|x| 3` against `fn(i64) -> f64` was
refused while `fn named(x: i64) -> f64 { 3 }`, the same body one closure apart,
was accepted, and so was `|x| { return 3 }`. That is the asymmetry
[T-INT-TO-FLOAT-CARRIED] records as fixed for named functions, still present
one shape in.

## spec/types.md

the expected type reached
only the *immediate* expression, so the same computation was accepted written
with `match` on the outside and refused written with `if`, one keyword apart.

## spec/semantics.md

However it got one. The rule looked for a literal `ref(...)` in the
initializer, so `let ROOT = mk()` where `mk` returns one was accepted — and so
was a struct or a list holding one. It is the constant's *type* that decides
now, so every carrier is covered.

## spec/types.md ([T-REF-VALUE])

Without it `ref(0)` typechecked, produced a `ref i64`, and **no expression in
the language could observe it** — a value that could be constructed and never
used. The workaround was a one-field wrapper struct, which is the same thing
with a name on it.

## spec/types.md ([T-REF-WRITE])

Without this a `ref` graph was **write-once**: closing a cycle needs one write
after both nodes exist, and that write typechecked and then panicked. The
compiled backend refused the shape for a while and no longer does — it writes
into the slab rather than rebuilding the referent, which is what a cycle
requires, and §11 no longer lists it. This paragraph said otherwise for a day
after it stopped being true, which is the failure mode a documented gap has:
the table is checked against the compiler on every run, and prose beside it is
not.

## spec/semantics.md ([E-MATCH-BUILTIN-ARMS])

Those two used to be the same answer, so a value match whose arms it did not
understand compiled to no value at all and the enclosing function returned
whatever was in the register. A wrong answer with nothing reported is the worst
outcome available, and strictly worse than the refusal that replaced it.
