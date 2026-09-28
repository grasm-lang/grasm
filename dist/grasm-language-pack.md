# Grasm — complete language definition

Everything needed to write correct Grasm, in one file. No prior exposure
required: the normative rules, then a guided tour.

Each `[TAG]` below is a normative clause. Every clause has a corresponding
executable program in one of the `grasm-conformance-*.md` files, and `cargo test -p
grasm_spec` fails if a clause has no program, if a program names a clause that
does not exist, or if any program behaves differently from what it declares. So
the rules and the examples cannot disagree.


<!-- ===== spec/grammar.md ===== -->

# Lexical Structure and Grammar

Notation: `|` alternation, `?` optional, `*` zero or more, `+` one or more,
`( )` grouping, `"x"` literal text. Nonterminals are `lower_snake`.

## 1. Source text

**[L-ENCODING]** A source file is a sequence of UTF-8 encoded Unicode
characters. Identifiers and keywords are restricted to ASCII; string literals
and comments may contain any Unicode character. A byte order mark at the very
start of the file is ignored: it is legal UTF-8, editors write one without
being asked, and lexed as a character it made the first declaration
"unexpected character `\u{feff}`" — a message that renders as nothing and
names nothing.

**[L-NEWLINE]** A newline (U+000A, optionally preceded by U+000D) terminates a
statement. Consecutive newlines are equivalent to one. Inside `(` … `)` and
`[` … `]` newlines are insignificant; inside `{` … `}` they are significant
again.

**[L-LEADING-DOT]** A newline immediately before a `.` or a `?.` is **not** a
statement terminator, so a method chain may be written down the left margin:

```grasm
let cleaned = raw
    .trim()
    .to_lower()
    .replace(",", ";")
```

This is safe for one reason, and only that reason: **no statement in the
language may begin with a `.` or a `?.`**, so a newline in front of one cannot
have been separating two statements. `..` and `..=` are distinct tokens, so a
range still cannot straddle a line, and a leading `+` is still an error — the
existing rule is that a line *ending* in an operator continues, and that rule is
unchanged.

Every language that terminates statements at the newline has had to answer this,
because a chain of five calls on one line is unreadable and a chain that cannot
be broken is unwritable. Go requires the `.` to end the line instead. Putting
the allowance on the *leading* token is the same rule seen from the other side,
and it is the side where the chain lines up under itself.

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

**[L-INT]** Integer literals are decimal (`42`), hex (`0xFF`) or binary
(`0b1010`); underscores are ignored. Decimal is a 64-bit signed magnitude,
hex and binary a bit pattern — all 64 bits, so `0xffffffffffffffff` is -1.

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
`\}`, and `\u{...}` — one to six hexadecimal digits naming a code point. Any
other escape is a lexical error, as is a `\u{...}` naming a surrogate or a
value at or above U+110000: those are not characters, and a string holds
characters ([L-ENCODING]).

`\u{...}` is how a character with no other spelling is written. Without it a
control character could not be put in a literal at all — a group separator was
written `"{1.to_char()}"`, an interpolation whose only purpose was to reach a
character the source could not say.

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
`->` is one token, never `-` followed by `>`. A number directly after `.` is
the exception, because there it is a tuple component: `t.1.0` is two of them,
not `t` and the float `1.0`. Without that, a component of a nested tuple had
no unparenthesised spelling and the error talked about floats.

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

**[G-PLACE]** A `place` — the left side of an assignment, and the receiver of
a mutating method — is a name followed by field reads and index reads. A
*tuple component* is not one: `identifier` cannot start with a digit, so `.0`
never parsed as part of a place. The checker accepted `t.0 = 5` anyway and the
program then panicked with `cannot access field \`0\`` — a message about
reading, for a program that wrote — while `t.0.push(x)` mutated the tuple
under `grasm run` and was refused by the compiled backend. A tuple is a fixed
group of values; a struct is what has parts worth writing to by name. `inout`
is a write too, and it was the spelling that outlived the others: `bump(t.0)`
typechecked and then panicked with `cannot access field \`0\`` — after the
snapshot had already emptied the component.

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

**[G-PAT-BIND]** A bare identifier binds unless it names a variant of the
scrutinee's type. A *nullary* one tests for that variant; one that carries
fields is refused, naming them — `Circle` for `| Circle(r: i64)` is a variant
test somebody wrote without its payload far more often than it is a binding,
and reading it as a binding would silently shadow the variant it looks like.

*The scrutinee's* type, not any type. `None` is a nullary variant of `Option`,
and an `i64` is not an `Option`, so `match n { None -> … }` over one binds the
name. The compiled backend asked whether the name was a nullary variant of
anything the program declared, so it emitted a variant test against a scalar:
the arm's body then read a name nothing had bound, and the binary died with
SIGSEGV and no output — or with `panic: assertion failed` — while the
interpreter printed the answer. Nothing in the program has to declare the
colliding type: `None` is in the prelude, so the shape is one variable name
away in any program at all.

## Wildcard bindings

**[G-WILDCARD-BINDING]** `_` in binding position means "do not name this". It is
accepted wherever a name is bound — `let _ = e`, `let (_, b) = e`, `for _ in e`,
`for (_, v) in e` — and not only in a `match` pattern.

Each `_` is a distinct binding, so `let (_, _) = pair` is legal and binds
nothing twice. None can be read back: there is no expression that names one.

`_` is **not** a way to discard a failure. [T-FALLIBLE-UNHANDLED] applies to a
wildcard binding exactly as it does to a named one, so `let _ = risky()` is
still refused and the annotation `let _: T ! E = risky()` is still what holds
the failure as a value. A binding being unnamed says nothing about whether its
error was considered.

## 11. Nesting limit

**[G-NEST-LIMIT]** The depth of the syntax tree is capped. Exceeding the cap
is a diagnostic, never a crash.

The cap is on tree depth, not on how many brackets were typed, because those
are not the same thing: `1 + 1 + … + 1` and `x.max(1).max(1)…` open no
brackets and recurse nowhere in the parser, and each builds a spine as deep as
it is long. Every pass after the parser walks that spine by recursion, so
8000 terms — well inside any bracket-counting limit — aborted the whole
toolchain with `fatal runtime error: stack overflow` and nothing on stderr to
say which program did it. A folded operator, a method call, a field access,
an index, a pattern and a type each count as one level, exactly as a bracket
does — and so does an interpolation, which is parsed as its own expression and
so restarted the count from zero every time it nested.

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
`match`'s arms, and the expression a function body ends with.

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

### Contracts

**[T-CONTRACT-REQUIRES]** `requires c`, between a function's signature and its
body, is checked on every call before the body runs; if `c` is false the call
panics, quoting `c` and naming the function. A function may have several.

**[T-CONTRACT-ENSURES]** `ensures |r| c` is checked on the value the body
produces, however it leaves; if `c` is false the call panics the same way. A
fallible function's `ensures` sees the ok value, and a failure is not checked.

