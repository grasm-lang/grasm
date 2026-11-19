#!/usr/bin/env python3
"""Journals made from a recording, for `scripts/clidiff.sh`'s `journal` section.

    jmangle.py <recorded.jrnl> <dir>

Writes into <dir>, from a journal in the `GRASMJRNL2` format
(`crates/grasm_journal`; docs/replay.md), one file per way a replay or `grasm
journal` must answer (docs/self-hosting.md §6.22):

  short.jrnl     the last record gone: a replay runs out at that call
  renamed.jrnl   the first record named `zz_renamed`: a replay diverges at call 1
  extra.jrnl     one record more at the end: a replay ends with it unread
                 and says nothing (docs/known-issues.md)
  cut.jrnl       cut in the middle of the last record: refused
  magic.jrnl     the magic broken: not a journal
  long.jrnl      every text and byte string grown past the 48-character
                 budget `grasm journal` prints within, and a list of five,
                 nested values, a float and a name that is not ASCII appended

Prints the names written. A journal with no record yields only the ones
that make sense for it.
"""
import os
import struct
import sys

MAGIC = b"GRASMJRNL2"


def u32(n):
    return struct.pack("<I", n)


def records(raw):
    assert raw[:10] == MAGIC
    at = 10
    out = []
    while at < len(raw):
        n = struct.unpack("<I", raw[at:at + 4])[0]
        name = raw[at + 4:at + 4 + n]
        at += 4 + n
        m = struct.unpack("<I", raw[at:at + 4])[0]
        out.append((name, raw[at + 4:at + 4 + m]))
        at += 4 + m
    return out


def frame(recs):
    return MAGIC + b"".join(u32(len(n)) + n + u32(len(v)) + v for n, v in recs)


def grow(v, at=0):
    """The value with each text and byte string made long, re-encoded."""
    tag = v[at]
    if tag == 0:
        return b"\x00", at + 1
    if tag in (1, 2):
        return v[at:at + 9], at + 9
    if tag == 3:
        return v[at:at + 2], at + 2
    if tag in (4, 5):
        n = struct.unpack("<I", v[at + 1:at + 5])[0]
        body = v[at + 5:at + 5 + n]
        filler = ("\n\t\"é" * 40).encode() if tag == 4 else bytes(range(200))
        new = body + filler
        return bytes([tag]) + u32(len(new)) + new, at + 5 + n
    if tag in (6, 7):
        n = struct.unpack("<I", v[at + 1:at + 5])[0]
        p = at + 5
        items = []
        for _ in range(n):
            x, p = grow(v, p)
            items.append(x)
        return bytes([tag]) + u32(n) + b"".join(items), p
    if tag == 8:
        p = at + 1
        parts = []
        for _ in range(2):
            n = struct.unpack("<I", v[p:p + 4])[0]
            parts.append(v[p:p + 4 + n])
            p += 4 + n
        n = struct.unpack("<I", v[p:p + 4])[0]
        p += 4
        items = []
        for _ in range(n):
            x, p = grow(v, p)
            items.append(x)
        return b"\x08" + b"".join(parts) + u32(n) + b"".join(items), p
    raise ValueError("unknown tag %d" % tag)


def text(s):
    b = s.encode()
    return b"\x04" + u32(len(b)) + b


def main():
    src, d = sys.argv[1], sys.argv[2]
    raw = open(src, "rb").read()
    recs = records(raw)
    os.makedirs(d, exist_ok=True)
    out = {}
    if recs:
        out["short"] = frame(recs[:-1])
        out["renamed"] = frame([(b"zz_renamed", recs[0][1])] + recs[1:])
        out["extra"] = frame(recs + [recs[-1]])
        whole = frame(recs)
        out["cut"] = whole[:len(whole) - max(1, len(recs[-1][1]) // 2)]
    out["magic"] = b"GRASMJRNL1" + raw[10:]
    extra = [
        (b"list_dir", b"\x08" + u32(1) + b"!" + u32(2) + b"Ok" + u32(1) + b"\x06" + u32(5) + b"".join(text("f%d" % i) for i in range(5))),
        (b"now_millis", b"\x02" + struct.pack("<d", -0.5)),
        (b"now_millis", b"\x02" + struct.pack("<d", 1e21)),
        ("名前".encode(), b"\x07" + u32(2) + b"\x03\x01" + b"\x00"),
        (b"env", b"\x08" + u32(6) + b"Option" + u32(4) + b"None" + u32(0)),
    ]
    out["long"] = frame([(n, grow(v)[0]) for n, v in recs] + extra)
    for k in sorted(out):
        with open(os.path.join(d, k + ".jrnl"), "wb") as f:
            f.write(out[k])
        print(k)


main()
