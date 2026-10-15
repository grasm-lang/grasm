<p align="center">
  <img src="assets/logo.svg" alt="Grasm" width="440">
</p>

<p align="center">
  <b>Rust-class safety at Go-class effort.</b><br>
  Effects you can audit · ownership without lifetimes · three engines that agree byte for byte
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-1.0.0-3DDC97?style=flat-square" alt="version 1.0.0">
  <img src="https://img.shields.io/badge/hello_world-85.7%20KB-3DDC97?style=flat-square" alt="85.7 KB">
  <img src="https://img.shields.io/badge/GC-none-3DDC97?style=flat-square" alt="no GC">
  <img src="https://img.shields.io/badge/engines-interpreter%20%7C%20native%20%7C%20wasm-17A2A2?style=flat-square" alt="three engines">
  <img src="https://img.shields.io/badge/gates-103%20green%20%C2%B7%201%20red-F6AE2D?style=flat-square" alt="gates">
</p>

---

## Why Grasm

| | Everywhere else | Grasm |
|---|---|---|
| **Dependency can read your disk** | Nothing stops it | `capabilities = []` and the compiler refuses. A tampered package fails as `performs [io] permits []` |
| **Memory safety costs lifetimes** | Annotations, or a GC | `own` / `inout`, value semantics, **no lifetime syntax and no collector** |
| **"Works on my machine"** | Logs and guesswork | `GRASM_RECORD` journals every read from outside. `GRASM_REPLAY` reproduces it — *on a different engine, in a sandbox with no sockets, no filesystem, no processes* |
| **Errors** | Exceptions or `Result` boilerplate | `T ! E` unions. Ignoring a fallible is a **compile error** |
| **Toolchain** | compiler + formatter + linter + LSP + fuzzer + bench | **one binary**, 15 subcommands |
| **Does the backend match the interpreter?** | Hope | **511 programs proven identical** across interpreter, native and wasm — and again on x86_64 |

## The niche nobody else fills

**A library you can prove is inert.** Capabilities are per package and checked at the boundary, so "this crypto crate cannot phone home" stops being a promise.

**A bug report that replays.** The journal is the whole interface to the outside world — 28 builtins across all four engines. A recording made by a native binary replays byte-identically *inside a wasm module that has no OS underneath it*.

**A spec small enough to hand to a machine.** `spec/` fits in one context window on purpose — the language is designed to be one an agent gets right, not one it guesses at.

**Differential correctness as a product, not a test suite.** Three engines, two architectures, one answer. When they disagree, that is the release blocker.

## Proof, measured 15 October 2026

| Gate | Checks | Result |
|---|---|---:|
| `difftest` | interpreter · native · bundle identical | **511** |
| `crosstest` | cross-compiled x86_64, identical under Rosetta | **511** |
| `wasmtest` | identical in the browser engine | **415** (96 need an OS) |
| `ProjectGold` | whole applications agreeing on every engine | **44** |
| `fuzz_laws` | algebraic laws held against arithmetic | **205** |
| `depth` | nested calls reached, against 10,000 promised | **31,444** |

| Goal | Target | Measured |
|---|---|---|
| Hello-world static binary | ≤ 300 KB | **85.7 KB** |
| Frontend throughput | ≥ 500k LOC/s | **535–547k** |
| Edit → diagnostic | ≤ 50 ms | **p50 21 ms** |
| Lightweight tasks | 1M per node | **1,000,000 in 22 s** |
| Speed vs Rust (geomean, 8 programs) | ≥ 0.95× | **0.965×** — five of eight at or above parity |

Not a win over Rust. `0.965×` means **3.5% behind**, and `docs/northstar.md` names which program carries it and why.

**One gate is red on purpose:** the specification is 71 pages against the 50 it binds itself to. Raising the ceiling would meet the number by redefining it. [`docs/known-issues.md`](docs/known-issues.md) has the cost of closing it.

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

**GrasmWire** — a VPN protocol written entirely in Grasm. Field arithmetic, cipher, authenticator, hash, key schedule, handshake, protocol. **3,099 lines, nothing underneath but this compiler.**

Checked against RFC 8439, 7693, 7748, 5869 and the XChaCha20 draft, then checked again against an implementation sharing no code with it — 753 random cases a run, whole handshake included. Two processes bring a tunnel up over real UDP, rekey, follow a roaming peer, refuse a replayed handshake.

**Every package below the endpoint declares no capabilities at all.** The curve, the hash, the AEAD and the handshake cannot open a socket or read a clock. Randomness and time arrive as arguments. → [`docs/grasmwire.md`](docs/grasmwire.md)

## Start

```sh
cargo build --release --workspace          # the runtime needs --workspace
./target/release/grasm run examples/hello.gr
./scripts/allgates.sh                      # 104 gates, ~2 h
```

| | |
|---|---|
| Full guide, install, every subcommand | [`docs/overview.md`](docs/overview.md) |
| Language specification | [`spec/`](spec/) |
| Goals against measurements | [`docs/northstar.md`](docs/northstar.md) |
| Strategy and roadmap | [`MASTERPLAN.md`](MASTERPLAN.md) |
| Open defects, with reproducers | [`docs/known-issues.md`](docs/known-issues.md) |
| 101 findings from building it | [`ProjectGold/FINDINGS.md`](ProjectGold/FINDINGS.md) |

<p align="center"><sub>Pre-alpha. One platform exercised (arm64 macOS); Linux type-checks and has never been linked.<br>Every number above comes from the gate run that produced it, recorded in <code>site/facts.json</code>.</sub></p>
