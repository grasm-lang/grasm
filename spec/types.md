# Type System

Grasm is **statically and strongly typed** with local inference. Types are
checked before any program runs; there is no dynamic typing and no implicit
conversion between unrelated types.

## 1. Primitive types

**[T-PRIM]** The primitive types are `i64` (64-bit signed integer), `f64`
(IEEE-754 double), `bool`, `str` (UTF-8 text), and `()` (unit, the type of a
statement that yields nothing).

**[T-INT-ALIAS]** The names `i8`, `i16`, `i32`, `u8`, `u16`, `u32`, `u64`,
`usize`, and `int` are currently aliases for `i64`; `f32` and `float` are
aliases for `f64`. Distinct fixed-width types are designed but unimplemented.

**[T-UNIT-RETURN]** A function returning `()` may say so and may `return ()`
anywhere, including from a branch. A backend may give it no return value at
all; it may not treat that form as different from falling off the end.

## 2. Composite types

**[T-LIST]** `[T]` is a homogeneous, growable sequence of `T`.

**[T-MAP]** `Map[K, V]` maps keys to values. **[T-EFFECT-FALLIBLE-CONSUME]** An effect operation declaring `! E` answers
with a `T ! E`, whichever way its handler ended: a value becomes `Ok`, a
`fail` inside the handler becomes `Err`. So `try`, `match` and `??` all
consume one, the same as any other fallible value.

**[T-MAP-KEY]** `K` must be `i64`, `str`, or `bool`; any other key type is a
compile error.

**[T-MAP-KEY-GENERIC]** A *type parameter* is allowed where a map key goes,
and [T-MAP-KEY] is applied at each call site instead, once the arguments have
said what the parameter is. There is no bounds syntax, so without this a
generic helper over maps — `fn bump[K](inout m: Map[K, i64], k: K)`, which is
the most repeated shape a counting program has — could not be written at all.

"Each call site" means the whole instantiated signature — the parameters and
the **result**. Only the parameters were checked, so
`fn fresh[K](k: K) -> Map[K, i64]`, which puts the key type nowhere in its
parameters, let `fresh(2.5)` past the checker and into a runtime
`invalid map key`.

A type parameter reaching a map *only* inside the body counts too, and is
named in no part of the signature at all: `fn tally[T](xs: [T]) -> i64`
holding a `Map[T, i64]` let `tally([1.5, 2.5])` through. What a body does with
its type parameters is known once every body has been walked, so these call
sites are checked at the end of the analysis rather than where they appear.

**[T-TUPLE]** `(A, B, …)` is a fixed-size heterogeneous product.

**[T-STRUCT]** `type P = { x: T, … }` declares a named product type whose
fields are accessed with `.`.

**[T-SUM]** `type S = | A(f: T) | B` declares a named sum type; values are
built with variant constructors and consumed with `match`.

**[T-ALIAS-NOT-RECURSIVE]** A `type` alias may not be defined in terms of
itself, directly or through other aliases. It is another spelling of the type
it names, so it has nowhere to stop — unlike a struct or a sum, which
[T-RECURSIVE] does allow to refer to themselves, because a value of one is
built out of *other* values.

**[T-RECURSIVE]** A type may refer to itself, directly or through a
collection: `type Json = | JArr(items: [Json]) | JObj(fields: Map[str, Json])`
is the shape a parser needs and is legal. Recursion through a field of the
same type is legal too, since every composite is a reference underneath.

**[T-OPTION]** `Option[T]` is the prelude sum type `| Some(v: T) | None`.

**[T-NO-NULL]** There is no null value. Absence is represented by
`Option[T]`, so no expression can be unexpectedly empty.

## 3. Inference

**[T-INFER-LOCAL]** The type of a `let`/`var` binding is inferred from its
initializer unless annotated. Function signatures are never inferred —
parameter and return types are always written explicitly, which keeps
inference local and errors near their cause.

A closure's body waits for its call sites ([T-FN-TYPE]), so what it returns is
not settled where it is written — and a *field read* of that result asked
before the block ended, and was told ``_` has no field `c``: a type no program
can write, about a value the same block infers perfectly well. Every other use
of the identical call worked, so the refusal turned on which use came first.
The read waits with the body; a receiver still unknown when the block ends is
genuinely unknown, and is refused then with the type it does have.

**[T-INT-TO-FLOAT]** An integer *literal* is accepted where `f64` is expected
and is converted at compile time. A non-literal `i64` value is not implicitly
converted; use `.as_f64()`.

**[T-INT-TO-FLOAT-CARRIED]** "Converted at compile time" means *everywhere the
value goes afterwards*, not only where it was written. `xs.push(3)` into a
`[f64]` pushes `3.0`, so the list sorts as floats, compares equal to the same
list written `3.0`, and prints with a point. This is a rule about the engines
rather than about the language, and it needed saying because neither engine had
the information: only the checker knows which literals are floats, and each
backend had been converting where it happened to remember to — differently, and
in both cases incompletely. The checker now hands the set over, so the answer is
the same one twice.

Order does not matter, and neither does position. A `fold`'s seed adopts the
element type, so `[1.5, 2.5].fold(0, |a, b| a + b)` needs no `0.0`. A list, a
map's values, an `if`'s branches and a `match`'s arms are checked with their
integer literals *last*, so whatever fixes the type does it first: `[1, 2.5]`
and `[2.5, 1]` are both `[f64]`. With no expected type at all the branches have
only each other, and the literal is the one that adopts: `let y = if c { 3 }
else { 2.5 }` is a `f64`, which it was not — while the two-arm `match` on a
`bool` that says the same thing was. Checking left to right made the first
element decide, which is the asymmetry `[E-NUM-PROMOTE]` records as fixed for
comparisons, still present one level up.

**[T-COMPARE]** `==` and `!=` compare any two values of one type; `<`, `<=`, `>`, `>=` need `i64`, `f64`, `str`, or a tuple of ordered components.

The promotion crosses the two halves of a fallible value too. In
`fn f(c: bool) -> f64 ! E { if c { 3 } else { Ok(4.5) } }` one branch gives the
bare ok half and the other the whole `T ! E` ([T-OK-ERR-RETURN]); the `3` is a
`f64` all the same. Only the all-bare form worked, so adding an `Ok` to one
branch turned a compiling function into "the branches of this `if` have
different types".

An expected type also reaches a literal *through the shape that carries it*: a
list, tuple or map literal, `Some`, `Ok`, `Err`, an `if`'s branches, a
`match`'s arms, and the expression a function body ends with. Each of those
used to report a type error about a program this clause calls well typed, and
the last was the sharpest: `fn f() -> f64 { 3 }` was refused while
`fn f() -> f64 { return 3 }` was accepted and printed `3.0`. In a `match` the
first arm decided for all of them, so `let x: f64 = match n { 1 -> 2  _ -> 3.5 }`
complained that the *second* arm produced `f64`, with help that said to "use a
literal, which does adopt the other side" — which is what was written.

**[T-EMPTY-LIT]** An empty collection literal takes its element type from the
surrounding annotation, struct field, parameter, or return type — or, failing
all of those, from a later use: `var xs = []` followed by `xs.push(1)` is a
`[i64]`, because inference reads the whole body before deciding. A literal
whose type nothing determines is rejected.

`map()` is an empty literal like `[]` and `{}`, and was not treated as one:
`let m = map()` used only through `values()` left the *key* type decided by
nothing, which `grasm run` answered anyway and the compiled backend refused —
an acceptance split between the two engines over a program the checker had
just approved. A method that pins the element type counts as determining it,
which is why `[].sum()` is an `i64`.

