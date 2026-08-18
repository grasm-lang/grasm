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

## F2 — `str.len()` counts code points, and byte length is unaskable (S4, library) — **fixed**

Found writing 04-palindrome-checker. `"🙂".len()` is 1 and `"añb".len()` is 3.
Consistent and probably right for a high-level language, but there is no way to
ask how many *bytes* a string occupies — which matters the moment a program
writes one to a socket or a file and has to say how long it is.

**Fixed** — `s.byte_len()`, alongside `len()` rather than replacing it
([L-LIB-STR]). Characters stay the default because they are the right unit for a
high-level language; bytes are what a `Content-Length` counts, and computing one
from `len()` is wrong for every string that is not ASCII.

Adding it found an S1 in the native backend, which is recorded as F17.

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

## F5 — WITHDRAWN. I did not check before recording it

I recorded that "a string literal cannot span lines and there is no raw form",
and stated flatly: "There is no triple-quote, no backtick, and no line
continuation inside a literal."

**That was false.** `"""…"""` raw blocks have existed since commit C20 — raw,
multi-line, no escapes, no interpolation, newlines kept. The lexer function is
called `lex_raw_string` and it is forty lines above the code I was reading.

```grasm
let doc = """the quick brown fox
jumps over the lazy dog
a line with { braces } and "quotes" and \n literal backslash-n"""
```

I hit "unterminated string literal" while writing project 10, concluded the
feature was absent, and wrote the finding without searching for it. The
diagnostic was right and my inference from it was wrong.

Project 10 has been rewritten to use the raw form, which is what it should have
used in the first place.

**The lesson is the finding.** Eleven of these entries were written the same way
— hit an error, infer a gap. This is the one that was checkable and wrong, and
there is no reason to think it is special. Every remaining open finding should be
re-verified against the implementation before any work is planned around it.

## F6 — a file can be written and truncated, never removed (S4) — **fixed**

Found writing 11-todo-file-storage. The library has `read_file`, `write_file`
and `file_exists`, and nothing that deletes. A program that creates a temporary
file can only empty it, so `file_exists` stays true forever and the next run
sees a leftover. There is also no way to list a directory, which the same
program would want for "load every todo file".

## F7 — there is no way to write to stderr (S3) — **fixed**

Found writing 15-sorting-benchmark. `print` goes to stdout and nothing goes to
stderr. A command-line tool cannot separate its diagnostics from its output,
which is the entire basis of `prog | filter` on any Unix system — and this
directory's own grep (project 10) would want it.

It also has a testing cost. A benchmark's timings are diagnostics, not results;
with only one stream they land in stdout and make the program's output
machine-dependent, so it cannot be compared across engines. Project 15 works
around it by printing derived facts (growth ratios, classified) instead of
milliseconds.

The runtime already separates them — a panic goes to stderr — so this was a
missing builtin rather than a missing capability.

**Fixed** as [L-LIB-EPRINT]. The compiled build brackets the write with a flag the
runtime reads rather than carrying a second copy of the formatting code, so there
is one code path and the two streams cannot format differently.

## F8 — `extern fn` cannot pass a pointer, so most of libc is unreachable (S3, FFI) — **fixed**

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

**Fixed** — [T-EXTERN-POINTER]. An `extern fn` parameter may now be `str` or
`bytes`, and crosses as a pointer. `strlen`, `atoi`, `memcmp`, `open`, `write`,
`getenv` and the rest of the pointer-taking half of libc are reachable.

This finding predicted its own fix: "A byte type would fix the FFI and the
binary-I/O gap (F9) at the same time." It did, and it took a year of this
repository's time in between — F9 shipped `bytes` for the binary-I/O half and
nothing then connected it to the boundary.

Three decisions, and the asymmetry between them is the whole design:

* A `str` crosses as a **NUL-terminated copy**, because Grasm strings carry
  their length and C looks for a terminator. A string containing a NUL panics
  rather than being silently cut short at it.
* `bytes` crosses as a pointer with **no copy and no length**. The buffer is
  already contiguous and appending a NUL would change its length, so how long
  it is stays the program's to pass — `memcmp(a, b, a.len())`.
* A pointer may **not come back**. Nothing about a returned pointer says who
  owns it, how long it lives, or how long it is, and there is no answer to
  invent. Passing one is expressible precisely because the buffer belongs to
  Grasm and is alive for the call.

