# TIR: the typed intermediate representation

TIR is a checked Grasm program with every decision the checker makes
written down: the type of every expression, what every name refers to,
which function or built-in every call reaches, and every conversion the
source leaves implicit. It is the seam of the self-hosting plan
(`docs/self-hosting.md`, stage S0):

- the Rust compiler emits it (`grasm tir <file.gr>`) and so does the
  Grasm frontend (S2); the two must produce **identical text** for every
  program in the corpus;
- the Grasm TIR interpreter runs it, and must print and exit exactly as
  `grasm run` does for every program in the corpus — which is how TIR is
  shown to carry the whole meaning of a program before anything else is
  built on it;
- the self-hosted backend (S3) lowers it to MIR.

TIR is **not** an optimisation IR. It is as close to the source as it can
be while needing no inference, no name lookup and no overload resolution to
consume. It is not monomorphized: a generic function appears once, with
explicit type arguments at every use.

## 1. What a consumer may assume

1. The program checked with no errors. Programs with errors have no TIR.
2. Nothing is resolved by the consumer. Every reference names exactly
   what it reaches: a function or constant by its qualified name
   (`module::name`, unique in a program), a type by its name (type names are
   global), a local by its variable number.
3. Every expression has a type with no inference variables in it.
4. Every implicit conversion is an explicit node (§6.6).
5. The parser's desugarings are already applied: `impl` methods are
   free functions named `method@Type`, contracts are their lowered
   functions, `|>` pipes are calls. TIR adds no desugaring of its own
   beyond those, so a TIR program and its source line up construct for
   construct.

## 2. Program

```
program
  files    : [file]         -- in loader order
  types    : [typedef]      -- named struct and sum types, prelude first
  effects  : [effectdef]
  consts   : [constdef]
  externs  : [externdef]
  fns      : [fndef]        -- functions, impl methods, contract functions
  exports  : [fn id]        -- each `export fn`, printed right after its fn
  cells    : [celldef]      -- each `thread_local`, in source order with the rest
  impls    : [impldef]      -- (interface, type) -> fn ids
  tests    : [testdef]
  benches  : [benchdef]
  main     : fn id | none
```

Every list is in **source order**: files in loader order, declarations
in the order they appear in their file, prelude types first. Two frontends
that agree on the source write the same declarations in the same order.

A `file` records its module name, alias, path as given to the loader, and
the base offset the loader gave it. Spans throughout are `(start, end)` in
that global offset space.

## 3. Types

```
type =
  | i64 | i8 | i16 | i32 | u8 | u16 | u32 | u64 | f64 | bool | str | bytes | unit | range
  | strbuilder | bytebuilder
  | addr                           -- an address in raw memory ([T-ADDR])
  | tuple [type]
  | list type
  | map type type
  | ref type
  | named typedef_id [type]        -- a struct or sum type with its arguments
  | fn [type] type row             -- row: unconstrained | {effect names}
  | fallible type type             -- T ! E
  | union [type]                   -- an error union (A | B)
  | generic n                      -- the n-th generic parameter of the enclosing definition
  | never                          -- no value: what `fail` returns, an error half nothing raises
```

`never` is what an inference variable that nothing constrained becomes: the
checker gives `fail(e)` a type that fits anywhere, and leaves the error half
of a `main` that never fails open, and in both no value of the type exists.

`typedef` = name, generic parameter names, linear flag, and either
`struct [(field name, type)]` or `sum [(variant name, [(field name, type)])]`.
Type aliases are resolved away: they do not appear in TIR.

## 4. Declarations

```
fndef     = qualified name, generic names, bounds per generic, params, ret, err, row, body
param     = mode (let | inout | own), var id, type
constdef  = qualified name, type, init expr
externdef = name, params, ret
celldef   = qualified name, layout type        -- printed (threadlocal "m::C" u16)
effectdef = name, ops [(name, params, ret)]   -- printed (op name (params) ret)
impldef   = interface name, type (as written), generics, [(method name, fn id)]
testdef   = name, params, body
benchdef  = name, body
```

