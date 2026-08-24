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
different route — `proc_run` under the `proc` capability ([L-LIB-PROC]) —
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
`read_file`, `write_bytes`, `tcp_listen`, `udp_bind`, and now `proc_run` —
and the bare verbs a program is likely to want for its own functions stay
available. A test asserts that: `run`, `send`, `open`, `close`, `read`, `write`,
`start`, `stop`, `wait`, `execute` and twenty more must never appear in the
builtin list.

## The region rule, attacked and holding

Not a finding — a confirmation, recorded because the claim it confirms is
load-bearing. MASTERPLAN §8.3 calls Tier 1's escape rule "airtight as of this
round" after four adversarial passes, and Tier 1 is the one tier that is only
half in: the rule is enforced, the bulk-free arena it exists to make sound is
not. If the rule leaks, the deferred half is unbuildable on it.

Nineteen routes out of a region, every one refused **by the region rule**:
assigning to an outer binding, pushing into an outer list, `return` through the
brace, yielding a list, writing through a `ref` made outside, an `inout` call,
a `ref` one field deep in a struct, an effect handled outside, a closure bound
outside, `try` carrying an `Err` payload, an effect through a one-line helper,
a closure taken from a list, from a struct field and from a map, a `ref` from a
list and from an `Option`, a generic `inout`, a UFCS-spelled storing helper,
and a qualified cross-module call — the last being the shape MASTERPLAN records
as a real hole once, where the effect row was read under the calling module.

### The first pass counted the wrong thing

It reported all eighteen attempts refused. But it read the *exit code*, and a
program that does not compile exits non-zero whatever the reason — several
attempts had a two-statement closure body on one line, which is a syntax error
in this language. Those were counted as the rule working.

Re-run reading the error *message*, the tally was six by the region rule and
one by "nothing says what `xs` is". Annotating the list literal turned that one
into `a function value cannot be used inside a region`, which is the rule.

This is the same defect as F25, F27 and F28, committed by the checker rather
than found by it: a result that reports on something it cannot distinguish. It
took twenty minutes to write the probe and an hour to make the probe honest.

`T-REGION-FOREIGN` now covers the effect reached through a wrapper and the
closure taken out of a container, which were the two indirections the earlier
adversarial rounds found and no program pinned.

## F31 — two more clauses promised a panic no program triggered (S2, tests) — **fixed**

After F29 and F30, both of which were "the first index past the end", the
obvious question was how many other clauses promise a panic and never
demonstrate one. Four operations are specified to panic out of range:
`xs[i]` and `s.code_at(i)` — already fixed — plus `b.at(i)`/`b.slice(...)`
([L-LIB-BYTES]) and `xs.remove(i)` ([L-LIB-REMOVE]).

Both remaining programs were `expect: output`. Neither triggered the panic its
own clause promises. Both now read the last valid element *and* one past it.

The bytes boundary kills its mutant: with `i >= len` changed to `i > len`,
`grasm run` panics and the compiled build dies with SIGABRT.

**The `remove` boundary does not, and the reason is worth writing down.**
`grasm_list_remove` checks the index and then calls `grasm_list_get`, which
checks it again — so the outer check is redundant in the compiled build and
mutating it changes only the panic *message*. The program still panics, still
exits 101, still prints the same thing on stdout.

Which exposes the real gap: **no broad gate checks a compiled build's panic
message.** `difftest.sh` compares stdout and the exit code, and a panic goes to
stderr; the conformance suite runs the interpreter, so a runtime defect is
invisible to it entirely. [E-PANIC-TRACE] says the panic line is identical in
all three engines, and the only gate that verifies that is `agreetest.sh` — on
sixteen programs.

### Fixed after all, in the gate built for it

Widening `difftest.sh` to compare stderr would make 218 programs assert their
own stack traces — a much larger promise than the clause makes. But
`agreetest.sh` already captures stderr from all three modes, strips the trace
block, and compares everything else including the panic line. It is the gate
for this, and it was checking six panic shapes when the language has fourteen.

Eleven more programs: indexing at exactly `len`, `code_at`, `bytes.at`,
`bytes.slice`, `remove`, `to_char` on a non-code-point, `to_base` and
`parse_int_base` on a bad base, a negative exponent, a failed `assert_eq`, and
a stale `ref`. Sixteen programs became twenty-seven, and all agree.

The `remove` mutant is the proof: it slipped past every gate when this finding
was written, and now fails `panic_remove` — `bundle (101): panic: remove 2 out
of bounds (len 2)` against the interpreter's `remove 1`. A redundant check
whose only observable effect is what the panic says is now observable.

## Mutation testing — the whole tally

Seven deliberate defects, each the smallest wrong thing its function could be,
with `difftest.sh` as the oracle.

**Caught, no action needed**

* map iteration order reversed — [E-MAP-ORDER]'s program sees it at once
* `to_base`'s digit table wrong above 9 — decimal unaffected, hex changes, and
  a program checks hex

**Genuine holes, found and fixed**

* a list index accepted at exactly `len` (F29)
* `code_at` accepted at exactly `len` (F30)

