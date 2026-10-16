# The two-hundred-crate list, audited against what exists

A list of "the top 200 Rust crates" arrived on 2026-10-16 with a brief: match
them in Grasm, and surpass them where it is technically possible. This is the
audit, in the same shape as `docs/ecosystem.md`, because the same thing is
true again — it is a good list, and planning against it without counting
first would have produced a year of work that was already done.

**There are 667 packages**, plus 203 `-demo` directories that exist to be
run by the gates rather than imported. (An earlier draft of this document
said 780. That number came from stripping `-demo` from every directory name
and counting what was left, which counts a demo whose base name is not a
package as though it were one. `scripts/site.py` counts the manifests, and
667 is what it has always said.)

Against the 200:

| | count | what it means |
|---|---|---|
| already here, by name | 21 | `aes` `argon2` `bcrypt` `bytes` `csv` `glob` `h2` `log` `mime` `prometheus` `redis` `regex` `ron` `rsa` `sha3` `tar` `time` `toml` `url` `uuid` `zip` |
| already here, under a Grasm name | 26 | `serde_json`→`json`, `reqwest`→`http_client`, `hyper`→`http_server`, `rustls`→`tls`, `prost`→`protobuf`, `petgraph`→`graphs`, `flate2`→`deflate`, `tungstenite`→`websocket`, and eighteen more |
| no meaning in Grasm | 22 | |
| already a language feature | 17 | |
| a runtime concern, not a package | 15 | |
| blocked on one missing capability | 50 | |
| writable today | 49 | |

So **47 of 200 exist**, 54 are not packages in this language at all, 50 wait
on a single thing, and 49 are real work that could start this afternoon.

## The four categories that are not "not built yet"

**No meaning in Grasm (22).** `syn`, `quote`, `proc-macro2`, `serde_derive`,
`darling`, `paste`, `prettyplease`, `strum_macros`, `typed-builder`,
`macro_rules_attribute` are procedural-macro machinery: they exist to parse
and emit Rust token streams at compile time. Grasm has no procedural macros
and no token stream to hand them. `cfg-if`, `autocfg`, `cc`, `pkg-config` are
Cargo build-script tooling. `winapi`, `windows`, `libc`, `nix` are raw system
bindings, which Grasm writes as `extern fn` directly — `packages/sdl` is the
worked example, and since this week its library is declared in the manifest.
`jemallocator` and `mimalloc` swap Rust's global allocator, which is not a
thing a Grasm program chooses. `pyo3` embeds Python.

Building Grasm versions of these would be building scaffolding for a compiler
we do not have.

**Already a language feature (17).** `anyhow`, `thiserror`, `eyre`,
`color-eyre`, `snafu`, `error-chain`, `derive_more`, `displaydoc`, `miette`
all exist because Rust's `Result` needs ergonomics bolted on. Grasm has error
types in the language: `fn f() -> T ! SomeError`, named variants, `try`, and
a compiler that will not let an error go unhandled. `futures` and
`async-trait` are polyfills for what the language does not express;
`lazy_static` and `once_cell` for initialisation order. `either` is a
two-variant sum type, which is three lines.

Writing these as packages would be adding a second, worse way to do something
the language already does — which is how an ecosystem gets two of everything.

**A runtime concern, not a package (15).** `hashbrown`, `smallvec`,
`tinyvec`, `arrayvec`, `compact_str`, `smartstring`, `bytemuck`, `slab`,
`ordered-float` are all the same shape: Rust hands the programmer the
allocator and the memory layout, so the ecosystem fills with crates that
choose differently. In Grasm the runtime owns layout. A small-vector
optimisation here is work in `grasm_rt`, measured by the benchmark suite,
not a package somebody installs. Whether it is worth doing is a separate
question and belongs in `docs/performance.md`.

## One capability blocks forty-nine of them

This is the finding that matters, and it is not "we need 153 more packages".

`tokio` and everything above it — `axum`, `actix-web`, `tower`, `tonic`,
`sqlx`, `sea-orm`, `tokio-postgres`, `mongodb`, `quinn`, `rocket`, `warp`,
`redis`'s async half, `lambda_runtime`, `opentelemetry` — is one dependency
repeated. So is the lock-free family: `crossbeam` and its four crates,
`dashmap`, `flume`, `arc-swap`, `concurrent-queue`, `event-listener` and
`parking_lot` all need atomics the language does not expose. Fifty in total.

Two capabilities, then, not 50 libraries:

1. **Non-blocking I/O with readiness notification** — an epoll/kqueue loop and
   a way to register a file descriptor. `mio`'s job.
2. **Atomic operations** as language primitives, with a memory ordering a
   programmer can name.

Everything in that column is downstream of those two. Anyone planning this
work should plan those two and then re-count, because the list will be much
shorter afterwards.

## What Grasm already has, which changes the shape of the async question

Grasm is not starting from nothing here. `parallel { spawn { … } }` is
structured concurrency with scoped lifetimes, there are channels
(`chan_send`, `chan_recv`, `chan_select`, `chan_close`) behind an async
effect row, and `docs/tutorial.md` records a property Tokio does not have:
**tasks run in spawn order, deterministically — the same program always
produces the same interleaving.**

