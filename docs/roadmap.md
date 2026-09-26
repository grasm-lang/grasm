# Road to a first release

Four workstreams, running alongside the defect loop that has been going for
weeks. The loop is not paused for these: it is what keeps them honest, and
every item below lands with gates green or it does not land.

Recorded 2026-08-27, revised 2026-08-28. Status is measured, not estimated —
each line says how it was checked.

## Where this starts from

Measured on 2026-08-27, seven days after 2026-08-20:

| | 20 Aug | 27 Aug |
|---|---|---|
| findings recorded | 31 | 79 |
| generators | 9 | 15 |
| corpus programs (whole, agreeing) | 218 | 261 |
| release libraries | 0 | 34 |
| specification clauses | 230 | 233 |

Ninety-two commits, +32,552 / −8,051 lines. Thirty-six defects found and
fixed, twelve of them S1 — wrong answers, crashes, engine disagreements. Zero
open. Every gate green.

## 1. The website

**Status: built, in `site/`.** Sixteen pages: index, getting started, install,
playground, a tour, effects and capabilities, ownership tiers, errors,
concurrency, language reference, library reference, packages, specification,
examples, status, roadmap.

It is generated. `scripts/site.py` holds the chrome — rail, head, footer — once
and `scripts/site_pages.py` holds each page's content, because fourteen hand-
written copies of a navigation drift the way five hand-written copies of a
directory list drifted (F68's neighbour, and the reason for
`scripts/lib/corpus.sh`).

Every figure the site states is computed, never typed. The ones countable from
the tree — clauses, findings, generators, ProjectGold programs and their line
count, library methods — are counted each time the site is written. The ones
only a run can know — how many programs three engines agreed on, how many wasm
matched, how many laws ran — are read from `site/facts.json`, which
`scripts/allgates.sh` writes *after* every gate passes. So a number on the page
is either a fact about the files or a fact about the last green run.

Four mutations were run against that: a page edited by hand, a figure that no
longer matches the run, a figure nothing computes, and no recorded run at all.
All four fail the `--check`. A fifth — a gate that stops printing the count the
site reads — fails the recorder rather than writing a stale number.

The playground runs in the page (2026-09-26). `site/grasm.wasm` is the module
`ProjectGold/web/` serves and `scripts/wasmtest.sh` holds to the other engines
-- `scripts/site.py --check` fails if the two differ -- and its examples are
the files in `examples/`, read when the site is written, so a program it shows
is one the gates run. Checked in a browser: all eight examples print what
`grasm run` prints, byte for byte; a type error, a panic and an edited program
come back as they do on the command line; and no page scrolls sideways at a
phone's 390 px, which three of them did before.

The tutorial and the library reference are rendered from `docs/tutorial.md`
and `spec/library.md` (2026-09-27) by `scripts/site_markdown.py`, which knows
the constructs those two documents use and refuses any other rather than
guessing at it. The library page had been a hand-kept list of 81 method names;
it is now the specification's own tables and clauses, anchored by clause tag.

The original list, for the record: a landing page that says what the language is and why it exists;
getting started and install; a tutorial; a language reference; a library
reference; the effects-and-capabilities story, which is the thing that makes
Grasm different and the thing nobody will find on their own; the tier model;
packages; a playground, which the wasm build already makes possible; a showcase
drawn from the 44 programs in `ProjectGold/`; the specification and its
stability levels; and an honest status page — including what does not work.

It lives in `site/`, as static files with no build step and no external
requests, because that is what the Artifact CSP and a plain static host both
accept, and because a site with a build step is a site that stops building.

## 2. Speed and quality

**Status: ongoing, measured.** G2 is 0.45× native against Rust, 0.39×
floor-subtracted, against a 0.95× target. `scripts/bench.sh` computes it from
the numbers it prints rather than from anything typed by hand.

Known costs, in the order they are worth attacking: `tree`'s box-per-node;
binary size 350 KB against a 300 KB target; the reference count on every boxed
value, which is atomic because a `parallel` body may hold one.