**Survived, and were not holes** — three of the four survivors

* the boxed-list bounds check: the backend emits its own inline check first
* `char_at` one past the end: `grasm_str_code_at` bounds-checks before calling it
* `substr`'s end clamp: `byte_at(n + 1)` answers `None` and falls back to the
  string's length, so clamping one further gives the identical answer

**Three quarters of the survivors were equivalent mutants**, which is the
result worth keeping. This runtime checks the same thing at more than one
depth, so a mutation in an inner check is invisible *by construction* — not
because a gate is missing. Every one of those three would have gone into this
file as a coverage hole if "no gate noticed" had been treated as sufficient.

So the method has two rules, both learned the hard way here: mutate the
**outermost** decision on a path, and prove a survivor is **observable** before
believing it. Both real holes were the same shape — the first index that is out
of range — and both conformance programs tested well past the end instead of
just past it.

## F30 — `code_at` panics out of range, and nothing tested that (S2, tests) — **fixed**

Found by the second round of mutation testing. `[L-LIB-CODES]` says
`s.code_at(i)` "panics out of range"; the conformance program for it checked
code points, the round trip through `to_char`, a hash built from them, and
non-ASCII counting — and never once indexed out of range.

Changing the check from `i >= n` to `i > n` left every gate green while
`s.code_at(s.len())` read one character past the end: `grasm run` panicked with
exit 101 and the compiled build died with **SIGABRT, exit 134**. The program now
reads the last valid code point and then exactly `len`.

### What the round taught about the method

Two of the four mutations in the first batch "survived" and neither was a hole.
`char_at` accepting one past the end is invisible because `grasm_str_code_at`
bounds-checks before calling it; the boxed list index check in F29 is invisible
because the backend emits its own inline check first. This runtime has defence
in depth, so **mutating an inner check proves nothing** — the outer one already
decided.

The rule that came out of it: mutate the *outermost* decision on a path. Every
survivor since has been checked against that before being believed, which is
what turned "no gate noticed" from a claim into a finding.

The map-ordering mutation was caught by `difftest.sh` immediately, which is the
other half of the result: the gates are not blind in general, they were blind at
two boundaries.

## F29 — the bounds test never tested the boundary (S2, tests) — **fixed**

Found by mutation testing: break the compiler on purpose, then ask which gate
notices. Changing a list index check from `i >= len` to `i > len` — so
`xs[len]` reads one element past the last instead of panicking — is the
smallest wrong thing a bounds check can be.

`E-INDEX-BOUNDS` read `xs[5]` on a list of two. Five is past the end by three,
so it panics either way and the mutation survives: comparing what `difftest.sh`
compares — stdout and exit code — both engines print nothing and exit 101.

    old  xs[5] on len 2       interp 101, native 101   mutation survives
    new  xs[xs.len()]         interp 101, native 134   mutation caught

A bounds check is exactly the place where only the boundary is worth testing.
The program now indexes at *precisely* `len`, on both list representations —
a `[i64]` is stored unboxed and a `[str]` is a list of pointers, which are two
different index paths — and checks the last valid index of each and a negative
one on the way.

### Three wrong measurements before a right one

The technique exists to check whether measurements mean anything, and getting
it working took three corrections:

* The first mutation to *survive* was in `grasm_list_get`, the boxed path. It
  survives because the backend emits its own inline bounds check first, so the
  runtime one is a second line of defence and mutating it changes nothing
  observable. An **equivalent mutant**, not a coverage hole — and it would have
  been reported as one.
* Verifying with `cargo test -p grasm_spec` reported nothing, because the
  conformance suite runs the **interpreter** and the mutation was in the
  runtime, which only compiled builds link. That suite cannot see a runtime
  defect at all; only the gates that compile can.
* Comparing with `2>&1` made every case look caught, because the interpreter
  prints a stack trace and the native build does not — a difference that is
  there with or without a mutation. `difftest.sh` compares stdout and the exit
  code, and the check has to compare the same things.

## F28 — the conformance suite cannot test a multi-module program, and two module clauses pretended otherwise (S3, tests) — **fixed**

Found probing modules for engine disagreements. There were none: ten shapes —
a struct type declared in one module and constructed in another, a constant, a
sum type matched across the boundary, a generic instantiated by the caller,
`inout` writing through the boundary, a function value taken from another
module, imported structs in a map, a private function reached only through its
module's public one, and an effect declared and performed there but *handled*
here — all agree across the interpreter, the bundle and the native build.

The defect is in what was checking them. `spec/conformance/<TAG>.gr` is **one
file**, and the harness compiles one file, so the suite structurally cannot
express a program with two modules. Two clauses that are entirely about
crossing that boundary had programs that did not:

* `T-MODULE-NS` tests only the *error* — two types of the same name in one
  file — and never that a type declared in one module is nameable, without a
  prefix, from another. Which is the rule.
* `T-MODULE-PUB` is a single-file program calling a private function *in the
  same file*. `pub` does nothing there. The program passes whether or not
  cross-module visibility works at all.

