#!/usr/bin/env python3
"""A small reader of x86-64 Mach-O relocatable objects, for the S7 gates.

    x64_macho.py compare <obj> <cases.tsv>   encodetest_x64: the object's bytes
                                             against the encoder's, case by
                                             case, and each relocation where
                                             the encoder says it must be
    x64_macho.py relocs <obj>                objtest_x64: every relocation in
                                             `__text` ends the instruction its
                                             kind relocates, and every call to
                                             a symbol and every RIP-relative
                                             instruction has one; `__data`,
                                             `__thread_vars` and the DWARF as
                                             `s3_macho.py relocs` checks them
    x64_macho.py lines <obj> <tir>           objtest_x64: each instruction
                                             that loads a string literal's
                                             address maps to the literal's
                                             file and line

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
import os
import re
import struct
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import s3_macho  # noqa: E402

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


# The arm64 reader's checks of what the two targets lay out alike -- the
# descriptors in `__data`, the thread-local descriptors, the DWARF's
# relocations, the line table read through `dwarfdump` -- are used as they
# are, reading sections through this module's reader, which takes an x86-64
# object where theirs takes an arm64 one.
s3_macho.sections = sections

INSN = re.compile(r"^\s*([0-9a-f]+):\s+((?:[0-9a-f]{2} )+)\s*(.*)$")


def instructions(obj):
    """`objdump -d`'s instructions in `__text`: (offset, length, text). The
    disassembler is used for where each instruction starts, which the bytes
    alone do not say on x86-64; what each relocation is checked against is
    the bytes themselves."""
    p = subprocess.run(["objdump", "-d", "--no-leading-headers", obj], capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit(f"objdump -d refuses {obj}: {p.stderr.strip()[:300]}")
    out = []
    for line in p.stdout.splitlines():
        m = INSN.match(line)
        if m:
            out.append((int(m.group(1), 16), len(m.group(2).split()), m.group(3).strip()))
    return out


def rip_relative(code, at, n):
    """Whether the instruction at `at` of `n` bytes is `lea` or `mov` from a
    RIP-relative address ending the instruction: REX.W, 8D or 8B, ModRM mode
    0 with rm 101."""
    if n != 7:
        return False
    rex, op, modrm = code[at], code[at + 1], code[at + 2]
    return rex & 0xF8 == 0x48 and op in (0x8D, 0x8B) and modrm & 0xC7 == 0x05


def cmd_relocs(path):
    data = open(path, "rb").read()
    code, t = text(data)
    syms = symbols(data)
    problems = []
    rel = {}
    for k, (addr, rtype, pcrel, length, ext, sym) in enumerate(relocations(data, t)):
        if not ext or sym >= len(syms):
            problems.append(f"relocation {k} at {addr:#x} names no symbol")
            continue
        if rtype not in (1, 2, 3, 9) or pcrel != 1 or length != 2:
            problems.append(f"relocation {k} at {addr:#x}: {RELOC_NAMES.get(rtype, rtype)}, pcrel {pcrel}, length {length}")
        if addr in rel:
            problems.append(f"two relocations at {addr:#x}")
        rel[addr] = (rtype, syms[sym][0])
    seen = set()
    for at, n, txt in instructions(path):
        end = at + n - 4
        if code[at] == 0xE8 and n == 5:
            if rel.get(end, (None,))[0] != 2:
                problems.append(f"the call at {at:#x} has no BRANCH relocation")
            seen.add(end)
        elif rip_relative(code, at, n):
            want = (1,) if code[at + 1] == 0x8D else (3, 9)
            if rel.get(end, (None,))[0] not in want:
                problems.append(f"the RIP-relative `{txt}` at {at:#x} has no {'SIGNED' if want == (1,) else 'GOT_LOAD or TLV'} relocation")
            seen.add(end)
    for addr in sorted(set(rel) - seen):
        problems.append(f"the relocation at {addr:#x} ({RELOC_NAMES.get(rel[addr][0])}, {rel[addr][1]}) ends no call or RIP-relative instruction")
    ndata = s3_macho.check_data_relocs(data, syms, problems)
    ndata += s3_macho.check_tlv_relocs(data, syms, problems)
    ndebug = s3_macho.check_debug_relocs(data, t, problems)
    for p in problems[:20]:
        print(p)
    print(f"relocations: {len(rel)}   data relocations: {ndata}   debug relocations: {ndebug}   problems: {len(problems)}")
    return 1 if problems else 0


def cmd_lines(obj, tir):
    """As `s3_macho.py lines`, for x86-64: a literal's address is loaded by
    one `leaq l_.str.N(%rip)` (a SIGNED relocation against the literal's
    symbol) per copy of the function it is in, and the instruction starts
    three bytes before its displacement."""
    data = open(obj, "rb").read()
    _code, t = text(data)
    secs, _ = sections(data)
    syms = symbols(data)
    files = s3_macho.tir_files(tir)
    if not files:
        print(f"{tir} names no source file")
        return 1
    tir_text = open(tir, encoding="utf-8").read()
    seqs = s3_macho.line_table(obj)
    nrows = sum(len(rows) for rows, _ in seqs)
    problems = []
    nlines = {}
    pieces = {}
    for path in files:
        try:
            src = open(path, encoding="utf-8").read()
        except OSError as e:
            problems.append(f"cannot read {path}: {e}")
            continue
        nlines[path] = src.count("\n") + 1
        for piece, ln in s3_macho.literal_pieces(src):
            pieces.setdefault(piece, []).append((path, ln))
    for rows, _end in seqs:
        for addr, name, ln in rows:
            if name not in nlines:
                problems.append(f"the row at {addr:#x} names {name}, which is not one of the program's files")
            elif ln > nlines[name]:
                problems.append(f"the row at {addr:#x} is line {ln} of {name}, which has {nlines[name]}")
    cstr = secs.get(("__TEXT", "__cstring"))
    sect_no = {key: 1 + k for k, key in enumerate(secs)}
    text_of = {}
    if cstr is not None:
        for k, (name, _ntype, nsect, value) in enumerate(syms):
            if name.startswith("l_.str.") and nsect == sect_no[("__TEXT", "__cstring")]:
                off = cstr["off"] + value - cstr["addr"]
                end = data.index(b"\0", off)
                text_of[k] = data[off:end].decode("utf-8", "replace")
    refs = {}
    for addr, rtype, _pcrel, _length, ext, sym in relocations(data, t):
        if ext and rtype == 1 and sym in text_of:
            refs.setdefault(sym, []).append(addr - 3)
    checked = 0
    repeated = 0
    for sym, addrs in sorted(refs.items()):
        s = text_of[sym]
        where = pieces.get(s, [])
        if len(s) < 3 or len(where) != 1:
            continue
        if len(addrs) > 1 and tir_text.count('"' + s + '"') + tir_text.count('::' + s + '"') != 1:
            continue
        path, want = where[0]
        for addr in addrs:
            row = s3_macho.lookup(seqs, addr)
            checked += 1
            if len(addrs) > 1:
                repeated += 1
            if row is None:
                problems.append(f"no line row covers {addr:#x}, which loads {s!r}")
            elif row[1] != path or row[2] != want:
                problems.append(f"{addr:#x}, which loads {s!r} (line {want} of {path}), maps to line {row[2]} of {row[1]}")
    for p in problems[:12]:
        print(p)
    print(f"line rows: {nrows}   literal loads checked: {checked}   wrong: {len(problems)}   of literals loaded more than once: {repeated}")
    return 1 if problems else 0


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "compare":
        return cmd_compare(sys.argv[2], sys.argv[3])
    if len(sys.argv) == 3 and sys.argv[1] == "relocs":
        return cmd_relocs(sys.argv[2])
    if len(sys.argv) == 4 and sys.argv[1] == "lines":
        return cmd_lines(sys.argv[2], sys.argv[3])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
