"""The code points Python's `\\w` matches in a str pattern, as ranges, in
the form packages/pyentrypoints declares them (`let PY_WORD = [...]`), so
the gate can tell when Python's Unicode tables have moved on."""
import re
import sys

w = re.compile(r"\w")
flat = []
start = None
for c in range(sys.maxunicode + 1):
    ok = w.match(chr(c)) is not None
    if ok and start is None:
        start = c
    if not ok and start is not None:
        flat += [start, c - 1]
        start = None
if start is not None:
    flat += [start, sys.maxunicode]
lines = []
row = []
for x in flat:
    row.append(str(x))
    if len(row) == 16:
        lines.append("    " + ", ".join(row) + ",")
        row = []
if row:
    lines.append("    " + ", ".join(row) + ",")
print("let PY_WORD = [\n" + "\n".join(lines) + "\n]")
