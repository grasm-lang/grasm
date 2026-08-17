# 10 — Basic Text Grep Utility

**Tests:** substring search implemented by hand, line and column indexing,
boundary conditions, multi-byte columns.

**Result:** works. All three engines agree.

## What it actually exercises

`.contains()` exists, so calling it would test nothing. This implements the
search twice — naive and Knuth-Morris-Pratt — and runs **both on every line of
every query**, reporting any disagreement. Three implementations that must agree
(counting the library's `index_of`) is a far better test than one that must
work. They never disagreed.

KMP earns its place on `aaab` in `aaaaaaaaab`, the shape that makes a naive
matcher backtrack repeatedly. Both find it at 6.

Six boundary cases are checked explicitly, because they are where search
implementations are actually wrong: empty needle, needle longer than haystack,
needle equal to haystack, empty haystack, both empty, and a match at the very
end.

`find_all` finds *every* occurrence, not just the first — `aa` in `aaaa` gives
`[0, 1, 2]`, which is where an off-by-one in the restart position shows up.

The multi-byte block matters: `café` in `naïve café` is at code-point index 6,
and the library's `index_of` agrees. A byte-wise grep would report 8 and print
the wrong column.

## Findings

**F5** — a string literal cannot span lines and there is no raw form. Found
here; the fixture had to be written on one line with `\n` escapes.