That is a real asset and it is also the constraint. A work-stealing scheduler
is non-deterministic by construction. Whether Grasm wants Tokio's throughput
badly enough to give up reproducible concurrency is a language decision, not
a library one, and it should be made deliberately rather than discovered
halfway through writing a runtime.

## Where surpassing Tokio is a hypothesis with a test

The brief asks whether we can beat Tokio. Research into its known failure
modes points at one answer worth taking seriously, and it is not throughput.

Tokio's multi-threaded scheduler is cooperative: a task is expected to reach
an `.await` promptly. A task that runs blocking code instead holds a worker
thread and stops it polling. With spare workers the system self-heals through
work-stealing; when blocked workers equal total workers, polling capacity
reaches zero. Published measurements of this put it at **0% failures at 15
concurrent requests and 94% at 50** — a cliff, not a slope. The runtime's own
documentation of the problem is blunt about why it cannot be fixed inside
Rust: *the compiler enforces memory safety, and does not enforce scheduling
safety.*

Grasm has the thing that is missing there. Effects are declared and checked:
a function that performs I/O says `/ {io}`, and the compiler already refuses a
package that exceeds its manifest's ceiling. The hypothesis is therefore
specific:

> **A task body may be required to declare its effects, and an effect that
> blocks may be rejected inside a scheduler scope at compile time.**
> `spawn_blocking` becomes the only way to write it, and the cliff becomes
> unreachable rather than undetected.

That is a claim that can be wrong, and the way to find out is cheap: take the
published starvation benchmark, write it in Grasm, and see whether the
compiler refuses it. If it does, the advantage is real and provable. If the
effect system cannot tell a blocking read from a non-blocking one — and today
both are `{io}` — then the honest answer is that a finer effect row is needed
first, and that is a language change with its own cost.

**No benchmark claim belongs in this document until that experiment has
run.** Tokio is a decade of tuning by people who measure carefully; matching
its throughput is not the plausible win and should not be promised. Making a
whole class of production outage impossible to compile might be.

## The forty-nine that are writable today

Grouped by what they need, not alphabetically, because the dependencies run
within the group:

**Text and parsing (11).** `aho-corasick`, `memchr`, `bstr`, `nom`, `winnow`,
`pest`, `combine`, `logos`, `heck`, `indoc`, `phf`. The parser combinators
are four takes on one idea and should not all be built; `regex` already exists
and `aho-corasick` is the multi-pattern engine under it.

**Terminal and CLI (12).** `clap`, `clap_complete`, `ratatui`, `indicatif`,
`dialoguer`, `console`, `anstyle`, `nu-ansi-term`, `terminal_size`,
`is-terminal`, `atty`, `predicates`. `termios` and `ansi` are already here,
so these are a layer, not a foundation.

**Serialization (7).** `serde` as a shape, `bincode`, `flexbuffers`,
`serde_yaml`, `serde_bytes`, `serde_spanned`, `serde_urlencoded`. Grasm has
no derive macros, so "serde" here means a reflection or codegen story and is
a design question before it is a package.

**Filesystem and OS (7).** `walkdir`, `tempfile`, `same-file`, `home`,
`dirs`, `fs_extra`, `socket2`.

**Numeric and data (7).** `num-traits`, `num-integer`, `itertools`,
`indexmap`, `bitflags`, `constant_time_eq`, `hmac`.

`indexmap` was measured before being built, and the measurement says not to
build it as a package. Grasm's `Map` iterates in sorted key order on
purpose, so insertion order genuinely is unavailable -- but the thing that
would want an ordered map with fast lookup is `json`, whose `get` is a
linear scan over parallel key and value arrays.

On a 1000-key object, 20,000 lookups take 0.49s: 24.5us each, which is bad.
On a 10- to 50-key object -- which is what a configuration file or an API
response actually is -- a scan of five to twenty-five string comparisons
beats hashing the key. (The first number measured was 0.55s for the 10-key
case, which looked like a flat cost and was: 0.45s of it is macOS charging
for the first execution of a new binary. The same binary run twice is
0.00s. See `docs/performance.md`.)

So the work is making `json.get` not linear for wide objects, which is a
change to that package's representation with a real API cost, and not a
standalone container nobody would wire in. Written down here so the
measurement does not have to be taken twice.

**Observability (4).** `tracing`, `tracing-subscriber`, `slog`,
`env_logger` — all above the existing `log`.

**Testing (1).** `assert_cmd`. `predicates` is counted with the CLI group
above, where it is used.

## What this document does not do

It does not design a package registry, a naming standard, or a governance
model. Those were asked for in the same brief and each is a substantial
document; `docs/packages-plan.md` and `docs/ecosystem-roadmap.md` are where
that work already lives and should be extended rather than restarted. The
registry in particular is not hypothetical — `grasm publish`, `grasm add`,
`grasm deps --locked`, lockfiles and checksums exist today, and the gap is
hosting and policy rather than mechanism.

It also does not promise 200 libraries. Against the audit above that number
is not meaningful: 47 are done, 54 should never be built, 49 are two
capabilities wearing a disguise, and 49 are ordinary work. Promising the
headline figure would mean building the 54 to make the count come out.
