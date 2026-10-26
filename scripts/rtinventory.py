#!/usr/bin/env python3
"""The Rust runtime's inventory, measured (docs/self-hosting.md §6.7).

    python3 scripts/rtinventory.py <libgrasm_rt_abi.a> [names.txt]

Every C entry point the archive exports (`_grasm_*`), grouped by family, and
for each family:

  * from the compiled archive, which is what a program links: the functions
    its entry points reach through the relocations of the archive's objects,
    their machine code by crate, the modules of `core`, `alloc` and
    `hashbrown` among them, and the C library functions they call -- exact
    for what was compiled, inlined code included in its caller;
  * from the source of the five crates a program links (`grasm_rt` with
    `tasks.rs`, `grasm_sys`, `grasm_fiber`, `grasm_journal`, `grasm_rt_abi`):
    the lines of the entry points themselves, and the lines of the items
    (functions, types with their `impl` blocks, statics) their calls and
    types reach -- those only this family reaches, and those it shares. An
    item reaches another by naming it as a call (`name(`, `Type::name(`), a
    type or a static; a method call `.name(` counts when one `impl` block
    alone defines `name`. That is an estimate and is printed as one.

With `names.txt` (`scripts/rtswap.sh`'s `RT_NAMES=`: `count side name` per
runtime name the corpus's objects name in a relocation), how many of each
family's entry points the corpus uses.
"""
import bisect
import re
import struct
import sys

CRATES = [
    "crates/grasm_rt/src/lib.rs",
    "crates/grasm_rt/src/tasks.rs",
    "crates/grasm_sys/src/lib.rs",
    "crates/grasm_fiber/src/lib.rs",
    "crates/grasm_journal/src/lib.rs",
    "crates/grasm_rt_abi/src/lib.rs",
]

# Families, by the entry point's name after `grasm_`; first match wins.
FAMILIES = [
    ("values and counting", r"^(val_inc|val_dec|val_dup|val_release|val_eq|val_cmp|box_\w+|unbox_\w+|region_\w+|live_allocations|check_leaks|layout_check\w*)$"),
    ("number formatting and parsing", r"^(str_from_(i64|u64|f64|bool)|str_parse_\w+|i64_to_base|u64_to_base|f64_to_fixed|str_build_(i64|f64|bool)|str_append_(i64|f64|bool)|str_pin_i64)$"),
    ("output", r"^out_\w+$"),
    ("panics, exit, stack floor", r"^(panic_\w+|exit|stack_floor|stack_check|assert_eq|report_main_err)$"),
    ("math", r"^(pow_\w+|f64_\w+|i64_abs|i64_min|i64_max|int_checked|int_saturating|int_try)$"),
    ("strings", r"^(str_\w+|builder_\w+|strbuf_new|i64_to_char|u64_to_char)$"),
    ("bytes", r"^(bytes_\w+|bytebuf_\w+|append_bytes)$"),
    ("lists and ranges", r"^(list_\w+|range_\w+|vals_\w+)$"),
    ("maps and hashing", r"^map_\w+$"),
    ("structs, variants, tuples, results", r"^(struct_\w+|variant_\w+|tuple\w*|ok_new|err_new|result_\w+|try_take\w*|is_ok|inline_\w+)$"),
    ("function values, effect handlers", r"^(fn_\w+|env\w*|handler_\w+)$"),
    ("generational references", r"^ref_\w+$"),
    ("args, files, clock, net, proc, C, raw memory", r"^(args|set_args|input|read_\w+|write_\w+|file_\w+|list_dir|make_dir\w*|remove_file|rename_file|sync_file|tcp_\w+|udp_\w+|net_close|net_port|net_wait|proc_\w+|now_millis|monotonic_\w+|sleep_millis|random_bytes|foreign_call|mem_\w+|tls_block)$"),
    ("tasks, channels, fibers, parallelism", r"^(chan_\w+|conc_run|par_\w+|net_workers\w*)$"),
]


def family(name):
    for fam, pat in FAMILIES:
        if re.match(pat, name):
            return fam
    return "unassigned"


# ----- the compiled archive -------------------------------------------------------

def members(path):
    d = open(path, "rb").read()
    at = 8
    while at + 60 <= len(d):
        name = d[at:at + 16].decode().strip()
        size = int(d[at + 48:at + 58].decode().strip())
        start = at + 60
        if name.startswith("#1/"):
            n = int(name[3:])
            name = d[start:start + n].split(b"\0")[0].decode()
            body = d[start + n:start + size]
        else:
            body = d[start:start + size]
        if not name.startswith("__.SYMDEF"):
            yield name, body
        at = start + size + (size % 2)


