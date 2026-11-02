#!/usr/bin/env python3
"""A small reader of x86-64 Mach-O relocatable objects, for the S7 gates.

    x64_macho.py compare <obj> <cases.tsv>   encodetest_x64: the object's bytes
                                             against the encoder's, case by
                                             case, and each relocation where
                                             the encoder says it must be

`cases.tsv` is what `s3 --encode-test <p> --target x86_64` writes: per line
the form, the encoder's bytes in hex, the assembly the assembler was given,
and the offset in the instruction and the type of the relocation the case
needs (-1 for none). An x86-64 instruction is 1 to 15 bytes, so the cases are
walked in order, each taking as many bytes of `__text` as the encoder made
for it. A case whose length is not the assembler's puts every later case out
of step, so a difference in the total length fails the comparison on its own,
and the first case reported as different is where to look.

It reads the file itself rather than parsing `otool` output, as
`scripts/s3_macho.py` does for arm64 objects.
"""
import struct
import sys

LC_SEGMENT_64 = 0x19
LC_SYMTAB = 0x2
CPU_TYPE_X86_64 = 0x01000007

# <mach-o/x86_64/reloc.h>
RELOC_NAMES = {0: "UNSIGNED", 1: "SIGNED", 2: "BRANCH", 3: "GOT_LOAD", 4: "GOT",
               5: "SUBTRACTOR", 6: "SIGNED_1", 7: "SIGNED_2", 8: "SIGNED_4", 9: "TLV"}


def sections(data):
    magic, cpu, _sub, ftype, ncmds, _size, _flags, _res = struct.unpack_from("<IiiIIIII", data, 0)
    if magic != 0xFEEDFACF:
        raise SystemExit("not a 64-bit Mach-O file")
    if cpu != CPU_TYPE_X86_64:
        raise SystemExit(f"cputype {cpu:#x} is not x86_64")
    if ftype != 1:
        raise SystemExit(f"filetype {ftype} is not MH_OBJECT")
    off = 32
    out = {}
    symtab = None
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", data, off)
        if cmd == LC_SEGMENT_64:
            nsects = struct.unpack_from("<I", data, off + 64)[0]
            s = off + 72
            for _k in range(nsects):
                name = data[s:s + 16].rstrip(b"\0").decode()
                seg = data[s + 16:s + 32].rstrip(b"\0").decode()
                addr, sz, foff, align, reloff, nreloc, flags = struct.unpack_from("<QQIIIII", data, s + 32)
                out[(seg, name)] = dict(addr=addr, size=sz, off=foff, reloff=reloff, nreloc=nreloc, flags=flags)
                s += 80
        elif cmd == LC_SYMTAB:
            symtab = struct.unpack_from("<IIII", data, off + 8)
        off += size
    return out, symtab


def symbols(data):
    _, symtab = sections(data)
    if symtab is None:
        return []
    symoff, nsyms, stroff, _strsize = symtab
    out = []
    for k in range(nsyms):
        strx, ntype, nsect, _desc, value = struct.unpack_from("<IBBHQ", data, symoff + 16 * k)
        end = data.index(b"\0", stroff + strx)
        out.append((data[stroff + strx:end].decode(), ntype, nsect, value))
    return out


def text(data):
    secs, _ = sections(data)
    t = secs.get(("__TEXT", "__text"))
    if t is None:
        raise SystemExit("no __text section")
    return data[t["off"]:t["off"] + t["size"]], t


def relocations(data, sec):
    """Each relocation of a section: (address, type, pcrel, length, extern, symbol number)."""
    out = []
    for k in range(sec["nreloc"]):
        addr, info = struct.unpack_from("<iI", data, sec["reloff"] + 8 * k)
        out.append((addr, info >> 28, (info >> 24) & 1, (info >> 25) & 3, (info >> 27) & 1, info & 0xFFFFFF))
    return out


def cmd_compare(path, tsv):
    data = open(path, "rb").read()
    code, t = text(data)
    syms = symbols(data)
    cases = [line.rstrip("\n").split("\t") for line in open(tsv)]
    total = sum(len(c[1]) // 2 for c in cases)
    if total != len(code):
        print(f"LENGTH the assembler made {len(code)} bytes of {len(cases)} lines; the encoder made {total}")
    rels = {}
    problems = []
    for addr, rtype, pcrel, length, ext, sym in relocations(data, t):
        if addr in rels:
            problems.append(f"two relocations at {addr:#x}")
        rels[addr] = (rtype, pcrel, length, ext, sym)
    per_form = {}
    bad = 0
    at = 0
    want_rels = set()
    for form, want, asm, roff, rkind in cases:
        n = len(want) // 2
        got = code[at:at + n].hex()
        ok = got == want
        roff, rkind = int(roff), int(rkind)
        if rkind >= 0:
            want_rels.add(at + roff)
            r = rels.get(at + roff)
            if r is None:
                ok = False
                problems.append(f"RELOC {form}: `{asm}` has no relocation at its byte {roff}")
            else:
                rtype, pcrel, length, ext, sym = r
                name = syms[sym][0] if ext and sym < len(syms) else "?"
                if rtype != rkind or pcrel != 1 or length != 2 or not ext or name not in asm:
                    ok = False
                    problems.append(f"RELOC {form}: `{asm}` assembler {RELOC_NAMES.get(rtype, rtype)} pcrel {pcrel} "
                                    f"length {length} against {name}, encoder {RELOC_NAMES.get(rkind, rkind)}")
        n_ok, n_all = per_form.get(form, (0, 0))
        per_form[form] = (n_ok + (1 if ok else 0), n_all + 1)
        if not ok:
            bad += 1
            if got != want and bad <= 20:
                print(f"MISMATCH {form}: `{asm}` encoder {want} assembler {got}")
        at += n
        if at > len(code):
            break
    for addr in sorted(set(rels) - want_rels):
        problems.append(f"RELOC a relocation at {addr:#x} that no case asked for")
    for p in problems[:20]:
        print(p)
    for form in sorted(per_form):
        n_ok, n_all = per_form[form]
        print(f"  {form:12s} {n_ok}/{n_all}")
    nrel = len([c for c in cases if int(c[4]) >= 0])
    print(f"forms: {len(per_form)}   cases: {len(cases)}   bytes: {total}   relocated: {nrel}   "
          f"identical: {len(cases) - bad}   different: {bad}")
    return 1 if bad or problems or total != len(code) else 0


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "compare":
        return cmd_compare(sys.argv[2], sys.argv[3])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
