#!/usr/bin/env python3
"""A small reader of the Grasm backend's relocatable ELF objects, for the
Linux gates (docs/self-hosting.md §6.18, stage S7).

    elf_obj.py relocs <obj>          objtest_elf: the header, sections and
                                     symbols are as `compiler/elf` lays them
                                     out; every relocation in a function's
                                     section is one its instruction takes,
                                     and every call and every page or
                                     RIP-relative address has one; in the
                                     data, every descriptor address has a
                                     64-bit pointer and nothing else does; in
                                     the debug sections, each address is a
                                     function's and each offset another
                                     debug section's
    elf_obj.py lines <obj> <tir>     objtest_elf: the line table, decoded
                                     here with its relocations, maps each
                                     instruction that loads a string
                                     literal's address to the file and line
                                     the literal is on

It reads the file itself, as `s3_macho.py` reads Mach-O, so that a tool's
presentation cannot make a gate pass or fail; the gate runs the tools as
well (`llvm-readelf`, `llvm-dwarfdump --verify`, GNU `readelf` and `ld`).
x86-64 instructions are found by `llvm-objdump -d`, since the bytes alone do
not say where one starts.
"""
import os
import re
import struct
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import s3_macho  # noqa: E402

EM_AARCH64 = 183
EM_X86_64 = 62
SHT_SYMTAB, SHT_RELA, SHT_NOBITS = 2, 4, 8
STT_FUNC, STT_SECTION = 2, 3

A64 = {257: "ABS64", 258: "ABS32", 275: "ADR_PREL_PG_HI21", 277: "ADD_ABS_LO12_NC",
       278: "LDST8_ABS_LO12_NC", 282: "JUMP26", 283: "CALL26", 284: "LDST16_ABS_LO12_NC",
       285: "LDST32_ABS_LO12_NC", 286: "LDST64_ABS_LO12_NC", 299: "LDST128_ABS_LO12_NC",
       311: "ADR_GOT_PAGE", 312: "LD64_GOT_LO12_NC", 549: "TLSLE_ADD_TPREL_HI12",
       551: "TLSLE_ADD_TPREL_LO12_NC"}
X64 = {1: "64", 2: "PC32", 4: "PLT32", 9: "GOTPCREL", 10: "32", 23: "TPOFF32", 42: "REX_GOTPCRELX"}


