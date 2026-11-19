#!/usr/bin/env python3
"""Whether two journals record the same run (`scripts/clidiff.sh`, `journal`).

    jnorm.py <a.jrnl> <b.jrnl>

Exit 0 when they are the same bytes once the values of the built-ins whose
answer depends on when and where the run happened -- the clock, entropy,
ports and peers: the only ones that differed between two recordings of the
corpus by the Rust `grasm` (docs/self-hosting.md §6.22) -- are set aside;
2 when what differs is an `IoFailed` message, which the engines word
differently (docs/known-issues.md, S1); 1 otherwise, saying where.
"""
import struct
import sys

VARY = {b"now_millis", b"monotonic_millis", b"monotonic_nanos", b"random_bytes", b"net_port", b"tcp_peer"}


def records(raw):
    if raw[:10] != b"GRASMJRNL2":
        return None
    at, out = 10, []
    while at < len(raw):
        n = struct.unpack("<I", raw[at:at + 4])[0]
        name = raw[at + 4:at + 4 + n]
        at += 4 + n
        m = struct.unpack("<I", raw[at:at + 4])[0]
        out.append((name, raw[at + 4:at + 4 + m]))
        at += 4 + m
    return out


a = records(open(sys.argv[1], "rb").read())
b = records(open(sys.argv[2], "rb").read())
if a is None or b is None:
    print("not a journal")
    sys.exit(1)
if [x[0] for x in a] != [x[0] for x in b]:
    print("the calls differ: %d and %d records" % (len(a), len(b)))
    sys.exit(1)
for i, (x, y) in enumerate(zip(a, b)):
    if x[1] != y[1] and x[0] not in VARY:
        if b"IoFailed" in x[1] or b"IoFailed" in y[1]:
            print("record %d (%s): an IoFailed message" % (i + 1, x[0].decode(errors="replace")))
            sys.exit(2)
        print("record %d (%s) differs" % (i + 1, x[0].decode(errors="replace")))
        sys.exit(1)
sys.exit(0)
