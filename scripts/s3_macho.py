#!/usr/bin/env python3
"""A small reader of arm64 Mach-O relocatable objects, for the S3 gates.

    s3_macho.py text <obj>                  the __text words, one hex word a line
    s3_macho.py compare <obj> <cases.tsv>   encodetest: the object's words against
                                            the encoder's, case by case
    s3_macho.py relocs <obj>                objtest: every relocation lands on the
                                            instruction its kind relocates, and
                                            every such instruction has one; in
                                            `__data`, every descriptor address
                                            has a 64-bit pointer relocation and
                                            nothing else does; in the debug
                                            sections, each is a `__text` address
    s3_macho.py lines <obj> <tir>           objtest: `dwarfdump --debug-line`
                                            maps each instruction that loads a
                                            string literal's address to the
                                            file and line the literal is on

It reads the file itself rather than parsing `otool` output, so a change in a
tool's presentation cannot make a gate pass or fail. The tools themselves are
run by the gates as well: `otool -l`, `nm` and the linker must each accept
every object.
"""
import json
import re
import struct
import subprocess
import sys

LC_SEGMENT_64 = 0x19
LC_SYMTAB = 0x2


def sections(data):
    magic, cpu, _sub, ftype, ncmds, _size, _flags, _res = struct.unpack_from("<IiiIIIII", data, 0)
    if magic != 0xFEEDFACF:
        raise SystemExit("not a 64-bit Mach-O file")
    if cpu != 0x0100000C:
        raise SystemExit(f"cputype {cpu:#x} is not arm64")
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


def text_words(data):
    secs, _ = sections(data)
    t = secs.get(("__TEXT", "__text"))
    if t is None:
        raise SystemExit("no __text section")
    n = t["size"] // 4
    return [struct.unpack_from("<I", data, t["off"] + 4 * k)[0] for k in range(n)], t


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


def cmd_text(path):
    data = open(path, "rb").read()
    words, _ = text_words(data)
    for w in words:
        print(f"{w:08x}")


def cmd_compare(path, tsv):
    data = open(path, "rb").read()
    words, _ = text_words(data)
    cases = [line.rstrip("\n").split("\t") for line in open(tsv)]
    if len(words) != len(cases):
        print(f"the assembler made {len(words)} words of {len(cases)} lines")
        return 1
    per_form = {}
    bad = 0
    for (form, want, asm), got in zip(cases, words):
        n_ok, n_all = per_form.get(form, (0, 0))
        if int(want, 16) == got:
            per_form[form] = (n_ok + 1, n_all + 1)
        else:
            per_form[form] = (n_ok, n_all + 1)
            bad += 1
            if bad <= 20:
                print(f"MISMATCH {form}: `{asm}` encoder {want} assembler {got:08x}")
    for form in sorted(per_form):
        n_ok, n_all = per_form[form]
        print(f"  {form:12s} {n_ok}/{n_all}")
    print(f"forms: {len(per_form)}   cases: {len(cases)}   identical: {len(cases) - bad}   different: {bad}")
    return 1 if bad else 0


# What each relocation type relocates, by instruction class.
def is_bl(w):
    return w & 0xFC000000 == 0x94000000


def is_adrp(w):
    return w & 0x9F000000 == 0x90000000


def is_add_imm64(w):
    return w & 0xFF800000 == 0x91000000


def is_ldr_imm64(w):
    return w & 0xFFC00000 == 0xF9400000


