# What thirty ordinary programs found

Running list. Each entry gets a severity from `docs/release-scope.md` and either
a fix or a reason it is not fixed.

## F1 — a wrapped condition must end the line with the operator (S4) — **fixed**

Found writing 06-contact-book. This is refused:

```grasm
if c.name.contains(needle)
    or c.email.contains(needle) {
```

and this is accepted:

```grasm
if c.name.contains(needle) or
    c.email.contains(needle) {
```

The rule is consistent — a newline ends a statement unless the line *ends* in an
operator, or you are inside brackets — and it is the same rule Go uses. The
defect was the diagnostic, which said `expected `{` but found end of line` and
nothing else. Nothing told the reader what to move.

**Fixed.** When the next line begins with a binary operator, the help now says
so and names it:

```
help: the next line starts with `or`, so this statement ended here — a newline
      ends a statement unless the line *ends* in an operator. Move the `or` up
      to the end of this line, or wrap the whole expression in parentheses
```

## F2 — `str.len()` counts code points, and byte length is unaskable (S4, library)

Found writing 04-palindrome-checker. `"🙂".len()` is 1 and `"añb".len()` is 3.
Consistent and probably right for a high-level language, but there is no way to
ask how many *bytes* a string occupies — which matters the moment a program
writes one to a socket or a file and has to say how long it is.

## F3 — no bitwise operators at all (S3, language)

No `>>`, `<<`, `&`, `|`, `^` on `i64`. Blocks 18 (BMP), 25 (DNS), 27 (SHA-256)
outright, and forces division-by-power-of-two in place of shifts everywhere
else. Hex literals (`0xFF`) do parse.

## F4 — `_` is a pattern in `match` and nowhere else (S3) — **fixed**

Found writing 08-fibonacci. All three of these are refused:

```grasm
for _ in 0..n { … }        // expected an identifier but found `_`
let _ = f()                // same
let (_, b) = pair          // same
```

while `match x { Some(_) -> … }` is fine. `spec/core.md` classifies `_` as a
pattern form and `let (a, b) = e` as a Full form that rewrites to projections —
so a reader who has read the specification expects `_` to work in all four
places.

The cost is not only cosmetic. [T-FALLIBLE-UNHANDLED] requires a failure to be
handled, and `let _ = risky()` is the natural way to write "I have read this and
I am deliberately dropping it". There is no spelling for that.

**Fixed**, as [G-WILDCARD-BINDING]. `_` is now accepted wherever a name is
bound, and each one is a distinct unwritable binding, so `let (_, _) = pair`
binds nothing twice and none of them can be read back.

One part of the finding was **wrong and is not fixed, deliberately**. `let _ =
risky()` is still refused. [T-FALLIBLE-UNHANDLED] applies to a wildcard exactly
as to a named binding, and it should: a binding being unnamed says nothing about
whether its error was considered. Rust allows `let _ =` to drop a `Result`
silently, which is why it needed `#[must_use]`; not repeating that is the better
answer, and the existing help already names the annotation that holds the
failure as a value.

## F5 — a string literal cannot span lines, and there is no raw form (S3, language)

Found writing 10-grep-utility. This is refused with "unterminated string
literal":

```grasm
let doc = "the quick brown fox
jumps over the lazy dog"
```

There is no triple-quote, no backtick, and no line continuation inside a
literal. Every embedded document — a test fixture, a block of JSON, a SQL
statement, an HTML template — has to be written as one long line with `\n`
escapes, or built by concatenating a list.

This matters more than it looks. `[G11]` measures Grasm's token count against
Python's, and a language whose only way to write a five-line fixture is five
concatenations pays for it in exactly the benchmark it is trying to win. It is
also the single most common thing a first program needs that this language
cannot express.

Workaround used here: `"line one\nline two"`, and `.lines()` to split it back.

## F6 — a file can be written and truncated, never removed (S4, library)

Found writing 11-todo-file-storage. The library has `read_file`, `write_file`
and `file_exists`, and nothing that deletes. A program that creates a temporary
file can only empty it, so `file_exists` stays true forever and the next run
sees a leftover. There is also no way to list a directory, which the same
program would want for "load every todo file".

## F7 — there is no way to write to stderr (S3, library)

Found writing 15-sorting-benchmark. `print` goes to stdout and nothing goes to
stderr. A command-line tool cannot separate its diagnostics from its output,
which is the entire basis of `prog | filter` on any Unix system — and this
directory's own grep (project 10) would want it.

It also has a testing cost. A benchmark's timings are diagnostics, not results;
with only one stream they land in stdout and make the program's output
machine-dependent, so it cannot be compared across engines. Project 15 works
around it by printing derived facts (growth ratios, classified) instead of
milliseconds.

The runtime already separates them — a panic goes to stderr — so this is a
missing builtin rather than a missing capability.

## F8 — `extern fn` cannot pass a pointer, so most of libc is unreachable (S3, FFI)

Found trying to write 17-simple-shell.

```grasm
extern fn system(cmd: str) -> i32
// error: `str` has no C representation
// help: an `extern fn` takes and returns `i64` or `f64`
```

The C boundary carries scalars only. That excludes `system`, `open`, `read`,
`write`, `printf`, `socket`, `getaddrinfo` — every libc function that takes a
pointer, which is nearly all of the useful ones. What `extern fn` can reach
today is `abs`, `sqrt`, `pow` and their kind.

MASTERPLAN G9 promises "C ABI import overhead: zero-cost" and `docs/northstar.md`
records it as *partly met*. This is how partial: the overhead is indeed zero, and
the set of functions you can call is arithmetic.

It is also the reason several projects on this list cannot be written. A byte
type would fix the FFI and the binary-I/O gap (F9) at the same time, since both
want the same thing: a contiguous run of bytes with a length, that a C function
can be handed a pointer to.

## F9 — no byte type, so binary formats and binary I/O are unwritable (S3, language)

A socket carries `str` ([L-LIB-NET]) and so does a file. There is no `[u8]`, no
`bytes`, and `str.len()` counts code points rather than bytes (F2), so even
measuring a payload is not possible.

Blocks 18 (read a BMP), 25 (DNS packets), and the binary half of 21 and 30. It
is also why `server.gr` cannot serve `grasm.wasm` — noted already in
`docs/wasm.md`.

## F10 — a `ref` to a scalar can be made but never read (S3, language)

Found writing 19-event-emitter. `ref(0)` typechecks and produces a `ref i64`,
and there is no expression that reads it. [T-REF-READ] and [T-REF-WRITE] both
define access through a *field* — `r.field`, `r.items.push(x)`, `r.n += 1` — and
a scalar has no field. `.get()`/`.set()` do not exist. Printing one gives
`ref#1.0`, the handle rather than the value.

So `ref` is usable only when it points at a struct or a list. The workaround is
a one-field wrapper:

```grasm
type Cell = { n: i64 }
let c = ref(Cell{ n: 0 })
c.n += 1
```

which is what this project uses. The gap is that `ref(0)` is accepted at all: a
value that can be constructed and never observed should either be refused at the
point of construction, or given the dereference that would make it useful.
