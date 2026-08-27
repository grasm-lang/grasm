# Road to a first release

Four workstreams, running alongside the defect loop that has been going for
weeks. The loop is not paused for these: it is what keeps them honest, and
every item below lands with gates green or it does not land.

Recorded 2026-08-27. Status is measured, not estimated — each line says how it
was checked.

## Where this starts from

Measured on 2026-08-27, seven days after 2026-08-20:

| | 20 Aug | 27 Aug |
|---|---|---|
| findings recorded | 31 | 67 |
| generators | 9 | 14 |
| corpus programs | 278 | 287 |
| specification clauses | 230 | 234 |

Ninety-two commits, +32,552 / −8,051 lines. Thirty-six defects found and
fixed, twelve of them S1 — wrong answers, crashes, engine disagreements. Zero
open. Every gate green.

## 1. The website

**Status: not started.** There is `ProjectGold/web/`, which is a wasm host page
for running the corpus in a browser. There is no site.

What it needs: a landing page that says what the language is and why it exists;
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
binary size 328 KB against a 300 KB target; the reference count on every boxed
value, which is atomic because a `parallel` body may hold one.

Quality is the loop. It does not stop.

## 3. Packages and the libraries a first release needs

**Status: the mechanism exists; the libraries do not.**

`crates/grasm_pkg` is 1,991 lines and already does manifests, dependency
resolution, lockfiles, path dependencies, a local registry under
`.grasm/registry`, and the capability ceiling — a package declares the effects
it may perform, effects are inferred from source, so a package that quietly
starts touching the network fails to build *in every consumer*. That last part
is the reason to have a package system at all rather than a directory of files,
and it is the part that is done.

What is missing is a remote registry — fetching, publishing, verifying — and
the libraries themselves.

What exists today: `lib-json`, `lib-markdown`, `lib-sha256`, all inside
`ProjectGold/`, none published as packages.

What a first release needs, in the order a user meets them:

- **http** — a server and a client. This is the one that decides whether the
  language is usable for the work most people do. `ProjectGold/21-http-server`
  is 347 lines of working proof that the primitives are there; it is a program,
  not a library.
- **json** — exists as `lib-json`, needs to become a package with a stable API.
- **async/task** — `parallel`/`spawn` are in the language. What is missing is
  the library above them: timers, channels, select.
- **testing** — assertions and a runner beyond `test` blocks.
- **log**, **time**, **uuid**, **base64**, **regex** — the small ones every
  program reaches for.
- **sql** or at least a driver interface. `ProjectGold/28-sql-engine` is 583
  lines and shows the shape.

The comparison the request named — hyper, axum, actix, gin, fiber, chi, tokio —
is a comparison against ecosystems with years and thousands of contributors.
The honest target for a first release is not those. It is: one good HTTP
library, one JSON library, one async library, and the packaging to publish
them, so that the second and third HTTP libraries can be written by somebody
else.

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