**[T-CONTRACT-PURE]** A contract is a `bool` that performs no effects.

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
— and may use constants declared before it. The functions in such a table must
themselves perform nothing, since naming one is the permission to call it
([T-FN-VALUE-ROW]) and that charge lands on the constant: a table of `io`
functions has no spelling as a constant, and writing the row on the constant's
type does not change that. Build one in a function instead. The initializer
*is* what the name means, so an effect in it would happen once per mention —
which would make how often something happens a property of how many times a
name appears. That is why the restriction exists rather than being a
limitation to work around: a value that has to be computed once, with effects,
is a function.

Because the initializer is effect-free, when it runs is unobservable, and an
engine may build the value once and hand out copies — which both of them do.
This is a promise about cost, not only a permission: rebuilding per mention
made every *read* of an n-element constant O(n), so a loop of k reads over a
4 000-entry dispatch table took 632 ms where the same loop over a local copy
took 1 ms. A constant is the shape this clause recommends for a table read in
a loop, and reading one is O(1).

**[E-CONST-REF]** A constant's value may not take a reference. A reference
names a slot with a lifetime, and a constant has no place to put one: built
per mention it allocates a different slot each time — the same constant
printed as two different references on one line, and a loop reading it
allocated one per iteration — and built once it is a slot any mention can
`release`, leaving every other mention holding a stale reference. This follows from the clause above and was worth saying
out loud, since it silently destroys the one thing a reference is for.

However it got one. The rule looked for a literal `ref(...)` in the
initializer, so `let ROOT = mk()` where `mk` returns one was accepted — and so
was a struct or a list holding one. It is the constant's *type* that decides
now, so every carrier is covered.

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

`??` groups to the **right**, so `a ?? b ?? c` is `a ?? (b ?? c)` and each
default may itself be something that might be absent. Grouping left, the
middle operand had to be a plain value, so `first() ?? second() ?? fallback`
did not compile at all — for an operator whose whole job is "or this default",
chaining defaults is the obvious thing to write. The layered case, an `Option`
of a fallible value, says what it means with brackets.

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
empty stderr. "Can reach itself" includes reaching itself through a
module-level constant: evaluating `let A = f()` runs `f`, and a function that
reads `A` re-enters it, so the constant is a node in that call graph like any
body. Left out of it, `let A = f()` beside `fn f() -> i64 { A }` was the one
cycle the check could not see. The bound above the guaranteed minimum is
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

A bundle's frames name files by their *basename* — `app.gr:4:5`, not the
directory the build happened in, which names nothing on the machine running
the binary. Every frame is spelled that way, the entry file included: it alone
read `<bundled>` while its imports read `helper.gr`, so one trace named the
same kind of thing two ways and one of them was not a file.

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

Building a *string* with `+` in a loop costs the same way, for the same
reason: strings are immutable, so each `s = s + x` copies everything so far.
400 000 appends take about 2.4 seconds. There is no string builder; collect the
pieces in a list and `join` them, which is linear.

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
otherwise it is a statement of type `()`, and using its value is an error.

A branch that must deliver a value has to *end in an expression*. The expected
type is pushed into a tail expression and there is nothing to push it into
otherwise, so a branch ending in a `with`, `for` or `while` statement sailed
past the demand entirely: `if c { 7 } else { with D handled_by h { n } }` in an
`-> i64` function checked clean, `grasm run` printed `()` where the integer
belonged and a compiled build printed `0` — while the same `with` as the
*whole* body was refused. A branch that leaves — `return`, `break`, `continue`
— delivers its value by not coming back, and is not this case.

**[E-IF-FALLIBLE-MERGE]** When the branches are `Err(e)` and `Ok(v)`, the
`if` is worth `T ! E` with `T` from the `Ok` side. `Err(e)` says nothing about
the ok half, so a branch that is only `Err` cannot decide the type by itself —
taking it anyway made `xs.map(|x| if p { Err(e) } else { Ok(x) })` produce a
list whose elements were later read as the wrong thing.

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

That covers arguments no allocation can satisfy. A length or width computed
from data can be any `i64`, and reaching Rust's own slice or allocator
messages — with an absolute path from the machine that built the compiler, and
exit 1 or 134 rather than 101 — is neither a panic nor a diagnostic. `substr`
and `slice` clamp, as their clauses say; `repeat` and `pad` answer with a
sentence.

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

A handler's operations are closures, so they capture where the handler is
*written*, not where it is installed. The compiled backend filled the
environment at the `with`, so a binding changed between the two was read at
its new value: `var n = 1`, build the handler, `n = 100`, install it — the
interpreter answered `1` and the compiled binary answered `100`, with matching
exit codes and nothing on stderr to say the two disagreed.

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

"Anywhere in the program" stops at the package boundary, because a
*capability* is a statement about a package's own source ([T-CAPABILITY]).
Applied across it, a `capabilities = []` library that installs a handler its
consumer passes in failed to build for a `print` written entirely in the
consumer — and the help text told the library's author to widen a manifest
with nothing wrong in it. Worse, when the consumer's handler performed an
effect the *consumer* declared, no manifest satisfied both diagnostics at
once: naming it in the library was refused as an effect the library does not
define. Authority flows from whoever wrote the handler and is charged there;
the consumer already has to declare it, so nothing escapes uncounted.

**[E-HANDLER-OPS]** A handler's operations are reached by performing the
effect, not by reading them off the handler. They are the one function value
whose row is not checked — that exemption is what lets a handler be written —
so `h.info(x)`, `par_map(h.info)` and `let f = h.info` turned the exemption
into a way to launder anything: `io` and `unsafe` crossed a declared empty
row, and an operation carrying a handled effect left its `with` block.

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

**[E-PAR-VALUES]** A parallel operation takes any list, however it was
built. A compiled backend may store a list of
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

The `f64` generator produces NaN, both infinities, both zeros, the subnormal
boundary and the extremes, as well as ordinary values.

**[E-TEST-SHRINK]** A failing property is reported with a *shrunk*
counterexample: the runner repeatedly substitutes smaller inputs and keeps any
that still fail, so the reported values are minimal rather than whatever the
generator happened to produce. Smaller means simpler as well as shorter — a
string shrinks its characters toward `a` once it can no longer lose any, since
`s = "🙂/🙂"` is length-minimal and tells a reader nothing about the property
that rejected it.

Parts that have to agree shrink together. Replacing one at a time is greedy,
and greedy cannot cross a failure the parts share: a property false only when
two components are equal reported `(107, 107)`, because `(0, 107)` and
`(107, 0)` both hold, so every single step landed on a passing value and the
search stopped at whatever the generator produced. The candidate list starts
with every part replaced at once, which reaches `(0, 0)` in one step.

**[E-TEST-SEED]** Generation is driven entirely by a seed. The same seed and
case count reproduce the same run exactly, and a failure reports the seed that
found it.

The default seed is *fixed*, so running `grasm test` twice runs the same cases
twice — a suite that passes does not become a suite that might not. Widening
coverage is `--seed`, deliberately, not repetition.

