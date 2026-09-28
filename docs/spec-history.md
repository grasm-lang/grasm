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