def crate_of_member(name):
    for c in ("grasm_rt_abi.", "grasm_rt-", "grasm_sys", "grasm_fiber", "grasm_journal", "hashbrown", "foldhash", "core-", "alloc-", "compiler_builtins"):
        if c in name:
            return c.strip(".-")
    return "compiler_builtins"


def parse_object(member, d):
    """Atoms (key, section kind, size, member) and the keys each one names."""
    if len(d) < 32 or struct.unpack_from("<I", d, 0)[0] != 0xFEEDFACF:
        return [], {}
    ncmds = struct.unpack_from("<I", d, 16)[0]
    p = 32
    sects = []
    symoff = nsyms = stroff = 0
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", d, p)
        if cmd == 0x19:
            n = struct.unpack_from("<I", d, p + 64)[0]
            for k in range(n):
                s = p + 72 + 80 * k
                sn = d[s:s + 16].split(b"\0")[0].decode()
                seg = d[s + 16:s + 32].split(b"\0")[0].decode()
                addr, sz = struct.unpack_from("<QQ", d, s + 32)
                reloff, nreloc, flags = struct.unpack_from("<III", d, s + 56)
                sects.append((sn, seg, addr, sz, reloff, nreloc, flags))
        elif cmd == 2:
            symoff, nsyms, stroff = struct.unpack_from("<III", d, p + 8)
        p += size
    syms = []
    for i in range(nsyms):
        e = symoff + 16 * i
        strx, ty, sect, desc, value = struct.unpack_from("<IBBHQ", d, e)
        end = d.index(b"\0", stroff + strx)
        name = d[stroff + strx:end].decode(errors="replace")
        syms.append((name, ty, sect, value))

    def key(i):
        name, ty, sect, value = syms[i]
        return name if ty & 1 else (member, name)

    by_sect = {}
    for i, (name, ty, sect, value) in enumerate(syms):
        if ty & 0xE0 == 0 and ty & 0x0E == 0x0E and sect > 0:
            by_sect.setdefault(sect, []).append((value, i))
    atoms = []
    edges = {}
    for si, (sn, seg, addr, sz, reloff, nreloc, flags) in enumerate(sects, start=1):
        if seg in ("__DWARF", "__LD") or sn in ("__eh_frame", "__compact_unwind"):
            continue
        starts = sorted(by_sect.get(si, []))
        if not starts:
            continue
        values = [v for v, _ in starts]
        code = sn == "__text"
        for k, (v, i) in enumerate(starts):
            end = starts[k + 1][0] if k + 1 < len(starts) else addr + sz
            atoms.append((key(i), code, end - v, member))
        for r in range(nreloc):
            a, w = struct.unpack_from("<iI", d, reloff + 8 * r)
            symnum = w & 0xFFFFFF
            ext = (w >> 27) & 1
            typ = w >> 28
            if not ext or typ in (1, 10):
                continue
            k = bisect.bisect_right(values, addr + a) - 1
            if k < 0:
                continue
            edges.setdefault(key(starts[k][1]), set()).add(key(symnum))
    return atoms, edges


CRATE_TOKEN = re.compile(r"(\d+)(core|alloc|hashbrown|foldhash|grasm_rt_abi|grasm_rt|grasm_sys|grasm_fiber|grasm_journal|compiler_builtins)(\d+)?([A-Za-z_][A-Za-z0-9_]*)?")


def crate_module(name, member):
    """The crate a function belongs to, and its first module when that is
    `core`, `alloc` or `hashbrown`: from the mangled name, else the member."""
    if isinstance(name, tuple):
        name = name[1]
    m = CRATE_TOKEN.search(name)
    if m and int(m.group(1)) == len(m.group(2)):
        crate = m.group(2)
        mod = None
        if m.group(3) and m.group(4):
            mod = m.group(4)[:int(m.group(3))]
        return crate, mod
    return crate_of_member(member), None


def compiled(archive, exported):
    atoms = {}
    edges = {}
    for mname, body in members(archive):
        a, e = parse_object(mname, body)
        for k, code, size, member in a:
            atoms.setdefault(k, (code, size, member))
        for k, ts in e.items():
            edges.setdefault(k, set()).update(ts)
    out = {}
    for fam, _ in FAMILIES:
        seen = set()
        work = ["_grasm_" + e for e in exported if family(e) == fam]
        while work:
            k = work.pop()
            if k in seen:
                continue
            seen.add(k)
            work.extend(edges.get(k, ()))
        out[fam] = seen
    return atoms, out


# ----- the source --------------------------------------------------------------------