def cmd_relocs(path):
    data = open(path, "rb").read()
    words, t = text_words(data)
    syms = symbols(data)
    rel = {}
    problems = []
    for k in range(t["nreloc"]):
        addr, info = struct.unpack_from("<iI", data, t["reloff"] + 8 * k)
        symnum = info & 0xFFFFFF
        pcrel = (info >> 24) & 1
        length = (info >> 25) & 3
        ext = (info >> 27) & 1
        rtype = info >> 28
        if addr % 4 or addr // 4 >= len(words):
            problems.append(f"relocation {k} at {addr:#x} is not on an instruction")
            continue
        if not ext or symnum >= len(syms):
            problems.append(f"relocation {k} at {addr:#x} names no symbol")
            continue
        if length != 2:
            problems.append(f"relocation {k} at {addr:#x} has length {length}")
        w = words[addr // 4]
        # 5 and 6: a C function's address through its GOT slot
        # (ARM64_RELOC_GOT_LOAD_PAGE21, _PAGEOFF12).
        want = {2: (is_bl, 1, "bl"), 3: (is_adrp, 1, "adrp"), 4: (is_add_imm64, 0, "add"),
                5: (is_adrp, 1, "adrp"), 6: (is_ldr_imm64, 0, "ldr")}.get(rtype)
        if want is None:
            problems.append(f"relocation {k} at {addr:#x} has type {rtype}, which this backend never writes")
            continue
        test, want_pcrel, what = want
        if not test(w):
            problems.append(f"relocation {k} ({what}, {syms[symnum][0]}) at {addr:#x} is on {w:08x}, which is not a `{what}`")
        if pcrel != want_pcrel:
            problems.append(f"relocation {k} at {addr:#x}: pcrel {pcrel} for type {rtype}")
        if addr in rel:
            problems.append(f"two relocations at {addr:#x}")
        rel[addr] = rtype
    # Every call and every page address must be relocated: none of them can
    # be resolved inside an object whose atoms the linker may move.
    for i, w in enumerate(words):
        if is_bl(w) and rel.get(4 * i) != 2:
            problems.append(f"the `bl` at {4 * i:#x} has no BRANCH26 relocation")
        if is_adrp(w) and rel.get(4 * i) not in (3, 5):
            problems.append(f"the `adrp` at {4 * i:#x} has no PAGE21 relocation")
    ndata = check_data_relocs(data, syms, problems)
    ndebug = check_debug_relocs(data, t, problems)
    for p in problems[:20]:
        print(p)
    print(f"relocations: {len(rel)}   data relocations: {ndata}   debug relocations: {ndebug}   "
          f"problems: {len(problems)}")
    return 1 if problems else 0


def check_debug_relocs(data, t, problems):
    """The compile unit's and each function's `DW_AT_low_pc`, and each line
    sequence's `DW_LNE_set_address`, hold a `__text` address: each carries a
    64-bit absolute relocation against the section (`ARM64_RELOC_UNSIGNED`,
    not extern, section 1), as clang's do, holding an address inside it.
    Answers how many there are."""
    secs, _ = sections(data)
    n = 0
    for name in ("__debug_info", "__debug_line"):
        d = secs.get(("__DWARF", name))
        if d is None:
            continue
        for k in range(d["nreloc"]):
            addr, info = struct.unpack_from("<iI", data, d["reloff"] + 8 * k)
            symnum = info & 0xFFFFFF
            pcrel = (info >> 24) & 1
            length = (info >> 25) & 3
            ext = (info >> 27) & 1
            rtype = info >> 28
            n += 1
            if rtype != 0 or length != 3 or pcrel or ext or symnum != 1:
                problems.append(f"{name} relocation {k} at {addr:#x}: type {rtype}, length {length}, "
                                f"pcrel {pcrel}, extern {ext}, section {symnum}")
                continue
            if addr < 0 or addr + 8 > d["size"]:
                problems.append(f"{name} relocation {k} at {addr:#x} is outside the section")
                continue
            value = struct.unpack_from("<Q", data, d["off"] + addr)[0]
            if not t["addr"] <= value <= t["addr"] + t["size"]:
                problems.append(f"{name} relocation {k} at {addr:#x} holds {value:#x}, not a __text address")
    return n


# ----- the line table ---------------------------------------------------------------

ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "\\": "\\", '"': '"', "0": "\0", "{": "{", "}": "}"}