That makes the opening cases worth spending carefully, so the first ten walk
the boundary lists in order rather than drawing from them: every `i64` in
`0 1 -1 2 -2 10 100 i64::MIN i64::MAX i64::MIN+1`, every `f64` in the list
[E-TEST-PROPERTY] names, both `bool`s, and the empty string. Drawn at random
each was only *likely*: `2` first appeared at case 138 of the default seed,
and the default run is 100 cases, so `assert(n != 2)` passed — a property
false at a value the generator names on purpose, reported as holding.

**[E-BENCH-WARMUP]** A benchmark body runs once before timing starts. A body
that panics is reported as a failure instead of a measurement.

## 11. Program startup

**[E-ENTRY]** Execution begins at `main` in the entry module. A program
without `main` is an error. `main` takes no parameters and produces no value:
both are refused, because nothing calls `main` and nothing reads its value.
Writing `fn main() -> i64` and ending the body with `2` looked like a status
code, and the program exited 0 — the status a program reports is `exit(n)`,
and an error reaching `main` is [E-MAIN-ERR]. A `! E` on `main` is allowed and
means what that clause says. Only the return type was refused for a while, so
`fn main(x: i64)` checked clean and then panicked at run time with
`unknown name \`x\`` — a compiler-internal message arriving after the point
where types are supposed to be settled. Read the command line with [E-ARGS]
instead.

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
| `args()` | `() -> [str]` | the program's own arguments; performs `io` |
| `panic(msg)` | `(str)` | prints `panic: msg`, exits 101 |
| `assert(c)` | `(bool)` | panics when `c` is false |
| `assert_eq(a, b)` | `(T, T)` | panics when `a != b` |
| `fail(e)` | `(E)` | raises the error branch of a `T ! E` |
| `map()` | `() -> Map[K, V]` | a new empty map |
| `now_millis()` | `() -> i64` | milliseconds since the epoch; `io` |
| `monotonic_millis()` | `() -> i64` | since an unnamed moment; only increases |
| `sleep_millis(n)` | `(i64) -> ()` | waits; a replay does not; both `io` |
| `random_bytes(n)` | `(i64) -> bytes ! IoError` | `n` bytes from the operating system; `io` |
| `env(name)` | `(str) -> Option[str]` | environment variable; `io` |
| `exit(code)` | `(i64)` | ends the program with that status — its low eight bits ([L-LIB-EXIT]) |
| `read_file(p)` | `(str) -> str ! IoError` | fallible, not `Option`; `io` ([L-LIB-IO]) |
| `write_file(p, s)` | `(str, str) -> () ! IoError` | fallible; `io` ([L-LIB-IO]) |
| `file_exists(p)` | `(str) -> bool` | `io` |

**[L-LIB-CLOCK]** `now_millis` is the *wall* clock and **moves backwards** — a
correction, a leap second, a resumed machine — so a duration measured with it
goes negative and a timeout fires wrong by that much. `monotonic_millis` only
increases. `sleep_millis` waits; a replay does not.

**[L-LIB-EXIT]** `exit(code)` ends the program with the **low eight bits** of
`code`, as an operating system reports it and as C's `exit` does: `exit(-1)` is
255, `exit(256)` is 0. Clamping to 0..=255 was worse where it matters — it made
`exit(-1)`, the commonest spelling of "fail" in a script, report *success*.
`exit` performs no effect, so a dependency declaring `capabilities = []` can end
the host process: deliberate, since `panic` and an endless loop already do — a
ceiling bounds what a dependency can *reach*, never how long it runs.
Nothing observable outside the process changes, so charging `io` for it would
buy no guarantee while making every program that reports a status ask for the
filesystem.

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
| `ref T` | the slot it names and that slot's generation, never what it points at — the value may already be gone, and reading it to render would be the use-after-free the tier prevents | `ref#0.0` |
| a named function | `<fn module::name>` | `<fn main::dbl>` |
| a closure | `<closure>` — it has no name to show | |
| a handler | a struct with the operation names as fields | `CHandler{now: <closure>}` |
| a range | `start..end`, as written | `1..5` |

Unquoted means a one-element list holding the empty string renders exactly as
an empty list does: `[""]` and `[]` are both `[]`, and `"".split(",")` — one
field, per the table above — looks like no fields at all. That is the price of
a rendering meant to be read rather than parsed, and it is stated because the
alternative is a reader concluding the library is wrong. `len()` tells them
apart.

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

The language's *own* free functions — `print`, `read_file`, `write_file`,
`file_exists`, `input`, `args`, `now_millis`, `env`, `exit`, `panic`, `assert`,
`assert_eq` — are not reached this way. They are not methods of any type, and a
second spelling for each of them buys nothing: `x |> print` already applies a
free function to a value ([G-PIPE]), and it is charged the same row a call is.
`"hi".print()` is refused, and the refusal says so and points at the pipeline —
it used to offer UFCS for `print`, which is the call it had just refused.

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

This is what draining a queue from the front *reads* like. Without it a ready
queue cost `ready = ready.slice(1, ready.len())` a step, which reads like the
workaround it was — but the cost is the same: `remove(0)` shifts the rest down,
so both are O(n) per step and a full drain is quadratic either way. Measured at
80 000 elements, 17.2 s for `remove(0)` against 12.8 s for the slice. If a
queue is what you have, reverse it once and `pop` from the end, which is O(1);
this method is for taking an element out of the middle.

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
| `x.to_bits()`, `n.from_bits()` | `f64 -> i64`, `i64 -> f64` | the IEEE-754 pattern, not the value ([L-LIB-AS-I64]) |

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
printed a `[f64]` as a list of integers, which [L-DISPLAY] exists to prevent.

**[L-LIB-AS-I64]** `as_i64` is total: every `f64` has an answer, so it never
panics. Too large gives `i64`'s maximum, too small its minimum, `NaN` gives `0`
— saturation, and a *clamp* rather than a truncated bit pattern, because the two
disagree and only one stays ordered. `to_bits` asks the other question, not what
a float means but what it **is**: without it nothing can put a float in a binary
format or compress a series, every such technique XORing consecutive patterns.

## 3. `str`

**[L-LIB-STR]** Lengths and offsets are in **characters, not bytes**
([E-STR-CHARS]). `s.byte_len()` is the one exception and is the only way to ask
the other question: `"🙂".len()` is 1 and `"🙂".byte_len()` is 4. Characters are
the right unit for a high-level language and the wrong one the moment a program
writes a string to a socket or a file and has to say how long it is — a
`Content-Length` counts bytes, and computing it from `len()` is wrong for every
string that is not ASCII.

**[L-LIB-TRIM-SIDES]** `trim_start` and `trim_end` remove whitespace from one
end only; `trim` removes it from both. None of the three takes an argument —
whitespace is what they trim. The one-sided pair exists because a line's
leading indentation usually carries meaning and its trailing newline does not,
and `trim` cannot tell you that.

