# North-star goals, measured

MASTERPLAN §2.3 sets ten measurable goals and §19.3 adds an eleventh. Four of
them had never been measured — not missed, *unmeasured*, which is worse: a
number nobody takes cannot be argued with or improved, and G3 was found to be
violated only by accident, when an evaluator happened to time a chain of field
accesses and found the checker taking exponential time on it.

`scripts/northstar.sh` takes the measurements that can be taken. It prints a
scorecard rather than failing on a miss, because three of these are missed today
and a gate that is red on purpose is a gate people learn to ignore. It *does*
fail when a measurement cannot be taken at all, which is how a metric quietly
stops being collected.

Figures below were taken on 2026-08-17 on an idle machine.

| # | Goal | Target | Measured | |
|---|---|---|---|---|
| G1 | Time for a Go/TS dev to ship a real service | ≤ 1 week | — | **unmeasurable without users** |
| G2 | Geomean vs Rust | ≥ 0.95× | 4 of 7 programs at or above; `wordfreq` 2.5× behind | **partly met** |
| G3 | Frontend throughput | ≥ 500k LOC/s | 458k–500k, mean ~483k | **borderline** |
| G3 | Incremental edit-to-diagnostic | ≤ 50 ms | — | **not built** — there is no query engine |
| G4 | Safe-subset guarantees | 100% | 204 programs identical across three engines, 204 leak-free, row and FFI soundness fuzzers clean | **strongest area** |
| G5 | Hello-world static binary | ≤ 300 KB | 328 KB | **missed by 9%** |
| G6 | Structured, uncolored concurrency | 1M+ tasks/node | `spawn` is deliberately single-core ([E-NOT-PARALLEL]); `par_map` is the only multicore path | **retreated** |
| G7 | Capability-sandboxed dependencies | 100% | the supply-chain benchmark's tamper is refused as `performs [io] permits []` | **met, for what exists** |
| G8 | Production failure → deterministic replay | ≥ 99% | — | **not built** |
| G9 | C ABI import overhead | zero-cost | `extern fn`, gated by `unsafe` and charged `io` | **partly met** |
| G10 | Adoption | ≥ 500 companies | zero — nobody outside this repository has run a Grasm program | **not started** |
| G11 | Tokens vs Python | ≤ 1.00× | **1.41×** | **missed by 41%** |
| G11 | Tokens vs Go | ≤ 0.60× | **0.74×** | **missed by 23%** |

## What the numbers say

**G3 is borderline and only just.** 483k against 500k is 97% of target — and it
is that only after three superlinear defects were fixed on 2026-08-17: a field
chain that was exponential, a method chain that was exponential, and a nested
tuple literal that was cubic. Before those, an ordinary program with a long
projection chain did not finish at all. The second half of G3 — the ≤50 ms
incremental figure, which is what the goal is actually about — has no
measurement because the "compiler as a queryable incremental database" the
executive summary describes was never built. What exists is a batch compiler
with an object cache.

**G11 is the clearest miss, and the most interesting one.** Grasm is 1.41× the
token count of Python where the goal is to be *below* it. Per program:

| | fib | sieve | matmul | wordfreq | sort | parmap |
|---|---|---|---|---|---|---|
| vs Python | 1.27× | 1.40× | 1.48× | 1.47× | 1.31× | 1.38× |
| vs Go | 1.00× | 0.88× | 0.86× | 0.99× | 0.71× | **0.44×** |

The one place the target is beaten outright is `parmap` — 0.44× of Go — which is
the concurrency program. That is the effect system paying for itself, and it is
evidence for the thesis: where Grasm's ideas are load-bearing it is half the
size of Go, and where a program is a loop over an array it is a typed language
paying typed-language costs. Closing the Python gap is a *syntax* question, not
a semantics one, and §19.3 made it a hard KPI for exactly that reason.

**G5 is missed by 9%**, down from 46%. The binary went 437 KB → 328 KB by
stripping the symbol table, which nothing reads at run time: a compiled program
carries no reflection and a `--native` build prints no stack trace at all
([E-PANIC-TRACE]). Two other levers were tried and reverted, both measured:

* `panic = "abort"` saved **1 KB**. The 39 KB of unwinding tables in a compiled
  program come from Rust's *precompiled* `std`, which the setting cannot reach.
* `opt-level = "z"` on the runtime made the binary **4 KB bigger**. Optimising
  the runtime for size blocks the inlining that lets link-time optimisation
  delete the code around it — the archive shrinks and the binary grows.

The remaining 28 KB is `std`: `core::fmt`'s formatting machinery, the allocator,
and those unwinding tables. Reaching 300 KB needs `-Z build-std` (nightly, so
not available to a stable toolchain) or a `no_std` runtime, which means
reimplementing float formatting, allocation and threads. That is a real project
with a known shape, not an open question — and it is the only thing between here
and the goal.

**G1 and G10 cannot be moved from inside this repository.** No human has run a
Grasm program. Every signal the project has is model-generated, and the two
goals that measure whether the language is *good to use* are the two with no
data at all.

## What this changes

The scorecard says the effort has gone almost entirely into G4 and G7 — safety
and capability soundness — which are the two strongest columns and were never
the differentiator. G3, G5 and G11 are all *measurable, close, and unattended*;
G1 and G10 need someone outside; G6, G8 and G10 measure features that do not
exist.

Re-run it with `./scripts/northstar.sh`.
