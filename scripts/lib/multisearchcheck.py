#!/usr/bin/env python3
"""Checks `packages/multisearch` against brute force.

    python3 scripts/lib/multisearchcheck.py < lines

Two line shapes are read:

    <name> <pattern>... | <text> | <idx>:<start>:<end>...
    <name> <idx>:<start>:<end>...        (with the case in NAMED below)

The oracle is the obvious quadratic search: for every end offset, for every
pattern, does the text end with it there. That is the right oracle for an
automaton -- it shares no structure with the thing under test, so a failure
link built wrong cannot make both agree.

Order is normalised before comparing. The automaton reports matches as the
single pass produces them, which is by end offset and shortest-first within
an offset; a caller is not promised more than the set.
"""

import sys

NAMED = {
    "classic": ([b"he", b"she", b"his", b"hers"], b"ushers"),
    "nested": ([b"ab", b"abc", b"bc", b"c"], b"xabcy"),
    "suffixes": ([b"a", b"aa", b"aaa"], b"aaaa"),
    "absent": ([b"zz", b"qq"], b"abcdef"),
    "empty_text": ([b"a"], b""),
    "longer_than_text": ([b"abcdefgh"], b"abc"),
    "one_byte": ([b"x"], b"xxx"),
    "whole": ([b"abc"], b"abc"),
    "repeat": ([b"ana"], b"bananana"),
    "high_bytes": ([bytes([200, 201]), bytes([201, 202])],
                   bytes([199, 200, 201, 202, 200, 201])),
    "many": ([b"the", b"he", b"e", b"there", b"her"], b"there is the heather"),
    "duplicate": ([b"ab", b"ab"], b"abab"),
}


def brute(pats, text):
    out = []
    for end in range(1, len(text) + 1):
        for i, p in enumerate(pats):
            if p and end >= len(p) and text[end - len(p):end] == p:
                out.append((i, end - len(p), end))
    return key_sort(out)


def key_sort(ms):
    return sorted(ms, key=lambda m: (m[2], m[2] - m[1], m[0]))


def parse_matches(fields):
    return [tuple(int(x) for x in f.split(":")) for f in fields]


def main():
    bad = 0
    named = 0
    fuzzed = 0
    for line in sys.stdin:
        if line.startswith("nodes"):
            continue
        if "|" in line:
            head, mid, tail = line.split("|")
            pats = [p.encode() for p in head.split()[1:]]
            text = mid.strip().encode()
            got = parse_matches(tail.split())
            label = head.split()[0] if head.split() else "?"
            fuzzed += 1
        else:
            fields = line.split()
            if not fields or fields[0] not in NAMED:
                continue
            label = fields[0]
            pats, text = NAMED[label]
            got = parse_matches(fields[1:])
            named += 1
        want = brute(pats, text)
        if key_sort(got) != want:
            bad += 1
            if bad <= 4:
                print(f"    {label}: patterns {pats} text {text!r}")
                print(f"      got  {key_sort(got)}")
                print(f"      want {want}")
    if named < len(NAMED):
        print(f"    only {named} of {len(NAMED)} named cases were printed")
        return 1
    if fuzzed < 100:
        print(f"    only {fuzzed} fuzz cases ran; the generator printed too few")
        return 1
    print(f"    cases: {named} named and {fuzzed} random, all agreeing with "
          f"brute force")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