A literal whose type nothing determines *is* rejected, by the checker, so both
engines agree about it. The clause used to say that anything without an
annotation is rejected, which was not true of `grasm run` and was true of
`grasm build --native` — so the
one thing it said, it said about a case where the engines differed, and a
reader consulting it about a program `check` had just accepted would have been
told the opposite. Both now accept it: the checker resolves an empty literal
against the whole body, and it hands the answer to the backend, which has
nothing to read the type off otherwise.

## 4. Generics

**[T-GENERIC-FN]** `fn f[T](x: T) -> T` is parametrically polymorphic. Type
arguments are inferred at the call site.

**[T-GENERIC-INST]** Each call instantiates the signature with fresh
inference variables; a generic body is checked once against its parameters.

**[T-GENERIC-TYPE]** `type Pair[A, B] = { fst: A, snd: B }` and
`type Opt[T] = | Has(v: T) | Nothing` declare *families*. A use fixes the
parameters: a struct literal reads them off its fields, a sum constructor off
its arguments. Two members are two types — `Has(5)` and `Has("text")` are an
`Opt[i64]` and an `Opt[str]`, and a `match` binds each payload at its own.

A parameter no argument mentions must be written down: `Good(1)` for
`Res[T, E]` fixes `T` and not `E`, so it needs `let r: Res[i64, str] = Good(1)`.
An engine carrying types at runtime can leave it open; one compiling ahead of
time needs a single representation and has nothing to choose it from, so the
requirement is the language's rather than either engine's. The check waits for inference to
finish, so a parameter a *later* line fixes is determined: `let a = Good(1)`
followed by a use that needs `Res[i64, str]` is accepted.

### Interfaces

**[T-INTERFACE]** An `interface` names methods a type can provide; an `impl`
provides them for one type:

```grasm
interface Show {
    fn show(self) -> str
}
impl Show for Point {
    fn show(self) -> str { "({self.x}, {self.y})" }
}
fn describe[T: Show](x: T) -> str { "<" + x.show() + ">" }
```

`self` is the receiver; `Self` names the implementing type. An `impl` method
is called as a method and resolves by the receiver's type, before any builtin
method of that name. An `impl` is for a declared non-generic type or for
`i64`, `f64`, `bool` or `str`, and a type has at most one `impl` of each
interface.

**[T-INTERFACE-BOUND]** In `[T: Show + Ord]`, a `T` offers exactly its bounds'
methods, checked once at the definition: a method no bound provides is an
error there, whatever the callers pass.

**[T-INTERFACE-UNSATISFIED]** Each call must give a bounded parameter a type
with an `impl` of every bound, or a type parameter of the caller carrying
those bounds; otherwise it is an error naming the type and the interface.

**[T-INTERFACE-IMPL]** An `impl` provides every method of its interface, each
with the interface's signature under `Self` = the implementing type, and
nothing else.

**[T-INTERFACE-PURE]** An interface method declares no effects, so an `impl`
method may perform none; one that does is an error at the `impl`.

## 5. Errors as values

**[T-FALLIBLE]** `T ! E` is the type of a computation yielding `T` or failing
with `E`. It is an ordinary value, not an exception.

**[T-FALLIBLE-UNHANDLED]** A fallible value may not be used as if it were `T`.
It must be consumed by `try`, by a `match` covering `Ok` and `Err`, or by
`??`. Discarding it counts as using it: a call written as a bare statement
throws its failure away, and nothing says so — and so does one written as the
last line of a loop body, of a branch of an `if` used as a statement, or of a
`spawn`, `parallel`, `test` or `bench` body, where the block's own value goes
nowhere either. A test whose entire body was a failing call reported `ok`,
which is worse than having no test. `write_file` to a path that
cannot be written printed "wrote the report", exited 0, wrote nothing to
stderr and left no file — and every user function that can fail behaved the
same way when its result was ignored.

A container built out of failures and then discarded discards them too:
`[risky(x)]` and `xs.map(risky)` written as bare statements throw away every
failure they hold. One pair of brackets was enough to get the old silence
back — and so was one *constructor*: `Some(save())`, a struct literal with a
fallible field, a sum variant carrying one. A named type holds values like any
other container. [T-FALLIBLE-VALUE] keeping the failures in the list is what makes them
readable — while somebody reads them.

`main` is not an exception to that half. Its open error slot ([E-MAIN-ERR])
carries *one* failure, so a directly fallible tail there is propagated rather
than discarded — but a container of them is not, and the whole rule was
skipped for `main` on the strength of the slot. `fn main() { xs.map(risky) }`
threw away every failure it held: silent, exit 0. Adding any line after it
caught the identical expression, so the silence depended only on position.

**[T-FALLIBLE-COALESCE]** `e ?? d` where `e : T ! E` yields the value or `d`,
the same shape `??` has over an `Option` ([E-COALESCE-LAZY] applies equally:
`d` is evaluated only on failure). Absence and failure are different things,
but "the value, or this default" is one thing, and writing it as a `match` at
every call site is the most repetitive shape in a program that reports errors
as values. The error itself is discarded — reach for `match` when it matters.

`() ! E` works the same way, and `f() ?? ()` is how a call whose failure is
deliberately ignored says so — the one consumer a value-less fallible call
otherwise lacks, since `let _ = f()` is refused by the clause above.

**[T-TRY]** `try e` where `e : T ! E` yields `T`, propagating `Err` to the
caller. It is a compile error in a function that declares no error type —
except in `main` and in tests, whose error slot is open because the runtime
reports whatever reaches it (see **[E-MAIN-ERR]**).

**[T-ERR-SUBSET]** `try` is allowed only when the callee's error type is a
subset of the caller's declared error union.

**[T-ERR-UNION-OPAQUE]** A union error type is, today, **opaque**: a value of
`(A | B)` can be raised, propagated, caught by `Err(e)`, and printed, and it
cannot be narrowed back to `A` or `B`. There is no pattern for it and no field
access through it — `e.key` is "`(A | B)` has no field `key`", and matching
`A(..)` against it is refused. So a function declaring two error types can say
*that* it failed and not *which way*, which is usually the reason for declaring
two.

This is a gap, not a design position, and it is stated here because the type is
easy to arrive at without asking for it: [T-ERR-SUBSET] promotes a caller's
error type to the union of what it propagates. Until narrowing exists, a
program that needs to branch on which failure occurred should declare one error
type that is a sum:

```
type Failure = | NotFound(key: str) | Cycle(path: [str])
```

which matches exhaustively, carries the same information, and is checked the
same way everywhere.

`NotFound` is also a variant of the prelude's `IoError`, and a variant name
shared by two types has to be *constructed* qualified: `fail(Failure.NotFound(k))`,
not `fail(NotFound(k))`. Reading one does not — `Err(NotFound(k))` in a match
arm already knows which type it is matching, because the scrutinee says so.
The example is written with the collision on purpose: it is the name anybody
reaches for first.

**[T-FAIL]** `fail(e)` raises the error branch. Its type fits any context, and
it is a compile error where no error type is declared.

## 6. Exhaustiveness

**[T-MATCH-EXHAUSTIVE]** A `match` must cover every possible value. Coverage
is checked for sum types, `bool`, `Ok`/`Err`, and products of booleans; other
scrutinee types require a `_` arm.

**[T-MATCH-NESTED]** A pattern's *payload* counts too. `Wrap(A)` matches only
the `Wrap`s carrying `A`, so it does not on its own cover `Wrap`.

