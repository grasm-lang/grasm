# Compile speed, measured for the first time

`MASTERPLAN.md` §2.3 sets **G3: ≥500k LOC/s frontend, ≤50 ms incremental
edit-to-diagnostic**. `benchmarks/README.md` listed compile time under "Not
benchmarked" and it had never been measured once. This is that measurement,
taken 2026-10-07 on an 8-core Apple machine at load 3.3.

The programs are generated rather than taken from the corpus, because
throughput needs single compilation units of a known size: `scratchpad/cs/gen.py`
writes realistic Grasm -- a record type, a sum type, and three functions per
unit doing a `match`, a loop with a branch, and a struct literal with
interpolation -- rather than one line repeated. An equivalent Rust generator
writes the same shapes for comparison.

## The frontend is fine, and under target

`grasm check` is parse plus typecheck, which is what G3 names.

| lines | best of 5 | LOC/s |
|---:|---:|---:|
| 5,052 | 38 ms | 133,000 |
| 25,052 | 76 ms | 330,000 |
| 75,052 | 188 ms | 399,000 |
| 175,052 | 479 ms | 365,000 |

A linear fit over the four is **382,000 LOC/s marginal with 11.6 ms of fixed
startup**. Throughput climbs as that startup amortises and then flattens, so
the honest figure is **365-400k LOC/s against a 500k target: missed by about a
quarter**, and missed for an ordinary reason rather than a pathological one.

**Edit to diagnostic** is met for a file a person edits and missed for a large
one: 39 ms at 5,052 lines, **80 ms at 25,052** against a 50 ms target. The
crossover is around 15k lines in one unit.

## The native backend is superlinear, and that is a defect

| | Grasm | rustc `-O` |
|---|---:|---:|
| ~21-25k lines | 1,549 ms (16,173 LOC/s) | 2,013 ms (10,455 LOC/s) |
| ~63-75k lines | 11,662 ms (6,436 LOC/s) | 3,497 ms (18,029 LOC/s) |
| **3x the lines costs** | **7.5x the time** | **1.7x the time** |

At the size of one ordinary module Grasm's whole pipeline is **1.3x faster
than `rustc -O`** and level with `rustc -O0` (1,216 ms). Three times the code
and `rustc -O` is **2.8x faster than Grasm**. rustc is sublinear there, as it
should be -- fixed costs amortising -- and Grasm is steeply superlinear.

Two things it is not:

- **Not the incremental cache.** `--no-incremental` at 75k lines takes
  11,581 ms against the cache's 11,215: identical. (The cache does work at
  5k lines, where a rebuild is 269 ms against a 633 ms cold build, and stops
  paying above that -- which is the same defect seen from the other side.)
- **Not the frontend.** `check` over the same step is 76 -> 188 ms, 2.5x for
  3x the lines. Sublinear, like rustc's.

So the cost is in whole-program native code generation, and something there
scans more than it should as the program grows. Finding it is the next step,
and it is worth taking: it is the difference between a compiler that is faster
than `rustc -O` and one that is three times slower, on the same code.

## What this changes

Cranelift's design point is fast code generation, and the assumption here has
been that giving up LLVM's optimiser bought compile speed. It buys it **at
module scale and loses it at program scale**, which is exactly the regime a
real codebase lives in. The claim "Grasm compiles faster than Rust" is true at
25k lines and false at 75k, and should not be made until the superlinearity is
gone.

Reproduce with `scratchpad/cs/gen.py N` and `genrs.py N`, timing
`grasm check`, `grasm build --native` and `rustc -O` on the output.
