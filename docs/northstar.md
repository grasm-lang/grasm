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

Figures below were taken on 2026-08-17 on an idle machine. Load matters: the
same build measured 478k LOC/s at load average 9 and 571k at load average 3, so
the scorecard takes the best of five runs and subtracts the process floor.

| # | Goal | Target | Measured | |
|---|---|---|---|---|
| G1 | Time for a Go/TS dev to ship a real service | ≤ 1 week | — | **unmeasurable without users** |
| G2 | Geomean vs Rust | ≥ 0.95× | 4 of 7 programs at or above; `wordfreq` 2.5× behind | **partly met** |
| G3 | Frontend throughput | ≥ 500k LOC/s | **571k** | **met** |
| G3 | Incremental edit-to-diagnostic | ≤ 50 ms | — | **not built** — there is no query engine |
| G4 | Safe-subset guarantees | 100% | 204 programs identical across three engines, 204 leak-free, row and FFI soundness fuzzers clean | **strongest area** |
| G5 | Hello-world static binary | ≤ 300 KB | 328 KB | **missed by 9%** |
| G6 | Structured, uncolored concurrency | 1M+ tasks/node | `spawn` is deliberately single-core ([E-NOT-PARALLEL]); `par_map` is the only multicore path | **retreated** |
| G7 | Capability-sandboxed dependencies | 100% | the supply-chain benchmark's tamper is refused as `performs [io] permits []` | **met, for what exists** |
| G8 | Production failure → deterministic replay | ≥ 99% | — | **not built** |
| G9 | C ABI import overhead | zero-cost | `extern fn`, gated by `unsafe` and charged `io` | **partly met** |
| G10 | Adoption | ≥ 500 companies | zero — nobody outside this repository has run a Grasm program | **not started** |
| G11 | Tokens vs Python | ≤ 1.00× | **1.04×** | **4% over** |
| G11 | Tokens vs Go | ≤ 0.60× | **0.61×** | **2% over** |

## What the numbers say

**G3 is met: 571k LOC/s against 500k.** It got there in four steps, all of them
found by measuring rather than guessing:

1. Three superlinear defects fixed — a field chain that was exponential, a
   method chain that was exponential, a nested tuple literal that was cubic.
   Before those, an ordinary program with a long projection chain did not finish
   at all, and no throughput figure meant anything.
2. Memoizing inference for the forms that cannot hold a closure. Added for the
   pathological chains, it turned out to be **2.2× on ordinary code too** — 257
   ms to 115 ms on a 47,000-line program.
3. Not freeing the syntax tree at exit. A third of `grasm check` was
   `drop_in_place` walking the tree node by node to release it, immediately
   before the process ended and the operating system reclaimed the whole address
   space in one step.
4. A non-cryptographic hasher for the tables keyed by span. The default hasher
   resists an attacker choosing collisions; the keys here are byte offsets in a
   file the compiler is already reading.

Two things were nearly shipped and caught. Exiting the process outright to skip
*all* destructors is simpler than step 3 and collapsed every program's own exit
code to 1, because an `ExitCode` cannot be read back. And the first version of
this scorecard shelled out for each timestamp, putting 25 ms of helper startup
*inside* the interval — a measurement of itself.

The second half of G3 — the ≤50 ms incremental figure, which is what the goal is
really about — still has no measurement, because the "compiler as a queryable
incremental database" the executive summary describes was never built. What
exists is a batch compiler with an object cache.

**G11 reads 1.04× of Python and 0.61× of Go**, against targets of 1.00× and
0.60×. Two separate things got it there, and only one of them is an improvement.

**The ruler was wrong.** The first measurement counted tokens with a regular
expression, which cannot compare these three languages fairly: Python's grammar
has `INDENT`, `DEDENT` and `NEWLINE` tokens and Go's lexer inserts semicolons,
so a regex over punctuation gives both of those away free and charges Grasm for
every brace. Braces were 55% of the apparent gap. Counted by each language's own
lexer — `grasm tokens`, Python's `tokenize`, Go's `go/scanner` — the same
programs read 1.04× rather than 1.22×. **The language did not get shorter; the
measurement got honest.**

**The language did also get shorter**, by 1.41× → 1.22× on the *unchanged* ruler,
from two things. A range gained the list combinators ([L-LIB-RANGE-OPS]), so
building a list from a count stopped needing `var xs: [i64] = []`, a `for`, a
`push` and two closing braces. And three of the six benchmark programs were
accumulating by hand where `.sum()` or `.values()` already existed — not a
language cost, and not what the benchmark README promises. One rewrite went the
wrong way and was reverted: replacing `sort`'s push loop with a
map-then-overwrite made it eleven tokens longer, and Python writes that one as a
push loop too.

Per program, by each language's own lexer:

| | fib | sieve | matmul | wordfreq | sort | parmap |
|---|---|---|---|---|---|---|
| vs Python | 1.12× | 1.13× | 1.10× | **0.98×** | 1.13× | **0.79×** |
| vs Go | 0.93× | 0.85× | 0.74× | **0.66×** | 0.70× | **0.29×** |

`parmap` is the number to read: **0.29× of Go and 0.79× of Python** — a
compiled, statically safe, GC-free language at four fifths of Python's length on
the concurrency program. That is the thesis holding where it is tested hardest.

**The remaining 4% is 19 tokens across six programs, and all of it is the entry
point.** `fn main() { … }` costs six tokens — `fn`, the name, two parentheses,
two braces — and six programs pay it: 36 tokens. Grasm's actual code is 521
against Python's 538, or **0.97×**.

That figure is *not* the one reported above, and deliberately. A Python
benchmark is a script and has no entry point; a compiled systems language names
where a program starts. Comparing the two without the entry point is comparing
different things — but so is quietly dropping it because the number comes out
better, which is the move this file exists to refuse. The goal is missed by 4%,
the 4% is entirely `fn main`, and both sentences are true.

Allowing top-level statements as an implicit `main` would close it and is an
exact rewrite, so it would cost no kernel form. It is not done, because it gives
a language two ways to write a program to win a metric by 19 tokens, and
principle 5 of the master plan is "one way to do it".

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