The safety story needed nothing new. A declared C function's row was already
`{io, unsafe}` and every use already sits inside `unsafe`, so widening what may
cross did not widen what a package can hide.

17-simple-shell, the project that found this, was unblocked earlier and by a
different route — `run_program` under the `proc` capability ([L-LIB-PROC]) —
because a shell wants to *start a program*, not to reach C. That was the right
call for that project and it is not what closed this.

It is also the reason several projects on this list cannot be written. A byte
type would fix the FFI and the binary-I/O gap (F9) at the same time, since both
want the same thing: a contiguous run of bytes with a length, that a C function
can be handed a pointer to.

## F9 — no byte type, so binary formats and binary I/O are unwritable (S3) — **fixed**

A socket carries `str` ([L-LIB-NET]) and so does a file. There is no `[u8]`, no
`bytes`, and `str.len()` counts code points rather than bytes (F2), so even
measuring a payload is not possible.

Blocked 18 (read a BMP), 25 (DNS packets), and the binary half of 21 and 30. It
was also why `server.gr` could not serve `grasm.wasm`.

**Fixed** as [L-LIB-BYTES]. `bytes` is a contiguous run of bytes, distinct from
`[i64]` because a list of integers cannot promise its elements are bytes and
stores eight bytes per element. It prints as hex rather than as the text it might
decode to, which would undo the distinction the type exists to make.

The proof is project 18: it writes a real BMP that `file` identifies as
"PC bitmap, Windows 3.x format, 3 x 2 x 24" and that an independent Python
decoder reads back pixel for pixel.

Still open for 25 (which also needs UDP, F12) and for `extern fn` (F8), which
needs a pointer representation on top of this.

## F10 — a `ref` to a scalar can be made but never read (S3, language) — **fixed**

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

**Fixed** — the dereference, not the refusal ([T-REF-VALUE]). `r.value` reads
and writes the referent, and a write lands in the slab, so a `ref i64` is a
shared counter rather than a copy. That is the thing the wrapper struct was
imitating.

It is deliberately not offered when the referent is a struct or a tuple: a
struct already has field names and `.value` would either shadow a real one or
change meaning the day somebody added it, and a tuple has `.0` and `.1`. The
rule is exactly "the referent has no way to name its parts, so this names the
whole thing", which leaves no case where both readings apply.

Four places had to learn it, which is the shape of this backend: the checker's
field typing, the interpreter's read and its write, and — in the native backend
— `place_type`, `read_place`, `read_place_boxed` and `write_place`. Each refusal
came with its own message, so each was found by compiling the same nine-line
program again.

## F11 — `net` can listen and accept, never dial (S3, library) — **fixed**

Found writing 22-multithreaded-scraper. [L-LIB-NET] gives `tcp_listen`,
`tcp_accept`, `tcp_read`, `tcp_write` and `tcp_close`. There is no
`tcp_connect`. A Grasm program can be a server and cannot be a client, so it
cannot fetch a URL, call an API, or talk to a database.

The missing piece is one builtin with the same shape as the others —
`tcp_connect(host: str, port: i64) -> i64 ! IoError` — and the capability, the
handle table and the read/write paths all already exist. This is the smallest of
the outstanding gaps and it blocks the most.

**Fixed** — `tcp_connect(host, port)` shipped as part of [L-LIB-NET], with the
same shape as the rest of the surface. Resolution failure and connection failure
are the same `IoFailed` to a caller, because both mean "there is nothing to talk
to", and the message carries the difference.

The last line of this finding, written before any of it was built, guessed
wrong: "`par_map` would supply the concurrency the project asks for; the threads
are not the problem." The threads were never available (F14) and were never
needed — `net_wait` and one thread were. Left here rather than edited out,
because a finding that records what was believed at the time is worth more than
one that pretends it was always right.

## F12 — no UDP (S4, library) — **fixed**

Found writing 25-dns-resolver. Only TCP exists. DNS is a UDP protocol, and
while it can be done over TCP, the project as stated is a datagram exercise.
Blocked behind F9 in any case: a DNS query is a packed binary structure with
bit-level flags, and there is no byte type to build one in.

**Fixed** — `udp_bind`, `udp_send` and `udp_recv` ([L-LIB-UDP]), once `bytes`
existed to carry the payload. The signatures deliberately do not mirror the
stream ones: a datagram is a whole message sent to an address named at send
time, so `udp_send` takes the destination on every call, and giving the two
operations the same shape would have thrown away the thing that makes UDP worth
having.

