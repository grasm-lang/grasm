# Known issues

Every defect that is known and not fixed, with its severity from
`docs/release-scope.md`, the clause it touches, and a reproducer small enough
to paste. S1 and S2 block a release; S3 and S4 do not, and this file is what
makes shipping them honest rather than quiet.

**Open S1: none. Open S2: none.** One S3 closed on 2026-08-16.

## S3 — over-strict, effect-row inference

### The fallback pool still charges a part nothing is known about — *closed*

[T-FN-TYPE-ROW-CARRIED] (`experimental`). Closed on 2026-08-16 by the first
stage of the effect-row rewrite: the type checker now publishes the row of
every function-valued expression it types, and the effect pass reads that
instead of guessing which binder a value came out of. A `match` payload, a
tuple destructuring and a `for` variable are now charged by their type, so a
`/ {}` body that performs nothing is no longer refused because an unrelated
parameter of the same function declares `io`.

The syntactic path remains as a fallback for expressions the checker could not
type. Removing it entirely is the rest of the rewrite.

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
