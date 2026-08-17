# 08 — Fibonacci Sequence Generator

**Tests:** recursion, call stacks, memoization, and `[E-ARITH-INT]` overflow.

**Result:** works. All three engines agree.

## What it actually exercises

Three implementations — iterative, naive recursive, memoized — are checked
against each other on every value from 0 to 19. A call-stack bug shows up as a
disagreement between them rather than as a crash, which is a much better signal.

The overflow section does not avoid the edge; it walks into it. `[E-ARITH-INT]`
specifies wrapping, so:

* `F(92) = 7540113804746346429` — the last one that fits in an `i64`.
* `F(93) = -6246583658587674878` — wrapped, and *specified* to wrap.

Detecting it is the caller's job, and the program shows how: Fibonacci is
strictly increasing, so a term smaller than its predecessor has wrapped. It
reports `F(93)`, which is correct.

`depth(50000)` confirms recursion goes as deep as [E-RECURSION-DEPTH] promises
on a native host. (It does not on wasm — see `docs/wasm.md`.)

## Findings

**F4** — `for _ in 0..n` is refused; `_` is a pattern in `match` and nowhere
else. Found here.
