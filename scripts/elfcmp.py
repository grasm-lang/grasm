#!/usr/bin/env python3
"""Compares two ELF executables part by part, for the Linux linker's gate
(`scripts/linkdiff_elf.sh`, docs/self-hosting.md §6.18): `compiler/link`'s
output and GNU ld's for the same inputs.

    elfcmp.py <ours> <ld's>     the ELF headers, the program headers, the
                                section headers and every section's bytes,
                                each difference named; exit 1 on any
    elfcmp.py --bytes <a> <b>   only whether the files are the same bytes,
                                and where they first differ
    elfcmp.py --glibc <exe>     the newest GLIBC_ version the executable's
                                `.gnu.version_r` requires (the oldest glibc
                                that can run it), or "none"
    elfcmp.py --floor <floor> <list> <exe>
                                `newest <version>` (as --glibc), then each
                                import bound to a GLIBC_ version newer than
                                <floor>: `listed <name> <version>` when
                                <list> (`scripts/glibc_floor.txt`) names it
                                with that version, `above <name> <version>`
                                when not

It reads the files itself, so that no tool's presentation decides.
"""
import struct
import sys

PT = {0: "NULL", 1: "LOAD", 2: "DYNAMIC", 3: "INTERP", 4: "NOTE", 6: "PHDR", 7: "TLS",
      0x6474e550: "GNU_EH_FRAME", 0x6474e551: "GNU_STACK", 0x6474e552: "GNU_RELRO", 0x6474e553: "GNU_PROPERTY"}


def parse(path):
    d = open(path, "rb").read()
    if d[:4] != b"\x7fELF":
        raise SystemExit(f"{path}: not an ELF file")
    hdr = struct.unpack_from("<HHIQQQIHHHHHH", d, 16)
    (_t, _m, _v, _entry, phoff, shoff, _f, _eh, phes, phnum, shes, shnum, shstrndx) = hdr
    phs = [struct.unpack_from("<IIQQQQQQ", d, phoff + phes * k) for k in range(phnum)]
    shs = [list(struct.unpack_from("<IIQQQQIIQQ", d, shoff + shes * k)) for k in range(shnum)]
    st = shs[shstrndx]
    for s in shs:
        s.append(d[st[4] + s[0]:d.index(b"\0", st[4] + s[0])].decode())
    return d, hdr, phs, shs


def glibc_floor(path):
    d, _hdr, _phs, shs = parse(path)
    best = None
    for s in shs:
        if s[1] != 0x6ffffffe:  # SHT_GNU_verneed
            continue
        strs = shs[s[6]]
        at = s[4]
        for _ in range(s[7]):
            _v, cnt, _file, aux, nxt = struct.unpack_from("<HHIII", d, at)
            a = at + aux
            for _ in range(cnt):
                _h, _f, _o, name, anext = struct.unpack_from("<IHHII", d, a)
                n = d[strs[4] + name:d.index(b"\0", strs[4] + name)].decode()
                if n.startswith("GLIBC_") and n[6:].replace(".", "").isdigit():
                    v = tuple(int(x) for x in n[6:].split("."))
                    if best is None or v > best:
                        best = v
                if anext == 0:
                    break
                a += anext
            if nxt == 0:
                break
            at += nxt
    return "none" if best is None else "GLIBC_" + ".".join(str(x) for x in best)


