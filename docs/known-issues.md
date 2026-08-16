# Known issues

Every defect that is known and not fixed, with its severity from
`docs/release-scope.md`, the clause it touches, and a reproducer small enough
to paste. S1 and S2 block a release; S3 and S4 do not, and this file is what
makes shipping them honest rather than quiet.

**Open S1: none. Open S2: none.** One S3 closed on 2026-08-16, and the
mechanism that produced it was removed.

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

### `--native` refuses shapes the interpreter accepts

The §11 table in the specification lists them, `scripts/nativegaps.sh` proves
the list matches the compiler in both directions, and every refusal names
itself and points at `grasm run`. Three conformance programs are in this state
today (`T-REF-CAPTURE`, `T-REF-WRITE`, `T-EFFECT-OP-RESULT`). Refusals, not
miscompiles.

## S4 — diagnostics

### `x.f()` does not reach a library free function

[L-UFCS] (`stable`) says a free function whose first parameter accepts the
receiver may be called in method position, and that this "applies to functions
a program declares, not only to the library". Functions a program declares do
work (`5.double()`); the built-in library ones do not:

```
fn main() {
    "hi".print()
}
```

```
error: no method `print` on `str`
  help: a free function `fn print(str, ...)` would also work here (UFCS)
```

The help describes exactly the call that was refused. `"hi" |> print` works and
is charged `io` correctly, so nothing is unreachable — but either the clause
should exclude the built-ins or the resolution should include them, and the
help should not promise what the compiler will not do.
