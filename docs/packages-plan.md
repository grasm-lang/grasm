# The packages a first release needs

Written 2026-08-27, extended the same day with a second list. Four exist. This
is the whole programme, in the order it gets built, with the reason each entry
is on it — and, at the end, the five things that are **not** buildable today
and what each one is waiting on.

## The rule that shapes every entry

A package declares the effects it may perform, effects are inferred from its
source, and a package that exceeds its ceiling **fails to build in every
consumer** ([T-CAPABILITY]). So the interesting column below is not "what does
it do" but "what may it do" — and the design goal for each package is to
declare *less*.

That is why `http` is three packages: parsing and routing declare nothing,
sending declares `net`, listening declares `io, net`. The same split applies
below wherever computation can be separated from reach.

The second lever is **effects instead of capabilities**. A library that logs by
calling `print` must declare `io`, and every consumer inherits a library that
can write anywhere. A library that logs by performing a `Log` effect declares
`Log`, and the consumer supplies the sink — which means the consumer decides
whether logging touches the world at all, and a test can capture it without a
file. Anywhere a library needs *the world* rather than *a computation*, the
answer is an effect, not a capability.

## Built

| | package | declares | |
|---|---|---|---|
| — | `http` | *nothing* | parse, route, render |
| — | `http_server` | `io`, `net` | the accept loop |
| — | `http_client` | `net` | dial, send, read a frame |
| — | `json` | *nothing* | parse, render, reach in, build up |

## The fifteen

**Eleven of the fifteen are built** — waves 1 and 2 complete, in
`packages/`, each with a demo the corpus builds three ways. What follows keeps
the original reasoning for each; the state of each is in `packages/README.md`.

### Wave 1 — no dependencies, and everything else needs them

**1. `bytes` — hex and base64.** `encode_hex`, `decode_hex`, `encode_base64`,
`decode_base64`, url-safe variants, constant-time compare. Declares nothing.
Needed by `uuid`, `crypto`, `http` (basic auth), `url`. Small, and wrong in a
way that is easy to test: every byte sequence round-trips, and every invalid
input is refused rather than silently truncated.

**2. `time` — a calendar, not a clock.** `now_millis()` exists and performs
`io`; everything *above* it is arithmetic. Civil date from an epoch
millisecond, back again, day of week, leap years, month lengths, ISO-8601
parse and format, durations, comparison. Declares nothing — the clock is a
parameter, which is also what makes it testable. A `time_io` companion
declaring `io` supplies `now()`.

**3. `random` — deterministic by default.** `random_bytes` performs `io`; a
seeded generator does not. xoshiro or PCG, `next`, `below(n)` without modulo
bias, `range`, `shuffle`, `choice`, `bytes(n)`. Declares nothing. A seeded
generator is what makes a test that uses randomness reproducible, and this
language already cares about reproducibility more than most.

**4. `uuid`.** v4 from a `random` generator, v7 from a millisecond and a
generator so identifiers sort by time, parse, format, `nil`, version and
variant. Declares nothing — the entropy comes in as an argument. Depends on
`bytes` and `random`.

**5. `url`.** Scheme, host, port, path, query, fragment. Parse, build, encode,
decode, resolve a relative reference. Declares nothing. `http`'s percent
coding moves here and `http` depends on it, so there is one implementation
rather than two.

**6. `path`.** Join, normalise (`..` and `.` without touching the disk), split,
extension, base name, directory, absolute-or-not. Declares nothing — it is
string arithmetic. The `io` half — walking a tree, globbing — is `fs` below.

### Wave 2 — the ones a program author reaches for on day one

**7. `log` — logging as an effect.** `effect Log { fn record(level, message,
fields) }`, levels, a structured record, and handlers: to stderr, to a file, to
a list for a test, discarding. A library that logs declares `Log` and nothing
else. This is the single clearest demonstration of what this language is for,
and it is small.

**8. `cli`.** Flags with types and defaults, positional arguments,
subcommands, generated help, and an exact error for a bad invocation. `args()`
performs `io`, so parsing takes a `[str]` and declares nothing; the reading of
`args()` is one line in the consumer's `main`.

**9. `testing`.** Table-driven cases, `expect_fail`, approximate comparison for
floats, golden-file comparison, a diff worth reading, and a summary. `test`
blocks are in the language; what is missing is everything that makes a hundred
of them legible.

**10. `csv`.** Parse and write, with quoting, embedded newlines, and a header
row. Declares nothing — text in, rows out. The most common data format that is
not JSON, and the one where the naive implementation is wrong.