**[T-MATCH-NESTED-COVER]** Payload patterns that *between them* cover the
payload type do cover the variant: `Wrap(A)` and `Wrap(B)` cover `Wrap` exactly
when `A` and `B` cover `Inner`. It is the same question one level down, asked
recursively. Without this the stricter rule above would push people to a `_`
catch-all, which hides the next variant anyone adds — trading one silent
failure for another. Where a variant carries more than one field, coverage is
not tracked precisely, and it needs an arm that binds rather than
destructures.

**[T-MATCH-GUARD]** A guarded arm does not contribute to coverage, because its
guard may fail.

**[T-MATCH-ALT]** An arm may list alternatives, `A | B -> …`, and covers every
one of them. No alternative may bind a name: allowing it would require every
alternative to bind the same names at the same types, which is a great deal of
rule for a form people reach for to group literals and nullary variants. Give
each alternative its own arm when it needs to bind.

**[T-ARM-TYPES]** Every arm of a `match` used as a value must produce the same
type.

## 7. Mutability

**[T-LET-IMMUTABLE]** `let` bindings cannot be reassigned or mutated.

**[T-VAR-MUTABLE]** `var` bindings can be reassigned, and mutating methods
such as `push` require one.

## 8. Ownership (Tier 0)

**[T-PARAM-LET]** A plain parameter passes the value without transferring
ownership; the caller keeps using it.

**[T-PARAM-OWN]** An `own` parameter transfers ownership. **[T-USE-AFTER-MOVE]**
Using a moved binding afterwards is a compile error. Reassignment revives it.

The transfer is tracked for *bindings*. An argument that is a field or an
index — `eat(b.xs)`, `eat(rows[0])` — is a value read out of a place, and
reading a place copies ([E-VALUE-SEMANTICS]), so the place keeps what it had
and using it afterwards is fine. That is sound, and it is not the invalidation
the first sentence describes, so it is written down here rather than left to
be discovered.

**[T-MOVE-LOOP]** Moving a binding declared outside a loop from inside that
loop is rejected: the second iteration would use a value that is gone. A
closure body is the same case — it runs once per call, and the second call
would find the value gone — so moving a *captured* binding out of one is
rejected too. It was refused in the loop and accepted in the closure, which
are the same program written two ways.

**[T-COPY]** `i64`, `f64`, `bool`, and `()` are copied rather than moved, so
`own` on them never invalidates the caller's binding.

**[T-PARAM-INOUT-PLACES]** A mutating method works on any place, not only a
plain variable: `vm.stack.pop()`, `bag.items.set(k, v)`, `rows[0].xs.remove(i)`.
And an `inout` parameter crosses a module boundary like any other. Both were
compiled-backend gaps, and both mattered more than they sound — `inout` is the
answer [T-CAPTURE-WRITE] gives for threading mutable state, so its stopping at
a file edge reshaped a public API.

**[T-PARAM-INOUT]** An `inout` parameter is an exclusive mutable borrow.
**[T-MUTATE-PLACE]** A mutating method's receiver must be a *place* — a
binding, or a field or index reached from one. `[3, 1, 2].remove(0)` and
`mk().pop()` are not: there is nowhere for the mutation to land, and they were
accepted and then panicked at run time, one of them printing a
compiler-internal `Span { start: .., end: .. }` into the terminal. Writing to
a `let` binding was already a compile error, so the temporary was the one
receiver nothing checked.

**[T-INOUT-PLACE]** Its argument must be a variable, field, or index
expression — never a temporary. **[T-INOUT-MUTABLE]** That place must be
mutable. **[T-INOUT-EXCLUSIVE]** The same *place* may not be passed as
`inout` twice in one call, nor as both `own` and `inout`. Two places overlap
when one is a prefix of the other — `p` covers `p.xs` — and two different
fields of one value do not: `two(p.xs, p.ys)` is legal, and comparing roots
alone refused it, on the most ordinary shape a struct-of-collections has. An
*index* is treated as naming any element, because `g[i]` and `g[j]` may be the
same one and nothing here can tell.

**[T-INOUT-REF]** Nor may an `inout` argument be reached through a `ref`.
`inout` works by snapshotting the place at the call and writing it back at the
return; a reference reaches the same value without going through that
snapshot, so a write made through one *during* the call is overwritten by the
writeback and vanishes with nothing reported. `f(r.xs, r)` where `f` pushes to
both left `[0, 1]`, not `[0, 1, 2]`, and swapping the two pushes did not change
which one survived. The exclusivity rule above did not catch it, because it
compares binding names and `r.xs` and `r` are different names for the same
storage. The two ways of mutating are each sound and cannot be combined: write
through the reference, or hold the value in a `var` and drop the reference.

## 9. Effects

**[T-EFFECT-DECL]** `effect E { fn op(…) -> T }` declares an effect and, with
it, a handler struct type `EHandler` whose fields are the operations. One
operation per line. Operations take parameters, and a handler closure receives
them in declaration order; an operation with no `->` returns nothing. A handler
may be written at `handled_by` or bound to a name first, and its body may
perform effects of its own — a logger that prints performs `io`, which is the
usual case rather than an exception.

**[T-EFFECT-FALLIBLE]** An operation may declare an error type,
`fn get(k: str) -> f64 ! Diag`. Its result is then fallible like any other, so
a caller must consume it with `try` or a `match` and `try` requires the caller
to declare an error type of its own. A handler's operation produces the plain
value or `fail`s, exactly as a fallible function's body does.

**[T-EFFECT-ROW]** Every function has an inferred set of effects. A declared
row `/ {io, E}` is an upper bound: performing an effect outside it is an error.

**[T-EFFECT-BUILTIN]** `io` is performed by `print` and the filesystem
builtins. `async` is performed by `parallel`/`spawn`. Both are discharged by
the runtime at `main` and in tests.

**[T-EFFECT-RESERVED]** `io`, `async` and `unsafe` may not be declared as
effects. They are capabilities the runtime discharges, not effects a program
handles — there is no `with io handled_by`, the same point [T-UNSAFE-FFI]
makes for `unsafe`.

**[T-EFFECT-DISCHARGE]** `with E handled_by h { … }` discharges `E` inside the
block. Any user effect still outstanding at `main` or in a test is an error.

**[T-FN-VALUE]** A function's name used where a value is expected produces a
*function value*, and a closure may be bound to a name. Both are called with
ordinary call syntax and may be stored in a list together when their parameter
and result types agree.

**[T-FN-TYPE]** `fn(A, B) -> R` is the type of a function value, and may be
written wherever a type may be — most usefully as a parameter, which is what
lets a program declare its own higher-order function:

```
fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}
```

A closure passed to such a parameter takes its parameter types from it. A
closure bound to a name and never called, with no function type to take them
from, still has nothing to infer from and is an error.

**[T-FIELD-CALL]** A struct field of function type is called through the
field: `n.run(5)`. Building the value and reading the field back both worked
from the start and the call did not, so the only spelling that ran was
`let f = n.run` followed by `f(5)` — which nothing pointed to, while the error
suggested declaring a free `fn run(Node, ...)`, a different function that would
not call the field at all. A field shadows nothing: a real method or a UFCS
free function of the same name still wins, and a non-function field called this
way is still "no method".

The same holds for the two other things a field can be reached through. A
tuple's components are fields spelled with numbers, so `p.0(4)` calls the
function in the first position; the error used to be "no method `0`", with
help suggesting a free `fn 0(...)`, which is not a name that can be written.
And a `ref` is read through, so `c.f(3)` calls the field of the referent.

**[T-FN-TYPE-ROW-CARRIED]** The row is part of the **type**, so it is checked
wherever a function value meets a written type — a parameter, a return type, a
`let` annotation, a struct field, a list element, a map value, a tuple member,
a generic argument — and structurally, because a function reaches its
destination inside things. It is a *subset* rule: a function that performs
nothing fits `fn(..) / {io}`, and one that prints does not fit `fn(..)`.

