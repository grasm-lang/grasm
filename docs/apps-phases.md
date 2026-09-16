# Building real applications, as a measurement

A working log. Three phases, each answering a question the gates cannot ask,
each with what it found — including the tests that found nothing, because a
test that found nothing is evidence too and a log that records only the
discoveries overstates the discovery rate.

## Why applications, when there are 2,177 generated programs

The generators probe the **type system, codegen and semantics**. That is where
all nine of the structural findings were, and they are good at it.

They cannot probe four things, because every program they build starts, prints
and exits:

| axis | what the corpus does today |
|---|---|
| **duration** | nothing runs longer than a second |
| **scale** | nothing holds more than a few thousand objects |
| **composition** | no program used more than three libraries at once |
| **use** | every program was written by whoever wrote the library |

Those four are what an application is for. The bound on all of it, stated
first: an application is *still a probe*, and building more surface produces
more findings forever. So each phase answers a question and then stops.

## What cannot be built, and why

From `docs/packages-plan.md` under "Blocked, and on what":

* **No TLS** — so `http://` behind a proxy that terminates it, which is how
  most services are deployed anyway. No HTTPS, no gRPC, no `wss://`.
* **No task that outlives its block** — `parallel` and `spawn` join before the
  next statement. So no reaper, no eviction thread, no keep-alive sweeper, no
  connection pool holding sockets between requests. Everything happens on a
  request.
* **No monotonic clock** — every duration measured can go backwards when the
  system clock is adjusted.
* **No sleep** — the only blocking call with a timeout is `net_wait`.

Every application below is designed inside that, and says where it is touching
an edge.

---

# Phase 1 — duration and scale

**The question.** Does the runtime survive being *used*, rather than being
started and stopped? Specifically the reference counting, which produced F79
and had never been watched under sustained churn.

## `apps/linkstore`

A link shortener. Nine libraries: `http`, `http_server`, `json`, `log`,
`metrics`, `sanitize`, `uuid`, `random`, `url`. Declares
`["io", "net", "Log", "Metric"]` and nothing else.

The handler is a *pure function* of the store and the request, so every route
is exercised offline in the self-test and the socket path calls the identical
function. That is what puts it in the corpus: all gates build it three ways.

### What it found

**Three defects, none of them in the libraries.**

**1. Every link got the same id.** A v7 UUID's first 48 bits *are* the
millisecond it was made — that is what makes it sort by time, and exactly what
makes the first eight hex characters identical for everything created in the
same 50-day window. `substr(0, 8)` produced one id for the whole store. Takes
from the tail now.

**2. A target carrying `\r\n` was stored.** `sanitize.header_value` strips
control characters on the way out, so the `Location` header was never
splittable — but the *store* held one, and the comment beside the check claimed
otherwise. Refused at the door now, so no other output has to carry the same
guard.

**3. Caught by the compiler before it ran.** `let REGISTRY = ref(...)` at the
top level was refused: *"a constant is evaluated again at every mention, so
each one would be a different slot"*. That would have looked like metrics
silently not accumulating. Worth recording as a diagnostic that did its job.

None of the three was reachable by any generator, because none is a language
defect. They are a service using a correct library wrongly, which is the class
an application probes and a generator cannot.

### What it measured

| | |
|---|---|
| 5,481 requests over 45s — reads, writes, redirects, refusals | no crash |
| **read-only, 5,010 requests** | rss grows to ~500K then **falls back**: bounded |
| with writes, ~783 links created | +3,280K, which is the store |
| `GRASM_LEAK_CHECK=1` on the serve path | **0 live allocations at exit** |

The read-only number is the finding. Under sustained load with nothing stored,
memory plateaus and comes down — the reference counting holding, which is what
had never been watched.

`apps/linkstore/load.sh` is the harness, and it samples rather than asserts: a
throughput number from one machine is not a fact about the language, and
growth that does not stop is.


## `apps/logpipe`

The other half of Phase 1. `linkstore` answered *duration* — does memory grow
while requests arrive. This answers *scale*: does anything become quadratic, or
hold more than it should, when one input is large. A service handles a thousand
small strings; a pipeline handles one enormous one, and they fail differently.

Seven libraries: `csv`, `fs`, `json`, `metrics`, `path`, `regex`, `time`.
Declares `io` and `Metric`.

### What it measured

The same code over 1,000, 10,000 and 100,000 log lines. The number that matters
is **cost per line**: if it climbs with `n`, something is quadratic.

| lines | interpreter | native |
|---|---|---|
| 1,000 | 106 µs/1k | 10 µs/1k |
| 10,000 | 101 µs/1k | 9 µs/1k |
| 100,000 | 105 µs/1k | 9 µs/1k |

