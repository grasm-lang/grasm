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

## F3 — no bitwise operators at all (S3) — **fixed as [L-LIB-BITS]**

No `>>`, `<<`, `&`, `|`, `^` on `i64`. Blocked 18 (BMP), 25 (DNS) and 27
(SHA-256) outright, and forced division-by-power-of-two in place of shifts
everywhere else. Hex literals (`0xFF` ) did parse.

**Fixed** — as seven methods, not operators: `bit_and`, `bit_or`, `bit_xor`,
`bit_not`, `shl`, `shr`, `ushr`. Every symbol was already taken or ambiguous:
`^` is exponentiation, `|` begins a closure and separates sum-type variants, and
`&&` is already reported as "written `and`". Words would collide with the
logical operators, which are `and`/`or`/`not` — `a and b` boolean and `a band b`
bitwise, one letter apart, is worse than either. Methods are what `.abs()` and
`.cmp()` already are.

The shift count is masked to `0..63` rather than trapping or being left
undefined. Rust's `<<` panics in debug and wraps in release, which is the
class of difference-between-builds this project refuses to ship; Cranelift
leaves an out-of-range count undefined, which would make the engines disagree.
Masking is what the hardware does and it is the same answer everywhere —
verified: `1.shl(64)` is `1` and `256.shr(-1)` is `0` under both engines.

This unblocked project 27, which now passes 12 of 12 SHA-256 vectors.

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

## F11 — `net` can listen and accept, never dial (S3, library)

Found writing 22-multithreaded-scraper. [L-LIB-NET] gives `tcp_listen`,
`tcp_accept`, `tcp_read`, `tcp_write` and `tcp_close`. There is no
`tcp_connect`. A Grasm program can be a server and cannot be a client, so it
cannot fetch a URL, call an API, or talk to a database.

The missing piece is one builtin with the same shape as the others —
`tcp_connect(host: str, port: i64) -> i64 ! IoError` — and the capability, the
handle table and the read/write paths all already exist. This is the smallest of
the outstanding gaps and it blocks the most.

`par_map` would supply the concurrency the project asks for; the threads are not
the problem.

## F12 — no UDP (S4, library)

Found writing 25-dns-resolver. Only TCP exists. DNS is a UDP protocol, and
while it can be done over TCP, the project as stated is a datagram exercise.
Blocked behind F9 in any case: a DNS query is a packed binary structure with
bit-level flags, and there is no byte type to build one in.

## F13 — every socket operation blocks (S3, library)

Found writing 30-async-chat. `tcp_accept` and `tcp_read` both block, and there
is no `select`, no poll, no timeout, and no way to ask whether a handle is
ready. A single-threaded server can therefore serve exactly one connection at a
time — which is what project 21 does, correctly and deliberately.

Multiple simultaneous connections would need either non-blocking handles with a
readiness check, or a thread per connection. `spawn` is documented as
single-core ([E-NOT-PARALLEL]) and `par_map` runs a fixed batch to completion,
so neither gives a connection its own thread of control today.

This is the one gap on the list that is a genuine design question rather than a
missing builtin, because the answer interacts with the effect system: an `async`
row already exists in the vocabulary and nothing performs it.

## F14 — there is no way to do concurrent I/O (S3, language) — the biggest one

Found writing 22-multithreaded-scraper, after `tcp_connect` unblocked it.

```grasm
let pages = paths.par_map(|p| fetch_one(8971, p))
```
```
error: this parallel operation needs an effect-free function, but the one given
       performs `net`
  help: elements are processed on several cores, so an effect would interleave
        differently between runs — use `map` for effectful work
```

The reasoning is sound: `par_map` promises a deterministic result, and effects
running on several cores would interleave differently between runs. But taken
together with the other half of the story it leaves nothing:

* `par_map` is the only multicore path, and it **refuses every effect**.
* `spawn` / `parallel` are documented as **deliberately single-core**
  ([E-NOT-PARALLEL]), so they give concurrency but not parallelism.

So a Grasm program can compute on many cores, and can perform I/O, and **cannot
do both at once**. Every concurrent server, every parallel fetcher, every
pipeline that reads while it computes is unwritable. Project 22 falls back to
fetching sequentially; project 30 is blocked outright.

This is the largest gap in this directory, and it sits directly under the
north-star goal that claims the most. MASTERPLAN G6 asks for "structured,
uncolored concurrency, 1M+ tasks/node"; `docs/northstar.md` already records it
as **retreated**. This is what that retreat costs in practice.

It is also the one gap whose fix is *interesting* rather than mechanical, and
the language already contains the shape of it. `async` is in the effect
vocabulary and nothing performs it. An I/O operation that performs `async`, a
handler that decides what that means, and `par_map`'s refusal narrowed from "any
effect" to "any effect that is not `async`" is a design that fits the existing
type system — and is close to how WebAssembly's stack-switching proposal is
specified, which is the same bet `docs/wasm.md` argues for.

## F15 — a wasm trap poisons the instance, and the docs did not say so (S4, docs)

Found by clicking "Run all" on the browser showcase, not by any harness.

Project 08 recurses 50,000 deep. A wasm module's stack is fixed at link time, so
it traps — which `docs/wasm.md` already records. What was not recorded is that
the trap leaves **the whole instance unusable**: every program run afterwards in
that same instance failed too, so one program taking the stack down turned into
sixteen "TRAPPED" lines.

The command-line harness never saw it, because `scripts/wasmtest.sh` runs each
program in its own node process.

Not a defect in the language or in the wasm build — it is how WebAssembly is
specified to behave. It is a defect in the *embedding guidance*: an embedder
running more than one program must instantiate per run, and `docs/wasm.md` now
says so. `WebAssembly.compile` once and `new WebAssembly.Instance` per program
costs about a millisecond and confines a trap to the program that caused it.