`row` is the function's declared or inferred effect row as a sorted list of
effect names. An `export fn` ([T-EXPORT]) is an ordinary `fndef` followed by
`(export "module::f")`.

An effect operation has no separate error type: one declared `! E` has the
return type `T ! E`, so its `ret` is printed `(fallible T E)` and an
`effectop` node's type says the same.

## 5. Variables

Every binding inside a function gets a **var id**, numbered from 0 in the
order its binding site appears in the source: parameters first, then `let`
and `var` names, `for` variables, pattern bindings, closure parameters, and
the names of a `let (a, b) = …`. A var records its name, type and whether
it is mutable. A closure's body shares its enclosing function's var
numbering.

## 6. Expressions

Every expression carries its type and its span. The kinds:

### 6.1 Leaves

```
int v k          -- integer literal, k its integer type; a u64 value is printed unsigned
float v          -- written as the shortest repr that reads back exactly
bool b
str [piece]      -- piece = text "..." | value expr (formatted by its type, §8)
unit
local var
fnref fn [type]          -- a named function used as a value, with type arguments
externref extern
constref const
```

An integer literal in an `f64` position is a `float` node: [T-INT-TO-FLOAT]
says it is converted at compile time, so TIR holds the converted value.

### 6.2 Construction

```
list [expr]              -- the element type is the node's type
map [(expr, expr)]
tuple [expr]
struct typedef [type] [(field index, expr)] base
variant typedef index [type] [expr]
closure [var] fallible body
```

### 6.3 Calls — every call says what it reaches

```
call fn [type] [expr]            -- a function, with type arguments
callextern fn [type] [expr]      -- an `extern fn`
callvalue expr [expr]            -- calling a function value
builtin name [expr]              -- print, map(), read_file, chan_new, Ok, fail, ...
effectop effect op [expr]        -- an effect operation
method name [expr]               -- a built-in method; args[0] is the receiver
boundcall interface method [expr] -- an interface method on a generic receiver
```

Two raw-memory arguments are names rather than values ([L-LIB-MEM],
[L-LIB-ATOMIC]), and print as leaves with no type: the type `mem_size_of`
and `mem_align_of` take is `(type u16)`, and an atomic's ordering, its last
argument, is `(order acquire)` with its span (`relaxed`, `acquire`,
`release`, `acqrel`, `seqcst`). Three more are names ([T-FN-ADDR],
[L-LIB-CALL-C], [T-TLS]): the function `mem_fn_addr` takes is `(fnname "module::f")`,
the signature that is `call_c`'s first argument is `(sig (i64 str) i64)`,
`unit` for no result, and the cell `mem_tls` takes is
`(tlsname "module::C")`. A consumer must not evaluate any of them. Every other
raw-memory operation is an ordinary `builtin` (`mem_alloc`, ...) or `method`
(`offset`, `load_u8`, `atomic_cas_i64`, `call_c`, ...) node. A `method` node
is a raw-memory one exactly when its receiver's type is `addr`, since an
address has no other built-in methods. A consumer goes by that, as the
engines go by the checker's table, and not by the name alone: `as_i64` is
also a method of every integer type.

A method call in the source becomes one of these, by the checker's
resolution: a built-in method (`method`), a UFCS call of a free function
(`call`), an interface method through its `impl` (`call` of the impl's
function), or a call of a function held in a struct field (`callvalue` of
a `field` node). On a receiver of a generic type the method an `impl`
provides is a `boundcall`: which function runs depends on the type the
generic is instantiated with, through the program's `impl` table.

### 6.4 Access

```
field expr index          -- struct field, by position (also through a `ref`)
tuplefield expr index
deref expr                -- `.value` of a `ref` to anything but a struct or tuple
index expr expr           -- a list element; nothing else can be indexed
range expr expr inclusive
```

`Type.Variant`, `module.CONSTANT` and `module.function` (as a value) are the
`variant`, `const` and `fnref` they name, not field accesses.

### 6.5 Operators

```
unary op expr          -- op: neg | not
binary op expr expr
```

