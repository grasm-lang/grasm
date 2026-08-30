# Grasm in a browser

`cargo build --release --target wasm32-unknown-unknown -p grasm_wasm` produces a
1.3 MB module that is the **reference interpreter** — the same engine `grasm run`
uses — running on a different machine.

That distinction is the whole design. A wasm *code generator* would be a fourth
engine, and a fourth engine has to enter the agreement gate before anything it
says can be believed. This is not a fourth engine: `grasm_interp` depends on
`grasm_syntax` and `grasm_sema` and nothing else, neither of which touches the
host, so the whole frontend and the whole semantics come across with no
reimplementation. Nothing here *can* disagree with `grasm run` about what a
program means, because it is `grasm run`.

It is still held to the gate — `scripts/wasmtest.sh` — because "cannot disagree"
is an argument and the project runs on measurements. **183 identical, 0
mismatched, 23 skipped**, each skip naming the host facility it needs.

## What the host does not have

`wasm32-unknown-unknown` has no threads, no sockets, no filesystem and no clock.
Each is a refusal at run time with the ordinary error, never a wrong answer:

| | on this target |
|---|---|
| `par_map` | runs on one core; the answer is the same, the parallelism is not |
| `tcp_*` ([L-LIB-NET]) | cannot bind — `Err(IoFailed)` |
| `read_file` / `write_file` | fail as they would on a machine with no such file |
| `extern fn` | no linker and no loaded process to search; refused by name |
| `now_millis` | **imported from the embedder**, see below |
| recursion depth | the module's stack is fixed at link time, so [E-RECURSION-DEPTH]'s depth is not reachable |

The clock is the one that could not be left alone. `std::time::SystemTime::now()`
does not merely fail on this target — it *panics*, and on a target with no
unwinding a panic is an abort, which takes the module down and loses the
program's output with it. A browser has a perfectly good clock, so the module
imports one rather than faking a zero. This was found by running
`examples/stdlib_tour.gr`, which ends by asking what time it is.

## One instance per program

An embedder that runs more than one program must **instantiate per run**, not
per page. A wasm trap leaves its instance unusable, so a single program that
exhausts the stack takes every later run down with it — which is exactly what
happened the first time the showcase ran all twenty-six: project 08 traps by
design, and the sixteen programs after it reported failures they had not caused.

`WebAssembly.compile` once and `new WebAssembly.Instance(module, imports)` per
program costs about a millisecond and confines a trap to the program that
caused it. `ProjectGold/web/index.html` is the reference for this;
`scripts/wasm/run.mjs` gets away with a single instance only because it runs one
program per process.

## The ABI

Three exports and no binding generator. `wasm-bindgen` would work and is
deliberately not used: a build tool nobody has installed is a barrier in front of
the only evidence this project has, and the surface here is small enough to write
out.

```
grasm_alloc(len) -> ptr        reserve len bytes for the caller to write
grasm_free(ptr, len)           release what alloc or run returned
grasm_run(ptr, len) -> ptr     run the UTF-8 source, answer with a frame
```

The frame, little-endian `u32`s:

```
[0..4)   total byte length, including these four
[4..8)   exit code, as an unsigned two's-complement i32
[8..12)  length of stdout
[12..)   stdout, UTF-8
then     length of the diagnostic text, then the text, UTF-8
```

Four imports are required. `env.grasm_now_millis` (answer with `Date.now()`),
`env.grasm_monotonic_millis` (answer with `performance.now()` — it must only
increase), `env.grasm_sleep_millis` (spin until the deadline; a call from wasm
is synchronous and the host cannot yield inside one), and `env.dlsym` (answer
with 0 — it is reached through Rust's `std` and can never be called, since a
Grasm program cannot reach a C symbol without a linker).

The list above is checked against the module rather than trusted:
`scripts/wasmtest.sh` reads the import section and fails if an embedder in this
tree does not name every entry. Adding a host function without teaching the
embedders about it broke every one of the 256 programs at once, because a
missing import is an *instantiation* failure — nothing runs, so nothing points
at the cause.

`scripts/wasm/run.mjs` is the reference embedding, in about twenty lines. It is
the same twenty lines a web page needs, and the gate runs *it* rather than a
second copy, so the glue that is tested is the glue that ships.

## What it is for

Until now, running a Grasm program meant cloning a repository and building a Rust
workspace. That is a barrier in front of the only evidence the project has —
G1 and G10 measure whether the language is good to use, and both are unmeasured
because nobody outside this repository has run a Grasm program.

`grasm-calc` is the first thing on the other side of it: a calculator whose
arithmetic is 151 lines of Grasm, typechecked and evaluated in the browser tab,
with no server, no Python and no Rust. Each press of `=` re-lexes, re-parses and
re-typechecks the whole program before running it, and takes 1–12 ms.

One thing it cannot do is serve itself. `server.gr` still works, but a Grasm
socket carries `str` ([L-LIB-NET]) and `grasm.wasm` is binary, so the page needs
an ordinary static host. Serving bytes needs a byte type, which the language does
not have.
