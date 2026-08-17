# Schedule-as-handler — the falsification test, and its verdict

**Verdict: the idea fails as stated.** It failed in a day, which is what the
test was sized for.

The claim was that Halide's separation of *algorithm* from *schedule* could be
expressed with Grasm's algebraic effect handlers — one algorithm written once, a
handler deciding how it runs — giving the separation without a second language.

## What was measured

`matmul.gr` is one algorithm text. The tile size comes from a `Schedule` effect,
and `tile = n` degenerates exactly to the naive triple loop, so every schedule
runs the same source and any difference is the schedule alone.

Written down in advance, the test fails if any of these holds:

| Criterion | Result |
|---|---|
| A handled schedule is not meaningfully faster than the plain loop | **FAILED** |
| Expressing a real scheduling decision needs syntax the language lacks | **FAILED** |
| The handler version is harder to read than the fast loop by hand | passed — it reads well |
| The three engines stop agreeing | passed — all three agree exactly |

## 1. Tiling bought nothing

| n | working set | naive | best tiled | speedup |
|---|---|---|---|---|
| 384 | 3.4 MB | 95 ms | 94 ms | 1.01× |
| 640 | 9.6 MB | 436 ms | 435 ms | 1.00× |
| 896 | 18.8 MB | 1191 ms | 1191 ms | 1.00× |

Nothing, at any size, including working sets far past any cache.

## 2. But scheduling *does* matter — just not this dimension

Loop order, same arithmetic, n = 512:

| order | time |
|---|---|
| `ijk` (accumulate in a local) | **133 ms** |
| `ikj` (read-modify-write the output) | 244 ms |
| `jki` (strided everywhere) | 741 ms |

**5.6×.** So the schedule is worth expressing. The dimension that pays is loop
order and accumulation strategy, not tile size.

## 3. And that is exactly what a handler cannot express

This is the fatal one, and it is structural rather than incidental.

A tile size is a **value**, and a handler supplies values well — it typechecked,
it ran, and all three engines agreed. Loop *order* is not a value. Expressing it
through a handler means either indirecting every index through a permutation, at
a cost larger than the win, or having the handler pick among pre-written loop
nests — which is a function pointer, and abandons "one algorithm written once".

**Effect handlers are runtime dispatch. Halide schedules are compile-time
transformations.** A handler can parameterise a loop nest and cannot restructure
one. That gap is the idea, and it does not survive.

## What this rules out, and what it points at

Ruled out: schedules as effect handlers.

Pointed at: if Grasm ever wants Halide-style scheduling, the mechanism is
**compile-time transformation** — comptime or macros — not effects. That is a
much larger piece of work than an afternoon's experiment, and it should not be
started on the strength of an analogy that has now been tested and broken.

## One measurement error worth recording

Chasing why `ikj` lost to `ijk`, a loop of `f = f + 1.0` measured 5× slower than
the same loop over integers, and the disassembly showed `fmov d20, #1.0` inside
the loop — Cranelift has no loop-invariant code motion. That looked like a
codegen defect.

It was not. Caching constants per function to fix it produced Cranelift verifier
errors, because a value defined in one block cannot dominate uses in another —
which is *why* Cranelift rematerialises constants. And the real cause was
simpler: four independent accumulator chains ran the same number of float adds in
**17 ms against 89 ms** for one serial chain. It was dependency-chain latency,
and the generated code was fine.

The change was reverted. The lesson is that a disassembly that looks wrong is not
evidence until the alternative has been measured.
