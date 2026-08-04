# GRASM — Master Plan
## A Next-Generation Systems Language to Succeed Go and Rust

**Document class:** Strategic & Technical Blueprint
**Version:** 0.3.0 (2026-08-04) — adds §19 Philosophy Alignment (kernel language, concision KPI, intent-first programming) and §20 Frontier Additions (secret types, cost contracts, protocol types, choreographies, mergeable data, sandboxed FFI)
**Status:** Founding document — pre-implementation
**Codename:** Grasm (working title)

---

# Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Vision, Mission, North-Star Goals](#2-vision-mission-north-star-goals)
3. [Market & Landscape Analysis](#3-market--landscape-analysis)
4. [Analysis of Current Leading Languages and Their Weaknesses](#4-analysis-of-current-leading-languages-and-their-weaknesses)
5. [Emerging Trends and Technology Evaluation](#5-emerging-trends-and-technology-evaluation)
6. [Problems in Current Languages → Grasm's Answers](#6-problems-in-current-languages--grasms-answers)
7. [Core Design Principles](#7-core-design-principles)
8. [Language Specification (Master Overview)](#8-language-specification-master-overview)
9. [Novel Features Not Found in Any Mainstream Language](#9-novel-features-not-found-in-any-mainstream-language)
10. [Compiler, Runtime & Toolchain Architecture](#10-compiler-runtime--toolchain-architecture)
11. [Developer Experience, Tooling & Ecosystem](#11-developer-experience-tooling--ecosystem)
12. [Performance, Scalability, Security, Reliability](#12-performance-scalability-security-reliability)
13. [Implementation Roadmap: Phases & Milestones](#13-implementation-roadmap-phases--milestones)
14. [Engineering Requirements & Infrastructure](#14-engineering-requirements--infrastructure)
15. [Adoption Strategy](#15-adoption-strategy)
16. [Risks, Challenges, Limitations & Mitigations](#16-risks-challenges-limitations--mitigations)
17. [Long-Term Evolution & Research Directions](#17-long-term-evolution--research-directions)
18. [Appendices](#18-appendices)

---

# 1. Executive Summary

**Thesis.** The systems programming world is caught between two poles. Go delivers
simplicity, fast builds, and effortless concurrency — but pays with GC pauses, a weak
type system, and no path into kernels, embedded, or hard real-time. Rust delivers
memory safety at zero runtime cost — but pays with a brutal learning curve
(2–6 months to productivity), slow compiles, and an async model that splits the
ecosystem into colored functions. Every serious new entrant (Zig, Mojo, Carbon, Vale,
Hylo, Austral) attacks one axis of this trade-off. **None attacks all of them
simultaneously, and none is designed for the era the industry is actually entering:
regulated memory safety, WASM-everywhere deployment, heterogeneous CPU/GPU compute,
and AI agents as first-class writers and readers of code.**

**Grasm** is a statically typed, ahead-of-time compiled systems language with:

- **Memory safety without a borrow checker and without garbage collection** — via a
  tiered model: mutable value semantics by default, compiler-inferred regions, and
  generational references as the safe escape hatch. No lifetime annotations exist in
  the safe language. Target: 95% of Rust's performance at 20% of its cognitive cost.
- **An inferred algebraic effect system** that unifies async/await (no function
  coloring), error handling, dependency injection, testing/mocking, and — critically —
  **capability-based security for the entire dependency tree** (a package that never
  declared network access can never touch the network).
- **A compiler that is a queryable incremental database, not a batch program** —
  sub-50 ms edit-to-diagnostic at 1M+ LOC, with the same query engine powering the
  LSP, refactoring tools, and a structured API for AI agents.
- **Deterministic execution and time-travel debugging as runtime primitives**, built
  on the effect boundary: record every effect, replay any production failure
  bit-for-bit on a laptop.
- **First-class WASM Component Model and heterogeneous (CPU/GPU/SIMD) targets** from
  a single source language.
- **Gradual formal verification**: contracts and refinement types that are
  SMT-proved where possible, runtime-checked where not, and erased where proven.

The plan targets a 7–10 year horizon: 18 months to a public prototype, 4 years to a
production 1.0, with a beachhead strategy (cloud/WASM infrastructure tooling) before
general-purpose expansion. Regulatory tailwinds are real and dated: CISA/NSA joint
guidance on memory-safe languages (June 2025), U.S. federal memory-safety roadmap
expectations (2026), and the EU Cyber Resilience Act with reporting obligations from
September 2026. A new language that is memory-safe *and* radically easier than Rust
arrives into a market that is being *pushed* off C/C++ by law, not just preference.

---

# 2. Vision, Mission, North-Star Goals

## 2.1 Vision

> **The first language where the safest program is also the easiest one to write —
> and where humans and AI systems are equally first-class authors.**

## 2.2 Mission

Build a general-purpose systems language, toolchain, and ecosystem that makes
Go-class productivity and Rust-class safety/performance the *same* experience
instead of a trade-off, and that treats determinism, capability security, and
machine-readable semantics as foundations rather than add-ons.

## 2.3 North-star goals (measurable, testable)

| # | Goal | Metric | Target |
|---|------|--------|--------|
| G1 | Learnability | Time for a Go/TypeScript dev to ship a real service | ≤ 1 week (Rust: 2–6 months) |
| G2 | Performance | Geomean vs Rust on systems benchmark suite | ≥ 0.95× |
| G3 | Compile speed | Clean build throughput / incremental edit-to-diagnostic | ≥ 500k LOC/s frontend; ≤ 50 ms incremental |
| G4 | Memory safety | Safe subset guarantees | 100% spatial + temporal + data-race safety, no GC |
| G5 | Binary output | Hello-world static binary | ≤ 300 KB, zero runtime deps |
| G6 | Concurrency | Structured, data-race-free, uncolored | 1M+ tasks/node; same fn works sync + async |
| G7 | Supply chain | Capability-sandboxed deps, reproducible builds | 100% of registry packages |
| G8 | Debuggability | Production failure → local deterministic replay | ≥ 99% of crashes replayable |
| G9 | Interop | C ABI import overhead | Zero-cost, no hand-written bindings |
| G10 | Adoption (yr 5) | Production companies / registry packages | ≥ 500 companies, ≥ 10k packages |

## 2.4 Non-goals

- Not a scripting language; no dynamic-typing mode.
- Not a Java/.NET-style managed VM platform.
- Not source-compatible with C++ (interop yes, migration dialect no — that is
  Carbon's lane).
- No mandatory monads, no academic ceremony: every advanced feature must have a
  "you can ignore this and still ship" default.

---

# 3. Market & Landscape Analysis

## 3.1 Structural forces (2025–2027)

1. **Memory safety is becoming law, not taste.**
   - CISA/NSA joint guide *"Memory Safe Languages: Reducing Vulnerabilities in
     Modern Software Development"* (June 24, 2025) formalizes MSL adoption guidance.
   - CISA recommends published memory-safety roadmaps (end of 2025); absence now
     reads as a compliance gap in federal procurement (~$100B+/yr market).
   - EU Cyber Resilience Act: vulnerability reporting obligations from Sept 2026,
     main obligations Dec 2027; manufacturers become liable for exploitable memory
     bugs. Germany's BSI names Rust directly in NIS-2 guidance.
   - ~70% of severe CVEs in large C/C++ codebases remain memory-safety bugs — a
     structural, not educational, problem.
   - **Implication:** the addressable market for a memory-safe systems language is
     being expanded by regulation. Rust is currently "the only production-ready
     option" for zero-overhead safety — a monopoly on a painful experience. That
     monopoly is the opening.

2. **The successor-language race is crowning winners of single niches, not the war.**
   - Zig approaches 1.0 (2026): brilliant toolchain and comptime, but *not*
     memory-safe — misaligned with the regulatory decade. Its single-compilation-unit
     + incremental strategy is state-of-the-art engineering to learn from.
   - Mojo 1.0-beta (May 2026, Modular — acquired by Qualcomm): proves demand for
     Python-ergonomics + MLIR-based heterogeneous compute, but is AI-workload-first,
     corporate-controlled, and compiler still closed at time of writing.
   - Carbon: explicitly a C++ migration vehicle; its memory-safety story converges
     on "largely match Rust" — inheriting Rust's cognitive cost.
   - Vale (generational references), Hylo (mutable value semantics), Austral
     (linear types + capabilities), Ante (effects): each validates one pillar of
     Grasm's design in isolation. None has critical mass or a full-spectrum plan.
   - **Implication:** the "safe + simple + fast + governable" quadrant is empty.

3. **WASM is becoming the universal deployment substrate.** WASI 0.3 (2026) adds
   native async; Component Model matures toward 1.0 (late 2026/2027); production
   users include Cloudflare, Fastly, Shopify, Fermyon. Capability-based imports in
   WASI are philosophically identical to Grasm's effect-capability model — a
   perfect-fit compile target no incumbent treats as first-class.

4. **AI is now a co-author.** Code generation shifts value from "easy to write" to
   "easy to verify, constrain, and review." Languages with strong static guarantees
   get *more* valuable (the compiler reviews the AI's code); languages whose
   toolchains expose structured semantics to agents get a compounding advantage.
   No mainstream language was designed for this.

5. **Hardware is heterogeneous.** A 2026 production system spans x86 servers,
   ARM edge nodes, NVIDIA/AMD GPUs, and custom accelerators. C++/CUDA demands
   parallel codebases per target; Mojo/MLIR shows single-source portability is
   possible. General-purpose languages ignore this at their peril.

## 3.2 Competitive positioning map

```
                    SAFETY (static, no GC)
                          ▲
                          │
             Rust ●       │        ◎ GRASM (target)
                          │
          Austral ●       │   ● Vale / Hylo (early)
                          │
  ── HARD TO LEARN ───────┼──────── EASY TO LEARN ──▶
                          │
            C++ ●         │   ● Go (GC; safe-ish, simple)
                          │
             Zig ●        │   ● Mojo (AI-niche)
                          │
                     (unsafe or GC)
```

Grasm's claim: the upper-right quadrant — full static safety with Go-class
approachability — is the most valuable unoccupied position in programming languages.

---

# 4. Analysis of Current Leading Languages and Their Weaknesses

## 4.1 Summary matrix

| Language | Core strength | Fatal weaknesses (for the next decade) |
|---|---|---|
| **C** | Ubiquity, ABI lingua franca | No safety; regulation is actively retiring it |
| **C++** | Performance + ecosystem | Unbounded complexity; UB everywhere; committee paralysis; safety retrofits (profiles) too little too late |
| **Rust** | Zero-cost memory/thread safety | 2–6 month learning curve; lifetime annotations leak into APIs; slow compiles; colored async + runtime fragmentation (tokio vs async-std); `unsafe` prevalence in real dependency graphs |
| **Go** | Simplicity, fast builds, goroutines | GC pauses & 10× memory overhead vs Rust (Discord case); zero values + nil; weak generics (no method type params); no sum types; `if err != nil` verbosity — and the Go team's 2025 decision to *never* add error-handling syntax; unusable for kernels/embedded/real-time |
| **Zig** | comptime, toolchain (zig cc), incremental compilation, SCU strategy | **Not memory-safe** — disqualifying for the regulated decade; manual memory management discipline; pre-1.0 churn |
| **Swift** | Value semantics done right, ARC ergonomics | Apple-gravity; ARC overhead; slow type checker; server story never landed |
| **Kotlin/Java/C#** | Enterprise ecosystems | JIT/VM + GC class: excluded from systems niche by design |
| **Mojo** | MLIR heterogeneous compute, Python interop | Single-vendor (Qualcomm/Modular); AI-inference-first; ownership model still Rust-like at the bottom; compiler not fully open |
| **Carbon** | Definition-checked generics, C++ interop discipline | Exists to serve C++ migration; safety model converges to Rust's costs; experimental, Google-dependent |
| **Vale / Hylo / Austral / Ante** | Each proves one Grasm pillar (gen-refs / MVS / linear+capability / effects) | Sub-critical funding, no tooling story, no ecosystem plan |
| **TypeScript/Node** | DX, ubiquity | Not a systems language; runtime cost; type system unsound |
| **Python** | ML ecosystem, learnability | Interpreter performance; GIL legacy; packaging chaos; not deployable as small static binaries |

## 4.2 The deep diagnosis — *why* these weaknesses persist

1. **Rust's complexity is not incidental — it is the price of first-class references.**
   Because references are first-class values that can be stored, returned, and
   nested, the compiler must reason about lifetimes explicitly, and that reasoning
   leaks into user-facing annotations (`<'a>`), trait bounds, and async (`Pin`,
   `Send + Sync + 'static`). Research languages (Hylo's mutable value semantics,
   safecpp's second-class references) show that demoting references from
   first-class status removes the entire lifetime-annotation surface while keeping
   zero-cost pass-by-reference. **Grasm adopts this as its foundation.**

2. **Go's simplicity is load-bearing on its GC.** Remove the GC and Go's entire
   design collapses; keep it and you inherit pause/footprint costs and lose the
   bottom of the stack. Simplicity must instead be earned by *inference*
   (the compiler doing the reasoning), not by *runtime machinery*.

3. **Function coloring is an unforced error.** Rust/C#/JS async split every
   ecosystem in two. OCaml 5, Koka, and the effect-handler literature demonstrate
   that concurrency delivered as an *effect* — handled by a scheduler, invisible in
   function signatures except as an inferred effect row — eliminates coloring
   entirely. Zig's 2026 "I/O as an interface" redesign is converging on the same
   conclusion from the systems side.

4. **Nobody owns supply-chain security at the language level.** npm/PyPI/crates
   attacks recur because any transitive dependency can execute arbitrary I/O at
   build time and runtime. WASI's capability-based imports prove the model;
   Austral proves capabilities can live in a language's type system. No mainstream
   language enforces "this JSON parser can never open a socket."

5. **Compilers are still batch programs with IDE afterthoughts.** rust-analyzer
   (Salsa) and the query-based-compiler literature settled the correct
   architecture: demand-driven, memoized, incremental queries powering compile,
   IDE, and analysis from one engine. Retrofitting this (Rust has tried for years)
   is brutally hard. It must be the day-one skeleton.

---

# 5. Emerging Trends and Technology Evaluation

| Trend | Maturity (2026) | Verdict for Grasm |
|---|---|---|
| **Algebraic effect systems** (Koka, Flix, OCaml 5, Effekt, Ante) | Research→early-production | **Adopt, with full inference + tail-resumptive compilation to zero-cost code.** The single highest-leverage feature: unifies async, errors, DI, mocking, capabilities. Danger: ergonomics for beginners — mitigated by inference and progressive disclosure (§8.5) |
| **Mutable value semantics** (Hylo; Swift lineage; safecpp draft) | Research-validated | **Adopt as Tier-0 memory model.** Removes lifetime annotations wholesale |
| **Generational references** (Vale) | Prototype-validated | **Adopt as Tier-2 safe escape hatch** for shared mutable graphs; ~1 branch per dereference, elidable via static analysis + regions |
| **Linear/affine types + Higher RAII** (Austral, Vale) | Validated | **Adopt** for resources (files, sockets, locks, capabilities): compiler guarantees cleanup/`use-exactly-once` |
| **Compile-time execution / comptime generics** (Zig) | Production-proven | **Adopt** a typed comptime (runs the full language at compile time) *plus* definition-checked interfaces (Carbon/Rust-style) so generic errors appear at definition, not instantiation |
| **Query-based incremental compilation** (Salsa, rust-analyzer, Roc, Zig SCU) | Production-proven | **Adopt as core architecture** (§10.1) |
| **WASM Component Model / WASI 0.3 async** | Standardizing, production pilots | **First-class compile target**; Grasm effects map 1:1 to WASI capability imports |
| **MLIR & retargetable heterogeneous compute** (Mojo) | Production in AI serving | **Adopt MLIR as the optimizing mid-end**; `kernel` functions retarget CPU/SIMD/GPU (§9.8) |
| **SMT-backed refinement types / gradual verification** (Liquid Haskell, Dafny, Verus, F*) | Research→specialist production | **Adopt gradually**: contracts always; SMT proof opportunistic; never block the build on the prover (§9.5) |
| **Deterministic execution & record/replay** (antithesis-style testing, FoundationDB methodology) | Proven in elite shops | **Productize in the runtime** — effects give a natural record boundary (§9.6) |
| **AI-assisted development** | Mainstream | **Design for it**: machine-readable diagnostics, compiler query API, semantic diff, verifiable contracts as guardrails for generated code (§9.7) |
| **Hermetic, reproducible, sandboxed builds** (Bazel, Nix lineage) | Mainstream in big tech | **Default-on** in the single toolchain |

Rejected technologies: tracing GC (excludes bottom of stack), full dependent types
(ergonomics cliff), JIT-first execution (breaks predictability + tiny binaries),
green-threads-only runtime without effects (repeats Go's ceiling), C++-style
template metaprogramming (unbounded complexity).

---

# 6. Problems in Current Languages → Grasm's Answers

| # | Industry problem | Where it bites | Grasm's answer |
|---|---|---|---|
| P1 | Memory unsafety is 70% of critical CVEs | C/C++ everywhere | Statically safe by construction (tiered ownership); `unsafe` exists but is capability-gated and auditable per-package |
| P2 | Safety currently costs months of learning | Rust | No lifetime annotations in the safe language; value semantics default; inference does the reasoning |
| P3 | GC pause/footprint tax | Go, Java, C# | No GC. Values + regions + generational refs; deterministic destruction (RAII) |
| P4 | Colored async splits ecosystems | Rust, JS, C#, Python | Concurrency is an effect; one function works sync, async, parallel, or replayed depending on handler |
| P5 | Error handling: verbose (Go) or exception-invisible (C++/Java/Python) | All | Typed error unions `T ! E` + `try` propagation + exhaustive matching; errors are values, signatures are honest, syntax is 1 char |
| P6 | Slow builds destroy iteration loops | Rust, C++, Scala | Query-based incremental compiler, parallel-by-default, Cranelift debug backend, binary module interfaces; ≤50 ms incremental target |
| P7 | Supply-chain attacks via transitive deps | npm, PyPI, crates | Capability-scoped packages: effects declared in manifest, enforced by type system transitively; sandboxed build scripts; signed reproducible artifacts |
| P8 | Data races | C++, Go (partially) | Exclusivity rules + `Share` types + effect-tracked spawn = data-race freedom in safe code |
| P9 | Nil/zero-value bugs | Go, Java, C# | No null; `Option[T]`; no implicit zero values; definite initialization enforced |
| P10 | Heisenbugs unreproducible in prod | All | Deterministic replay: record effect log, replay bit-exact locally; time-travel debugger |
| P11 | Two codebases for CPU + GPU | C++/CUDA | Single-source `kernel` functions retargeted via MLIR |
| P12 | Language evolution breaks code or fossilizes | Python 2→3, C++ | Editions + `grasm migrate` (AST-level auto-migration shipped with every breaking change); N and N-1 editions co-link |
| P13 | AI-generated code is unverifiable | All | Contracts + effects + refinement types = machine-checkable guardrails; compiler query API lets agents verify before proposing |
| P14 | FFI is unsafe and manual | Most | `import c "header.h"` — compiler-integrated (libclang) zero-cost binding; sandboxed-by-capability foreign calls; Fearless-FFI-style isolation of unsafe memory |

---

# 7. Core Design Principles

1. **Progressive disclosure of power.** Day-1 Grasm reads like Go/Python. Effects,
   regions, comptime, contracts reveal themselves only when needed. Every advanced
   feature has a sensible inferred default.
2. **The compiler does the reasoning, the human states intent.** Inference over
   annotation, always — annotations are for *interfaces* (public API boundaries),
   never for convincing the compiler of local facts.
3. **Safe is the default; unsafe is a visible, auditable, capability-gated privilege.**
4. **Zero-cost or opt-in cost.** No pervasive runtime tax. Checks that can't be
   proven away are cheap, local, and elidable (generation checks ~1 predictable
   branch).
5. **One way to do it, and it's the fast way.** Values by default; the performant
   pattern and the idiomatic pattern must be the same pattern.
6. **Determinism is a feature.** Same inputs → same outputs, same iteration order,
   same float behavior per target class; all nondeterminism flows through effects.
7. **Signatures are honest.** A function's type tells you everything it can do:
   inputs, outputs, errors, effects. No hidden I/O, no hidden throws, no hidden
   global state.
8. **Toolchain is the product.** One binary: compiler, LSP, formatter, linter,
   tests, docs, package manager, profiler, replay debugger. Install-to-first-program
   in under 2 minutes.
9. **Errors are UX.** Every diagnostic names the fix or teaches the concept
   (Elm-standard), and every diagnostic has a stable ID + machine-readable form.
10. **Design for the reader and the machine.** Code is read 10× more than written —
    by humans and now by AI. Canonical formatting, semantic diff, stable public
    semantics APIs.
11. **Editions over eternity.** The language may change; user code migrates
    mechanically; nobody gets left behind and the language never fossilizes.
12. **Interop is oxygen.** C ABI in/out with zero friction; WASM Component export;
    Python extension generation. A language nobody can call is a language nobody
    adopts.

---

# 8. Language Specification (Master Overview)

> Full normative spec is a Phase-0/1 deliverable (`spec/` in-repo, executable test
> suite per clause). This section fixes the load-bearing design decisions.

## 8.1 Syntax philosophy

Brace-delimited, expression-oriented, newline-terminated (no semicolons), canonical
formatter with **no configuration**. Type inference for locals; explicit types at
public API boundaries (enforced by lint tier). Sample:

```grasm
// hello.gr
pub fn main() {
    print("hello, world")          // effect: io — inferred, handled by runtime
}

// Values, immutability, inference
fn stats(xs: [f64]) -> (mean: f64, sd: f64) {
    let n = xs.len.as[f64]()
    let mean = xs.sum() / n
    var acc = 0.0
    for x in xs { acc += (x - mean)^2 }
    (mean, sqrt(acc / n))
}

// Sum types + exhaustive matching
type Shape =
    | Circle(r: f64)
    | Rect(w: f64, h: f64)

fn area(s: Shape) -> f64 {
    match s {
        Circle(r)  -> PI * r^2
        Rect(w, h) -> w * h
    }
}
```

## 8.2 Type system

- **Foundation:** Hindley–Milner-style local inference + bidirectional checking;
  no global inference (public signatures explicit → stable incremental builds).
- **Algebraic data types**: sum types, product types (structs), tuples, exhaustive
  `match` with guards, or-patterns, destructuring.
- **No null.** `Option[T]` with `?.`, `??`, and pattern sugar.
- **Generics:** parametric polymorphism with **definition-checked interfaces**
  (`trait` equivalent, called `interface`): generic bodies type-check against
  declared bounds at *definition* (Carbon/Rust model, not C++/Zig instantiation
  model) → errors surface once, at the right place. Monomorphized by default,
  `dyn`-style existentials opt-in.
- **Const generics** (array sizes, units) via comptime parameters.
- **Refinement layer (opt-in):** `where` clauses on types —
  `type Port = i32 where 0 < self <= 65535` — checked by SMT when decidable,
  by runtime assert otherwise (§9.5).
- **Linear resources:** types can be declared `linear` (file handles, sockets,
  locks, capabilities, one-shot tokens). The compiler enforces exactly-once
  consumption — "Higher RAII": forgetting to close/commit/rollback is a
  *compile-time* error, not a leak.

## 8.3 Memory model — Tiered Ownership (the crown jewel)

**Design goal:** 100% static memory safety, zero GC, zero lifetime annotations in
safe code.

**Tier 0 — Mutable value semantics (the default, ~85% of code).**
Everything is a value. Assignment moves or copies (compiler picks; `Copy` for small
PODs). Functions take parameters by convention:

```grasm
fn read(cfg: Config)          // `let` param: caller keeps it; callee sees immutable
                              //   (compiled as pass-by-reference — zero copy)
fn update(inout cfg: Config)  // exclusive in-place mutation, caller sees result
fn consume(own cfg: Config)   // ownership transfer (move)
```

References are **second-class**: you can pass by reference (`inout`), but you
cannot *store* a reference in a struct, return it, or capture it beyond the callee's
frame in Tier 0. This one restriction is what deletes the entire lifetime-annotation
apparatus. Exclusivity is enforced (no two `inout` aliases to overlapping places) —
which also delivers data-race freedom and optimizer-grade noalias facts.

**Tier 1 — Regions (inferred arenas).**
Escape analysis groups allocations into compiler-inferred regions with stack-like
lifetimes; bulk-freed, cache-friendly. User can name regions for control
(`region temp { ... }` — everything allocated inside dies at the brace) — the
Zig-arena pattern with static safety.

**Tier 2 — Generational references (`ref T`), the safe graph escape hatch.**
For genuinely shared/cyclic/long-lived object graphs (GUI trees, ECS, caches):

```grasm
let node: ref Node = graph.insert(Node{...})   // 128-bit fat ref: ptr + generation
node.value += 1                                 // checked deref: 1 predictable branch
```

Deref checks the allocation's generation; stale access → deterministic panic
(never UB). Static analysis + region facts elide the overwhelming majority of
checks (Vale's own-lending result: elision can reach 100% in linear style).

**Tier 3 — `unsafe`.** Raw pointers, FFI, intrinsics. Only inside
`unsafe` blocks, only in packages that declare the `unsafe` capability (visible in
the manifest, auditable transitively, deniable by consumers: your JSON parser
cannot quietly contain `unsafe`).

**Guarantees across tiers 0–2:** no use-after-free, no double-free, no OOB
(bounds-checked, elided by refinement/iterator proofs), no uninitialized reads
(definite initialization), no data races, no UB. Deterministic destruction order.

## 8.4 Errors

```grasm
fn parse(s: str) -> Config ! ParseError          // T ! E — error union
fn load(path: str) -> Config ! (IoError | ParseError) {
    let text = try fs.read(path)                 // propagate, 1 keyword
    let cfg  = try parse(text)
    cfg
}
// caller side
match load("app.toml") {
    Ok(cfg)              -> run(cfg)
    Err(IoError.NotFound)-> run(Config.default())
    Err(e)               -> fatal("bad config: {e}")
}
```

- Errors are values; unions auto-widen; exhaustive matching enforced.
- `panic` = program bug (assert/contract failures), not control flow; unwinds to
  task boundary; converted to error at supervision points.
- Zero-cost: error unions compile to tagged returns (no unwinding machinery on the
  happy path).

## 8.5 Effects — the unifying system

Every function has an inferred **effect row**; the safe language cannot perform an
effect that isn't in the row. Core built-in effects: `io.fs`, `io.net`, `io.time`,
`io.env`, `io.rand`, `alloc`, `async`, `panic`, plus user-defined effects.

```grasm
// Effects are INFERRED — beginners never write them:
fn fetch_price(sym: str) -> f64 ! HttpError {    // inferred: {io.net, async}
    let resp = try http.get("https://api.x.com/{sym}")
    try resp.json[f64]()
}

// Public API boundaries state them (lint-enforced honesty):
pub fn fetch_price(sym: str) -> f64 ! HttpError / {io.net}

// User-defined effect + handler = DI, mocking, interpreters:
effect Clock { fn now() -> Instant }

fn is_expired(t: Token) -> bool / {Clock} { Clock.now() > t.exp }

test "expiry" {
    with Clock handled_by FakeClock(at: EPOCH + 3600) {
        assert !is_expired(token_valid_for(7200))
    }
}
```

**What effects buy, concretely:**
1. **No colored functions.** `async` is an effect; the scheduler is its handler.
   The same `fetch_price` runs on a blocking thread, an io_uring event loop, or a
   deterministic test scheduler — caller's choice, zero code change.
2. **Capability security.** A dependency's manifest declares its maximum effect
   set; the compiler rejects any package whose code exceeds it, transitively.
   `left-pad`-class utility packages are *provably* pure.
3. **Testing without mock frameworks.** Handlers replace any effect in scope.
4. **Deterministic replay** (§9.6): the effect boundary is the record/replay cut.
5. **Compiles to zero-cost:** tail-resumptive handlers (the overwhelming majority)
   compile to direct calls/closures (Koka's evidence-passing result); no stack
   switching unless a handler actually captures a continuation.

**Ergonomics guardrails (the make-or-break):** full inference in private code;
effect *aliases* (`/ {web}` = `{io.net, async, alloc}`); coarse default profiles
(`app`, `pure`, `kernel`); diagnostics phrased as capabilities ("this function
touches the network via http.get → add io.net or handle it"), never as type algebra.

## 8.6 Concurrency

- **Structured concurrency only.** Tasks spawn inside scopes; a scope cannot exit
  until its children finish or cancel — no orphan goroutines, no detached futures.

```grasm
fn crawl(urls: [Url]) -> [Page] / {io.net, async} {
    task.scope(max_parallel: 64) |s| {
        for u in urls { s.spawn { fetch(u) } }
    }   // joins all, propagates first error (cancelling siblings), returns results
}
```

- **Data-race freedom statically:** spawned closures may capture `own` values,
  `Share`d immutables (frozen deeply-immutable graphs, zero-copy), or actors.
  `inout` captures are rejected at compile time.
- **Actors** for mutable shared state: `actor Counter { var n = 0; fn inc(inout self) }`
  — exclusive by construction, messages are effects.
- **Channels** in stdlib, built on effects (selectable, cancellable).
- **Deterministic scheduler mode** for tests: seeded interleavings, reproducible
  concurrency bugs, built-in race-exploration (`grasm test --schedules 10000`).
- 1M+ lightweight tasks/node; work-stealing runtime ~50 KB, linked only if `async`
  effect reaches `main`.

## 8.7 Compile-time programming

- **`comptime`:** the full language executes at compile time (Zig-proven), in the
  same interpreter that powers the REPL and replay. Table generation, config
  parsing, embedding, specialization — no separate macro language.
- **Typed, hygienic macros** (`macro fn`) operate on typed AST fragments with
  quasi-quotation; can *add* code, never mutate distant code; expansions are
  previewable (`grasm expand`) and debuggable.
- **Derive system:** `#[derive(Eq, Hash, Json, Diff)]` implemented as library
  macros over reflection facts exposed by the compiler query engine.

## 8.8 Interop

- **C:** `import c "sqlite3.h"` — libclang-integrated; zero-cost calls; foreign
  memory quarantined from safe heap (Fearless-FFI); requires `ffi` capability.
- **Export:** any `pub extern` fn emits a C-ABI symbol + generated header. Grasm
  libraries embeddable in C/C++/Rust/Python/Node from day one.
- **WASM Component Model:** `grasm build --target wasm-component` emits a component
  with WIT interfaces derived from Grasm signatures; effects map to WASI
  capability imports 1:1.
- **Python:** `grasm build --pymodule` emits a native extension module (the Mojo
  adoption lesson: meet giant ecosystems where they are).

## 8.9 Modules, packages, visibility

- File = module; directory = package; explicit `pub` exports; no cyclic package
  deps (enforced DAG → parallel compilation for free).
- Manifest (`grasm.toml`): semver, **capability ceiling** (`caps = ["io.net"]`),
  edition, reproducibility lockfile with content hashes + signatures.

---

# 9. Novel Features Not Found in Any Mainstream Language

> Individually, several exist in research languages. **As a productized, integrated
> set, none exists anywhere.** Each is listed with its precedent and Grasm's delta.

### 9.1 Capability-secured dependency graphs (type-enforced supply chain)
*Precedent:* Austral capabilities, WASI imports, Deno permissions (runtime-level).
*Delta:* enforcement in the **static type system across the whole dependency
tree**, surfaced in the package manager UX:

```
$ grasm add left-pad
  left-pad 2.1.0 — capabilities: {} (pure)  ✓ auto-approved
$ grasm add totally-not-malware
  ⚠ requests io.net + io.env (reads env vars, opens sockets)
    Your app policy allows: {io.fs}. Add exception? [y/N]
```

A compromised patch release that adds an effect **fails to compile** in every
consumer. This kills the dominant open-source attack class at the language layer.
No mainstream language can express this today.

### 9.2 Memory safety with **zero annotations and zero GC** (Tiered Ownership)
*Precedent:* Hylo (MVS), Vale (gen-refs), Swift (value semantics + ARC).
*Delta:* the integrated tier stack (§8.3) — value semantics default, inferred
regions, generational refs for graphs, capability-gated unsafe — as a *shipping,
tooled* language rather than a research artifact. Rust-class guarantees; Go-class
annotation burden (i.e., none).

### 9.3 Uncolored concurrency via effect handlers, in a no-GC systems language
*Precedent:* OCaml 5 (GC language), Koka (research), Zig's I/O-as-interface
(dynamic, not typed).
*Delta:* statically typed effect rows + evidence-passing compilation **without a
GC** — the same function is sync, async, parallel, or deterministic-replayed by
handler choice. Ends the tokio-vs-std, sync-vs-async ecosystem split for good.

### 9.4 Linear resources with Higher RAII as default culture
*Precedent:* Austral, Vale linear style.
*Delta:* mainstream ergonomics — stdlib files/sockets/locks/transactions are
linear; "you forgot to commit or roll back this transaction on the error path" is
a **compile error** with a fix-it, not a production incident.

### 9.5 Gradual verification: one contract language, three enforcement levels
*Precedent:* Dafny/Verus (prove-everything), Eiffel (runtime-only contracts).
*Delta:* the **same** `requires`/`ensures`/refinement syntax is (a) SMT-proved
when decidable → checks erased; (b) runtime-checked otherwise; (c) fuzzed by
`grasm test --contracts` using contracts as oracles. A dial, not a cliff:

```grasm
fn binary_search(xs: [i32], k: i32) -> Option[usize]
    requires xs.is_sorted()
    ensures  |r| match r { Some(i) -> xs[i] == k, None -> !xs.contains(k) }
```

AI-generated code with contracts becomes *reviewable by the compiler* — the
guardrail the AI era is missing.

### 9.6 Deterministic replay & time-travel debugging as a language guarantee
*Precedent:* rr (best-effort, x86-Linux-only), FoundationDB sim testing (bespoke),
Antithesis (external platform).
*Delta:* because *all* nondeterminism is an effect, the runtime records the effect
log (cheap: syscall-boundary events, not memory traces) in a ring buffer.
Any crash ships a `.replay` capsule; `grasm replay crash.replay` re-executes
bit-for-bit locally with **reverse-step** debugging. Concurrency schedules are
part of the log → heisenbugs die. No mainstream language guarantees this.

### 9.7 The compiler as a public, stable, queryable database — AI-native by design
*Precedent:* Salsa/rust-analyzer (internal architecture only).
*Delta:* the query engine is **the product's API**: `grasm query` (and a gRPC/JSON
server) exposes types, effects, ownership facts, call graphs, "what breaks if I
change this signature," semantic (AST-aware) diff/merge for CI and review — the
same substrate powering the LSP. Agents verify claims against ground truth instead
of hallucinating; refactors become transactional semantic operations.

### 9.8 Single-source heterogeneous compute in a general-purpose language
*Precedent:* Mojo (AI-vertical), CUDA/SYCL (C++ dialect bolt-ons).
*Delta:* `kernel fn` — a restricted, effect-`pure`, verified-total subset —
compiles via the MLIR mid-end to CPU/SIMD, NVIDIA, AMD, and WASM-SIMD from one
definition, with cost-model diagnostics ("this branch prevents vectorization").
General-purpose language, not an AI-serving stack.

### 9.9 Editions with shipped auto-migration
*Precedent:* Rust editions (syntax-level, conservative).
*Delta:* every breaking change must land **with** its `grasm migrate` transform
(typed-AST rewrite, semantic-diff-verified). The language can keep evolving for
30 years without a Python-3 event; N and N−1 editions co-link in one binary.

### 9.10 Structured observability as effects
*Precedent:* OpenTelemetry libraries (opt-in, inconsistent).
*Delta:* `trace`/`metric`/`log` are effects: zero-cost when unhandled (compiled
out), uniformly handleable (sampling, redaction, export) at `main` — and the
replay capsule (§9.6) doubles as a perfect-fidelity trace.

### 9.11 Hot state-preserving reload with typed migration
*Precedent:* Erlang hot code loading (dynamic, unchecked).
*Delta:* dev-server mode diffs old/new types; where a total migration function is
derivable (or user-provided), live state migrates under the type checker's
guarantee — Erlang's superpower, statically safe.

---

# 10. Compiler, Runtime & Toolchain Architecture

## 10.1 Architecture principles

1. **Query-based core, batch mode as a special case** (Salsa-style memoized
   demand-driven queries; compile = "evaluate `artifact(main)` query").
2. **Everything incremental, everything parallel** (per-file parse; per-item
   typecheck; per-function codegen; no global phases except linking).
3. **Resilient by construction:** error-tolerant parser (CST with error nodes),
   typechecker produces facts for broken code → IDE quality on incomplete programs.
4. **Two backends, one truth:** Cranelift-class fast backend for dev (compile
   speed), LLVM for release (peak perf), sharing one mid-level IR; differential
   testing between them is a correctness weapon.
5. **The interpreter is load-bearing:** one definitional interpreter executes
   comptime, the REPL, contracts-under-test, and replay — and serves as the
   executable semantics for differential testing against both backends.

## 10.2 Pipeline

```
 source ──► Lexer ──► CST (lossless, error-tolerant)
                       │
                       ▼
                      AST ──► Name resolution ─┐
                       │                        │   ┌────────────────────────┐
                       ▼                        ├──►│  QUERY ENGINE (salsa-  │
                      HIR  ◄── macro/comptime ──┘   │  style, memoized,      │
                       │        expansion           │  parallel, persistent  │
                       ▼                            │  on-disk cache)        │
              Typed HIR (types + effect rows        └──────────┬─────────────┘
                       │   + definite-init)                    │
                       ▼                                       ▼
                      MIR ── ownership/exclusivity ──   LSP · queries · lints
                       │     region inference           semantic diff · docs
                       │     gen-check elision          refactoring · agents
                       │     contract lowering
                       ▼
              ┌── MLIR mid-end (opt, vectorize, kernel retarget) ──┐
              ▼                    ▼                               ▼
        FastCG (dev)          LLVM (release)                 WASM Component
              └──────────────┬────┘
                             ▼
              built-in linker (mold-class, incremental)
```

## 10.3 Frontend details

- Hand-written recursive-descent parser (error recovery, resilience, speed);
  grammar fuzzed from day one; CST preserves trivia → formatter/refactors lossless.
- Name resolution and type/effect inference as fine-grained queries keyed on item
  signatures → editing a function body invalidates almost nothing.
- **Binary module interfaces (BMI):** packages export a compact serialized
  signature file; downstream compilation never re-parses upstream source
  (C++20-modules lesson, done properly).

## 10.4 MIR: where the guarantees live

SSA-based, ownership-aware mid-level IR. Passes: definite-initialization;
exclusivity check (the "one `inout` at a time" rule — a *local, function-at-a-time*
analysis, unlike Rust's whole-lifetime inference: this is why compile stays fast
and errors stay local); escape analysis → region assignment; generation-check
elision (dominator + region facts); linear-consumption check; contract lowering
(prove → erase / else runtime check); panic-path outlining.

## 10.5 Runtime (deliberately boring)

- No GC, no green-thread machinery unless `async` reaches `main`.
- Allocator: size-classed, region-aware (bump allocation inside regions), with
  generation slots for Tier-2 allocations.
- Scheduler (linked on demand): work-stealing tasks; io_uring (Linux), kqueue
  (macOS), IOCP (Windows) event loops as `async` handlers; deterministic seeded
  scheduler for tests/replay.
- Panic = unwind-to-task-boundary + capsule capture; abort mode for embedded.
- Replay recorder: lock-free ring buffer of effect events; ~1–3% overhead target,
  on by default in dev/test, sampling + opt-in in release.

## 10.6 Toolchain products (one binary: `grasm`)

| Command | Function |
|---|---|
| `build` / `run` / `check` | AOT build (dev/release), execute, typecheck-only |
| `test` | unit + property + contract-fuzz + schedule-exploration |
| `fmt` / `lint` | canonical format (zero config); staged lint tiers |
| `doc` | docs from source + examples-as-tests |
| `add` / `vendor` / `publish` | package ops with capability review UX |
| `lsp` | language server (same query engine) |
| `repl` | interpreter REPL with effect sandbox |
| `expand` | show macro/comptime expansions |
| `query` | semantic DB access (human + JSON) |
| `migrate` | edition/API auto-migration |
| `replay` | time-travel debug a capsule |
| `prove` | run SMT obligations, show proved/runtime-checked map |
| `bench` / `profile` | criterion-class benchmarking; perf integration |
| `cc` | bundled C cross-compiler (Zig-lesson: instant credibility utility) |

---

# 11. Developer Experience, Tooling & Ecosystem

## 11.1 The two-minute rule
`curl grasm.dev/install | sh` → `grasm new api && cd api && grasm run` in under
2 minutes on macOS/Linux/Windows. Toolchain is a single static binary + stdlib.
Cross-compilation to every tier-1 target out of the box (`--target` just works —
the Zig standard).

## 11.2 Error messages as curriculum
Elm-quality prose, Rust-quality spans, plus: stable error IDs (`E0423`), one-command
explanations (`grasm explain E0423`), machine-readable JSON for agents/IDEs, and
**auto-fix coverage tracked as a KPI** (target: ≥60% of diagnostics carry a
verified fix-it).

## 11.3 Learning stack
Interactive in-browser tour (compiler-as-WASM); *The Grasm Book*; "Grasm for
Rustaceans / Gophers / Pythonistas" migration guides; examples-are-tests doctrine
(every doc snippet compiles in CI); official LLM context packs (curated,
versioned language descriptions for AI assistants — cheap, high-leverage,
nobody does it well).

## 11.4 Package ecosystem
- Registry with **namespaced packages** (`@org/name` — no name-squatting),
  signed + reproducible artifacts, capability ceilings surfaced in UI and CLI.
- Build scripts run in a WASM sandbox with declared capabilities (no more
  `build.rs` exfiltration class).
- Stdlib: batteries-included Go-style (http, json, crypto-audited, sql, cli,
  test, time, os) — the stdlib is a curriculum and a quality floor.
- `grasm vendor` for hermetic/air-gapped builds; crater-style whole-registry
  regression runs before every release.

## 11.5 IDE & AI surfaces
LSP feature-complete at *beta* (not after 1.0 — rust-analyzer's late arrival cost
Rust years); DAP debugger with replay integration; semantic diff/merge drivers for
git; `grasm query --serve` for agents; VS Code + JetBrains + Zed plugins owned by
the core team.

---

# 12. Performance, Scalability, Security, Reliability

## 12.1 Performance engineering
- **Compile:** ≥500k LOC/s frontend per core (parallel scaling); ≤50 ms p95
  incremental edit→diagnostic on a 1M-LOC monorepo; dev-backend codegen within 3×
  of interpreter startup for small programs.
- **Runtime:** ≥0.95× Rust geomean on the benchmark suite (bounds/gen checks are
  the budget line — elision rates published per release); zero-allocation
  fast paths in stdlib; PGO/LTO one flag each.
- **Method:** perf lab with pinned hardware (x86 + ARM + GPU), nightly dashboards,
  regression gates on every merge (±1% budget), public results (the Zig-devlog
  transparency lesson).

## 12.2 Scalability
Monorepo-scale builds (distributed query cache — content-addressed, shareable
CI↔dev); 1M+ tasks per node; NUMA-aware scheduler; incremental link.

## 12.3 Security model (layered)
1. Language: no UB in safe code; effects = least privilege; linear resources.
2. Package: capability ceilings, sandboxed builds, signed reproducible artifacts,
   namespace ownership.
3. Toolchain: reproducible bootstrap (see 14.4), SLSA-class provenance, fuzzed
   parser/IR/backends continuously.
4. Deployment: WASM component export inherits platform sandboxing; `--hardened`
   profile (CFI, shadow stacks, always-on checks) for regulated users.
5. Disclosure: security.txt, CVE program, embargo process from beta onward.

## 12.4 Reliability of the toolchain itself
Definitional-interpreter differential testing (interpreter vs FastCG vs LLVM on
generated programs — Csmith-style); spec conformance suite (every normative clause
→ executable tests); miscompilation = P0 with public postmortem; compiler crash
always yields a minimized repro via built-in reducer.

---

# 13. Implementation Roadmap: Phases & Milestones

> Timeline assumes the staffing curve of §14.1. Dates are elapsed-from-start.
> Every phase has **exit criteria** — no phase ends on the calendar.

## Phase 0 — Foundations (months 0–6) · 3–5 people
**Goal: de-risk the two novel bets on paper and in prototypes.**
- Write spec drafts for: tiered ownership; effect inference + evidence-passing
  compilation without GC; concrete syntax.
- Two throwaway prototypes: (a) MVS+regions+gen-refs checker over a toy IR;
  (b) effect-handler compilation benchmark (prove tail-resumptive ≈ direct call).
- Decide implementation host language: **Rust** (ecosystem: logos/salsa/cranelift/
  inkwell; team hireability) — revisit self-hosting at Phase 4.
- Governance charter, license (Apache-2.0 + LLVM exception), trademark, RFC process.
- **Exit criteria:** both prototypes hit perf targets on micro-suites; spec drafts
  survive external review by 3+ PL researchers; syntax user-tested on 20 developers.

## Phase 1 — Vertical slice (months 6–18) · 6–10 people
**Goal: one thin path through the whole system, real programs run.**
- Query-engine skeleton; lexer/parser/CST/AST; name resolution; core typechecker
  (no generics yet → then generics); definitional interpreter.
- Tier-0 ownership (values, `inout`, exclusivity) + definite init.
- Error unions + `try`; basic effects (inferred rows, `io.*`, handlers, no
  continuation capture yet).
- FastCG native backend for x86-64/aarch64; hello-world → real CLI programs.
- `grasm` CLI: build/run/check/fmt/test; CI, fuzzing on parser from month 7.
- **Exit criteria:** self-hosted *formatter* written in Grasm; 10k-LOC internal
  test app; incremental check <100 ms on it; zero known soundness holes in Tier-0.

## Phase 2 — The language exists (months 18–30) · 10–15 people
**Goal: public alpha (v0.1–v0.3), brave-early-adopter grade.**
- Generics + interfaces (definition-checked); comptime; derive macros.
- Tier-1 regions + Tier-2 generational refs + elision pass; linear types.
- Structured concurrency + scheduler + uncolored async over io_uring/kqueue.
- LLVM release backend; WASM (core module) target; C import (libclang).
- LSP v1 (diagnostics, completion, goto, rename); package manager + registry MVP
  with capability ceilings; stdlib v0 (io, net, json, test, time).
- **Public launch of repo + playground.** Devlog cadence (Zig lesson: build trust
  through public behavior).
- **Exit criteria:** 3 external teams build real tools on it; compile-speed targets
  hit on 100k LOC; memory-safety red-team (hired) finds no safe-code UB; 1k GitHub
  contributors' issues triaged under 14-day median.

## Phase 3 — Beta & beachhead (months 30–48) · 15–25 people
**Goal: v0.5–v0.9 — production-adjacent, one domain won.**
- Effects complete (user effects, continuation capture, deterministic scheduler);
  replay capsules + time-travel debugger v1; contracts + SMT opportunistic proving.
- WASM Component Model + WIT export as tier-1 target; Python extension export.
- MLIR mid-end online; first `kernel` CPU/SIMD retargeting (GPU experimental).
- Editions machinery + `grasm migrate` proven on a real breaking change.
- **Beachhead: cloud/WASM infrastructure tooling** — CLIs, proxies, plugins,
  serverless components (where Go lives and Rust is over-priced in effort).
  Fund 2–3 flagship OSS apps (e.g., an envoy-class plugin runtime, a wasmCloud/Spin
  competitor, a fast data CLI) built by the team + partners.
- Hire DevRel; conference talks; university course partnerships.
- **Exit criteria:** 25+ companies in production on the beachhead; p99 incremental
  <50 ms at 1M LOC; replay works for ≥99% of crashes in flagship apps; benchmark
  geomean ≥0.9× Rust (0.95× by 1.0).

## Phase 4 — 1.0 (months 48–66) · 25–40 people
**Goal: the stability event.**
- Spec frozen for edition-2030; soundness review (external formal-methods audit of
  ownership + effects core, ideally a mechanized core-calculus proof — see §17).
- Stability guarantee: code compiling on 1.0 compiles on every 1.x; N/N−1 edition
  co-linking; LTS releases (18-month) for regulated industries.
- Self-host the frontend (compiler-in-Grasm) as the ultimate dogfood; keep LLVM/
  MLIR in C++ via FFI (pragmatism over purity).
- Foundation launch: multi-vendor governance, trademark transfer, funded core team
  (the "no single-company capture" differentiator vs Mojo/Carbon/Swift).
- **Exit criteria:** 1.0 ships when — and only when — 3 independent auditors sign
  off on soundness; 100+ production companies; 5k registry packages.

## Phase 5 — Expansion (months 66+)
GPU kernels GA; embedded/`no-runtime` profile (deterministic panics→reset,
regions-only mode) for the CRA-regulated embedded market; distributed-systems
toolkit (deterministic simulation testing as a product — FoundationDB methodology
for everyone); OS-development story; verification deepening (§17).

## 13.1 Milestone summary table

| Milestone | Elapsed | Proof point |
|---|---|---|
| M0 prototypes validate memory+effect bets | 6 mo | micro-benchmarks + external review |
| M1 first real program; self-hosted formatter | 18 mo | vertical slice alive |
| M2 public alpha, playground, registry | 30 mo | outsiders build things |
| M3 beta; beachhead production users; replay v1 | 48 mo | 25+ prod companies |
| M4 **1.0** + foundation + soundness audit | 60–66 mo | stability promise |
| M5 embedded/GPU/distributed expansion | 66+ mo | second + third domains |

---

# 14. Engineering Requirements & Infrastructure

## 14.1 Team composition & growth

| Phase | Headcount | Key roles added |
|---|---|---|
| 0 | 3–5 | Language lead/BDFL, type-system researcher, compiler eng ×2 |
| 1 | 6–10 | + backend eng, runtime eng, tooling eng, technical writer |
| 2 | 10–15 | + LSP/IDE eng ×2, stdlib eng ×2, registry/infra eng, security eng |
| 3 | 15–25 | + DevRel ×2, GPU/MLIR eng, verification eng, release eng, community mgr |
| 4 | 25–40 | + LTS/support, foundation ops, docs team, ecosystem grants program |

Non-negotiable hires: at least one person who has shipped a production compiler;
one PL-theory PhD (soundness); one security engineer from Phase 2.

## 14.2 Funding reality
Comparable efforts burn $3–6M/yr mid-phase. Viable structures: (a) foundation +
corporate sponsors (Rust/Zig hybrid model); (b) venture-backed company with an
open-core *services* plan — hosted registry, replay cloud, LTS support — **never**
closed language features (Mojo cautionary tale); (c) patron/major-sponsor model.
Decision point end of Phase 1. Total to 1.0: roughly $25–45M — plan for it
explicitly rather than discovering it.

## 14.3 Development infrastructure
- Monorepo (compiler, stdlib, tools, spec, website); trunk-based; merge queue with
  full test + perf gates.
- CI matrix: {Linux, macOS, Windows} × {x86-64, aarch64} + cross to RISC-V/WASM;
  sanitizer builds of the compiler itself.
- Continuous fuzzing (parser, macro expander, MIR passes, backends via
  differential execution); OSS-Fuzz enrollment at public alpha.
- Perf lab: dedicated bare-metal fleet, nightly benchmark dashboards (public).
- Crater-class ecosystem runs: compile the entire registry against every release
  candidate.
- Spec-as-tests: normative clause ↔ conformance test, tracked coverage.

## 14.4 Bootstrap & supply-chain integrity of the toolchain
Stage-0 built from pinned Rust toolchain; reproducible builds from day one
(bit-identical across builders); signed releases with public transparency log;
post-self-hosting, maintain a minimal seed interpreter path to preserve
diverse-double-compilation (trusting-trust defense).

## 14.5 Governance
Phase 0–2: BDFL + small core team, public RFC process (all design in the open —
the Zig/Rust trust pattern). Phase 3: elected domain teams (lang, compiler,
stdlib, tooling, community). Phase 4: independent foundation owns trademark +
spec; corporate seats capped; language decisions require core-team consensus, not
sponsor votes.

---

# 15. Adoption Strategy

## 15.1 Beachhead doctrine
General-purpose ambition, **niche-first execution** (Go had Google-scale cloud;
Rust had Firefox/Servo). Grasm's beachhead: **cloud/WASM infrastructure tooling**
— CLIs, proxies, plugins, edge functions, serverless components. Why: Go's home
turf (its users feel GC/typing pain and are pragmatists), WASM Component tailwind,
small-binary + fast-start superpowers land immediately, and effect-capabilities
are a *visible* security feature in that market. Secondary wedge (Phase 5):
CRA-regulated embedded (safety story sells itself to compliance).

## 15.2 The migration funnel
- **From Go:** "keep your productivity, drop the GC, gain real types" — migration
  guide, `net/http`-shaped stdlib APIs, goroutine→task.scope mental map.
- **From Rust:** "your safety, a tenth of the friction" — zero-annotation examples,
  crate-bridge FFI so Rust libraries are callable early.
- **From Python/TS:** WASM + native extension exports let them adopt Grasm for the
  hot 5% without leaving home.
- **Killer-app funding:** grants program pays maintainers to build the missing
  flagship libraries (the "SQLAlchemy/Flask moment" cannot be left to chance).

## 15.3 Trust-building mechanics
Public devlog + roadmap; every design decision as a written RFC with rationale;
benchmarks published even when losing; soundness bugs disclosed with postmortems;
no vaporware claims — the credibility asset compounding over 5 years is the moat
no incumbent can copy quickly.

## 15.4 KPIs
Time-to-first-program; tutorial completion; weekly active toolchains; registry
growth; retention (repeat monthly users); production-company count; diagnostic
auto-fix rate; median issue triage latency; contributor diversity (bus factor).

---

# 16. Risks, Challenges, Limitations & Mitigations

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R1 | **Effect-system ergonomics fail for average devs** (the #1 design risk — effects are unproven at mainstream scale) | Med-High | Fatal | Full inference; effect aliases/profiles; usability testing every phase; ship coarse `app` profile that hides rows entirely; be willing to *cut generality* (single-shot handlers only) for UX |
| R2 | **Tiered memory model has a soundness hole** at a tier boundary | Med | Fatal | Phase-0 prototype + external PL review; mechanized core-calculus proof (Phase 3–4); red-team audits; interpreter-vs-backend differential testing |
| R3 | Second-class references too restrictive → devs hit walls | Med | High | Tier-2 gen-refs escape hatch is *ergonomic, not punitive*; collect telemetry on friction; subscript/projection sugar (Hylo pattern) for the common "return a view" cases |
| R4 | Compile-speed targets vs feature richness | Med | High | Perf gates in CI from Phase 1; features that break incrementality get redesigned or cut; BMI + local-only analyses (exclusivity is function-local by design) |
| R5 | **Adoption cold-start** — world doesn't need another language | High | Fatal | Beachhead discipline; killer-app grants; interop-first (call C/Rust, export to C/WASM/Python); regulatory tailwind marketing to compliance buyers |
| R6 | Funding gap in years 2–4 | Med-High | Fatal | Structure decision end of Phase 1; services open-core plan; sponsor diversification rule (no sponsor >40%) |
| R7 | Key-person risk (BDFL) | Med | High | Written rationale culture; core team of 3+ owning spec by Phase 2 |
| R8 | Rust/Go close the gap (Rust ergonomics initiative; Go arena/GC advances) | Med | Med | They're constrained by compatibility (Rust can't delete lifetimes; Go can't delete GC); track releases; speed is the startup's advantage |
| R9 | Zig/Mojo occupy mindshare first | Med | Med | Differentiate on safety (vs Zig) and openness/generality (vs Mojo); don't race their launches — race their gaps |
| R10 | SMT prover flakiness leaks into UX | Med | Med | Proving is *never* required for a build; deterministic fallback to runtime checks; prover version pinned per edition |
| R11 | GPU/heterogeneous scope creep destabilizes core | Med | Med | MLIR path is additive; `kernel` subset frozen small; GPU GA explicitly post-1.0 |
| R12 | AI code-gen commoditizes new-language advantage ("LLMs know Rust, not Grasm") | Med-High | Med | LLM context packs from alpha; partner for fine-tuned model support; machine-readable everything makes Grasm the *easiest* language for agents to be correct in — turn the threat into the moat |
| R13 | Replay overhead unacceptable in hot paths | Low-Med | Med | Sampling modes; per-effect opt-out; capsule = effects only (not memory) keeps cost ~1–3% |
| R14 | Trademark/legal/patent ambush | Low | Med | Foundation ownership; defensive publication of novel mechanisms |

**Known limitations (accepted, documented):** no shared-memory mutable aliasing
without actors/gen-refs (by design); Tier-2 deref costs a branch when not elided;
effects add one concept Go never asks you to learn (bet: it replaces five you
learn anyway — async, exceptions, DI, mocks, permissions); pre-1.0 churn will
break early adopters (editions machinery arrives Phase 3 to soften it).

---

# 17. Long-Term Evolution & Research Directions

1. **Mechanized soundness.** Coq/Lean formalization of the core calculus (tiered
   ownership + effect rows); long-term: verified MIR→machine-code path for a
   certified subset (CompCert-class story for regulated industries).
2. **Verification deepening.** From opportunistic SMT to modular refinement
   inference; proof-carrying registry packages ("this crypto lib's constant-time
   claim is machine-checked").
3. **Distributed effects.** Effects as the RPC boundary: location-transparent
   handlers, deterministic simulation of whole clusters (`grasm sim`), typed
   sagas/compensation — FoundationDB-style rigor as a language feature.
4. **Persistent-memory & CXL-era memory tiers** as region kinds.
5. **Structured editing & projectional surfaces.** CST-first tooling opens
   optional projections (notebooks, visual dataflow) over the same semantics.
6. **AI-native evolution.** Compiler-in-the-loop code synthesis (agent proposes,
   query engine verifies, contracts constrain); natural-language contract
   authoring; automatic migration synthesis for ecosystem-wide API evolution.
7. **Hardware co-design.** Generation-check acceleration via memory tagging (MTE/
   CHERI convergence — gen-refs are software CHERI; hardware can erase the cost).
8. **Edition-2030+:** revisit single-shot handler restriction, const-generic
   arithmetic, effect polymorphism sugar — guided by 5 years of telemetry.

---

# 18. Appendices

## 18.A Syntax cheat sheet (draft, subject to Phase-0 user testing)

```grasm
let x = 5                      // immutable binding
var y = 5.0                    // mutable
fn f(a: i32, inout b: Buf, own c: Conn) -> u64 ! IoError / {io.net}
type User = { id: u64, name: str, email: Option[str] }
type Event = | Click(x: i32, y: i32) | Key(code: u32)
interface Ord[T] { fn cmp(a: T, b: T) -> Ordering }
impl Ord[User] { fn cmp(a, b) { a.id.cmp(b.id) } }
effect Rand { fn next() -> u64 }
macro fn derive_json(t: TypeDef) -> [Decl] { ... }
comptime { let table = build_crc_table() }
region scratch { let buf = Buf.with_capacity(1<<20) ... } // bulk-freed
task.scope |s| { s.spawn { work() } }
assert / requires / ensures / invariant
unsafe { ptr.write(v) }        // needs package capability "unsafe"
```

## 18.B Comparison matrix (Grasm targets vs incumbents)

| Dimension | Go | Rust | Zig | Mojo | **Grasm target** |
|---|---|---|---|---|---|
| Memory safety (static, no GC) | ✗ (GC) | ✓ | ✗ | partial | ✓ |
| Lifetime annotations | — | required | — | some | **none** |
| Time to productivity | days | months | weeks | weeks | **days** |
| Uncolored async | ✓ (GC-based) | ✗ | ✓ (0.16+, dynamic) | ✗ | ✓ (typed effects) |
| Data-race freedom | ✗ | ✓ | ✗ | partial | ✓ |
| Incremental compile | good | poor | excellent | good | **excellent** |
| Capability-secured deps | ✗ | ✗ | ✗ | ✗ | **✓** |
| Deterministic replay | ✗ | ✗ | ✗ | ✗ | **✓** |
| Contracts/refinements | ✗ | ✗ | partial (comptime) | ✗ | **✓ gradual** |
| GPU single-source | ✗ | ✗ | ✗ | ✓ | ✓ (post-1.0 GA) |
| WASM Component first-class | ✗ | partial | partial | ✗ | **✓** |
| Editions + auto-migrate | ✗ | partial | ✗ | ✗ | **✓** |
| Governance | corporate | foundation | foundation | corporate | **foundation @1.0** |

## 18.C Bibliography / research inputs (selected)

- CISA/NSA, *Memory Safe Languages: Reducing Vulnerabilities in Modern Software
  Development* (June 2025) — media.defense.gov
- EU Cyber Resilience Act timeline; BSI NIS-2 guidance — corrode.dev/blog/memory-safety
- Zig devlog 2026 (incremental compilation w/ LLVM; io_uring std.Io; I/O-as-interface) — ziglang.org/devlog/2026
- Modular, *The Path to Mojo 1.0* (Dec 2025); Mojo 1.0.0b1 (May 2026) — modular.com
- Carbon safety design (Rust-aligned strategy) — docs.carbon-lang.dev/docs/design/safety
- Vale: generational references; linear style + regions; Higher RAII — vale.dev, verdagon.dev
- Hylo / mutable value semantics; safecpp lifetime-free safety draft — safecpp.org/draft-lifetimes.html
- *Why Algebraic Effects?* (Ante) — antelang.org/blog/why_effects
- Koka evidence-passing compilation; Effekt handler specialization
- Effect-system trade-off analysis — typesanitizer.com/blog/effects-convo.html
- Query-based compiler architectures; Salsa; rust-analyzer design docs
- WASI 0.3 / Component Model status 2026 — platform.uno, zeonedge.com, eunomia.dev
- Rust learning-curve & compile-time critiques; Go limitations threads; Go team
  error-syntax decision (June 2025) — go.dev/blog/error-syntax
- Discord Go→Rust migration (10× memory, 5× tail latency) — tech-insider.org/rust-vs-go-2026

---

# 19. Philosophy Alignment — The Path to True #1

> Founding philosophy (project owner): **world-#1 true language — easy to learn,
> easy to implement, minimal code needed, future-proof for the AI era, and a true
> innovation the world has never seen.** This section audits v0.1 against that
> philosophy and amends the design. These amendments are **binding** on Phase 0.

## 19.1 Audit of v0.1

| Criterion | v0.1 status | Verdict |
|---|---|---|
| Easy to learn | Progressive disclosure, zero lifetime annotations, G1 = days | ✅ mostly (effects UX remains the risk) |
| Easy to implement | 66-month compiler, large spec | ❌ **failed** — optimized for power, not implementability |
| Minimal code needed | Inference reduces boilerplate; concision not a stated goal | ◐ partial |
| Future-proof (AI) | Query API, machine diagnostics, contracts, LLM packs | ✅ strong; deepened below |
| Never-seen innovation | Novel *set*; each pillar individually has precedent | ◐ upgraded below |

## 19.2 Amendment A — The Kernel Language (Grasm-Core)

Grasm is redefined as **two layers**:

- **Grasm-Core:** ≤ ~30 constructs. Formal spec ≤ 50 pages. Reference interpreter
  ≤ 5,000 lines, implementable by one competent engineer in ~3 months (the
  Lua/Scheme/WASM property). Contains: values & tiered ownership, functions,
  ADTs + match, interfaces, error unions, effect rows + handlers, comptime, and
  nothing else.
- **Grasm-Full:** everything users see beyond core — async/task scopes, actors,
  channels, derives, string interpolation, comprehensions, contracts sugar —
  defined as **self-hosted libraries and macros that provably lower to Core**.

Consequences (each is a competitive weapon):
1. Alternative/embedded implementations become tractable → standardization-grade
   credibility, survives any single implementation's death.
2. Mechanized soundness proof targets Core only → auditable, feasible (§17.1).
3. Teaching follows the layer boundary: learn Core in a weekend, meet Full as
   sugar over what you already know.
4. **The full Core spec + semantics fits in one LLM context window.** No
   mainstream language can be *completely known* by an AI at inference time;
   Grasm-Core can. This makes Grasm the language AIs are most *correct* in — a
   structural moat that grows as AI writes more of the world's code.

New roadmap gates: Phase-0 exit adds "Core calculus defined, ≤ 30 constructs,
lowering demonstrated for async + derives." Phase-2 exit adds "independent
hobbyist implements a Core interpreter from spec alone, without asking questions."

## 19.3 Amendment B — Concision as a hard KPI (G11)

| # | Goal | Metric | Target |
|---|---|---|---|
| G11 | Minimal code | Token count vs Python/Go on an app-level benchmark suite (CLI, HTTP service, data transform, concurrent crawler) | ≤ 1.0× Python, ≤ 0.6× Go — while compiled, statically safe, no GC |

Levers: full local inference, pipelines (`xs |> filter(f) |> map(g)`), pattern
matching everywhere, derive-everything, default/named args, batteries stdlib.
Anti-lever discipline: concision never at the cost of readability (no APL golf);
the formatter's canonical output is the measured artifact. Published per release
next to performance numbers.

## 19.4 Amendment C — Intent-First Programming (the never-seen innovation)

**The flip:** in every prior language, the *body* is the source of truth and
comments/tests describe it. In Grasm, a function may declare its **intent** —
signature + contract + examples + prose — as the primary artifact, with the body
as a **derived, machine-verified implementation**:

```grasm
fn dedupe[T: Eq](xs: [T]) -> [T]
    ensures |r| r.is_unique() and r.all_in(xs) and r.preserves_order(xs)
    example dedupe([3,1,3,2]) == [3,1,2]
    example dedupe([])        == []
    body: @derive        // implementation synthesized (AI or human),
                         // admitted ONLY if verifier accepts it
```

The verifier ladder (reuses §9.5 machinery — no new theory needed):
1. SMT proof against `ensures` where decidable → body admitted, checks erased.
2. Property-based fuzzing of contracts + examples otherwise → admitted with
   runtime contract checks.
3. Neither passes → body rejected; the human sees a counterexample, not a diff.

Workflow: `grasm holes` lists unimplemented intents; any agent (or human) proposes
bodies; CI's verifier — not code review — is the merge gate for `@derive` bodies.
Regenerating a body against an updated intent is a normal build step; verified
bodies become cached, reproducible artifacts (locked by content hash next to the
lockfile). Prior art and the gap: Dafny/Verus prove hand-written code (experts
only); Copilot-class tools generate unverified code (everyone, unsafely). **No
language makes verification-gated machine implementation the default workflow.**
This is Grasm's identity claim:

> **Humans write intent. Machines write implementation. The compiler proves the
> match.**

Scope discipline (risk control): intent-first is *optional per function* — Grasm
without `@derive` is still a complete, excellent language (Amendments A/B and the
v0.1 pillars stand alone). If synthesis quality disappoints in early phases, the
feature degrades gracefully to "best contract + example system in industry,"
not a crater. Phase-1 adds a walking-skeleton of the verifier ladder; Phase-3
makes `@derive` public-beta with a curated model + local-model option (no
cloud lock-in; the verifier, not the model, is the trust anchor).

## 19.5 Moonshot decision record

| Candidate | Decision | Reason |
|---|---|---|
| Intent-first verified programming | **Adopted — headline** | Rides the structural AI shift; verifier-as-moat; graceful degradation |
| Content-addressed codebase, no text files (Unison-style) | Rejected as identity; adopted internally | Alienates git/review/tooling world → adoption killer. Its mechanics (content-addressed definitions) power the build cache & `@derive` artifact lock instead |
| Deterministic replay everywhere | Retained — second headline | Already §9.6; unique among mainstream candidates |

## 19.6 Revised identity statement

Grasm is: **(1)** a kernel language small enough for one person — or one AI
context window — to know completely; **(2)** with Rust-class safety at Go-class
effort and Python-class concision; **(3)** where all power (async, actors,
contracts) is library-level sugar over the small core; **(4)** where intent is
the source of truth and machine-written implementations are admitted only by
proof; **(5)** where every run can be replayed and every dependency is
capability-bound. #1 is claimed one property at a time — each independently
verifiable, each currently unclaimed.

---

# 20. Frontier Additions (v0.3) — Second Research Sweep

> Systematic sweep of remaining frontier areas (constant-time programming, resource
> analysis, session types, choreographies, local-first/CRDT, interaction nets,
> infrastructure-from-code, differentiable programming). Verdicts below; adopted
> items are binding amendments.

## 20.1 Adopted

### A. Secret types & compiler-preserved constant time (`secret T`)
*Research base:* FaCT (UCSD), Jasmin, "Robust Constant-Time Cryptography" (PLDI
2025), "Do Compilers Break Constant-Time Guarantees?" (FC 2025 — yes, they do).
Timing side-channels are a compiler problem: mainstream backends miscompile
hand-written constant-time code. Grasm adds an information-flow label to the type
system: `secret u8` cannot be branched on, indexed with, or leaked to a `public`
sink without explicit `declassify`. The guarantee is preserved **through codegen**
(constant-time selection instructions, no secret-dependent memory addresses) and
verified on emitted machine code in CI. Fits the existing lattice machinery
(effects are already row/label inference). **No mainstream language offers
end-to-end constant-time preservation.** Market: the regulated-crypto decade
(§3.1). Cost: small spec surface, one MIR pass + backend contract. Phase 3.

### B. Cost contracts — resource bounds as gradual types (G12)
*Research base:* RaML (CMU) — automatic amortized resource analysis derives
polynomial cost bounds for real functional programs; WCET industry practice.
Grasm reuses the §9.5 gradual-verification ladder for *resources*:

```grasm
fn parse(input: [u8]) -> Ast ! ParseError
    bounds time: O(input.len), alloc: O(input.len)   // proved → erased;
                                                     // else measured under test
```

Wins: DoS-proof parsers ("this endpoint is provably linear-time"), embedded/
real-time WCET stories, serverless cost prediction, energy as a metric (ticks
generalize to joules). Opportunistic like SMT contracts — never blocks a build.
**No mainstream language types resource use.** Spec Phase 2, GA post-1.0.

### C. Protocol types for channels & actors (session-type lite)
*Research base:* multiparty session types (Honda et al.), Rumpsteak/MultiCrusty
(Rust), 25 years of theory, zero mainstream ergonomic wins so far. Grasm adopts
the 80% version: a `protocol` declaration (message sequence/choice/recursion)
checkable on any channel or actor pair; compiler rejects out-of-order sends and
proves exhaustive handling + termination of the conversation; deadlock-freedom
for declared protocols. Full MPST ceremony rejected — opt-in, inference-first,
one keyword. Pairs with deterministic-schedule testing (§8.6). Phase 3+.

### D. Choreographic distribution (upgrade to §17.3)
*Research base:* CP is now a PLDI-co-located conference (2026); ChoRus proves
library-level feasibility in Rust. Write a distributed system as **one program**;
endpoint projection compiles per-node binaries; Grasm's effects mark the
communication boundary, and the deterministic simulator (§9.6) runs the *whole
choreography* on one laptop with fault injection. Distribution becomes: write
once, project, simulate, deploy. This composes three things Grasm already has —
effects, replay, WASM components — into a capability nobody ships. Post-1.0
flagship (Phase 5), spec'd during Phase 3.

### E. Mergeable data types (local-first, stdlib-level)
*Research base:* Automerge 3.0 (2025, 10× memory cut), Loro 1.0, ElectricSQL/
PowerSync sync engines; FOSDEM 2026 dedicated track — the movement is real.
Verdict: language-core CRDTs are premature, but Grasm stdlib ships `merge`-able
data types (a `Mergeable` interface + CRDT implementations: counters, maps,
lists, text) with sync-boundary-as-effect (`io.sync` handler = pluggable engine).
Local-first apps get offline/collab semantics with static types; Grasm becomes
the best compile-to-WASM language for the local-first ecosystem. Phase 3 stdlib.

### F. Sandboxed FFI by default (RLBox pattern productized)
*Research base:* RLBox (Firefox, production-proven): compile untrusted C deps to
WASM, run in-process with tainted-type boundaries. Grasm: `import c` gains
`sandbox` mode — the C dependency is compiled to a WASM component, its memory
isolated, its returns quarantined as tainted values requiring validation. The
"FFI capability" (§8.8) then has two tiers: `ffi.native` (audited, fast) and
`ffi.sandboxed` (default for registry packages). Kills the "safe language, unsafe
C dependency" hole every competitor accepts. Phase 3.

### G. Differentiable & probabilistic programming via handlers (§17 upgrade)
*Research base:* "Automatic Differentiation via Effects and Handlers" (Sigal),
effect-handler AD correctness proofs (de Vilhena & Pottier 2023), JAX-on-MLIR
precedent (Reactant lowers to the same StableHLO stack Grasm's mid-end targets).
Gradients as an effect handler over `kernel` functions: `grad(f)` is a handler
interpretation, compiled through MLIR to fused kernels. Same trick yields
probabilistic programming (sampling as effect). A systems language with typed,
zero-cost autodiff = scientific/AI computing wedge without becoming an AI-vertical
language (the Mojo trap). Research track now, product post-1.0.

## 20.2 Rejected (with reasons — decision record)

| Candidate | Verdict | Reason |
|---|---|---|
| Interaction combinators / HVM-Bend evaluation model | **Watch, don't adopt** | Genuinely novel (near-linear GPU scaling of high-level code) but interpreter-on-GPU overhead makes it slower than one tuned CPU core for most real work today; wrong foundation for a systems language. Revisit if HVM-class runtimes mature. MLIR `kernel` path covers the practical need |
| Cloud primitives in the language (Wing/Darklang model) | **Reject core; adopt as library** | Both pioneers validated the *pain* and failed as *languages* (Wing startup dead 2026, Darklang pivoted/rebuilt). Lesson: infrastructure-from-code must be an effect-handler library (deploy-time handler provisions resources), never language surface |
| Language-core CRDT semantics | Reject core | Merge semantics evolve too fast to freeze in a spec; stdlib interface + ecosystem (§20.1-E) captures the value without the fossilization risk |
| Full multiparty session types | Reject full ceremony | 25 years of theory, no ergonomic mainstream win; protocol-lite (§20.1-C) takes the 80% |
| Content-addressed codebase (revisited) | Stays rejected (§19.5) | Unchanged |

## 20.3 Updated goal table additions

| # | Goal | Metric | Target |
|---|---|---|---|
| G12 | Resource transparency | Stdlib/public-API functions with proved cost bounds | ≥ 60% of stdlib by 2.0 |
| G13 | Side-channel safety | Crypto-relevant stdlib code constant-time-verified on emitted machine code | 100% of `crypto` module |
| G14 | Untrusted-dep isolation | Registry packages with native (non-sandboxed) FFI | ≤ 5%, each human-audited |

## 20.4 Completeness assessment

With §19 + §20, the plan now covers every currently-identified frontier axis:
memory (tiers), control (effects), correctness (intent/contracts), time
(replay/determinism), cost (bounds), confidentiality (secret types + capabilities),
distribution (choreographies + components), collaboration (mergeable data),
differentiation (handlers/MLIR), evolution (editions), implementation
(kernel core), and AI symbiosis (context-window spec + verifier-gated synthesis).
Remaining unknowns are *execution* unknowns, not *design* unknowns. Frontier
sweeps recur each phase gate; next scheduled review: Phase-0 exit.

---

*End of master plan v0.3.0. Next actions: Phase-0 kickoff — (1) Grasm-Core
calculus draft (≤30 constructs) + lowering sketches, (2) effect-compilation
micro-prototype, (3) tiered-ownership checker prototype, (4) verifier-ladder
walking skeleton (contract → SMT/fuzz → admit/reject), (5) syntax user-testing
deck, (6) spec repo scaffold.*