| method | type | notes |
|--------|------|-------|
| `s.len()` | `-> i64` | characters |
| `s.byte_len()` | `-> i64` | bytes, for a length that goes on the wire ([L-LIB-STR]) |
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
| `n.to_base(b)` | `(i64) -> str` | digits `0-9a-z`; a base outside `2..36` panics ([L-LIB-BASE]) |
| `s.parse_int_base(b)` | `(i64) -> Option[i64]` | the other direction; no prefix accepted ([L-LIB-BASE]) |
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

`len`, `code_at` and `substr` all answer in constant time (`substr` in time
proportional to the slice it returns), whatever the text. They
count *characters*, so each used to decode the string from the start:
`while i < s.len()` was quadratic in two ways at once, and 400 000 characters
took 6.7 seconds where the identical loop over a list took 0.21. A string
never changes once built, so its character count is computed once and kept.
When that count equals the byte count the text is ASCII and an index is an
index; when it does not, a character index is built once, the first time one
is asked for by position. Fixing only the ASCII half left the same 6.3 seconds
waiting behind a single two-byte character, and building the index for
`code_at` while leaving `substr` to walk from the start left a `substr(i, 1)`
scan quadratic — 39 seconds over 262 144 characters — next to a `code_at` scan
of the same string costing 0.11.

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
| `xs.sort()` | | mutates; ascending. Ordered means `i64`, `f64`, `str`, and a tuple of those ([L-LIB-TUPLE]) — sorting anything else is a compile error, the same one `<` gives |
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
the *elements* ordered, and a struct is not. The key is `i64`, `f64`, `str`, or
a **tuple** of those, which is what makes
`xs.sort_by(|p| (p.dept, p.age, p.name))` a multi-key sort rather than an error
([L-LIB-TUPLE]). This clause said "must be `i64`, `f64` or `str`" while tuple
keys worked in all three engines — two clauses of one document disagreeing, and
the one that was wrong was the one a reader would find first. The sort is stable: elements with equal keys stay in the order they
were written. It sorts **ascending**, always; descending is
`xs.sort_by(k)` followed by `xs = xs.rev()`. Negating the key works for a
number and has no counterpart for a `str`, so the reversal is the general
answer and the one to reach for. The assignment is the whole point: `sort_by`
mutates and `rev` answers with a new list, so the clause's earlier wording —
`xs.rev()` on its own line — left the list ascending and said nothing.

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
asking whether the element was ordered, so `xs.enumerate().min()` was refused
on a list `sort` orders happily.

That example is worth spelling out, because it does *not* mean "index of the
smallest": components compare left to right, so `enumerate().min()` compares
indices first and always answers `(0, xs[0])`. To find the smallest and where
it was, put the value first — `xs.enumerate().map(|p| (p.1, p.0)).min()`.

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

**[L-LIB-RANGE-OPS]** A range answers `to_list`, `map`, `filter`, `sum` and
`len`, producing a list of the same elements `for i in a..b` walks. There is one
definition of what a range contains and not two: every engine builds that list
and reuses the list method.

A range is also a **value**: `let r = 0..n` binds one, it prints as `0..n`
rather than as its elements, and two compare equal when their bounds and their
inclusiveness match — so `0..3 == 0..3` and `0..3 != 0..=3`. It holds three
numbers and not its elements, so `0..2_000_000` costs the same as `0..2`.

Its type is written **`range`**: `fn window(n: i64) -> range`, `type W = { span:
range }`, `[range]`. That name reached the error messages before the syntax, so
a range could be bound and never declared — a value you can hold and cannot
pass is stuck in one function body. It reaches its methods from any shape that
produces one, not only from a name: `[0..3][0].len()` and `window(2).sum()`.

`len` is answered from the bounds without building anything; the others build
the list first. `for i in a..b` written literally never builds a range at all —
it is compiled into a counted loop — and that is why the loop costs nothing and
holding a range costs one allocation.

The clause exists because of what it replaces. Building a list from a count
needed `var xs: [i64] = []`, a `for`, a `push` and two closing braces — four
lines where `(0..n).map(f)` says the same thing, in every program that builds a
list from a count. [G11] budgets Grasm at or below Python's token count and it
measured 1.41×; this is most of the difference, and adding it moved the figure
to 1.22× without adding a syntactic form — a call is a call, so the kernel's
count in [`core.md`](core.md) does not move.

## Network — the `net` capability

**[L-LIB-NET]** Eight builtins reach the network, and each performs `net`:

| | type | |
|---|---|---|
| `tcp_listen(port)` | `(i64) -> i64 ! IoError` | listens on `127.0.0.1:port`; `0` asks the system to choose |
| `tcp_connect(host, port)` | `(str, i64) -> i64 ! IoError` | dials `host:port`; a name is resolved |
| `tcp_accept(h)` | `(i64) -> i64 ! IoError` | waits for a connection, answering with a handle for it |
| `tcp_read(h)` | `(i64) -> str ! IoError` | what has arrived, as text |
| `tcp_read_bytes(h)` | `(i64) -> bytes ! IoError` | the same read, unconverted |
| `tcp_write(h, text)` | `(i64, str) -> () ! IoError` | writes and flushes |
| `tcp_write_bytes(h, b)` | `(i64, bytes) -> () ! IoError` | writes and flushes |
| `net_close(h)` | `(i64) -> ()` | closes a listener or a connection |
| `net_wait(hs, ms)` | `([i64], i64) -> [i64] ! IoError` | the handles that can be read without waiting |
| `net_port(h)` | `(i64) -> i64 ! IoError` | the local port a handle is bound to |

`tcp_connect` is the half that makes a program a *client*. Without it a Grasm
program could listen and accept and never dial — no fetching a URL, no calling
an API, no talking to a database — which is a gap invisible until somebody tries
to write an ordinary program. Resolution failure and connection failure are the
same `IoFailed` to a caller, because both mean "there is nothing to talk to",
and the message carries the difference.

A handle is an integer and a payload is text, so reaching the network needed no
pointer type and no byte buffer. That is the reason the surface looks like this:
adding either to the language would have been a far larger change than adding
the network, and `extern fn` could already call `socket()` and `listen()` —
what it could not do was pass a `sockaddr` or a buffer.

`tcp_read` performs **one** read rather than looping to end-of-stream, because
an HTTP client holds the connection open after its request and reading to the
end would wait forever. A caller that needs more reads again. Bytes that are not
valid UTF-8 are replaced rather than refused: a socket carries bytes, a `str`
carries text, and a client's mistake should not end a server.

That replacement is right for a protocol made of text and wrong for one that is
not. `multipart/form-data` carries a JPEG, and turning every invalid sequence
into U+FFFD corrupts the file in a way nothing downstream can detect — the
upload arrives, the length looks plausible, and the image is broken.
`tcp_read_bytes` and `tcp_write_bytes` are the same two calls without the
conversion. They needed no new type: `bytes` already existed ([L-LIB-BYTES]),
and the socket simply had no way to speak it.

Both pairs stay, rather than one replacing the other, because an HTTP header is
text and a body may not be: a server reads the request as bytes and decodes the
part it knows is text, and a program serving only text should not have to.

