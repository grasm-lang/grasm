<p align="center">
  <img src="assets/hero.svg" alt="Grasm — nothing happens that the code didn't declare" width="100%">
</p>

<p align="center">
  <a href="https://grasm-lang.github.io"><img src="https://img.shields.io/badge/grasm--lang.github.io-visit-3DDC97?style=for-the-badge&labelColor=0A101C" alt="website"></a>
  <img src="https://img.shields.io/badge/v1.1.0-pre--alpha-17A2A2?style=for-the-badge&labelColor=0A101C" alt="v1.1.0">
  <img src="https://img.shields.io/badge/self--hosting-S8%20of%20S9-F6AE2D?style=for-the-badge&labelColor=0A101C" alt="self-hosting: S8 of S9">
  <img src="https://img.shields.io/badge/GC-none-3DDC97?style=for-the-badge&labelColor=0A101C" alt="no GC">
</p>

<p align="center">
  <b><a href="https://grasm-lang.github.io">Website</a></b> ·
  <a href="https://grasm-lang.github.io/playground.html">Playground</a> ·
  <a href="docs/overview.md">Guide</a> ·
  <a href="spec/">Spec</a> ·
  <a href="docs/northstar.md">Scorecard</a>
</p>

---

# Three stories. One compiler that answers them.

<br>

### I. The dependency you never read

> A patch release lands. Four thousand lines of transitive diff, and one of them
> opens `~/.aws/credentials`. You find out from a bill.

<img src="assets/capability.svg" alt="Packages that declare no capabilities are refused at the manifest boundary" width="100%">

In Grasm, reach is a **declaration**, not a convention. A package that says
`capabilities = []` cannot open a socket, read a file, spawn a process or ask
the time — and it isn't a sandbox catching it at runtime, it's the compiler
refusing to build it.

```toml
# grasm.toml — a crypto library has no business outside its own arithmetic
[package]
name = "crypto-core"
capabilities = []          # ← the whole attack surface, on one line
```
```
error: package `crypto-core` performs [io] but permits []
```

<br>

### II. "Cannot reproduce"

> It crashed once, at 03:14, on a machine you can't log into. You have a stack
> trace, a log line, and a week.

<img src="assets/replay.svg" alt="A recorded journal replays byte-identically on the interpreter, native binary and wasm" width="100%">

Every read from outside the program goes through **44 journaled builtins**,
and `GRASM_RECORD` writes down what each one returned. That journal *is* the bug
report. `GRASM_REPLAY` runs it back — on a different engine, in a sandbox with
no sockets, no filesystem, no clock and no processes.

```sh
GRASM_RECORD=crash.j ./server      # production
GRASM_REPLAY=crash.j grasm run server.gr   # your laptop, same bytes out
```

<br>

### III. The backend that miscompiles

> It worked in the interpreter. It worked in debug. Then the optimiser got it,
> and now one branch in ten thousand is wrong.

<img src="assets/engines.svg" alt="Three engines on two architectures produce one answer" width="100%">

Three engines, two architectures, **one answer**. Every program in the corpus
runs on all of them every gate run, and the outputs are compared byte for byte.
When they disagree, nothing ships — that is the release blocker, not a filed
issue.

<br>

---

## The niche nobody else fills

- 🔒 &nbsp;**A library you can prove is inert.** Capabilities are per package and checked at the boundary. "This crate cannot phone home" stops being a promise.
- 🔁 &nbsp;**A bug report that runs.** A recording made by a native binary replays inside a wasm module that has no OS underneath it.
- 📐 &nbsp;**A spec small enough to hand to a machine.** `spec/` fits in one context window *on purpose* — designed to be a language an agent gets right, not one it guesses at.
- ⚖️ &nbsp;**Differential correctness as the product.** Not a test suite bolted on. The thing being sold.

## If you already write something else

| You write | What carries over | What's gone |
|---|---|---|
| **Rust** | ownership, ADTs, exhaustive matching, typed error flow | lifetime annotations, `Send`/`Sync` puzzles, six separate tools |
| **Go** | fast builds, one static binary, cheap concurrent tasks | the garbage collector, `interface{}`, nil at runtime |
| **Zig** | nothing hidden, no allocation you didn't ask for | hand-threaded allocators and `defer` for memory |
| **TypeScript / Python** | ADTs, inference, iteration speed | a runtime to install, and type errors that wait until production |

