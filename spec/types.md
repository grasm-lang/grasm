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
`invalid map key`. A type parameter reaching a map *only* inside the body,
mentioned in no part of the signature, is still not caught this way.

**[T-TUPLE]** `(A, B, …)` is a fixed-size heterogeneous product.

**[T-STRUCT]** `type P = { x: T, … }` declares a named product type whose
fields are accessed with `.`.

**[T-SUM]** `type S = | A(f: T) | B` declares a named sum type; values are
built with variant constructors and consumed with `match`.

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

Order does not matter. A list, a map's values, and a `match`'s arms are
checked with their integer literals *last*, so whatever fixes the type does it
first: `[1, 2.5]` and `[2.5, 1]` are both `[f64]`. Checking left to right made
the first element decide, which is the asymmetry `[E-NUM-PROMOTE]` records as
fixed for comparisons, still present one level up.

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

**[T-FAIL]** `fail(e)` raises the error branch. Its type fits any context, and
it is a compile error where no error type is declared.

## 6. Exhaustiveness

**[T-MATCH-EXHAUSTIVE]** A `match` must cover every possible value. Coverage
is checked for sum types, `bool`, `Ok`/`Err`, and products of booleans; other
scrutinee types require a `_` arm.

**[T-MATCH-NESTED]** A pattern's *payload* counts too. `Wrap(A)` matches only
the `Wrap`s carrying `A`, so it does not on its own cover `Wrap`.

This used to not be checked at all: a payload pattern was recorded as covering
its whole variant, so `match o { Wrap(A) -> … Plain -> … }` compiled and
panicked on `Wrap(B)`. The same held for an integer payload, an `Option`
payload and an `Ok`/`Err` payload. That is precisely the runtime surprise the
clause above exists to prevent.

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
mutable. **[T-INOUT-EXCLUSIVE]** The same variable may not be passed as
`inout` twice in one call, nor as both `own` and `inout`.

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

**[T-FN-VALUE-ROW]** Naming a function contributes its effect row to the
function doing the naming, whether or not the call is written there. Holding a
function is the permission to call it — the rule [T-EXTERN-VALUE] states for a
C function, and it holds for every function for the same reason.

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

**[T-FN-TYPE-ROW]** A function type carries its effect row explicitly:
`fn(i64) -> i64 / {io}`. **No row means the empty row**, so `fn(i64) -> i64`
accepts only functions that perform nothing, and passing one that prints is an
error at the call site. This is the only place a row can be checked rather than
inferred — a written type has no body to infer from — and without it a function
value would be a way to carry an effect past a declared row.

A closure literal is checked the same way a named function is. It was not:
only the shape was compared when a closure met a written function type, so
`Box{ f: shout }` was refused and `Box{ f: |n| shout(n) }` was accepted — the
same function, one closure apart. Every consequence followed from that one
gap: a `/ {}` function printed, `[T-PAR-MAP-PURE]`'s compile-time proof
passed, a C function reached libc from a declared-empty row, a closure
carrying a handled effect left its `with` block inside a list, and a package
declaring `capabilities = []` exported one.

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

`return` takes either half too. Only the tail expression was taught this, so
`return Ok(1)` was "this expression can fail but `i64` is required" — a
message about the wrong problem, in the shape early exits are written in, and
the workaround was to rewrite every one of them as `fail`.

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

**[T-TYPE-ALIAS]** `type Name = T` gives a type another name; it is not a new
type, and the two are interchangeable everywhere. An alias may name any type,
including a function type, a map, and a fallible one — and when a function's
declared return type resolves to `T ! E`, the function is fallible even though
its declaration does not spell the `!` out.

**[T-EXTERN-VALUE]** A C function may be named without being called, the way
any function may. Naming one carries the same `io` a call does: holding a
reference to it *is* the permission to call it, so a bare mention counts
against a package's capability ceiling and disqualifies a function from
`par_map`. Without that rule, `xs.par_map(sqrt)` typechecked and the ceiling
had a door straight through it.

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
compiled backend refuses the shape by name for now (§11), so it is one of the
places the two engines differ in what they accept rather than in what they do.

Anywhere on the path, not only at the root. A node held in a list or in a
struct field is the ordinary Tier 2 arena — `[T-REF-GENERATIONAL]` names
storing a reference in a struct, a list or a map as the reason the tier
exists — and `rs[0].v = 5` typechecked and then panicked `cannot access field
'v' of 'ref#0.0'`, while reading the identical path worked. The container's
mutability is not the question either, for the same reason the root's is not:
a `[ref N]` parameter cannot be declared `var` at all, so the demand had no
answer.

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
which is a closure and carries its captures. A closure written *inside* the
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

**[T-UNSAFE-FFI]** `unsafe { … }` is Tier 3: the block where the guarantees of
the safe tiers do not apply. Using a declared C function — calling one *or
naming one*, which are the same permission — is allowed only inside it.

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
namespace, so a type name may not be declared twice across a program. One
namespace means one namespace: an effect operation's signature may name a type
from any module, the same as an ordinary function's.

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