def literal_pieces(text):
    """Every text piece of every string literal in a source file -- the runs
    between its quotes and its interpolations, escapes decoded -- with the
    line the literal opens on ([L-STRING]: a literal does not span lines).
    Raw blocks and comments are skipped."""
    out = []
    n = len(text)

    def string(i, line):
        i += 1
        buf = []
        while i < n:
            c = text[i]
            if c == "\n":
                return i
            if c == "\\" and i + 1 < n:
                e = text[i + 1]
                if e == "u":
                    j = text.find("}", i)
                    try:
                        buf.append(chr(int(text[i + 3:j], 16)))
                    except ValueError:
                        buf.append("?")
                    i = j + 1
                    continue
                buf.append(ESCAPES.get(e, e))
                i += 2
                continue
            if c == '"':
                out.append(("".join(buf), line))
                return i + 1
            if c == "{":
                out.append(("".join(buf), line))
                buf = []
                depth = 1
                i += 1
                while i < n and depth:
                    c2 = text[i]
                    if c2 == "\n":
                        return i
                    if c2 == '"':
                        i = string(i, line)
                        continue
                    if c2 == "{":
                        depth += 1
                    elif c2 == "}":
                        depth -= 1
                    i += 1
                continue
            buf.append(c)
            i += 1
        return i

    i = 0
    line = 1
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
        elif text.startswith('"""', i):
            j = text.find('"""', i + 3)
            j = n if j < 0 else j + 3
            line += text.count("\n", i, j)
            i = j
        elif c == '"':
            i = string(i, line)
        else:
            i += 1
    return out


FILE_LINE = re.compile(r'^\(file "(?:[^"\\]|\\.)*" "(?:[^"\\]|\\.)*" "((?:[^"\\]|\\.)*)" ([0-9]+)')


def tir_files(tir):
    """TIR's source files (docs/tir.md §2), in loader order, as the loader
    was given them."""
    out = []
    for line in open(tir, encoding="utf-8"):
        m = FILE_LINE.match(line)
        if m:
            out.append(json.loads('"' + m.group(1) + '"'))
    return out


ROW = re.compile(r"^0x([0-9a-f]+)\s+([0-9]+)\s+([0-9]+)\s+([0-9]+)\s+[0-9]+\s+[0-9]+\s+[0-9]+\s*(.*)$")