Honest about the trade: on eight benchmark programs Grasm's geomean is
**0.965× of Rust `-O3`**. That is **3.5% behind**. It is not a win over Rust,
and the chart below shows exactly which program carries the deficit.

## Packages: Blade, and no second tool to install

Every other ecosystem hands you a second program — `pip`, `npm`, `cargo`,
`maven`, `gem`, `conan`. Grasm folds it in, the way `go`, `zig`, `deno` and
`bun` do:

| Elsewhere | Here |
|---|---|
| `pip install` · `npm i` · `cargo add` | `grasm add` |
| `cargo publish` · `npm publish` · `gem push` | `grasm publish` |
| `cargo update` · `npm ci` · `bundle install` | `grasm deps` → `grasm.lock` |
| crates.io · PyPI · npm registry | **Blade** |

**Blade** is the registry packages live in. It is a name for the place, not a
program to install — the commands above are subcommands of the one binary you
already have, and every package in Blade declares its capability ceiling, which
is the thing no other registry can tell you.

## Is this a Rust wrapper?

No. A `.gr` file never becomes Rust source and never touches `rustc`.

| Stage | What it produces |
|---|---|
| `grasm_syntax` | its own lexer, parser and formatter |
| `grasm_sema` | its own types, **effect rows**, ownership, exhaustiveness |
| `grasm_interp` | the normative answer every other engine must match |
| `grasm_native` | **Cranelift IR → an object file** |
| `grasm_sys` | the operating system over **raw syscalls and libc — no `std`** |

Cranelift is a code generator, the way LLVM is one for Clang, Swift and Rust
itself. Nobody calls Rust C++-based because LLVM is written in C++.

**Nothing of the host survives into what you ship.** All four crates a compiled
program links are `#![no_std]`; `grasm_rt_abi` supplies Grasm's own
`#[global_allocator]` and `#[panic_handler]`. A hello-world is **85.7 KB**
and `nm` finds no Rust standard library in it.

The compiler you download is *written in* Rust, the way Rust's first compiler
was written in OCaml and Go's in C. That is a bootstrap, and it is the one
thing here designed to be replaced. The replacement exists:
[`compiler/`](compiler/) is a second compiler written in Grasm — frontend,
arm64, x86-64 and wasm32 backends, Mach-O and ELF writers, its own linker, its
own runtime, the `grasm` command — and **it builds itself**. Built by the Rust
toolchain it is stage 1; stage 1 builds it again with no Rust runtime, no `cc`
and no `grasm` on the way; that stage 2 builds a stage 3 that is the same bytes,
54 artifacts of 54. It does not ship yet: until the cutover the Rust toolchain
stays the oracle every gate compares against and what a release is built with.
[`docs/self-hosting.md`](docs/self-hosting.md) has the plan and every gate's
line.

## Speed, stated honestly

<img src="assets/speed.svg" alt="Eight benchmark programs against Rust -O3, geomean 0.965×" width="100%">

## Measured

| Gate | What it compares | Programs |
|---|---|---:|
| `difftest` | interpreter · native · bundle agree | **648** |
| `crosstest` | cross-compiled x86_64, identical | **648** |
| `wasmtest` | identical in the browser engine | **508** |
| `ProjectGold` | whole applications, every engine | **44** |
| `fuzz_laws` | algebraic laws against arithmetic | **218** |
| `depth` | nested calls reached, 10,000 promised | **31,093** |

Those six from the run of 25 October 2026 (`site/facts.json`). The compiler in
Grasm, from its runs of 25 and 26 November 2026:

| Gate | What it compares | Result |
|---|---|---:|
| `fixpoint` | stage 3 against stage 2, byte for byte | **54 of 54** |
| `s3subset` | its arm64 build against the Cranelift build | **648**, 0 mismatched |
| `wasmsubset` | its wasm build against `grasm run` and native | **533**, all |
| `playtest` | its playground module against the Rust one | **508**, all |
| `elfsubset` | its Linux build, per triple, in a local VM (13 Nov) | **647 of 648** |