**Flat across two orders of magnitude, on both engines.** Nothing is
quadratic. 100,000 lines in 908 ms natively, 10.6 s interpreted — an 11×
gap, which is the ordinary difference between the two here.

Peak resident memory on the 100,000-line run: **43 MB**, for about 7 MB of log
text. Roughly six times the input, which is what a non-streaming design costs:
the whole text is held, plus a string per field per line. That is a real
number and a real limit, not a leak — it is bounded by the input, and the
input is bounded by memory.

`GRASM_LEAK_CHECK=1`: **0 live allocations at exit.**

### What it found

**Nothing.** No defect in the language, the libraries or the application.

That is worth recording as plainly as a discovery. It is the first application
in these phases to find nothing, and the honest reading is not "the language is
fine" — it is that this axis, with these libraries, at this scale, produced no
disagreement. A different shape of input would be a different test.

### A limit it ran into and did not work around

`read_file` answers the whole text, so a file larger than memory cannot be
processed and there is no incremental reader to build one from. The pipeline
therefore does not stream, and says so in its own header rather than inventing
a workaround that would hide the gap. **Streaming file input is a library gap
with a name**, and it belongs in `docs/packages-plan.md`.

### A note on where `regex` was not used

`parse_line` splits on spaces rather than matching a pattern, and the comment
beside it says why: a Thompson machine is linear in the input, and a million
lines through one is a million matches — linear, and a constant factor more
expensive than a split. `regex` is used where the pattern comes from the
*user*, which is the case a split cannot handle. The distinction is worth
recording because reaching for the general tool everywhere is how a linear
pipeline acquires a large constant.

The catastrophic pattern `(a+)+b` is run as a filter over the same input, and
returns — which is the property `packages/regex` was built for, exercised
here in the place it would actually appear.


---

# Phase 2 — composition

**The question.** Sixteen libraries in one program: do packages written
separately fit together, or do they collide? Grasm's types share one namespace
across the whole program ([T-MODULE-NS]), which F78 found the hard way at eight
libraries, so this is the shape that finds the next one.

## `apps/apigateway`

An API gateway. Sixteen dependencies, seventeen packages in the graph:
`cache`, `cors`, `crypto_hash`, `errors`, `http`, `http_server`, `json`, `jwt`,
`log`, `metrics`, `random`, `resilience`, `sanitize`, `time`, `trace`, `url`.

Authenticate, rate-limit, break the circuit on a failing upstream, cache what
can be cached, trace it, answer CORS. Everything but the last is a thing that
goes wrong in production, which is why they are the ones here.

### What it found — three defects, all in the language or the fleet

**F82 — three more pairs of packages could not be imported together.**
`errors`/`log` both declared `Field`, `cli`/`metrics` both `Kind`,
`redis`/`toml` both `Value`. Every one invisible, because a collision only
appears when a program imports *both*, and no program had. F78 gated the
*rule* — two packages declaring one name must be refused. It did not gate the
*fleet*. A rule that holds and a fleet that satisfies it are different claims,
and only the first was checked.

**F83 — a qualified call was charged its own row and a local function's.**
`tr.finish(s)` names the function in `tr`; a local `finish` is a different
function; the row walk charged the caller both. So a function calling only
pure things was refused for an effect nothing in it reaches. It needs a
qualified call whose bare name collides with a local one — a shape that appears
when a program is large enough to have its own `finish` beside a library's, and
no program here had been.

**F84 — a generic container could not be built in a struct field.**
`Gateway{ responses: cache.new_cache(64, 30000) }`, where the field's declared
type is `Cache[str]` and says exactly what `V` is. F77 taught the backend to
read a *binding's* annotation and stopped there. The repair is the one
`docs/probe-removal.md` built: the checker settled that call's type, and asking
it covers every position at once rather than one more. **The first defect the
probe-removal work fixed rather than measured.**

### What it demonstrated

| | |
|---|---|
| `alg: none`, wrong key, expired, malformed | all four refused, by `jwt` rather than by the gateway remembering to |
| the cache keyed on path **and** subject | bob gets a MISS on a path ada cached — caching on the path alone is how one user's data reaches another |
| three upstream failures | the fourth request is 503 from the breaker, not 502 from the upstream: the failing service stops being called |
| ten tokens, then 429 | and 200 again three seconds later |
| a preflight | never reaches authentication, because the browser sends it *without* the credentials it is asking permission for |

### The ceiling, which is the point of the whole arrangement

`grasm deps` reports **`apigateway performs [io]`** — against seventeen
packages, three of which perform their own effects. `main` installs a handler
for `Log`, `Metric` and `Trace`, so [T-CAPABILITY] counts them handled rather
than performed, and they do not escape.