**Fixed** where it can be: `examples/multi` is a real two-module program that
`difftest.sh` already runs through all three engines, and it now exercises all
ten shapes. The conformance programs stay as they are — they test what a single
file can test — and the limitation is recorded here rather than left to look
like coverage.

The general shape is one this file keeps finding: a check that reports on
something it cannot see. F25 was three checks that never ran; F27 was a timer
measuring itself; this is a suite whose file format cannot hold the thing two of
its clauses are about.

## F27 — the benchmark harness added 21 ms to every measurement it ever took (S2, benchmarks) — **fixed**

Found while trying to explain why `sieve` was 37 ms in `bench.sh` and 16 ms
when timed directly. Both were the same binary.

`time_ms` read the clock by launching `python3` on either side of the program
it was timing:

```sh
start=$(python3 -c '...perf_counter...')
"$@" >"$WORK/out"
end=$(python3 -c '...perf_counter...')
```

The **second** interpreter's own startup lands inside the interval. On this
machine that is 21 ms, added to every cell of every table this script has ever
printed. The C sieve takes about 5 ms and was reported at 30.

A constant added to every measurement is the worst error a benchmark can have.
It does not look like noise, it survives repetition, and it **compresses every
ratio toward 1** — so it flatters whatever is slowest. Grasm is about 2 to 2.4x
C on the scalar programs, not the 1.2x the published table implied. The
correction is not uniformly unflattering: `sort` is a much better result than it
looked, at roughly half C's time rather than a tenth under it.

**Fixed** — one `python3` that spawns the program and times it, so the only
thing in the interval is the program.

### Two gates were watching and neither could see it

`bench.sh` already refuses to report a timing for a wrong answer, and refuses to
run at all on a busy machine. Both are checks on the *inputs* to a measurement.
Nothing checked the timer itself against a program of known duration, and the
published table had been *reproduced* — two runs hours apart agreeing within
3 ms, which is exactly what a harness with a constant offset produces.
Reproducibility is necessary and not sufficient.

Both tables are withdrawn rather than corrected in place, because a number
edited by hand is not a measurement, and `bench.sh` has refused every run since
on load.

## F26 — `bytes.is_empty()` could not be compiled at all (S1, backend) — **fixed**

Found by `scripts/fuzz_methods.py`, written for this purpose: it crosses every
builtin method with every *shape* of receiver — literal, `let`, `var`, loop
variable, struct field, list element — because F17 only appeared on a loop
variable and passed every test written against a literal.

The grid found something simpler and worse. **Every** `b.is_empty()` in the
language failed to compile, on every receiver shape:

```rust
let eq = self.b.ins().icmp(IntCC::Equal, n, zero);
(self.b.ins().uextend(types::I8, eq), VType::Bool)
```

`icmp` already answers in `i8`, which is what a `bool` is, so the `uextend`
widens a value to the width it already has — which Cranelift's verifier
rejects. Every other `is_empty` in the backend calls a runtime function that
returns `i8`; this one was written inline and got it wrong.

**Fixed** by deleting the `uextend`, and the conformance program for
[L-LIB-BYTES] now calls the method.

### The gate was reporting it as an honest refusal

Worse than the defect: the backend distinguishes "does not yet support X" —
a named, documented gap — from "codegen bug", which is the compiler reporting a
verifier failure against itself. The first version of this fuzzer counted both
as declines, so it printed six of these and called them "not a defect".

The fuzzer now treats "codegen bug" as a failure. That distinction was already
in the backend's own error text and in the comment beside it; the gate simply
was not reading it.

## F25 — three checks in `nativegaps.sh` had never run (S2, tests) — **fixed**

Found while running the gap gate after F24. Three call sites spelled the helper
`expect_compiles`; the function is `expect_compiled`. The shell printed
`command not found`, the script does not use `set -e`, and nothing set the
failure flag — so the gate passed while three of its checks did nothing.

A gate that silently skips part of itself is worse than one that is missing it,
because the report says the check ran.

**Fixed** — the names match, all five `expect_compiled` checks run, and all
pass.

## F24 — `inout` through a place ran interpreted and would not compile (S2, backend) — **fixed**

Found by `scripts/fuzz_places.py`, written for this purpose: it enumerates the
shapes a place can take — local, field, element, field of an element, list in a
struct, reference at the root, reference in a field, reference in a list — and
crosses them with read, write, compound assignment, and the mutating list
methods. Eighty-four programs, interpreter against native.

Everything agreed except three, and all three were the same thing:

```grasm
fn bump(inout ys: [i64]) { ys.push(1) }
bump(s.xs)      // runs under `grasm run`, refused by the native backend
```

The `inout` ABI passes a pointer to the caller's slot, which exists for a local
and not for a place inside something. `grasm run` reads the place, calls, and
writes back — which is exactly what the mutating methods already do through a
place, so the machinery was there and this one path did not use it.

**Fixed** — the backend reads the place (taking rather than copying, so an
`inout` through a place is not O(n) in what it points at), passes the slot's
address, and writes back through `write_place`. Twenty thousand iterations
through a struct field, a nested field and a list element: leak-free.

