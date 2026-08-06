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

**[T-MAP]** `Map[K, V]` maps keys to values. **[T-MAP-KEY]** `K` must be
`i64`, `str`, or `bool`; any other key type is a compile error.

**[T-TUPLE]** `(A, B, …)` is a fixed-size heterogeneous product.

**[T-STRUCT]** `type P = { x: T, … }` declares a named product type whose
fields are accessed with `.`.

**[T-SUM]** `type S = | A(f: T) | B` declares a named sum type; values are
built with variant constructors and consumed with `match`.

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
It must be consumed by `try` or by a `match` covering `Ok` and `Err`.

**[T-TRY]** `try e` where `e : T ! E` yields `T`, propagating `Err` to the
caller. It is a compile error in a function that declares no error type —
except in `main` and in tests, whose error slot is open because the runtime
reports whatever reaches it (see **[E-MAIN-ERR]**).

**[T-ERR-SUBSET]** `try` is allowed only when the callee's error type is a
subset of the caller's declared error union.

**[T-FAIL]** `fail(e)` raises the error branch. Its type fits any context, and
it is a compile error where no error type is declared.

## 6. Exhaustiveness

**[T-MATCH-EXHAUSTIVE]** A `match` must cover every possible value. Coverage
is checked for sum types, `bool`, `Ok`/`Err`, and products of booleans; other
scrutinee types require a `_` arm.

**[T-MATCH-GUARD]** A guarded arm does not contribute to coverage, because its
guard may fail.

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

**[T-PARAM-INOUT]** An `inout` parameter is an exclusive mutable borrow.
**[T-INOUT-PLACE]** Its argument must be a variable, field, or index
expression — never a temporary. **[T-INOUT-MUTABLE]** That place must be
mutable. **[T-INOUT-EXCLUSIVE]** The same variable may not be passed as
`inout` twice in one call, nor as both `own` and `inout`.

## 9. Effects

**[T-EFFECT-DECL]** `effect E { fn op(…) -> T }` declares an effect and, with
it, a handler struct type `EHandler` whose fields are the operations.

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

**[T-FN-TYPE-ROW]** A function type carries its effect row explicitly:
`fn(i64) -> i64 / {io}`. **No row means the empty row**, so `fn(i64) -> i64`
accepts only functions that perform nothing, and passing one that prints is an
error at the call site. This is the only place a row can be checked rather than
inferred — a written type has no body to infer from — and without it a function
value would be a way to carry an effect past a declared row.

**[T-PAR-MAP-PURE]** The function given to a parallel operation — `par_map` or
`par_filter` — must have an empty effect row. Elements are processed concurrently, so an effect would interleave
differently between runs; requiring purity is what makes the parallelism
deterministic. This is a compile error, not a race.

**[T-TASK-CAPTURE]** A `spawn` body may only use bindings that already existed
when the enclosing `parallel` block began, plus the ones it declares itself.
Tasks run *after* the block finishes, so a binding declared inside it — a loop
variable included — no longer exists by then. This is a compile error rather
than a runtime failure.

## 10. Modules

**[T-MODULE-NS]** Functions are per-module; types and effects share one global
namespace, so a type name may not be declared twice across a program.

**[T-MODULE-PUB]** Only `pub` functions are callable from another module.

**[T-MODULE-RESOLVE]** `import a.b` names the module in `a/b.gr`, relative to
the importing package's source directory. The last segment is the qualifier
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
