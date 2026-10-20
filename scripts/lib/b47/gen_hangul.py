"""The compatibility and halfwidth jamo table packages/hangul carries, from
Python 3.13's unicodedata (NFKD). Prints the table line; with a path, writes
it into the package."""
import sys
import unicodedata as u
pairs = []
for c in list(range(0x3131, 0x318F)) + list(range(0xFFA0, 0xFFDD)):
    d = u.normalize("NFKD", chr(c))
    if d != chr(c):
        assert len(d) == 1, hex(c)
        pairs.append(f"(0x{c:04x}, 0x{ord(d):04x})")
line = "    return [" + ", ".join(pairs) + "]"
if len(sys.argv) == 1:
    print(line)
    sys.exit(0)
p = sys.argv[1]
s = open(p).read()
a = s.index("    // BEGIN COMPAT")
b = s.index("    // END COMPAT")
s = s[:a] + "    // BEGIN COMPAT (scripts/lib/b47/gen_hangul.py, from Python 3.13's NFKD)\n" + line + "\n" + s[b:]
open(p, "w").write(s)