`net_port` is what makes `tcp_listen(0)` usable. Asking the system to choose a
free port is the only way to write a program that runs on a machine where
something else already holds the port it wanted — and with no way to read the
choice back, a program could bind a port it was then unable to name, dial, or
print.

`net_wait` is what lets one thread hold many conversations. Every other call
here blocks: without it a server serves one client at a time, and serving two
would need a thread per connection — which a Grasm program cannot spawn, because
`spawn` does not carry `net`. So the language had a network a program could not
usefully *serve* on, and the missing piece was readiness, not threads.

It answers with the subset of `hs` that will not block, **in the order they were
asked in** rather than whichever order the kernel reports, so a program that
walks the result serves its connections in a fixed order ([L-DETERMINISM]). A
timeout of `0` asks without waiting, a negative timeout waits indefinitely, and
an empty wait set is answered immediately with an empty list rather than an
error — a server that has just closed its last connection asks exactly that on
its next turn. A handle that names nothing is an `IoFailed`, the same as using a
connection after closing it.

A hung-up peer is reported as **ready**. The read that follows returns nothing,
which is how a caller learns the conversation ended; reporting it as not-ready
would leave a dead connection in the wait set forever, and the loop would never
notice it should be dropped.

Readiness needed no new type: the wait set is an ordinary `[i64]` and the
timeout an ordinary `i64`, which is the same reason the rest of this surface
looks the way it does.

**[L-LIB-UDP]** Four more reach the network as **datagrams**, each performing
`net`:

| | type | |
|---|---|---|
| `udp_bind(port)` | `(i64) -> i64 ! IoError` | binds a datagram socket; `0` asks the system to choose |
| `udp_send(h, host, port, b)` | `(i64, str, i64, bytes) -> i64 ! IoError` | sends one datagram, answering with the bytes sent |
| `udp_recv(h)` | `(i64) -> bytes ! IoError` | receives **one** datagram, whole |
| `udp_recv_from(h)` | `(i64) -> (bytes, str, i64) ! IoError` | the same, and where it came from |

UDP is not TCP with the reliability removed but a different shape. A stream
fixes both ends at connect time; a datagram is a *whole message* with **no fixed
peer**, so `udp_send` names a destination every call and `udp_recv_from` answers
the sender's — a receiver that cannot learn who wrote can only answer peers
configured in advance ([L-LIB-PROC] for why a tuple).

Payloads are `bytes`, not `str`: the protocols that use UDP are binary, and
routing a DNS query's bit-level flags through the lossy UTF-8 conversion the
stream operations correctly use for text would corrupt it ([L-LIB-BYTES]).

A read returning half a message, or two joined, would destroy the only thing
UDP guarantees. Nothing past 65535 bytes arrives over IPv4 in one piece, so the
buffer cannot truncate a deliverable datagram.

Datagram handles share the table with stream handles, so `net_wait`, `net_port`
and `net_close` work on all three kinds unchanged. A program that speaks two
protocols multiplexes them in one loop, which is the ordinary case rather than
an advanced one. Using a stream operation on a datagram handle is an `IoFailed`
naming the operation to use instead.

Closing a handle twice is not an error, for the same reason releasing a `ref`
twice is not ([T-REF-GENERATIONAL]) — cleanup order is not something a caller
should have to track. Using a closed handle *is* an error, and an ordinary one:
`tcp_read` on it answers `Err`, never a panic and never a wrong answer.

**`net` is a capability of its own, not part of `io`.** That distinction is the
whole point. [T-CAPABILITY]'s headline example is a package permitted to read a
file and forbidden to open a socket, and until there was a network to gate, that
example could not be shown to anybody:

```
error: package `lib` performs `net` but its manifest only permits `io`
  help: either add the capability to that package's manifest, or stop it from
        performing the effect
```

**[L-LIB-BASE]** `n.to_base(b)` writes an integer in base `b` and
`s.parse_int_base(b)` reads one back. Digits are `0-9a-z`, lower case, and a
negative number gets a leading `-`.

Base ten is not the only base a systems program needs. Every protocol, hash,
colour and permission mask is read and written in hex or binary — the same
programs that needed [L-LIB-BITS] — and without these, printing a packet ID the
way every other tool prints it means hand-writing a digit table.

A base outside `2..36` **panics** rather than answering. There is no right
answer to return and a wrong one would be silently wrong, which is the same
reasoning as `code_at` panicking out of range.

Neither side writes or accepts a prefix. The base is the argument, not something
to be guessed from the text: `"ff".parse_int_base(16)` is `Some(255)` and
`"0xff".parse_int_base(16)` is `None`. That keeps it consistent with `parse_int`,
which refuses `0xFF` ([L-LIB-PARSE]) — the difference is now that a program can
say which base it means instead of being stuck with one.

`to_base(10)` and `to_str` agree, including at `i64::MIN`, which is converted
through its unsigned magnitude because negating it overflows.

## Processes — [L-LIB-PROC]

**[L-LIB-PROC]** `proc_run(program, args)` runs another program to completion and
answers with `(exit code, stdout, stderr)`:

| | type | |
|---|---|---|
| `proc_run(prog, args)` | `(str, [str]) -> (i64, str, str) ! IoError` | performs `proc` |
| `proc_pipeline(stages, input)` | `([(str, [str])], str) -> (i64, str, str) ! IoError` | each stage's output feeds the next |

**`proc` is a capability of its own, and it is strictly larger than `io` or
`net`.** A program that can run `curl` has the network whether or not it
declared `net`; one that can run `rm` has the filesystem. Folding process
execution into `io` would make every capability claim in the language weaker
than it reads — a package's manifest could say `io` and mean everything.

The result is a **tuple, not a new type**. Three values with obvious positions
did not need a name, and inventing one would have put a type in the prelude that
nothing else uses.

**Output is captured, not inherited.** A builtin that wrote to the parent's
stdout would make the three engines disagree about ordering, because the child's
writes and the parent's go through different buffers. A caller that wants the
output shown prints it, which also means it can be examined first.

`args` are passed as a list and never through a shell, so a filename containing
a space or a `;` is one argument and not an injection. A program that genuinely
wants shell semantics asks for them: `proc_run("sh", ["-c", line])`.

`proc_pipeline` exists because a pipeline **cannot be built out of
`proc_run`**. That waits for a program to finish and hands back its output,
and a pipeline needs one child's stdout to *be* another's stdin. Copying the
bytes through Grasm instead would need concurrent reads and writes on pipe
handles — a much larger surface for a smaller result, and the kernel already
does this.

`input` feeds the first stage's stdin, which is the other thing `proc_run`
could not do: a filter with nothing to filter is not a filter. It is written on
a thread, because a large input and a chatty child deadlock if the parent writes
everything before it reads anything. An empty input closes stdin rather than
leaving it open, so a stage reading to end-of-input does not wait for input that
is never coming.

