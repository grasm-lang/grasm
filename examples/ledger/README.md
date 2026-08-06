# A ledger whose dependencies are all effects

```
grasm run  examples/ledger/main.gr
grasm test examples/ledger/main.gr
```

The clock, the configuration, and the audit trail are effects. The logic
cannot tell real dependencies from three lines of test double, and the
compiler will not let it pretend otherwise — an effect that reaches `main`
unhandled is a compile error, so a function that reads a clock cannot claim
it doesn't.

The tests are the point. There is no mocking framework and nothing to
register: a test supplies a handler, including a 50% fee no real schedule
would have, and the code under test is unchanged.

## What writing it found

The first version had a `Store` effect with `read`, `write`, and `accounts`,
backed by a list the handler closed over. It ran. It printed plausible
numbers. The numbers were wrong — a balance that had just been credited read
back at its old value.

Handler operations are closures, and **closures capture values, not
bindings**. Every `write` updated a copy; every `read` saw the original.

That rule was nowhere in the specification. `[E-SPAWN-SHARED]` documented the
*opposite* behaviour for `spawn { … }`, which is the other block-shaped
construct, so the only written guidance pointed the wrong way. It is
`[E-CLOSURE-CAPTURE]` now, with a program, and it says the consequence
outright: **an effect handler cannot hold mutable state.**

So this version threads the book through `inout` and leaves the effects doing
what they can honestly do — supplying values and accepting output. That is a
real constraint on the effect system, and worth knowing before you design
around it rather than after.