It was a refusal and not a wrong answer, so S2. It was also a *documented* gap —
`docs/tutorial.md` §11 listed it — which is the honest way to carry a hole, and
`scripts/nativegaps.sh` now asserts it compiles instead.

## F23 — a C function reached any way but directly was handed the wrong pointer (S1, backend) — **fixed**

Found by extending `scripts/fuzz_extern.py` to the pointer boundary, which is
the newest and most dangerous surface in the language and had exactly one
conformance program covering it — one that only ever called `strlen(s)`.

[L-UFCS] says `s.strlen()` is the same call as `strlen(s)`, and a function value
is the same function. The pointer conversion lives at the *call site*, so every
other spelling is a place it can be missed. Two were:

* `s.strlen()` — the UFCS branch pushed the receiver unchanged;
* `measure(strlen, s)` and `[s].map(strlen)[0]` — the wrapper that lets a named
  function be used as a value forwarded its parameters unchanged.

Both handed C **the boxed value's own address**. `strlen` walked an `RtVal`,
found a zero byte one in, and answered `1` for every string in the language,
while `grasm run` answered correctly. A wrong answer and an engine disagreement,
from reading memory that was never the string — and with a different heap layout
it reads until it finds a zero, wherever that is.

**Fixed** in both places, with the temporaries freed and the boxes released on
each new path (20,000 iterations through all four spellings, leak-free). The
conformance program now exercises every spelling, and the fuzzer checks
`strlen(s) == s.byte_len()` for strings on both sides of the fifteen-byte inline
boundary and both sides of ASCII, on both engines.

The lesson is the one this file keeps recording, now three times over (F17, F22,
this): **a rule added at one site has not been added to the language.** What is
new is the method. F17 and F22 were found by writing a program that happened to
hit the second site; this was found by making the *test* enumerate the sites.

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

## F20 — `proc_run` captures both streams, so there can be no pipeline (S4, library) — **fixed**

Found writing 17-simple-shell after [L-LIB-PROC] unblocked it. `proc_run` waits for
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
deliberately not a widening of `proc_run`: a call that sometimes waits and sometimes
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

## F14 — there is no way to do concurrent I/O (S3, language) — **mostly fixed**

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

### The compute half, half of it

There were two limits here, not one, and only the second was ever written down.

The first was silent: `par_map` and `par_filter` ran on several cores **only
when every element, capture and result was a machine word**. Anything boxed — a
string, a list, a struct — fell back to a sequential `map` with no diagnostic,
because a worker touching a boxed value would race on a `Cell<i64>` reference
count. So "parse on many cores" was not refused, it just quietly did not
happen, which is worse.

That one is closed. The count is an `AtomicI64` now, which was measured before
it was taken: about half a percent on the geometric mean of the benchmark set,
4.3% on `tree` and nothing anywhere else. Boxed elements, boxed captures and
boxed results all run on every core — 8,000 strings through a
concatenation-heavy function took 128.5 ms as a `map` and 34.9 ms as a
`par_map`, **3.68× on eight cores**. `docs/northstar.md` carries the numbers.

The second limit is unchanged and still right: `par_map` refuses an effectful
closure, because results would interleave differently between runs and it
promises a deterministic answer. A program that wants to **fetch** on many
cores still cannot, and MASTERPLAN G6's "structured, uncolored concurrency" is
still what `docs/northstar.md` records it as — *retreated*.

Note for honesty about this project: the `par_map` at the bottom of
`22-multithreaded-scraper` has seven elements, and the threshold for spawning
workers is 256. It now takes the parallel *path* where before it took the
sequential fallback, and at seven elements it runs on one core either way. It
demonstrates the ordering promise, not the parallelism.

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

## F32 — the committed wasm module was four months behind the language (S2, tooling) — **fixed**

Found writing phase 2's project 2, whose whole claim is that the browser runs
the *server's own* validation rules: it fetches `main.gr` and executes it
through the wasm build, so there is one copy of the rules rather than two that
drift.

The browser refused the file.

```
<browser>:214:19: error: unknown name `eprint`
<browser>:260:81: error: no method `byte_len` on `str`
```

Both builtins had worked in every command-line engine for weeks. The module in
`ProjectGold/web/grasm.wasm` was built on 18 August and never rebuilt, and
nothing compared it against the compiler beside it. `scripts/wasmtest.sh` — the
gate whose whole subject is the wasm engine — tests a **fresh** build, so it
passed the entire time.

An artifact that is committed and never checked is a fourth engine nobody is
running. The gate now compares every committed `grasm.wasm` against a fresh
build and fails on a difference, with the refresh command in the message.

### And the same drift in the other direction

Rebuilding turned up two conformance programs reported as *mismatches* rather
than skips: `L-LIB-BYTES` and `L-LIB-PROC`. The harness carries a list of what
`wasm32-unknown-unknown` cannot do — threads, sockets, a filesystem — and that
list had not grown when the library did. `write_bytes` and `proc_pipeline`
arrived after it was written, so programs correctly *refusing* on a target with
no filesystem and no processes were being counted as disagreements.

