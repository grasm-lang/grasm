"""The ranges of code points Python 3.13's str.isprintable() refuses above
U+007F, written as the Grasm function packages/pyurlparse carries. With
--print it prints the function; b49_diff.py checks the package holds
exactly this text, so a newer Python fails the gate instead of drifting."""
import sys

ranges, start = [], None
for c in range(0x80, 0x110000):
    refused = not chr(c).isprintable()
    if refused and start is None:
        start = c
    if not refused and start is not None:
        ranges.append((start, c - 1))
        start = None
if start is not None:
    ranges.append((start, 0x10FFFF))
lines = ["/// Starts and ends of the ranges above U+007F that Python 3.13's",
         "/// `str.isprintable` refuses (Unicode 15.1), from scripts/lib/b49/pyurl/gen_printable.py.",
         "fn unprintable_ranges() -> [i64] {", "    return ["]
flat = [str(v) for r in ranges for v in r]
for k in range(0, len(flat), 12):
    lines.append("        " + ", ".join(flat[k:k + 12]) + ("," if k + 12 < len(flat) else ""))
lines += ["    ]", "}"]
print("\n".join(lines))
