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

**[T-EMPTY-LIT]** An empty collection literal takes its element type from the
surrounding annotation, struct field, parameter, or return type. Without any
of those the program is rejected.

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
`??`.

**[T-FALLIBLE-COALESCE]** `e ?? d` where `e : T ! E` yields the value or `d`,
the same shape `??` has over an `Option` ([E-COALESCE-LAZY] applies equally:
`d` is evaluated only on failure). Absence and failure are different things,
but "the value, or this default" is one thing, and writing it as a `match` at
every call site is the most repetitive shape in a program that reports errors
as values. The error itself is discarded — reach for `match` when it matters.

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

**[T-MOVE-LOOP]** Moving a binding declared outside a loop from inside that
loop is rejected: the second iteration would use a value that is gone.

**[T-COPY]** `i64`, `f64`, `bool`, and `()` are copied rather than moved, so
`own` on them never invalidates the caller's binding.

**[T-PARAM-INOUT-PLACES]** A mutating method works on any place, not only a
plain variable: `vm.stack.pop()`, `bag.items.set(k, v)`, `rows[0].xs.remove(i)`.
And an `inout` parameter crosses a module boundary like any other. Both were
compiled-backend gaps, and both mattered more than they sound — `inout` is the
answer [T-CAPTURE-WRITE] gives for threading mutable state, so its stopping at
a file edge reshaped a public API.

**[T-PARAM-INOUT]** An `inout` parameter is an exclusive mutable borrow.
**[T-INOUT-PLACE]** Its argument must be a variable, field, or index
expression — never a temporary. **[T-INOUT-MUTABLE]** That place must be
mutable. **[T-INOUT-EXCLUSIVE]** The same variable may not be passed as
`inout` twice in one call, nor as both `own` and `inout`.

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

**[T-FN-TYPE-ROW]** A function type carries its effect row explicitly:
`fn(i64) -> i64 / {io}`. **No row means the empty row**, so `fn(i64) -> i64`
accepts only functions that perform nothing, and passing one that prints is an
error at the call site. This is the only place a row can be checked rather than
inferred — a written type has no body to infer from — and without it a function
value would be a way to carry an effect past a declared row.

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