The result is the **last stage's** exit code, stdout and stderr, which is what
every shell means by a pipeline's status. An earlier stage's stderr goes to the
program's own stderr rather than being discarded: a diagnostic nobody sees is
worse than one in the wrong place. If any stage cannot be started, everything
already running is killed and the whole call is an `IoFailed` — a pipeline that
failed halfway leaves nothing behind. An empty pipeline is an error, not a
silent success.

An exit code of **`-1`** means a signal killed the process. No process can exit
with `-1`, so it cannot be confused for a status the program chose. A program
that could not be started at all is an `IoFailed` naming it, which is a
different thing from a program that ran and failed.

## Bit manipulation — [L-LIB-BITS]

**[L-LIB-BITS]** Seven methods on `i64`:

| | | |
|---|---|---|
| `a.bit_and(b)` | `(i64, i64) -> i64` | bitwise and |
| `a.bit_or(b)` | `(i64, i64) -> i64` | bitwise or |
| `a.bit_xor(b)` | `(i64, i64) -> i64` | bitwise exclusive or |
| `a.bit_not()` | `(i64) -> i64` | ones' complement |
| `a.shl(n)` | `(i64, i64) -> i64` | shift left |
| `a.shr(n)` | `(i64, i64) -> i64` | shift right, **sign-preserving** |
| `a.ushr(n)` | `(i64, i64) -> i64` | shift right, **zero-filling** |

**Methods rather than operators**, and the reason is that every symbol is
already taken or ambiguous. `^` is exponentiation. `|` begins a closure and
separates sum-type variants, so `a | b` cannot be told from the start of one.
`&&` is already reported as "written `and`". And words would collide with the
logical operators, which in this language are `and`, `or` and `not` — so
`a and b` would be boolean and `a band b` bitwise, one letter apart. Methods are
what `.abs()` and `.cmp()` already are, and they carry their own precedence.

**The shift count is masked to `0..63`.** `1.shl(64)` is `1`, not `0` and not a
trap, and `256.shr(-1)` is `256.shr(63)`, which is `0`. This is what the
hardware does, and it is stated because the alternatives are worse: Rust's `<<`
panics in a debug build and wraps in a release one, which is the class of
difference-between-builds this project refuses to ship, and Cranelift leaves an
out-of-range count undefined, which would make the two engines disagree.

`shr` preserves the sign and `ushr` does not, which is the only reason both
exist: `(-16).shr(2)` is `-4` and `(-16).ushr(60)` is `15`.

There is no unsigned integer type, so a program working in 32 bits masks with
`0xFFFFFFFF` itself. `spec/conformance/L-LIB-BITS.gr` shows the rotation that
SHA-256 needs, written that way.

## The other stream — [L-LIB-EPRINT]

**[L-LIB-EPRINT]** `eprint(v)` writes to standard error. It is `print` in every
other respect: same single argument, same interpolation, same refusal to take a
fallible value, same `io` effect.

It exists because a program that cannot separate its diagnostics from its output
cannot be used in a pipeline, and being usable in a pipeline is most of what a
command-line tool is for. A benchmark's timings, a compiler's warnings and a
tool's progress are all diagnostics; sending them to stdout puts them in the
middle of the data, which is the one place they cannot be noticed and the one
place they do damage. It is the same reason a panic does not go there
([E-PANIC-EXIT]).

Both engines write the two streams independently and identically. The compiled
build brackets the write with a flag the runtime reads, rather than carrying a
second copy of the formatting code — one code path, so the two streams cannot
format differently.

## Files and directories — [L-LIB-FS]

**[L-LIB-FS]** Two additions to the file surface:

| | type | |
|---|---|---|
| `make_dir(path)` | `(str) -> () ! IoError` | creates the whole path; already existing is success |
| `remove_file(path)` | `(str) -> () ! IoError` | deletes; a missing path is `NotFound` |
| `list_dir(path)` | `(str) -> [str] ! IoError` | entry names, **sorted** |

`make_dir` creates every missing component and treats an existing directory as
success, so a program can prepare its own working directory without first asking
whether one is there.

Without these a file could be created and truncated but never removed, a
directory could not be read at all, and a program could not make one — so a program that made a temporary file
could only empty it, and the next run saw a leftover.

**`list_dir` sorts.** A filesystem's own order differs between machines and
between runs, and principle 6 makes determinism a feature: an unsorted listing
would make any program that printed one untestable, and would break three-engine
agreement on the first run. Both engines sort, and identically.

Names are returned, not paths — the caller already knows the directory it asked
about, and joining is one concatenation.

## Bytes — [L-LIB-BYTES]

**[L-LIB-RANDOM]** `random_bytes(n)` answers `n` bytes from the operating
system's entropy source, and performs `io`.

It is charged `io` because it reaches outside the program and two calls do not
agree — the same reason the clock is. A pure function that answered differently
each time would break every promise an effect row makes.

The primitive is bytes rather than an integer or a float because everything else
is built from it and the conversions are where the mistakes are: a number in a
range wants rejection sampling, not a modulo, and a language that shipped
`random_int(lo, hi)` with a modulo in it would have shipped a bias.

`n` of zero is an empty answer rather than an error. A negative count, or one
past 1048576, is refused — a count is usually something a program computed, and
a mistake there should be a refusal rather than a gigabyte.

A short read is a failure, not a short answer: a caller asking for 32 bytes of
key material and receiving 12 must not proceed.

**[L-BUNDLE-SOURCE]** A whole program may be written as **one source file**: a
*source bundle*, in which each module is preceded by a line reading `//!module
<name>` and the entry module comes first. Nothing is renamed and nothing is
rewritten, so a bundle loads exactly the modules the filesystem would have, and
a single-module program is a bundle of one.

`grasm build <file> --source -o <out>.gr` writes one. Every command that opens a
program opens a bundle, because the detection is in the loader they share.

This exists for embedders that have no filesystem. The wasm engine takes one
source string, and an `import` cannot be resolved by reading a file that is not
there — so without this, a program with a single import could not run in a
browser however small it was, and almost every real program has one.

**[L-LIB-BYTES]** `bytes` is a contiguous run of bytes, and it is the type `str`
deliberately is not.

A `str` is *text*: UTF-8, with anything above 127 re-encoded on the way out —
correct for text, fatal otherwise. Asked for the seven bytes of `mov rax, 1` a
`str` put **nine** on disk, and a language whose only sequence type does that
cannot emit an object file.

