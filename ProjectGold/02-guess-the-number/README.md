# 02 — Guess the Number

**Tests:** pseudo-random generation, `input()` loops, `parse_int` on bad input,
and `[E-ARITH-INT]` — integer overflow.

**Result:** works. All three engines agree.

## What it actually exercises

There is no `rand` in the library, which makes this the more interesting version
of the exercise: the generator is written in Grasm, and a linear congruential
generator is exactly the shape that depends on what the language does when an
integer runs off the end.

`[E-ARITH-INT]` says `+ - *` on `i64` wrap, two's complement. So the textbook
Numerical Recipes constants work as written — and this program doubles as a test
of that clause, since an LCG overflows on nearly every step and must produce the
*same sequence* under all three engines or one of them is not wrapping the way
the specification says. It does.

The low bits of an LCG are its worst ones, so the top 31 are taken. With no
shift operator (FINDINGS.md F3) that is a division by `4294967296`, which is the
same thing for a non-negative value — and the value is made non-negative first.

`input()` returning `""` at end of stream is what ends the loop, and a
non-numeric line is handled through `parse_int() -> Option[i64]` rather than by
crashing.

## Findings

None new. Confirmed `[E-ARITH-INT]` holds identically across three engines.
