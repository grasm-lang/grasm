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
ref  try  type  unsafe  var  while  with
```

**[L-CONTEXTUAL]** `parallel`, `spawn` and `region` are *contextual* keywords:
they introduce a scope only when a `{` follows. In every other position they
are ordinary identifiers, so existing code using those names keeps compiling.
`region` was briefly a reserved word and it broke a program in this repository
that used it as a field name, which is the compatibility this rule exists for.

## 4. Literals

**[L-INT]** Integer literals are decimal (`42`), hexadecimal (`0xFF`), or
binary (`0b1010`). Underscores may separate digits (`1_000_000`) and are
ignored. Integers are 64-bit signed.

**[L-FLOAT]** Float literals contain a fractional part (`3.14`) or an exponent
(`1e9`, `2.5e-3`). `1..5` is a range, not a float.

**[L-BOOL]** `true` and `false`.

**[L-STRING]** String literals are delimited by `"`. They may not span lines.

**[L-RAW-STRING]** `"""…"""` is a **raw block**: every character between the
delimiters is taken literally, newlines included. No escapes, no
interpolation. A newline immediately after the opening delimiter is dropped so
the content can begin on its own line.

This exists because embedding a document in a program — a JSON fixture, a CSV
sample, an expected report — otherwise means a list of strings joined on
`"\n"` with every `{` and `}` written as `\{` and `\}`, which is more noise
than data. The one thing a raw block cannot contain is `"""`.

**[L-ESCAPE]** Recognized escapes: `\n`, `\t`, `\r`, `\\`, `\"`, `\0`, `\{`,
`\}`. Any other escape is a lexical error.

**[L-INTERP]** `{` inside a string begins an interpolation whose contents are
parsed as an expression and whose value is converted to text. Braces nest. A
literal brace is written `\{`.

**[L-INTERP-ONE]** The text between `{` and `}` is exactly one expression.
Anything left over after it is an error rather than being discarded, so a typo
inside an interpolation cannot silently delete part of the output.

## 5. Operators and punctuation

```
( ) [ ] { } , . .. ..= : -> | |> + - * / % ^
= == != < <= > >= ! ?? += -= *= /= _
```

**[L-MAXMUNCH]** The lexer always takes the longest matching operator, so
`->` is one token, never `-` followed by `>`.

`!` appears only in `T ! E` (a function's error type); negation is `not`.
There is deliberately no `::`, `=>`, `@`, postfix `?`, or `?.`: paths use `.`,
match arms use `->`, and an `Option` is opened with `??` or `match`. Typing
one of them is reported by name rather than as a stray token.

## 6. Declarations

```
module      = declaration*
declaration = fn_decl | const_decl | type_decl | effect_decl | test_decl
            | bench_decl | import_decl
const_decl  = "pub"? "let" identifier ( ":" nested )? "=" expression

fn_decl     = "pub"? "fn" identifier generics? params ret? err? effects? block
generics    = "[" identifier ( "," identifier )* "]"
params      = "(" ( param ( "," param )* )? ")"
param       = ( "inout" | "own" )? identifier ":" type
ret         = "->" type
err         = "!" type
effects     = "/" "{" ( path ( "," path )* )? "}"

type_decl   = "pub"? "type" identifier generics? "=" type_body
type_body   = struct_body | sum_body | type
struct_body = "{" field ( ("," | newline) field )* "}"
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
type       = named_type | list_type | tuple_type | union_type | fn_type
             | ref_type
nested     = type ( "!" type )?
fn_type    = "fn" "(" ( nested ( "," nested )* )? ")" ( "->" nested )? effect_row?
effect_row = "/" "{" ( path ( "," path )* )? "}"
named_type = identifier type_args?
type_args  = "[" nested ( "," nested )* "]"
list_type  = "[" nested "]"
ref_type   = "ref" nested
tuple_type = "(" ( nested ( "," nested )* )? ")"
union_type = "(" nested ( "|" nested )+ ")"
```

**[G-TYPE-FALLIBLE]** `T ! E` is a type wherever a type is written *inside*
something else: a parameter, a struct or variant field, a type argument, a
tuple member, a `let` annotation, an alias. `[i64 ! E]` is what `xs.map(parse)`
produces when `parse` can fail — a thing the checker could infer and nobody
could write, so crossing a `map` with a fallible function meant declaring a
sum type by hand to carry what the language already had.

The one place the `!` is *not* part of the type is a function declaration's
return: `fn f() -> i64 ! E` gives it to the declaration, alongside the effect
row.

Writing the type is also how a fallible value is *held*. `let r = risky()` is
an error, because forgetting `try` is the far more common reading of it; the
annotation `let r: i64 ! E = risky()` says the failure is meant to be kept,
and the error names that spelling.

**[G-TYPE-NOT-QUALIFIED]** A type is named by a bare identifier, never
`module.Type`. Types share one global namespace ([T-MODULE-NS]), so the
qualifier would carry no information — and the grammar used to allow one,
which meant the only way to find out was the error.

**[G-TYPE-UNIT]** `()` is the unit type.

## 8. Statements

```
block     = "{" statement* "}"
statement = let_stmt | assign_stmt | expr_stmt | return_stmt
          | break_stmt | continue_stmt | while_stmt | for_stmt
          | with_stmt | parallel_stmt | spawn_stmt

let_stmt      = ( "let" | "var" ) ( identifier ( ":" type )? | tuple_names ) "=" expression
tuple_names   = "(" identifier ( "," identifier )* ")"
```

**[G-LET-TUPLE]** `let (a, b) = pair` binds a tuple's components by name. The
count must match the tuple's arity. `for (a, b) in pairs` is the same binding
done once per element, and means exactly `for e in pairs { let (a, b) = e … }`
— which is what iterating `entries()`, `enumerate()` or `zip()` otherwise
costs. Together these are the only destructuring forms: there is no `if let`,
and patterns elsewhere appear only in `match`.

```
assign_stmt   = place ( "=" | "+=" | "-=" | "*=" | "/=" ) expression
return_stmt   = "return" expression?
break_stmt    = "break"
continue_stmt = "continue"
while_stmt    = "while" expression block
for_stmt      = "for" ( identifier | "(" identifier ( "," identifier )* ")" )
                "in" expression block
with_stmt     = "with" path "handled_by" expression block
parallel_stmt = "parallel" block
spawn_stmt    = "spawn" block

place = identifier ( "." identifier | "[" expression "]" )*
```

**[G-STMT-TERM]** A statement is terminated by a newline, by the `}` that
closes the block it is in, or by end of file. There is no separator that lets
two statements share a line: the `}` of a *nested* block does not end the
statement that follows it, so `var x = 0 if c { }` is an error and not two
statements.

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
primary      = literal | identifier | list_lit | map_lit | tuple_lit
             | struct_lit | if_expr | match_expr | block | closure
             | unsafe_expr | region_expr | ref_expr | "(" expression ")"
unsafe_expr  = "unsafe" block
region_expr  = "region" block
ref_expr     = "ref" "(" expression ")"
list_lit     = "[" ( expression ( "," expression )* ","? )? "]"
map_lit      = "{" ( map_entry ( "," map_entry )* ","? )? "}"
map_entry    = expression ":" expression
tuple_lit    = "(" ( expression ( "," expression )+ ","? )? ")"
struct_lit   = identifier "{" struct_lit_body "}"
struct_lit_body
             = ".." expression ","? ( field_init ( ("," | newline) field_init )* )?
             | field_init ( ("," | newline) field_init )*
field_init   = identifier ":" expression
if_expr      = "if" expression block ( "else" ( if_expr | block ) )?
match_expr   = "match" expression "{" match_arm* "}"
match_arm    = alt_pattern ( "if" expression )? "->" expression
alt_pattern  = pattern ( "|" pattern )*
closure      = "|" ( identifier ( "," identifier )* )? "|" ( expression | block )
```

**[G-STRUCT-NONEMPTY]** A struct has at least one field, and a struct literal
names at least one field or a base. `{}` after a name is a *block* — that is
what makes `with C handled_by h { … }` unambiguous — so a zero-field struct
would be a type with no way to write a value of it. The rules above used to
make both the field list and the literal's contents optional, so `type E = { }`
was declared, accepted, and unbuildable, and `E{}` failed with "expected end of
line after statement". For a value that carries nothing, use a sum variant:
`type Marker = | Done`.

**[G-MATCH-ARM-EXPR]** A match arm's body is an *expression*. `for`, `while`,
`let`, `var` and `with` are statements, so they need a block: `1 -> { for i in
0..2 { … } }`, not `1 -> for i in 0..2 { … }`. The rule used to read
`( expression | statement )`, which the parser had never accepted — a grammar
that promises more than the implementation gives is a worse guide than no
grammar, because a reader trusts it and gets a parse error with no idea which
of the two is wrong.

**[G-PIPE]** `x |> f(a)` is exactly `f(x, a)`; `x |> f` is `f(x)`.

**[G-STRUCT-UPDATE]** `Point{ ..p, x: 1 }` takes every field from `p` and
overwrites the ones written after it. The base must be the same struct type,
and it is evaluated first ([E-ORDER-LTR]). With a base, no field is required;
without one, all of them are.

`p` is unchanged — this builds a new value, like every other expression
([E-VALUE-SEMANTICS]). Without it an accumulator restated all its fields at
every step, which is exactly where a wrong one hides.

**[G-MAP-BLOCK-AMBIG]** In expression position `{` opens a map literal if
what follows is `}` or an expression then `:`, and a block otherwise. The two
never both fit: a statement cannot carry a `:` where a first entry's colon
would sit. The braces of `if`, `for`, `while`, `match` and function bodies are
not in expression position and are always blocks.

A match arm's body is the one place both readings are useful, and there `{}`
is the empty *block* — an arm that does nothing, which is what somebody
writing `-> {}` meant. A non-empty `{k: v}` there is still a map.

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

**[G-NEST-LIMIT]** The depth of the syntax tree is capped. Exceeding the cap
is a diagnostic, never a crash.

The cap is on tree depth, not on how many brackets were typed, because those
are not the same thing: `1 + 1 + … + 1` and `x.max(1).max(1)…` open no
brackets and recurse nowhere in the parser, and each builds a spine as deep as
it is long. Every pass after the parser walks that spine by recursion, so
8000 terms — well inside any bracket-counting limit — aborted the whole
toolchain with `fatal runtime error: stack overflow` and nothing on stderr to
say which program did it. A folded operator, a method call, a field access and
an index each count as one level, exactly as a bracket does.

The exact number is deliberately not specified, and no diagnostic quotes one:
the cap is far above any program a person writes, and where it sits depends on
the stack the implementation gives itself. What is normative is that some cap
exists and that reaching it is reported.


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

**[T-REF-EQ]** Two references are equal when they name the same live slot —
identity, not the equality of what they point at, which is what a graph walk
needs to recognise a node it has already visited. Equality recurses, so this is
also what makes a struct or list *containing* a reference compare sensibly.

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
slot may already hold something else — released slots are reused immediately,
which is what keeps a long-lived graph from growing without bound — so the
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
there. The rule was stated for direct writes first, and two independent
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


<!-- ===== spec/semantics.md ===== -->

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

**[E-HANDLER-MISSING]** A user effect that reaches `main` or a test without a
handler is rejected *at compile time*, so the runtime's missing-handler panic is
a defensive backstop that well-typed programs never reach.

## 8. Concurrency

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
generator happened to produce.

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


<!-- ===== spec/library.md ===== -->

# Standard Library

Every operation the language provides, by receiver type. There are no imports:
these are always available.

Each `L-` clause below has a conformance program under `spec/conformance/`
that exercises what it claims. That is per clause, not per row: a table entry
whose clause is covered elsewhere can still drift, which is how `read_file`
came to be documented as returning `Option[str]` when it is fallible.

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
| `read_file(p)` | `(str) -> str ! IoError` | fallible, not `Option`; `io` ([L-LIB-IO]) |
| `write_file(p, s)` | `(str, str)` | `io` |
| `file_exists(p)` | `(str) -> bool` | `io` |

**[L-LIB-IO]** The three ways a library call reports "no": `read_file` is
**fallible** (`str ! IoError`) because a failure has a reason worth reading —
`NotFound(path)`, and so on. `env` returns `Option[str]`, because an unset
variable has no reason to give. `file_exists` returns `bool`, because the
question was already yes-or-no.

## 1a. How values are written

**[L-DISPLAY]** `print(x)` and string interpolation render every type the same
way, and this is normative because a program's output is what its correctness
is judged on.

| type | rendering | example |
|------|-----------|---------|
| `i64` | decimal | `-7` |
| `f64` | the fewest **digits** that round-trip, written out positionally — never exponent notation, and **always with a point** at every magnitude | `1.5`, `1000000.0`, `1000000000000000.0`, `-0.0`; `1e300` is 303 characters |
| `f64`, not finite | `inf`, `-inf`, `NaN` — `NaN` is the one mixed-case rendering here | `inf` |
| `()` | `()` | |
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

**[L-LIB-FIXED]** `x.to_fixed(places)` is `f64` as text with exactly that
many digits after the point, 0 to 17; anything else panics. Both engines
format through the same code, so they agree digit for digit — which a
formatter built by hand out of `round` and `to_str` cannot promise, and which
is what every report with an average needs.

A value that is not finite answers with `inf`, `-inf` or `NaN` regardless of
the requested precision, matching [L-DISPLAY].

Ties go **away from zero**, the same rule `round` uses ([L-LIB-ROUND]), so
`(2.5).to_fixed(0)` is `"3"` and `(0.125).to_fixed(2)` is `"0.13"`. A tie
means the *stored* value is exactly halfway, which is rarer than it looks:
`2.675` is really `2.67499…`, so `(2.675).to_fixed(2)` is `"2.67"`. Nothing is
scaled to decide this — scaling `2.675` by 100 lands on `267.5` and would
round it the wrong way.

**[L-LIB-REMOVE]** `xs.remove(i)` takes the element at `i` out of the list
and answers with it, shifting the rest down. Out of range panics, the way
indexing does — `pop` is the one that answers `Option`, because "the list was
empty" is a thing a caller can reasonably not know and "index 9 of 2" is not.

This is what draining a queue from the front needs. Without it a ready queue
cost `ready = ready.slice(1, ready.len())` a step, which is O(n) and reads
like the workaround it was.

**[L-LIB-SUM]** `xs.sum()` of an empty list is the zero of the element type:
`0` for `[i64]`, `0.0` for `[f64]`.

This held only in the compiled backend for a long time. The interpreter decides
int-versus-float from the elements, and an empty list has none, so an empty
float sum was `0` under `grasm run` and `0.0` from a native binary — and a
float method on the result panicked with `no method to_fixed on 0`, in the
engine this specification calls normative, on the empty-average case every
reporting program has. It was recorded here as a known deviation on the
grounds that fixing it needed the interpreter to carry element types. It did
not: the checker knows the sum is an `f64` and can say so, which is the same
answer it now gives for a promoted literal ([T-INT-TO-FLOAT-CARRIED]).

**[L-LIB-CMP]** `a.cmp(b)` answers `-1`, `0` or `1` in exactly the order
`sort` uses — [L-LIB-SORT-NAN]'s total order for floats included. It exists
because that order was specified, used internally, and callable by nothing: a
program needing it reimplemented it from `<` and `>`, got NaN wrong, and
scrambled a column in precisely the way that clause warns about. Naming a
hazard without handing over the fix is how a specification becomes a trap.

It is also what makes a *comparator* sort writable — several keys with
directions decided at run time, which a single `sort_by` key expression cannot
express.

**[L-LIB-SORT-NAN]** Floats sort by a **total** order: `-NaN` before every
number, `NaN` after every number, and `-0.0` before `0.0`. This is not IEEE
comparison, where a NaN is unordered with everything — `<` keeps that. A sort
needs an order, and calling every NaN comparison "equal" is not one: given a
non-order a sort returns a list that is not sorted *at all*, so one `nan` cell
in a file scrambled a whole column rather than misplacing one value.

**[L-LIB-ROUND]** `round` breaks ties **away from zero**: `2.5` and `3.5`
round to `3.0` and `4.0`, `-2.5` to `-3.0`, `-0.5` to `-1.0`. It is not
banker's rounding, and a report's figures depend on which it is.
`(i64_min).abs()` is `i64_min`: there is no positive `i64` that far out, and
the wrap is the same one `/` and `%` take ([E-ARITH-INT]).

**[L-LIB-NUM]**

| method | type | notes |
|--------|------|-------|
| `n.abs()` | `-> i64` / `-> f64` | |
| `n.min(m)`, `n.max(m)` | `(T) -> T` | |
| `x.floor()`, `x.ceil()`, `x.round()` | `f64 -> f64` | |
| `x.sqrt()` | `f64 -> f64` | |
| `n.to_str()` | `-> str` | the same text `print` gives, [L-DISPLAY] included ([L-LIB-TO-STR]) |
| `n.as_f64()` | `i64 -> f64` | widening |
| `x.as_i64()` | `f64 -> i64` | truncates toward zero, saturating ([L-LIB-AS-I64]) |

Conversion is by method, not by a cast or a `int(...)`-style function. A `str`
becomes a number with `parse_int` / `parse_float`, which return `Option`
because the text may not be a number.

**[L-LIB-MATH]** Trigonometry, exponentials and logarithms, on `f64`:

| method | type | notes |
|--------|------|-------|
| `x.sin()`, `x.cos()`, `x.tan()` | `f64 -> f64` | radians |
| `x.asin()`, `x.acos()`, `x.atan()` | `f64 -> f64` | radians |
| `x.atan2(y)` | `(f64) -> f64` | `x.atan2(y)` is `atan(x/y)` with the quadrant of `(y, x)` |
| `x.sinh()`, `x.cosh()`, `x.tanh()` | `f64 -> f64` | |
| `x.exp()`, `x.ln()` | `f64 -> f64` | base *e* |
| `x.log2()`, `x.log10()` | `f64 -> f64` | |
| `x.log(b)` | `(f64) -> f64` | base `b` |
| `x.cbrt()`, `x.hypot(y)` | `f64 -> f64` | |
| `x.to_radians()`, `x.to_degrees()` | `f64 -> f64` | |
| `pi()`, `e()` | `-> f64` | |

Every one is pure: no allocation, no error case, and **no capability**. That
last part is the reason they are here rather than left to `extern fn cos` — a
declared C function performs `io` ([T-EXTERN-VALUE]), so reaching libm for a
cosine put a trigonometry helper over the ceiling of any package declaring
`capabilities = []`. A pure function should not cost a capability.

`pi` and `e` are the only two builtins that yield to a function you declare
yourself: a program with its own `fn e()` keeps it. `e` is a common enough
identifier that taking it outright would break working programs.

**[L-LIB-TO-STR]** `to_str` produces exactly what `print` would. In
particular an `f64` keeps its point at every magnitude: `(1.0).to_str()` is
`"1.0"`. The interpreter used to hand the job to its host language, which drops
the point on a whole value, so it answered `"1"` where the compiled backend
answered `"1.0"` — and the documented idiom `xs.map(|v| v.to_str()).join(", ")`
printed a `[f64]` as a list of integers, which is the one thing [L-DISPLAY]
exists to prevent.

**[L-LIB-AS-I64]** `as_i64` is total: it has an answer for every `f64`, so it
never panics and never returns `Option`. A value too large for `i64` gives
`i64`'s maximum, one too small gives its minimum, and `NaN` gives `0`. That is
saturation, not wrapping — `1.0e30.as_i64()` is `9223372036854775807`, and it
being a *clamp* rather than a truncated bit pattern is the part worth knowing,
because the two disagree and only one of them stays ordered.

## 3. `str`

**[L-LIB-STR]** Lengths and offsets are in **characters, not bytes**
([E-STR-CHARS]).

**[L-LIB-TRIM-SIDES]** `trim_start` and `trim_end` remove whitespace from one
end only; `trim` removes it from both. None of the three takes an argument —
whitespace is what they trim. The one-sided pair exists because a line's
leading indentation usually carries meaning and its trailing newline does not,
and `trim` cannot tell you that.

| method | type | notes |
|--------|------|-------|
| `s.len()` | `-> i64` | characters |
| `s.is_empty()` | `-> bool` | |
| `s.substr(start, len)` | `(i64, i64) -> str` | **start and length**, not start and end; clamps past the end rather than panicking, unlike indexing |
| `s.chars()` | `-> [str]` | one single-character string per element |
| `s.lines()` | `-> [str]` | splits on newlines; no trailing empty line |
| `s.split(sep)` | `(str) -> [str]` | on an exact separator; adjacent separators give empty elements |
| `s.words()` | `-> [str]` | on runs of whitespace, with no empties ([L-LIB-WORDS]) |
| `s.contains(t)`, `s.starts_with(t)`, `s.ends_with(t)` | `(str) -> bool` | |
| `s.index_of(t)` | `(str) -> Option[i64]` | `None` when absent |
| `s.replace(from, to)` | `(str, str) -> str` | |
| `s.trim()` | `-> str` | |
| `s.trim_start()`, `s.trim_end()` | `-> str` | one side only ([L-LIB-TRIM-SIDES]) |
| `s.to_upper()`, `s.to_lower()` | `-> str` | |
| `s.repeat(n)` | `(i64) -> str` | |
| `s.pad_start(w, fill)`, `s.pad_end(w, fill)` | `(i64, str) -> str` | fill repeats and is cut to width ([E-PAD-CYCLE]) |
| `s.parse_int()` | `-> Option[i64]` | surrounding space and a leading `+` are accepted; `1_000` and `0xFF` are **not** ([L-LIB-PARSE]) |
| `s.parse_float()` | `-> Option[f64]` | accepts `inf`, `-inf`, `nan`; overflow gives an infinity, not `None` ([L-LIB-PARSE]) |
| `x.is_nan()`, `x.is_finite()` | `-> bool` | what `parse_float` makes necessary ([L-LIB-PARSE]) |
| `s.code_at(i)` | `(i64) -> i64` | Unicode code point of character `i`; panics out of range ([L-LIB-CODES]) |
| `s.codes()` | `-> [i64]` | every code point, in order |
| `n.to_char()` | `i64 -> str` | a code point back to a one-character string; panics if `n` is not one |

**[L-LIB-CODES]** `s.code_at(i)` and `s.codes()` give Unicode **code points**
(`char` scalar values), counting characters not bytes, and `n.to_char()` turns
one back into text. `code_at` panics out of range, like indexing; `to_char`
panics on a value that is not a code point, surrogates included. Together they
are what lets a program compute *on* text — hash it, encode it, parse a number
by hand — rather than only cut and compare it.

**[L-LIB-WORDS]** `s.words()` splits on runs of whitespace and yields no
empty elements, which is what reading a line of aligned columns needs.
`s.split(" ")` is the other thing — an exact separator, where two spaces in a
row mean an empty element between them — and both are worth having, because a
CSV wants the second and a log line wants the first.

**[L-LIB-PARSE]** `parse_int` and `parse_float` answer `None` for text that
is not a number, and are worth stating exactly because untrusted input reaches
them:

- `parse_int` accepts surrounding whitespace and a leading `+` or `-`. It does
  **not** accept the `1_000` and `0xFF` forms that [L-INT] allows in *source*:
  a literal in a program and a number in a data file are different things.
- `parse_float` accepts `inf`, `-inf` and `nan`, and a value too large to
  represent gives an infinity rather than `None`. So `"1e400"` parses, and the
  `inf` it produces then flows into arithmetic, `as_i64` ([L-LIB-NUM]) and
  sorting ([L-LIB-SORT-NAN]) — which is how those three came to be specified.

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
| `xs.remove(i)` | `(i64) -> T` | mutates; panics out of range, like indexing ([L-LIB-REMOVE]) |
| `a.cmp(b)` | `(T) -> i64` | `-1`, `0` or `1` in the order `sort` uses; `i64`, `f64`, `str` and tuples ([L-LIB-CMP]) |
| `xs.sort()` | | mutates; ascending. Only `i64`, `f64` and `str` are ordered — sorting anything else is a compile error, the same one `<` gives |
| `xs.rev()` | `-> [T]` | |
| `xs.slice(a, b)` | `(i64, i64) -> [T]` | `a` up to but excluding `b`; clamps past either end rather than panicking, like `substr` |
| `xs.contains(v)` | `(T) -> bool` | |
| `xs.join(sep)` | `(str) -> str` | `[str]` only — map to `str` first |
| `xs.sum()` | `-> T` | numeric elements; `0` of the element type when empty ([L-LIB-SUM]) |
| `xs.min()`, `xs.max()` | `-> Option[T]` | `None` when empty; not available for `bool` |
| `xs.fold(init, f)` | `(B, (B, T) -> B) -> B` | left fold; the accumulator's type comes from `init`, not from `T` |
| `xs.map(f)` | `(T -> U) -> [U]` | |
| `xs.par_map(f)` | `(T -> U) -> [U]` | multi-core; `f` must be effect-free ([T-PAR-MAP-PURE]) |
| `xs.filter(f)` | `(T -> bool) -> [T]` | |
| `xs.par_filter(f)` | `(T -> bool) -> [T]` | multi-core; `f` must be effect-free; input order kept ([E-PAR-FILTER-ORDER]) |
| `xs.any(f)`, `xs.all(f)` | `(T -> bool) -> bool` | short-circuit ([E-SEARCH-SHORTCIRCUIT]) |
| `xs.find(f)` | `(T -> bool) -> Option[T]` | first match |
| `xs + ys` | `([T], [T]) -> [T]` | concatenation, the way `+` joins strings ([E-LIST-CONCAT]) |
| `xs.sort_by(key)` | `(T -> K)` | mutates; sorts by an extracted `i64`/`f64`/`str` key, stably ([L-LIB-SORT-BY]) |
| `xs.enumerate()` | `-> [(i64, T)]` | pair each element with its index |
| `xs.zip(ys)` | `([U]) -> [(T, U)]` | pairs up to the shorter of the two |
| `pair.0`, `pair.1`, … | `-> T` | a tuple's components are named by position ([L-LIB-TUPLE]) |

**[L-LIB-SORT-BY]** `xs.sort_by(key)` sorts by what `key` returns for each
element, which is where a list of structs gets its order from — `sort` needs
the *elements* ordered, and a struct is not. The key must be `i64`, `f64` or
`str`. The sort is stable: elements with equal keys stay in the order they
were written. It sorts **ascending**, always; descending is
`xs.sort_by(k)` followed by `xs.rev()`. Negating the key works for a number
and has no counterpart for a `str`, so the reversal is the general answer and
the one to reach for.

**[L-LIB-PAIRS]** `xs.enumerate()` pairs each element with its index and
`xs.zip(ys)` pairs the two lists positionally, stopping at the shorter — there
is no element to invent past the end of one. Both give `[(A, B)]`, which
`for (a, b) in …` binds by name ([G-LET-TUPLE]).

**[L-LIB-TUPLE]** A tuple's components are read by position — `pair.0`,
`pair.1` — and a tuple of ordered components is itself ordered, compared
component by component. That is what `<` on tuples means, what `sort` on a
list of them does, `min`/`max` included, and what makes
`xs.sort_by(|p| (p.dept, p.age, p.name))` a multi-key sort rather than an
error. `min` and `max` used to spell out the types they accepted instead of
asking whether the element was ordered, so `xs.enumerate().min()` — the obvious
"index of the smallest" — was refused on a list `sort` orders happily.

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
| `m.entries()` | `-> [(K, V)]` | pairs, in key order — the only way to reach both at once |

## 6. `Option[T]`

**[L-LIB-OPTION]**

| form | notes |
|------|-------|
| `Some(v)`, `None` | constructors |
| `o ?? fallback` | the value, or `fallback`; the right side is evaluated only when needed ([E-COALESCE-LAZY]) |
| `o.is_none()`, `o.is_some()` | |
| `o.unwrap_or(d)` | same as `??` |
| `o.expect(msg)` | the payload, or panics with `msg` — for when absence is a bug, not a case |
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

A `let` at module level is a **constant** — one value for the whole program:

```
let TABLE_WIDTH = 77
let TITLE = "Sales report"

fn main() {
    print("{TITLE}, {TABLE_WIDTH} wide")
}
```

A constant's value cannot perform effects. It is evaluated wherever its name
is used, so an effect in it would happen once per mention; a value that has to
be computed once, with effects, is a function.

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

A tuple can be split into names, in a `let` or in a loop header:

```
fn main() {
    let (a, b) = (1, "two")
    print("{a} {b}")

    for (i, w) in ["x", "y"].enumerate() {
        print("{i}{w}")
    }
}
```

Those are the only destructuring forms — there is no `if let`, and patterns
elsewhere appear only in `match`. They matter most with maps, where
`entries()` hands back pairs, and with `enumerate()` and `zip()`.

Write a map with `{key: value}`, or build one up from `{}`. Maps iterate in
sorted key order, so printing one is deterministic — and what it prints is
what you would type to write it back:

```
fn main() {
    let ripeness = {"pear": 2, "fig": 1}
    print("{ripeness}")          // {fig: 1, pear: 2} — sorted, every run

    var counts: Map[str, i64] = {}
    counts.set("pear", 2)
    print("{counts}")
}
```

`{}` is the empty map. Braces holding *statements* are a block, and an empty
block has no use as an expression, so `()` is how you write unit. The braces
of `if`, `for`, `while`, `match` and function bodies are always blocks.

Lists join with `+`, and sort by a key you extract — which is how a list of
structs gets sorted at all, since a struct has no order of its own:

```
type Run = { name: str, ms: i64 }

fn main() {
    var runs = [Run{ name: "b", ms: 30 }, Run{ name: "a", ms: 10 }]
    runs.sort_by(|r| r.ms)
    for r in runs {
        print("{r.name} {r.ms}")
    }

    let all = [1, 2] + [3]
    print("{all}")
}
```

Text is characters, not bytes, and `code_at` / `codes` / `to_char` move
between a character and its Unicode code point — which is what lets you
compute on text rather than only cut and compare it:

```
fn hash(s: str) -> i64 {
    var h = 0
    for c in s.codes() {
        h = (h * 31 + c) % 1000007
    }
    h
}

fn main() {
    print("{hash("grasm")} {104.to_char()}")
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

A closure captures the **values** it uses at the moment it is created, not the
bindings ([E-CLOSURE-CAPTURE]). Changing a captured variable afterwards is not
visible inside, and assigning to one inside changes only the closure's copy:

```
fn main() {
    var n = 1
    let plus_n = |x| x + n
    n = 100
    print("{plus_n(0)}")     // 1, not 100
}
```

That has one consequence worth knowing before you design around it: **an
effect handler cannot hold mutable state**, because a handler's operations are
closures. A `Store` whose `write` updates a captured list updates a copy, and
the next `read` will not see it. Handlers supply values and accept output;
state that changes belongs in the caller. `examples/ledger/` is that lesson
written down after getting it wrong.

Note that `spawn { … }` does *not* work this way — a task body shares the
enclosing scope ([E-SPAWN-SHARED]). Two block-shaped constructs, two rules.

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

The two forms divide the work between them, and it is worth knowing which you
want before you reach for one. **`for x in xs { spawn { … x … } }` is not
writable**: a task's body reads the enclosing scope as it stands when the
tasks run, so a loop variable is gone by then, and the compiler says so. The
number of tasks a `parallel` block starts is fixed where it is written.

So: a *runtime-sized* amount of work over data is `par_map`/`par_filter`,
which is also the form that gets real cores and that the compiler proves safe.
`parallel`/`spawn` is for a fixed set of distinct activities that share results
through the enclosing scope.

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

A capability is what a package needs from **outside** itself, so what counts is
what *escapes*: the effects `main` performs, and the effects left in a `pub`
function's row for a consumer to handle. An effect you declare, perform and
handle inside your own package needs nothing from anyone and is not a
capability — but the handler answers for what it really does, so a `Log`
handler that calls `print` puts `io` in the ceiling whoever asked for it.

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

`grasm test file.gr` runs the tests of every module the program imports, not
just that file's, and names each one by its module. `grasm bench` does the
same for benchmarks. A library whose tests are
failing should not look like a library whose tests passed.

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
reports time per operation, in whatever unit keeps the figure readable.

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

The fenced block is **executable**. `grasm test` runs it as a test, compiled
in the module that owns it: that module's own items are in scope unqualified,
and everything else it imports is available the way the module has it. So an
example writes `square(4)` — the way the module's own code does — and the same
example passes whether the library is tested on its own or through a consumer.
Documentation that stops being true fails the suite instead of quietly
misleading the next reader.

Lines beginning `# ` inside a block are hidden setup: they run, but they are
not part of what a reader sees.

## 10a. Calling C

`extern fn` declares a C function. There is no body and no binding layer: a
compiled build resolves the symbol at link time, and `grasm run` finds it in
its own process.

```
extern fn sqrt(x: f64) -> f64
extern fn getpid() -> i64

fn main() {
    let r = unsafe {
        sqrt(16.0)
    }
    print("{r}")
}
```

Parameters and results are `i64` and `f64`. `bool` is deliberately not
allowed: C's is one byte and Grasm's is a register, and that kind of mismatch
surfaces as garbage far from where it was caused.

**It has to be inside `unsafe`, and it counts as both `unsafe` and `io`.**
Calling C is Tier 3 — outside everything the safe tiers guarantee — so the
source says so at the use, and the package says so in its manifest:

```
capabilities = ["io", "unsafe"]
```

Naming a C function without calling it needs `unsafe` as well: holding the
function *is* the ability to call it. What a C function does cannot be known,
so the safe assumption is the only one available — an `extern` use counts
against the ceiling, and a function that makes one cannot be handed to
`par_map`. A consumer can see `unsafe` in your manifest, transitively, and
refuse to depend on you: your JSON parser cannot quietly contain one.

## 10b. Knowing what you broke

`grasm api` prints a package's public surface — every `pub` item, sorted, one
line each. Save it, and a later `--against` says what changed:

```text
$ grasm api src/main.gr > v1.api
$ grasm api src/main.gr --against v1.api
CHANGED  lib::Point
    was  type Point = { x: i64, y: i64 }
    now  type Point = { x: f64, y: i64 }

breaking: 0 removed, 1 changed, 0 added — this is a MAJOR release
```

It exits 1 on a breaking change, so a build can refuse one that the version
number does not admit to.

A version number is supposed to answer "will this break me". One a human typed
cannot answer it — it records what somebody *believed*. This one is read off
the code.

The limit is worth stating: this compares **shape**, not behaviour. A function
that keeps its signature and starts returning something else is a `PATCH` here
and a disaster for you. Deciding that automatically is undecidable, and a tool
that claimed to would be worse than one that says where it stops.

## 11. What `--native` will not compile yet

`grasm run` and `grasm build --native` are two implementations of one
language, and `scripts/difftest.sh` holds them to identical output on every
program in the repository. The compiled backend is nonetheless a *smaller*
language than this document describes. It refuses what it cannot do, by name
and at build time — it never miscompiles it — but a program using one of these
runs only under `grasm run`:

| shape | example |
|-------|---------|
| a generic **type** declaration | `type Pair[A, B] = { fst: A, snd: B }` |
| an effect handler as a return type | `fn mk() -> ClockHandler { … }` |
| a range held as a value | `let r = 1..5` (`for i in 0..5` is fine) |
| `par_map`/`par_filter` with a closure bound to a name | `let f = \|n\| n * 2` then `xs.par_map(f)` — write it inline, or as a top-level `fn` |
| `map` with a function that returns nothing | `xs.map(\|x\| print(x))` — a `for` loop is what that meant |
| a type parameter appearing **only** in a function-typed parameter's result | `fn key_by[T, K](xs: [T], f: fn(T) -> K) -> [K]` |
| an empty list literal with no type in view | `var xs = []` — write `var xs: [i64] = []` |
| a `match` no arm of which has an inferable type | every arm `map()` — give one arm a type, or lift it into an annotated `let` |
| an `inout` argument that is not a plain variable | `push_one(b.items)`, `add(xs[0], 5)` — bind it to a `var` first |
| returning a closure | `fn mk(n: i64) -> fn(i64) -> i64 { \|x\| x + n }` — a top-level `fn` works |
| a handler that is a loop variable or a list element | `for h in hs { with Src handled_by h { … } }` — a `let`-bound handler compiles |
| writing *through* a `ref` | `r.field = v`, `r.items.push(x)` — reading through one compiles |
| a handler operation that is a named `fn` | `SrcHandler{ get: named }` — write the closure inline |
| `==` on two bare `ref`s | `a == b` — comparing structs or lists *containing* refs compiles |

Each of these is a defect being worked off, not a design boundary. The list is
checked against the implementation rather than remembered: if one of them
starts compiling, this table is what has to change.

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

## `E-BORROWED-READ.gr`

```grasm
//! tag: E-BORROWED-READ
//! expect: output
//! Hello World|15|[Hello, World]|15
//! [2, 1, 3] 3 false
//! [a, b] [1, 2] [(a, 1), (b, 2)] 2
//! 15 3 2
//! 2000 40000 1 20
// A read-only method on a plain variable does not touch the value's
// refcount: the variable already holds a reference for the whole call, so
// the usual +1/-1 around it is traffic and nothing else.
//
// This program exists to prove the value survives it. Every receiver is read
// several times and then used again, and the last block builds and keeps two
// thousand lists while reading them, which is where an early free would show.
fn main() {
    let s = "  Hello World  "
    let xs = [3, 1, 2]
    var m: Map[str, i64] = {"a": 1, "b": 2}
    print("{s.trim()}|{s.to_upper().len()}|{s.words()}|{s.chars().len()}")
    print("{xs.rev()} {xs.len()} {xs.is_empty()}")
    print("{m.keys()} {m.values()} {m.entries()} {m.len()}")
    // Still usable after every one of those.
    print("{s.len()} {xs.len()} {m.len()}")

    var kept: [[i64]] = []
    var i = 0
    while i < 2000 {
        var ys: [i64] = []
        var j = 0
        while j < 20 {
            ys.push(j)
            j = j + 1
        }
        if ys.len() > 0 and not ys.is_empty() {
            kept.push(ys)
        }
        i = i + 1
    }
    var total = 0
    for k in kept {
        total = total + k.len()
    }
    var held: Map[str, [i64]] = {}
    held.set("a", kept.get(0) ?? [])
    print("{kept.len()} {total} {held.len()} {(held.get("a") ?? []).len()}")
}
```

## `E-CLOSURE-CAPTURE.gr`

```grasm
//! tag: E-CLOSURE-CAPTURE
//! expect: output
//! 1
//! 2
//! 7
// A closure captures values, not bindings. Changing the variable afterwards
// is not visible inside, and writing to it inside changes only a copy.
//
// This is worth a program because the same author who wrote the rule got it
// wrong writing an effect handler: a `write` operation that updates a
// captured list updates a copy, and the next `read` does not see it. The
// counter below is that mistake in miniature.
fn main() {
    var n = 1
    let plus_n = |x| x + n
    n = 100
    // Still 1: the closure kept the value it was built with.
    print("{plus_n(0)}")

    var xs = [1, 2]
    let size = |_ignored| xs.len()
    xs.push(3)
    print("{size(0)}")

    // A captured binding may be read freely.
    let base = 7
    let from_base = |x| base + x
    print("{from_base(0)}")

    // Writing to a captured variable is a compile error, because the write
    // would change a copy nobody can observe. See T-CAPTURE-WRITE.
}
```

## `E-COALESCE-DIVERGE.gr`

```grasm
//! tag: E-COALESCE-DIVERGE
//! expect: output
//! 1 0
//! 10 -1
//! 7
// `?? fail(e)` — absence turned into a failure, which is the shape most
// fallible function bodies take.
type E = { m: str }

fn need(m: Map[str, i64], k: str) -> i64 ! E {
    m.get(k) ?? fail(E{ m: "no {k}" })
}

fn doubled(o: Option[i64]) -> i64 ! E {
    let v = o ?? fail(E{ m: "none" })
    v * 2
}

fn main() {
    let m = {"a": 1}
    print("{need(m, "a") ?? 0} {need(m, "b") ?? 0}")
    print("{doubled(Some(5)) ?? 0 - 1} {doubled(None) ?? 0 - 1}")
    // A fallback that does produce a value still does.
    print("{m.get("zz") ?? 7}")
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

## `E-CONST-REF.gr`

```grasm
//! tag: E-CONST-REF
//! expect: error
//! a constant's value cannot take a reference
// A constant cannot hold a reference.
//
// [E-CONST] says a constant is evaluated wherever its name is used, so every
// mention allocates a *different* slot. `print("{ROOT} {ROOT}")` printed two
// different references on one line; `release(ROOT)` gave up a slot nothing
// else would ever name and `alive(ROOT)` was then still true; a loop reading
// the constant allocated a slot per iteration. All three engines agreed, so it
// was language semantics rather than a bug — and it silently destroyed the one
// thing a reference is for. It follows from [E-CONST] and nothing said so.
type Node = { id: i64 }

let ROOT = ref(Node{ id: 1 })

fn main() {
    print("{ROOT.id}")
}
```

## `E-CONST.gr`

```grasm
//! tag: E-CONST
//! expect: output
//! Sales report is 77 wide, tab 9
//! #####
//! [10, 20, 30] 3
//! 77 154
//! 4 6
// A module-level `let` is a constant. Without them every layout constant had
// to be a zero-argument function, which is what a program ends up full of.
let TABLE_WIDTH = 77
let TAB = 9
let TITLE: str = "Sales report"
let LIMITS = [10, 20, 30]
/// A constant may be computed from another declared before it.
let DOUBLE_WIDTH = TABLE_WIDTH * 2
/// And from a function, or naming one as a value — a dispatch table is the
/// shape this is for.
let TWICE_TWO = two() * 2
let AGGS: Map[str, fn([i64]) -> i64] = {"sum": total, "count": howmany}

fn two() -> i64 {
    2
}

fn total(xs: [i64]) -> i64 {
    xs.sum()
}

fn howmany(xs: [i64]) -> i64 {
    xs.len()
}

fn bar(n: i64) -> str {
    "#".repeat(n.min(TABLE_WIDTH))
}

fn main() {
    print("{TITLE} is {TABLE_WIDTH} wide, tab {TAB}")
    print("{bar(5)}")
    print("{LIMITS} {LIMITS.len()}")
    print("{TABLE_WIDTH} {DOUBLE_WIDTH}")
    let f = AGGS.get("sum") ?? howmany
    print("{TWICE_TWO} {f([1, 2, 3])}")
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

## `E-EFFECT-NO-INOUT.gr`

```grasm
//! tag: E-EFFECT-NO-INOUT
//! expect: error
//! cannot take an `inout` parameter
// An effect operation cannot take `inout`.
//
// A handler runs through a function value, so the write landed in a copy the
// caller never saw: the handler reported success, the caller read back an
// empty list, and every engine agreed on the wrong answer — which is the one
// shape no differential test can catch. It typechecked, so the only way to
// learn this was to run it and disbelieve the result.
effect Audit {
    fn record(inout sink: [str], entry: str)
}

fn main() {
    print("unreachable")
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

## `E-HANDLER-OWN-ROW.gr`

```grasm
//! tag: E-HANDLER-OWN-ROW
//! expect: output
//! 42
// A handler's own effects are handled where the handler is installed.
//
// [E-HANDLER-SCOPE] describes a dynamic extent, and this is narrower: the `B`
// handler's body performs `A`, so `A` must be handled around the `with B`
// block — not merely around the calls to `B.b()`. Writing it the other way
// round is refused, and the message names the enclosing function, which is the
// one place the fix does not go.
effect A {
    fn a() -> i64
}

effect B {
    fn b() -> i64
}

fn main() {
    // `A` outside, `B` inside: the B handler's body may then perform A.
    with A handled_by AHandler{ a: || 21 } {
        with B handled_by BHandler{ b: || A.a() * 2 } {
            print("{B.b()}")
        }
    }
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

## `E-IF-DIVERGE.gr`

```grasm
//! tag: E-IF-DIVERGE
//! expect: output
//! 25 ok2 0.25 true
//! N 7
//! 15 -1
//! 25
//! 0 -> err z
//! 1 -> 100
//! 2 -> 50
// A branch that leaves — `fail`, `panic`, `return` — has no value, so the
// `if` is worth whatever the *other* branch produced.
//
// Taking the type from the then branch alone meant `if c { fail(e) } else
// { v }` in tail position had no merge value at all: the compiled function
// returned a zero, with exit 0 and nothing reported. For a boxed payload it
// was a SIGSEGV later, in whichever code read the value back.
type E = { c: str }
type V = | N(n: i64) | S(s: str)

fn fi(y: i64) -> i64 ! E {
    if y == 0 { fail(E{ c: "z" }) } else { 100 / y }
}

fn fs(y: i64) -> str ! E {
    if y == 0 { fail(E{ c: "z" }) } else { "ok{y}" }
}

fn ff(y: i64) -> f64 ! E {
    if y == 0 { fail(E{ c: "z" }) } else { 1.0 / y.as_f64() }
}

fn fb(y: i64) -> bool ! E {
    if y == 0 { fail(E{ c: "z" }) } else { y > 1 }
}

/// A boxed payload: this was the SIGSEGV.
fn fv(y: i64) -> V ! E {
    if y == 0 { fail(E{ c: "z" }) } else { V.N(y) }
}

/// An `else if` chain of failures ending at the one arm with a value.
fn chain(y: i64) -> i64 ! E {
    if y == 0 {
        fail(E{ c: "a" })
    } else if y == 1 {
        fail(E{ c: "b" })
    } else {
        y * 3
    }
}

fn main() {
    print("{fi(4) ?? 0 - 1} {fs(2) ?? "?"} {ff(4) ?? 0.0} {fb(2) ?? false}")
    match fv(7) {
        Ok(V.N(n)) -> print("N {n}")
        Ok(V.S(s)) -> print("S {s}")
        Err(e) -> print("e {e.c}")
    }
    print("{chain(5) ?? 0 - 1} {chain(1) ?? 0 - 1}")
    // Stored and read back, which is where the boxed case crashed.
    var m: Map[str, i64 ! E] = {}
    m.set("a", fi(4))
    print("{m.get("a") ?? fi(1) ?? 0 - 1}")
    // And the whole shape end to end.
    for n in [0, 1, 2] {
        match fi(n) {
            Ok(v) -> print("{n} -> {v}")
            Err(e) -> print("{n} -> err {e.c}")
        }
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

## `E-INOUT-FAIL.gr`

```grasm
//! tag: E-INOUT-FAIL
//! expect: output
//! 1 -1 [1, 2]
//! -1 [1, 2]
// A function that *fails* still returns, and the writes it made before
// failing happened.
//
// The interpreter dropped `inout` writebacks on the error path, so a `push`
// before a `fail` was silently undone — and the compiled binary kept it. The
// two engines disagreed about what the program had *done*, not just what it
// answered.
type E = { m: str }
type Box = { xs: [i64] }

fn push_then_fail(inout xs: [i64], v: i64) -> i64 ! E {
    xs.push(v)
    if v > 1 {
        fail(E{ m: "too big" })
    }
    v
}

fn add(inout b: Box, v: i64) -> i64 ! E {
    b.xs.push(v)
    if v > 1 {
        fail(E{ m: "no" })
    }
    v
}

/// Through a `try` chain, where the failure crosses two frames.
fn outer(inout b: Box) -> i64 ! E {
    let a = try add(b, 1)
    let c = try add(b, 2)
    a + c
}

fn main() {
    var xs: [i64] = []
    let a = push_then_fail(xs, 1) ?? 0 - 1
    let b = push_then_fail(xs, 2) ?? 0 - 1
    print("{a} {b} {xs}")

    var box = Box{ xs: [] }
    print("{outer(box) ?? 0 - 1} {box.xs}")
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

## `E-LIST-CONCAT.gr`

```grasm
//! tag: E-LIST-CONCAT
//! expect: output
//! [1, 2, 3]
//! [a, b, c]
//! [1.5, 2.5] [true, false]
//! [1] [1] []
//! [1, 2] [3] 3 kept
//! [[1], [2], [3]]
//! [0, 1, 4]
fn main() {
    print("{[1, 2] + [3]}")
    print("{["a"] + ["b", "c"]}")
    print("{[1.5] + [2.5]} {[true] + [false]}")
    // An empty operand on either side, and both.
    let e: [i64] = []
    print("{e + [1]} {[1] + e} {e + e}")
    // Neither operand is changed.
    let a = [1, 2]
    let b = [3]
    let joined = a + b
    print("{a} {b} {joined.len()} kept")
    // Lists of lists concatenate as lists, not element-wise.
    print("{[[1], [2]] + [[3]]}")
    // Building up in a loop.
    var acc: [i64] = []
    for i in 0..3 {
        acc = acc + [i * i]
    }
    print("{acc}")
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

## `E-LOOP-RANGE-MAX.gr`

```grasm
//! tag: E-LOOP-RANGE-MAX
//! expect: output
//! 1 1 3
//! 10 0 0
// A range may reach the ends of `i64`.
//
// The interpreter built `a..=b` as `a..(b + 1)`, which wraps at the top of the
// range: `mx..=mx` iterated over nothing. The compiled backend incremented
// and then re-tested `i <= end`, so `i` wrapped and the loop never ended. One
// program, an empty loop and an endless one, from an overflow neither engine
// reported.
fn main() {
    let mx = 9223372036854775807
    let mn = 0 - mx - 1
    var a = 0
    for i in mx..=mx {
        a = a + 1
    }
    var b = 0
    for i in mn..=mn {
        b = b + 1
    }
    var c = 0
    for i in (mx - 2)..=mx {
        c = c + 1
    }
    print("{a} {b} {c}")
    // The ordinary cases, unchanged: inclusive counts the end, exclusive does
    // not, and a backwards range runs zero times rather than forever.
    var d = 0
    for i in 0..=4 {
        d = d + i
    }
    var e2 = 0
    for i in 3..3 {
        e2 = e2 + 1
    }
    var f = 0
    for i in 3..=2 {
        f = f + 1
    }
    print("{d} {e2} {f}")
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

## `E-MAP-LIT.gr`

```grasm
//! tag: E-MAP-LIT
//! expect: output
//! {a: 1, b: 2}
//! {a: 9}
//! {} 0
//! {k: 1} 3 ()
//! 2 entries
//! [k1, v1, k2, v2]
fn main() {
    // Written order does not matter: iteration is sorted (E-MAP-ORDER).
    let ordered = {"b": 2, "a": 1}
    print("{ordered}")
    // A repeated key keeps the last value written.
    let dup = {"a": 1, "a": 9}
    print("{dup}")
    // `{}` is the empty map where a map is expected...
    var empty: Map[str, i64] = {}
    print("{empty} {empty.len()}")
    // ...including as a struct field, which supplies the type.
    var b = Holder{ items: {} }
    b.items.set("k", 1)
    // Braces holding statements are still a block; `{}` is the empty map,
    // and the empty block — being useless as an expression — is spelled `()`.
    let block = { let n = 2
        n + 1 }
    print("{b.items} {block} {()}")
    // Each key is evaluated before its own value, and entries left to right.
    var log: [str] = []
    let m = {key(log, "k1"): val(log, 1), key(log, "k2"): val(log, 2)}
    print("{m.len()} entries")
    print("{log}")
}

/// Records that it ran, then answers with the key it was given.
fn key(inout log: [str], k: str) -> str {
    log.push(k)
    k
}

/// The same, for a value.
fn val(inout log: [str], v: i64) -> i64 {
    log.push("v{v}")
    v
}

type Holder = { items: Map[str, i64] }
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

## `E-MATCH-BUILTIN-ARMS.gr`

```grasm
//! tag: E-MATCH-BUILTIN-ARMS
//! expect: output
//! 2.718281828459045
//! 0 false
// A match whose every arm is a builtin call still yields a value.
//
// The compiled backend types a match by probing its arms, and no builtin free
// function was in the probe's table. When *no* arm could be typed the result
// was not a refusal: "nothing could be typed" and "every arm yields nothing"
// were the same answer, so a value match compiled as a statement match and the
// enclosing function returned whatever was there. `args()` in both arms of a
// fallible function returned `0`. The fuzzer found it the day boolean
// expressions started being generated at all.
fn constant(p: i64) -> f64 ! str {
    match p {
        0 -> e()
        _ -> pi()
    }
}

fn from_env(p: i64) -> [str] ! str {
    match p {
        0 -> args()
        _ -> args()
    }
}

fn exists(p: i64) -> bool ! str {
    match p {
        0 -> file_exists("/definitely/not/here")
        _ -> file_exists("/nor/here")
    }
}

fn main() {
    match constant(0) {
        Ok(v) -> print("{v}")
        Err(x) -> print("{x}")
    }
    let n = match from_env(0) {
        Ok(v) -> v.len()
        Err(_) -> -1
    }
    let b = match exists(0) {
        Ok(v) -> v
        Err(_) -> true
    }
    print("{n} {b}")
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
//! 5.5 5.5
//! true true
//! true true true
//! 2.5 7.5
//! 3.5
// An integer *literal* meeting an `f64` becomes one — on either side, and for
// arithmetic and comparison alike. The rule used to be written as "mixing
// promotes", which is not what happens, and comparisons had an asymmetry
// nothing recorded: `5.0 == 5` was accepted and `5 == 5.0` was not.
fn main() {
    let f = 2.5
    print("{3 + f} {f + 3}")
    // Equality, both ways round.
    print("{5.0 == 5} {5 == 5.0}")
    // Ordering, both ways round.
    print("{5.5 < 6} {6 > 5.5} {6 >= 5.5}")
    // A literal in any operand position.
    print("{f * 1} {f * 3}")
    // A non-literal `i64` is converted where the program says so, and only
    // there ([T-INT-TO-FLOAT]).
    let n = 1
    print("{n.as_f64() + f}")
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

## `E-PANIC-TRACE.gr`

```grasm
//! tag: E-PANIC-TRACE
//! expect: panic
//! stack overflow: recursion too deep
// The panic line is the same in every engine; only the trace after it differs.
//
// The two engines stop at different depths on purpose — one counts calls, the
// other measures remaining stack — and both bounds are implementation-defined
// ([E-RECURSION-DEPTH]). The interpreter used to name its own number here, so
// the same failure read as two different failures depending on how you ran it.
fn down(n: i64) -> i64 {
    down(n + 1)
}

fn main() {
    print("{down(0)}")
}
```

## `E-PAR-FAIL.gr`

```grasm
//! tag: E-PAR-FAIL
//! expect: output
//! []
//! []
//! [1, 3]
// A failure inside a parallel closure is the whole call's failure — not a
// value in the result. The native backend runs such a closure sequentially,
// because its workers have nowhere to put an error.
type E = { m: str }

fn doubled(xs: [i64]) -> [i64] ! E {
    xs.par_map(|x| if x == 2 { fail(E{ m: "two" }) } else { x })
}

fn kept(xs: [i64]) -> [i64] ! E {
    xs.par_filter(|x| if x == 2 { fail(E{ m: "two" }) } else { true })
}

fn main() {
    print("{doubled([1, 2, 3]) ?? []}")
    print("{kept([1, 2, 3]) ?? []}")
    // Without a failing element the parallel path runs as usual.
    print("{kept([1, 3]) ?? []}")
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

## `E-POW.gr`

```grasm
//! tag: E-POW
//! expect: output
//! 1024 1024.0
//! 1.5 0.5 0.25
// `^` is exponentiation, not xor.
//
// Coming from the C family, `^` reads as bitwise xor, and `2 ^ 10` being 1024
// rather than 8 is the kind of difference that produces a wrong answer instead
// of an error. It was in the grammar as an operator and nowhere as a meaning.
fn main() {
    print("{2 ^ 10} {2.0 ^ 10.0}")
    // A float exponent may be fractional or negative; an integer one may not
    // be negative, which is [E-POW-NEG].
    print("{2.25 ^ 0.5} {2.0 ^ -1.0} {2.0 ^ -2.0}")
}
```

## `E-PRINT-ATOMIC.gr`

```grasm
//! tag: E-PRINT-ATOMIC
//! expect: output
//!   side 3
//!   side 4
//! x=6 y=8
// `print` evaluates every interpolation before it writes anything.
//
// Emitting each piece as it was computed saved an allocation and cost two
// guarantees: a panic partway along a line left the earlier pieces on stdout,
// against [E-PANIC-EXIT], and a nested `print` landed in the middle of the
// outer line instead of before it. This program pins the second; the first is
// in scripts/agreetest.sh, which can see stderr and the exit code.
fn side(n: i64) -> i64 {
    print("  side {n}")
    n * 2
}

fn main() {
    print("x={side(3)} y={side(4)}")
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

## `E-RECURSION-PARALLEL.gr`

```grasm
//! tag: E-RECURSION-PARALLEL
//! expect: output
//! 4000000
// Deep recursion inside a parallel closure is bounded the same way.
//
// A `par_map` worker runs on its own stack, and it took the platform default —
// small enough that the guard computed its floor below the end of it and could
// never fire. Recursion this deep in a worker then ran off the stack for real:
// SIGBUS, exit 138, nothing on stderr, in the one place [E-RECURSION-DEPTH]
// promises a panic and exit 101. Inputs small enough to run sequentially were
// unaffected, which is why it took a few hundred elements to see.
fn descend(n: i64) -> i64 {
    if n <= 0 {
        0
    } else {
        descend(n - 1) + 1
    }
}

fn main() {
    var xs: [i64] = []
    for i in 0..500 {
        xs.push(8000)
    }
    print("{xs.par_map(|x| descend(x)).sum()}")
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

## `G-LET-TUPLE.gr`

```grasm
//! tag: G-LET-TUPLE
//! expect: output
//! 1 two
//! fig=1
//! pear=2
//! 3
//! 3
//! 0a
//! 1b
//! 1x
//! 2y
//! 30
// Binding a tuple's components by name.
//
// `m.entries()` hands back `[(K, V)]` and splitting it used to cost a whole
// `match` arm — a type system charging rent rather than paying for itself.
fn main() {
    let (a, b) = (1, "two")
    print("{a} {b}")

    var m = map()
    m.set("pear", 2)
    m.set("fig", 1)
    var total = 0
    // `for (k, v) in …` is exactly the loop below with the split written
    // into the header instead of the first line of the body.
    for (k, v) in m.entries() {
        print("{k}={v}")
        total = total + v
    }
    print("{total}")

    // The long form still means the same thing.
    var again = 0
    for e in m.entries() {
        let (k, v) = e
        again = again + v + k.len() - k.len()
    }
    print("{again}")

    // It works over any list of tuples, not just `entries()`.
    for (i, w) in ["a", "b"].enumerate() {
        print("{i}{w}")
    }
    for (n, w) in [1, 2].zip(["x", "y"]) {
        print("{n}{w}")
    }

    // `var` binds them mutably.
    var (p, q) = (10, 20)
    p = p + q
    print("{p}")
}
```

## `G-MAP-BLOCK-AMBIG.gr`

```grasm
//! tag: G-MAP-BLOCK-AMBIG
//! expect: output
//! map {a: 1}
//! block 3
//! empty {}
//! body ok
//! cond ok
//! arm [2] {a: 1}
type Cfg = { on: bool }

fn main() {
    // An expression then `:` — a map.
    let m = {"a": 1}
    print("map {m}")
    // Statements — a block, whose value is its last expression.
    let b = { let n = 2
        n + 1 }
    print("block {b}")
    // Nothing at all — the empty map.
    let e: Map[str, i64] = {}
    print("empty {e}")
    // The braces of `if`/`for`/`while`/`match` and a function body are not in
    // expression position, so they stay blocks even when the first thing in
    // them could start a map entry.
    if true {
        let a = 1
        print("body {label(a)}")
    }
    // A condition's brace opens the body (G-STRUCT-AMBIG), and the body is a
    // block, not a map, even though `on: true` would read as an entry.
    let c = Cfg{ on: true }
    if c.on {
        print("cond ok")
    }
    // In a match arm, `{}` is the empty block — an arm that does nothing.
    var kept: [i64] = []
    for n in [1, 2] {
        match n {
            1 -> {}
            _ -> kept.push(n)
        }
    }
    // A non-empty map literal is still a map there.
    let picked = match 1 {
        1 -> {"a": 1}
        _ -> {"b": 2}
    }
    print("arm {kept} {picked}")
}

fn label(n: i64) -> str {
    if n == 1 { "ok" } else { "no" }
}
```

## `G-MATCH-ARM-EXPR.gr`

```grasm
//! tag: G-MATCH-ARM-EXPR
//! expect: error
//! expected an expression but found `for`
// A match arm's body is an expression. The grammar read
// `( expression | statement )`, which the parser had never accepted — so a
// reader who trusted it got this error with no way to tell which of the two
// was wrong. Write `1 -> { for i in 0..2 { ... } }` when you want a statement.
fn main() {
    let n = 1
    match n {
        1 -> for i in 0..2 { print("{i}") }
        _ -> print("no")
    }
}
```

## `G-NEST-LIMIT.gr`

```grasm
//! tag: G-NEST-LIMIT
//! expect: output
//! ok 1
// Depth is bounded, and the bound is on the tree rather than on brackets.
//
// None of these three is deep: the point is that the second and third build a
// spine as long as they are while opening no brackets at all, which is how a
// few thousand terms used to abort the toolchain with a stack overflow and
// nothing on stderr. They are counted the same way now.
fn main() {
    let nested = ((((((((((1))))))))))
    let chain = 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1
    let postfix = (1).max(1).max(1).max(1).min(1)
    if nested == 1 and chain == 8 {
        print("ok {postfix}")
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

## `G-STRUCT-NONEMPTY.gr`

```grasm
//! tag: G-STRUCT-NONEMPTY
//! expect: error
//! struct `Empty` has no fields
// A struct has at least one field.
//
// `{}` after a name is a block — that is what makes `handled_by h { … }`
// unambiguous — so a zero-field struct is a type no value of which can be
// written. The grammar made both the field list and the literal's contents
// optional, so this was declared and accepted, and `Empty{}` then failed with
// "expected end of line after statement": a parse error about the wrong thing,
// for a declaration the grammar had offered. Use a sum variant for a value
// that carries nothing.
type Empty = { }

fn main() {
    print("unreachable")
}
```

## `G-STRUCT-UPDATE.gr`

```grasm
//! tag: G-STRUCT-UPDATE
//! expect: output
//! run n=3 total=8.0 min=1.0 max=4.0
//! 3 99 run
//! 3
//! 1 2
// Taking the fields you are not changing from a value you already have.
type Stats = { count: i64, total: f64, min: f64, max: f64, label: str }

/// The shape this exists for: an accumulator that otherwise restates every
/// field at every step.
fn add(s: Stats, v: f64) -> Stats {
    Stats{ ..s, count: s.count + 1, total: s.total + v, min: s.min.min(v), max: s.max.max(v) }
}

type Pair = { a: i64, b: i64 }

fn main() {
    var s = Stats{ count: 0, total: 0.0, min: 1.0e308, max: 0.0 - 1.0e308, label: "run" }
    for v in [3.0, 1.0, 4.0] {
        s = add(s, v)
    }
    print("{s.label} n={s.count} total={s.total.to_fixed(1)} min={s.min.to_fixed(1)} max={s.max.to_fixed(1)}")

    // The base is unchanged: this builds a new value ([E-VALUE-SEMANTICS]).
    let bumped = Stats{ ..s, count: 99 }
    print("{s.count} {bumped.count} {bumped.label}")

    // No overrides at all is a copy.
    let copy = Stats{ ..s }
    print("{copy.count}")

    // Overriding every field is allowed, and the base still decides the type.
    let p = Pair{ a: 9, b: 9 }
    let q = Pair{ ..p, a: 1, b: 2 }
    print("{q.a} {q.b}")
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

## `G-TYPE-FALLIBLE.gr`

```grasm
//! tag: G-TYPE-FALLIBLE
//! expect: output
//! held=5
//! b!bad x
//! p=7
//! k=9
//! 3 42
//! ok 1
//! err bad x
//! ok 41
// `[i64 ! E]` is what `map` over a fallible function produces. The checker
// could always infer it; until now nothing could write it down, so a program
// that wanted to name the type declared a sum type by hand to carry it.
type E = { m: str }

fn parse(s: str) -> i64 ! E {
    s.parse_int() ?? fail(E{ m: "bad {s}" })
}

/// A fallible type as a parameter, which is the point of being able to write
/// it: this function could not be declared at all before.
fn total(rs: [i64 ! E]) -> i64 {
    var sum = 0
    for r in rs {
        sum = sum + (r ?? 0)
    }
    sum
}

/// A fallible parameter, which is what an error-reporting helper needs.
fn show(label: str, r: i64 ! E) -> str {
    match r {
        Ok(v) -> "{label}={v}"
        Err(e) -> "{label}!{e.m}"
    }
}

type Box = { r: i64 ! E, tag: str }

fn main() {
    // Held as a value, which the annotation is what permits.
    let held: i64 ! E = parse("5")
    print("{show("held", held)}")
    // A struct field, a tuple member, a type argument.
    let b = Box{ r: parse("x"), tag: "b" }
    print("{show(b.tag, b.r)}")
    let pair: (i64 ! E, str) = (parse("7"), "p")
    print("{show(pair.1, pair.0)}")
    let m: Map[str, i64 ! E] = {"k": parse("9")}
    print("{show("k", m.get("k") ?? parse("0"))}")

    let rs: [i64 ! E] = ["1", "x", "41"].map(parse)
    print("{rs.len()} {total(rs)}")
    for r in rs {
        match r {
            Ok(v) -> print("ok {v}")
            Err(e) -> print("err {e.m}")
        }
    }
}
```

## `G-TYPE-NOT-QUALIFIED.gr`

```grasm
//! tag: G-TYPE-NOT-QUALIFIED
//! expect: error
//! a type is named without a module qualifier
// Types share one global namespace ([T-MODULE-NS]), so a module qualifier on
// a type name would carry no information. The grammar used to permit one,
// which meant the only way to find out was the error.
type Thing = { n: i64 }

fn f(t: main.Thing) -> i64 {
    t.n
}

fn main() {
    print("{f(Thing{ n: 1 })}")
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
//! 1000000000000000.0
//! 1e300 has a point too
//! ()
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
    // Past 1e15, where the old rule quietly stopped applying: a float has to
    // stay distinguishable from an integer at every magnitude, or reading a
    // program's output means guessing which type produced it.
    print("{1000000000000000.0}")
    let big = 1.0e300
    let shown = "{big}"
    print("1e300 has a point too")
    assert(shown.contains("."))
    // Unit, which [T-PRIM] lists and the table did not.
    print("{()}")
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

## `L-INTERP-ONE.gr`

```grasm
//! tag: L-INTERP-ONE
//! expect: error
//! an interpolation holds one expression
// `{...}` holds exactly one expression, and anything left over is an error.
//
// It used to be a silent truncation: `"A{1 2}B"` printed `A1B`. The parser
// read one expression and discarded the rest of the braces' contents without
// a word, so a typo inside `{...}` quietly deleted part of the output. Both
// engines agreed on the wrong answer, which is why no differential test could
// ever have found it.
fn main() {
    print("A{1 2}B")
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

## `L-LIB-AS-I64.gr`

```grasm
//! tag: L-LIB-AS-I64
//! expect: output
//! 3 -3
//! 9223372036854775807 -9223372036854775808 0
// `as_i64` saturates. It is total, so it has an answer for every f64 and never
// panics — but "truncates toward zero" was the whole of what was written down,
// which says nothing about the values that have no i64 at all.
fn main() {
    print("{3.7.as_i64()} {(-3.7).as_i64()}")
    let huge = 1.0e30
    let nan = 0.0 / 0.0
    print("{huge.as_i64()} {(-huge).as_i64()} {nan.as_i64()}")
}
```

## `L-LIB-CMP.gr`

```grasm
//! tag: L-LIB-CMP
//! expect: output
//! -1 0 1
//! -1 1 -1
//! -1 0 1
//! -1 1
//! [(a, 3), (b, 3), (c, 1)]
// The total order sorting uses ([L-LIB-SORT-NAN]), as a number.
//
// It was specified, used internally by `sort`, and callable by nothing — so a
// program needing it reimplemented it with `<` and `>`, got NaN wrong, and
// scrambled a column exactly as that clause warns. A language that names the
// hazard should hand over the fix.
fn nan() -> f64 {
    "nan".parse_float() ?? 0.0
}

/// What `cmp` makes writable: a comparator sort, with per-key direction
/// decided at run time rather than baked into a key expression.
fn sort_desc_by_count(rows: [(str, i64)]) -> [(str, i64)] {
    var out: [(str, i64)] = []
    for r in rows {
        var at = out.len()
        var i = 0
        for o in out {
            // Count descending, then name ascending.
            let by_count = r.1.cmp(o.1)
            let better = if by_count != 0 { by_count > 0 } else { r.0.cmp(o.0) < 0 }
            if better and i < at {
                at = i
            }
            i += 1
        }
        var next: [(str, i64)] = []
        var j = 0
        for o in out {
            if j == at {
                next.push(r)
            }
            next.push(o)
            j += 1
        }
        if at >= out.len() {
            next.push(r)
        }
        out = next
    }
    out
}

fn main() {
    print("{(1).cmp(2)} {(2).cmp(2)} {(3).cmp(2)}")
    // NaN sorts after every number, which is the part hand-rolling gets wrong.
    print("{(1.0).cmp(2.0)} {nan().cmp(1.0)} {(1.0).cmp(nan())}")
    print("{"a".cmp("b")} {"b".cmp("b")} {"c".cmp("b")}")
    print("{(1, 2).cmp((1, 3))} {(2, 0).cmp((1, 9))}")
    print("{sort_desc_by_count([("c", 1), ("a", 3), ("b", 3)])}")
}
```

## `L-LIB-CODES.gr`

```grasm
//! tag: L-LIB-CODES
//! expect: output
//! 104 233
//! [104, 233, 108, 108, 111]
//! hé
//! héllo
//! 615042
//! non-bytes: 1 [8364]
fn main() {
    // Code points, counted in characters — `é` is one character, two bytes.
    print("{"héllo".code_at(0)} {"héllo".code_at(1)}")
    print("{"héllo".codes()}")
    // And back again.
    print("{104.to_char()}{233.to_char()}")
    // A full round trip through integers.
    var back = ""
    for c in "héllo".codes() {
        back = back + c.to_char()
    }
    print("{back}")
    // What the pair makes possible: arithmetic over text.
    print("{hash("grasm")}")
    // Characters outside ASCII are one code point, not their byte count.
    print("non-bytes: {"€".len()} {"€".codes()}")
}

/// A string hash, which cannot be written without code points at all.
fn hash(s: str) -> i64 {
    var h = 0
    for c in s.codes() {
        h = (h * 31 + c) % 1000007
    }
    h
}
```

## `L-LIB-FIXED.gr`

```grasm
//! tag: L-LIB-FIXED
//! expect: output
//! 3 4 1 -1
//! 0.13 0.38 -0.13
//! 2.67 9.99 1.00
//! 100 0.01
//! 289.22 289 289.22222
//! -1.5 0.000
//! 0.3333
//! [      3.50]
//! inf -inf NaN
//! 6.875 2.292
fn main() {
    // Ties go away from zero, matching `round` ([L-LIB-ROUND]).
    print("{(2.5).to_fixed(0)} {(3.5).to_fixed(0)} {(0.5).to_fixed(0)} {(0.0 - 0.5).to_fixed(0)}")
    print("{(0.125).to_fixed(2)} {(0.375).to_fixed(2)} {(0.0 - 0.125).to_fixed(2)}")
    // A tie is a property of the stored value: 2.675 is really 2.67499...
    print("{(2.675).to_fixed(2)} {(9.995).to_fixed(2)} {(1.005).to_fixed(2)}")
    // Carrying past the point.
    print("{(99.5).to_fixed(0)} {(0.005).to_fixed(2)}")
    let mean = 289.2222222222222
    print("{mean.to_fixed(2)} {mean.to_fixed(0)} {mean.to_fixed(5)}")
    // Negatives and zero keep the requested width.
    print("{(0.0 - 1.5).to_fixed(1)} {0.0.to_fixed(3)}")
    print("{(1.0 / 3.0).to_fixed(4)}")
    // It composes with the padding a column needs.
    print("[{(3.5).to_fixed(2).pad_start(10, " ")}]")
    // Not-finite values ignore the precision, as [L-DISPLAY] renders them.
    let z = 0.0
    print("{(1.0 / z).to_fixed(2)} {((0.0 - 1.0) / z).to_fixed(0)} {(z / z).to_fixed(3)}")
    var total = 0.0
    for x in [1.5, 2.25, 3.125] {
        total = total + x
    }
    print("{total.to_fixed(3)} {(total / 3.0).to_fixed(3)}")
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

## `L-LIB-IO.gr`

```grasm
//! tag: L-LIB-IO
//! expect: output
//! missing: NotFound(grasm_absent_9f3c.txt)
//! exists: false
//! env: unset
//! types: ok
// The io builtins report absence in three different ways, on purpose — and
// nothing pinned which was which, so `read_file` sat documented as returning
// `Option[str]` while it was fallible.
//
// Only the failing paths are exercised, because those are the ones that need
// no filesystem state and give the same answer on every machine. The writing
// side is still *typechecked*, in a branch that never runs.
fn main() {
    // Fallible: the error says why.
    match read_file("grasm_absent_9f3c.txt") {
        Ok(s) -> print("missing: read {s.len()}")
        Err(e) -> print("missing: {e}")
    }
    // bool: the question was already yes-or-no.
    print("exists: {file_exists("grasm_absent_9f3c.txt")}")
    // Option: an unset variable has no reason to give.
    print("env: {env("GRASM_A_VARIABLE_NOBODY_SETS_9F3C") ?? "unset"}")
    // Typechecked, never run — enough to pin the signatures.
    if file_exists("grasm_absent_9f3c.txt") {
        write_file("grasm_absent_9f3c.txt", "unused")
        let line = input()
        let ms = now_millis()
        print("{line} {ms}")
    }
    print("types: ok")
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
//! 6 6 123
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

    // A left fold, whose accumulator type comes from the initial value —
    // `[i64].fold("", ..)` produces a `str`.
    print("{xs.fold(0, |a, b| a + b)} {xs.fold(1, |a, b| a * b)} {xs.fold("", |a, b| a + b.to_str())}")
}
```

## `L-LIB-MAP.gr`

```grasm
//! tag: L-LIB-MAP
//! expect: output
//! 2 false 1 true [a, b] [1, 2]
//! [(a, 1), (b, 2)]
//! a=1
//! b=2
//! true 1
fn main() {
    var m = map()
    m.set("b", 2)
    m.set("a", 1)
    print("{m.len()} {m.is_empty()} {m.get("a") ?? 0} {m.has("b")} {m.keys()} {m.values()}")

    // Pairs, in key order. Without this the only way to walk a map was to
    // iterate `keys()` and look each one up again — a probe per entry, and a
    // `?? default` for a key that certainly exists.
    print("{m.entries()}")
    for e in m.entries() {
        match e {
            (k, v) -> print("{k}={v}")
        }
    }

    let gone = m.delete("b")
    print("{gone} {m.len()}")
}
```

## `L-LIB-MATH.gr`

```grasm
//! tag: L-LIB-MATH
//! expect: output
//! 3.141593 2.718282
//! 0.0000 1.0000 0.785398
//! 2.718282 1.000000 3.0000 3.0000
//! 5.00 3.00 4.0000 1.047198
//! 3.141593 180.00 1.1752 0.7616
//! 99.0
// The math library, and the reason it is a library at all.
//
// Reaching libm through `extern fn cos` works, and it costs a capability: a
// declared C function performs `io` ([T-EXTERN-VALUE]), so a package declaring
// `capabilities = []` could not compute a cosine. A pure function should not
// cost a capability.
fn shadowed() -> f64 {
    // `pi` and `e` are the only builtins that yield to a function of the same
    // name. `e` is common enough as an identifier that taking it outright
    // would break working programs.
    99.0
}

fn main() {
    print("{pi().to_fixed(6)} {e().to_fixed(6)}")
    print("{(0.0).sin().to_fixed(4)} {(0.0).cos().to_fixed(4)} {(1.0).atan().to_fixed(6)}")
    print("{(1.0).exp().to_fixed(6)} {e().ln().to_fixed(6)} {(8.0).log2().to_fixed(4)} {(1000.0).log10().to_fixed(4)}")
    print("{(3.0).hypot(4.0).to_fixed(2)} {(27.0).cbrt().to_fixed(2)} {(81.0).log(3.0).to_fixed(4)} {(0.5).acos().to_fixed(6)}")
    print("{(180.0).to_radians().to_fixed(6)} {pi().to_degrees().to_fixed(2)} {(1.0).sinh().to_fixed(4)} {(1.0).tanh().to_fixed(4)}")
    print("{shadowed()}")
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
//! 3
fn main() {
    let xs = [3, 1]
    let o = xs.get(0)
    print("{o.is_none()} {o.unwrap_or(0)} {xs.get(99).is_none()} {xs.get(99) ?? 0}")
    // For when absence is a bug rather than a case: the message is required,
    // because a panic nobody can read is not better than the `??` it replaced.
    print("{xs.first().expect("a two-element list has a first")}")
}
```

## `L-LIB-PAIRS.gr`

```grasm
//! tag: L-LIB-PAIRS
//! expect: output
//! [(0, a), (1, b), (2, c)]
//! [(1, x), (2, y)]
//! [] []
//! a0 b1 c2
//! 1x 2y
fn main() {
    let ws = ["a", "b", "c"]
    print("{ws.enumerate()}")
    // `zip` stops at the shorter of the two — pairing past the end of one
    // would have to invent an element.
    print("{[1, 2, 3].zip(["x", "y"])}")
    // Empty on either side gives empty.
    let none: [i64] = []
    print("{none.enumerate()} {none.zip(ws)}")
    // What they are for: naming both halves in the loop header.
    var out: [str] = []
    for (i, w) in ws.enumerate() {
        out.push("{w}{i}")
    }
    print("{out.join(" ")}")
    var pairs: [str] = []
    for (n, w) in [1, 2].zip(["x", "y"]) {
        pairs.push("{n}{w}")
    }
    print("{pairs.join(" ")}")
}
```

## `L-LIB-PARSE.gr`

```grasm
//! tag: L-LIB-PARSE
//! expect: output
//! inf NaN inf -inf
//! none none
//! 42 -7 none none
//! [1.5, -2.5] true
//! cdef |  |
// What a parser does with input that is nearly a number. Untrusted data
// reaches these, and every surprise here becomes a surprise somewhere else:
// `"1e400"` parsing to `inf` is what put an infinity into `as_i64` and into a
// sort, so both of those had to be specified too.
fn show(o: Option[f64]) -> str {
    match o {
        Some(v) -> "{v}"
        None -> "none"
    }
}

fn showi(o: Option[i64]) -> str {
    match o {
        Some(v) -> "{v}"
        None -> "none"
    }
}

fn main() {
    // Infinities and NaN are accepted, and overflow gives an infinity.
    print("{show("inf".parse_float())} {show("nan".parse_float())} {show("1e400".parse_float())} {show("-inf".parse_float())}")
    // Text that is not a number is `None`, not zero.
    print("{show("abc".parse_float())} {showi("abc".parse_int())}")
    // Space and a sign are fine; source-literal spellings are not.
    print("{showi(" +42 ".parse_int())} {showi("-7".parse_int())} {showi("1_000".parse_int())} {showi("0xFF".parse_int())}")
    // Which is why a program reading untrusted numbers can ask.
    var kept: [f64] = []
    for c in ["1.5", "1e400", "nan", "-2.5"] {
        let v = c.parse_float() ?? 0.0
        if v.is_finite() {
            kept.push(v)
        }
    }
    print("{kept} {(0.0 / 0.0).is_nan()}")
    // `substr` clamps rather than panicking, unlike indexing.
    print("{"abcdef".substr(2, 100)} | {"abc".substr(5, 2)} |")
}
```

## `L-LIB-REMOVE.gr`

```grasm
//! tag: L-LIB-REMOVE
//! expect: output
//! 10 30 [20, 40]
//! b [a, c]
//! x 1
//! [3, 1, 2]
// Taking an element out by index, which is what draining a queue from the
// front needs.
type Job = { id: i64, name: str }

fn main() {
    var q = [10, 20, 30, 40]
    let first = q.remove(0)
    let mid = q.remove(1)
    print("{first} {mid} {q}")

    // Boxed elements too.
    var names = ["a", "b", "c"]
    let b = names.remove(1)
    print("{b} {names}")
    var jobs = [Job{ id: 1, name: "x" }, Job{ id: 2, name: "y" }]
    let j = jobs.remove(0)
    print("{j.name} {jobs.len()}")

    // The shape it exists for: a queue drained in order.
    var ready = [3, 1, 2]
    var order: [i64] = []
    while ready.len() > 0 {
        order.push(ready.remove(0))
    }
    print("{order}")
}
```

## `L-LIB-ROUND.gr`

```grasm
//! tag: L-LIB-ROUND
//! expect: output
//! 3.0 4.0 -3.0 -1.0
//! 2.0 3.0 -2.0 -0.0
//! inf -inf NaN
//! -9223372036854775808
fn main() {
    // Ties go away from zero, not to even.
    print("{(2.5).round()} {(3.5).round()} {(0.0 - 2.5).round()} {(0.0 - 0.5).round()}")
    // Non-ties are unremarkable, and `-0.4` rounds to `-0.0`, not `0.0`.
    print("{(2.4).round()} {(2.6).round()} {(0.0 - 2.4).round()} {(0.0 - 0.4).round()}")
    // Division by zero on floats is not a panic; it is the IEEE answer.
    let z = 0.0
    print("{1.0 / z} {(0.0 - 1.0) / z} {z / z}")
    // The one integer with no positive counterpart wraps to itself.
    let least = 0 - 9223372036854775807 - 1
    print("{least.abs()}")
}
```

## `L-LIB-SORT-BY.gr`

```grasm
//! tag: L-LIB-SORT-BY
//! expect: output
//! al 25
//! cy 30
//! bo 30
//! [3, 2, 1]
//! [a, bb, ccc]
//! y
//! z
//! [7] []
type P = { name: str, age: i64 }

fn main() {
    // A list of structs has no order of its own; the key does.
    var ps = [P{ name: "cy", age: 30 }, P{ name: "al", age: 25 }, P{ name: "bo", age: 30 }]
    ps.sort_by(|p| p.age)
    // Stable: `cy` was written before `bo` and both are 30, so it stays first.
    for p in ps {
        print("{p.name} {p.age}")
    }
    // Any i64/f64/str key works, including a computed one.
    var ns = [3, 1, 2]
    ns.sort_by(|n| 0 - n)
    print("{ns}")
    var ws = ["ccc", "a", "bb"]
    ws.sort_by(|w| w.len())
    print("{ws}")
    // A key extracted from a named function works the same as a closure.
    var ps2 = [P{ name: "z", age: 1 }, P{ name: "y", age: 0 }]
    ps2.sort_by(age_of)
    for p in ps2 {
        print("{p.name}")
    }
    // An empty list and a single element are both no-ops, not errors.
    var one = [7]
    one.sort_by(|n| n)
    var none: [i64] = []
    none.sort_by(|n| n)
    print("{one} {none}")
}

fn age_of(p: P) -> i64 {
    p.age
}
```

## `L-LIB-SORT-NAN.gr`

```grasm
//! tag: L-LIB-SORT-NAN
//! expect: output
//! [1.0, 2.0, 3.0, 4.0, 5.0, NaN]
//! [-inf, -1.0, 0.0, 1.0, inf, NaN]
//! [(1.0, a), (2.0, b), (NaN, c)]
// A NaN in a list of floats used to leave the whole list unsorted, not just
// the NaN misplaced: calling every NaN comparison "equal" is not an order,
// and a sort given a non-order returns whatever it happens to return. One
// `nan` cell in an otherwise clean file was enough to scramble a column.
fn nan() -> f64 {
    "nan".parse_float() ?? 0.0
}

fn main() {
    var xs = [5.0, 4.0, 3.0, nan(), 2.0, 1.0]
    xs.sort()
    print("{xs}")
    // NaN sorts last; the infinities sit at the ends where they belong.
    var ys = [1.0, nan(), 0.0 - 1.0, 1.0 / 0.0, 0.0, (0.0 - 1.0) / 0.0]
    ys.sort()
    print("{ys}")
    // And the same order is what a float sort key uses.
    var ps = [(nan(), "c"), (2.0, "b"), (1.0, "a")]
    ps.sort_by(|p| p.0)
    print("{ps}")
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

## `L-LIB-SUM.gr`

```grasm
//! tag: L-LIB-SUM
//! expect: output
//! 6 0
//! 6.0 3.0
//! 0
//! 0.0 0.0000
//! 0.0
// The sum of an empty list is the zero of the element type.
//
// The last two lines were the deviation this file used to work around. The
// interpreter read int-versus-float off the elements, and an empty list has
// none, so `[f64].sum()` was `0`: it printed without a point and then panicked
// `no method to_fixed on 0` when the caller used the `f64` the checker had
// promised. This program asserted only what both engines agreed on, so it
// passed throughout. The deviation was recorded on the grounds that fixing it
// needed the interpreter to carry element types; it needed the checker to
// mention what it already knew.
fn total(xs: [f64]) -> f64 {
    xs.sum()
}

fn main() {
    // Integers, empty and not.
    let some: [i64] = [1, 2, 3]
    let none: [i64] = []
    print("{some.sum()} {none.sum()}")

    // Floats with elements are unambiguous in both engines.
    let fs: [f64] = [1.0, 2.0, 3.0]
    print("{fs.sum()} {fs.filter(|x| x < 3.5).sum() - 3.0}")

    // A filter that removes everything still has the element type.
    let gone = some.filter(|x| x > 99)
    print("{gone.sum()}")

    // An empty float list, directly and through a function that declares the
    // return type — the shape that used to panic.
    let empty: [f64] = []
    print("{empty.sum()} {total(empty).to_fixed(4)}")
    // And an emptied one, where the element type comes from the source list.
    print("{fs.filter(|x| x > 99.0).sum()}")
}
```

## `L-LIB-TO-STR.gr`

```grasm
//! tag: L-LIB-TO-STR
//! expect: output
//! 1.0-2.5-0.0-1000000000000000.0
//! 0.0 3.0 0.0
//! 7 -12
// `to_str` gives exactly what `print` gives.
//
// The interpreter handed the job to its host language, which drops the point
// on a whole float. So `(1.0).to_str()` was `"1"` under `grasm run` and `"1.0"`
// compiled, and the documented idiom below printed a `[f64]` as a list of
// integers — the one thing [L-DISPLAY] exists to prevent, in the engine this
// specification calls normative.
fn main() {
    let xs: [f64] = [1.0, 2.5, 0.0, 1.0e15]
    print("{xs.map(|v| v.to_str()).join("-")}")
    // Through a conversion, and through a sum with nothing to add.
    let empty: [f64] = []
    print("{(0 - 0.0).to_str()} {(3).as_f64().to_str()} {empty.sum().to_str()}")
    // An integer is still an integer.
    print("{(7).to_str()} {(0 - 12).to_str()}")
}
```

## `L-LIB-TRIM-SIDES.gr`

```grasm
//! tag: L-LIB-TRIM-SIDES
//! expect: output
//! [  hi] [hi  ] [hi]
//! [    let x = 1]
// Trimming one side. `trim` alone loses the distinction, and a line's leading
// indentation is often exactly what you are trying to keep.
fn main() {
    let s = "  hi  "
    print("[{s.trim_end()}] [{s.trim_start()}] [{s.trim()}]")
    // The case it exists for: dropping the trailing newline and spaces a file
    // leaves on a line without flattening the indentation that carries meaning.
    let line = "    let x = 1   "
    print("[{line.trim_end()}]")
}
```

## `L-LIB-TUPLE.gr`

```grasm
//! tag: L-LIB-TUPLE
//! expect: output
//! 1 a
//! eng 25 bo
//! eng 30 cy
//! ops 25 al
//! [(c, 5), (a, 3), (b, 3)]
//! true false true
//! [(1, 2), (1, 5), (2, 1)]
type P = { name: str, dept: str, age: i64 }

fn main() {
    // Components are read by position.
    let pair = (1, "a")
    print("{pair.0} {pair.1}")

    // A tuple key is a multi-key sort: department, then age, then name.
    var ps = [
        P{ name: "cy", dept: "eng", age: 30 },
        P{ name: "al", dept: "ops", age: 25 },
        P{ name: "bo", dept: "eng", age: 25 },
    ]
    ps.sort_by(|p| (p.dept, p.age, p.name))
    for p in ps {
        print("{p.dept} {p.age} {p.name}")
    }

    // The counting idiom: most frequent first, ties broken by key ascending.
    var pairs = [("b", 3), ("a", 3), ("c", 5)]
    pairs.sort_by(|e| (0 - e.1, e.0))
    print("{pairs}")

    // The same order is what `<` and `sort` use.
    print("{(1, 2) < (1, 3)} {(2, 0) < (1, 9)} {(1, "a") < (1, "b")}")
    var ts = [(2, 1), (1, 5), (1, 2)]
    ts.sort()
    print("{ts}")
}
```

## `L-LIB-WORDS.gr`

```grasm
//! tag: L-LIB-WORDS
//! expect: output
//! [2026-08-09T10:00:00Z, INFO, server, started]
//! 10 4
//! [] [] [a, b, c]
//! [a, , b] [a, b]
fn main() {
    // A line of aligned columns: what `split(" ")` cannot read.
    let line = "2026-08-09T10:00:00Z INFO     server   started"
    print("{line.words()}")
    print("{line.split(" ").len()} {line.words().len()}")
    // Empty, all-whitespace, and mixed whitespace kinds.
    print("{"".words()} {"   ".words()} {"\ta\nb  c ".words()}")
    // The two are different on purpose: `split` keeps the empty, `words` does
    // not, and a CSV needs the first while a log line needs the second.
    print("{"a,,b".split(",")} {"a  b".words()}")
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

## `L-RAW-STRING.gr`

```grasm
//! tag: L-RAW-STRING
//! expect: output
//! 3
//! [{"users": [{"name": "ada"}], "ok": true}]
//! [a	tab and "quotes" kept]
//! [a \n that is two characters]
//! 4 [97, 92, 110, 98]
//! interpolation still works: 2
//! empty:[] one:[x]
// A raw block: every character literal, newlines included.
let DOC = """
{"users": [{"name": "ada"}], "ok": true}
a	tab and "quotes" kept
a \n that is two characters
"""

let ESCAPES = """a\nb"""
let EMPTY = """"""
let ONE = """x"""

fn main() {
    print("{DOC.lines().len()}")
    for l in DOC.lines() {
        print("[{l}]")
    }
    // `\n` inside a raw block is a backslash and an `n`, not a newline.
    print("{ESCAPES.len()} {ESCAPES.codes()}")
    // Ordinary strings are unaffected.
    print("interpolation still works: {1 + 1}")
    print("empty:[{EMPTY}] one:[{ONE}]")
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
//! expect: output
//! 3.0
//! 2.0
//! ok
// Every arm of a `match` used as a value produces the same type.
//
// This program is here because a `match` whose arms the backend could not
// type did not fail — inside a fallible function it compiled, and then
// panicked at run time with `unbox: not an f64` and no source location.
// Wrong code, from arms that were perfectly well typed by the checker and
// merely unknown to the backend's own inference table.
type Diag = { msg: str }

fn call(name: str, a: f64, b: f64) -> f64 ! Diag {
    match name {
        "sqrt" -> a.sqrt()
        "min" -> a.min(b)
        _ -> fail(Diag{ msg: "unknown function {name}" })
    }
}

fn show(name: str, a: f64, b: f64) {
    match call(name, a, b) {
        Ok(v) -> print("{v}")
        Err(d) -> print("{d.msg}")
    }
}

fn main() {
    show("sqrt", 9.0, 0.0)
    show("min", 9.0, 2.0)
    print("ok")
}
```

## `T-CAPTURE-WRITE.gr`

```grasm
//! tag: T-CAPTURE-WRITE
//! expect: error
//! captured from outside
// A closure captures values, not bindings ([E-CLOSURE-CAPTURE]), so a write
// to a captured name changes a copy that is discarded when the call returns.
// It used to compile and silently do nothing — which is how a handler that
// appears to accumulate state accumulates none.
fn main() {
    var seen: [i64] = []
    let remember = |v| seen.push(v)
    remember(1)
    remember(2)
    print("{seen.len()}")
}
```

## `T-CONST-PUB.gr`

```grasm
//! tag: T-CONST-PUB
//! expect: output
//! 42 84
// `pub let` exports a constant. A bare `let` keeps it to its own module.
//
// `pub` on a constant used to parse, check, and do nothing at all: no other
// module could read one, `doc` omitted it and `api` omitted it. Syntax with no
// meaning is worse than an error, because it looks like it worked.
pub let WIDTH = 42
let SCALE = 2

pub fn scaled() -> i64 {
    WIDTH * SCALE
}

fn main() {
    print("{WIDTH} {scaled()}")
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

## `T-DIVERGE.gr`

```grasm
//! tag: T-DIVERGE
//! expect: output
//! 4 -1
//! pos non-pos b c
//! -2 0
//! 3
//! 7
// `return`, `break` and `continue` leave before a value exists, so they fit
// wherever an expression is expected. Reading a returning arm as `()` used to
// put the type error on whichever *other* arm did produce a value.
type S = | A(v: i64) | B | C(s: str)

fn classify(s: S) -> str {
    match s {
        A(v) -> if v > 0 { "pos" } else { return "non-pos" }
        B -> return "b"
        C(t) -> t
    }
}

/// `continue` as an arm body, inside a loop.
fn first_negative(xs: [i64]) -> i64 {
    for x in xs {
        match x {
            0 -> continue
            _ -> if x < 0 { return x }
        }
    }
    0
}

/// `break` as an arm body.
fn sum_until_nine(xs: [i64]) -> i64 {
    var total = 0
    for x in xs {
        match x {
            9 -> break
            _ -> total = total + x
        }
    }
    total
}

/// `fail` already had this rule; the three now share it.
fn checked(n: i64) -> i64 ! S {
    match n {
        0 -> fail(S.B)
        _ -> n
    }
}

/// `fail` is an expression and may sit on the right of `??`; `return` is a
/// statement and may not ([E-COALESCE-DIVERGE]).
fn from_option(o: Option[i64]) -> i64 ! S {
    o ?? fail(S.B)
}

fn main() {
    print("{from_option(Some(4)) ?? 0} {from_option(None) ?? 0 - 1}")
    print("{classify(S.A(1))} {classify(S.A(-1))} {classify(S.B)} {classify(S.C("c"))}")
    print("{first_negative([0, 3, -2, 5])} {first_negative([1, 2])}")
    print("{sum_until_nine([1, 2, 9, 100])}")
    print("{checked(7) ?? 0}")
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
//! I: starting
//! W: disk almost full
//! a=1
//! b=2
//! 3
//! 42 9 0
//! 8
// Declaring `Cfg` also declares the handler type `CfgHandler`, whose fields
// are the operations.
effect Cfg {
    fn value() -> i64
}

// Operations take parameters, and more than one, and may return nothing.
// A handler closure receives them in declaration order.
effect Log {
    fn info(msg: str)
    fn warn(msg: str)
}

effect Kv {
    fn put(key: str, value: i64)
    fn size() -> i64
}

fn main() {
    with Cfg handled_by CfgHandler{ value: || 99 } {
        print("{Cfg.value()}")
    }

    // A handler body may itself perform effects — here `io` — which is the
    // shape almost every real handler has and the one worth demonstrating.
    with Log handled_by LogHandler{ info: |m| print("I: {m}"), warn: |m| print("W: {m}") } {
        Log.info("starting")
        Log.warn("disk almost full")
    }

    // A handler bound to a name first, rather than written at `handled_by`.
    let store = KvHandler{ put: |k, v| print("{k}={v}"), size: || 3 }
    with Kv handled_by store {
        Kv.put("a", 1)
        Kv.put("b", 2)
        print("{Kv.size()}")
    }

    // A handler that reads its enclosing scope. This is most of what a
    // handler is for — injecting a table, a configuration, a fake clock —
    // and it used to compile only under the interpreter.
    var table = map()
    table.set("x", 42)
    table.set("y", 9)
    with Kv handled_by KvHandler{ put: |k, v| print("{k}={v}"), size: || table.len() } {
        print("{table.get("x") ?? 0} {table.get("y") ?? 0} {table.get("z") ?? 0}")
    }

    // The captured value is read at the moment the handler is installed, and
    // an operation may be reached from any depth while the block is open.
    let scale = 4
    with Cfg handled_by CfgHandler{ value: || scale * 2 } {
        print("{doubled()}")
    }
}

fn doubled() -> i64 {
    Cfg.value()
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

## `T-EFFECT-FALLIBLE-CONSUME.gr`

```grasm
//! tag: T-EFFECT-FALLIBLE-CONSUME
//! expect: output
//! V DEFAULT
//! ok V / err no bad
//! got V / caught no bad
// A fallible effect operation answers with a `T ! E`, so all three ways of
// consuming one work — not only `try`.
//
// A handler is an ordinary closure, so calling one produced a bare value and a
// `fail` inside it propagated straight past the caller. `??` fell through to
// nothing and `match` panicked with "no match arm matched `V`" on the
// *success* path. Only `try` worked, which is the one form the original
// conformance program used.
type A = { a: str }

effect Src {
    fn fetch(k: str) -> str ! A
}

fn with_default(k: str) -> str / {Src} {
    Src.fetch(k) ?? "DEFAULT"
}

fn matched(k: str) -> str / {Src} {
    match Src.fetch(k) {
        Ok(v) -> "ok {v}"
        Err(e) -> "err {e.a}"
    }
}

fn propagated(k: str) -> str ! A / {Src} {
    let v = try Src.fetch(k)
    "got {v}"
}

fn main() {
    with Src handled_by SrcHandler{ fetch: |k| if k == "good" { "V" } else { fail(A{ a: "no {k}" }) } } {
        print("{with_default("good")} {with_default("bad")}")
        print("{matched("good")} / {matched("bad")}")
        let a = propagated("good") ?? "?"
        let b = match propagated("bad") {
            Ok(v) -> v
            Err(e) -> "caught {e.a}"
        }
        print("{a} / {b}")
    }
}
```

## `T-EFFECT-FALLIBLE.gr`

```grasm
//! tag: T-EFFECT-FALLIBLE
//! expect: output
//! 42.0
//! err: missing y
// An effect operation that can fail.
//
// The grammar has always allowed `! E` on an operation, and the checker used
// to drop it: an error raised in a handler crossed every function on the way
// out while `grasm check` reported no errors, which defeats the rule that a
// fallible value cannot be used as if it were plain. Now the operation's
// result is fallible, so a caller must `try` it and declare an error type of
// its own — and a handler must produce one.
type Diag = { msg: str }

effect Cfg {
    fn get(name: str) -> f64 ! Diag
}

fn read(name: str) -> f64 ! Diag / {Cfg} {
    let v = try Cfg.get(name)
    v * 2.0
}

fn show(name: str) / {Cfg, io} {
    match read(name) {
        Ok(v) -> print("{v}")
        Err(d) -> print("err: {d.msg}")
    }
}

fn main() {
    // A handler body producing a plain value is producing `Ok(value)`, the
    // same rule a fallible function's body follows.
    with Cfg handled_by CfgHandler{
        get: |n| if n == "x" { 21.0 } else { fail(Diag{ msg: "missing {n}" }) },
    } {
        show("x")
        show("y")
    }
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

## `T-ERR-UNION-OPAQUE.gr`

```grasm
//! tag: T-ERR-UNION-OPAQUE
//! expect: error
//! has no field `key`
// A union error type cannot be narrowed back to one of its members.
//
// It can be raised, propagated, caught and printed — and a function declaring
// two error types can therefore say *that* it failed and not *which way*,
// which is usually why two were declared. [T-ERR-SUBSET] promotes a caller's
// error type to this union automatically, so a program arrives here without
// asking. The way to branch on the failure today is one error type that is a
// sum. This is recorded as a gap rather than left for a reader to discover.
type NotFound = { key: str }
type Cycle = { path: [str] }

fn boom(w: i64) -> i64 ! (NotFound | Cycle) {
    fail(NotFound{ key: "k" })
}

fn main() {
    match boom(0) {
        Ok(v) -> print("{v}")
        Err(e) -> print("{e.key}")
    }
}
```

## `T-EXTERN-VALUE.gr`

```grasm
//! tag: T-EXTERN-VALUE
//! expect: output
//! 2.0 3.0
//! [1.0, 2.0, 3.0]
// A C function named without being called.
//
// The compiled backend made one a function value and the interpreter said
// `unknown name` — for a program `check` accepted. And naming one carried no
// effect at all, so `xs.par_map(sqrt)` typechecked: the purity proof, the
// declared row and a package's capability ceiling all passed something that
// reaches libc. Holding a C function is the same permission as calling one.
extern fn sqrt(x: f64) -> f64

fn apply(f: fn(f64) -> f64 / {io}, v: f64) -> f64 / {io} {
    f(v)
}

fn main() {
    unsafe {
        // Naming one is the same permission as calling one, so the *name*
        // needs `unsafe` too, not just the call.
        let f = sqrt
        print("{f(4.0)} {apply(sqrt, 9.0)}")
        print("{[1.0, 4.0, 9.0].map(sqrt)}")
    }
}
```

## `T-EXTERN.gr`

```grasm
//! tag: T-EXTERN
//! expect: output
//! 7 4.0
//! 3.0 -2
//! true
// Calling C. The compiled backend resolves the symbol at link time; the
// interpreter finds it in its own process, which already has libc loaded.
// Both make the identical call.
//
// Signatures are `i64` and `f64` only. `bool` is excluded on purpose: C's is
// one byte and Grasm's is a register, and a silent width mismatch at an FFI
// boundary shows up as garbage three calls later.
extern fn abs(n: i64) -> i64
extern fn sqrt(x: f64) -> f64
extern fn pow(base: f64, exp: f64) -> f64
extern fn getpid() -> i64

fn main() {
    // Every use is inside `unsafe` — Tier 3 ([T-UNSAFE-FFI]). Leaving the
    // safe tiers is the thing being asked for, and it is asked for in the
    // source and again in the manifest.
    let a = unsafe {
        abs(0 - 7)
    }
    let s = unsafe {
        sqrt(16.0)
    }
    print("{a} {s}")
    unsafe {
        print("{pow(3.0, 1.0)} {abs(2) - 4}")
        // A real process id, so this proves the call actually happened.
        print("{getpid() > 0}")
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

## `T-FALLIBLE-COALESCE.gr`

```grasm
//! tag: T-FALLIBLE-COALESCE
//! expect: output
//! 42
//! 0
//! []
//! 101
//! evaluated once
// `??` over a fallible value, which used to be a four-line `match`.
type E = { m: str }

fn parse(s: str) -> i64 ! E {
    match s.parse_int() {
        Some(n) -> n
        None -> fail(E{ m: "not a number: {s}" })
    }
}

fn names() -> [str] ! E {
    fail(E{ m: "none" })
}

/// Prints when it runs, so laziness is observable rather than asserted.
fn fallback() -> i64 {
    print("evaluated once")
    -1
}

fn main() {
    print("{parse("42") ?? 0}")
    print("{parse("oops") ?? 0}")
    print("{names() ?? []}")
    print("{(parse("1") ?? 0) + (parse("x") ?? 100)}")
    // The success path does not evaluate the fallback; the failing one does.
    let _ok = parse("5") ?? fallback()
    let _bad = parse("z") ?? fallback()
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

## `T-FALLIBLE-VALUE.gr`

```grasm
//! tag: T-FALLIBLE-VALUE
//! expect: error
//! found `fn(i64) -> bool ! E`
// A fallible function passed where a plain predicate is wanted.
//
// This used to typecheck, because a function referenced by name lost its
// `! E`. `filter` then tested `Ok(true)`, which is not `true`, so every
// element was dropped and the program printed `[]` and exited 0.
type E = { m: str }

fn odd(n: i64) -> bool ! E {
    if n == 3 {
        fail(E{ m: "three" })
    }
    n % 2 == 1
}

fn main() {
    print("{[1, 2, 3, 4].filter(odd)}")
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

## `T-FIELD-CALL.gr`

```grasm
//! tag: T-FIELD-CALL
//! expect: output
//! 10 hi bob
//! 105 BOB
//! 42 a
// A struct field holding a function is called through the field.
//
// Storing one and reading it back both worked; `n.run(5)` did not. The only
// spelling that ran was `let f = n.run` and then `f(5)`, which nothing pointed
// to — the error suggested declaring a free `fn run(Node, ...)`, which is a
// different function and would not call the field.
type Node = { name: str, run: fn(i64) -> i64, greet: fn(str) -> str }

fn twice(n: i64) -> i64 {
    n * 2
}

fn main() {
    let n = Node{ name: "a", run: twice, greet: |s| "hi {s}" }
    print("{n.run(5)} {n.greet("bob")}")
    // A closure that captured something, stored in a field and called there.
    let base = 100
    let m = Node{ name: "b", run: |x| x + base, greet: |s| s.to_upper() }
    print("{m.run(5)} {m.greet("bob")}")
    // Reading the field as a value still works, and a plain field is still a
    // plain field.
    let f = n.run
    print("{f(21)} {n.name}")
}
```

## `T-FN-TYPE-ROW-CARRIED.gr`

```grasm
//! tag: T-FN-TYPE-ROW-CARRIED
//! expect: error
//! but this one performs `io`
// A function's effect row travels with it, into every shape.
//
// The row check ran in one place: a call argument whose parameter type was
// written inline as `fn(...)`. Every other route from a named function into a
// written function type dropped the row — a return type, a `type Stage =
// fn(..)` alias, a struct field, a list element, a map value, a tuple member.
// Each was a way to hand a function that prints to a `/ {}` row, past
// [T-PAR-MAP-PURE], past the missing-handler check [E-HANDLER-MISSING] calls a
// backstop "well-typed programs never reach", and past a package's
// `capabilities = []` ceiling — which is the strongest guarantee here.
//
// The cause was that the row was not part of the *type*: `fn(i64) -> i64` and
// `fn(i64) -> i64 / {io}` were the same type, so there was nothing to compare.
fn shout(n: i64) -> i64 {
    print("noisy")
    n
}

type Box = { f: fn(i64) -> i64 }

fn main() {
    let b = Box{ f: shout }
    print("{b.f(1)}")
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

## `T-FN-VALUE-ROW.gr`

```grasm
//! tag: T-FN-VALUE-ROW
//! expect: error
//! performs effect(s) `io` not in its declared row
// Naming a function carries its effect row, calling it or not.
//
// A built-in higher-order method's function parameter is deliberately
// unconstrained — `xs.map(|x| print(x))` is ordinary — so a *named* effectful
// function handed to one had no row to be checked against, and
// `[1, 2].map(shout)` inside a `/ {}` function typechecked and printed. One
// hop further, `let fs = [uses]` then `fs.map(call)`, defeated even the
// hand-written purity check on `par_map`: the effectful function is never
// called in the source, only mentioned, and something else calls it.
//
// The rule is the one already applied to a declared C function — holding one
// is the permission to call it. It is conservative, and it is what keeps
// `/ {}`, [T-PAR-MAP-PURE], [E-HANDLER-MISSING] and a package's
// `capabilities = []` from being one indirection away from meaningless.
fn shout(n: i64) -> i64 {
    print("noisy {n}")
    n
}

fn quiet() -> [i64] / {} {
    [1, 2].map(shout)
}

fn main() {
    print("{quiet()}")
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

## `T-INOUT-REF.gr`

```grasm
//! tag: T-INOUT-REF
//! expect: error
//! cannot pass `r` as `inout` — it is reached through a `ref`
// Two sound ways of mutating that cannot be combined.
//
// `inout` snapshots the place at the call and writes it back at the return. A
// `ref` reaches the same value without that snapshot, so the writeback wins
// and the write made through the reference vanishes — silently, and the same
// way whichever order the two happen in. Exclusivity did not catch it: that
// rule compares binding names, and `r.xs` and `r` are two names for one slot.
type B = { xs: [i64] }

fn f(inout a: [i64], r: ref B) {
    a.push(1)
    r.xs.push(2)
}

fn main() {
    var r = ref(B{ xs: [0] })
    f(r.xs, r)
    print("{r.xs}")
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

## `T-INT-TO-FLOAT-CARRIED.gr`

```grasm
//! tag: T-INT-TO-FLOAT-CARRIED
//! expect: output
//! [1.5, 3.0] 3.0
//! true true
//! 7.0 4.0 -2.0
//! 3.0 true
//! 1.5 3.0
// An integer literal in an `f64` position is a float *everywhere*.
//
// [T-INT-TO-FLOAT] says the conversion happens at compile time, and only the
// checker knows which literals are affected. Neither engine was told, so each
// converted where it happened to remember to. The interpreter — the normative
// one — kept `xs.push(3)` into a `[f64]` as an integer, and that list then
// sorted wrong, compared unequal to the same list written `3.0`, and printed
// with no point. The compiled backend forgot elsewhere: a `Map[str, f64]`
// value panicked with `unbox: not an f64`, and `contains(3)` answered `false`
// to a comparison that was true.
type P = { v: f64 }

fn arg(v: f64) -> f64 {
    v
}

fn main() {
    var xs: [f64] = []
    xs.push(3)
    xs.push(1.5)
    xs.sort()
    print("{xs} {xs.max() ?? 0.0}")
    // Structural equality has to see the same value on both sides.
    let ys: [f64] = [1.0]
    var zs: [f64] = []
    zs.push(1)
    print("{P{ v: 1 } == P{ v: 1.0 }} {ys == zs}")
    // A parameter, a map value, a negated literal.
    var m: Map[str, f64] = {}
    m.set("a", 4)
    print("{arg(7)} {m.get("a") ?? 0.0} {arg(-2)}")
    // Inferred from a sibling element rather than from an annotation.
    let x = 1.5
    print("{[x, 3].max() ?? 0.0} {[x, 3, 2, -4].contains(3)}")
    // A method argument. The checker accepted this and the interpreter
    // panicked with `min expects a float`.
    print("{x.min(3)} {x.max(3)}")
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

## `T-MAP-KEY-GENERIC.gr`

```grasm
//! tag: T-MAP-KEY-GENERIC
//! expect: error
//! map keys must be `i64`, `str`, or `bool`, not `f64`
// A type parameter used as a map key is checked at the call site.
//
// Only the parameters were checked, and this function puts `K` nowhere in its
// parameters — so `fresh(2.5)` typechecked and panicked with `invalid map key`
// at run time. The result is part of the instantiated signature too.
fn fresh[K](k: K) -> Map[K, i64] {
    var m: Map[K, i64] = {}
    m.set(k, 1)
    m
}

fn main() {
    print("{fresh(2.5)}")
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

## `T-MATCH-ALT.gr`

```grasm
//! tag: T-MATCH-ALT
//! expect: output
//! 1 2 0
//! 3 4 3
//! true false
// An arm listing alternatives, which covers every one of them.
//
// Without this, grouping a handful of literals meant an arm each — the kind
// of tax that pushes people to match on strings and throw away the sum type
// they were given.
type Shape =
    | Circle
    | Square
    | Tri

fn arity(name: str) -> i64 {
    match name {
        "sqrt" | "abs" | "round" -> 1
        "min" | "max" -> 2
        _ -> 0
    }
}

/// Alternatives count toward exhaustiveness: no `_` arm is needed here.
fn sides(s: Shape) -> i64 {
    match s {
        Circle | Tri -> 3
        Square -> 4
    }
}

fn small(n: i64) -> bool {
    match n {
        0 | 1 | 2 -> true
        _ -> false
    }
}

fn main() {
    print("{arity("abs")} {arity("min")} {arity("nope")}")
    print("{sides(Circle)} {sides(Square)} {sides(Tri)}")
    print("{small(1)} {small(9)}")
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

## `T-MATCH-NESTED-COVER.gr`

```grasm
//! tag: T-MATCH-NESTED-COVER
//! expect: output
//! wrapped b plain none a
// The other side of [T-MATCH-NESTED]: payload patterns that between them cover
// the payload type do cover the variant, so the stricter rule does not push
// anyone toward a `_` that would hide the next variant added.
type Inner = | A | B
type Outer = | Wrap(i: Inner) | Plain

fn describe(o: Outer) -> str {
    match o {
        Wrap(A) -> "wrapped a"
        Wrap(B) -> "wrapped b"
        Plain -> "plain"
    }
}

fn opt(o: Option[Inner]) -> str {
    match o {
        Some(A) -> "a"
        Some(B) -> "b"
        None -> "none"
    }
}

fn main() {
    // And a binding payload still covers the variant outright.
    let bound = match Plain {
        Wrap(i) -> "w"
        Plain -> "plain"
    }
    print("{describe(Wrap(B))} {bound} {opt(None)} {opt(Some(A))}")
}
```

## `T-MATCH-NESTED.gr`

```grasm
//! tag: T-MATCH-NESTED
//! expect: error
//! match is not exhaustive — missing `Wrap`
// A payload pattern does not cover the whole variant.
//
// `Wrap(A)` matches only the `Wrap`s carrying `A`, and it was recorded as
// covering every `Wrap`. So this compiled and then panicked on `Wrap(B)` — the
// runtime surprise [T-MATCH-EXHAUSTIVE] exists to prevent, and the one the
// tutorial advertises against by name. The same held for an integer payload,
// an `Option` payload and an `Ok`/`Err` payload.
//
// `Wrap(A)` and `Wrap(B)` in two arms still cover `Wrap`, because between them
// they cover `Inner` — refusing that would push people to a `_` catch-all,
// which hides the next variant anyone adds.
type Inner = | A | B
type Outer = | Wrap(i: Inner) | Plain

fn describe(o: Outer) -> str {
    match o {
        Wrap(A) -> "wrapped a"
        Plain -> "plain"
    }
}

fn main() {
    print("{describe(Wrap(B))}")
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

## `T-OK-ERR-RETURN.gr`

```grasm
//! tag: T-OK-ERR-RETURN
//! expect: output
//! 1 -1
//! 1 -1
// `Ok(v)` and `Err(e)` in a fallible function's own return position.
//
// They worked everywhere a fallible value could be *stored* and nowhere it
// was *produced*: the body's type was checked against the ok half alone, so
// returning the whole `T ! E` was an error — and the two runtimes then wrapped
// it a second time, giving `Ok(Ok(1))` in one and a raw pointer in the other.
type E = { m: str }
type Cell = i64 ! E

fn tail(b: bool) -> i64 ! E {
    if b {
        Ok(1)
    } else {
        Err(E{ m: "no" })
    }
}

/// The same through an alias, which is where it was first noticed.
fn held(b: bool) -> Cell {
    if b {
        Ok(1)
    } else {
        Err(E{ m: "no" })
    }
}

fn main() {
    print("{tail(true) ?? 0 - 1} {tail(false) ?? 0 - 1}")
    print("{held(true) ?? 0 - 1} {held(false) ?? 0 - 1}")
}
```

## `T-OK-ERR.gr`

```grasm
//! tag: T-OK-ERR
//! expect: output
//! a = 1
//! b ! bad ref
//! 4
//! ok 7 / err gone
// `Ok(v)` and `Err(e)` build a fallible value.
//
// They were matchable and not constructible, so a program storing per-item
// results — which is what `T ! E` is for — needed a trampoline function whose
// entire body was `v`, and another whose body was `fail(e)`, because `fail` is
// legal only inside a fallible function.
type E = { c: str }
type V = | N(n: i64) | S(s: str)

fn describe(r: i64 ! E) -> str {
    match r {
        Ok(v) -> "ok {v}"
        Err(e) -> "err {e.c}"
    }
}

fn main() {
    var cells: Map[str, V ! E] = {}
    cells.set("a", Ok(V.N(1)))
    cells.set("b", Err(E{ c: "bad ref" }))
    for k in cells.keys() {
        match cells.get(k) ?? Err(E{ c: "missing" }) {
            Ok(V.N(n)) -> print("{k} = {n}")
            Ok(V.S(s)) -> print("{k} = {s}")
            Err(e) -> print("{k} ! {e.c}")
        }
    }
    // In a list, and consumed with `??`.
    let xs: [i64 ! E] = [Ok(1), Err(E{ c: "z" }), Ok(3)]
    var total = 0
    for r in xs {
        total = total + (r ?? 0)
    }
    print("{total}")
    // And passed to something expecting a fallible parameter.
    print("{describe(Ok(7))} / {describe(Err(E{ c: "gone" }))}")
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

## `T-PARAM-INOUT-PLACES.gr`

```grasm
//! tag: T-PARAM-INOUT-PLACES
//! expect: output
//! 3 [1, 2]
//! 1 [2]
//! 8 [9]
//! [0, 1]
// Mutating a list held in a struct field, and taking from it.
//
// `vm.stack.pop()` is the core operation of anything keeping a stack in a
// struct, and the compiled backend refused it — so a VM had to use `slice`
// and turn an O(1) pop into O(depth).
type VM = { stack: [i64], mem: [i64] }

fn main() {
    var vm = VM{ stack: [1, 2, 3], mem: [1, 0] }
    print("{vm.stack.pop() ?? 0} {vm.stack}")
    print("{vm.stack.remove(0)} {vm.stack}")

    // Through an index as well as a field.
    var rows = [VM{ stack: [9, 8], mem: [] }]
    print("{rows[0].stack.pop() ?? 0} {rows[0].stack}")

    // And the mutating methods that were already reachable stay so.
    vm.mem.sort()
    print("{vm.mem}")
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

## `T-RECURSIVE.gr`

```grasm
//! tag: T-RECURSIVE
//! expect: output
//! 6
//! 3
// A type referring to itself, which is the first thing a parser needs and
// which the type section did not mention. Through a list, through a map, and
// through a plain field.
type Tree =
    | Leaf(n: i64)
    | Node(kids: [Tree])

type Doc =
    | Text(s: str)
    | Obj(fields: Map[str, Doc])

fn total(t: Tree) -> i64 {
    match t {
        Leaf(n) -> n
        Node(kids) -> kids.fold(0, |acc, k| acc + total(k))
    }
}

fn depth(d: Doc) -> i64 {
    match d {
        Text(_s) -> 1
        Obj(fields) -> 1 + fields.values().fold(0, |acc, v| acc.max(depth(v)))
    }
}

fn main() {
    print("{total(Node([Leaf(1), Node([Leaf(2), Leaf(3)])]))}")
    var inner = map()
    inner.set("a", Text("x"))
    var mid = map()
    mid.set("b", Obj(inner))
    print("{depth(Obj(mid))}")
}
```

## `T-REF-CAPTURE.gr`

```grasm
//! tag: T-REF-CAPTURE
//! expect: output
//! [started, finished] 2
// A captured `ref` may be written through, and that is what lets an effect
// handler accumulate.
//
// [T-CAPTURE-WRITE] refuses a closure that writes to a captured binding,
// because a closure captures *values* and the write would land in a copy
// nobody can see. A reference copies as a slot id, not as a value, so a write
// through one is visible to everyone holding it — the rule does not apply, and
// applying it anyway left a handler unable to accumulate anything at all. The
// obvious spelling was refused and so was the reference that answers it, which
// is what made `inout` on an effect operation look like the only option left
// ([E-EFFECT-NO-INOUT]) — and that silently discarded the write.
type Sink = { lines: [str] }

effect Audit {
    fn record(entry: str)
}

fn work() / {Audit} {
    Audit.record("started")
    Audit.record("finished")
}

fn main() {
    let sink = ref(Sink{ lines: [] })
    with Audit handled_by AuditHandler{ record: |e| sink.lines.push(e) } {
        work()
    }
    print("{sink.lines} {sink.lines.len()}")
}
```

## `T-REF-EQ.gr`

```grasm
//! tag: T-REF-EQ
//! expect: output
//! true true false true
// Two references are equal when they name the same live slot.
//
// The interpreter had no case for it, so a `ref` was not equal to *itself* —
// and the failure was contagious, because equality recurses: any struct, list,
// tuple or `Option` holding one was never equal to itself either. `contains`,
// `assert_eq` and every dedup silently misbehaved, against [E-EQ-STRUCTURAL],
// and the compiled backend disagreed because it compares the packed words and
// got this right by accident. A cycle-safe graph walk keyed on `seen.contains`
// looped forever under `grasm run` and terminated compiled — the same program.
type Node = { v: i64 }
type Edge = { to: ref Node, w: i64 }

fn main() {
    let r = ref(Node{ v: 1 })
    let e1 = Edge{ to: r, w: 5 }
    let e2 = e1
    var seen: [Edge] = [e1]
    print("{e1 == e1} {e1 == e2} {e1 != e1} {seen.contains(e1)}")
}
```

## `T-REF-GENERATIONAL.gr`

```grasm
//! tag: T-REF-GENERATIONAL
//! expect: output
//! abcab
//! true 7
//! false true
// Tier 2 — `ref T`, the safe graph escape hatch.
//
// Tier 0's references are second-class: they can be passed but not stored,
// which is what removes lifetime annotations from safe code and also makes a
// cyclic or shared graph inexpressible. A `ref T` is a slot in a shared heap
// plus the generation that slot held when the reference was taken. It copies
// freely, so it can live in a struct or a list — that is the point — and every
// read checks the generation, so a reference to a released slot is a
// deterministic panic and never a read of whatever occupies the slot now.
type Node = { name: str, next: i64, v: i64 }

fn main() {
    // A cycle. Each node names the next by slot; the last points back.
    var slots: [ref Node] = []
    slots.push(ref(Node{ name: "a", next: 1, v: 7 }))
    slots.push(ref(Node{ name: "b", next: 2, v: 8 }))
    slots.push(ref(Node{ name: "c", next: 0, v: 9 }))
    var at = 0
    var walk = ""
    for step in 0..5 {
        walk = walk + slots[at].name
        at = slots[at].next
    }
    print("{walk}")

    let first = slots[0]
    print("{alive(first)} {first.v}")
    // Releasing a slot leaves every reference into its previous life stale,
    // and says so rather than reading the slot's next occupant.
    release(first)
    print("{alive(first)} {alive(slots[1])}")
}
```

## `T-REF-MATCH.gr`

```grasm
//! tag: T-REF-MATCH
//! expect: output
//! 3 6
// A reference to a sum type is matched through.
//
// Without this a `ref` to a sum could never be opened, and the two diagnostics
// formed a closed loop: `r.v` said "sum types are accessed via `match`", and
// the match said "pattern `Num(..)` cannot match a `ref Expr`". That excluded
// exactly the recursive shapes an AST or a graph node is made of — which is
// the case this tier exists for.
type Expr = | Num(v: i64) | Add(l: ref Expr, r: ref Expr)

fn eval(e: ref Expr) -> i64 {
    match e {
        Num(v) -> v
        Add(l, r) -> eval(l) + eval(r)
    }
}

fn main() {
    let one = ref(Num(1))
    let two = ref(Num(2))
    let sum = ref(Add(one, two))
    // And nested one level deeper, so the recursion is real.
    print("{eval(sum)} {eval(ref(Add(sum, ref(Num(3)))))}")
}
```

## `T-REF-STALE.gr`

```grasm
//! tag: T-REF-STALE
//! expect: panic
//! stale reference: the value it pointed to was released
// Reading through a released reference is a deterministic panic.
//
// This is the property that makes Tier 2 safe rather than merely convenient:
// the slot may already have been reused, and without the generation check the
// read would return whatever now lives there — a use-after-free with no
// symptom. The generation moves on release, so a reference taken before it can
// never match again.
type Node = { v: i64 }

fn main() {
    let r = ref(Node{ v: 7 })
    print("{r.v}")
    release(r)
    print("{r.v}")
}
```

## `T-REF-WRITE.gr`

```grasm
//! tag: T-REF-WRITE
//! expect: output
//! ababa
//! 42 [7] 43
// Writing through a reference, which is what makes a graph a graph.
//
// This typechecked and panicked at run time, so a `ref` graph was write-once:
// closing a cycle needs one write after both nodes exist, and there was no
// way to make it. An evaluation building a real object graph had to keep every
// mutable edge in a `Map[str, [ref Cell]]` keyed by name — the arena-of-indices
// pattern this tier exists to replace. The spec's own [T-REF-GENERATIONAL]
// program built its "cycle" out of `next: i64` indices for the same reason,
// which is how the gap stayed invisible.
//
// The binding's own mutability is not the question: writing *through* a
// reference does not modify the reference. `let` is right here.
type Node = { name: str, out: [ref Node] }
type Counter = { n: i64, hits: [i64] }

fn main() {
    let a = ref(Node{ name: "a", out: [] })
    let b = ref(Node{ name: "b", out: [a] })
    // The write that closes the cycle.
    a.out.push(b)
    var at = a
    var walk = ""
    for step in 0..5 {
        walk = walk + at.name
        at = at.out[0]
    }
    print("{walk}")

    // Assignment, a mutating method, and a compound assignment, all through
    // an immutably-bound reference.
    let c = ref(Counter{ n: 1, hits: [] })
    c.n = 42
    let first = c.n
    c.hits.push(7)
    c.n += 1
    print("{first} {c.hits} {c.n}")
}
```

## `T-REGION-ESCAPE.gr`

```grasm
//! tag: T-REGION-ESCAPE
//! expect: error
//! cannot mutate `out` from inside a region
// Tier 1 — `region { ... }`. Nothing allocated inside outlives the brace.
//
// The rule is what a region *is*, and it is what would make bulk deallocation
// sound: a scope whose allocations provably have no reader afterwards can be
// freed in one sweep. Writing to a binding declared outside is how a value
// would get out — and by a *mutating method*, not only by assignment, which is
// the case a rule about assignment alone would miss.
fn main() {
    var out: [str] = []
    region {
        var scratch: [str] = []
        scratch.push("fine — declared inside")
        out.push("not fine")
    }
    print("{out.len()}")
}
```

## `T-REGION-FOREIGN.gr`

```grasm
//! tag: T-REGION-FOREIGN
//! expect: error
//! cannot perform effect `Audit.record` inside a region
// Whose code runs inside the region, not just what the region's own text says.
//
// The handler is written outside and captures a reference. Calling the effect
// from inside the region runs that outside body while the region is open, and
// what it is given is allocated inside — so the region's text contains no
// write to anything outside, and the value escapes anyway.
type Sink = { lines: [str] }

effect Audit {
    fn record(e: str)
}

fn main() {
    let sink = ref(Sink{ lines: [] })
    with Audit handled_by AuditHandler{ record: |e| sink.lines.push(e) } {
        region {
            let made = "inside"
            Audit.record(made)
        }
    }
    print("{sink.lines.len()}")
}
```

## `T-REGION-INOUT.gr`

```grasm
//! tag: T-REGION-INOUT
//! expect: error
//! cannot pass `inout` `kept` from inside a region
// The escape rule covers `inout`, because `inout` is a write.
//
// The direct spelling `kept.push(made)` was refused and this was not — so the
// same write, moved one call away, defeated assignment, mutating methods,
// deep places, `map.set`, writes through a `ref`, and `spawn`. Two independent
// evaluations found it within an hour of the rule shipping, which is what a
// rule stated in terms of syntax rather than of effect gets you.
fn stash(inout sink: [[str]], v: [str]) {
    sink.push(v)
}

fn main() {
    var kept: [[str]] = []
    region {
        let made = ["allocated inside"]
        stash(kept, made)
    }
    print("{kept.len()}")
}
```

## `T-REGION-REF.gr`

```grasm
//! tag: T-REGION-REF
//! expect: error
//! a `ref` cannot be used inside a region
// A reference is a way out of a region that names nothing outside it.
//
// `stash` is handed a reference the region never mentions by an outside name,
// and a plain parameter, not `inout` — so neither the escape rule nor the
// `inout` rule sees it. The value pushed was allocated inside, and reading it
// after the brace is the one thing a region promises cannot happen.
type Sink = { kids: [str] }

fn stash(s: ref Sink, n: str) {
    s.kids.push(n)
}

fn main() {
    let sink = ref(Sink{ kids: [] })
    region {
        let n = "born inside"
        stash(sink, n)
    }
    print("{sink.kids[0]}")
}
```

## `T-REGION.gr`

```grasm
//! tag: T-REGION
//! expect: output
//! 3 0
//! 9
// What a region is for: scratch work whose result is a number.
//
// Everything the block allocates is provably unreachable after the brace, so
// its peak memory is the block. A scalar may come out, because a scalar is not
// an allocation; a list may not.
fn main() {
    var kept: [str] = []
    let words = region {
        var parts: [str] = []
        parts.push("alpha")
        parts.push("beta")
        parts.push("gamma")
        parts.len()
    }
    print("{words} {kept.len()}")
    // Regions nest, and a region may be an ordinary statement.
    var total = 0
    for i in 0..3 {
        let n = region {
            var scratch: [str] = []
            scratch.push("x{i}")
            region {
                let inner = ["a", "b"]
                inner.len()
            } + scratch.len()
        }
        total = total + n
    }
    print("{total}")
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

## `T-TYPE-ALIAS.gr`

```grasm
//! tag: T-TYPE-ALIAS
//! expect: output
//! <a> 6
//! 8 -1
//! {k: 1}
//! HI
// `type Name = str` — a name for a type, not a new type.
//
// The compiled backend refused every one of these, so a program that named its
// types could not be built at all: `type Name = str`, `type Ids = [i64]`, a
// function type, a map, and a fallible one.
type E = { c: str }
type Name = str
type Ids = [i64]
type Res = i64 ! E
type Counts = Map[Name, i64]
type Stage = fn(str) -> str

fn label(n: Name) -> Name {
    "<{n}>"
}

fn total(xs: Ids) -> i64 {
    xs.sum()
}

/// An alias whose target carries the `! E`: the function is fallible even
/// though its declaration does not say so directly.
fn risky(n: i64) -> Res {
    if n < 0 {
        fail(E{ c: "neg" })
    } else {
        n * 2
    }
}

fn up(s: str) -> str {
    s.to_upper()
}

fn run(f: Stage, v: str) -> str {
    f(v)
}

fn main() {
    print("{label("a")} {total([1, 2, 3])}")
    print("{risky(4) ?? 0 - 1} {risky(0 - 1) ?? 0 - 1}")
    var c: Counts = {}
    c.set("k", 1)
    print("{c}")
    print("{run(up, "hi")}")
}
```

## `T-TYPE-NO-PUB.gr`

```grasm
//! tag: T-TYPE-NO-PUB
//! expect: error
//! `pub` means nothing on type `Point`
// `pub` on a type is not a permission, so it is not accepted.
//
// Types share one namespace across the whole program ([T-MODULE-NS]), so a
// type declared without `pub` is already reachable from every module — which
// the grammar offered anyway, `grasm api` printed, and nothing enforced. That
// is the same defect [T-CONST-PUB] records for `pub let` and
// [G-STRUCT-NONEMPTY] for a zero-field struct: syntax that parses, checks, and
// does nothing, while reading to a person as a permission being granted.
pub type Point = { x: i64, y: i64 }

fn main() {
    print("{Point{ x: 1, y: 2 }.x}")
}
```

## `T-UNSAFE-FFI.gr`

```grasm
//! tag: T-UNSAFE-FFI
//! expect: error
//! can only be used inside `unsafe`
// Tier 3 is asked for, not fallen into.
//
// Calling C leaves everything the safe tiers guarantee, so it is spelled at
// the use — `unsafe { ... }` — and again in the manifest, where a consumer can
// see it. `T-EXTERN.gr` and `T-EXTERN-VALUE.gr` show the accepted form; this
// one pins the refusal, including that it covers *naming* a C function and not
// only calling it.
extern fn sqrt(x: f64) -> f64

fn main() {
    print("{sqrt(16.0)}")
}
```

## `T-UNSAFE-UFCS.gr`

```grasm
//! tag: T-UNSAFE-UFCS
//! expect: error
//! can only be used inside `unsafe`
// `x.f()` is the same call as `f(x)`, Tier 3 included.
//
// [L-UFCS] says the two spellings are one call, and only one of them was
// gated. `n.close()` reached libc with no `unsafe`, no `io` in the row, and no
// capability — a package declaring `capabilities = []` called a C function and
// `grasm deps` reported `performs []`. The compiled binary really made the
// call; the interpreter said `no method`, so the two engines disagreed about
// it as well.
extern fn labs(n: i64) -> i64

fn quiet(n: i64) -> i64 / {} {
    n.labs()
}

fn main() {
    print("{quiet(0 - 5)}")
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
