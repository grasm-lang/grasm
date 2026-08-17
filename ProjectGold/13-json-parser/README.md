# 13 — Custom JSON Parser
**Tests:** recursive descent, a recursive heterogeneous value type, round trips. **Result:** works, all three engines agree.

The project that most tests the type system, because JSON's value is genuinely
recursive and genuinely heterogeneous. In Grasm that is a sum type whose
variants hold `ref Json`, and getting it to typecheck is most of the exercise.

Correctness is checked by round trip, not by eye: parse, re-serialise, parse
again, compare the two trees **structurally** as well as textually. Both hold
for every valid input, including `Aé` → `Aé`.

Twelve malformed inputs — trailing comma, unclosed container, unquoted key,
missing colon, trailing junk, bare word, empty input, lone minus, unterminated
string, bad escape, bad hex — are each an error with a position. None panics.

## Findings
None in the language. Confirmed that `"{"` needs writing as `"\{"`, with a
diagnostic that says exactly that.
