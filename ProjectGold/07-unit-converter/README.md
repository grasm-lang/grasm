# 07 — Unit Converter CLI

**Tests:** floating-point precision, constants, a table-driven design.

**Result:** works. All three engines agree.

## What it actually exercises

Printing `100 C = 212 F` proves the multiplier was typed correctly and nothing
else. So the precision test is a **round trip**: convert there and back, and
report the drift. All six round trips are exact to ten places.

Every unit in a family is `base = value * to_base + offset`, so one pair of
functions converts anything to anything — including temperature, where the
offset is what makes Fahrenheit awkward. `-40 C = -40 F` is the check that the
offset is right, since it is the one temperature where the two scales meet.

The last block prints `0.1 + 0.2 = 0.30000000000000004` and that
`0.1 + 0.2 == 0.3` is `false`. That is a fact about binary floats and a language
that hides it is lying about its own arithmetic. Grasm prints the true value.

## Findings

None.
