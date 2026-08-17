# 05 — Simple Matrix Math Tool

**Tests:** nested lists (`[[f64]]`), two-level indexing, loop nesting, shape
errors as values.

**Result:** works. All three engines agree.

## What it actually exercises

Two-dimensional data is a list of lists, so this tests whether nesting allocates
correctly and whether indexing two levels deep is bounds-checked.

Every operation returns `T ! Shape` rather than panicking, because "add a 2×3 to
a 3×2" is a caller's mistake and not a bug in the matrix code — and a language
with typed error unions should make that distinction cost nothing. It does.

Three properties are checked rather than three examples:

* `A × I = A` and `I × A = A`. A swapped `i`/`j` in the multiply loop gives the
  transpose instead, and identity is what catches it.
* A ragged list is a legal Grasm value and is not a legal matrix, so it is
  rejected once at the boundary rather than assumed everywhere.
* An empty matrix is rectangular and must not divide by zero anywhere —
  `identity(0)` and `add([], [])` are both exercised.

## Findings

None. Bounds and shapes behaved.