The list now covers `par_filter`, `spawn`, `tcp_connect`, `udp_bind`,
`net_wait`, `read_bytes`, `write_bytes`, `list_dir`, `make_dir`, `proc_run` and
`proc_pipeline`. `wasmtest.sh`: 181 identical, 0 mismatched, 38 skipped.

Two defects of one shape: a harness's model of the world stops being updated,
and it goes on reporting success.

## F33 — the wasm engine cannot run a program that has an `import` (S3, tooling) — **fixed**

Found writing phase 2's project 3, a live markdown editor. The design was the
one project 33 had already proved: the browser fetches `main.gr` and runs it
through `grasm_run_with_stdin`, so the preview is compiled by the server's own
compiler and the two cannot disagree about what a document means.

It returned nothing. The reason is structural rather than a bug:

* `grasm_run_with_stdin(src, len, stdin, len)` takes **one source string**.
* `wasm32-unknown-unknown` has **no filesystem**, so an `import` cannot be
  resolved by reading the file it names.

Project 33 worked because it is a single file. Project 36 imports
`lib_markdown` — which is the entire point of it, and of the package extraction
in project 5 — so the browser is out.

This is larger than one project. Every Grasm program worth writing has imports,
and the fourth engine can run none of them. The browser showcase works only
because all 31 phase-1 programs happen to be single files, which was never a
design decision.

### What would close it

`grasm build --source -o one.gr`: the loader already resolves the whole import
graph and holds every file, so emitting one source with module members
qualified is a pass over a structure that already exists. The browser then
fetches one file and the existing ABI is enough.

An alternative — extending the ABI to accept named modules — puts the resolver
in the embedder, which is the wrong place for it.

### Fixed: `grasm build --source`

A **source bundle** is a whole program as one `.gr` file — every module it
imports, transitively, laid end to end behind `//!module <name>` lines, entry
first. Nothing is renamed and nothing is rewritten, so the modules a bundle
loads are the modules the disk would have loaded and a reader can still find the
code they are looking at.

It needed no new machinery. The disk loader already takes a `provider:
&dyn Fn(&str) -> Option<String>` — a module name to its source — so a bundle
loads by looking up in a table instead of reading a file, and every other
decision about imports, aliases and spans is made by the same function.

Detection sits in `load_from_disk_in`, so `run`, `build`, `check`, `test` and
the language server all open a bundle without knowing the format exists. The
wasm engine's single-source entry checks for it too, which is the point.

    $ grasm build main.gr --source -o bundle.gr
    wrote `bundle.gr`: 2 module(s), 16777 bytes

Project 36's preview is back on the compiler it claims: the page fetches
`bundle.gr`, runs it in wasm, and the HTML it shows was diffed against the
HTML the server publishes for the same document — identical, byte for byte.

An earlier draft of that page asked the server to render on every keystroke,
which was honest and worse. It is kept in the history rather than the file,
because what a project settles for when a tool is missing stops being
interesting once the tool exists.

## F34 — there is no source of randomness (S3, library) — **fixed**

Found writing phase 2's project 7, which is authentication: a password hash, a
session token, and a route that refuses without one.

Grasm has `now_millis()` and nothing else. There is no `random()`, no
`random_bytes(n)`, and no way to reach the operating system's entropy without
`extern fn` — which needs `unsafe`, a C library on the link line, and is
therefore not available to an ordinary program.

The two halves of that project land on opposite sides of it.

**A salt survives.** Its job is *uniqueness* — two accounts must not share one,
so that a rainbow table cannot be reused and two identical passwords do not have
identical hashes. A clock, a counter and the address give that, and the project
checks it: eight salts for one address, all different.

**A session token does not.** A token is exactly a secret that must not be
guessable, and one derived from the clock is guessable by anyone who knows
roughly when the session began — a search over a few million millisecond values
against a live endpoint. The project issues them anyway, says so in its first
paragraph and again in its README, and is explicitly not something to deploy.

This is not only about authentication. Without randomness a program cannot
shuffle, cannot pick a nonce, cannot make a UUID, cannot sample, and cannot seed
its own property tests independently of the harness.

### Fixed: `random_bytes(n)` ([L-LIB-RANDOM])

`n` bytes from `/dev/urandom`, charged `io` — because it reaches outside the
program and two calls do not agree, which is the same reason the clock is
charged.

`/dev/urandom` rather than a userspace generator: it is seeded by the kernel, it
needs no reseeding, and a language with no audited crypto has no business
shipping its own. A short read is a failure rather than a short answer, because
a caller asking for 32 bytes of key material and getting 12 must not proceed.

**The primitive is bytes, deliberately.** Everything else is built from it and
the conversions are where the mistakes live: a number in a range wants rejection
sampling, not a modulo, and a language that shipped `random_int(lo, hi)` with a
modulo in it would have shipped a bias. `L-LIB-RANDOM.gr` builds a fair one in
Grasm to show what the primitive is for, and explains in a comment why `b % 5`
makes 0 and 1 come up 52 times in 256 and the rest 51.