This needed saying because it was not true. The row lived only in the syntax
someone wrote and never in the type, so `fn(i64) -> i64` and
`fn(i64) -> i64 / {io}` were the same type here and nothing could compare them;
one hand-written check covered direct call arguments and nothing else. Every
other route dropped the row, which made each of them a way to hand a printing
function to a `/ {}` row, to `par_map` past [T-PAR-MAP-PURE], to the
missing-handler backstop [E-HANDLER-MISSING] says well-typed programs never
reach, and to a package declaring `capabilities = []`.

That is the rule for a value *arriving* at a written type. Calling one that
arrived is the other half, and it was only carried by `let`. `let f = fs[0]`
charged `io` and `for f in fs` charged nothing — the same program, one keyword
apart — and so did a `match` payload, a tuple destructuring and a closure
parameter. Each of those is a `for` loop away from a printing function running
inside a `/ {}` function, on `par_map`'s workers, and in a package declaring
`capabilities = []`. A call through a local that no binder accounts for is
charged every row written in the *enclosing function's own signature* at a
position with no field name — its list elements, its tuple components, its
`Option` payloads. Scoped to that signature rather than to the program: the
wider pool charged `io` to a function whose own element type declares no
effects, on the strength of an unrelated declaration elsewhere in the file.

And the pool is reached for only where nothing is known about the part. Where
every binding the expression reads has a written type, *those* types say what
came out: a tuple component, a `??` between two known carriers, a block around
one. Each of those drew the pool anyway, so a `/ {}` body performing nothing
was refused on the strength of an unrelated parameter — including a parameter
whose own row is charged exactly, at every mention of it. Where both a field
name and the binding's own type are known, the answer is their
**intersection**: each over-approximates the true row, so the intersection
does too, and it is the tighter of the two.

An **empty** carried row is a fact, not the absence of one. A parameter written
`fs: [fn(i64) -> i64]` says its elements perform nothing, and reading one out
hands that on — but only non-empty rows were recorded, so `let f = fs[0]`
looked unaccounted and drew the whole pool: a sibling parameter whose own
*named field* holds an `io` function refused a `/ {}` body that performs
nothing, which is the shape this clause already records as fixed, one scope
level in.

Reading a part out of a binding hands on *that* binding's sources, and so does
copying one whole: `let f = step` is `step`'s row, not the pool. A plain named
parameter is not a position "with no field name", but the copy fell through to
the fallback anyway, so an unrelated `log: fn(str) / {io}` in the same
signature charged `io` to a call through a pure `step` — while `xs.map(step)`
written directly was accepted. The same program one `let` apart, again.

`let g =
fs[0]` fell through to the pool of every function value in the function, so an
unrelated closure elsewhere charged its effect to a call through `g` — while
the same program written `for g in fs` was accepted, which is the pair this
clause exists to close, back in the other direction. Where nothing is known
about the part, the fallback is this function's own signature and nothing
wider: reaching for every nameless row in the *program* meant an **unused**
`type Loud = fn(i64) -> i64 / {io}` — in this file or in a dependency — refused
a `/ {}` function whose own tuple component declares no effects.

A `type` name counts as the type it stands for, here as everywhere
([T-TYPE-ALIAS]). This is inferred from written syntax rather than from
resolved types, and a row behind a name was not there: `f: Cb` for
`type Cb = fn(i64) -> i64 / {io}` recorded an *empty* row, which then shadowed
every fallback. One `type` line laundered `io` past all four of the guarantees
above.

**[T-FN-VALUE-ROW]** Naming a function contributes its effect row to the
function doing the naming, whether or not the call is written there. Holding a
function is the permission to call it — the rule [T-EXTERN-VALUE] states for a
C function, and it holds for every function for the same reason.

*And for every name, not only a function's.* A binding spelled like an imported
module made `binding.field` take the *module function's* row: an `io` field
looked pure, so a printing function reached `par_map` and a `capabilities = []`
package; and a pure field was charged an unreachable private helper's `io`, so
a package that performs nothing failed to build in its consumer. A parameter
spelled like an **effect** demanded a handler for an effect the body never
performs. Both directions of one defect, and both are the rule below applied
to a name that is not a function's.

*And only when it is a function.* Scope decides at a **call** as well as at a
mention: the two arms disagreed, so a *parameter* spelled like a private helper
was charged that helper's row — a `capabilities = []` package broke in every
consumer, and `par_map`'s purity proof failed, over one parameter's spelling.
A local of that name is a different thing:
a parameter spelled like a private helper charged that helper's row, which
broke a `capabilities = []` package in every consumer and failed a `par_map`
purity proof over a closure parameter's spelling. Scope decides, here as
everywhere else.

*However it is named.* `lib.shout` is the same function `shout` would be and
carries the same row, and it did not: only the bare spelling was charged, so
one `import` emptied the row and with it every rule that rests on it —
`/ {}`, [T-PAR-MAP-PURE], [E-CONST]'s constant table, [T-REGION-FOREIGN],
[E-HANDLER-MISSING]'s "well-typed programs never reach it", and a package
declaring `capabilities = []`, which `grasm deps` then certified as performing
nothing while it printed. A capability ceiling one `import` away from
meaningless is not a ceiling.

This is deliberately conservative: storing an effectful function and never
calling it is refused, and the way out is to declare the row. The alternative
was worse. A built-in higher-order method's function parameter is
unconstrained by design, since `xs.map(|x| print(x))` is ordinary code — so a
*named* effectful function passed to one had no row to check against, and
`[1, 2].map(shout)` inside a `/ {}` function typechecked and printed. One hop
further, `let fs = [uses]` and then `fs.map(call)`, went past `par_map`'s own
purity check as well: the effectful function is never called in the source,
only mentioned, and something else calls it. That reached through `/ {}`,
[T-PAR-MAP-PURE], the [E-HANDLER-MISSING] backstop, and a package's
`capabilities = []` ceiling.

That covers a named function. A function value that arrives as a *parameter*
carries its row on its written type instead, and mentioning one is the same
permission as mentioning a name: `xs.map(f)` where `f: fn(i64) -> i64 / {io}`
performs `io`, exactly as `f(x)` does. Only the direct call was charged, so
`map`, `fold`, `filter`, `any`, `all`, `find` and `sort_by` each carried an
effect past a declared row. `par_map` was checked, which is what made the hole
look closed.

A declared C function has no body to infer a row from and what it does is
unknowable, so its row is `{io, unsafe}` — and that travels with the value.
Without it, pushing `sqrt` into a `[fn(f64) -> f64 / {io}]` inside an `unsafe`
block was accepted, and calling it back out ran libc from a function declaring
nothing at all.

**[T-FN-TYPE-ROW-PARAM]** A row in a *parameter* position is checked the
other way round: what is handed over must accept everything the expected type
promises to give it. Checked covariantly with the result, a `call` declaring
`fn(i64) -> i64` accepted a list of `/ {io}` functions, and
`fs.par_map(call)` then ran the effect on several cores — [T-PAR-MAP-PURE]'s
"a compile error, not a race", reached by giving the row to a parameter rather
than to the callback itself.

**[T-FN-TYPE-ROW]** A function type carries its effect row explicitly:
`fn(i64) -> i64 / {io}`. **No row means the empty row**, so `fn(i64) -> i64`
accepts only functions that perform nothing, and passing one that prints is an
error at the call site. This is the only place a row can be checked rather than
inferred — a written type has no body to infer from — and without it a function
value would be a way to carry an effect past a declared row.