**11. `toml`.** Grasm's own manifests are TOML, so a Grasm program that reads
configuration should not need a second format. Tables, arrays, inline tables,
the datetime types, and a refusal that names the line. Declares nothing.

### Wave 3 — larger, and each one unblocks a class of program

**12. `regex`.** — **built**, `packages/regex`. A Thompson NFA — no backtracking, so no input makes it
exponential. Character classes, repetition, alternation, anchors, groups,
`find`, `find_all`, `replace`, `split`. Declares nothing. Large, self-contained,
and the thing whose absence is felt in every text-processing program.

**13. `fs`.** — **built**, `packages/fs`. Walk a directory tree, glob, copy, move, temporary files and
directories, read lines without holding the file. Declares `io`. Everything
that is *not* reach — path arithmetic — is already in `path`, so this package
is small and its ceiling is honest.

**14. `task` — channels, select, and a clock.** `parallel` and `spawn` are in
the language and join deterministically; what is missing above them is a
channel, a select over several, a timeout, and a sleep. The only blocking
primitive with a timeout is `net_wait(handles, ms)`, so a sleep is a wait on no
handles — which is worth writing down because it is the sort of thing that is
obvious once and never again. Declares `async`, and `net` for the timeout.

**15. `sql` — an interface and one implementation.** A statement, a parameter,
a row, a cursor, and a transaction, as an *interface* — an effect, so the
driver is a handler and a program can be tested against an in-memory table and
run against a socket. `ProjectGold/28-sql-engine` is 583 lines showing the
parser and planner already work. The interface declares an effect; the
in-memory driver declares nothing; a real driver declares `net`.

## Beyond the fifteen — the production set

The fifteen make the language usable. These make it usable *in production*, and
several of them are blocked on something the language does not have yet, which
is said plainly against each one rather than discovered later.

### Reliability

**16. `crypto_hash`.** — **built**, `packages/crypto_hash`: SHA-256, HMAC,
PBKDF2, constant-time compare, every digest checked against a published
vector. Still wanted: SHA-512,
HMAC, and a constant-time compare from `bytes`. Declares nothing. Safe to write
here: a hash has no key schedule and no timing surface beyond the compare.

**17. `password`.** — **built**, `packages/password` (PBKDF2). PBKDF2-HMAC-SHA256 first, then Argon2id. These are
*meant* to be slow, so a pure-Grasm implementation is not the liability it
would be elsewhere. Declares nothing.

**18. `sanitize`.** — **built**, `packages/sanitize`. HTML escaping (in `http` already, moves here), attribute
and URL contexts, SQL identifier quoting, and a strict allow-list HTML
subset. Declares nothing.

> **Not AES or RSA.** Both are writable and neither should be written here.
> A block cipher without constant-time guarantees from the compiler is a
> timing oracle, and Grasm makes no such guarantee today. When TLS arrives it
> arrives by binding a reviewed C library, not by hand-rolling primitives.

### Observability

**19. `metrics`.** — **built**, `packages/metrics`. Counters, gauges, histograms with explicit buckets, and a
Prometheus text exposition format. The registry is an effect, so a library can
count things without declaring `io`; the exposition endpoint is `http`.

**20. `trace`.** — **built**, `packages/trace`. Spans, a span context that propagates through
`traceparent`, and an exporter shape. The *propagation* and the data model are
pure and declare nothing; the exporter is `http`. A W3C trace context header
is text, which this language is good at.

**21. `errors`.** — **built**, `packages/errors`. An error value that carries a chain, a source location and
fields; capture, format, and a report shape an exporter can send. Declares
nothing. Grasm has no exceptions and no backtrace object, so the chain is
built by the program — which is more work and is also why it cannot silently
lose one.

### Networking

**22. `websocket`.** — **built**, `packages/websocket`. The HTTP upgrade handshake, `Sec-WebSocket-Accept` (SHA-1
plus base64 — both available), frame encode and decode, masking, ping and
pong, and close codes. The framing declares nothing; the connection declares
`net`. `ws://` only until there is TLS.

**23. `resilience`.** Retry with exponential backoff and jitter, a circuit
breaker, a token-bucket rate limiter, a timeout wrapper, and a bulkhead. The
policies are pure arithmetic over a clock and a random source that both come
in as arguments — so the whole package declares nothing and is testable
without waiting for real time to pass.

**24. `cache`.** An in-memory key-value store with LRU and TTL eviction, and a
`get_or_compute`. Declares nothing. Single-process only, for the reason under
"Blocked" below.

### Data and drivers

**25. `redis`.** — **built**, `packages/redis`. RESP2 and RESP3 encode and decode, a client over `net`. The
easiest real driver to write and the most useful per line: the protocol is
length-prefixed text.