`wasm32-unknown-unknown` has no filesystem, so the wasm engine refuses it the
way it refuses `net` — a refusal, never a wrong answer.

### Project 37's tokens are no longer guessable

The finding's own example is closed. Session tokens were
`sha256("token:" + now_millis() + ...)`; they are 256 bits of kernel entropy
now. And if entropy is unavailable, `login` answers **500 rather than issuing
one** — a token that might be guessable is worse than an error, because it looks
exactly like a good one.

The salt still uses the clock, and that is correct rather than left over: a
salt's job is uniqueness, not unpredictability.

## F35 — a `capabilities = []` package could read and write any file (S2, soundness) — **fixed**

Found by auditing the builtin lists against each other while adding a
randomness primitive, not by any harness. Nothing was failing.

`read_bytes` and `write_bytes` arrived with [L-LIB-BYTES], beside `read_file`
and `write_file` — and were left off `IO_BUILTINS`, the list that decides which
builtins charge `io`. Everything downstream reads that list, so:

```grasm
fn sneaky() -> i64 / {} {          // declares no effects at all
    match read_bytes("/etc/hosts") {
        Ok(b) -> b.len()
        Err(e) -> -1
    }
}
```

compiled, ran, and answered `212`.

The package-level consequence is the serious one, because it is the property
[T-CAPABILITY] exists to provide:

```
a capabilities=[] package read 212 bytes off the disk
```

A dependency declaring **no capabilities** could read any file the process
could, and write any file too. The supply-chain benchmark that G7 rests on
tests a package that *prints* — and printing was charged correctly, so the
benchmark passed while the hole was open. A gate that tests one member of a
class and a class whose membership is a hand-maintained list is a gate that
will eventually miss one.

Fixed by adding both to `IO_BUILTINS`. Both breaches are now refused, at the
function's row and at the package's ceiling.

### The gate that would have caught it

`spec/conformance/T-EFFECT-BUILTIN.gr` now **names every builtin that reaches
outside the program** inside one `/ {io}` function. That function never runs:
mentioning a builtin in a row that declares `io` is what makes the checker
charge it, so if any one of them stopped being charged the file would fail to
compile rather than silently permit it.

That is the general shape of the defence — enumerate the class in a program
rather than trust a list in the compiler — and it is worth applying to `net`,
`proc` and `unsafe` the same way.

### What this says about the scorecard

`docs/northstar.md` recorded **G7 as "met, for what exists"**. It was not met,
and had not been since `bytes` shipped. The row now says so, because a
scorecard that is optimistic about the one property the language most wants to
be trusted on is worse than no scorecard.

## F36 — the wasm gate compared a module that no longer built (S1, gate) — **fixed**

`scripts/wasmtest.sh` read `target/wasm32-unknown-unknown/release/grasm_wasm.wasm`
and compared it against the interpreter. It never built it.

So when `grasm_interp` grew a dependency on `grasm_rt` — done so the interpreter
could share the record-and-replay journal — the wasm target stopped compiling
entirely, and the gate went on reporting:

```
identical: 182   mismatched: 0   skipped: 39
```

against a module from the previous week. Two full gate runs passed with the
browser build broken.

The staleness check that already existed made this worse rather than better. It
compares the *committed* copies in `ProjectGold/*/grasm.wasm` against the module
in `target/`, and found them equal — because both were the same stale build. A
freshness check whose reference is itself stale reports freshness.

`grasm_rt` is written for a 64-bit host: `std::os::fd`, `1 << 33`, and a
`size_of::<RtVal>() == 64` assertion that a 32-bit target fails at compile time.
It cannot be a dependency of anything the browser build reaches.

### Two fixes, and the second is the one that matters

**The journal moved to its own crate.** `grasm_journal` is a format — an
encoding three separate value representations agree on — and not a runtime. It
has no dependencies and no host assumptions. `grasm_rt` re-exports it as
`replay`, so every existing path is unchanged.

**The gate builds the module it is about to test**, and fails if the build
fails. Confirmed by breaking the wasm crate on purpose:

```
error: could not compile `grasm_wasm` (lib) due to 1 previous error
error: the wasm build failed — the module below would be stale
gate exit = 1
```

### The general shape, and the audit it prompted

*A gate that reads an artefact must produce that artefact.*

The sentence written here first was "every other gate in this repository builds
what it tests" — and it was wrong, which is why it is quoted rather than
deleted. **None of them did.** Every gate opened `./target/release/grasm` and
none of them built it, so the identical failure was available to all nine:
break the compiler, leave yesterday's binary on disk, and `difftest.sh` reports
221 identical against a compiler that no longer compiles.

`scripts/lib/fresh.sh` now stands at the top of all nine. Cargo is a no-op when
nothing changed, so the whole class costs a tenth of a second per gate. It is
skipped when the caller named its own binary through `GRASM=`, because then the
artefact under test is theirs.

Confirmed by breaking `grasm_sema` on purpose:

```
$ bash scripts/agreetest.sh
error: could not compile `grasm_sema` (lib) due to 1 previous error
error: the workspace does not build — every result below would be
       measured against a stale binary, so there are none.
agreetest exit = 1
```

The lesson is not about wasm. It is that a gate reporting a number is making a
claim about *the code as it stands*, and reading an artefact it did not build
is the one way to make that claim without checking it.

## F37 — a recorded `()` came back as `0` on another engine (S1, replay) — **fixed**

The compiled runtime answered `write_file`, `write_bytes`, `make_dir`,
`remove_file` and both socket writes with `Ok(0)`, where the interpreter answers
`Ok(())`.

No direct run could tell those apart. The static type is `()`, and a compiled
build prints unit from the type rather than from the value — so the shortcut had
been correct for as long as the value never left the engine that made it.

The journal is exactly a value leaving the engine that made it:

```
$ GRASM_RECORD=j ./program      # native
()
[(), ()]
$ GRASM_REPLAY=j grasm run p    # the interpreter, given that recording
0
[0, 0]
```

An engine disagreement, and so S1 — scoped to replay, because nothing else could
observe it.

`RtPayload::Unit` already existed for precisely this, and its own doc comment
names the same bug in a different place: *"boxing it as `I64(0)` made `[(), ()]`
print `[0, 0]` under a compiled build"*. Six call sites had not been converted.
They are now.

### The gate that would have caught it

`scripts/replaytest.sh` checked that every builtin's hook was still there. It did
not check that the value coming back through the journal was the value that went
in — because every program it ran printed `y` or `n` rather than what it got.

It now records one program that **prints the values**, including a bare unit and
a list of them, and replays it on all three engines that can replay, requiring
byte-identical output. Confirmed by putting `I64(0)` back:

```
2c2
< ()
---
> 0
journaled: 28   unhooked: 2
a native recording replayed elsewhere: DIFFERS under interp, wasm
```

The shape of the miss is worth keeping: a test that asserts *a call happened*
will not notice that *the wrong value came back*. Both gates were needed, and
the first one existing is what made the second easy to forget.

## F38 — the native backend had no declaration for `IoError` (S3, over-strict) — **fixed**

```
error: native backend does not yet support the sum type `IoError`
```

for this, which is the ordinary way to read *why* a file could not be read:

```
match read_file(path) {
    Ok(s) -> ...
    Err(e) -> match e {
        NotFound(p) -> ...
        IoFailed(m) -> ...
    }
}
```

The cause is small and the shape of it is not. `IoError` is in the **prelude** —
nobody declares it in source — and the backend filled its type table by walking
the declarations in each module. So there was never anything to find, and every
program that wanted to tell a missing file from a broken one was refused at
build time, in a language where every `io` call hands one of these back.

It cost twice in one session before anyone looked at it: a ProjectGold project
was restructured around it, and a conformance program carried a comment
explaining why it could not write the annotated form.

### Read from the checker's table, not copied into a second one

`grasm_sema::types::IO_ERROR_VARIANTS` is now the list, and both the checker's
prelude and the backend's type table are built from it. Copying it into the
backend would have fixed this program and left the next divergence to whoever
adds a third variant — the same reasoning as `builtins.rs` in **F35**.

Two gaps closed for one fix: `let r: str ! IoError = read_file(p)` — the
annotated binding — compiles now too, for the same reason it did not before.

Three engines agree on both variants, on a hand-built `NotFound("...")`, and on
the `_` wildcard arm.

## F39 — one wrapper, two meanings of "environment" (S1, would-be) — **caught before it shipped**

A named function used as a handler operation needs the `(env, params…)` shape
every function value has, and that wrapper already existed. Reusing it produced
a program that printed the right answer and then died:

```
$ ./t
2
exit=138          # SIGBUS
```

The wrapper's first act is to release the environment it was handed, because a
*function value's* environment is a reference it owns. A **handler
operation's** environment is the address of a stack slot in the frame that
installed the handler. The wrapper decremented a refcount through a pointer
into a live frame.

Two things about how it presented are worth keeping:

**The answer was right.** The operation ran, returned `2`, and the damage was
only read later. A test that checked the value and not the exit status would
have passed.

**It looked positional.** One named operation worked; a named operation at
field index 1 crashed; a named operation at index 0 with a second operation
after it crashed. That is not a real pattern — it is which stack addresses
happened to matter — and it sent the first two hypotheses (a lifting-order
problem, then emitting a function during the collection pass) in the wrong
direction. The second hypothesis was implemented and changed nothing, which is
what said the model was wrong rather than the code.

### The fix

`wrapper_for(key, span, owns_env)`, with `fn_value_wrapper` passing `true` and
`handler_op_wrapper` passing `false`, and separate cache keys so the same
function used both ways gets both wrappers. That last part is checked: `seven`
is called through a function value, installed as a handler operation, and
called through a value again, in one program.

### What it says about the shape of the bug