class Elf:
    def __init__(self, path):
        self.path = path
        d = self.data = open(path, "rb").read()
        if d[:4] != b"\x7fELF" or d[4] != 2 or d[5] != 1:
            raise SystemExit("not a little-endian ELF64 file")
        (etype, self.machine, _v, _entry, _phoff, shoff, _flags, ehsize, _phes, phnum, shes, shnum,
         shstrndx) = struct.unpack_from("<HHIQQQIHHHHHH", d, 16)
        if etype != 1:
            raise SystemExit(f"e_type {etype} is not ET_REL")
        if self.machine not in (EM_AARCH64, EM_X86_64):
            raise SystemExit(f"e_machine {self.machine} is neither EM_AARCH64 nor EM_X86_64")
        if ehsize != 64 or shes != 64 or phnum != 0:
            raise SystemExit(f"header sizes {ehsize}/{shes}, {phnum} program headers")
        self.secs = []
        for k in range(shnum):
            (name, typ, flags, addr, off, size, link, info, align,
             entsize) = struct.unpack_from("<IIQQQQIIQQ", d, shoff + 64 * k)
            self.secs.append(dict(name=name, type=typ, flags=flags, addr=addr, off=off, size=size,
                                  link=link, info=info, align=align, entsize=entsize))
        st = self.secs[shstrndx]
        for s in self.secs:
            s["name"] = self.cstr(st["off"] + s["name"])
        self.by_name = {s["name"]: k for k, s in enumerate(self.secs)}
        symx = [k for k, s in enumerate(self.secs) if s["type"] == SHT_SYMTAB]
        if len(symx) != 1:
            raise SystemExit(f"{len(symx)} symbol tables")
        self.symtab = symx[0]
        sym = self.secs[self.symtab]
        strs = self.secs[sym["link"]]
        self.first_global = sym["info"]
        self.syms = []
        for k in range(sym["size"] // 24):
            name, info, other, shndx, value, size = struct.unpack_from("<IBBHQQ", d, sym["off"] + 24 * k)
            self.syms.append(dict(name=self.cstr(strs["off"] + name), bind=info >> 4, type=info & 15,
                                  other=other, shndx=shndx, value=value, size=size))

    def cstr(self, at):
        return self.data[at:self.data.index(b"\0", at)].decode("utf-8", "replace")

    def bytes_of(self, k):
        s = self.secs[k]
        return self.data[s["off"]:s["off"] + s["size"]]

    def relas(self, target):
        """The relocations of section `target`: (offset, symbol, type, addend)."""
        out = []
        for s in self.secs:
            if s["type"] == SHT_RELA and s["info"] == target:
                if s["link"] != self.symtab or s["entsize"] != 24:
                    raise SystemExit(f"{s['name']}: link {s['link']}, entsize {s['entsize']}")
                for k in range(s["size"] // 24):
                    off, info, add = struct.unpack_from("<QQq", self.data, s["off"] + 24 * k)
                    out.append((off, info >> 32, info & 0xFFFFFFFF, add))
        return out


# ----- arm64 instruction classes ---------------------------------------------------------

def is_bl(w):
    return w & 0xFC000000 == 0x94000000


def is_b(w):
    return w & 0xFC000000 == 0x14000000


def is_adrp(w):
    return w & 0x9F000000 == 0x90000000


def is_add_imm(w, shifted):
    return w & 0x7F800000 == 0x11000000 and ((w >> 22) & 1) == (1 if shifted else 0)


def ldst_size(w):
    """A load or store with an unsigned 12-bit offset: its access size in
    bytes, or None."""
    if w & 0x3B000000 != 0x39000000:
        return None
    size = w >> 30
    if (w >> 26) & 1 and size == 0 and (w >> 23) & 1:
        return 16
    return 1 << size


def is_ldr_x(w):
    return w & 0xFFC00000 == 0xF9400000


def a64_fits(rtype, w):
    if rtype == 283:
        return is_bl(w)
    if rtype == 282:
        return is_b(w)
    if rtype in (275, 311):
        return is_adrp(w)
    if rtype == 277:
        return is_add_imm(w, False)
    if rtype in (278, 284, 285, 286, 299):
        return ldst_size(w) == {278: 1, 284: 2, 285: 4, 286: 8, 299: 16}[rtype]
    if rtype == 312:
        return is_ldr_x(w)
    if rtype == 549:
        return is_add_imm(w, True)
    if rtype == 551:
        return is_add_imm(w, False)
    return False


# ----- x86-64 instructions ---------------------------------------------------------------

INSN = re.compile(r"^\s*([0-9a-f]+):\s+((?:[0-9a-f]{2} )+)\s*(.*)$")
SECT = re.compile(r"^Disassembly of section (\S+):$")
OBJDUMP = os.environ.get("LLVM_OBJDUMP", "/opt/homebrew/opt/llvm@21/bin/llvm-objdump")


def x64_instructions(path):
    """`llvm-objdump -d`'s instructions, by section: {name: [(offset, length, text)]}."""
    p = subprocess.run([OBJDUMP, "-d", path],
                       capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit(f"llvm-objdump -d refuses {path}: {p.stderr.strip()[:300]}")
    out = {}
    cur = None
    for line in p.stdout.splitlines():
        m = SECT.match(line)
        if m:
            cur = out.setdefault(m.group(1), [])
            continue
        m = INSN.match(line)
        if m and cur is not None:
            cur.append((int(m.group(1), 16), len(m.group(2).split()), m.group(3).strip()))
    return out


def rip_form(code, at, n):
    """`lea`/`mov` from a RIP-relative address that ends the instruction
    (REX.W, 8D or 8B, ModRM mode 0 with rm 101): 'lea', 'mov', or None."""
    if n != 7:
        return None
    rex, op, modrm = code[at], code[at + 1], code[at + 2]
    if rex & 0xF8 == 0x48 and modrm & 0xC7 == 0x05:
        return {0x8D: "lea", 0x8B: "mov"}.get(op)
    return None


def tpoff_form(code, at, n):
    """`leaq disp32(%rax), %rax` whose displacement ends the instruction: the
    local-exec thread-local add (REX.W 8D, ModRM mode 2 rm 0)."""
    return n == 7 and code[at] & 0xF8 == 0x48 and code[at + 1] == 0x8D and code[at + 2] & 0xC0 == 0x80


# ----- relocs ------------------------------------------------------------------------------

def cmd_relocs(path):
    e = Elf(path)
    x64 = e.machine == EM_X86_64
    problems = []
    syms = e.syms
    # Symbols: locals first, every global a function defined in a `.text.`
    # section or undefined; the null symbol first.
    if syms[0]["name"] or syms[0]["shndx"] or syms[0]["type"] or syms[0]["bind"]:
        problems.append("symbol 0 is not the null symbol")
    for k, s in enumerate(syms):
        local = s["bind"] == 0
        if (k < e.first_global) != local:
            problems.append(f"symbol {k} `{s['name']}` is {'local' if local else 'global'} on the wrong side of {e.first_global}")
        if not local and s["shndx"] != 0:
            sec = e.secs[s["shndx"]]["name"]
            if s["type"] != STT_FUNC or not sec.startswith(".text."):
                problems.append(f"global `{s['name']}` is not a function in a `.text.` section ({sec})")
    # Every function's section is named for it and holds it alone at 0.
    funcs = {}
    for k, s in enumerate(syms):
        if s["type"] == STT_FUNC and s["shndx"]:
            sec = e.secs[s["shndx"]]
            if sec["name"] != ".text." + s["name"] or s["value"] != 0 or s["size"] != sec["size"]:
                problems.append(f"function `{s['name']}` is at {s['value']} size {s['size']} of {sec['name']} ({sec['size']} bytes)")
            if sec["flags"] != 6 or sec["align"] != (16 if x64 else 4):
                problems.append(f"{sec['name']}: flags {sec['flags']:#x}, alignment {sec['align']}")
            funcs[s["shndx"]] = k
    texts = [k for k, s in enumerate(e.secs) if s["name"].startswith(".text.")]
    for k in texts:
        if k not in funcs:
            problems.append(f"{e.secs[k]['name']} defines no function")
    if ".note.GNU-stack" not in e.by_name or e.secs[e.by_name[".note.GNU-stack"]]["flags"] != 0:
        problems.append("no `.note.GNU-stack` without flags: the stack would be executable")
    main = [s for s in syms if s["name"] == "main"]
    if main and (main[0]["bind"] != 1 or main[0]["type"] != STT_FUNC or not main[0]["shndx"]):
        problems.append("`main` is not a defined global function")
    nrel = 0
    insns = x64_instructions(path) if x64 else {}
    for k in texts:
        name = e.secs[k]["name"]
        code = e.bytes_of(k)
        rel = {}
        for off, sym, rtype, add in e.relas(k):
            nrel += 1
            if sym == 0 or sym >= len(syms):
                problems.append(f"{name}+{off:#x}: relocation against symbol {sym}")
                continue
            sname = syms[sym]["name"] or e.secs[syms[sym]["shndx"]]["name"]
            if off in rel:
                problems.append(f"{name}+{off:#x}: two relocations")
            rel[off] = (rtype, add, sname)
            if x64:
                if rtype not in (2, 4, 9, 23, 42) or add != (0 if rtype == 23 else -4):
                    problems.append(f"{name}+{off:#x}: R_X86_64_{X64.get(rtype, rtype)} {add:+d} against {sname}")
            else:
                if off % 4 or off + 4 > len(code):
                    problems.append(f"{name}+{off:#x}: not on an instruction")
                    continue
                w = struct.unpack_from("<I", code, off)[0]
                if rtype not in A64 or not a64_fits(rtype, w) or add != 0:
                    problems.append(f"{name}+{off:#x}: R_AARCH64_{A64.get(rtype, rtype)} {add:+d} ({sname}) on {w:08x}")
        if x64:
            seen = set()
            for at, n, txt in insns.get(name, []):
                end = at + n - 4
                want = None
                if code[at] == 0xE8 and n == 5:
                    want = (4,)
                elif rip_form(code, at, n) == "lea":
                    want = (2,)
                elif rip_form(code, at, n) == "mov":
                    want = (9, 42)
                elif tpoff_form(code, at, n) and end in rel:
                    want = (23,)
                if want is not None:
                    seen.add(end)
                    if rel.get(end, (None,))[0] not in want:
                        problems.append(f"{name}+{at:#x}: `{txt}` has no {'/'.join(X64[t] for t in want)} relocation")
            for off in sorted(set(rel) - seen):
                problems.append(f"{name}+{off:#x}: R_X86_64_{X64.get(rel[off][0])} ({rel[off][2]}) ends no instruction that takes it")
        else:
            for i in range(0, len(code) - 3, 4):
                w = struct.unpack_from("<I", code, i)[0]
                # Return-address signing on Linux is by hints, which a
                # processor without pointer authentication runs as no-ops;
                # `retab` and `retaa` are not hints and are undefined there,
                # and `pacibsp` is Darwin's form (docs/self-hosting.md
                # §6.18).
                if w in (0xD65F0FFF, 0xD65F0BFF, 0xD503237F):
                    problems.append(f"{name}+{i:#x}: {w:08x} ({ {0xD65F0FFF: 'retab', 0xD65F0BFF: 'retaa', 0xD503237F: 'pacibsp'}[w]}), "
                                    "not the hint forms Linux frames use")
                if is_bl(w) and rel.get(i, (None,))[0] != 283:
                    problems.append(f"{name}+{i:#x}: the `bl` has no CALL26 relocation")
                if is_adrp(w) and rel.get(i, (None,))[0] not in (275, 311):
                    problems.append(f"{name}+{i:#x}: the `adrp` has no page relocation")
    ndata = check_data(e, problems)
    ndebug = check_debug(e, problems)
    for p in problems[:20]:
        print(p)
    print(f"functions: {len(texts)}   relocations: {nrel}   data relocations: {ndata}   "
          f"debug relocations: {ndebug}   problems: {len(problems)}")
    return 1 if problems else 0


def check_data(e, problems):
    """Each `.data.` section is a struct descriptor (the runtime's
    `StructDesc`): a cache word, the type name's address and length, the
    field count, then per field its name's address, length and kind, then an
    auxiliary word per field. The addresses, and nothing else, carry a
    64-bit absolute relocation. Answers how many there are."""
    want_type = 1 if e.machine == EM_X86_64 else 257
    n = 0
    for k, s in enumerate(e.secs):
        if not s["name"].startswith(".data."):
            continue
        if s["flags"] != 3 or s["align"] != 8:
            problems.append(f"{s['name']}: flags {s['flags']:#x}, alignment {s['align']}")
        d = e.bytes_of(k)
        if len(d) < 32:
            problems.append(f"{s['name']} is {len(d)} bytes, not a descriptor")
            continue
        nfields = struct.unpack_from("<Q", d, 24)[0]
        want = {8} | {32 + 24 * i for i in range(nfields)}
        got = set()
        for off, sym, rtype, add in e.relas(k):
            n += 1
            got.add(off)
            if rtype != want_type or add != 0 or not (0 < sym < len(e.syms)):
                problems.append(f"{s['name']}+{off:#x}: type {rtype}, addend {add}, symbol {sym}")
        if got != want:
            problems.append(f"{s['name']}: pointers relocated at {sorted(got)}, the descriptor's are at {sorted(want)}")
    return n


def check_debug(e, problems):
    """In `.debug_info`, `.debug_ranges` and `.debug_line`, every 64-bit
    relocation is a function's address plus at most its size, and every
    32-bit one (in `.debug_info`) is an offset into another debug section,
    against its section symbol, inside it. Every relocated field holds 0.
    Answers how many there are."""
    x64 = e.machine == EM_X86_64
    t64, t32 = (1, 10) if x64 else (257, 258)
    n = 0
    for name in (".debug_info", ".debug_ranges", ".debug_line"):
        if name not in e.by_name:
            problems.append(f"no {name}")
            continue
        k = e.by_name[name]
        d = e.bytes_of(k)
        for off, sym, rtype, add in e.relas(k):
            n += 1
            s = e.syms[sym] if sym < len(e.syms) else None
            if s is None:
                problems.append(f"{name}+{off:#x}: symbol {sym}")
                continue
            if rtype == t64:
                if s["type"] != STT_FUNC or not 0 <= add <= s["size"]:
                    problems.append(f"{name}+{off:#x}: an address of {s['name']}{add:+d}, not in a function")
                if d[off:off + 8] != bytes(8):
                    problems.append(f"{name}+{off:#x}: the relocated address holds bytes")
            elif rtype == t32 and name == ".debug_info":
                target = e.secs[s["shndx"]] if s["shndx"] < len(e.secs) else None
                if s["type"] != STT_SECTION or target is None or not target["name"].startswith(".debug_") \
                        or not 0 <= add < max(target["size"], 1):
                    problems.append(f"{name}+{off:#x}: an offset {add} against {s['name'] or s['shndx']}")
                if d[off:off + 4] != bytes(4):
                    problems.append(f"{name}+{off:#x}: the relocated offset holds bytes")
            else:
                problems.append(f"{name}+{off:#x}: relocation type {rtype}")
    n += check_debug_fields(e, problems)
    return n


def uleb_at(d, q):
    v = s = 0
    while True:
        b = d[q]
        v |= (b & 0x7F) << s
        s += 7
        q += 1
        if not b & 0x80:
            return v, q


def check_debug_fields(e, problems):
    """Every field of `.debug_info` that holds an offset into another debug
    section -- the unit's offset into `.debug_abbrev`, each `DW_FORM_strp`
    and `DW_FORM_sec_offset` -- and every subprogram's `DW_AT_low_pc`
    carries a relocation: in one object the offsets are right without one,
    and wrong once ld puts several objects' debug sections end to end.
    Decoded here from the abbreviations. Answers how many fields."""
    if ".debug_info" not in e.by_name or ".debug_abbrev" not in e.by_name:
        return 0
    k = e.by_name[".debug_info"]
    d = e.bytes_of(k)
    ab = e.bytes_of(e.by_name[".debug_abbrev"])
    rel = {off: rtype for off, _sym, rtype, _add in e.relas(k)}
    shapes = {}
    q = 0
    while q < len(ab) and ab[q]:
        code, q = uleb_at(ab, q)
        tag, q = uleb_at(ab, q)
        q += 1
        attrs = []
        while True:
            at, q = uleb_at(ab, q)
            form, q = uleb_at(ab, q)
            if at == 0 and form == 0:
                break
            attrs.append((at, form))
        shapes[code] = (tag, attrs)
    want = [6]
    q = 11
    end = 4 + struct.unpack_from("<I", d, 0)[0]
    while q < end:
        code, q = uleb_at(d, q)
        if code == 0:
            continue
        if code not in shapes:
            problems.append(f".debug_info+{q:#x}: abbreviation {code} is not in .debug_abbrev")
            return 0
        tag, attrs = shapes[code]
        for at, form in attrs:
            if form in (0x0e, 0x17):
                want.append(q)
                q += 4
            elif form == 0x01:
                if tag == 0x2E:
                    want.append(q)
                q += 8
            elif form == 0x06:
                q += 4
            elif form == 0x0F:
                _v, q = uleb_at(d, q)
            else:
                problems.append(f".debug_info+{q:#x}: form {form:#x}, which this backend does not write")
                return 0
    for off in want:
        if off not in rel:
            problems.append(f".debug_info+{off:#x}: an offset or address that is not relocated")
    return len(want)


# ----- the line table ------------------------------------------------------------------------

def line_program(e):
    """`.debug_line` decoded, its sequences' start addresses taken from their
    relocations: [(function section, [(offset, file name, line)], end)]."""
    k = e.by_name[".debug_line"]
    d = e.bytes_of(k)
    start_of = {}
    for off, sym, _rtype, add in e.relas(k):
        start_of[off] = (e.syms[sym]["shndx"], add)
    unit_len, version, hdr_len = struct.unpack_from("<IHI", d, 0)
    if version != 4:
        raise SystemExit(f".debug_line version {version}")
    p = 10
    min_inst, _max_ops, default_stmt, line_base, line_range, opcode_base = struct.unpack_from("<BBBbBB", d, p)
    p += 6
    std_lens = list(d[p:p + opcode_base - 1])
    p += opcode_base - 1
    while d[p]:
        p = d.index(b"\0", p) + 1
    p += 1
    files = [None]
    while d[p]:
        end = d.index(b"\0", p)
        files.append(d[p:end].decode())
        p = end + 1
        for _ in range(3):
            while d[p] & 0x80:
                p += 1
            p += 1
    p += 1
    if p != 10 + hdr_len:
        raise SystemExit(f".debug_line: the header ends at {p}, not {10 + hdr_len}")

    def uleb(q):
        v = s = 0
        while True:
            b = d[q]
            v |= (b & 0x7F) << s
            s += 7
            q += 1
            if not b & 0x80:
                return v, q

    def sleb(q):
        v = s = 0
        while True:
            b = d[q]
            v |= (b & 0x7F) << s
            s += 7
            q += 1
            if not b & 0x80:
                if b & 0x40:
                    v -= 1 << s
                return v, q

    seqs = []
    end_all = 4 + unit_len
    addr, fil, line, sect, rows = 0, 1, 1, None, []
    while p < end_all:
        op = d[p]
        p += 1
        if op == 0:
            n, p = uleb(p)
            sub = d[p]
            if sub == 1:
                seqs.append((sect, rows, addr))
                addr, fil, line, sect, rows = 0, 1, 1, None, []
            elif sub == 2:
                if p + 1 not in start_of:
                    raise SystemExit(f".debug_line: set_address at {p + 1:#x} has no relocation")
                sect, addr = start_of[p + 1]
            p += n
        elif op >= opcode_base:
            adj = op - opcode_base
            addr += (adj // line_range) * min_inst
            line += line_base + adj % line_range
            rows.append((addr, files[fil], line))
        elif op == 1:
            rows.append((addr, files[fil], line))
        elif op == 2:
            v, p = uleb(p)
            addr += v * min_inst
        elif op == 3:
            v, p = sleb(p)
            line += v
        elif op == 4:
            fil, p = uleb(p)
        elif op == 8:
            addr += ((255 - opcode_base) // line_range) * min_inst
        elif op == 9:
            addr += struct.unpack_from("<H", d, p)[0]
            p += 2
        else:
            for _ in range(std_lens[op - 1]):
                _v, p = uleb(p)
    return seqs


def cmd_lines(obj, tir):
    e = Elf(obj)
    x64 = e.machine == EM_X86_64
    files = s3_macho.tir_files(tir)
    if not files:
        print(f"{tir} names no source file")
        return 1
    tir_text = open(tir, encoding="utf-8").read()
    seqs = line_program(e)
    by_sect = {}
    for sect, rows, end in seqs:
        if sect in by_sect:
            print(f"two line sequences for section {sect}")
            return 1
        by_sect[sect] = (rows, end)
    nrows = sum(len(rows) for _s, rows, _e in seqs)
    problems = []
    nlines = {}
    pieces = {}
    for path in files:
        try:
            src = open(path, encoding="utf-8").read()
        except OSError as ex:
            problems.append(f"cannot read {path}: {ex}")
            continue
        nlines[path] = src.count("\n") + 1
        for piece, ln in s3_macho.literal_pieces(src):
            pieces.setdefault(piece, []).append((path, ln))
    for sect, rows, end in seqs:
        if sect is None or e.secs[sect]["size"] != end:
            problems.append(f"a sequence of section {sect} ends at {end:#x}, not its size")
        for addr, name, ln in rows:
            if name not in nlines:
                problems.append(f"a row names {name}, which is not one of the program's files")
            elif ln > nlines[name]:
                problems.append(f"a row is line {ln} of {name}, which has {nlines[name]}")
    # The string pool's symbols and their texts.
    text_of = {}
    pool = e.by_name.get(".rodata.str1.1")
    if pool is not None:
        d = e.bytes_of(pool)
        for k, s in enumerate(e.syms):
            if s["name"].startswith("l_.str.") and s["shndx"] == pool:
                text_of[k] = d[s["value"]:d.index(b"\0", s["value"])].decode("utf-8", "replace")
    refs = {}
    for k, s in enumerate(e.secs):
        if not s["name"].startswith(".text."):
            continue
        for off, sym, rtype, _add in e.relas(k):
            if sym not in text_of:
                continue
            if x64 and rtype == 2:
                refs.setdefault(sym, []).append((k, off - 3, "a"))
            elif not x64 and rtype in (275, 277):
                refs.setdefault(sym, []).append((k, off, "p" if rtype == 275 else "o"))
    checked = 0
    repeated = 0
    for sym, rs in sorted(refs.items()):
        s = text_of[sym]
        where = pieces.get(s, [])
        if len(s) < 3 or len(where) != 1:
            continue
        kinds = [r[2] for r in rs]
        if not x64 and (kinds.count("p") != kinds.count("o")):
            continue
        loads = len(rs) if x64 else len(rs) // 2
        if loads > 1 and tir_text.count('"' + s + '"') + tir_text.count('::' + s + '"') != 1:
            continue
        path, want = where[0]
        for sect, addr, _kind in rs:
            rows = by_sect.get(sect, ([], 0))[0]
            best = None
            for r in rows:
                if r[0] > addr:
                    break
                best = r
            checked += 1
            if loads > 1:
                repeated += 1
            if best is None:
                problems.append(f"no line row covers {e.secs[sect]['name']}+{addr:#x}, which loads {s!r}")
            elif best[1] != path or best[2] != want:
                problems.append(f"{e.secs[sect]['name']}+{addr:#x}, which loads {s!r} (line {want} of {path}), "
                                f"maps to line {best[2]} of {best[1]}")
    for p in problems[:12]:
        print(p)
    print(f"line rows: {nrows}   literal loads checked: {checked}   wrong: {len(problems)}   of literals loaded more than once: {repeated}")
    return 1 if problems else 0


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "relocs":
        return cmd_relocs(sys.argv[2])
    if len(sys.argv) == 4 and sys.argv[1] == "lines":
        return cmd_lines(sys.argv[2], sys.argv[3])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