Datagram handles share the socket table with stream handles, so `net_wait`,
`net_port` and `net_close` work on all three kinds. That is what lets the
resolver time out on a server that never answers instead of hanging — the
ordinary failure, not the exotic one. 25 now resolves against 8.8.8.8.

The general operations were renamed from `tcp_close`/`tcp_port` to
`net_close`/`net_port` at the same time. They were never TCP-specific, and a
`tcp_close` that closes a UDP socket is a name that lies.

## F16 — a method chain cannot be split across lines (S4, grammar) — **fixed**

Found writing 25-dns-resolver. A statement ends at the newline, so

```grasm
u16_bytes(id)
    .concat(u16_bytes(flags))
```

does not parse: the second line starts a new statement with a leading `.`.
Building a DNS packet is nine concatenations, and the choice is one very long
line or a `join([...])` helper.

Not a defect — newline termination is a deliberate choice and it is why Grasm
needs no semicolons. But every language with newline termination and method
chaining has had to answer this (Python's parentheses, Swift's and Kotlin's
continuation rules, Go's rule that the `.` must end the line), and Grasm had
not.

**Fixed** — [L-LEADING-DOT]. A newline immediately before a `.` or a `?.` is
absorbed. This is safe for one reason and only that reason: no statement in the
language may begin with a `.`, so a newline in front of one cannot have been
separating two statements. `..` and `..=` are distinct tokens, so a range still
cannot straddle a line, and a leading `+` is still an error — the existing rule
that a line *ending* in an operator continues is unchanged.

Project 25's packet builders were rewritten as ordinary chains, which is the
only honest test of the fix: the program that found the gap now reads the way it
wanted to, and its output is byte-identical.

## F21 — the callable builtins and the reserved names were two lists, and they had drifted (S2, checker) — **fixed**

Found adding `run` for [L-LIB-PROC]. The checker had a list of builtin free
functions a call resolves to, and a *separate*, hand-written list of names a
program may not declare. They were meant to be the same list.

They were not. The whole `tcp_*` family was callable and none of it was
reserved, so a program could declare `fn tcp_read` and watch the builtin answer
instead — the accepted-and-ignored failure the reserved-name rule exists to
prevent, in the exact place it was supposed to work.

Adding `run` hit the same seam from the other side: the name became callable
and was still not reserved, so two programs on this list that already had a
`fn run` kept compiling while their own function stopped being reached. The
error they got was about argument counts and said nothing about a name being
taken.

**Fixed** — one list. `is_builtin_fn_name` is derived from `BUILTIN_FREE_FNS`
rather than written out again, so the drift cannot recur, and a test asserts
every callable builtin is reserved.

### The wrong fix, and why it was reverted

The first attempt made a program's own function *shadow* a builtin of the same
name. It cost three separate edits — checker, effect pass, backend — and each
one was a place the two could disagree. It also introduced an engine
disagreement (`inout` write-back was lost on the shadowed path, so `grasm run`
printed `1` where the native build printed `6`) and a performance collapse: the
interpreter formatted a qualified name on *every* call to answer "is this
shadowed?", and 08-fibonacci went from under a second to five minutes.

All of that was solving a problem the language had already solved, better, with
a rule and a clear error. The real defect was that `run` is a bad name for a
builtin.

### The rule that came out of it

Reserving a name is permanent and total. So a builtin is named `verb_noun` —
`read_file`, `write_bytes`, `tcp_listen`, `udp_bind`, and now `run_program` —
and the bare verbs a program is likely to want for its own functions stay
available. A test asserts that: `run`, `send`, `open`, `close`, `read`, `write`,
`start`, `stop`, `wait`, `execute` and twenty more must never appear in the
builtin list.

## F22 — `.value` was handled at the root of a place and not inside one (S2, backend) — **fixed**

Found by probing the surface shipped this session rather than by any harness.
Eight features had landed in a few hours, each with a conformance program
written by the same hand that wrote the feature, which is exactly the shape of
coverage that agrees with itself.

```grasm
var cells: [ref i64] = []
cells.push(ref(0))
cells[0].value = 100
```

`grasm run` accepted it. The native backend refused with "does not yet support
a write to field `value` of this type". [T-REF-VALUE] had been implemented where
a place *starts* at a reference, and a reference reached through a list or a
struct field is met halfway down `write_place` instead — a second arm, which the
first patch did not know about.

A refusal rather than a miscompile, so it is S2 and not S1: the compiled build
declines to build instead of building something that disagrees. But it is still
two engines disagreeing about what is a legal program, and the conformance
program did not catch it because it only wrote through a reference held in a
local.

**Fixed**, and the conformance program now writes through a list element and a
struct field as well. The general lesson is the same one F17 taught: this
backend handles a place at more than one site, and a rule added at one of them
is not added to the language.

## F20 — `run_program` captures both streams, so there can be no pipeline (S4, library)

Found writing 17-simple-shell after [L-LIB-PROC] unblocked it. `run_program` waits for
the program to finish and hands back its output, which is the right default for
a builtin and the wrong primitive for a shell: a pipeline needs one child's
stdout to *be* another's stdin, and redirection needs a child's descriptor
pointed at a file. Neither is expressible when the only thing a caller gets is
a string after the fact.

So the shell has no `|`, no `>`, no `<`, no background `&` and no job control.

The shape of an answer is a second builtin that starts a program without waiting
— returning a handle the way `tcp_listen` does — with the streams as handles
`net_wait` already understands. That would make a pipeline an ordinary program
rather than a special form, and it is the same primitive F14 needs. It is
deliberately not a widening of `run_program`: a call that sometimes waits and sometimes
does not is two functions with one name.

## F19 — an integer cannot be formatted in any base but ten (S4, library) — **fixed**

Found writing 25-dns-resolver. A `bytes` prints as hex and an `i64` prints as
decimal, and there is no way to ask for a different radix. Every DNS tool prints
a packet ID as `0x1234`; doing that in Grasm means

```grasm
fn hex_digit(n: i64) -> str {
    "0123456789abcdef".substr(n.bit_and(15), 1)
}
```

Small, and it appears in every program that touches a protocol, a hash, a colour
or a permission mask — the same programs that needed [L-LIB-BITS], which is the
tell.

**Fixed** — `n.to_base(b)` and `s.parse_int_base(b)` ([L-LIB-BASE]). Both
directions, because `parse_int` refuses `0xFF` ([L-LIB-PARSE]) and a language
that can write hex and not read it back has only done half the job. Neither
side accepts a prefix: the base is the argument, not something to be guessed
from the text.

A base outside `2..36` panics rather than answering, for the same reason
`code_at` panics out of range — there is no right answer and a wrong one would
be silently wrong. `i64::MIN` converts through its unsigned magnitude, which is
the place a hand-written version is usually wrong.

Project 25's `hex_digit` table is gone; it now writes `n.to_base(16).pad_start(2, "0")`
and its output is byte-identical.

## F17 — a native-backend catch-all silently miscompiled a new method (S1, backend) — **fixed**

Found adding `byte_len` for F2. `borrowed_read` in the native backend chooses
the runtime function to call in one table and the *type it returns* in another,
and the second ended in

```rust
(VType::Str, _) => VType::Str,
```

So `byte_len`, which returns an `i64`, was typed as a boxed string. The compiled
program called `.len()` on the integer 10 as though it were a pointer and took
SIGSEGV, while `grasm run` printed the right answer: a crash *and* an engine
disagreement, from one missing row.

The literal test case passed, because a literal receiver takes a different path.
It only appeared on a receiver that was a loop variable — which is why it was
found by writing a real program and not by the unit test written alongside the
feature.

**Fixed** — the catch-all is gone and every method is named. An unlisted method
now falls through to the owning path, which is slower and correct, instead of
being miscompiled. This is the general shape: a permissive fallback in a table
that must agree with another table is a defect waiting for the next addition.

## F18 — two gate programs asserted on wall-clock time (S2, tests) — **fixed**

Found when `ProjectGold/run.sh` failed twice in six runs on code that had not
changed. 15-sorting-benchmark asserted that bubble sort's *elapsed time* grew at
least 3x when `n` doubled, and 30-async-chat waited a fixed 200 ms for a message
and carried on regardless. Both are true on an idle machine and neither is true
on a busy one.

A gate that fails when the machine is busy is worse than no gate, because it
teaches you to re-run it — and re-running is exactly what you must not do when
it fails for real.

**Fixed** — 15 now counts *comparisons*, which is what the complexity claim is
actually about and is identical on every engine and under any load; the clock is
still read (that is what charges `main` with `io`) but nothing is asserted about
how long anything took. 30 now waits until it has actually handled an event
rather than for a fixed interval. Six consecutive gate runs, clean.

## F13 — every socket operation blocks (S3, library) — **fixed**

Found writing 30-async-chat. `tcp_accept` and `tcp_read` both block, and there
is no `select`, no poll, no timeout, and no way to ask whether a handle is
ready. A single-threaded server can therefore serve exactly one connection at a
time — which is what project 21 does, correctly and deliberately.

Multiple simultaneous connections would need either non-blocking handles with a
readiness check, or a thread per connection. `spawn` is documented as
single-core ([E-NOT-PARALLEL]) and `par_map` runs a fixed batch to completion,
so neither gives a connection its own thread of control today.

### What it turned out to be

The design question was smaller than it looked, and asking it the other way
around answered it. A thread per connection is *unavailable by construction*:
`spawn` does not carry `net`, so a spawned task cannot touch a socket, and that
is [T-CAPABILITY] working as designed rather than a limitation to route around.
So the answer had to be readiness, and readiness is a builtin.

`net_wait(handles, timeout_ms)` was added ([L-LIB-NET]): it answers with the
subset of handles that can be read without blocking, in the order they were
asked in. The listener goes in the same wait set as the connections, so a new
arrival and a message from an existing client are the same kind of event. One
thread now multiplexes any number of connections, which is what every real
server does underneath. 30-async-chat is no longer blocked, and its
demonstration dials its own clients from the thread that then serves them —
which only runs at all because nothing blocks.

Adding it also exposed a smaller gap on the way: `tcp_listen(0)` asks the system
to choose a port, and nothing could read the choice back, so that spelling could
not be used for anything. `tcp_port(h)` was added with it.

This is readiness, not asynchrony. `tcp_read` on a ready handle still performs a
blocking read; it simply cannot block, because the bytes are already there. The
`async` row still exists in the vocabulary with nothing performing it, and F14
below — that `spawn` cannot perform I/O at all — is untouched.

## F14 — there is no way to do concurrent I/O (S3, language) — **half fixed**

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

### The I/O half, fixed — and it did not need threads

The finding conflated two things that turned out to be separable: doing many I/O
operations at once, and doing computation on many cores. Only the second needs
threads.

`net_wait` ([L-LIB-NET]) closed the first. One thread holds every request open
at once: connect and send them all, then read whichever is ready. Project 22
does that now, and its demonstration runs a server *and* six clients from a
single wait loop, which a blocking accept would deadlock — so the transcript is
the evidence. Project 30 was unblocked the same way. Neither needed a thread,
and looking for one was the wrong instinct: the thing a sequential fetcher gets
wrong is not that it uses one core, it is that it blocks on the first response.

### The compute half, still open

`par_map` still refuses an effectful closure, and it is still right to: results
would interleave differently between runs and it promises a deterministic
answer. A program that wants to fetch on many cores *and* parse on many cores
still cannot, and MASTERPLAN G6's "structured, uncolored concurrency" is still
what `docs/northstar.md` records it as — *retreated*.

The shape of that fix is unchanged and the language already contains it. `async`
is in the effect vocabulary and nothing performs it. An I/O operation that
performs `async`, a handler that decides what that means, and `par_map`'s
refusal narrowed from "any effect" to "any effect that is not `async`" fits the
existing type system, and is close to how WebAssembly's stack-switching proposal
is specified — the same bet `docs/wasm.md` argues for.

What is different is the size of the prize. The concrete programs that motivated
this finding are written; what remains is a real capability and no longer a
blocker.

## F15 — a wasm trap poisons the instance, and the docs did not say so (S4, docs) — **fixed**

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

## Correction to F9 — it blocks code *generation*, not self-hosting

Recorded here because it reverses a conclusion this file previously supported.

F9 (no byte type) was read as "Grasm cannot host its own back end", on the
strength of a real demonstration: asked to write seven bytes of x86, the
language put nine bytes of mangled UTF-8 on disk, because `str` re-encodes
anything above 127.

That demonstration is correct and the conclusion drawn from it was too wide.
It blocks **direct machine-code emission**. It does not block a back end that
emits **C**, which is the route Nim, Vala, Haxe and the original Go frontend
take, and which needs only text.

Project 31 tests it rather than arguing it: a compiler written in Grasm emits C,
`cc -Wall -Wextra` accepts it without a warning, and the resulting native binary
agrees with the interpreter. So the self-hosting path is open today, and F9's
real cost is narrower than recorded — it blocks reading a BMP (18), building a
DNS packet (25), and serving binary over a socket, which are what it was
originally found for.
