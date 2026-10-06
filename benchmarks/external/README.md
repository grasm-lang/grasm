# Benchmarks somebody else designed

`benchmarks/` is ours, which is the problem with it: we chose the programs. This
directory is for workloads other people made claims about, run here, with the
other languages' implementations beside ours so anyone can check.

Every one holds to the same three rules. **The same algorithm** — not the same
output by a cleverer route. **The same answer, checked before timing** — a
comparison of two programs that disagree is worth nothing. **Interleaved runs,
medians reported** — this machine drifts 30% under load, so a sequential sweep
cannot separate four implementations.

## mandelbrot — "N times faster than Python"

The kernel behind every such claim, including Modular's 35,000x figure for
Mojo. 3000x2000 points, 200 iterations, counting the points inside; all five
implementations answer **1272341**.

Medians of seven interleaved rounds on an 8-core Apple machine (6 performance,
2 efficiency) at load 8-11. Python is the best of two, because it takes 36
seconds.

| | time | vs Python | vs Rust |
|---|---:|---:|---:|
| Python 3 | 36.04 s | 1x | — |
| Grasm, `map` (one core) | 0.99 s | **36x** | 0.92x |
| Rust, single-threaded | 0.91 s | 40x | 1.00x |
| C, `-O3`, single-threaded | 0.87 s | 41x | 1.05x |
| Grasm, `par_map` (8 cores) | **0.17 s** | **212x** | 5.35x |

Two things worth taking from this.

**Grasm is 0.92x of Rust on one core here** — the same ratio `matmul` gives.
Two independent compute-bound float kernels landing on the same number is
better evidence about where the backend stands than either alone.

**The honest "faster than Python" number is 212x, not 35,000x.** Modular's
figure is not wrong, it is a different measurement: a 16-core AWS instance
against our 8, explicit SIMD vectorisation against our none, and the same naive
pure Python denominator. Most of the distance between 212 and 35,000 is
hardware and vector width rather than language. Anyone quoting a number like
that should say which three things it includes, and `run.sh` says which three
these include.

`par_map` buys 5.8x over Grasm's own serial run on 8 cores. That it helps here
at all is the point: this kernel is arithmetic-bound, which `docs/apps-phases.md`
records as the condition for it — the same `par_map` over a memory-bound audio
mixdown measured **6.3 s of wall time and 40 s of CPU against 1.2 s on one
core**.

## Running it

    ./mandelbrot/run.sh

It builds all five, refuses to time anything until every implementation agrees
on the answer, and prints each round rather than a single number, so the spread
is visible. `ROUNDS=15` for a tighter median.

## Still to add

`nbody` from the Computer Language Benchmarks Game, which is the other workload
Modular published figures for. Mojo itself, when there is a build to install.
Zig, which is already in the main suite and belongs here too.
