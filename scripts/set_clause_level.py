#!/usr/bin/env python3
"""Sets a clause's stability level and refiles spec/stability.md.

    scripts/set_clause_level.py T-MATCH-ALT provisional

The file lists every clause twice over — once by the heading it sits under,
once by the word after the dash — and editing it by hand means keeping those
in step by hand. Three separate times a clause was added under the wrong
heading, and each time the gate caught it after the fact rather than the edit
being right in the first place. This rewrites the lists from the levels, so
the two cannot disagree.
"""

import re
import sys
from pathlib import Path

LEVELS = ("stable", "provisional", "experimental")
PATH = Path(__file__).resolve().parent.parent / "spec" / "stability.md"


def read(text):
    """Every clause and its level, regardless of where it currently sits."""
    out = {}
    for m in re.finditer(r"- `([A-Z0-9-]+)` — (stable|provisional|experimental)\s*$", text, re.M):
        out[m.group(1)] = m.group(2)
    return out


def write(text, levels):
    by = {lv: sorted(c for c, l in levels.items() if l == lv) for lv in LEVELS}

    def block(level):
        return "\n".join(f"- `{c}` — {level}" for c in by[level]) + "\n"

    # Each section is its heading and prose, then a list this replaces whole.
    for level, anchor in (
        ("stable", r"(## Stable\n\n(?:(?!\n- `)[^\n]*\n)*\n)(?:- `[^\n]*\n)+"),
        ("provisional", r"(## Provisional\n\n(?:(?!\n- `)[^\n]*\n)*\n)(?:- `[^\n]*\n)+"),
        ("experimental", r"(## Experimental\n\n(?:(?!\n- `)[^\n]*\n)*\n)(?:- `[^\n]*\n)+"),
    ):
        new, n = re.subn(anchor, lambda m, lv=level: m.group(1) + block(lv), text)
        if n != 1:
            raise SystemExit(f"could not find the {level} list to replace")
        text = new

    total = sum(len(v) for v in by.values())
    counts = f"Today: {len(by['stable'])} stable, {len(by['provisional'])} provisional, {len(by['experimental'])} experimental, of {total}."
    text = re.sub(r"Today: \d+ stable, \d+ provisional, \d+ experimental, of \d+\.", counts, text)
    return text, by


def main():
    if len(sys.argv) == 3:
        clause, level = sys.argv[1], sys.argv[2]
        if level not in LEVELS:
            raise SystemExit(f"level must be one of {', '.join(LEVELS)}")
    elif len(sys.argv) == 1:
        clause = level = None  # refile only
    else:
        raise SystemExit("usage: set_clause_level.py [CLAUSE LEVEL]")

    text = PATH.read_text()
    levels = read(text)
    if clause:
        levels[clause] = level
    text, by = write(text, levels)
    PATH.write_text(text)
    if clause:
        print(f"{clause} is now {level}")
    print(f"{len(by['stable'])} stable, {len(by['provisional'])} provisional, {len(by['experimental'])} experimental")


if __name__ == "__main__":
    main()
