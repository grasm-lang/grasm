# Known issues

Every defect that is known and not fixed, with its severity from
`docs/release-scope.md`, the clause it touches, and a reproducer small enough
to paste. S1 and S2 block a release; S3 and S4 do not, and this file is what
makes shipping them honest rather than quiet.

**Open S1: none. Open S2: none.**

## S3 — over-strict, effect-row inference

### The fallback pool still charges a part nothing is known about

[T-FN-TYPE-ROW-CARRIED] (`experimental`). Where a function value is bound by a
binder this pass does not model — a `match` payload, a tuple destructuring, a
closure parameter — the row charged is every row written in the enclosing
function's own signature at a position with no field name. That is deliberate
and documented, and it is conservative in the safe direction, but it means a
program can be refused for a row it does not perform:

```
type Cb = fn(i64) -> i64
type Loud = fn(i64) -> i64 / {io}

fn go(v: V, l: Loud) -> i64 / {} {   // `l` is never called
    match v {
        Hold(f) -> f(1)              // `f` is pure, and is charged `io`
        Nil -> 0
    }
}
```

The fix is not another cell of the grid: it is rows travelling through the type
checker's unification, so a binder does not have to be modelled at all. Until
then this is the known cost of the approximation.

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