| | type | |
|---|---|---|
| `bytes_of(list)` | `([i64]) -> bytes` | every element must be in `0..255` |
| `s.to_bytes()` | `(str) -> bytes` | the UTF-8 encoding of the text |
| `b.to_str()` | `(bytes) -> str` | lossy, as `tcp_read` is |
| `b.len()` | `(bytes) -> i64` | count of bytes |
| `b.is_empty()` | `(bytes) -> bool` | |
| `b.at(i)` | `(bytes, i64) -> i64` | one byte, `0..255`; out of range panics |
| `b.slice(from, end)` | `(bytes, i64, i64) -> bytes` | `end` exclusive, as `list.slice`; out of range panics rather than clamping |
| `b.concat(other)` | `(bytes, bytes) -> bytes` | |
| `b.to_list()` | `(bytes) -> [i64]` | |
| `read_bytes(path)` | `(str) -> bytes ! IoError` | the whole file |
| `write_bytes(path, b)` | `(str, bytes) -> () ! IoError` | replaces it |
| `read_bytes_at(path, off, n)` | `(str, i64, i64) -> bytes ! IoError` | a range; short at the end, empty past it |
| `append_bytes(path, b)` | `(str, bytes) -> () ! IoError` | adds to the end, creating the file |
| `file_size(path)` | `(str) -> i64 ! IoError` | without reading it |
| `sync_file(path)` | `(str) -> () ! IoError` | on the disk before answering |
| `rename_file(a, b)` | `(str, str) -> () ! IoError` | atomic where the platform allows |

**The last five exist because whole-file calls cannot express data**: a table
larger than memory is unreadable, a log rewritten per record is quadratic, and a
crash-safe commit is write-beside, `sync_file`, `rename_file`.

**`bytes` is distinct from `[i64]` on purpose.** A list of integers cannot
promise its elements are bytes, stores eight per element, and cannot be handed
to a buffer — the promise is the type. `at` gives `0..255`; one integer type is
the trade. It **prints as hex**, and `to_str` is lossy for `tcp_read`'s reason.


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

## 3a. Interfaces

A generic function works on any type — which means that inside it, a `T` can
do almost nothing, because nothing is known about it. An `interface` names
what a type must provide, and a bound says a type parameter provides it:

```
interface Show {
    fn show(self) -> str
}

type Point = { x: i64, y: i64 }

impl Show for Point {
    fn show(self) -> str {
        "({self.x}, {self.y})"
    }
}

impl Show for i64 {
    fn show(self) -> str {
        "#{self}"
    }
}

fn all_shown[T: Show](xs: [T]) -> str {
    var out = ""
    for x in xs {
        out += x.show() + " "
    }
    out
}

fn main() {
    print(all_shown([Point{ x: 1, y: 2 }, Point{ x: 3, y: 4 }]))
    print(all_shown([7, 8]))
}
```

`self` is the value the method is called on, and `Self` in an interface means
"the implementing type". The method is chosen by the receiver's type, so
`Point` and `i64` can each have a `show`.

The bound is checked where the function is written, not where it is used:
`x.show()` in a function declared `[T]` rather than `[T: Show]` is an error
even if every caller happens to pass something showable. And a call with a
type that has no `impl` is refused at that call, by name — "`Dog` does not
implement `Show`" — rather than somewhere inside the generic function.

Two limits for now, both refused with a message: an `impl` is for a declared
non-generic type or for `i64`, `f64`, `bool` or `str`; and an interface method
performs no effects, so an `impl` method cannot print. The second is what lets
a generic function's own effect row stay honest without knowing which `impl`
it will reach.

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

## 4a. Contracts

A function can state what it expects and what it promises, and every call
checks both:

```
fn isqrt(n: i64) -> i64
    requires n >= 0
    ensures |r| r * r <= n and (r + 1) * (r + 1) > n
{
    var r = 0
    while (r + 1) * (r + 1) <= n {
        r += 1
    }
    r
}

fn main() {
    print("{isqrt(17)} {isqrt(1000000)}")
}
```

`requires` is checked before the body runs; `ensures |r|` is checked on the
result, however the body leaves. A broken one panics and quotes itself —
`requires `n >= 0` failed in `isqrt`` — so a wrong answer stops at the function
that produced it instead of three calls later. A contract is a plain `bool`
that performs nothing: it states a fact, it does not do work.

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

Which is also why only the compiled backend actually uses them: `grasm run`
and `grasm build -o` produce a bundled *interpreter*, and there a `par_map` is
a `map`. The answer is identical — that is the whole point of requiring purity
— but the throughput is not, so measure with `grasm build --native`.

A short list is not automatically a sequential one. The runtime computes the
first element on the calling thread, times it, and splits the rest when what
remains is worth more than the spawns — so six elements of real work get the
cores, and a thousand cheap ones are not split for the sake of it. Being wrong
costs one element's time and never changes the answer.

What threads cannot help with is worth knowing before reaching for them: if
the body mostly *moves* memory rather than computing — copying a buffer,
building a list — eight workers share one memory bus and gain nothing, and if
the body reads a large structure shared with the other workers, the reference
counts on it are atomic and the contention can cost more than the work.
`docs/apps-phases.md` has both measured, from an audio mixdown where the
difference was 1.2 s against 6.3 s.

A panicking task aborts the whole scope; tasks spawned after it do not run.
A `return` inside a task body returns from the function the `parallel` block
is in, and the rest of the scope — the tasks not yet run, and the statements
after the block — is abandoned. That is a way out, not a way to end one task.

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
It has to name a file *inside* the package: the lockfile's checksum, the
capability ceiling and `grasm publish` all cover the package's own directory,
so an `entry` reaching out of it with `..` — or an absolute path — is refused
rather than silently leaving all three describing a tree the program does not
use. It has to end in `.gr` for the same reason: those three collect the
package's Grasm sources, and an entry named anything else was executed but
recorded nowhere — `--locked` reported "up to date" after it changed, and
`grasm publish` shipped a package that could not build into a slot that cannot
be republished.
Everything a package exports lives behind that one file, so a dependency's
other modules are reached as `import mathlib.helpers`.

"The package's sources" is **one** answer to all three questions, which it was
not: the checksum skipped `target/` and every dot-directory, both of which are
legal import path segments; the `.gr` extension was matched case-sensitively,
so `HELPER.GR` — what `import helper` resolves to on a case-insensitive
filesystem — was invisible to the checksum, to `publish` and to the ceiling
alike; and `publish` walked from the entry's directory while the checksum
walked from the package's, so one released version had two checksums. A
directory symlink is followed once: a package holding two self-links took
`deps` and `publish` from "34 copies into an immutable slot" to "never
returns".

A package with a `path` dependency cannot be published at all. Only the
package's own directory travels, so the release named a dependency that
resolved outside the version directory — every consumer failed on a version
that cannot be republished.

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
leaving it out means "do not check me". A key `[package]` does not recognise is
refused rather than ignored, because that difference is exactly what a typo
would spend: `capabilitys = []` used to leave the package unrestricted, and the
only trace was a column in `grasm deps`. Because effects are inferred from source, a dependency that
quietly starts reading the filesystem exceeds its declared ceiling and fails
to build **in every consumer** — a compile error, not a runtime surprise, and
not something a reviewer has to happen to notice.

