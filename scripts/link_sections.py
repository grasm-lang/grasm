#!/usr/bin/env python3
"""Two arm64 executables, section by section and blob by blob.

    scripts/link_sections.py <ld's> <ours> [--symbols]

For `scripts/linkdiff.sh`: every section's contents (zero-fill excepted) and
the __LINKEDIT blobs -- chained fixups, export trie, function starts, data in
code -- must be the same bytes, sections in the same order at the same
addresses. With `--symbols`, the symbol table, the indirect symbol table and
the string table too; without, they are left out, for an image whose objects
carry DWARF, where ld writes a debug map (N_OSO stabs) and compiler/link does
not (docs/self-hosting.md §6.6). Load commands are not compared here, and
neither is the code signature, which hashes them.

Prints one line per section or blob and a count; exits 1 on any difference.
"""
import struct
import sys


def parse(path):
    d = open(path, "rb").read()
    ncmds = struct.unpack_from("<I", d, 16)[0]
    p = 32
    secs = []
    blobs = {}
    names = {0x80000034: "chained fixups", 0x80000033: "export trie", 0x26: "function starts",
             0x29: "data in code"}
    for _ in range(ncmds):
        cmd, cs = struct.unpack_from("<II", d, p)
        if cmd == 0x19:
            n = struct.unpack_from("<I", d, p + 64)[0]
            for k in range(n):
                s = p + 72 + 80 * k
                name = d[s:s + 16].rstrip(b"\0").decode()
                seg = d[s + 16:s + 32].rstrip(b"\0").decode()
                addr, size, off = struct.unpack_from("<QQI", d, s + 32)
                flags = struct.unpack_from("<I", d, s + 64)[0]
                secs.append((seg, name, addr, size, off, flags))
        elif cmd in names:
            off, size = struct.unpack_from("<II", d, p + 8)
            blobs[names[cmd]] = (off, size)
        elif cmd == 2:
            symoff, nsyms, stroff, strsize = struct.unpack_from("<IIII", d, p + 8)
            blobs["symbol table"] = (symoff, 16 * nsyms)
            blobs["string table"] = (stroff, strsize)
        elif cmd == 0xb:
            v = struct.unpack_from("<18I", d, p + 8)
            blobs["indirect symbols"] = (v[12], 4 * v[13])
        p += cs
    return d, secs, blobs


def main():
    a, sa, ba = parse(sys.argv[1])
    b, sb, bb = parse(sys.argv[2])
    symbols = "--symbols" in sys.argv
    bad = 0
    if [(s[0], s[1], s[2], s[3]) for s in sa] != [(s[0], s[1], s[2], s[3]) for s in sb]:
        print("sections differ in name, order, address or size:")
        for x, y in zip(sa, sb):
            print("  ld %s,%s %#x+%#x   ours %s,%s %#x+%#x" % (x[0], x[1], x[2], x[3], y[0], y[1], y[2], y[3]))
        sys.exit(1)
    for x, y in zip(sa, sb):
        if x[5] & 0xFF in (1, 0x0C, 0x12):
            continue
        ca = a[x[4]:x[4] + x[3]]
        cb = b[y[4]:y[4] + y[3]]
        if ca == cb:
            print("%s,%s: same %d bytes" % (x[0], x[1], x[3]))
        else:
            bad += 1
            first = next(i for i in range(min(len(ca), len(cb))) if ca[i] != cb[i])
            print("%s,%s: DIFFERS from byte %d (address %#x)" % (x[0], x[1], first, x[2] + first))
    want = ["chained fixups", "export trie", "function starts", "data in code"]
    if symbols:
        want += ["symbol table", "indirect symbols", "string table"]
    for k in want:
        if k not in ba or k not in bb:
            bad += 1
            print("%s: missing in %s" % (k, "ld's" if k not in ba else "ours"))
            continue
        x = a[ba[k][0]:ba[k][0] + ba[k][1]]
        y = b[bb[k][0]:bb[k][0] + bb[k][1]]
        if x == y:
            print("%s: same %d bytes" % (k, len(x)))
        else:
            bad += 1
            print("%s: DIFFERS (%d bytes, ours %d)" % (k, len(x), len(y)))
    print("sections and blobs differing: %d" % bad)
    sys.exit(1 if bad else 0)


main()