The manifest first declared all five. That built, and was wrong in the way that
matters: **a ceiling wider than what a package does is a permission nobody
needed, and the compiler does not complain about one.** Narrowed to `io, net`.
A gate for over-declaration would be worth having and does not exist.


---

# Phase 3 — use

**The question.** Does the language hold up for somebody who is *not* writing
a demo? The bar in `grasm-finish-bar` needs two independent cold-model
evaluation rounds, which cannot be run from inside this session. The nearest
available substitute is a tool that has to read code **nobody wrote for it** —
so it runs over this repository's own 358 programs, and anything it cannot
handle is a finding rather than an excuse.

It is also self-hosting pressure at a size that can be finished. A compiler
written in its own language is the standard proof a language is done; a
*linter* written in its own language exercises the same thing — reading Grasm
at scale — today.

## `apps/grlint`

A linter for Grasm, in Grasm. Five libraries: `cli`, `fs`, `json`, `path`,
`regex`. Declares `io` and nothing else, which is what a tool run over
somebody else's repository should be able to promise.

Eight rules, every one answerable from a *line*. The rules that need scope —
an unused binding, shadowing, a non-exhaustive match — are named by
`unsupported()` rather than approximated, because a rule guessed at with a
regular expression is wrong on exactly the programs that matter.

### What it measured

**640 findings over 358 programs in 0.57 s**, natively. The whole corpus —
every package, example, conformance program, ProjectGold program, benchmark
and application — read and checked in under a second.

| rule | findings |
|---|---|
| `long-line` | 278 |
| `no-doc` | 215 |
| `empty-catch` | 113 |
| `unwrap-default` | 22 |
| `todo` · `wide-row` | 4 each |
| `tab-indent` | 3 |
| `trailing-space` | 1 |

### What it found in the code written this session

Four `empty-catch` findings in `packages/` and `apps/`, and **all four were
real**. A write to a closed socket, an accept that raced a hang-up, a partial
parse — all deliberate, all written `Err(e) -> {}`, which binds a name and
throws it away and reads as an oversight. They say `Err(_gone)`,
`Err(_hung_up)`, `Err(_incomplete)` now, each with the sentence explaining why
nothing happens.

Not a bug. Four places where correct code did not say it was correct, which is
what a linter is for.

### What building it found

**A rule that checked the wrong thing, and a self-test that agreed with it.**
`no-doc` is "a `pub fn` with no `///` above it", and the first version asked
`starts_with("//")` — which every ordinary comment satisfies. The rule fired on
almost nothing, and the self-test recorded that as correct.

Caught by *reading the output*, not by the test. Worth stating because it is
the general case of a defect this repository keeps finding: F70's framing check
measured the body the way the body was measured, F29's bounds test never
reached the boundary. **A test that asserts what the code does agrees with the
code.** The rules that survive are the ones checked against something outside
themselves — which is why every codec in `packages/` is checked against a
published vector, and why `fuzz_laws.py` checks against arithmetic.

**An invocation that was wrong, and a specification that was right.**
`./grlint -- --summary` silently ignored every flag. [E-ARGS] says a compiled
binary has no `--` separator — the one in `grasm run prog.gr -- a b` belongs to
the toolchain — so the literal `--` arrived as an ordinary argument and `cli`
read everything after it as positional, which is precisely what `--` means on a
command line. Correct behaviour by both, and a thing a first-time user gets
wrong exactly once.

## What Phase 3 does *not* substitute for

A real outside user would bring programs nobody here imagined. `grlint` reads
this repository's code, which this repository wrote — so it probes *scale of
reading* and not *diversity of input*. The cold-model rounds in
`docs/release-scope.md` remain the thing that has not been run, and this does
not replace them. It narrows what they would be looking for.

---

# Phase 4 — infrastructure

**The question.** The first three phases built programs that *use* a network.
This one asks whether the language can be the thing on the other end of the
socket: a server holding its own on-disk format, answering many connections at
once, and keeping a second copy of itself in step. It is the first application
whose correctness is about what survives being killed.

## `apps/broker`

A message broker. Topics, an append-only log per topic, subscribers that
follow it as it is written, and a peer that replicates it. Two packages carry
the parts that do not need a socket: `packages/brokerwire` is the framing,
`packages/topiclog` is the storage, its segments and its recovery.

**Shape, and why it is the only one available.** Grasm has no thread a program
can put a socket on — `spawn` does not carry `net`, `parallel` is not
multicore — so a broker here is one process, one loop over `net_wait`, many
connections. That is how nginx and Redis are built and is not how Kafka is
built, and the consequences are not hidden: one slow consumer holds the turn,
because `tcp_write` blocks until the kernel takes the bytes and the language
offers no writability readiness and no partial write. Batches are bounded so
the turn stays short. A follower is a subscriber that appends what it receives
at the offsets the leader gave it, which is one protocol rather than two.

