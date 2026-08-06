# Grasm — complete language definition

Everything needed to write correct Grasm, in one file. No prior exposure
required: the normative rules, a guided tour, and an executable program for
every rule.

Each `[TAG]` below is a normative clause. Every clause has a corresponding
program in the conformance section, and `cargo test -p grasm_spec` fails if a
clause has no program, if a program names a clause that does not exist, or if
any program behaves differently from what it declares. So the rules and the
examples cannot disagree.


<!-- ===== spec/grammar.md ===== -->

# Lexical Structure and Grammar

Notation: `|` alternation, `?` optional, `*` zero or more, `+` one or more,
`( )` grouping, `"x"` literal text. Nonterminals are `lower_snake`.

## 1. Source text

**[L-ENCODING]** A source file is a sequence of UTF-8 encoded Unicode
characters. Identifiers and keywords are restricted to ASCII; string literals
and comments may contain any Unicode character.

**[L-NEWLINE]** A newline (U+000A, optionally preceded by U+000D) terminates a
statement. Consecutive newlines are equivalent to one. Inside `(` … `)` and
`[` … `]` newlines are insignificant; inside `{` … `}` they are significant
again.

**[L-WHITESPACE]** Spaces (U+0020) and tabs (U+0009) separate tokens and are
otherwise ignored.

## 2. Comments

**[L-COMMENT-LINE]** `//` begins a comment that ends at the next newline.

**[L-COMMENT-BLOCK]** `/*` begins a comment ending at the matching `*/`.
Block comments nest.

## 3. Identifiers and keywords

```
identifier  = ( letter | "_" ) ( letter | digit | "_" )*
letter      = "a".."z" | "A".."Z"
digit       = "0".."9"
```

**[L-IDENT]** An identifier that is not a reserved word denotes a name.

**[L-KEYWORD]** The reserved words are:

```
and  break  continue  effect  else  false  fn  for  handled_by  if
import  in  inout  let  match  not  or  own  pub  return  test  true
try  type  var  while  with
```

**[L-CONTEXTUAL]** `parallel` and `spawn` are *contextual* keywords: they
introduce a concurrency scope only when a `{` follows. In every other position
they are ordinary identifiers, so existing code using those names keeps
compiling.

## 4. Literals

**[L-INT]** Integer literals are decimal (`42`), hexadecimal (`0xFF`), or
binary (`0b1010`). Underscores may separate digits (`1_000_000`) and are
ignored. Integers are 64-bit signed.

**[L-FLOAT]** Float literals contain a fractional part (`3.14`) or an exponent
(`1e9`, `2.5e-3`). `1..5` is a range, not a float.

**[L-BOOL]** `true` and `false`.

**[L-STRING]** String literals are delimited by `"`. They may not span lines.

**[L-ESCAPE]** Recognized escapes: `\n`, `\t`, `\r`, `\\`, `\"`, `\0`, `\{`,
`\}`. Any other escape is a lexical error.

**[L-INTERP]** `{` inside a string begins an interpolation whose contents are
parsed as an expression and whose value is converted to text. Braces nest. A
literal brace is written `\{`.

## 5. Operators and punctuation

```
( ) [ ] { } , . .. ..= : :: -> => | |> + - * / % ^
= == != < <= > >= ! ? ?. ?? += -= *= /= _ @
```

**[L-MAXMUNCH]** The lexer always takes the longest matching operator, so
`->` is one token, never `-` followed by `>`.

## 6. Declarations

```
module      = declaration*
declaration = fn_decl | type_decl | effect_decl | test_decl | bench_decl
            | import_decl

fn_decl     = "pub"? "fn" identifier generics? params ret? err? effects? block
generics    = "[" identifier ( "," identifier )* "]"
params      = "(" ( param ( "," param )* )? ")"
param       = ( "inout" | "own" )? identifier ":" type
ret         = "->" type
err         = "!" type
effects     = "/" "{" ( path ( "," path )* )? "}"

type_decl   = "pub"? "type" identifier generics? "=" type_body
type_body   = struct_body | sum_body | type
struct_body = "{" ( field ( ("," | newline) field )* )? "}"
field       = identifier ":" type
sum_body    = ( "|" variant )+
variant     = identifier ( "(" ( field ( "," field )* )? ")" )?

effect_decl = "effect" identifier "{" fn_sig* "}"
fn_sig      = "fn" identifier params ret? err?

test_decl   = "test" string params? block
bench_decl  = "bench" string block
import_decl = "import" path
path        = identifier ( "." identifier )*
```

**[G-DECL-ORDER]** Declarations may appear in any order; forward references
are permitted.

**[G-TEST-PARAMS]** A `test` with a parameter list is a *property* test: the
runner supplies the values, so the parameters may not be `inout` or `own` and
are immutable in the body. A `test` without one is an ordinary unit test.

**[G-BENCH-CONTEXTUAL]** `bench` is a keyword only immediately before a string
literal at declaration position. Everywhere else it is an ordinary identifier,
so existing code that uses `bench` as a name keeps working.

## 7. Types

```
type       = named_type | list_type | tuple_type | union_type
named_type = path type_args?
type_args  = "[" type ( "," type )* "]"
list_type  = "[" type "]"
tuple_type = "(" ( type ( "," type )* )? ")"
union_type = "(" type ( "|" type )+ ")"
```

**[G-TYPE-UNIT]** `()` is the unit type.

## 8. Statements

```
block     = "{" statement* "}"
statement = let_stmt | assign_stmt | expr_stmt | return_stmt
          | break_stmt | continue_stmt | while_stmt | for_stmt
          | with_stmt | parallel_stmt | spawn_stmt

let_stmt      = ( "let" | "var" ) identifier ( ":" type )? "=" expression
assign_stmt   = place ( "=" | "+=" | "-=" | "*=" | "/=" ) expression
return_stmt   = "return" expression?
break_stmt    = "break"
continue_stmt = "continue"
while_stmt    = "while" expression block
for_stmt      = "for" identifier "in" expression block
with_stmt     = "with" path "handled_by" expression block
parallel_stmt = "parallel" block
spawn_stmt    = "spawn" block

place = identifier ( "." identifier | "[" expression "]" )*
```

**[G-STMT-TERM]** A statement is terminated by a newline, a `}`, or end of
file.

**[G-STMT-LAST]** The value of a block is the value of its final statement if
that statement is an expression; otherwise the block has type `()`.

## 9. Expressions

Precedence, loosest to tightest:

| Level | Operators | Associativity |
|---|---|---|
| 1 | `or` | left |
| 2 | `and` | left |
| 3 | `==` `!=` `<` `<=` `>` `>=` | left |
| 4 | `..` `..=` | none |
| 5 | `??` | left |
| 6 | `\|>` | left |
| 7 | `+` `-` | left |
| 8 | `*` `/` `%` | left |
| 9 | `^` | **right** |
| 10 | unary `-` `not` `try` | prefix |
| 11 | call, index, field access | postfix |

```
expression   = or_expr
primary      = literal | identifier | list_lit | tuple_lit | struct_lit
             | if_expr | match_expr | block | closure | "(" expression ")"
list_lit     = "[" ( expression ( "," expression )* ","? )? "]"
tuple_lit    = "(" ( expression ( "," expression )+ ","? )? ")"
struct_lit   = identifier "{" ( field_init ( ("," | newline) field_init )* )? "}"
field_init   = identifier ":" expression
if_expr      = "if" expression block ( "else" ( if_expr | block ) )?
match_expr   = "match" expression "{" match_arm* "}"
match_arm    = pattern ( "if" expression )? "->" ( expression | statement )
closure      = "|" ( identifier ( "," identifier )* )? "|" ( expression | block )
```

**[G-PIPE]** `x |> f(a)` is exactly `f(x, a)`; `x |> f` is `f(x)`.

**[G-STRUCT-AMBIG]** In the condition position of `if`, `while`, `for … in`,
and `match`, an identifier followed by `{` is *not* a struct literal — the
brace opens the body. Parenthesize to force a struct literal there.

**[G-ARM-CONTROL]** A match arm body may be any statement — an assignment or a
bare `break`, `continue`, or `return` — without wrapping it in a block.

## 10. Patterns

```
pattern = "_" | int_lit | "-" int_lit | bool_lit | string_lit
        | identifier
        | path ( "(" ( pattern ( "," pattern )* )? ")" )?
        | "(" ( pattern ( "," pattern )* )? ")"
```

**[G-PAT-BIND]** A bare identifier binds unless it names a nullary variant of
the scrutinee's type, in which case it tests for that variant.

## 11. Nesting limit

**[G-NEST-LIMIT]** Expression and block nesting is capped at 64 levels.
Exceeding it is a diagnostic, never a crash.


<!-- ===== spec/types.md ===== -->

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


<!-- ===== spec/semantics.md ===== -->

# Evaluation Semantics

The tree-walking interpreter in `grasm_interp` is the **reference
implementation**: where the native backend and the interpreter disagree, the
interpreter is right and the backend has a bug. `scripts/difftest.sh` enforces
this on every program in the repository.

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

**[E-COALESCE-LAZY]** `a ?? b` evaluates `b` only when `a` is `None`.

## 3. Arithmetic

