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

