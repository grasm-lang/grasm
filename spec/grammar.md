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