A closure literal is checked the same way a named function is.

"The same way" is about the *body* too: a named function's is checked against
its declared return, and a closure's was merely inferred. So the expected type
never reached the literals in its tail — `|x| 3` against `fn(i64) -> f64` was
refused while `fn named(x: i64) -> f64 { 3 }`, the same body one closure apart,
was accepted, and so was `|x| { return 3 }`. That is the asymmetry
[T-INT-TO-FLOAT-CARRIED] records as fixed for named functions, still present
one shape in.

**[T-OK-ERR]** `Ok(v)` and `Err(e)` build a `T ! E` directly. They are the
constructors matching what `match` already destructures, and without them a
plain value could not be *put into* a fallible slot at all: `fail` is legal
only inside a fallible function, so a program storing per-item results needed
a trampoline whose whole body was `v` and another whose body was `fail(e)`.

**[T-OK-ERR-RETURN]** A fallible function's body may produce either the ok
half — the ordinary case, wrapped in `Ok` on the way out — or the whole
`T ! E`, which is what `Ok(v)` and `Err(e)` build. The second was rejected,
so the constructors worked everywhere a fallible value could be *stored* and
nowhere one was *produced*.

The wrapping happens wherever the branch lands, not only on the way out of a
function. A fallible function re-wraps its whole body, which masked a gap
everywhere except the positions that do not — a `let` annotation, a struct
field, a list element — where the interpreter stored the *raw* value:
`let a: i64 ! E = if true { 1 } else { Ok(2) }` held `1` rather than `Ok(1)`,
so it printed as `1`, compared unequal to `Ok(1)`, and matched no arm of a
`match` over it. A native build, which wraps by produced type, answered
correctly — the normative engine was the wrong one, at exit 0 with nothing on
stderr.

Nesting does not change the position. A branch whose own body branches is at
the fallible position too, so `if a { if b { 1 } else { Ok(2) } } else { Ok(3) }`
merges to `i64 ! E` exactly as the flat form does; the expected type reached
only the *immediate* expression, so the same computation was accepted written
with `match` on the outside and refused written with `if`, one keyword apart.

This is a rule about the *position*, not about how it was reached: an
annotation naming `T ! E` — a `let`, a struct field, a parameter — accepts the
same two shapes per branch, and demanding the whole type of every branch made
`let v: f64 ! E = if c { 3.0 } else { Ok(4.5) }` a type mismatch while the same
expression without the annotation was an unhandled failure whose help says to
annotate it. Two diagnostics pointing at each other is no way to write the
program at all.

`return` takes either half too, including through a branching expression:
`return if c { 3 } else { Ok(4) }` is the two shapes in one statement. Only the
tail expression was taught this, so `return Ok(1)` was "this expression can
fail but `i64` is required" — a message about the wrong problem, in the shape
early exits are written in, and the workaround was to rewrite every one of them
as `fail`. Deciding by *how the value was written* rather than by what it
turned out to be left the same hole one level out: an `if` whose branches
merged to the whole `T ! E` was wrapped a second time on the way out, and the
caller printed a heap address.

The two halves mix *within one expression*: an `if` may give the bare ok half
in one branch and the whole `T ! E` in another, and a `match` may do the same
across its arms. What the expression is worth is the wider of the two — the
whole `T ! E` — and the branches giving only the ok half are the ones wrapped.
Taking the type from whichever branch came first instead meant
`if n == 1 { 10 } else { Ok(n * 3) }` was typed `i64`: the `Ok` branch's value
travelled as a machine word and was wrapped a second time on the way out, so a
caller that matched `Ok(v)` printed a heap address where the number was.

**[T-FALLIBLE-VALUE]** A function's `! E` is part of what it answers with, so
it travels with the function *as a value*: `risky` referenced by name has type
`fn(A) -> T ! E`, not `fn(A) -> T`. Passing it where a plain function is
wanted is a type error.

`map` and `par_map` are not that case, and the difference is deliberate: they
place no demand on what the function returns, so `xs.map(risky)` is
well-typed and produces `[T ! E]` ([G-TYPE-FALLIBLE]) — every failure kept,
none of them silently dropped. `filter`, `par_filter`, `any`, `all`, `find`
and `sort_by` each need a specific answer — a `bool`, an ordered key — and
`Ok(true)` is not `true`, so those reject it.

This is the rule that stops [T-FALLIBLE-UNHANDLED] from being sidestepped.
Without it `xs.filter(risky)` typechecked, and then filtered on `Ok(true)` —
which is not `true` — so the answer was the empty list, with nothing reported
and exit code 0. The same door made `any` always `false`, `find` always
`None`, and `sort_by` leave the list untouched.

**[T-FALLIBLE-UNIT]** `() ! E` is a fallible type like any other. A function
that produces nothing can still fail; `try f()` on one yields `()`, and
`f() ?? ()` is how a caller says the failure is deliberately ignored. The
empty ok half changes nothing about where the failure has to be handled
([T-FALLIBLE-UNHANDLED]) — it is the ordinary shape of a setup call, and the
one a compiled backend is most likely to get wrong, because the value it
merges the two halves through does not exist.

An effect operation declared `-> () ! E` is the same shape one hop further
out, and the position with no arm for it at all: a handler whose body gives
the bare `()` half in one branch and `fail(e)` in the other produced no value,
and the lifted body returned a null pointer typed as a fallible. The build
succeeded and the program died with SIGSEGV and nothing on stderr — the one
outcome §11 promises cannot happen, since a backend that cannot compile
something names it at build time.