Quality is the loop. It does not stop.

## 3. Packages and the libraries a first release needs

**Status: the mechanism was already built; thirty-four libraries now exist.**

`crates/grasm_pkg` does manifests, dependency resolution, lockfiles, path
dependencies, a local registry, and the capability ceiling — a package
declares the effects it may perform, effects are inferred from source, so a
package that quietly starts touching the network fails to build *in every
consumer*. That is the reason to have a package system rather than a
directory of files, and it was done before this workstream started.

What has been built since: `packages/`, and `docs/packages-plan.md` is the
programme it follows. Every one has a demo the corpus builds three ways, so
`difftest`, `leakcheck` and `crosstest` hold three engines to one answer on
every library on every run. **Twenty-six of the thirty-four declare no
capability at all.**

Two rules shape all of them.

**Split reach from computation.** `path` is arithmetic and declares nothing;
walking a directory is `fs`. `url` is text; dialling is `http_client`. `http`
parses and routes and *cannot open a socket*; listening is `http_server` and
sending is `http_client`. `time` is a calendar; reading the clock is one `io`
call in the consumer's `main`. `random` is a seeded generator; entropy is
`random_bytes`. In every case the half that reaches is a separate name a
consumer has to ask for, and the compiler holds them to the difference.

**Prefer an effect to a capability.** `log`, `metrics` and `trace` are the
clearest cases: a library that logs by calling `print` declares `io`, and
every consumer then depends on something that can write anywhere. A library
that logs by *performing* `Log` declares `Log` — and what logging means is
decided by the line that installs the handler. `packages/log-demo`'s `work`
function logs from three calls deep with the row `/ {Log}`, and therefore
cannot print, cannot open a file, cannot reach the network.

**Checked against the outside world, not against itself.** A codec tested
only against its own output agrees with nothing. So: `crypto_hash` against
FIPS 180-4, RFC 4231 and RFC 6070; `websocket` against RFC 6455's own accept
value; `postgres` SCRAM against RFC 7677's proof *and* server signature;
`oauth2` PKCE against RFC 7636; `protobuf` against Google's published byte
sequences; `url` resolution against RFC 3986 section 5.4's table.

**What is still missing.** A remote registry — nothing fetches, publishes or
verifies over a network. `mongo`. And everything under "Blocked, and on what"
in the plan: TLS, and so HTTPS, gRPC, `wss://` and OAuth2 against a real
provider; a task that outlives its block, and so an async runtime, a thread
pool and connection pooling; a monotonic clock; a sleep; process metrics.

**What writing them found.** Eight defects in the language and its gates —
F72 to F79 — every one by writing a library rather than by a gate. A hex
literal could not express a bit pattern with the top bit set, which is where
every published hash constant lives. A handler a call answers could not be
passed as an argument. A `match` whose live arms all yield nothing could not
be typed. A call into another module could not be typed where the type had to
be inferred. **Two gates counted the programs they had not checked and passed
anyway.** A generic container could not be built empty. Two packages could
not both declare a `Bad`. A boxed place assigned to itself was never freed.

That last group is the argument for this workstream beyond the libraries: a
language finds out what it is missing when somebody builds on it.

## 4. Is the compiler ready

**Yes, with the limits stated.**

Three engines — interpreter, bundle, native — agree byte for byte on 227
programs, on stdout, exit code and diagnostics, and a fourth (wasm32) agrees on
the 189 of them that do not need a filesystem, sockets or threads. The same 227
cross-compile to x86_64 and agree there too. Zero open defects. The list of
shapes the native backend cannot compile is one entry long — polymorphic
recursion, which is what monomorphization is and no amount of work removes.

What "ready" does not mean: it has no users but this project, no third-party
libraries, and a standard library of 81 methods. It runs at 0.45× native
against Rust. Defects are still being found at roughly one per working session,
falling but not zero.
