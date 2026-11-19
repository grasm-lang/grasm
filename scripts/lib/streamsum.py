#!/usr/bin/env python3
"""Prints the length and SHA-256 of standard input, read as it arrives.

A debug-adapter session can answer with gigabytes (`sieve`'s million-element
list shown at each of 200 stops), so `scripts/clidiff.sh` compares the two
streams by this and never stores them (docs/self-hosting.md §6.22). With an
argument, also keeps the first that many bytes in `$KEEP` for a diff.
"""
import hashlib
import os
import sys

h = hashlib.sha256()
n = 0
keep = int(sys.argv[1]) if len(sys.argv) > 1 else 0
kept = bytearray()
f = sys.stdin.buffer
while True:
    b = f.read(1 << 20)
    if not b:
        break
    h.update(b)
    n += len(b)
    if len(kept) < keep:
        kept += b[:keep - len(kept)]
if keep and os.environ.get("KEEP"):
    with open(os.environ["KEEP"], "wb") as out:
        out.write(bytes(kept))
print(n, h.hexdigest())