The two callers agreed on the *layout* of the ABI — a pointer, then the
parameters — and disagreed about **who owns the pointer**. Nothing in the type
of a `*const u8` says that, and both sides were written correctly against their
own reading of it. An ownership convention that lives only in a comment is one
the compiler cannot check, and this is the second time today that a mismatch
between what two sides of an interface knew produced a defect — the other being
the backend re-deriving what the checker had already settled.

## F40 — eleven gate cases that never ran, written by me, today (S1, gate) — **fixed**

`scripts/nativegaps.sh` defines two functions: `expect_refused` and
`expect_compiled`. Every gap closed today was recorded by calling
**`expect_compiles`**, which is not a function.

Bash wrote `expect_compiles: command not found` to stderr, returned 127, and —
with no `set -e` — carried on. Nothing set `fail`. The script ended with:

```
the gap list matches the compiler
```

having run 28 of its 39 cases. The eleven that did nothing were exactly the
eleven closures this session was built on.

### What it did and did not cost

The closures are almost certainly sound: each was checked by hand against the
interpreter, for output and for leaks, before the gate line was written. What
was lost is the *permanent* check — eleven shapes with no gate, and a gate
reporting success for work it had not done.

That is F36's defect in a second place, and this time self-inflicted: **a gate
that reports a number it did not check.** F36 was a gate reading an artefact it
did not build. The shape is the same one twice in one day.

### The fix, and why it is not "rename the calls"

Renaming them is the repair. The fix is that the script now counts what it
declared and compares it with what it ran:

```
declared=$(grep -cE '^expect_(compiled|refused) "' "$0")
...
if [ "$ran" != "$declared" ]; then
    echo "only $ran of $declared cases ran — the rest were never reached,"
    echo "so this run checked less than it looks like it did."
    exit 1
fi
```

A misspelt case, a case after an early return, a case inside a branch that
never fires — all of them now fail, whatever the reason. 39 declared, 39 ran.

### Two holes in `expect_compiled` itself, found while it was being repaired

**It compared output and discarded the exit code.** F39 printed the right value
and took SIGBUS afterwards; that is agreement by this test's standard. It now
compares both, and reintroducing F39 produces `DISAGREE: a handler op that is a
named fn` — which is how the repair was verified.

**It never leak-checked.** These programs are not in the corpus, so
`scripts/leakcheck.sh` never sees them: every gap closed here was checked for
agreement and not for whether it releases what it allocates. It runs them under
`GRASM_LEAK_CHECK=1` now.

### F40's sequel: a gate for the gates

`scripts/gatecheck.sh` reads every gate script and fails when a line at column 0
calls an identifier that is neither a function that script defines (or sources)
nor a real command. That is exactly the shape F40 took, and it is the check that
would have caught it the moment it was written.

`command_not_found_handle` would do this at run time in one line. Bash 3.2 is
what macOS ships and what these run under, and it does not have it — so the
check is static.

Two mutation tests, because a gate that cannot fail is the thing being guarded
against here:

* Reintroducing the misspelling gives `undefined: 1` and exit 1.
* `nativegaps.sh` catches it independently now as well. Its own count compared
  *declared* against *ran*, and a misspelt case is neither — so the totals
  agreed and it passed. Counting any `expect_` line as declared is what makes a
  misspelt one show up as declared-but-never-ran.

The second point is the general one: **a coverage count that derives both of its
numbers from the same assumption cannot detect a broken assumption.** The first
version counted `expect_(compiled|refused)`, which is the set of things that
work by construction.

## F41 — a generator that printed MISMATCH and exited 0 (S1, gate) — **fixed**

`scripts/fuzz_fallible.py` ends by printing its tally. Every other generator
here ends with `sys.exit(1 if ... else 0)`; this one ended with the print. A run
that found a disagreement between the interpreter and `--native` printed the
program, the two outputs and the seed — and exited 0.

It is listed in `CONTRIBUTING.md` among the checks to run before a change. A
person reading exit status, or any loop running these in sequence, would have
been told the run was clean.

Found by auditing the generators after F40, on the theory that the verification
layer was where the risk was rather than the language. That was three for three
in one day: F36 (a gate reading an artefact it did not build), F40 (a gate
reporting cases it did not run), F41 (a gate finding a defect and reporting
success).

### And a second hole in the same file

A program the native backend refused was skipped by a bare `continue` and
counted nowhere — neither compared nor rejected. A change that made `--native`
decline every generated program would have produced `compared: 0   mismatched:
0` and looked like a clean run.

Every generator now reports **declined**, and the number is not small:

```
fuzz_handlers: programs: 20   mismatched: 0   rejected: 0   declined: 9
```

Nine of twenty generated handler programs cannot be compiled natively. Those are
the four handler shapes still on the gap list — a handler returned, stored in a
struct, taken from a list, or printed — and the generator has been walking past
them silently. It is the first measurement of how much of that surface is
missing rather than a count of the shapes, and it is a better argument for
finishing them than the list was.

### Mutation-tested

Forcing the comparison to fail gives exit 1; restoring it gives exit 0. The
check that a gate can fail is the one thing that cannot be taken on faith, and
not doing it is how F40 survived a day.
