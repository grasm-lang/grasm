# What thirty ordinary programs found

Running list. Each entry gets a severity from `docs/release-scope.md` and either
a fix or a reason it is not fixed.

## F1 — a wrapped condition must end the line with the operator (S4, diagnostics)

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
defect is the diagnostic, which says `expected `{` but found end of line` and
then reports a second, invented error at the leading `or`. Nothing tells the
reader that moving the operator up one line works.

## F2 — `str.len()` counts code points, and byte length is unaskable (S4, library)

Found writing 04-palindrome-checker. `"🙂".len()` is 1 and `"añb".len()` is 3.
Consistent and probably right for a high-level language, but there is no way to
ask how many *bytes* a string occupies — which matters the moment a program
writes one to a socket or a file and has to say how long it is.

## F3 — no bitwise operators at all (S3, language)

No `>>`, `<<`, `&`, `|`, `^` on `i64`. Blocks 18 (BMP), 25 (DNS), 27 (SHA-256)
outright, and forces division-by-power-of-two in place of shifts everywhere
else. Hex literals (`0xFF`) do parse.

## F4 — `_` is a pattern in `match` and nowhere else (S3, language)

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

Workaround: invent a name. The language does not warn about an unused binding,
so `let ignored = f()` compiles — which is worse than `_`, because now the
reader cannot tell a deliberate discard from a forgotten one.

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
