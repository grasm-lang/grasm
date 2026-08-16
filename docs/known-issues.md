# Known issues

Every defect that is known and not fixed, with its severity from
`docs/release-scope.md`, the clause it touches, and a reproducer small enough
to paste. S1 and S2 block a release; S3 and S4 do not, and this file is what
makes shipping them honest rather than quiet.

**Open S1: none. Open S2: none. Open S3: none. Open S4: none.**

Closed on 2026-08-16: the effect-row fallback pool (and the mechanism behind
it), writes through a `ref` under `--native`, returning a closure under
`--native`, and the UFCS help text.

This file is not empty because nothing is wrong — it is empty because what was
wrong is listed above as closed. The next round of evaluation is what refills
it.

## S3 — over-strict, effect-row inference

### The fallback pool — *removed*

[T-FN-TYPE-ROW-CARRIED] (`experimental`). Closed on 2026-08-16, and then the
mechanism behind it was deleted. The type checker publishes the row of every
function-valued expression it types and of every call site whose callee is not
an expression of its own; the effect pass reads that. The syntactic
reconstruction — a map of name to sources, a pool of every function value a
body mentions, the rows a signature writes at nameless positions, and the
per-binder special cases that fed them — is gone, 511 lines of it.

There is no fallback pool to be over-strict any more, so the shape recorded
here (a `match` payload charged a sibling parameter's row) cannot recur.

### `--native` — *no gaps in the repository's programs*

Every program in `spec/conformance/`, `examples/`, `projects/` and
`benchmarks/` compiles natively: **204 identical across all three engines, 0
unsupported, 0 leaking.** The §11 table still lists shapes the backend refuses
— a generic `type` declaration, a handler used as a value, calling a function
value through anything but a name, and others — and
`scripts/nativegaps.sh` proves that list matches the compiler in both
directions. Each is a refusal by name at build time, never a miscompile.

Closed on 2026-08-16: writes through a `ref` (including around a cycle) and
returning a closure.

## S4 — diagnostics

### `x.f()` does not reach a library free function — *closed*

Closed on 2026-08-16 by narrowing the clause rather than widening the
implementation. [L-UFCS] now says the language's own free functions are not
methods of any type, and `"hi".print()` is refused with help that names the
spelling that works — `"hi" |> print`, which was always there and is charged
the same row a call is. The old help offered UFCS for `print`, which is the
call it had just refused.