ITEM = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))? )?(?:unsafe )?(?:extern \"C\" )?(?:const )?"
    r"(fn|struct|enum|impl|const|static|trait|type|macro_rules!|thread_local!|union)\b"
)


def items_of(path):
    """(name, kind, first, last, text, methods) for each item at any module depth."""
    lines = open(path).read().split("\n")
    out = []
    pending = None
    depth = 0
    mods = [0]
    i = 0
    while i < len(lines):
        line = lines[i]
        s = line.strip()
        level = depth == mods[-1]
        if level and (s.startswith("///") or s.startswith("#[") or s.startswith("//!")):
            pending = i if pending is None else pending
            i += 1
            continue
        if level and re.match(r"^(pub )?mod \w+ \{", s):
            depth += 1
            mods.append(depth)
            pending = None
            i += 1
            continue
        inv = re.match(r"^\s*(\w+)!\((grasm_\w+),", line) if level else None
        if inv:
            out.append((inv.group(2), "fn", i + 1, i + 1, inv.group(1) + "(", []))
            pending = None
            i += 1
            continue
        if level and re.match(r'^\s*(?:unsafe )?extern "C" \{', line):
            j = i
            d = 0
            while True:
                d += lines[j].count("{") - lines[j].count("}")
                for f in re.finditer(r"\bfn (\w+)", lines[j]):
                    ln = re.search(r'link_name = "(\w+)"', lines[j - 1])
                    out.append((f.group(1), "foreign", j + 1, j + 1, ln.group(1) if ln else f.group(1), []))
                if d <= 0 and j > i:
                    break
                j += 1
            i = j + 1
            pending = None
            continue
        m = ITEM.match(line) if level else None
        if m:
            kind = m.group(1)
            first = pending if pending is not None else i
            j = i
            d = 0
            opened = False
            while j < len(lines):
                d += lines[j].count("{") - lines[j].count("}")
                opened = opened or "{" in lines[j]
                if (opened and d <= 0) or (not opened and lines[j].rstrip().endswith(";")):
                    break
                j += 1
            text = "\n".join(lines[first:j + 1])
            if kind == "thread_local!":
                for st in re.finditer(r"static (\w+)", text):
                    out.append((st.group(1), "static", first + 1, j + 1, text, []))
            elif kind == "impl":
                im = re.search(r"\bimpl(?:<[^>]*>)?\s+(?:[\w:]+(?:<[^>]*>)?\s+for\s+)?([\w:]+)", line)
                methods = re.findall(r"^\s+(?:pub(?:\([^)]*\))? )?(?:unsafe )?(?:const )?fn (\w+)", text, re.M)
                out.append(((im.group(1) if im else "?").split("::")[-1], "impl", first + 1, j + 1, text, methods))
            else:
                nm = re.search(r"\b(?:fn|struct|enum|const|static|trait|type|union|macro_rules!)\s+(?:mut\s+)?(\w+)", line)
                out.append((nm.group(1) if nm else "?", kind, first + 1, j + 1, text, []))
            pending = None
            i = j + 1
            continue
        if s and not s.startswith("//"):
            pending = None
        depth += line.count("{") - line.count("}")
        while len(mods) > 1 and depth < mods[-1]:
            mods.pop()
        i += 1
    return out


def source(exported):
    items = []
    for path in CRATES:
        items.extend(items_of(path))
    names = {}
    method_owner = {}
    for k, it in enumerate(items):
        if it[1] != "foreign":
            names.setdefault(it[0], []).append(k)
        for mth in it[5]:
            method_owner.setdefault(mth, set()).add(k)
    call = re.compile(r"(?<![.\w])([a-z_][a-z0-9_]*)\s*(?:::<[^>]*>)?\(")
    path_call = re.compile(r"\b([A-Z][A-Za-z0-9_]*)::")
    method = re.compile(r"\.([a-z_][a-z0-9_]*)\s*(?:::<[^>]*>)?\(")
    upper = re.compile(r"\b([A-Z][A-Za-z0-9_]*)\b")
    foreign = {}
    for k, it in enumerate(items):
        if it[1] == "foreign":
            foreign.setdefault(it[0], it[4])
    uses = []
    cfns = []
    for k, it in enumerate(items):
        u = set()
        c = set()
        if it[1] != "foreign":
            t = it[4]
            for nm in set(call.findall(t)):
                hit = [j for j in names.get(nm, []) if items[j][1] in ("fn", "macro_rules!")]
                u.update(hit)
                if not hit and nm in foreign:
                    c.add(foreign[nm])
            for nm in set(path_call.findall(t)) | set(upper.findall(t)):
                u.update(names.get(nm, []))
            for nm in set(method.findall(t)):
                owners = method_owner.get(nm, set())
                if len(owners) == 1:
                    u.update(owners)
            u.discard(k)
        uses.append(u)
        cfns.append(c)
    entry = {}
    for k, it in enumerate(items):
        if it[1] == "fn" and it[0].startswith("grasm_"):
            entry.setdefault(it[0][6:], k)
    reach = {}
    for fam, _ in FAMILIES:
        seen = set()
        work = [entry[e] for e in exported if family(e) == fam and e in entry]
        while work:
            k = work.pop()
            if k in seen:
                continue
            seen.add(k)
            work.extend(uses[k])
        reach[fam] = seen
    return items, entry, reach


