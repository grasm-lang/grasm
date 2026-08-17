# 03 — FizzBuzz Variant

**Tests:** modulo, boolean composition, string accumulation, iteration over a
list of structs.

**Result:** works. All three engines agree.

## What it actually exercises

Plain FizzBuzz is four lines and proves nothing, so the rules are *data*: a
`[Rule]` of divisor-and-word pairs that the loop composes a string from. That
turns a fixed if-chain into iteration over structs, and it is the version where
"did anything match at all" can be got wrong.

The 1..100 run is summarised rather than printed, because a hundred lines of
output hides a wrong answer instead of showing one. The counts are checked by
arithmetic that has to add to 100: 27 Fizz + 14 Buzz + 6 FizzBuzz + 53 plain.

Three rules is where the variant earns its place — 105 is divisible by 3, 5 and
7, so it must name all three *in rule order*. And two degenerate rule sets are
included: one that never matches, one that always does.

## Findings

None.