| Goal | Target | Measured |
|---|---|---|
| Hello-world static binary | ≤ 300 KB | **85.7 KB** |
| Frontend throughput | ≥ 500k LOC/s | **535–547k** |
| Edit → diagnostic | ≤ 50 ms | **p50 21 ms** |
| Lightweight tasks | 1M per node | **1,000,000 in 22 s** |

> **One commitment is missed on purpose.** The specification is 70 pages
> against the 50 it binds itself to. Its test allows 70 and no more, and prose
> moved out of the spec, not a raised ceiling, brought it back there. Raising
> either number would meet it by redefining it.
> [`docs/known-issues.md`](docs/known-issues.md) prices the fix.

## Taste

```grasm
effect Clock { fn now() -> i64 }

type Shape = | Circle(r: f64) | Rect(w: f64, h: f64)

fn area(s: Shape) -> f64 {
    match s {                        // exhaustive, or it does not compile
        Circle(r) -> 3.14159 * r ^ 2
        Rect(w, h) -> w * h
    }
}

fn load(path: str) -> str ! IoError / {io} {   // the `/ {io}` is enforced
    try read_file(path)
}

fn main() {
    with Clock handled_by ClockHandler{ now: || 1000 } {
        print("{Clock.now()}: {[Circle(1.0), Rect(2.0, 3.0)].map(|s| area(s)).sum()}")
    }
}
```

## Built in it

**GrasmWire** — a VPN protocol written entirely in Grasm. Field arithmetic,
cipher, authenticator, hash, key schedule, handshake, protocol: **3,099 lines,
nothing underneath but this compiler.**

Checked against RFC 8439, 7693, 7748 and 5869, then checked again against an
implementation sharing no code with it — 753 random cases a run, whole handshake
included. Two processes bring a tunnel up over real UDP, rekey, follow a roaming
peer, and refuse a replayed handshake.

**Every package below the endpoint declares no capabilities at all.** The curve,
the hash, the AEAD and the handshake cannot open a socket or read a clock.
Randomness and time arrive as arguments. → [`docs/grasmwire.md`](docs/grasmwire.md)

## Start

```sh
curl -fLO https://github.com/grasm-lang/grasm/releases/latest/download/grasm-macos-arm64.tar.gz
tar -xzf grasm-macos-arm64.tar.gz
./grasm-macos-arm64/grasm run hello.gr
```

**One binary, seventeen subcommands**, with the runtime library that native
builds link beside it in the archive. On its own it checks, formats, lints,
tests and runs programs; v1.1.0's `grasm build --native` also needs a C
toolchain's `cc` to link with. Grasm's own linker, `grasm-link`, now exists:
built from source, a native build on arm64 macOS links with it and runs no C
compiler or system linker, and the arm64 macOS archive carries it from the
release after v1.1.0. Intel Macs take `grasm-macos-x86_64.tar.gz`; checksums
are in `SHA256SUMS` beside them. There is no Linux release.
[All downloads →](https://github.com/grasm-lang/grasm/releases/latest)

Building from source needs a Rust toolchain to bootstrap the compiler, the way
every language starts, until the cutover —
[`docs/overview.md`](docs/overview.md) has it, and nothing of that toolchain
reaches what you ship.

| What | Where |
|---|---|
| 🌐 Website, tour, playground | **[grasm-lang.github.io](https://grasm-lang.github.io)** |
| 📘 Full guide, install, every subcommand | [`docs/overview.md`](docs/overview.md) |
| 📐 Language specification | [`spec/`](spec/) |
| 📊 Goals against measurements | [`docs/northstar.md`](docs/northstar.md) |
| 🗺️ Strategy and roadmap | [`MASTERPLAN.md`](MASTERPLAN.md) |
| 🔁 The compiler in Grasm, stage by stage | [`docs/self-hosting.md`](docs/self-hosting.md) |
| 🐛 Open defects, with reproducers | [`docs/known-issues.md`](docs/known-issues.md) |
| 🔎 101 findings from building it | [`ProjectGold/FINDINGS.md`](ProjectGold/FINDINGS.md) |

<p align="center"><sub>Pre-alpha. Released for macOS (arm64; x86_64, exercised under Rosetta). Linux builds have run only in a local virtual machine, never on CI, and there is no Linux release.<br>Every number on this page comes from the gate run that produced it, recorded in <code>site/facts.json</code>, <code>docs/northstar.md</code> and <code>docs/self-hosting.md</code>.</sub></p>