`op` is written with the operand type that decides its meaning:
`add.i64`, `add.u8`, `add.f64`, `add.str` (concatenation), `add.list`,
`eq.<type>`, `lt.str`, `and`, `or`, `coalesce.option`, `coalesce.fallible`,
`pow.i64`, … A fixed-width operation is written with its width, so the
wrapping [T-INT-FIXED] requires is in the node, not in a side table.

### 6.6 Conversions

```
wrapok expr        -- the bare ok half where `T ! E` is wanted ([T-OK-ERR-RETURN])
```

Literal conversion to `f64` is folded (§6.1) and narrow-integer wrapping
is part of the operator (§6.5), so `wrapok` is the only conversion node.

### 6.7 Control

```
if expr block else
match expr [arm]          -- arm = pattern, guard, body
block block
region block
unsafe block
try expr                  -- propagate the error half
```

## 7. Statements and patterns

```
let var expr | lettuple [var] expr | assign place op expr | expr expr
return expr? | break | continue
while expr block | for var iter block
parallel concurrent proved block | spawn concurrent block
with effect handler block
```

`iter` records what is iterated, by the iterable's type: `range`, `list`,
`str` (its characters) or `bytes`. A map is walked through a list such as
`m.keys()`. Only `range` and `list` occur in the corpus, so the `str` and
`bytes` kinds are unexercised: nothing has run them from TIR. `place` is a
local, a field path, or an index path rooted at a local. `proved` is the
checker's [E-SPAWN-ORDER] verdict that a `parallel` block's tasks are
independent.

```
pattern = pwild | pint v | pbool b | pstr s | pbind var
        | pvariant type index [pattern] | pok pattern | perr pattern
        | ptuple [pattern] | por [pattern]
```

`pok` and `perr` match the halves of a `T ! E`. A bare name the checker did
not bind (`None`) is the nullary variant of that name. A pattern matches
through a `ref`. The alternatives of a `por` bind the same names to the same
variables.

## 8. What TIR does not record

Captures, liveness, reference-count operations, evaluation order beyond
the source's left-to-right, and string formatting rules: these are the
consumers' business and are specified by the language (`spec/`). A `str`
piece holds the expression; how a value of its type is written is the
same rule `print` follows.

## 9. Text form

One s-expression per declaration, printed in program order, with
two-space indentation and no trailing spaces, ending in a newline. Types
print in the syntax of §3. Spans print as `@start:end`. Strings print
JSON-escaped. Floats print as the shortest text that reads back as the same
`f64`; negative zero as `-0.0`; non-finite values as `nan`, `inf`, `-inf`.
The text form round-trips: reading it and printing it again gives the same
bytes, and two programs with the same TIR text are the same program.

## 10. Holes

The emitter resolves nothing; anything the checker did not record is
printed `?` and reported with its file, line and source text, and
`grasm tir` then fails. The `tirtest` gate requires the whole corpus to
emit with none.

## 11. Status

S0 in progress. The Rust emitter lives in `crates/grasm_tir`; the checker
records the decisions it makes (name resolution, call and method targets,
type arguments) in tables for it, so no resolution is done twice.

The TIR interpreter is `tools/tirinterp`, written in Grasm and built
natively. It reads the text form with its own reader and runs `main` from
the TIR alone, and `scripts/tirinterp.sh` holds it to `grasm run`: every
corpus program that emits TIR must read and print back to the same bytes,
and every program with a `main` must print and exit as the reference does.
No program is skipped: a C function is found in the interpreter's own
process with `dlsym` and called through `call_c`, as `grasm run` finds and
calls one in itself (docs/self-hosting.md §5.2). No program in the corpus
needed anything the TIR does not say. Raw memory runs in a sandbox of the
interpreter's own, built as the reference's is ([E-MEM-UNCHECKED]): the same
made-up addresses, the same checks, the same panics; a function's address and
a thread-local cell are made from it as the reference makes them. A compiled
build checks none of those, so the programs that hold the interpreter to them
cannot be corpus programs; they are in `tools/tirinterp/panics`, one per
panic, and only this gate runs them.