**[T-DIVERGE]** `return`, `break` and `continue` produce no value, so they do
not have to agree with one: an `if` branch or a match arm that ends in one is
compatible with any other branch's type. This is what makes `0 -> return
false` legal next to an arm that yields a `bool` — reading the returning arm
as `()` put the error on the *other* arm.

They are **statements**, not expressions, so unlike `fail` they cannot appear
as a sub-expression: `o ?? return -1` and `n.max(return 0)` do not parse.
`fail(e)` can, and is the form to reach for on the right of `??`
([E-COALESCE-DIVERGE]).

**[T-CAPTURE-WRITE]** Assigning to, calling a mutating method on, or passing
as `inout` a binding a closure captured from outside is a compile error. The
write would change the closure's own copy ([E-CLOSURE-CAPTURE]) and be
discarded when the call returns, so there is no program for which it is what
the author meant. A closure's own `var` bindings and its parameters are
unaffected.

The `inout` case is the one worth naming separately: the call *runs*, and the
callee's writes land in a copy the caller cannot read, so the mistake shows up
as a counter that stays at zero rather than as anything failing.

**[T-PAR-MAP-PURE]** The function given to a parallel operation — `par_map` or
`par_filter` — must have an empty effect row. Elements are processed concurrently, so an effect would interleave
differently between runs; requiring purity is what makes the parallelism
deterministic. This is a compile error, not a race.

*And a block hands its tail through.* `let f = { loud }` is the same function
value `let f = loud` is, and `unsafe { sqrt }` is the same C function a bare
mention names ([T-EXTERN-VALUE]) — but a block recorded no source at all, not
even an unaccounted one, so the proof saw an empty row and `print` really did
run on the workers. One pair of braces from the spelling that was refused.

*And it may not write through a `ref` it captured.* An empty row is not the
whole of purity: mutation is not an effect, so nothing in the row system saw
`xs.par_map(|x| { acc.n = acc.n + 1; x })` — every element's call writing the
same slot, which is exactly the interleaving the rule promises cannot happen.
The write a closure is otherwise allowed to make ([T-REF-CAPTURE]) is the one
that has to be refused here. A `ref` created *inside* the function is one per
element and is left alone, as is reading a captured one.

*And it may not reach a `ref` from outside at all.*

So an **element type that reaches a reference** is refused at the operation —
`[ref T]`, or a struct holding one anywhere inside — because every element is
then the shared thing and there is no outside binding to point at. A reference
*created inside* the function is one per element and is left alone, which is
the shape that makes a per-element accumulator writable.

A type **parameter** in that position says nothing yet, so it is recorded and
settled at the call sites — the same treatment [T-MAP-KEY-GENERIC] gives a map
key, and needed for the same reason: `fn go[T](xs: [T], f: fn(T) -> i64) {
xs.par_map(f) }` hid `[ref Acc]` from the check entirely, which is a door
straight back to the race. It propagates through a generic that forwards to a
generic.

*And a name carries what the closure behind it reaches.* `let f = |x| { acc.n =
1; x }` and then `xs.par_map(f)` was the last shape to reach past this rule, on
the argument that the native backend refused that spelling for an unrelated
reason and both other engines map sequentially. That is the same argument the
paragraph above made, right until it stopped being true — so the closure is
recorded where it is written and the name is refused where it is used. Settled
at the end of the analysis, because a closure whose parameter types nothing
fixes has its body checked after the `let` that binds it, and sometimes after
the very call that fixed them.

**[T-TASK-CAPTURE]** A `spawn` body may only use bindings that already existed
when the enclosing `parallel` block began, plus the ones it declares itself.
Tasks run *after* the block finishes, so a binding declared inside it — a loop
variable included — no longer exists by then. This is a compile error rather
than a runtime failure.

One consequence is worth stating outright, because it is the first thing
somebody tries: **`for x in xs { spawn { … x … } }` is not writable.** The
number of tasks a `parallel` block starts is fixed where it is written, and
each task's body reads the enclosing scope as it stands when the tasks run —
not as it stood when that `spawn` was reached.

That is a division of labour, not an omission. Work *per element of a list* is
`par_map` / `par_filter`, which is where a runtime-sized amount of parallelism
belongs and which enforces purity so the answer cannot depend on scheduling
([T-PAR-MAP-PURE], [E-PAR-MAP-ORDER]). `parallel`/`spawn` is for a fixed set
of distinct concurrent activities that share results through the enclosing
scope ([E-SPAWN-SHARED]).

## 10. Modules

"Fixed where it is written" means each `spawn` *statement* starts one task
each time it is reached — not that the count is a constant. A `spawn` inside a
`while` in the same block starts one per iteration, so the number can depend on
a runtime value; what it cannot do is depend on a binding made inside the
block, which is what the capture rule refuses.

**[T-TYPE-ALIAS]** `type Name = T` gives a type another name; it is not a new
type, and the two are interchangeable everywhere. An alias may name any type,
including a function type, a map, and a fallible one — and when a function's
declared return type resolves to `T ! E`, the function is fallible even though
its declaration does not spell the `!` out.

**[T-EXTERN-POINTER]** An `extern fn` **parameter** may be `i64`, `f64`, `str`
or `bytes`. A `str` or `bytes` argument crosses as a pointer:

```grasm
extern fn strlen(s: str) -> i64
extern fn memcmp(a: bytes, b: bytes, n: i64) -> i64
```

Without this the C boundary carried scalars only, which excluded `open`, `read`,
`write`, `system`, `getenv`, `socket` — every libc function taking a pointer,
which is nearly all of the useful ones. What could be called was `abs`, `sqrt`,
`pow` and their kind, so "C ABI import overhead: zero-cost" was true and
described arithmetic.

A `str` crosses as a **NUL-terminated copy**. Grasm strings carry their length
and are not terminated; C decides where a string ends by looking for the NUL,
so handing over the raw bytes would have the callee read past the end of the
buffer. A string containing a NUL byte panics rather than being silently cut
short at it — the two engines say the same words, because the compiled build
sees a pointer and not a declaration.

`bytes` crosses as a pointer to the buffer itself, with no copy and no
terminator: it is already contiguous, it is not text, and appending a NUL would
change its length. **How long it is stays the program's to pass**, because C has
no way to ask — `memcmp(a, b, a.len())`, not `memcmp(a, b)`.

The pointer is valid **for the duration of the call** and the buffer belongs to
Grasm. That is the whole reason this is expressible: nothing about ownership has
to be decided at the boundary.

**A pointer may not come back.** A returned `str` or `bytes` is refused, and the
asymmetry is the point: nothing about a returned pointer says who owns it, how
long it lives, or how long it is. An `extern fn` returns `i64`, `f64` or
nothing.

None of this makes the boundary safe, and none of it is meant to. C may write
through a pointer it was handed, and Grasm cannot see that. This is why a
declared C function's row is `{io, unsafe}` and every use sits inside `unsafe`
([T-EXTERN-VALUE]) — the capability was already there, and widening what may
cross did not widen what a package can hide.

**[T-EXTERN-VALUE]** A C function may be named without being called, the way
any function may. Naming one carries the same `io` a call does: holding a
reference to it *is* the permission to call it, so a bare mention counts
against a package's capability ceiling and disqualifies a function from
`par_map`. Without that rule, `xs.par_map(sqrt)` typechecked and the ceiling
had a door straight through it.

**[T-EFFECT-OP-RESULT]** An operation's declared *result* carries its row like
any other written type. [E-HANDLER-OWN-ROW] exempts the handler's own
operations from the installing function's row; it says nothing about the value
an operation hands back. An operation answering
`-> fn(i64) -> i64 / {io}` gave out a printing function that nothing charged,
so a function declaring `/ {Mk}` printed, `par_map`'s purity proof passed, and
a package permitting only `Mk` reached `io` — while the same signature written
as an ordinary `fn` was refused, one keyword apart.

**[T-CAPABILITY]** A package's manifest may declare `capabilities`: the set of
effects it is permitted to perform. What counts against it is what *escapes*
the package — the effects `main` performs, the effects left in a `pub`
function's row for a consumer to handle, and the effects its own `test`, `bench`
and doc-example bodies perform, since a consumer's `grasm test` runs those too.
A dependency's `pub` rows do not count toward its consumer, whose ceiling is
what it needs rather than what everything it depends on offers, and a
handler's effects are counted where the handler was *written*
([E-HANDLER-OWN-ROW]).

Omitting the key means **unrestricted**, not empty. `capabilities = []` says
"performs nothing"; leaving it out says "do not check me". That difference is
why a key or table the manifest does not recognise is refused rather than
ignored — a typo would otherwise spend the whole guarantee silently.

The package is the unit on both sides of that boundary. Its sources are every
`.gr` file under the package root — not only those beside the entry file, since
a `tests/` or a `src/deep/` directory is as much the package as `src/` is — and
its rows are inferred with only its own handlers in scope, so a library taking
a handler from its consumer performs nothing of its own. Charging every handler
in the loaded set to every module made that shape a build error naming a
package that permits the effect; enumerating from the entry's directory let a
`capabilities = []` package print from a file the checksum and `publish` both
shipped.

A package that exceeds its ceiling fails to build **in every consumer**, not
only in `grasm deps`: because effects are inferred from source rather than
declared by hand, a dependency that quietly starts reading the filesystem is a
compile error wherever it is used, and not something a reviewer has to happen
to notice. This clause was cited by others for some time before it was written
down, which for the rule the security story rests on is the wrong way round.

**[T-CONST-PUB]** `pub let` exports a constant; a bare `let` keeps it to its
own module, like a function. Reaching a private one through its module is the
same error a private function gives.

**[T-REF-GENERATIONAL]** `ref T` is Tier 2: a *generational reference* into a
shared heap. `ref(v)` puts a value there and hands back a reference — a slot
index paired with the generation that slot was on. Reading a field through one
reads the value it points at. A reference is a plain word: it copies, and it
can be stored in a struct, a list or a map, which is exactly what Tier 0's
second-class references cannot do and the reason this tier exists. A cyclic or
shared object graph has no other safe representation here.

`release(r)` gives the slot up and `alive(r)` asks whether a reference still
points at something. Releasing an already-released slot does nothing: a double
release is not a use-after-free, and making it an error would turn cleanup
order into something every caller has to track.

Reclamation is manual: a slot returns to the pool when `release` is called on
a reference to it, and not otherwise. Four hundred thousand un-released `ref`s
hold about 180 MB. Nothing collects them, which is the point of the tier — the
lifetime is the program's to state — but "reused immediately" describes what
happens *after* a release, not a reason to skip one.

**[T-REF-VALUE]** `r.value` reads and writes what a reference points at, when
that has no fields of its own:

```grasm
let n = ref(0)
n.value += 1
print("{n.value}")        // 1
```

Without it `ref(0)` typechecked, produced a `ref i64`, and **no expression in
the language could observe it** — a value that could be constructed and never
used. The workaround was a one-field wrapper struct, which is the same thing
with a name on it.

It is *not* offered when the referent is a struct or a tuple. A struct already
has field names, and `.value` would either shadow a real one or change meaning
the day somebody added one; a tuple's components are `.0` and `.1`. So the rule
is exactly "the referent has no way to name its parts, so this names the whole
thing", and there is no case where both readings apply.

`.value` consumes no more of a path than any other dereference: `r.value` is
the referent, and `r.items.push(x)` still reaches through to a struct field
without it. Writing through it lands **in the slab**, so every other holder of
the reference sees it ([T-REF-GENERATIONAL]) — the same guarantee as writing to
a field, and the reason a `ref i64` is a shared counter rather than a copy.

**[T-REF-EQ]** Two references are equal when they name the same slot *and the
same generation* — identity, not the equality of what they point at, which is
what a graph walk needs to recognise a node it has already visited. Equality
recurses, so this is also what makes a struct or list *containing* a reference
compare sensibly.

Liveness is not consulted, and does not need to be: releasing a slot bumps its
generation, so a stale reference is never equal to a live one that reuses the
same slot. Two copies of the same released reference *are* equal, which is
what identity means — a value is equal to itself, and a rule that said
otherwise would make `a == a` depend on when it was asked.

**[T-REF-MATCH]** A `match` on a reference matches what it points at, so a
`ref` to a sum type is opened the ordinary way. Without it the two diagnostics
formed a closed loop — a field access said to use `match`, and the match said
the pattern could not match a `ref` — which excluded the recursive shapes an
AST or a graph node is made of, and those are the case this tier exists for.

**[T-REF-WRITE]** A reference is written through the same way it is read
through: `r.field = v`, `r.items.push(x)`, `r.n += 1`. The binding's own
mutability is not the question — writing through a reference does not modify
the reference — so `let r = ref(...)` is the ordinary form and `var` is not
required.

Without this a `ref` graph was **write-once**: closing a cycle needs one write
after both nodes exist, and that write typechecked and then panicked. The
compiled backend refused the shape for a while and no longer does — it writes
into the slab rather than rebuilding the referent, which is what a cycle
requires, and §11 no longer lists it. This paragraph said otherwise for a day
after it stopped being true, which is the failure mode a documented gap has:
the table is checked against the compiler on every run, and prose beside it is
not.

Anywhere on the path, not only at the root. A node held in a list or in a
struct field is the ordinary Tier 2 arena — `[T-REF-GENERATIONAL]` names
storing a reference in a struct, a list or a map as the reason the tier
exists — and `rs[0].v = 5` typechecked and then panicked `cannot access field
'v' of 'ref#0.0'`, while reading the identical path worked. The container's
mutability is not the question either, for the same reason the root's is not:
a `[ref N]` parameter cannot be declared `var` at all, so the demand had no
answer.

*Anywhere* means through more than one, too. A graph edge is a reference held
by a node that is itself reached through one, so `a.out[0].name = "B!"` — and
every write that closes a cycle — crosses two. The write descended into one
slab entry and panicked with the same message the moment it met the next, so a
node holding a reference to a node was still write-once, which is every graph
the tier exists for. A path is now followed reference by reference, each one
checked for staleness as it is crossed, up to a bound no hand-written path
reaches.

**[T-REF-CAPTURE]** A captured `ref` may be written through inside a closure.
[T-CAPTURE-WRITE] refuses writing to a captured binding because a closure
captures *values*, so the write would land in a copy nobody can see — a
reference copies as a slot rather than as a value, so a write through one is
visible to everyone holding it and the reason does not apply.

This is what lets an effect handler accumulate. Without it the obvious spelling
was refused and so was the reference that answers it, leaving `inout` on an
effect operation as the apparent last option — which typechecked and silently
discarded the write ([E-EFFECT-NO-INOUT]). A feature with no working spelling
sends people somewhere worse.

**[T-REF-STALE]** Reading through a released reference **panics**, always. The
slot may already hold something else — a released slot is reused by the next
`ref`, which is what keeps a long-lived graph from growing without bound — so
the
generation is what distinguishes a live reference from one into the slot's
previous life. This is the property that makes the tier safe rather than merely
convenient: a stale read is caught every time, never undefined behaviour, and
never a silently wrong value.

**[T-REGION]** `region { ... }` is Tier 1: a scope **nothing allocated inside
may outlive**. Three rules give that. It may not write to a binding declared
outside it — by assignment or by a mutating method, since `outer.push(x)` is
how a value would otherwise get out. It may not `return` through the brace. And
it yields a scalar or nothing, because a scalar is not an allocation.

What it buys today is the guarantee itself: the peak memory of the block is the
block, and that is checked rather than hoped for. What it does *not* yet buy is
the bulk deallocation the guarantee exists to make sound — the compiled runtime
is reference-counted, and freeing a region's allocations in one sweep needs an
allocator that does not individually free, which is a change to the runtime
rather than to this rule. The rule comes first because the sweep is only
correct if the rule holds.

**[T-REGION-ESCAPE]** The escape rule is enforced against *writes*, not only
against assignment: `outer.push(made_here)` is refused for the same reason
`outer = made_here` is. A rule about assignment alone would miss the case
people actually write.

**[T-REGION-INOUT]** And against writes made *through a call*: passing a
binding declared outside the region as `inout` is refused, because the callee
can store into it. `fail` is refused for the same reason `return` is — it
leaves the region carrying a value, and that value is usually built right
there — and `try` for the same reason again, being a conditional `return` of
an `Err` whose payload is normally allocated where the failure happened. The rule was stated for direct writes first, and two independent
evaluations found both holes within an hour, which is what a rule expressed in
terms of syntax rather than of effect gets you.

**[T-REGION-REF]** A `ref` may not be used inside a region at all. A reference
reaches storage the region cannot see; it needs no `inout`, so the rule above
never fires, and it can be copied to a name declared *inside* the region, so
the declared-outside rule never fires either. Two independent evaluations
carried a value out through one — in one line, four different ways. Since a
region's entire guarantee is that nothing allocated inside outlives the brace,
and a reference is the one thing that can carry something out without naming
anything outside, a region and a reference do not mix.

**[T-REGION-FOREIGN]** A region may not run code written outside it that
carries state: an effect operation, whose handler is written outside and may
keep what it is given, and a call through a function *value* bound outside,
which is a closure and carries its captures. However the call is spelled: the
row was read under the *calling* module, so `region { lib.cfg_user() }`
carried none and was accepted, while the same function called unqualified was
refused. A closure written *inside* the
region is fine — it can only capture what a region already allows. This is the
same rule as the one above seen from the other side: the earlier rules ask
what a region's own text may write, and this one asks whose text runs there.

The rule is about what a call can *reach*, not about where the text sits, and
that distinction is the whole of it. Each region rule was written first
against the syntax between the braces, and a one-hop wrapper walked through
every one: a helper that performs the effect, a struct holding the reference
one field deep, a struct holding the closure. So a call made from inside a
region may not perform a declared effect, and may not be handed a function
value — `io`, `async` and `unsafe` are exempt, because they have no handler
and retain nothing, and a closure written inside the region is exempt, because
it can only capture what a region already allows.

"Retains nothing" is about *memory*, which is what a region governs. Writing a
region's contents to a file with `write_file` is allowed and is not a hole:
what leaves is a copy of the bytes, and nothing holds a reference into the
arena past the closing brace. The rule exists so no live pointer outlasts the
region, not to make what a region computes secret. `unsafe` is the one exempt
row where retention is possible — a C function can keep whatever address it is
given — and that is Tier 3's bargain, stated in [T-UNSAFE-FFI] rather than
weakened here.

**[T-UNSAFE-FFI]** `unsafe { … }` is Tier 3: the block where the guarantees of
the safe tiers do not apply. Using a declared C function — calling one *or
naming one*, which are the same permission — is allowed only inside it.

An `extern fn` may not be `pub`. A C function is a link-time symbol of the
package that declares it, and exporting the name carried it past every gate
that makes Tier 3 visible: `libext.getpid()` in another module needed no
`unsafe`, charged no capability, panicked under the interpreter with `module
has no function` and printed an answer from a native build — three engines,
two of them wrong, over a symbol the consumer's manifest never mentioned. A
package offers a C function by wrapping it: `pub fn root(x) { unsafe { sqrt(x)
} }`, which is the spelling that shows up in the ceiling.

The block is not merely syntax. An `unsafe` block puts `unsafe` in the
enclosing function's effect row, so it reaches the package's capability ceiling
by the same path every other effect does, and a package containing one must
declare `capabilities = [..., "unsafe"]` or fail to build. That declaration is
in the manifest, which every consumer can read, transitively: a library cannot
quietly contain an `unsafe` block, and a consumer can refuse to depend on one
that does. `unsafe` is a capability rather than a handleable effect — there is
no `with unsafe handled_by`, the same as for `io`.

**[T-UNSAFE-UFCS]** The `unsafe` requirement applies to `x.f()` exactly as to
`f(x)`, because [L-UFCS] makes them the same call. Only the second spelling was
gated: `n.close()` reached libc with no `unsafe`, no `io` in the row and no
capability, and a package declaring `capabilities = []` was reported as
`performs []` while its compiled binary called C. `unsafe` is also writable in
an effect row — `/ {io, unsafe}` — which it was not, being a keyword.

Both halves, at that. Gating `unsafe` on `x.f()` and leaving `io` behind
turned the same defect down by one bit rather than closing it: `n.labs()`
inside an `unsafe` block charged nothing while `labs(n)` charged `io`, so a
package declaring `capabilities = ["unsafe"]` was reported as reaching no `io`
while its binary called libc. Every other spelling — `labs(n)`, `n |> labs`,
`let f = labs` then `f(n)`, `[n].map(labs)` — was charged.

**[T-EXTERN]** `extern fn name(a: i64) -> f64` declares a C function. It has
no body: a compiled build resolves the symbol at link time, and `grasm run`
finds it in the interpreter's own process. Parameters and results are `i64`
and `f64` only — `bool` is excluded because C's is one byte and Grasm's is a
register, and a width mismatch at an FFI boundary surfaces as garbage far from
its cause.

**Calling one performs `io`.** What a C function does cannot be known, so the
conservative row is the only sound one: an `extern` call counts against a
package's capability ceiling and disqualifies a function from `par_map`
([T-PAR-MAP-PURE]). Without that, FFI would be a hole straight through both
guarantees — and it was, in the first version of this feature.

**[T-MODULE-NS]** Functions are per-module; types and effects share one global
namespace, so a type name may not be declared twice across a program — and an
*effect* is one of the things that claims a name, two of them in fact: `effect
Clock` declares `Clock` and `ClockHandler`. Both were free to collide with a
type, and both names stayed usable afterwards. One
namespace means one namespace: an effect operation's signature may name a type
from any module, the same as an ordinary function's. A *type alias* is a type
declaration like any other, and it was the one that escaped both this rule and
[T-TYPE-NO-PUB]: aliases resolve a pass after struct and sum bodies, so
nothing had registered the first `type Id = i64` when the second was
collected, and two declarations of one name checked clean with the later one
silently winning.

**[T-TYPE-NO-PUB]** It follows that `pub` on a type is meaningless, so it is
**rejected** rather than accepted and ignored. A type is reachable from every
module whether or not the word is there; accepting it would read as a
permission being granted, which is the defect [T-CONST-PUB] records for
`pub let`.

**[T-MODULE-PUB]** Only `pub` functions are callable from another module, and
the same rule governs using one as a *value*: `xs.map(utils.helper)` reaches
exactly the function `utils.helper(x)` would, and a private one is refused the
same way.

**[T-MODULE-RESOLVE]** `import a.b` names the module in `a/b.gr`, relative to
**the source directory of the package the importing file belongs to** — a
dependency's imports resolve inside that dependency, not in whatever consumer
happens to be building it. Otherwise a file at the same relative path in the
consumer silently replaced the library's own private module, and the program
ran and printed the wrong answer. The last segment is the qualifier
calls use, so `import lib.math` is called as `math.f(...)`. Execution begins in
the file passed to the toolchain, which is the only module required to define
`main`.

**[T-MODULE-DEP]** A package may also import a dependency declared in its
`grasm.toml` by that dependency's name, which resolves to the dependency's
entry file. A package is a directory containing the manifest and a source tree; the entry
file is `src/main.gr` unless `[package] entry` names another path relative to
the manifest. An import of a dependency resolves to that entry file, and a
dotted import into a dependency (`import mathlib.helpers`) resolves relative to
the directory containing it.

`import mathlib` after

```toml
[dependencies.mathlib]
path = "../mathlib"
version = "^0.2.0"        # optional
```

makes its `pub` functions available as `mathlib.f(...)`. A dependency is a
library and needs no `main`.

`version` is optional beside a `path`, and required without one. `=X.Y.Z`
accepts that version alone; `^X.Y.Z` — which a bare `X.Y.Z` also means —
accepts it or any later version that does not change the leading non-zero
number, so `^1.2.3` stops below `2.0.0`, `^0.2.3` stops below `0.3.0`, and
`^0.0.3` accepts only `0.0.3`. A name resolves to exactly one package:
reaching one name through two different directories is an error, not a choice
the toolchain makes on the program's behalf.

A dependency with a `version` and no `path` comes from the **registry**, a
directory of `<name>/<version>/` packages located by `GRASM_REGISTRY` or the
nearest enclosing `.grasm/registry`. Versions are chosen by **minimal version
selection**: for each package, take the highest floor any requirement stated,
select the lowest version the registry holds at or above it, and then check
that selection against every requirement. There is no search — requirements
that cannot all hold are an error naming each requirer, so the selected set is
a function of the manifests alone and does not depend on the order the graph
was walked.
