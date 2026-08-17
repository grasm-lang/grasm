# 19 — Event Emitter / Pub-Sub Engine
**Tests:** function values, callback lists, closures over state — and effect rows. **Result:** works, all three engines agree.

This is where Grasm's type system says something most languages cannot say.

A listener is a function value, and in Grasm a function value carries its
**effect row** in its type ([T-FN-TYPE-ROW]). So a pipeline whose rules are typed
`fn(i64) -> i64` — a *written empty row* — cannot be handed a rule that prints,
and the refusal lands at the registration site rather than when the pipeline
eventually runs. Verified, not asserted:

```
error: this closure performs `io`, which the expected type does not allow
  help: widen the type's effect row, or move the effect out of the closure
error: this expects a function that performs nothing, but this one performs `io`
```

"This callback cannot do I/O" is a claim most languages cannot make at all.

The rest works as expected: registration order preserved, removal idempotent,
emitting to nothing returns 0, and a closure counting into shared state via `ref`.

## Findings
**F10** — a `ref` to a scalar can be constructed and never read. `ref(0)`
typechecks, and [T-REF-READ]/[T-REF-WRITE] define access through a *field*,
which a scalar has none of. The workaround is a one-field wrapper struct, which
this project uses.

Also noted: a braceless closure body must be an expression, and assignment is a
statement — so `|p| x = 1` needs braces. Consistent, and the diagnostic was clear.