def imports(path):
    """Each imported symbol and the version it is bound to, `name GLIBC_x.y`
    (or `name -` unversioned), from `.dynsym`, `.gnu.version` and
    `.gnu.version_r`."""
    d, _hdr, _phs, shs = parse(path)
    names = {}
    for s in shs:
        if s[1] != 0x6ffffffe:  # SHT_GNU_verneed
            continue
        strs = shs[s[6]]
        at = s[4]
        for _ in range(s[7]):
            _v, cnt, _file, aux, nxt = struct.unpack_from("<HHIII", d, at)
            a = at + aux
            for _ in range(cnt):
                _h, _f, other, name, anext = struct.unpack_from("<IHHII", d, a)
                names[other] = d[strs[4] + name:d.index(b"\0", strs[4] + name)].decode()
                if anext == 0:
                    break
                a += anext
            if nxt == 0:
                break
            at += nxt
    dyn = next((s for s in shs if s[1] == 11), None)
    ver = next((s for s in shs if s[1] == 0x6fffffff), None)
    out = []
    if dyn is None:
        return out
    strs = shs[dyn[6]]
    for k in range(1, dyn[5] // 24):
        name, _info, _other, shndx, _value, _size = struct.unpack_from("<IBBHQQ", d, dyn[4] + 24 * k)
        if shndx != 0:
            continue
        n = d[strs[4] + name:d.index(b"\0", strs[4] + name)].decode()
        v = "-"
        if ver is not None:
            v = names.get(struct.unpack_from("<H", d, ver[4] + 2 * k)[0] & 0x7fff, "-")
        out.append((n, v))
    return out


def version(v):
    return tuple(int(x) for x in v.split("."))


def floor(limit, listfile, path):
    listed = set()
    for line in open(listfile):
        if line.strip() and not line.startswith("#"):
            name, ver = line.split("\t")[:2]
            listed.add((name, ver))
    print("newest", glibc_floor(path).replace("GLIBC_", ""))
    for name, v in imports(path):
        if not v.startswith("GLIBC_") or not v[6:].replace(".", "").isdigit():
            continue
        v = v[6:]
        if version(v) > version(limit):
            print("listed" if (name, v) in listed else "above", name, v)


def main():
    if len(sys.argv) == 5 and sys.argv[1] == "--floor":
        floor(sys.argv[2], sys.argv[3], sys.argv[4])
        return 0
    if len(sys.argv) == 3 and sys.argv[1] == "--glibc":
        print(glibc_floor(sys.argv[2]))
        return 0
    if len(sys.argv) == 4 and sys.argv[1] == "--bytes":
        a = open(sys.argv[2], "rb").read()
        b = open(sys.argv[3], "rb").read()
        if a == b:
            print("the same bytes")
            return 0
        n = min(len(a), len(b))
        first = next((i for i in range(n) if a[i] != b[i]), n)
        print(f"differ: sizes {len(a)} and {len(b)}, first difference at {first:#x}")
        return 1
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    a, ha, pa, sa = parse(sys.argv[1])
    b, hb, pb, sb = parse(sys.argv[2])
    problems = []
    names = ["type", "machine", "version", "entry", "phoff", "shoff", "flags", "ehsize", "phentsize", "phnum",
             "shentsize", "shnum", "shstrndx"]
    for i, n in enumerate(names):
        if ha[i] != hb[i]:
            problems.append(f"ELF header {n}: {ha[i]:#x} vs {hb[i]:#x}")
    for k in range(max(len(pa), len(pb))):
        x = pa[k] if k < len(pa) else None
        y = pb[k] if k < len(pb) else None
        if x != y:
            fmt = lambda p: "none" if p is None else f"{PT.get(p[0], hex(p[0]))} flags {p[1]} off {p[2]:#x} vaddr {p[3]:#x} filesz {p[5]:#x} memsz {p[6]:#x} align {p[7]:#x}"
            problems.append(f"program header {k}: {fmt(x)} vs {fmt(y)}")
    for k in range(max(len(sa), len(sb))):
        x = sa[k] if k < len(sa) else None
        y = sb[k] if k < len(sb) else None
        if x is None or y is None:
            problems.append(f"section {k}: {x and x[10]} vs {y and y[10]}")
            continue
        fields = ["name", "type", "flags", "addr", "offset", "size", "link", "info", "align", "entsize"]
        for i, f in enumerate(fields):
            if i == 0:
                if x[10] != y[10]:
                    problems.append(f"section {k}: named {x[10]} vs {y[10]}")
                continue
            if x[i] != y[i]:
                problems.append(f"section {k} {x[10]}: {f} {x[i]:#x} vs {y[i]:#x}")
        if x[1] != 8 and y[1] != 8:
            ca = a[x[4]:x[4] + x[5]]
            cb = b[y[4]:y[4] + y[5]]
            if ca != cb:
                n = min(len(ca), len(cb))
                first = next((i for i in range(n) if ca[i] != cb[i]), n)
                problems.append(f"section {k} {x[10]}: contents differ, first at +{first:#x} "
                                f"({ca[first:first + 8].hex()} vs {cb[first:first + 8].hex()})")
    for p in problems[:40]:
        print(p)
    if a == b:
        print("the same bytes")
    print(f"differences: {len(problems)}")
    return 1 if problems or a != b else 0


if __name__ == "__main__":
    sys.exit(main())
