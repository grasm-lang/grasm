#!/usr/bin/env python3
"""`grasm bench`'s output with the two numbers no one can reproduce taken out.

    benchnorm.py <out>

Writes the output to stdout with each timing line's time and iteration count
replaced by `T`, the name and its padding kept, so two commands' outputs
compare byte for byte on everything else (docs/self-hosting.md §6.19). A
timing line must be the format exactly -- the time right-aligned in 12
characters, one decimal under 1,000 ns and two above, the unit its size
says; the count a positive multiple of 16 no larger than 20,000,000 -- or
this exits 1 naming it.
"""
import re
import sys

LINE = re.compile(r"^(.*)  \(([0-9]+) iterations\)$")
TIME = re.compile(r"^([0-9]+)\.([0-9]+) (ns|µs|ms)/op$")


def main():
    text = open(sys.argv[1], encoding="utf-8").read()
    out = []
    bad = []
    for line in text.split("\n"):
        m = LINE.match(line)
        if not m:
            out.append(line)
            continue
        rest, iters = m.groups()
        # The time is right-aligned in 12 characters, or wider when it does
        # not fit; before it, the padded name and two spaces.
        word = rest.split(" ")
        unit_at = len(rest) - len(word[-1])
        num_start = rest.rfind(" ", 0, unit_at - 1) + 1
        time = rest[num_start:]
        field_start = min(num_start, len(rest) - 12) if len(time) <= 12 else num_start
        field = rest[field_start:]
        head = rest[:field_start]
        t = TIME.match(time)
        ok = t is not None and head.endswith("  ") and field.strip() == time
        if ok:
            unit = t.group(3)
            value = float(t.group(1) + "." + t.group(2))
            ok = len(t.group(2)) == (1 if unit == "ns" else 2)
            ok = ok and (unit == "ms" or value < 1000.0)
        n = int(iters)
        ok = ok and n > 0 and n % 16 == 0 and n <= 20000000
        if not ok:
            bad.append(line)
        out.append(head + "T")
    sys.stdout.write("\n".join(out))
    if bad:
        for line in bad:
            print(f"malformed timing line: {line!r}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