**What it found.**

1. **The compiled backend refused `inout` places through two of its three call
   paths.** Passing `s.logs[at]` to a package function that appends to it —
   the ordinary shape of a server that keeps its state in one struct — was
   rejected with "inout arguments that aren't plain variables" for a
   module-qualified call and for a method receiver, though a plain call
   accepted it and the interpreter accepted all three. Fixed in
   `crates/grasm_native`; `spec/conformance/T-INOUT-PLACE-ARG.gr` is the
   regression case and the broker gate covers the module form.

2. **A self-test that wrote files only held the first time it was run.** The
   second run read the first run's segments and reported seven failures. A
   test that passes only on a clean checkout is not a test, so the self-test
   now clears its own directory — and the gate runs it twice and compares.

3. **`grasm build` without `--native` bundles the interpreter, and for a
   server that is a sixtyfold difference.** The same broker served 448
   messages a second bundled and 29,735 compiled. This is documented
   behaviour, not a defect — `agreetest` exists because the bundled build is a
   supported way to run a program — but nothing had ever measured what it
   costs a program that runs for more than a second, and a profile of the
   bundled binary is a profile of `eval_expr`.

4. **Two per-byte loops cost more than everything else put together.**
   `brokerwire` copied a payload into a frame one byte at a time through an
   `[i64]`, and `topiclog` rebuilt CRC-32's 256-entry table for every record
   it wrote. Neither is wrong in the library each came from; both are wrong at
   forty thousand records a second. Copying blobs whole and keeping the table
   on the log took 128-byte records from 29.7k to 44.8k a second and
   4-kilobyte records from 13 MB/s to 45 MB/s.

**What it measured.** One connection reaches the same rate as eight, so the
ceiling is the loop's per-message work and not its connection handling.
Throughput *rises* with pipeline depth — 23.3k a second at four in flight,
46.4k at 128 — which is the batching working: per-tick costs amortise instead
of compounding.

| connections | 128-byte records a second |
|---|---|
| 1 | 38,211 |
| 8 | 37,313 |
| 64 | 29,524 |
| 256 | 26,357 |
| 512 | 20,661 |
| 1024 | 14,423 |

The falloff from 8 to 1024 was steeper before this phase — 6,943 a second at
512 — because every tick walked every connection to build the wait set, the
handle index and the flush list. Keeping those incrementally makes a tick cost
what it handles rather than what is connected, and 512 connections went from
6.9k to 20.7k a second. On one core, in one thread, with no `unsafe` anywhere
in the program.

**What the gate checks**, in `scripts/broker_test.sh`, seventeen claims: the
three engines agree on the deterministic self-test and it repeats; a live
server publishes, fetches and subscribes over a real socket; SIGKILL during a
write loses only what was never acknowledged, the log reopens, every surviving
record reads back whole, and the offsets continue rather than restart; and a
follower catches up to the leader and its segment files match byte for byte.

**What it does not show.** Nothing here is multicore, so a second core would
need a second process and a partitioning scheme this does not have. There is
no `fsync` policy beyond "when the publisher asked", no leader election, no
consumer groups, and no authentication. The blocking-write hazard is real and
bounded rather than solved: a consumer that stops reading still holds the turn
until the kernel takes its bytes.

---

# What the four phases found, together

| phase | application | libraries | defects |
|---|---|---|---|
| 1, duration | `linkstore` | 9 | 3, none in the libraries |
| 1, scale | `logpipe` | 7 | **0** |
| 2, composition | `apigateway` | 16 | 3 — F82, F83, F84 |
| 3, use | `grlint` | 5 | 1 in the tool, 4 in this session's code |
| 4, infrastructure | `broker` | 2 (its own) | 2 — a backend refusal and a self-test that held only once |

**Nine defects and one clean result.** The clean one is `logpipe`, and it is
recorded as plainly as the others: this axis, with these libraries, at this
scale, produced no disagreement.

The pattern across all five: **an application finds what the shape of a real
program reaches, and nothing else.** `linkstore` found library misuse.
`apigateway` found three language defects, every one needing a program with
sixteen dependencies to exist at all. `grlint` found four places where correct
code did not say so. `broker` found a hole in the compiled backend that only
a program keeping its state in one struct and handing pieces of it to a
package would meet. None of the nine was reachable by any of the 2,177
generated programs, and none of them is the kind a generator could be extended
to reach — they are consequences of *size* and *combination*, not of shape.
