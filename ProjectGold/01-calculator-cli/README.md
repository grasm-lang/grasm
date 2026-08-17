# 01 — Calculator CLI

**Tests:** operator precedence, an explicit AST, recursive descent, `ref` for
recursive types, mutual recursion, `inout` cursors, typed error unions.

**Result:** works. All three engines agree.

## What it actually exercises

An AST is built rather than evaluated during the parse. That separates the two
questions — does the grammar bind `*` tighter than `+`, and does the evaluator
walk a tree correctly — because a one-pass evaluator can get the right answer
with the wrong precedence and nobody finds out until expressions get longer.

The tree needs `ref Node`: a variant holding another node of its own type has no
fixed size, and `ref T` is the copyable generational handle that gives it one
([T-REF-GENERATIONAL]). This is the first project that forces that.

Associativity is checked in both directions, which is the part that usually
breaks:

* `2 ^ 3 ^ 2 = 512` — right-associative, so `2 ^ (3 ^ 2)` and not `(2 ^ 3) ^ 2 = 64`.
* `100 / 10 / 2 = 5` — left-associative, so `(100 / 10) / 2` and not `100 / (10 / 2) = 20`.
* `3 + 5 * 2 = 13`, `(3 + 5) * 2 = 16` — precedence.

`^` is implemented by repeated multiplication rather than `exp(ln(x) * y)`,
because the library has no `pow` and the logarithm form is wrong for a negative
base and imprecise for an exact one.

Every failure is a value: `1 / 0`, `(1 + 2`, `1 +`, `` and `4 $ 2` all return
`Err` with a position. Nothing panics.

## Findings

None. This project found no defects — it was written and worked.