**[E-ARITH-INT]** `+ - *` on `i64` wrap on overflow (two's complement), and
so does `/`: `i64::MIN / -1` has no representable result and yields `i64::MIN`
rather than trapping. `i64::MIN % -1` is `0`, as `x % -1` is for every `x`.

**[E-DIV-ZERO]** Integer `/` or `%` by zero panics.

**[E-POW-NEG]** A negative integer exponent panics; use floats.

**[E-RECURSION-DEPTH]** Recursion is bounded. **At least 10,000 nested calls
are guaranteed to work.** Past an implementation's own bound the program
panics with a stack-overflow message and exit 101 — it does not abort, crash,
or corrupt the process. The bound above the guaranteed minimum is
implementation-defined and differs between the two engines, so a program that
recurses deeper than 10,000 should not assume it will run everywhere.

**[E-ARITH-FLOAT]** Float arithmetic follows IEEE-754, including infinities and
NaN. Division by zero yields an infinity rather than panicking.

**[E-NUM-PROMOTE]** Mixing `i64` and `f64` in one operation promotes to `f64`.

**[E-STR-CONCAT]** `+` on two `str` values concatenates them.

**[E-STR-COMPARE]** `< <= > >=` on `str` compare lexicographically by code
point.

**[E-EQ-STRUCTURAL]** `==` compares structurally and recursively; two values of
the same shape and contents are equal.

## 4. Collections

**[E-INDEX-BOUNDS]** Indexing a list out of bounds panics with the index and
the length. `list.get(i)` returns `Option` instead of panicking.

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

**[E-IF-VALUE]** `if` is an expression when both branches yield the same type;
otherwise it is a statement of type `()`.

**[E-MATCH-ORDER]** Match arms are tried top to bottom; the first whose pattern
matches and whose guard passes is taken.

**[E-LOOP-RANGE]** `for i in a..b` iterates `a` up to but excluding `b`;
`a..=b` includes `b`. An empty range runs zero times.

**[E-LOOP-LIST]** `for x in xs` iterates a list's elements directly, in order.
The loop variable is the element, not its index. Iterating an empty list runs
zero times.

## 6. Errors and panics

**[E-ERR-PROPAGATE]** `try` on `Err(e)` returns `Err(e)` from the enclosing
function immediately, skipping the rest of its body.

**[E-PANIC-EXIT]** A panic prints `panic: <message>` and exits with status
101. It is a program bug, not a recoverable condition.

**[E-MAIN-ERR]** An error reaching `main` prints
`error: unhandled error from main: <value>` and exits with status 1.

## 7. Effects

**[E-HANDLER-INNERMOST]** An effect operation dispatches to the innermost
enclosing handler for that effect.

**[E-HANDLER-SCOPE]** A handler is installed for the dynamic extent of its
`with` block, including calls made from inside it.

**[E-HANDLER-MISSING]** A user effect that reaches `main` or a test without a
handler is rejected *at compile time*, so the runtime's missing-handler panic is
a defensive backstop that well-typed programs never reach.

## 8. Concurrency

**[E-PARALLEL-JOIN]** A `parallel` block does not exit until every task spawned
inside it has completed.

**[E-SPAWN-ORDER]** Tasks run to completion in the order they were spawned.
This is deterministic: the same program always produces the same output.

**[E-SPAWN-SHARED]** A task body sees the enclosing scope, which is how tasks
share results.

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

## 9. Ownership at runtime

**[E-INOUT-WRITEBACK]** Mutations to an `inout` parameter are visible to the
caller when the function returns.

**[E-OWN-RUNTIME]** `own` has no runtime effect beyond an ordinary pass; the
guarantee it provides is entirely static.

## 10. Testing

**[E-TEST-PROPERTY]** A property test runs its body once per generated case.
It passes only if every case holds; the first failing case ends the run.

**[E-TEST-SHRINK]** A failing property is reported with a *shrunk*
counterexample: the runner repeatedly substitutes smaller inputs and keeps any
that still fail, so the reported values are minimal rather than whatever the
generator happened to produce.

**[E-TEST-SEED]** Generation is driven entirely by a seed. The same seed and
case count reproduce the same run exactly, and a failure reports the seed that
found it.

**[E-BENCH-WARMUP]** A benchmark body runs once before timing starts. A body
that panics is reported as a failure instead of a measurement.

## 11. Program startup

**[E-ENTRY]** Execution begins at `main` in the entry module. A program
without `main` is an error.

**[E-ARGS]** `args()` returns the arguments after `--`, excluding the program
name.


<!-- ===== spec/library.md ===== -->

# Standard Library

Every operation the language provides, by receiver type. There are no imports:
these are always available.

`spec/conformance/L-LIBRARY.gr` exercises this whole surface, so an entry here
that stopped being true would fail the test suite.

## 1. Free functions

**[L-LIB-FREE]** Available without a receiver.

| call | type | notes |
|------|------|-------|
| `print(x)` | `(T)` | writes `x` then a newline; performs `io` |
| `input()` | `() -> str` | one line from stdin, without the newline; `io` |
| `args()` | `() -> [str]` | arguments after `--` |
| `panic(msg)` | `(str)` | prints `panic: msg`, exits 101 |
| `assert(c)` | `(bool)` | panics when `c` is false |
| `assert_eq(a, b)` | `(T, T)` | panics when `a != b` |
| `fail(e)` | `(E)` | raises the error branch of a `T ! E` |
| `map()` | `() -> Map[K, V]` | a new empty map |
| `now_millis()` | `() -> i64` | milliseconds since the epoch; `io` |
| `env(name)` | `(str) -> Option[str]` | environment variable; `io` |
| `exit(code)` | `(i64)` | ends the program with that status |
| `read_file(p)` | `(str) -> Option[str]` | `io` |
| `write_file(p, s)` | `(str, str)` | `io` |
| `file_exists(p)` | `(str) -> bool` | `io` |

## 1a. How values are written

**[L-DISPLAY]** `print(x)` and string interpolation render every type the same
way, and this is normative because a program's output is what its correctness
is judged on.

| type | rendering | example |
|------|-----------|---------|
| `i64` | decimal | `-7` |
| `f64` | shortest form that round-trips, always with a point | `1.5`, `1000000.0`, `-0.0` |
| `bool` | `true` / `false` | |
| `str` | its characters, **unquoted and unescaped** | `b c` |
| `[T]` | elements in order, `, `-separated, in brackets | `[1, 2, 3]`, `[a, b c]` |
| `(A, B)` | components in order, `, `-separated, in parentheses | `(1, one, true)` |
| `Map[K, V]` | `key: value` pairs in sorted key order, in braces | `{k: 1}` |
| struct | `Name{field: value, …}` in declaration order | `P{x: 3, y: hi}` |
| variant with fields | `Name(v, …)` | `B(7)` |
| variant without | `Name` | `A` |
| `Option[T]` | `Some(v)` or `None` | `Some(1)` |

Strings nested inside a collection are **not** quoted, so `[a, b c]` is a
two-element list whose second element contains a space. Rendering is for people
reading output; it is not a serialisation format and does not round-trip.

## 1b. Calling a free function as a method

**[L-UFCS]** A free function whose first parameter accepts the receiver may be
called in method position: `f(x, y)` and `x.f(y)` are the same call. This
applies to functions a program declares, not only to the library.

```
fn double(n: i64) -> i64 { n * 2 }

fn main() {
    print("{double(5)}")     // 10
    print("{5.double()}")    // 10 — the same call
}
```

Method syntax is resolved against the built-in methods for the receiver's type
first, and a free function is considered only when no built-in matches. So
declaring a function named `len` does not change what `xs.len()` means.

## 2. `i64` and `f64`

**[L-LIB-NUM]**

| method | type | notes |
|--------|------|-------|
| `n.abs()` | `-> i64` / `-> f64` | |
| `n.min(m)`, `n.max(m)` | `(T) -> T` | |
| `x.floor()`, `x.ceil()`, `x.round()` | `f64 -> f64` | |
| `x.sqrt()` | `f64 -> f64` | |
| `n.to_str()` | `-> str` | |
| `n.as_f64()` | `i64 -> f64` | widening |
| `x.as_i64()` | `f64 -> i64` | truncates toward zero |

Conversion is by method, not by a cast or a `int(...)`-style function. A `str`
becomes a number with `parse_int` / `parse_float`, which return `Option`
because the text may not be a number.

## 3. `str`

**[L-LIB-STR]** Lengths and offsets are in **characters, not bytes**
([E-STR-CHARS]).

| method | type | notes |
|--------|------|-------|
| `s.len()` | `-> i64` | characters |
| `s.is_empty()` | `-> bool` | |
| `s.substr(start, len)` | `(i64, i64) -> str` | **start and length**, not start and end |
| `s.chars()` | `-> [str]` | one single-character string per element |
| `s.lines()` | `-> [str]` | splits on newlines; no trailing empty line |
| `s.split(sep)` | `(str) -> [str]` | |
| `s.contains(t)`, `s.starts_with(t)`, `s.ends_with(t)` | `(str) -> bool` | |
| `s.index_of(t)` | `(str) -> Option[i64]` | `None` when absent |
| `s.replace(from, to)` | `(str, str) -> str` | |
| `s.trim()` | `-> str` | |
| `s.to_upper()`, `s.to_lower()` | `-> str` | |
| `s.repeat(n)` | `(i64) -> str` | |
| `s.pad_start(w, fill)`, `s.pad_end(w, fill)` | `(i64, str) -> str` | fill repeats and is cut to width ([E-PAD-CYCLE]) |
| `s.parse_int()` | `-> Option[i64]` | |
| `s.parse_float()` | `-> Option[f64]` | |

## 4. `[T]`

**[L-LIB-LIST]** Indexing out of bounds panics; `get` returns `Option`
([E-INDEX-BOUNDS]).

| method | type | notes |
|--------|------|-------|
| `for x in xs { … }` | | iterates elements, not indices ([E-LOOP-LIST]) |
| `xs.len()`, `xs.is_empty()` | | |
| `xs[i]` | `-> T` | panics when out of range |
| `xs.get(i)` | `(i64) -> Option[T]` | |
| `xs.first()`, `xs.last()` | `-> Option[T]` | |
| `xs.push(v)` | `(T)` | mutates; receiver must be `var` |
| `xs.pop()` | `-> Option[T]` | mutates |
| `xs.sort()` | | mutates; ascending. Bools have no order, so sorting them does nothing |
| `xs.rev()` | `-> [T]` | |
| `xs.slice(a, b)` | `(i64, i64) -> [T]` | `a` up to but excluding `b` |
| `xs.contains(v)` | `(T) -> bool` | |
| `xs.join(sep)` | `(str) -> str` | `[str]` only — map to `str` first |
| `xs.sum()` | `-> T` | numeric elements |
| `xs.min()`, `xs.max()` | `-> Option[T]` | `None` when empty; not available for `bool` |
| `xs.map(f)` | `(T -> U) -> [U]` | |
| `xs.par_map(f)` | `(T -> U) -> [U]` | multi-core; `f` must be effect-free ([T-PAR-MAP-PURE]) |
| `xs.filter(f)` | `(T -> bool) -> [T]` | |
| `xs.par_filter(f)` | `(T -> bool) -> [T]` | multi-core; `f` must be effect-free; input order kept ([E-PAR-FILTER-ORDER]) |
| `xs.any(f)`, `xs.all(f)` | `(T -> bool) -> bool` | short-circuit ([E-SEARCH-SHORTCIRCUIT]) |
| `xs.find(f)` | `(T -> bool) -> Option[T]` | first match |

## 5. `Map[K, V]`

**[L-LIB-MAP]** Keys are `i64`, `str`, or `bool`. Iteration is in sorted key
order ([E-MAP-ORDER]).

| method | type | notes |
|--------|------|-------|
| `m.len()`, `m.is_empty()` | | |
| `m.get(k)` | `(K) -> Option[V]` | |
| `m.set(k, v)` | `(K, V)` | mutates; receiver must be `var` |
| `m.has(k)` | `(K) -> bool` | |
| `m.delete(k)` | `(K) -> bool` | mutates; whether it was present |
| `m.keys()` | `-> [K]` | sorted |
| `m.values()` | `-> [V]` | in key order |

## 6. `Option[T]`

**[L-LIB-OPTION]**

| form | notes |
|------|-------|
| `Some(v)`, `None` | constructors |
| `o ?? fallback` | the value, or `fallback`; the right side is evaluated only when needed ([E-COALESCE-LAZY]) |
| `o.is_none()`, `o.is_some()` | |
| `o.unwrap_or(d)` | same as `??` |
| `match o { Some(v) -> … None -> … }` | exhaustive |


<!-- ===== docs/tutorial.md ===== -->

# Grasm in an hour

A guided tour, in the order the pieces build on each other. Every program here
runs — `grasm run <file>` — and the code blocks are the same ones the
conformance suite and `examples/` exercise, so nothing drifts out of date
silently.

If you want the exact rules rather than the tour, read `spec/`. This document
explains; the spec decides.

---

## 1. A first program

```
fn main() {
    print("hello, world")
}
```

`grasm run hello.gr` typechecks and executes. `grasm build hello.gr --native`
produces a standalone binary through Cranelift.

There are no semicolons, no imports needed for `print`, and no type
annotations required where they can be inferred:

```
fn main() {
    let name = "world"
    let count = 3
    for i in 1..=count {
        print("hello, {name} ({i})")
    }
}
```

`{...}` inside a string interpolates any expression. `1..=3` includes 3;
`1..3` excludes it.

## 2. Bindings

`let` is immutable, `var` is not. That is the whole rule.

```
fn main() {
    let x = 1
    var total = 0
    total += x        // fine
    // x = 2         // error: cannot assign to `x` — it is immutable
    print("{total}")
}
```

Values have **value semantics**: assigning or passing never creates an alias,
so mutating one binding can never be observed through another.

```
fn main() {
    var a = [1, 2, 3]
    var b = a
    b.push(4)
    print("{a.len()} {b.len()}")   // 3 4
}
```

Copies are lazy — a copy costs a reference-count bump and only becomes a real
copy on mutation of a shared value — but that is an optimization you cannot
observe.

## 3. Types

Scalars are `i64`, `f64`, `bool`, `str`. Collections are `[T]` and
`Map[K, V]`. Annotate when you want to; infer when you do not.

```
type Point = { x: f64, y: f64 }

type Shape =
    | Circle(r: f64)
    | Rect(w: f64, h: f64)

fn area(s: Shape) -> f64 {
    match s {
        Circle(r) -> 3.14159 * r * r
        Rect(w, h) -> w * h
    }
}

fn main() {
    print("{area(Circle(2.0))}")
}
```

`match` must be exhaustive. Adding a variant to `Shape` turns every incomplete
match into a compile error rather than a surprise at runtime.

Maps iterate in sorted key order, so printing one is deterministic:

```
fn main() {
    var counts = map()
    counts.set("pear", 2)
    counts.set("fig", 1)
    print("{counts}")            // same output on every run
}
```

## 4. Errors are values

There are no exceptions. A function that can fail says so in its type with
`! E`, raises with `fail`, and callers propagate with `try`.

```
fn divide(a: i64, b: i64) -> i64 ! str {
    if b == 0 {
        fail("division by zero")
    }
    a / b
}

fn compute() -> i64 ! str {
    let half = try divide(10, 2)
    try divide(half, 0)
}

fn main() {
    match compute() {
        Ok(v) -> print("got {v}")
        Err(e) -> print("failed: {e}")
    }
}
```

You cannot forget to handle one: using a fallible value without `try` or a
`match` is a compile error. An error that reaches `main` exits 1 with a
message rather than exiting 0 as though nothing happened.

A **panic** is different: it means a bug (index out of bounds, divide by zero,
a failed `assert`), prints `panic: <message>`, and exits 101. Panics are not
for recoverable conditions.

`Option` covers absence:

```
fn main() {
    let xs = [3, 1, 4]
    print("{xs.min() ?? 0}")           // 1
    print("{xs.find(|x| x > 99) ?? -1}")  // -1
}
```

`??` evaluates its right side only when the left is `None`.

## 5. Effects

A function's type says what it *does*, not just what it returns. Built-in
effects are `io` and `async`; you can declare your own.

```
effect Clock {
    fn now() -> i64
}

fn stamp() -> str / {Clock} {
    "at {Clock.now()}"
}

fn main() {
    with Clock handled_by ClockHandler{ now: || 1700000000 } {
        print(stamp())
    }
}
```

An effect that reaches `main` without a handler is a *compile* error. This is
what makes tests hermetic by construction: a function that touches the clock
or the network cannot pretend otherwise, and a test supplies a handler instead
of a mocking framework.

The same property is what the package manager enforces at the dependency
boundary — see §8.

## 6. Ownership without a borrow checker

Three parameter modes, and the default is the one you want:

- plain — the callee gets a value; the caller keeps theirs.
- `inout` — the callee mutates the caller's binding in place. The caller
  passes a mutable place, and the same place cannot be passed as `inout` twice
  in one call.
- `own` — the caller gives the value up. Using it afterwards is a compile
  error, **for types that are not copied**. `i64`, `f64`, `bool`, and `()` are
  copied ([T-COPY]), so `own` on one of those transfers nothing and the caller
  keeps using its binding. `str`, lists, maps, and structs do move. Write the
  demonstration with one of those, or it will appear to disprove the rule it
  is demonstrating:

```
fn consume(own xs: [i64]) -> i64 {
    xs.len()
}

fn main() {
    let xs = [1, 2, 3, 4, 5]
    print("{consume(xs)}")
    // print("{xs.len()}")   // error: use of moved value `xs`
}
```

```
fn bump(inout n: i64) {
    n += 1
}

fn main() {
    var count = 0
    bump(count)
    print("{count}")     // 1
}
```

The mode lives on the parameter, so the call site stays plain. No lifetimes,
no borrow annotations on types, no `&`/`&mut` sprinkled through the code.

## 6a. Functions as parameters

`fn(A, B) -> R` is a type, so a program can declare its own higher-order
function rather than being limited to the built-in ones:

```
fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn double(n: i64) -> i64 {
    n * 2
}

fn main() {
    print("{apply(double, 5)}")
    print("{apply(|n| n + 1, 5)}")
}
```

A closure passed to such a parameter takes its parameter types from it, so it
needs no annotation.

The type carries an effect row, and **no row means the empty row**. So
`fn(i64) -> i64` accepts only functions that perform nothing, and passing one
that prints is an error where you pass it — not a surprise inside. Write
`fn(i64) -> i64 / {io}` when the caller should be allowed to hand over
something effectful. This is the one place a row is written rather than
inferred, because a type has no body to infer from.

## 7. Structured concurrency

`parallel` opens a scope; `spawn` adds a task to it. The scope does not exit
until every task inside it has finished.

```
fn main() {
    var log: [str] = []
    parallel {
        spawn {
            log.push("first")
        }
        spawn {
            log.push("second")
        }
    }
    print(log.join(" "))     // first second
}
```

Tasks run in spawn order, deterministically: the same program always produces
the same output. That determinism is deliberate and is *not* multi-core
speedup — `parallel` buys you structure and reproducibility, not throughput,
because its tasks share the enclosing scope and running them at once would
race.

Throughput has its own form, and the effect system is what makes it safe:

```
fn main() {
    let xs = [1, 2, 3, 4, 5]
    print("{xs.par_map(|x| x * x)}")
}
```

`par_map` and `par_filter` spread the work across cores. The function must be
effect-free and **the compiler proves it** — put a `print` inside and the
build fails — so there are no `Send`/`Sync` bounds to satisfy and no race
detector to run. Results come back in input order whatever the core count is,
so the answer never depends on the machine.

A panicking task aborts the whole scope; tasks spawned after it do not run.

## 8. Modules and packages

```
import lib.math

fn main() {
    print("{math.square(4)}")
}
```

`pub` controls what crosses a module boundary. `import helper` finds
`helper.gr` beside the importing file; `import lib.math` finds `lib/math.gr`.
No manifest is needed for either — a manifest is what turns a directory into a
*package*, which is what dependencies and capabilities are about.

A package is a directory laid out like this:

```
mathlib/
  grasm.toml          the manifest
  src/
    main.gr           the entry file — what `import mathlib` resolves to
```

`src/main.gr` is the default; `entry = "src/lib.gr"` in `[package]` changes it.
Everything a package exports lives behind that one file, so a dependency's
other modules are reached as `import mathlib.helpers`.

```toml
[package]
name = "mathlib"
version = "0.1.0"
capabilities = []          # this package performs no effects

[dependencies.utils]
path = "../utils"
version = "^1.0.0"         # optional: 1.x, and no other line
```

`version` accepts a compatible range with `^` (a bare `1.0.0` means the same)
or one exact version with `=`. Before 1.0 the minor number carries the weight
of a major, so `^0.2.0` stops below `0.3.0`.

Drop the `path` and the package comes from a registry — a directory of
`<name>/<version>/` packages, pointed at by `GRASM_REGISTRY` or found as the
nearest `.grasm/registry`. Which version you get is the *lowest* one that
satisfies everybody, so adding a dependency never quietly upgrades something
else you were already using.

`capabilities` is a **ceiling**, and it is checked against what the code
actually does. **Omitting the key means unrestricted**, not empty — a package
with no `capabilities` line is not held to anything, and `grasm deps` reports
it as `(unrestricted)`. Write `capabilities = []` to mean "performs nothing";
leaving it out means "do not check me". Because effects are inferred from source, a dependency that
quietly starts reading the filesystem exceeds its declared ceiling and fails
to build **in every consumer** — a compile error, not a runtime surprise, and
not something a reviewer has to happen to notice.

`grasm deps` resolves the graph, checks every package against its ceiling, and
writes `grasm.lock` with a SHA-256 over each package's sources.

## 9. Testing

One declaration form covers three kinds of test.

```
fn reverse(xs: [i64]) -> [i64] {
    var out: [i64] = []
    var i = xs.len() - 1
    while i >= 0 {
        out.push(xs[i])
        i = i - 1
    }
    out
}

test "reverse of a known list" {
    assert(reverse([1, 2, 3]) == [3, 2, 1])
}

test "reversing twice is the identity" (xs: [i64]) {
    assert(reverse(reverse(xs)) == xs)
}
```

Parameters make it a **property** test: the runner generates the inputs.
Pointing one at `str` is fuzzing — the generator emits tabs, newlines, quotes,
backslashes, and multi-byte characters.

A failure reports a *shrunk* counterexample — the smallest input that still
fails — plus the seed that found it:

```text
test "doubling is monotonic" ... FAILED on case 5 of 100
  assertion failed
  counterexample: n = 4611686018427387904
  replay: --seed 24301 --cases 100
```

That number is exactly the smallest `n` where `n * 2` wraps.

`grasm test --coverage` reports which statement lines the suite reached.
`bench "name" { ... }` bodies are warmed up, then sampled, and `grasm bench`
reports ns/op.

## 10. Documentation that cannot rot

`///` comments document the declaration that follows. `grasm doc` renders them
as Markdown with signatures taken from the AST, so a documented effect row or
error type cannot drift from the real one.

````
/// Adds two integers.
///
/// Addition wraps on overflow.
///
/// ```
/// assert(add(2, 3) == 5)
/// ```
fn add(a: i64, b: i64) -> i64 {
    a + b
}
````

The fenced block is **executable**. `grasm test` runs it as a test, in a
program that keeps the real module structure — so an example may write
`mathlib.square(4)`, exactly as a consumer would. Documentation that stops
being true fails the suite instead of quietly misleading the next reader.

Lines beginning `# ` inside a block are hidden setup: they run, but they are
not part of what a reader sees.

## Where to go next

- `spec/` — the normative rules, each tagged and backed by a program in
  `spec/conformance/` that the test suite runs.
- `examples/` — runnable programs, including `stdlib_tour.gr` and
  `testing_tour.gr`.
- `projects/` — twelve larger programs, beginner through advanced.
- `MASTERPLAN.md` — where the language is going and why.


# Conformance programs

One per normative clause. Each header states the tag it demonstrates and the exact expected behaviour.

## `E-ARGS.gr`

```grasm
//! tag: E-ARGS
//! expect: output
//! 0
fn main() {
    // No arguments were passed after `--`.
    print("{args().len()}")
}
```

## `E-ARITH-FLOAT.gr`

```grasm
//! tag: E-ARITH-FLOAT
//! expect: output
//! 0.30000000000000004
//! inf
fn main() {
    print("{0.1 + 0.2}")
    print("{1.0 / 0.0}")
}
```

## `E-ARITH-INT.gr`

```grasm
//! tag: E-ARITH-INT
//! expect: output
//! 14
//! -9223372036854775808
//! -9223372036854775808
//! 0
//! 7 0 -7
fn main() {
    print("{2 + 3 * 4}")
    // Wrapping on overflow, two's complement.
    var max = 9223372036854775807
    print("{max + 1}")

    // `i64::MIN / -1` has no i64 result, so it wraps like any other overflow
    // rather than trapping. The hardware divide instruction does trap on it,
    // which is why this is worth a program: the native backend has to avoid
    // handing the CPU that operand pair.
    var min = -9223372036854775807
    min = min - 1
    print("{min / -1}")
    print("{min % -1}")

    print("{-7 / -1} {-7 % -1} {7 / -1}")
}
```

## `E-BENCH-WARMUP.gr`

```grasm
//! tag: E-BENCH-WARMUP
//! expect: output
//! 4
// A `bench` declaration is checked like any other body but is only executed by
// `grasm bench`, so running this program prints only what `main` prints.
bench "doubling" {
    let x = 2 * 2
    assert(x == 4)
}

fn main() {
    print("{2 * 2}")
}
```

## `E-COALESCE-LAZY.gr`

```grasm
//! tag: E-COALESCE-LAZY
//! expect: output
//! 5
//! 7
fn main() {
    let some = Some(5)
    print("{some ?? 0}")
    let none: Option[i64] = None
    print("{none ?? 7}")
}
```

## `E-COW.gr`

```grasm
//! tag: E-COW
//! expect: output
//! 3 4
fn main() {
    // Copy-on-write is never observable: the copy is independent.
    var xs = [1, 2, 3]
    var ys = xs
    ys.push(4)
    print("{xs.len()} {ys.len()}")
}
```

## `E-DIV-ZERO.gr`

```grasm
//! tag: E-DIV-ZERO
//! expect: panic
//! division by zero
fn main() {
    let a = 1
    let b = 0
    print("{a / b}")
}
```

## `E-ENTRY.gr`

```grasm
//! tag: E-ENTRY
//! expect: output
//! started
fn main() {
    print("started")
}
```

## `E-EQ-STRUCTURAL.gr`

```grasm
//! tag: E-EQ-STRUCTURAL
//! expect: output
//! true true false
type P = { x: i64, y: [str] }
fn main() {
    let a = P{ x: 1, y: ["k"] }
    let b = P{ x: 1, y: ["k"] }
    let c = P{ x: 2, y: ["k"] }
    print("{a == b} {[1, 2] == [1, 2]} {a == c}")
}
```

## `E-ERR-PROPAGATE.gr`

```grasm
//! tag: E-ERR-PROPAGATE
//! expect: output
//! caught Bad
//! skipped rest
type E = | Bad
fn failing() -> i64 ! E {
    fail(Bad)
}
fn caller(inout reached: bool) -> i64 ! E {
    let v = try failing()
    reached = true
    v
}
fn main() {
    var reached = false
    match caller(reached) {
        Ok(v) -> print("ok {v}")
        Err(e) -> print("caught {e}")
    }
    if not reached {
        print("skipped rest")
    }
}
```

## `E-HANDLER-INNERMOST.gr`

```grasm
//! tag: E-HANDLER-INNERMOST
//! expect: output
//! 1
//! 2
//! 1
effect Clock {
    fn now() -> i64
}
fn main() {
    with Clock handled_by ClockHandler{ now: || 1 } {
        print("{Clock.now()}")
        with Clock handled_by ClockHandler{ now: || 2 } {
            print("{Clock.now()}")
        }
        print("{Clock.now()}")
    }
}
```

## `E-HANDLER-MISSING.gr`

```grasm
//! tag: E-HANDLER-MISSING
//! expect: error
//! unhandled effect
effect Cfg {
    fn value() -> i64
}
fn reader() -> i64 {
    Cfg.value()
}
fn main() {
    // No `with Cfg handled_by ...` anywhere: the checker rejects this before
    // the program can run, so the runtime panic is unreachable here.
    print("{reader()}")
}
```

## `E-HANDLER-SCOPE.gr`

```grasm
//! tag: E-HANDLER-SCOPE
//! expect: output
//! 42
effect Cfg {
    fn value() -> i64
}
fn deep() -> i64 {
    Cfg.value()
}
fn middle() -> i64 {
    deep()
}
fn main() {
    with Cfg handled_by CfgHandler{ value: || 42 } {
        print("{middle()}")
    }
}
```

## `E-IF-VALUE.gr`

```grasm
//! tag: E-IF-VALUE
//! expect: output
//! big
//! 0
fn describe(n: i64) -> str {
    if n > 10 {
        "big"
    } else {
        "small"
    }
}
fn main() {
    print(describe(50))
    var count = 0
    if false {
        count = 1
    }
    print("{count}")
}
```

## `E-INDEX-BOUNDS.gr`

```grasm
//! tag: E-INDEX-BOUNDS
//! expect: panic
//! index 5 out of bounds (len 2)
fn main() {
    let xs = [1, 2]
    print("{xs[5]}")
}
```

## `E-INOUT-WRITEBACK.gr`

```grasm
//! tag: E-INOUT-WRITEBACK
//! expect: output
//! 3 [1, 2]
fn bump(inout n: i64) {
    n += 1
}
fn add(inout xs: [i64], v: i64) {
    xs.push(v)
}
fn main() {
    var n = 2
    bump(n)
    var xs = [1]
    add(xs, 2)
    print("{n} {xs}")
}
```

## `E-LOOP-LIST.gr`

```grasm
//! tag: E-LOOP-LIST
//! expect: output
//! alpha beta gamma
//! 6
//! 0
fn main() {
    // The loop variable is the element itself, not an index.
    let words = ["alpha", "beta", "gamma"]
    var joined = ""
    for w in words {
        joined = joined + w + " "
    }
    print(joined.trim())

    var total = 0
    for n in [1, 2, 3] {
        total = total + n
    }
    print("{total}")

    // An empty list runs the body zero times.
    var count = 0
    let none: [i64] = []
    for _x in none {
        count = count + 1
    }
    print("{count}")
}
```

## `E-LOOP-RANGE.gr`

```grasm
//! tag: E-LOOP-RANGE
//! expect: output
//! 0123
//! 01234
//! empty
fn main() {
    var a = ""
    for i in 0..4 {
        a += "{i}"
    }
    print(a)
    var b = ""
    for i in 0..=4 {
        b += "{i}"
    }
    print(b)
    var n = 0
    for _i in 5..5 {
        n += 1
    }
    if n == 0 {
        print("empty")
    }
}
```

## `E-MAIN-ERR.gr`

```grasm
//! tag: E-MAIN-ERR
//! expect: output-failing
//! error: unhandled error from main: Bad
type E = | Bad
fn main() ! E {
    fail(Bad)
}
```

## `E-MAP-ORDER.gr`

```grasm
//! tag: E-MAP-ORDER
//! expect: output
//! [alpha, beta, zulu]
//! {alpha: 1, beta: 2, zulu: 3}
fn main() {
    var m = map()
    m.set("zulu", 3)
    m.set("alpha", 1)
    m.set("beta", 2)
    print("{m.keys()}")
    print("{m}")
}
```

## `E-MATCH-ORDER.gr`

```grasm
//! tag: E-MATCH-ORDER
//! expect: output
//! first
fn main() {
    let n = 5
    let r = match n {
        x if x > 0 -> "first"
        5 -> "second"
        _ -> "third"
    }
    print(r)
}
```

## `E-MINMAX-OPTION.gr`

```grasm
//! tag: E-MINMAX-OPTION
//! expect: output
//! 1 9
//! fig pear
//! true true
fn main() {
    print("{[3, 1, 9].min() ?? 0} {[3, 1, 9].max() ?? 0}")
    print("{["pear", "fig"].min() ?? "?"} {["pear", "fig"].max() ?? "?"}")
    let none: [i64] = []
    print("{none.min().is_none()} {none.max().is_none()}")
}
```

## `E-NOT-PARALLEL.gr`

```grasm
//! tag: E-NOT-PARALLEL
//! expect: output
//! sequential
fn main() {
    // Tasks run one at a time, so a task always observes the previous
    // task's completed writes — never a partial interleaving.
    var log = ""
    parallel {
        spawn {
            log += "seq"
        }
        spawn {
            log += "uential"
        }
    }
    print(log)
}
```

## `E-NUM-PROMOTE.gr`

```grasm
//! tag: E-NUM-PROMOTE
//! expect: output
//! 7.5
fn main() {
    let f = 2.5
    print("{3 * f}")
}
```

## `E-ORDER-LTR.gr`

```grasm
//! tag: E-ORDER-LTR
//! expect: output
//! a
//! b
//! 3
fn note(s: str, v: i64) -> i64 {
    print(s)
    v
}
fn main() {
    print("{note("a", 1) + note("b", 2)}")
}
```

## `E-OWN-RUNTIME.gr`

```grasm
//! tag: E-OWN-RUNTIME
//! expect: output
//! 3 3
fn consume(own xs: [i64]) -> i64 {
    xs.len()
}
fn plain(xs: [i64]) -> i64 {
    xs.len()
}
fn main() {
    let a = [1, 2, 3]
    let b = [1, 2, 3]
    // `own` and a plain parameter behave identically at runtime.
    print("{consume(a)} {plain(b)}")
}
```

## `E-PAD-CYCLE.gr`

```grasm
//! tag: E-PAD-CYCLE
//! expect: output
//! [007]
//! [ababx]
//! [x-=-=-]
//! [wide]
fn main() {
    print("[{"7".pad_start(3, "0")}]")
    // A multi-character fill repeats and is cut to the exact width.
    print("[{"x".pad_start(5, "ab")}]")
    print("[{"x".pad_end(6, "-=")}]")
    // Already wide enough: unchanged.
    print("[{"wide".pad_start(2, "-")}]")
}
```

## `E-PANIC-EXIT.gr`

```grasm
//! tag: E-PANIC-EXIT
//! expect: panic
//! deliberate
fn main() {
    panic("deliberate")
}
```

## `E-PAR-FILTER-ORDER.gr`

```grasm
//! tag: E-PAR-FILTER-ORDER
//! expect: output
//! [5, 8, 9, 4]
//! true
//! true
fn main() {
    // Survivors come back in input order, not completion order.
    print("{[5, 2, 8, 1, 9, 4].par_filter(|x| x > 3)}")

    // Enough elements to cross the threshold where workers are used.
    var xs: [i64] = []
    for i in 0..20000 {
        xs.push(i)
    }
    print("{xs.par_filter(|x| x % 7 == 3) == xs.filter(|x| x % 7 == 3)}")

    let none: [i64] = []
    print("{none.par_filter(|x| x > 0).is_empty()}")
}
```

## `E-PAR-MAP-ORDER.gr`

```grasm
//! tag: E-PAR-MAP-ORDER
//! expect: output
//! true
//! [1, 4, 9, 16, 25]
//! true
fn main() {
    // Enough elements to cross the threshold where workers are used, so this
    // exercises the threaded path rather than the sequential fallback.
    var xs: [i64] = []
    for i in 0..20000 {
        xs.push(i)
    }
    // Same answer as `map`, in input order, however many cores ran it.
    print("{xs.par_map(|x| x * 2) == xs.map(|x| x * 2)}")

    print("{[1, 2, 3, 4, 5].par_map(|x| x * x)}")

    let none: [i64] = []
    print("{none.par_map(|x| x + 1).is_empty()}")
}
```

## `E-PARALLEL-JOIN.gr`

```grasm
//! tag: E-PARALLEL-JOIN
//! expect: output
//! 3
fn main() {
    var done: [i64] = []
    parallel {
        spawn {
            done.push(1)
        }
        spawn {
            done.push(2)
        }
        spawn {
            done.push(3)
        }
    }
    print("{done.len()}")
}
```

## `E-POW-NEG.gr`

```grasm
//! tag: E-POW-NEG
//! expect: panic
//! negative integer exponent
fn main() {
    let e = 0 - 1
    print("{2 ^ e}")
}
```

## `E-RECURSION-DEPTH.gr`

```grasm
//! tag: E-RECURSION-DEPTH
//! expect: output
//! 10000
//! 10000
// Ten thousand nested calls is the guaranteed floor, so it has to work in
// both engines. The interpreter recurses through its host's stack, and used
// to exhaust it — aborting with no output — a thousand calls before its own
// depth guard could report anything.
fn down(n: i64) -> i64 {
    if n <= 0 {
        0
    } else {
        down(n - 1) + 1
    }
}

/// Mutual recursion counts against the same bound.
fn ping(n: i64) -> i64 {
    if n <= 0 {
        0
    } else {
        pong(n - 1) + 1
    }
}

fn pong(n: i64) -> i64 {
    if n <= 0 {
        0
    } else {
        ping(n - 1) + 1
    }
}

fn main() {
    print("{down(10000)}")
    print("{ping(10000)}")
}
```

## `E-SEARCH-SHORTCIRCUIT.gr`

```grasm
//! tag: E-SEARCH-SHORTCIRCUIT
//! expect: output
//! any saw 1
//! any saw 2
//! true
//! all saw 1
//! all saw 2
//! all saw 3
//! false
fn main() {
    let xs = [1, 2, 3, 4]
    let hit = xs.any(|x| {
        print("any saw {x}")
        x == 2
    })
    print("{hit}")
    let ok = xs.all(|x| {
        print("all saw {x}")
        x < 3
    })
    print("{ok}")
}
```

## `E-SHORTCIRCUIT.gr`

```grasm
//! tag: E-SHORTCIRCUIT
//! expect: output
//! false
//! true
fn boom(inout hit: bool) -> bool {
    hit = true
    true
}
fn main() {
    var hit = false
    let r = false and boom(hit)
    print("{hit}")
    let r2 = true or boom(hit)
    print("{r2}")
}
```

## `E-SPAWN-ORDER.gr`

```grasm
//! tag: E-SPAWN-ORDER
//! expect: output
//! abc
fn main() {
    var log = ""
    parallel {
        spawn {
            log += "a"
        }
        spawn {
            log += "b"
        }
        spawn {
            log += "c"
        }
    }
    print(log)
}
```

## `E-SPAWN-SHARED.gr`

```grasm
//! tag: E-SPAWN-SHARED
//! expect: output
//! 30
fn main() {
    let base = 10
    var total = 0
    parallel {
        spawn {
            total += base
        }
        spawn {
            total += base * 2
        }
    }
    print("{total}")
}
```

## `E-STR-CHARS.gr`

```grasm
//! tag: E-STR-CHARS
//! expect: output
//! 3 3
fn main() {
    // Characters, not bytes: each of these is multi-byte in UTF-8.
    let s = "héllo"
    print("{"abc".len()} {s.substr(0, 3).len()}")
}
```

## `E-STR-COMPARE.gr`

```grasm
//! tag: E-STR-COMPARE
//! expect: output
//! true false
fn main() {
    print("{"abc" < "abd"} {"b" < "a"}")
}
```

## `E-STR-CONCAT.gr`

```grasm
//! tag: E-STR-CONCAT
//! expect: output
//! hello, world
fn main() {
    print("hello, " + "world")
}
```

## `E-TASK-PANIC.gr`

```grasm
//! tag: E-TASK-PANIC
//! expect: panic
//! task exploded
fn main() {
    parallel {
        spawn {
            panic("task exploded")
        }
        spawn {
            print("must not run")
        }
    }
}
```

## `E-TEST-PROPERTY.gr`

```grasm
//! tag: E-TEST-PROPERTY
//! expect: test-output
//! test "addition commutes" ... ok (500 cases)
//! test "every int is tiny" ... FAILED
test "addition commutes" (a: i64, b: i64) {
    assert(a + b == b + a)
}

// A property passes only if *every* generated case holds.
test "every int is tiny" (n: i64) {
    assert(n < 3)
}

fn main() {
}
```

## `E-TEST-SEED.gr`

```grasm
//! tag: E-TEST-SEED
//! expect: test-output
//! counterexample: n = 1
// Generation is a pure function of the seed, so this report is identical on
// every run and every machine.
test "no positive ints" (n: i64) {
    assert(n <= 0)
}

fn main() {
}
```

## `E-TEST-SHRINK.gr`

```grasm
//! tag: E-TEST-SHRINK
//! expect: test-output
//! counterexample: n = 3
//! counterexample: xs = [0, 0]
// The generator produces large values, but the reported counterexample is the
// smallest one that still fails.
test "under three" (n: i64) {
    if n > 0 {
        assert(n < 3)
    }
}

test "at most one element" (xs: [i64]) {
    assert(xs.len() < 2)
}

fn main() {
}
```

## `E-VALUE-SEMANTICS.gr`

```grasm
//! tag: E-VALUE-SEMANTICS
//! expect: output
//! 2 3
//! 1 99
type Box = { n: i64 }
fn main() {
    var a = [1, 2]
    var b = a
    b.push(3)
    print("{a.len()} {b.len()}")

    var p = Box{ n: 1 }
    var q = p
    q.n = 99
    print("{p.n} {q.n}")
}
```

## `G-ARM-CONTROL.gr`

```grasm
//! tag: G-ARM-CONTROL
//! expect: output
//! 3
fn main() {
    var n = 0
    for i in 0..10 {
        match i {
            3 -> break
            _ -> n = i
        }
    }
    print("{n + 1}")
}
```

## `G-BENCH-CONTEXTUAL.gr`

```grasm
//! tag: G-BENCH-CONTEXTUAL
//! expect: output
//! 42
fn bench(n: i64) -> i64 {
    n * 2
}

bench "doubling" {
    let x = bench(1)
    assert(x == 2)
}

fn main() {
    // `bench` is only a keyword before a string at declaration position.
    print("{bench(21)}")
}
```

## `G-DECL-ORDER.gr`

```grasm
//! tag: G-DECL-ORDER
//! expect: output
//! 20
// `main` calls a function declared after it, and uses a type declared later.
fn main() {
    let b = Box{ n: 10 }
    print("{double(b)}")
}
fn double(b: Box) -> i64 {
    b.n * 2
}
type Box = { n: i64 }
```

## `G-NEST-LIMIT.gr`

```grasm
//! tag: G-NEST-LIMIT
//! expect: output
//! ok
fn main() {
    // Deep but within the 64-level cap.
    let x = ((((((((((1))))))))))
    if x == 1 {
        print("ok")
    }
}
```

## `G-PAT-BIND.gr`

```grasm
//! tag: G-PAT-BIND
//! expect: output
//! none-case bound-42
type S = | Empty | Full(v: i64)
fn describe(s: S) -> str {
    match s {
        Empty -> "none-case"
        Full(v) -> "bound-{v}"
    }
}
fn main() {
    print("{describe(Empty)} {describe(Full(42))}")
}
```

## `G-PIPE.gr`

```grasm
//! tag: G-PIPE
//! expect: output
//! 12
fn double_all(xs: [i64]) -> [i64] {
    xs.map(|x| x * 2)
}
fn total(xs: [i64]) -> i64 {
    xs.sum()
}
fn main() {
    print("{[1, 2, 3] |> double_all |> total}")
}
```

## `G-STMT-LAST.gr`

```grasm
//! tag: G-STMT-LAST
//! expect: output
//! 9
fn compute() -> i64 {
    let a = 4
    let b = 5
    a + b
}
fn main() {
    print("{compute()}")
}
```

## `G-STMT-TERM.gr`

```grasm
//! tag: G-STMT-TERM
//! expect: error
//! expected end of line after statement
fn main() {
    let a = 1 let b = 2
}
```

## `G-STRUCT-AMBIG.gr`

```grasm
//! tag: G-STRUCT-AMBIG
//! expect: output
//! taken
type P = { ok: bool }
fn main() {
    let flag = true
    // `flag {` is a condition + block, not a struct literal.
    if flag {
        print("taken")
    }
    // Parenthesize to build a struct in that position.
    if (P{ ok: true }).ok {
        // reachable
    }
}
```

## `G-TEST-PARAMS.gr`

```grasm
//! tag: G-TEST-PARAMS
//! expect: error
//! supplied by the runner
test "the runner owns its inputs" (inout n: i64) {
    n = n + 1
}
fn main() {
}
```

## `G-TYPE-UNIT.gr`

```grasm
//! tag: G-TYPE-UNIT
//! expect: output
//! done
fn noop() -> () {
}
fn main() {
    noop()
    print("done")
}
```

## `L-BOOL.gr`

```grasm
//! tag: L-BOOL
//! expect: output
//! true false
fn main() {
    print("{true} {false}")
}
```

## `L-COMMENT-BLOCK.gr`

```grasm
//! tag: L-COMMENT-BLOCK
//! expect: output
//! 1
fn main() {
    /* outer /* nested */ still a comment */
    print("1")
}
```

## `L-COMMENT-LINE.gr`

```grasm
//! tag: L-COMMENT-LINE
//! expect: output
//! 1
fn main() {
    // a whole-line comment
    print("1") // and a trailing one
}
```

## `L-CONTEXTUAL.gr`

```grasm
//! tag: L-CONTEXTUAL
//! expect: output
//! 5 7
fn spawn(n: i64) -> i64 {
    n + 2
}
fn main() {
    // `spawn`/`parallel` are ordinary names when no block follows.
    let parallel = 5
    print("{parallel} {spawn(parallel)}")
}
```

## `L-DISPLAY.gr`

```grasm
//! tag: L-DISPLAY
//! expect: output
//! -7
//! 1.5
//! 1000000.0
//! true
//! b c
//! [1, 2, 3]
//! [a, b c]
//! [[1, 2], [3]]
//! (1, one, true)
//! {k: 1, z: 2}
//! P{x: 3, y: hi}
//! A
//! B(7)
//! Some(1)
//! None
// Every type's rendering, because a program's output is what its correctness
// is judged on. Strings are unquoted even inside a collection, so `[a, b c]`
// is two elements and the second contains a space.
type P = { x: i64, y: str }

type S =
    | A
    | B(n: i64)

fn main() {
    print("{-7}")
    print("{1.5}")
    print("{1000000.0}")
    print("{true}")
    print("{"b c"}")
    print("{[1, 2, 3]}")
    print("{["a", "b c"]}")
    print("{[[1, 2], [3]]}")
    print("{(1, "one", true)}")
    var m = map()
    m.set("z", 2)
    m.set("k", 1)
    print("{m}")
    print("{P{ x: 3, y: "hi" }}")
    print("{A}")
    print("{B(7)}")
    print("{[1].first()}")
    print("{[1].get(9)}")
}
```

## `L-ENCODING.gr`

```grasm
//! tag: L-ENCODING
//! expect: output
//! héllo ✓ 日本語
fn main() {
    // Source is UTF-8; strings and comments may hold any Unicode: ✓ 日本語
    print("héllo ✓ 日本語")
}
```

## `L-ESCAPE.gr`

```grasm
//! tag: L-ESCAPE
//! expect: output
//! a	b
//! quote" brace{ close}
fn main() {
    print("a\tb")
    print("quote\" brace\{ close\}")
}
```

## `L-FLOAT.gr`

```grasm
//! tag: L-FLOAT
//! expect: output
//! 3.14 1000000000.0 0.0025
fn main() {
    print("{3.14} {1e9} {2.5e-3}")
}
```

## `L-IDENT.gr`

```grasm
//! tag: L-IDENT
//! expect: output
//! 6
fn main() {
    let snake_case = 1
    let _leading = 2
    let with2digits = 3
    print("{snake_case + _leading + with2digits}")
}
```

## `L-INT.gr`

```grasm
//! tag: L-INT
//! expect: output
//! 255 10 1000000
fn main() {
    print("{0xFF} {0b1010} {1_000_000}")
}
```

## `L-INTERP.gr`

```grasm
//! tag: L-INTERP
//! expect: output
//! n=3 sum=7 nested=YES
fn main() {
    let n = 3
    print("n={n} sum={3 + 4} nested={if n > 1 { "YES" } else { "no" }}")
}
```

## `L-KEYWORD.gr`

```grasm
//! tag: L-KEYWORD
//! expect: error
//! expected an identifier
fn main() {
    // `type` is reserved and cannot name a variable.
    let type = 1
}
```

## `L-LIB-FREE.gr`

```grasm
//! tag: L-LIB-FREE
//! expect: output
//! 0 true
fn main() {
    // `args()` is empty with no `--` arguments; a fresh map has no entries.
    var m = map()
    print("{args().len()} {m.is_empty()}")
    assert(1 + 1 == 2)
    assert_eq(2 * 2, 4)
}
```

## `L-LIB-LIST.gr`

```grasm
//! tag: L-LIB-LIST
//! expect: output
//! 3 false 3 -1 3 2
//! [1, 2, 3] 4 [3, 2, 1] [1, 2] true 1-2-3
//! 6 1 3
//! [2, 4, 6] [2, 3, 4] [2, 3]
//! true true 2
fn main() {
    var xs = [3, 1, 2]
    print("{xs.len()} {xs.is_empty()} {xs[0]} {xs.get(9) ?? 0 - 1} {xs.first() ?? 0} {xs.last() ?? 0}")
    xs.push(4)
    let popped = xs.pop() ?? 0
    xs.sort()
    // `join` takes [str], so numbers are mapped to text first.
    print("{xs} {popped} {xs.rev()} {xs.slice(0, 2)} {xs.contains(2)} {xs.map(|v| v.to_str()).join("-")}")
    print("{xs.sum()} {xs.min() ?? 0} {xs.max() ?? 0}")
    print("{xs.map(|v| v * 2)} {xs.par_map(|v| v + 1)} {xs.filter(|v| v > 1)}")
    print("{xs.any(|v| v > 2)} {xs.all(|v| v > 0)} {xs.find(|v| v > 1) ?? 0}")
}
```

## `L-LIB-MAP.gr`

```grasm
//! tag: L-LIB-MAP
//! expect: output
//! 2 false 1 true [a, b] [1, 2]
//! true 1
fn main() {
    var m = map()
    m.set("b", 2)
    m.set("a", 1)
    print("{m.len()} {m.is_empty()} {m.get("a") ?? 0} {m.has("b")} {m.keys()} {m.values()}")
    let gone = m.delete("b")
    print("{gone} {m.len()}")
}
```

## `L-LIB-NUM.gr`

```grasm
//! tag: L-LIB-NUM
//! expect: output
//! 4 2.0 3.0 3.0 3.0
//! 3 5 7 3 2.0
fn main() {
    print("{(0 - 4).abs()} {(2.5).floor()} {(2.1).ceil()} {(2.6).round()} {(9.0).sqrt()}")
    // Conversion is by method: as_f64 widens, as_i64 truncates.
    print("{(3).min(5)} {(3).max(5)} {(7).to_str()} {(3.7).as_i64()} {(2).as_f64()}")
}
```

## `L-LIB-OPTION.gr`

```grasm
//! tag: L-LIB-OPTION
//! expect: output
//! false 3 true 0
fn main() {
    let xs = [3, 1]
    let o = xs.get(0)
    print("{o.is_none()} {o.unwrap_or(0)} {xs.get(99).is_none()} {xs.get(99) ?? 0}")
}
```

## `L-LIB-STR.gr`

```grasm
//! tag: L-LIB-STR
//! expect: output
//! 11 false
//! ell abc 2
//! true true true 6
//! HeLLo,WorLd x HELLO,WORLD hello,world abab
//! 007 7.. 2 5 2.5
fn main() {
    let s = "Hello,World"
    print("{s.len()} {s.is_empty()}")
    // substr is (start, length) — not (start, end).
    print("{s.substr(1, 3)} {"abcdef".substr(0, 3)} {s.split(",").len()}")
    print("{s.contains("World")} {s.starts_with("He")} {s.ends_with("ld")} {s.index_of("W") ?? 0 - 1}")
    print("{s.replace("l", "L")} {"  x ".trim()} {s.to_upper()} {s.to_lower()} {"ab".repeat(2)}")
    print("{"7".pad_start(3, "0")} {"7".pad_end(3, ".")} {"a\nb".lines().len()} {"5".parse_int() ?? 0} {"2.5".parse_float() ?? 0.0}")
}
```

## `L-MAXMUNCH.gr`

```grasm
//! tag: L-MAXMUNCH
//! expect: output
//! true
fn f() -> i64 {
    1
}
fn main() {
    // `->` is one token; `<=` is one token.
    print("{f() <= 1}")
}
```

## `L-NEWLINE.gr`

```grasm
//! tag: L-NEWLINE
//! expect: output
//! 6
fn main() {
    let a = 1


    let b = 2
    // Newlines are insignificant inside ( ) and [ ].
    let xs = [
        a,
        b,
        3,
    ]
    print("{xs.sum()}")
}
```

## `L-STRING.gr`

```grasm
//! tag: L-STRING
//! expect: error
//! unterminated string
fn main() {
    let s = "no closing quote
}
```

## `L-UFCS.gr`

```grasm
//! tag: L-UFCS
//! expect: output
//! 10
//! 10
//! 7
//! 6
//! 3
// A free function whose first parameter accepts the receiver may be called in
// method position. The two spellings are the same call.
fn double(n: i64) -> i64 {
    n * 2
}

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn total(xs: [i64]) -> i64 {
    xs.sum()
}

/// A user function named `len` does not shadow the built-in `len` method:
/// built-ins are resolved first.
fn len(s: str) -> i64 {
    999
}

fn main() {
    print("{double(5)}")
    print("{5.double()}")
    print("{3.add(4)}")
    print("{[1, 2, 3].total()}")
    print("{"abc".len()}")
}
```

## `L-WHITESPACE.gr`

```grasm
//! tag: L-WHITESPACE
//! expect: output
//! 3
fn main() {
	let    a	=	1
	let  b  =  2
	print("{a+b}")
}
```

## `T-ARM-TYPES.gr`

```grasm
//! tag: T-ARM-TYPES
//! expect: error
//! type mismatch
fn f(n: i64) -> i64 {
    match n {
        0 -> 1
        _ -> "not a number"
    }
}
fn main() {
    print("{f(0)}")
}
```

## `T-COPY.gr`

```grasm
//! tag: T-COPY
//! expect: output
//! 6 6
fn consume(own n: i64) -> i64 {
    n * 2
}
fn main() {
    let n = 3
    // Scalars copy, so `n` is still usable after an `own` call.
    print("{consume(n)} {consume(n)}")
}
```

## `T-EFFECT-BUILTIN.gr`

```grasm
//! tag: T-EFFECT-BUILTIN
//! expect: output
//! io and async are built in
fn writes() / {io} {
    print("io and async are built in")
}
fn concurrent() / {async} {
    parallel {
        spawn {
            let x = 1
        }
    }
}
fn main() {
    writes()
    concurrent()
}
```

## `T-EFFECT-DECL.gr`

```grasm
//! tag: T-EFFECT-DECL
//! expect: output
//! 99
effect Cfg {
    fn value() -> i64
}
fn main() {
    // Declaring `Cfg` also declares the handler type `CfgHandler`.
    with Cfg handled_by CfgHandler{ value: || 99 } {
        print("{Cfg.value()}")
    }
}
```

## `T-EFFECT-DISCHARGE.gr`

```grasm
//! tag: T-EFFECT-DISCHARGE
//! expect: error
//! unhandled effect
effect Clock {
    fn now() -> i64
}
fn main() {
    print("{Clock.now()}")
}
```

## `T-EFFECT-ROW.gr`

```grasm
//! tag: T-EFFECT-ROW
//! expect: error
//! not in its declared row
effect Clock {
    fn now() -> i64
}
fn quiet() / {} {
    let t = Clock.now()
}
fn main() {
    with Clock handled_by ClockHandler{ now: || 1 } {
        quiet()
    }
}
```

## `T-EMPTY-LIT.gr`

```grasm
//! tag: T-EMPTY-LIT
//! expect: output
//! 1 0
type Bag = { items: [str] }
fn main() {
    var xs: [i64] = []
    xs.push(1)
    let b = Bag{ items: [] }
    print("{xs.len()} {b.items.len()}")
}
```

## `T-ERR-SUBSET.gr`

```grasm
//! tag: T-ERR-SUBSET
//! expect: error
//! only declares
type A = | AErr
type B = | BErr
fn wide() -> i64 ! (A | B) {
    1
}
fn narrow() -> i64 ! A {
    try wide()
}
fn main() {
    match narrow() {
        Ok(v) -> print("{v}")
        Err(e) -> print("err")
    }
}
```

## `T-FAIL.gr`

```grasm
//! tag: T-FAIL
//! expect: error
//! declares no error type
type E = | Bad
fn no_error_clause() -> i64 {
    fail(Bad)
}
fn main() {
    print("{no_error_clause()}")
}
```

## `T-FALLIBLE-UNHANDLED.gr`

```grasm
//! tag: T-FALLIBLE-UNHANDLED
//! expect: error
//! can fail
type E = | Bad
fn risky() -> i64 ! E {
    1
}
fn main() {
    let v = risky()
}
```

## `T-FALLIBLE.gr`

```grasm
//! tag: T-FALLIBLE
//! expect: output
//! ok 5
//! err Bad
type E = | Bad
fn maybe(fail_it: bool) -> i64 ! E {
    if fail_it {
        fail(Bad)
    }
    5
}
fn main() {
    match maybe(false) {
        Ok(v) -> print("ok {v}")
        Err(e) -> print("err {e}")
    }
    match maybe(true) {
        Ok(v) -> print("ok {v}")
        Err(e) -> print("err {e}")
    }
}
```

## `T-FN-TYPE-ROW.gr`

```grasm
//! tag: T-FN-TYPE-ROW
//! expect: error
//! expects a function that performs nothing
// `fn(i64) -> i64` states the empty row, so a function that prints does not
// fit it. Without this check a function value would be a way to carry an
// effect past a declared row — which is exactly what it used to be.
fn shout(n: i64) -> i64 {
    print("{n}")
    n
}

fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn main() {
    print("{apply(shout, 1)}")
}
```

## `T-FN-TYPE.gr`

```grasm
//! tag: T-FN-TYPE
//! expect: output
//! 10
//! 6
//! 12
//! [2, 4, 6]
// A program declaring its own higher-order function, which needs a written
// function type: there is no body to infer a parameter's type from.
fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn twice(f: fn(i64) -> i64, x: i64) -> i64 {
    f(f(x))
}

/// Takes a function and a list, which is `map` written in the language.
fn over(f: fn(i64) -> i64, xs: [i64]) -> [i64] {
    xs.map(f)
}

fn double(n: i64) -> i64 {
    n * 2
}

fn main() {
    print("{apply(double, 5)}")
    // A closure takes its parameter types from the parameter it is passed to.
    print("{apply(|n| n + 1, 5)}")
    print("{twice(double, 3)}")
    print("{over(double, [1, 2, 3])}")
}
```

## `T-FN-VALUE.gr`

```grasm
//! tag: T-FN-VALUE
//! expect: output
//! 8
//! 12
//! 10 15
//! -3 -1
fn double(n: i64) -> i64 {
    n * 2
}

fn triple(n: i64) -> i64 {
    n * 3
}

fn main() {
    // A function's name used as a value; called through the binding.
    let f = double
    print("{f(4)}")

    // A closure bound to a name. Its parameter type comes from the call.
    let times3 = |x| x * 3
    print("{times3(4)}")

    // Both kinds share one representation, so a list holds either.
    let stages = [double, triple]
    var out = ""
    for stage in stages {
        out = out + "{stage(5)} "
    }
    print(out.trim())

    // Negative literals are written directly — no need for `0 - n`.
    let below = -3
    print("{below} {[1, 2].get(9) ?? -1}")
}
```

## `T-GENERIC-FN.gr`

```grasm
//! tag: T-GENERIC-FN
//! expect: output
//! 1 a
fn first[T](xs: [T]) -> Option[T] {
    xs.get(0)
}
fn main() {
    print("{first([1, 2]) ?? 0} {first(["a"]) ?? "none"}")
}
```

## `T-GENERIC-INST.gr`

```grasm
//! tag: T-GENERIC-INST
//! expect: output
//! 2 2
fn count[T](xs: [T]) -> i64 {
    xs.len()
}
fn main() {
    // One generic body, two instantiations.
    print("{count([1, 2])} {count(["a", "b"])}")
}
```

## `T-INFER-LOCAL.gr`

```grasm
//! tag: T-INFER-LOCAL
//! expect: output
//! 3 2.5 hi true
fn main() {
    // No annotations: each type comes from the initializer.
    let i = 3
    let f = 2.5
    let s = "hi"
    let b = i > 1
    print("{i} {f} {s} {b}")
}
```

## `T-INOUT-EXCLUSIVE.gr`

```grasm
//! tag: T-INOUT-EXCLUSIVE
//! expect: error
//! passed as `inout` twice
fn both(inout a: [i64], inout b: [i64]) {
    a.push(1)
}
fn main() {
    var xs = [1]
    both(xs, xs)
}
```

## `T-INOUT-MUTABLE.gr`

```grasm
//! tag: T-INOUT-MUTABLE
//! expect: error
//! it is immutable
fn bump(inout n: i64) {
    n += 1
}
fn main() {
    let n = 1
    bump(n)
}
```

## `T-INOUT-PLACE.gr`

```grasm
//! tag: T-INOUT-PLACE
//! expect: error
//! must be a variable
fn bump(inout n: i64) {
    n += 1
}
fn main() {
    bump(41)
}
```

## `T-INT-ALIAS.gr`

```grasm
//! tag: T-INT-ALIAS
//! expect: output
//! 3 5.0
fn add(a: i32, b: u8) -> int {
    a + b
}
fn main() {
    print("{add(1, 2)} {5.0}")
}
```

## `T-INT-TO-FLOAT.gr`

```grasm
//! tag: T-INT-TO-FLOAT
//! expect: output
//! 3.0 6.0
fn main() {
    let x: f64 = 3
    print("{x} {x * 2}")
}
```

## `T-LET-IMMUTABLE.gr`

```grasm
//! tag: T-LET-IMMUTABLE
//! expect: error
//! it is immutable
fn main() {
    let x = 1
    x = 2
}
```

## `T-LIST.gr`

```grasm
//! tag: T-LIST
//! expect: output
//! [1, 2, 3] 3 1
fn main() {
    var xs: [i64] = [1, 2]
    xs.push(3)
    print("{xs} {xs.len()} {xs[0]}")
}
```

## `T-MAP-KEY.gr`

```grasm
//! tag: T-MAP-KEY
//! expect: error
//! map keys must be
fn main() {
    var m: Map[[i64], str] = map()
}
```

## `T-MAP.gr`

```grasm
//! tag: T-MAP
//! expect: output
//! 2 7 true
fn main() {
    var m: Map[str, i64] = map()
    m.set("a", 7)
    m.set("b", 8)
    print("{m.len()} {m.get("a") ?? 0} {m.has("b")}")
}
```

## `T-MATCH-EXHAUSTIVE.gr`

```grasm
//! tag: T-MATCH-EXHAUSTIVE
//! expect: error
//! not exhaustive
type S = | A | B | C
fn f(s: S) -> i64 {
    match s {
        A -> 1
        B -> 2
    }
}
fn main() {
    print("{f(A)}")
}
```

## `T-MATCH-GUARD.gr`

```grasm
//! tag: T-MATCH-GUARD
//! expect: error
//! not exhaustive
fn f(b: bool) -> i64 {
    match b {
        true -> 1
        false if b -> 0
    }
}
fn main() {
    print("{f(true)}")
}
```

## `T-MODULE-DEP.gr`

```grasm
//! tag: T-MODULE-DEP
//! expect: error
//! cannot load module
// A bare name is tried as a sibling source file first, then as a dependency
// the manifest declares. Here it is neither: there is no `undeclared_package.gr`
// beside this file and no `grasm.toml` naming it, so it does not resolve and
// the error names both places it looked.
//
// (A bare name *does* resolve when either one exists — `import helper` finds
// `helper.gr` next to the importing file with no manifest involved.)
import undeclared_package

fn main() {
    print("{undeclared_package.f()}")
}
```

## `T-MODULE-NS.gr`

```grasm
//! tag: T-MODULE-NS
//! expect: error
//! duplicate type name
type Thing = { a: i64 }
type Thing = { b: i64 }
fn main() {
}
```

## `T-MODULE-PUB.gr`

```grasm
//! tag: T-MODULE-PUB
//! expect: output
//! 4
fn double(n: i64) -> i64 {
    n * 2
}
fn main() {
    print("{double(2)}")
}
```

## `T-MODULE-RESOLVE.gr`

```grasm
//! tag: T-MODULE-RESOLVE
//! expect: error
//! cannot load module
// `import a.b` resolves to `a/b.gr` beside this file. Nothing declares one
// here, so the import fails rather than silently resolving to something else.
import missing.helper

fn main() {
    print("{helper.value()}")
}
```

## `T-MOVE-LOOP.gr`

```grasm
//! tag: T-MOVE-LOOP
//! expect: error
//! next loop iteration
fn consume(own s: str) -> i64 {
    s.len()
}
fn main() {
    let name = "ada"
    for _i in 0..3 {
        let n = consume(name)
    }
}
```

## `T-NO-NULL.gr`

```grasm
//! tag: T-NO-NULL
//! expect: output
//! 10 -1
fn main() {
    let xs = [10]
    // Absence is Option, never null.
    print("{xs.get(0) ?? 0} {xs.get(9) ?? 0 - 1}")
}
```

## `T-OPTION.gr`

```grasm
//! tag: T-OPTION
//! expect: output
//! 5 none
fn describe(o: Option[i64]) -> str {
    match o {
        Some(v) -> "{v}"
        None -> "none"
    }
}
fn main() {
    print("{describe(Some(5))} {describe(None)}")
}
```

## `T-PAR-MAP-PURE.gr`

```grasm
//! tag: T-PAR-MAP-PURE
//! expect: error
//! effect-free
fn main() {
    let xs = [1, 2, 3]
    let ys = xs.par_map(|x| {
        print("{x}")
        x
    })
    print("{ys}")
}
```

## `T-PARAM-INOUT.gr`

```grasm
//! tag: T-PARAM-INOUT
//! expect: output
//! 2
fn bump(inout n: i64) {
    n += 1
}
fn main() {
    var n = 1
    bump(n)
    print("{n}")
}
```

## `T-PARAM-LET.gr`

```grasm
//! tag: T-PARAM-LET
//! expect: output
//! 2 2
fn size(xs: [i64]) -> i64 {
    xs.len()
}
fn main() {
    let xs = [1, 2]
    // A plain parameter does not consume the caller's binding.
    print("{size(xs)} {size(xs)}")
}
```

## `T-PARAM-OWN.gr`

```grasm
//! tag: T-PARAM-OWN
//! expect: output
//! 3
fn consume(own s: str) -> i64 {
    s.len()
}
fn main() {
    let name = "ada"
    print("{consume(name)}")
}
```

## `T-PRIM.gr`

```grasm
//! tag: T-PRIM
//! expect: output
//! 42 2.5 true text
fn main() {
    let i: i64 = 42
    let f: f64 = 2.5
    let b: bool = true
    let s: str = "text"
    print("{i} {f} {b} {s}")
}
```

## `T-STRUCT.gr`

```grasm
//! tag: T-STRUCT
//! expect: output
//! 3 4
type Point = { x: i64, y: i64 }
fn main() {
    var p = Point{ x: 3, y: 0 }
    p.y = 4
    print("{p.x} {p.y}")
}
```

## `T-SUM.gr`

```grasm
//! tag: T-SUM
//! expect: output
//! circle-2.0 dot
type Shape = | Circle(r: f64) | Dot
fn name(s: Shape) -> str {
    match s {
        Circle(r) -> "circle-{r}"
        Dot -> "dot"
    }
}
fn main() {
    print("{name(Circle(2.0))} {name(Dot)}")
}
```

## `T-TASK-CAPTURE.gr`

```grasm
//! tag: T-TASK-CAPTURE
//! expect: error
//! declared inside the `parallel` block
fn main() {
    var log: [str] = []
    parallel {
        for i in 0..3 {
            spawn {
                log.push("{i}")
            }
        }
    }
    print(log.join(","))
}
```

## `T-TRY.gr`

```grasm
//! tag: T-TRY
//! expect: error
//! declares no error type
type E = | Bad
fn risky() -> i64 ! E {
    1
}
// `main` and tests have an open error slot; an ordinary function does not.
fn caller() -> i64 {
    try risky()
}
fn main() {
    print("{caller()}")
}
```

## `T-TUPLE.gr`

```grasm
//! tag: T-TUPLE
//! expect: output
//! 1 is one
fn main() {
    let pair = (1, "one")
    match pair {
        (n, name) -> print("{n} is {name}")
    }
}
```

## `T-USE-AFTER-MOVE.gr`

```grasm
//! tag: T-USE-AFTER-MOVE
//! expect: error
//! use of moved value
fn consume(own s: str) -> i64 {
    s.len()
}
fn main() {
    let name = "ada"
    let n = consume(name)
    print(name)
}
```

## `T-VAR-MUTABLE.gr`

```grasm
//! tag: T-VAR-MUTABLE
//! expect: output
//! 3 [1, 2]
fn main() {
    var n = 1
    n = 3
    var xs = [1]
    xs.push(2)
    print("{n} {xs}")
}
```