def line_table(obj):
    """`dwarfdump --debug-line`'s rows as sequences: each a list of
    (address, file name, line) and the address its `end_sequence` row has."""
    p = subprocess.run(["dwarfdump", "--debug-line", obj], capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit(f"dwarfdump --debug-line refuses {obj}: {p.stderr.strip()[:300]}")
    names = {}
    seqs = []
    cur = []
    pending = None
    for line in p.stdout.splitlines():
        m = re.match(r"^file_names\[\s*([0-9]+)\]:", line)
        if m:
            pending = int(m.group(1))
            continue
        m = re.match(r'^\s+name: "(.*)"$', line)
        if m and pending is not None:
            names[pending] = m.group(1)
            pending = None
            continue
        m = ROW.match(line)
        if m:
            addr, ln, f = int(m.group(1), 16), int(m.group(2)), int(m.group(4))
            if "end_sequence" in m.group(5):
                seqs.append((cur, addr))
                cur = []
            else:
                cur.append((addr, names.get(f, f"#{f}"), ln))
    return seqs


def lookup(seqs, addr):
    """The row that covers `addr`: the last at or before it in the sequence
    whose range holds it."""
    for rows, end in seqs:
        if rows and rows[0][0] <= addr < end:
            best = None
            for r in rows:
                if r[0] > addr:
                    break
                best = r
            return best
    return None


def cmd_lines(obj, tir):
    data = open(obj, "rb").read()
    _words, t = text_words(data)
    secs, _ = sections(data)
    syms = symbols(data)
    files = tir_files(tir)
    if not files:
        print(f"{tir} names no source file")
        return 1
    seqs = line_table(obj)
    nrows = sum(len(rows) for rows, _ in seqs)
    problems = []
    nlines = {}
    pieces = {}
    for path in files:
        try:
            text = open(path, encoding="utf-8").read()
        except OSError as e:
            problems.append(f"cannot read {path}: {e}")
            continue
        nlines[path] = text.count("\n") + 1
        for piece, ln in literal_pieces(text):
            pieces.setdefault(piece, []).append((path, ln))
    # Every row's line is one its file has, and every row names a file.
    for rows, _end in seqs:
        for addr, name, ln in rows:
            if name not in nlines:
                problems.append(f"the row at {addr:#x} names {name}, which is not one of the program's files")
            elif ln > nlines[name]:
                problems.append(f"the row at {addr:#x} is line {ln} of {name}, which has {nlines[name]}")
    # The string pool: each literal's symbol and text.
    cstr = secs.get(("__TEXT", "__cstring"))
    sect_no = {key: 1 + k for k, key in enumerate(secs)}
    text_of = {}
    if cstr is not None:
        for k, (name, _ntype, nsect, value) in enumerate(syms):
            if name.startswith("l_.str.") and nsect == sect_no[("__TEXT", "__cstring")]:
                off = cstr["off"] + value - cstr["addr"]
                end = data.index(b"\0", off)
                text_of[k] = data[off:end].decode("utf-8", "replace")
    # The instructions that load each literal's address.
    refs = {}
    for k in range(t["nreloc"]):
        addr, info = struct.unpack_from("<iI", data, t["reloff"] + 8 * k)
        symnum = info & 0xFFFFFF
        rtype = info >> 28
        if (info >> 27) & 1 and rtype in (3, 4) and symnum in text_of:
            refs.setdefault(symnum, []).append((rtype, addr))
    checked = 0
    for symnum, rs in sorted(refs.items()):
        s = text_of[symnum]
        # A literal written once in the program and loaded once, by one
        # `adrp` and one `add`. A string the compiler makes itself, or one
        # written twice, says nothing about which line is right.
        where = pieces.get(s, [])
        if len(s) < 3 or len(where) != 1 or sorted(r for r, _ in rs) != [3, 4]:
            continue
        path, want = where[0]
        for _rtype, addr in rs:
            row = lookup(seqs, addr)
            checked += 1
            if row is None:
                problems.append(f"no line row covers {addr:#x}, which loads {s!r}")
            elif row[1] != path or row[2] != want:
                problems.append(f"{addr:#x}, which loads {s!r} (line {want} of {path}), "
                                f"maps to line {row[2]} of {row[1]}")
    for p in problems[:12]:
        print(p)
    print(f"line rows: {nrows}   literal loads checked: {checked}   wrong: {len(problems)}")
    return 1 if problems else 0


def check_data_relocs(data, syms, problems):
    """`__DATA,__data` holds struct descriptors, the runtime's `StructDesc`:
    a cache word, the type name's address and length, the field count, then
    per field its name's address, length and kind, then an auxiliary word per
    field. Each address -- and nothing else -- must carry a 64-bit absolute
    relocation (ARM64_RELOC_UNSIGNED) against a symbol, so the linker fills it
    in. Answers how many relocations the section has."""
    secs, _ = sections(data)
    d = secs.get(("__DATA", "__data"))
    if d is None:
        return 0
    got = set()
    for k in range(d["nreloc"]):
        addr, info = struct.unpack_from("<iI", data, d["reloff"] + 8 * k)
        symnum = info & 0xFFFFFF
        pcrel = (info >> 24) & 1
        length = (info >> 25) & 3
        ext = (info >> 27) & 1
        rtype = info >> 28
        if rtype != 0 or length != 3 or pcrel or not ext:
            problems.append(f"data relocation {k} at {addr:#x}: type {rtype}, length {length}, pcrel {pcrel}, extern {ext}")
        if addr % 8 or addr + 8 > d["size"]:
            problems.append(f"data relocation {k} at {addr:#x} is not an aligned word of __data")
        if symnum >= len(syms):
            problems.append(f"data relocation {k} at {addr:#x} names no symbol")
        if addr in got:
            problems.append(f"two data relocations at {addr:#x}")
        got.add(addr)
    want = set()
    # Sections are numbered from 1 in the order the load command lists them.
    data_sect = 1 + list(secs.values()).index(d)
    for name, ntype, nsect, value in syms:
        if nsect != data_sect or not name.startswith("l_.desc."):
            continue
        off = value - d["addr"]
        nfields = struct.unpack_from("<Q", data, d["off"] + off + 24)[0]
        want.add(off + 8)
        for i in range(nfields):
            want.add(off + 32 + 24 * i)
    for a in sorted(want - got):
        problems.append(f"the descriptor word at __data+{a:#x} is an address with no relocation")
    for a in sorted(got - want):
        problems.append(f"the data relocation at __data+{a:#x} is on no descriptor's address")
    return len(got)


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "text":
        cmd_text(sys.argv[2])
        return 0
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