**26. `postgres`.** — **built**, `packages/postgres` (SCRAM-SHA-256, no TLS). The v3 wire protocol over `net`: startup, `cleartext` and
`md5` authentication, simple and extended query, the binary and text formats,
`COPY`. SCRAM-SHA-256 needs `crypto_hash`, which is on this list. No TLS, so
this reaches a database on a trusted network or a local socket, which is
exactly what a development machine is.

**27. `mongo`.** The OP_MSG wire protocol and BSON. BSON is a binary format
this language can write; the driver is `net`.

**28. `protobuf`.** — **built**, `packages/protobuf` (the wire format). Wire-format encode and decode, and a `.proto` parser that
generates Grasm. The encoding declares nothing. **gRPC does not follow from
it** — see below.

### Environment

**29. `dotenv`.** — **built**, `packages/dotenv`. Read a `.env` file, with quoting, comments, multi-line
values, and interpolation — and, importantly, *without* mutating the process
environment, which is a global other threads can see change. It answers a map;
the program decides what to do with it. Declares `io`.

**30. `xml`.** Parse and render, entities, namespaces, and a refusal that
names the line. No DTD and no external entity resolution, ever: that is how
XXE happens and there is no reason a first release needs it. Declares nothing.

### Enterprise security

**31. `jwt`.** — **built**, `packages/jwt` (HS256). Encode, decode and verify HS256, HS384 and HS512 from
`crypto_hash`, with `exp`, `nbf`, `iat`, `aud` and `iss` checked and a clock
that comes in as an argument. RS256 waits for a reviewed RSA, which waits for
the same thing TLS does. Declares nothing.

**32. `oauth2`.** The authorisation-code flow with PKCE, the client-credentials
flow, token refresh, and state. The URL building and the token parsing are
pure; the exchange is `http_client`. Against a real provider it needs TLS.

**33. `cors`.** — **built**, `packages/cors`. Origin matching, preflight, credentials, exposed headers and
max-age, as a function from a request to the headers a response needs. Declares
nothing — it is a decision, not an action, which is exactly the shape `http`
already has.

## Blocked, and on what

These are not scheduling decisions. Each one names a thing the language does
not have, so that nobody spends a week discovering it.

**1. TLS — and so HTTPS, gRPC, `wss://`, and OAuth2 against a real provider.**
There is no TLS and there will not be a hand-written one. The realistic path is
binding a reviewed C library through `extern fn`, which works and which costs
`unsafe` in the row — meaning a package doing it declares `unsafe` and every
consumer sees that. That is the correct outcome and it is worth the honesty.

**2. A background task that outlives its block — and so an async runtime, a
thread pool, and a connection pool holding live sockets.** `parallel` and
`spawn` join before the statement after them, by design and for
determinism. Nothing detaches. A connection pool can reuse a socket within one
request; it cannot keep one warm across requests in another task, because there
is no other task. This is a language question, not a library one.

**3. A monotonic clock.** `now_millis()` is wall time. Every duration a metric,
a trace span, a timeout or a circuit breaker measures can therefore go backwards
when the system clock is adjusted. The fix is a primitive.

**4. A sleep.** The only blocking call with a timeout is `net_wait(handles,
ms)`, so a sleep is a wait on no handles. That works and it is a strange thing
to have to know, so `task` will wrap it and the wrapper will say why.

**5. Process and machine metrics — CPU, memory, file descriptors.** They come
from `/proc` on Linux and from `sysctl` on macOS, so one is `io` and the other
is `extern`. The `metrics` package will carry the registry and the exposition
format and will not pretend to carry these.

## Also worth having, and not on either list

`markdown` and `sha256` exist under `ProjectGold/` and need promoting rather
than writing — cheap, and they remove two more duplicate copies.
`collections` (set, deque, heap, ordered map), `template`, `ini`, `compress`,
`tar`, `stats`, `text` (unicode segmentation), `retry`, `metrics`. None of them
decides whether the language is usable; all fifteen above do.

## What each one has to satisfy to count as done

- A `grasm.toml` declaring the smallest ceiling that compiles.
- A `*-demo` beside it that exercises the whole surface with awkward input,
  so `difftest`, `leakcheck` and `crosstest` hold three engines to one answer
  on every run.
- Every refusal a value, not a panic: a document, a URL, a flag, a CSV row is
  input somebody else chose.
- Where the answer is a law rather than a recorded string — a round trip, an
  invariant — it goes in `scripts/fuzz_laws.py`, because a recorded expectation
  makes the implementation's answer the definition of the right answer (F69).
