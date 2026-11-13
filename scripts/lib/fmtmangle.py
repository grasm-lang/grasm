#!/usr/bin/env python3
"""Rewrites of a Grasm source file for holding two formatters to each other.

    fmtmangle.py <file.gr> <outdir>

writes four rewrites of the file into <outdir>, each deterministic:

    dedent.gr   every line's leading whitespace removed
    spaced.gr   every run of spaces doubled
    tight.gr    every space beside ASCII punctuation removed
    crlf.gr     dedent.gr with every newline written as CR LF

The corpus is canonically formatted (`fmttest`), so formatting it as it is
tests only "already formatted". These give `grasm fmt` work to do -- the
indentation, the spacing, the CR before a newline -- and whether a rewrite
still parses does not matter: a file that does not is a refusal, and the two
commands must refuse it alike (docs/self-hosting.md §6.19).
"""
import os
import re
import sys

PUNCT = set(b"!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~")


def dedent(src):
    return b"\n".join(line.lstrip(b" \t") for line in src.split(b"\n"))


def spaced(src):
    return re.sub(rb" +", lambda m: m.group(0) * 2, src)


def tight(src):
    out = bytearray()
    for i, c in enumerate(src):
        if c == 32:
            before = src[i - 1] if i > 0 else None
            after = src[i + 1] if i + 1 < len(src) else None
            if before in PUNCT or after in PUNCT:
                continue
        out.append(c)
    return bytes(out)


def main():
    src = open(sys.argv[1], "rb").read()
    outdir = sys.argv[2]
    os.makedirs(outdir, exist_ok=True)
    d = dedent(src)
    for name, text in (
        ("dedent.gr", d),
        ("spaced.gr", spaced(src)),
        ("tight.gr", tight(src)),
        ("crlf.gr", d.replace(b"\n", b"\r\n")),
    ):
        with open(os.path.join(outdir, name), "wb") as f:
            f.write(text)


if __name__ == "__main__":
    main()