A capability is what a package needs from **outside** itself, so what counts is
what *escapes*: the effects `main` performs, the effects left in a `pub`
function's row for a consumer to handle, and the effects its `test` and `bench`
bodies and its **doc examples** perform. That last group surprises people — a
test is not part of the interface, and an example is documentation — but
`grasm test` runs the tests *and the doc examples* of every imported module,
so a dependency's test body and its `///` fenced blocks really do execute in
your process. The consequence is worth knowing: a `capabilities = []` package
cannot `print` from a test or from a doc example, which rules out the usual
`/// print("{f(2)}")` — write the example as `assert(f(2) == 4)` instead, which
performs nothing and checks more. Every module in the package counts,
not only the ones its entry file imports — a consumer reaches `pub fn f` with
`import dep.module` whether or not the dependency's own entry mentions that
module, so a `capabilities = []` package could otherwise keep a filesystem
reader in an unimported sibling file and be reported as performing nothing.
A *dependency's* `pub` rows do not count toward the consumer, whose ceiling is
what it needs, not what everything it depends on offers; what the consumer
actually calls is already in the row of whatever calls it. An effect you declare, perform and
handle inside your own package needs nothing from anyone and is not a
capability — but the handler answers for what it really does, so a `Log`
handler that calls `print` puts `io` in the ceiling whoever asked for it.

`grasm deps` resolves the graph, checks every package against its ceiling, and
writes `grasm.lock` with a SHA-256 over each package's manifest and sources.
The manifest is in the digest because the `capabilities` line is what a
consumer is relying on: three releases with identical code and different
ceilings hashed the same without it.

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
test "doubling is monotonic" ... FAILED on case 1 of 100
  assertion failed
  counterexample: n = 0
  replay: --seed 24301 --cases 100
```

`0` is the simplest witness the shrinker could reach: `0 * 2` is `0`, which is
not greater. The property is false for `0`, for every negative number, and for
`4611686018427387904` and up, where `n * 2` wraps — the shrinker reports the
simplest one it can, not the first one it found.

That seed is the *default*, and it does not change between runs: `grasm test`
twice runs the same hundred cases twice, so a suite that passes today passes
tomorrow. Running it again is not how you widen coverage — `--seed` is. What
the fixed run does guarantee is the boundary values: the first ten cases walk
them in order, so `0`, `±1`, `±2`, `10`, `100`, the `i64` extremes, NaN, both
infinities, both zeros, both `bool`s and the empty string are tried every time
rather than merely often. The rest of the run is the widening random mix.

`grasm test --coverage` reports which statement lines the suite reached —
counting the lines that doc examples reach, since those are tests too. It did
not, so a function reached *only* by its own documented example was reported
uncovered while the suite had just run it — and then it did, but only for the
entry file: a group of examples runs in a synthesized program where the owner's
source is inlined, so the lines an *imported* module's examples reached were
being read against the wrong file.
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

## 10c. Making a failure happen twice

A program is a function of its code and everything it read from outside itself.
The code is in the binary; the rest can be written down:

```text
$ GRASM_RECORD=j ./server          # the run that went wrong
$ GRASM_REPLAY=j grasm run src/main.gr
```

The second run does not read the clock, the disk, the network or the
environment. It takes every answer from `j` — so it behaves the same on your
laptop as it did on the machine that failed, as many times as you need, under a
debugger.

It also does not *write*. Replaying a `write_file` to reproduce a bug would
damage a disk. `print` is the deliberate exception: a replay whose output you
cannot see is not worth running.

`grasm journal` shows what was recorded:

```text
$ grasm journal j
j — 5 record(s), 264 bytes

  1  now_millis  1787564958324
  2  env         Some("/home/dev")
  3  read_file   Ok("port = 8080\n")
  4  tcp_listen  Err(IoFailed("Permission denied (os error 13)"))
  5  now_millis  1787564958324

  5 call(s) across 4 builtin(s): env 1, now_millis 2, read_file 1, tcp_listen 1
```

**Why this is cheap here and expensive elsewhere.** Knowing which calls are
non-deterministic is normally the hard part of building a record-and-replay
tool. In Grasm it was already written down: a function that reaches outside the
program declares an effect (§5), and one that does not *cannot*. The journal is
"every call with a row, in order, and what it answered".

**Divergence is refused, not guessed.** If the program takes a different path
from the recorded one, the run stops and says which call disagreed:

```text
grasm: at call 3 the recording says `read_file` and this run asked for
`env` — the program has taken a different path from the one recorded, so
this is not a replay of it
```

Handing back the next record anyway would produce something that looks like a
replay and is not, and a tool whose whole promise is *this is what happened*
must not be able to lie about it.

A recording is portable between engines, which is the case that matters: the
failure happens in the compiled build and the debugger is in `grasm run`. It
even replays in a browser tab — see [replay.md](replay.md).

## 11. Where `--native` stops

`grasm run` and `grasm build --native` are two implementations of one language,
and `scripts/difftest.sh` holds them to identical output — and identical exit
codes — on every program in the repository.

For most of this project's life the compiled backend was the *smaller* of the
two, and this section was a list of shapes that ran under `grasm run` and could
not be built. That list was seventeen entries long. It is one, and it is not a
defect being worked off:

| shape | why |
|-------|-----|
| polymorphic recursion | `fn f[T](x: T) -> i64 { f([x]) }` needs one compiled copy per type and the types never stop growing. No amount of work removes this; it is what monomorphization is, and it is refused by name at build time |

Everything else the interpreter accepts, the compiled backend compiles.
`scripts/nativegaps.sh` holds that claim to 55 cases, running each under both
engines and comparing output, exit code and leaks. Six of the fourteen generators in
`scripts/` report how many of the programs they invented the backend turned
away, and all six report `declined: 0` — and in all six a decline is a
*failure* rather than a report. Every shape they generate is one the backend
compiles, so a refusal is coverage lost rather than an honest no: it is either
a new gap, which belongs in the table above, or a defect. Twice now a generator
has counted one and exited 0, and both times the programs stopped running while
the summary said nothing was wrong.

That is a statement about what has been checked, not a proof. A shape nobody has
written is a shape nobody has tried. What has changed is the direction the gate
points: it used to record what the backend could not do, and now it mostly
records what it can.

A refusal, when one appears, is by name and at build time rather than a wrong
answer. That is the promise, and it has been broken: a `fold` whose accumulator
was fallible was named in this table as refused and was quietly miscompiled
instead (F43). Both halves of that shape are in the gate list now — the one that
works and the one that must not.

One row left this table by being answered rather than implemented. A
constructor that leaves a type parameter undetermined — `Good(1)` for
`Res[T, E]` — was refused here and accepted by the other two engines, which
made it look like a backend limitation. [T-GENERIC-TYPE] says it is the
language's rule, so the *checker* enforces it now and all three engines agree.
A shape on a list of backend gaps is worth asking about twice: whether the
backend should accept it, or whether the others should not.

## Where to go next

- `spec/` — the normative rules, each tagged and backed by a program in
  `spec/conformance/` that the test suite runs.
- `examples/` — runnable programs, including `stdlib_tour.gr` and
  `testing_tour.gr`.
- `projects/` — twelve larger programs, beginner through advanced.
- `docs/replay.md` — recording a run and replaying it, in full.
- `docs/wasm.md` — running a Grasm program in a browser tab.
- `MASTERPLAN.md` — where the language is going and why.