def main():
    archive = sys.argv[1]
    names_path = sys.argv[2] if len(sys.argv) > 2 else None
    exported = sorted({n[len("_grasm_"):] for n, b in (
        (l.split()[-1], l) for l in __import__("subprocess").run(
            ["nm", "-g", archive], capture_output=True, text=True).stdout.split("\n") if " T _grasm_" in l)})
    atoms, creach = compiled(archive, exported)
    items, entry, sreach = source(exported)
    owners = {}
    for fam, seen in sreach.items():
        for k in seen:
            owners.setdefault(k, set()).add(fam)
    corpus = {}
    if names_path:
        for line in open(names_path):
            parts = line.split()
            if len(parts) == 3:
                corpus[parts[2]] = corpus.get(parts[2], 0) + int(parts[0])

    def nlines(k):
        it = items[k]
        return it[3] - it[2] + 1

    rows = []
    every = None
    for fam, _ in FAMILIES:
        syms = [e for e in exported if family(e) == fam]
        entry_lines = sum(nlines(entry[e]) for e in syms if e in entry)
        own = sum(nlines(k) for k in sreach[fam] if owners[k] == {fam})
        shared = sum(nlines(k) for k in sreach[fam] if len(owners[k]) > 1)
        bytes_by = {}
        mods = set()
        cfns = set()
        for k in creach[fam]:
            if k in atoms:
                code, size, member = atoms[k]
                if code:
                    crate, mod = crate_module(k, member)
                    bytes_by[crate] = bytes_by.get(crate, 0) + size
                    if crate in ("core", "alloc", "hashbrown") and mod:
                        mods.add(f"{crate}::{mod}")
                    elif crate in ("core", "alloc", "hashbrown"):
                        mods.add(crate)
            elif isinstance(k, str) and not k.startswith("_grasm_"):
                cfns.add(k[1:] if k.startswith("_") else k)
        every = set(cfns) if every is None else every & cfns
        used = sum(1 for e in syms if e in corpus)
        rows.append((fam, syms, used, entry_lines, own, shared, bytes_by, mods, cfns))
    total = sum(sum(1 for _ in open(p)) for p in CRATES)
    print(f"entry points: {len(exported)}; source items: {len(items)}; lines in the five crates: {total}")
    print()
    print("| family | symbols | used by the corpus | Rust lines: entry points / reached only here / shared | machine code reached (KB): ours / core+alloc / hashbrown / builtins | core, alloc, hashbrown modules (besides the common set) | C library (besides the common set) |")
    print("|---|---:|---:|---|---|---|---|")
    common_mods = set.intersection(*(row[7] for row in rows))
    for fam, syms, used, el, own, shared, by, mods, cfns in rows:
        ours = sum(v for c, v in by.items() if c.startswith("grasm"))
        std = by.get("core", 0) + by.get("alloc", 0)
        hb = by.get("hashbrown", 0) + by.get("foldhash", 0)
        cb = by.get("compiler_builtins", 0)
        rest = sorted(cfns - every)
        print(f"| {fam} | {len(syms)} | {used if corpus else '?'} | {el} / {own} / {shared} | {ours / 1024:.1f} / {std / 1024:.1f} / {hb / 1024:.1f} / {cb / 1024:.1f} | {', '.join(sorted(mods - common_mods)) or '--'} | {', '.join(rest) or '--'} |")
    print()
    print(f"the common set of C functions, reached by every family: {', '.join(sorted(every))}")
    print(f"the common set of core/alloc/hashbrown modules, reached by every family: {', '.join(sorted(common_mods))}")
    if corpus:
        print(f"entry points the corpus names: {len([e for e in exported if e in corpus])} of {len(exported)}")
        print(f"never named: {' '.join(e for e in exported if e not in corpus)}")
    un = [e for e in exported if family(e) == "unassigned"]
    if un:
        print(f"unassigned: {' '.join(un)}")
    print()
    for row in rows:
        print(f"{row[0]}: {' '.join(row[1])}")


main()
