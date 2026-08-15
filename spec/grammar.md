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
