#!/usr/bin/env python3
"""Raw memory and atomics, against a Python model of the bytes.

[L-LIB-MEM] and [L-LIB-ATOMIC] are a few dozen operations whose whole meaning
is which bytes they read and write, at what width, signed or not, in what
order -- and what an atomic answers. Two engines that share a mistake agree
with each other, so every answer here comes from a `bytearray` per allocation
instead, read and written with `struct` in little-endian.

Five parts, each its own way of being wrong:

1. Random programs: allocations of random sizes and alignments; stores and
   loads of every layout type at aligned offsets; atomics of every width with
   every ordering each may have; addresses kept in a struct field and a list,
   moved with `offset` and measured with `diff`. Each runs under `grasm run`
   and as a native build, both must print the model's lines, and the native
   one must leak nothing. No address's integer is ever printed: the language
   does not say what it is.
2. Misuse, under `grasm run` only ([E-MEM-UNCHECKED]): every kind the sandbox
   refuses -- past the end, after free, misaligned, null, wild, freed twice,
   freed from inside -- each a `test` the model says the exact panic for, and
   accesses that end exactly at an allocation's end, which must pass. A
   compiled build checks none of this, so none of it runs there.
3. `mem_alloc`'s panics, which both engines give word for word.
4. The orderings: every one an operation may have checks, and every other is
   refused by the checker with its sentence.
5. On arm64, the compiled atomics are atomic instructions: a program of
   atomics and the same program with plain loads and stores differ by exactly
   one `ldar`, `stlr`, `cas` or `ldadd` per operation. A single-threaded
   program prints the same either way, so nothing else could notice an
   atomic lowered to a plain access.

    scripts/fuzz_rawmem.py [--programs N] [--seed S] [--keep]
"""

import argparse
import os
import platform
import random
import re
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")

import importlib.util as _ilu


def _lib(name):
    spec = _ilu.spec_from_file_location(
        name, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", f"{name}.py"))
    mod = _ilu.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


leaked = _lib("leaks").leaked
build_native = _lib("inplace").build
_lib("fresh").fresh_or_stop(GRASM)

# name -> (size, struct code); `addr` and `f64` are handled apart.
INTS = {
    "i8": (1, "<b"), "i16": (2, "<h"), "i32": (4, "<i"), "i64": (8, "<q"),
    "u8": (1, "<B"), "u16": (2, "<H"), "u32": (4, "<I"), "u64": (8, "<Q"),
}
SIZE = {**{k: v[0] for k, v in INTS.items()}, "f64": 8, "addr": 8}
# Floats whose shortest text Grasm and Python write the same way.
FLOATS = [0.5, -2.25, 1024.0, 1.5, 100.125, -0.75, 3.0, 65536.5]
ORDERS = ["Relaxed", "Acquire", "Release", "AcqRel", "SeqCst"]
ALLOWS = {
    "load": ["Relaxed", "Acquire", "SeqCst"],
    "store": ["Relaxed", "Release", "SeqCst"],
    "cas": ORDERS,
    "add": ORDERS,
}
ALIGNS = [1, 2, 4, 8, 16, 64, 4096]


def rng_int(rng, t):
    size, code = INTS[t]
    bits = 8 * size
    lo, hi = (-(1 << (bits - 1)), (1 << (bits - 1)) - 1) if code[1].islower() else (0, (1 << bits) - 1)
    edges = [lo, hi, 0, 1, -1, lo + 1, hi - 1]
    v = rng.choice(edges) if rng.random() < 0.4 else rng.randint(lo, hi)
    return max(lo, min(hi, v))


def lit(t, v):
    """A literal of type `t`; a negative one is written so it parses."""
    if t == "f64":
        return repr(float(v))
    return str(v)


class Block:
    def __init__(self, name, size, align):
        self.name, self.size, self.align = name, size, align
        self.data = bytearray(size)
        # Byte offsets whose contents the model cannot predict as numbers:
        # the bytes of a stored address.
        self.opaque = set()
        # offset -> (block name, offset) for each address stored whole.
        self.addrs = {}
        # offset -> float for each f64 stored whole.
        self.floats = {}

    def clear(self, off, n):
        for k in range(off, off + n):
            self.opaque.discard(k)
        for d in (self.addrs, self.floats):
            for o in [o for o in d if o < off + n and off < o + 8]:
                del d[o]

    def known(self, off, n):
        return all(k not in self.opaque for k in range(off, off + n))

    def read(self, t, off):
        return struct.unpack(INTS[t][1], bytes(self.data[off:off + SIZE[t]]))[0]

    def write_int(self, t, off, v):
        size, code = INTS[t]
        bits = 8 * size
        v &= (1 << bits) - 1
        if code[1].islower() and v >= 1 << (bits - 1):
            v -= 1 << bits
        self.clear(off, size)
        self.data[off:off + size] = struct.pack(code, v)


def program(rng, idx):
    lines = [
        "type Cell = { at: addr, n: i64 }",
        "",
        "fn at(p: addr, k: i64) -> addr / {unsafe, mem} {",
        "    unsafe {",
        "        p.offset(k)",
        "    }",
        "}",
        "",
        "fn main() / {io, unsafe, mem} {",
        "    unsafe {",
    ]
    want = []
    blocks = []
    for i in range(rng.randint(2, 4)):
        size = rng.choice([0, 1, 8, 12, 16, 24, 40, 64, rng.randint(1, 96)])
        align = rng.choice(ALIGNS)
        b = Block(f"b{i}", size, align)
        blocks.append(b)
        lines.append(f"        let {b.name} = mem_alloc({size}, {align})")
    # The same addresses, held in a field and in a list.
    lines.append(f"        let cell = Cell{{ at: {blocks[0].name}, n: 0 }}")
    lines.append(f"        let held = [{', '.join(b.name for b in blocks)}]")
    printed = 0

    def place(b, off):
        """A spelling of `b`'s address plus `off`, chosen at random."""
        k = blocks.index(b)
        base = rng.choice([b.name, f"held[{k}]"] + (["cell.at"] if k == 0 else []))
        r = rng.random()
        if off == 0 and r < 0.3:
            return base
        if r < 0.5:
            return f"at({base}, {off})"
        if r < 0.7 and off >= 8:
            return f"{base}.offset({off + 8}).offset(-8)"
        return f"{base}.offset({off})"

    def say(expr, value):
        nonlocal printed
        printed += 1
        lines.append(f'        print("{idx}.{printed} {{{expr}}}")')
        want.append(f"{idx}.{printed} {value}")

    for _ in range(rng.randint(25, 60)):
        b = rng.choice([x for x in blocks if x.size > 0] or blocks)
        if b.size == 0:
            break
        r = rng.random()
        t = rng.choice(list(SIZE))
        n = SIZE[t]
        slots = [o for o in range(0, b.size - n + 1, n)]
        if not slots:
            continue
        off = rng.choice(slots)
        p = place(b, off)
        if r < 0.25:
            # A plain store.
            if t == "addr":
                target = rng.choice(blocks)
                k = rng.randrange(target.size + 1)
                lines.append(f"        {p}.store_addr({target.name}.offset({k}))")
                b.clear(off, 8)
                b.opaque.update(range(off, off + 8))
                b.addrs[off] = (target.name, k)
            elif t == "f64":
                v = rng.choice(FLOATS)
                lines.append(f"        {p}.store_f64({lit(t, v)})")
                b.clear(off, 8)
                b.data[off:off + 8] = struct.pack("<d", v)
                b.floats[off] = v
            else:
                v = rng_int(rng, t)
                lines.append(f"        {p}.store_{t}({lit(t, v)})")
                b.write_int(t, off, v)
        elif r < 0.5:
            # A plain load, of something the model can predict.
            if t == "addr":
                if off in b.addrs:
                    tn, k = b.addrs[off]
                    say(f"{p}.load_addr() == {tn}.offset({k})", "true")
            elif t == "f64":
                if off in b.floats:
                    say(f"{p}.load_f64()", repr(b.floats[off]))
                elif b.known(off, 8) and not any(b.data[off:off + 8]):
                    say(f"{p}.load_f64()", "0.0")
            elif b.known(off, n):
                say(f"{p}.load_{t}()", b.read(t, off))
        elif r < 0.8 and t in INTS and b.known(off, n):
            # An atomic, with an ordering it may have.
            op = rng.choice(["load", "store", "cas", "add"])
            o = rng.choice(ALLOWS[op])
            old = b.read(t, off)
            if op == "load":
                say(f"{p}.atomic_load_{t}({o})", old)
            elif op == "store":
                v = rng_int(rng, t)
                lines.append(f"        {p}.atomic_store_{t}({lit(t, v)}, {o})")
                b.write_int(t, off, v)
            elif op == "cas":
                want_v = old if rng.random() < 0.5 else rng_int(rng, t)
                new = rng_int(rng, t)
                say(f"{p}.atomic_cas_{t}({lit(t, want_v)}, {lit(t, new)}, {o})", old)
                if want_v == old:
                    b.write_int(t, off, new)
            else:
                d = rng_int(rng, t)
                say(f"{p}.atomic_add_{t}({lit(t, d)}, {o})", old)
                b.write_int(t, off, old + d)
        elif r < 0.9:
            # Arithmetic on addresses, which never needs their integers.
            k = rng.randint(-40, 40)
            j = rng.randint(-40, 40)
            say(f"{b.name}.offset({k}).diff({b.name}.offset({j}))", k - j)
        else:
            tt = rng.choice(list(SIZE))
            f = rng.choice(["mem_size_of", "mem_align_of"])
            say(f"{f}({tt})", SIZE[tt])
    for b in blocks:
        lines.append(f"        mem_free({b.name})")
    lines += ["    }", "}"]
    return "\n".join(lines) + "\n", "\n".join(want) + ("\n" if want else "")


def misuse_file(rng, count):
    """Tests that each commit one misuse, with the panic the sandbox gives."""
    lines, want = [], []
    ops_of = lambda t: [f"load_{t}", f"store_{t}"] + (
        [f"atomic_{a}_{t}" for a in ("load", "store", "cas", "add")] if t in INTS else [])

    def call(op, t):
        o = rng.choice(ALLOWS[op.split("_")[1]]) if op.startswith("atomic_") else None
        v = "mem_addr(64)" if t == "addr" else ("1.5" if t == "f64" else "1")
        if op.startswith("load_"):
            return f"let _v = {{p}}.{op}()"
        if op.startswith("store_"):
            return f"{{p}}.{op}({v})"
        kind = op.split("_")[1]
        args = {"load": [o], "store": ["1", o], "cas": ["1", "2", o], "add": ["1", o]}[kind]
        lhs = "" if kind == "store" else "let _v = "
        return f"{lhs}{{p}}.{op}({', '.join(args)})"

    # The edges first, for every type: an access that runs one byte past the
    # end of an allocation is refused, and one that ends inside it is not.
    # Drawn at random these are rare, and a bounds check off by one byte is
    # exactly what they are for.
    for t in SIZE:
        n = SIZE[t]
        size = 2 * n - 1
        for name, off, msg in (
            (f"over_{t}", n, f"of {n} byte(s) at offset {n} of a {size}-byte allocation"),
            (f"last_{t}", 0, None),
        ):
            op = rng.choice(ops_of(t))
            lines += [f'test "{name}" {{', "    unsafe {",
                      f"        let a = mem_alloc({size}, {n})",
                      "        " + call(op, t).replace("{p}", f"a.offset({off})"),
                      "    }", "}", ""]
            want.append((name, f"{op} {msg}" if msg else None))
    for i in range(count):
        t = rng.choice(list(SIZE))
        n = SIZE[t]
        op = rng.choice(ops_of(t))
        kind = rng.choice(["oob", "freed", "misaligned", "null", "wild", "twice", "inside", "end"])
        if kind == "misaligned" and n == 1:
            kind = "oob"
        size = rng.choice([n * k for k in (1, 2, 3, 5)] + [n * 2 + rng.randint(0, n - 1)])
        if kind == "misaligned":
            size = max(size, 2 * n)
        align = rng.choice([a for a in ALIGNS if a >= n] or [n])
        body = [f"let a = mem_alloc({size}, {align})"]
        name = f"m{i}"
        if kind == "oob":
            first = (size - n) // n * n + n if size >= n else 0
            far = [o for o in range(first, size + 64 - n + 1, n)] or [first]
            off = first if rng.random() < 0.5 else rng.choice(far)
            body.append(call(op, t).replace("{p}", f"a.offset({off})"))
            msg = f"{op} of {n} byte(s) at offset {off} of a {size}-byte allocation"
        elif kind == "freed":
            body.append("mem_free(a)")
            body.append(call(op, t).replace("{p}", "a"))
            msg = f"{op} after its allocation was freed"
        elif kind == "misaligned":
            off = rng.choice([o for o in range(1, size - n + 1) if o % n])
            body.append(call(op, t).replace("{p}", f"a.offset({off})"))
            msg = f"{op} at an address not aligned to {n}"
        elif kind == "null":
            body.append(call(op, t).replace("{p}", "mem_addr(0)"))
            msg = f"{op} through the null address"
        elif kind == "wild":
            far = (1 << 46) + rng.randrange(1 << 20) * 8
            body.append(call(op, t).replace("{p}", f"mem_addr({far})"))
            msg = f"{op} through an address no allocation holds"
        elif kind == "twice":
            body += ["mem_free(a)", "mem_free(a)"]
            msg = "mem_free of an allocation already freed"
        elif kind == "inside":
            k = rng.randint(1, max(1, size - 1))
            body.append(f"mem_free(a.offset({k}))")
            msg = "mem_free of an address inside an allocation, not its start"
        else:
            # Ending exactly at the end is in bounds, and must pass.
            off = (size - n) // n * n
            body.append(call(op, t).replace("{p}", f"a.offset({off})"))
            body.append("mem_free(a)")
            msg = None
        lines.append(f'test "{name}" {{')
        lines.append("    unsafe {")
        lines += [f"        {s}" for s in body]
        lines += ["    }", "}", ""]
        want.append((name, msg))
    lines.append("fn main() {\n}")
    return "\n".join(lines) + "\n", want


ALLOC_PANICS = [
    ("mem_alloc(-1, 8)", "mem_alloc: negative size -1"),
    ("mem_alloc(-4096, 16)", "mem_alloc: negative size -4096"),
    ("mem_alloc(8, 0)", "mem_alloc: alignment 0 is not a power of two from 1 to 4096"),
    ("mem_alloc(8, 3)", "mem_alloc: alignment 3 is not a power of two from 1 to 4096"),
    ("mem_alloc(8, 24)", "mem_alloc: alignment 24 is not a power of two from 1 to 4096"),
    ("mem_alloc(8, 8192)", "mem_alloc: alignment 8192 is not a power of two from 1 to 4096"),
    ("mem_alloc(8, -8)", "mem_alloc: alignment -8 is not a power of two from 1 to 4096"),
    ("mem_alloc(1099511627777, 8)",
     "mem_alloc: 1099511627777 bytes is more than an allocation can hold"),
]


def disasm_counts(exe):
    """How many of each atomic instruction family the binary holds."""
    p = subprocess.run(["otool", "-tv", exe], capture_output=True, text=True, timeout=300)
    fams = {"ldar": 0, "stlr": 0, "cas": 0, "ldadd": 0, "ldaxr": 0}
    for line in p.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) < 2:
            continue
        m = parts[1].strip()
        for fam in fams:
            if re.fullmatch(fam + r"[a-z]*", m) and not (fam == "ldar" and m.startswith("ldaxr")):
                fams[fam] += 1
                break
    return fams


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--programs", type=int, default=int(os.environ.get("RAWMEM_PROGRAMS", "60")))
    ap.add_argument("--seed", type=int, default=int(os.environ.get("SEED", "0")) or random.randrange(1 << 30))
    ap.add_argument("--keep", action="store_true")
    o = ap.parse_args()
    rng = random.Random(o.seed)
    work = tempfile.mkdtemp(prefix="grasm_rawmem_")
    bad = 0
    checked = 0
    try:
        path = os.path.join(work, "p.gr")
        exe = os.path.join(work, "p.bin")

        def keep(src, tag):
            if o.keep:
                with open(os.path.join(ROOT, f"rawmem_fail_{tag}.gr"), "w") as fh:
                    fh.write(src)

        # 1. Random programs, both engines, against the model.
        for idx in range(o.programs):
            src, want = program(rng, idx)
            checked += 2 * len(want.splitlines())
            with open(path, "w") as fh:
                fh.write(src)
            r = subprocess.run([GRASM, "run", path], capture_output=True, text=True, timeout=600)
            bp = build_native(GRASM, path, exe, "--native")
            if bp.returncode != 0:
                print(f"program {idx}: the native build failed:\n{(bp.stdout + bp.stderr)[:2000]}")
                bad += 1
                keep(src, idx)
                continue
            n = subprocess.run([exe], capture_output=True, text=True, timeout=600)
            for engine, got in (("interpreter", r), ("native", n)):
                if got.returncode != 0 or got.stdout != want:
                    bad += 1
                    print(f"program {idx}: {engine} disagrees with the model (exit {got.returncode})")
                    wl, gl = want.splitlines(), got.stdout.splitlines()
                    shown = 0
                    for w, g in zip(wl, gl + [""] * len(wl)):
                        if w != g and shown < 4:
                            print(f"    want {w}\n    got  {g}")
                            shown += 1
                    if got.stderr:
                        print("    stderr: " + got.stderr.strip()[:300])
                    keep(src, idx)
            held = leaked(exe)
            if held:
                bad += 1
                print(f"program {idx}: LEAKS {held}")

        # 2. Misuse, in the interpreter's sandbox.
        src, cases = misuse_file(rng, max(40, o.programs * 2))
        with open(path, "w") as fh:
            fh.write(src)
        t = subprocess.run([GRASM, "test", path], capture_output=True, text=True, timeout=600)
        report = t.stdout + t.stderr
        for name, msg in cases:
            checked += 1
            line = next((l for l in report.splitlines() if l.startswith(f'test "{name}" ...')), None)
            expect = f'test "{name}" ... ok' if msg is None else f'test "{name}" ... FAILED: {msg}'
            if line != expect:
                bad += 1
                print(f"misuse {name}: want `{expect}`\n               got  `{line}`")
                keep(src, "misuse")

        # 3. `mem_alloc`'s panics, the same in both engines.
        for call, msg in rng.sample(ALLOC_PANICS, 3):
            checked += 2
            with open(path, "w") as fh:
                fh.write(f"fn main() / {{io, unsafe, mem}} {{\n    print(\"before\")\n"
                         f"    unsafe {{\n        let a = {call}\n    }}\n}}\n")
            r = subprocess.run([GRASM, "run", path], capture_output=True, text=True, timeout=120)
            bp = build_native(GRASM, path, exe, "--native")
            n = subprocess.run([exe], capture_output=True, text=True, timeout=120) if bp.returncode == 0 else bp
            for engine, got in (("interpreter", r), ("native", n)):
                first = (got.stderr.strip().splitlines() or [""])[0]
                if (got.returncode, got.stdout, first) != (101, "before\n", f"panic: {msg}"):
                    bad += 1
                    print(f"{call}: {engine} gave exit {got.returncode}, `{first}`; want `panic: {msg}`")

        # 4. Orderings: the allowed ones check, every other is refused.
        allowed = []
        for ty in INTS:
            for op, os_ in ALLOWS.items():
                for od in os_:
                    args = {"load": [od], "store": ["1", od], "cas": ["1", "2", od], "add": ["1", od]}[op]
                    allowed.append(f"        let _{op}{ty}{od} = a.atomic_{op}_{ty}({', '.join(args)})"
                                   if op != "store" else f"        a.atomic_store_{ty}({', '.join(args)})")
        with open(path, "w") as fh:
            fh.write("fn f(a: addr) / {unsafe, mem} {\n    unsafe {\n" + "\n".join(allowed)
                     + "\n    }\n}\n\nfn main() {\n}\n")
        c = subprocess.run([GRASM, "check", path], capture_output=True, text=True, timeout=120)
        checked += 1
        if c.returncode != 0:
            bad += 1
            print("the orderings every operation may have were refused:\n" + c.stdout[:1500] + c.stderr[:1500])
        for ty in rng.sample(list(INTS), 3):
            for op, od in (("load", "Release"), ("load", "AcqRel"), ("store", "Acquire"), ("store", "AcqRel")):
                call = (f"let _x = a.atomic_load_{ty}({od})" if op == "load"
                        else f"a.atomic_store_{ty}(1, {od})")
                with open(path, "w") as fh:
                    fh.write(f"fn f(a: addr) / {{unsafe, mem}} {{\n    unsafe {{\n        {call}\n    }}\n}}\n\nfn main() {{\n}}\n")
                c = subprocess.run([GRASM, "check", path], capture_output=True, text=True, timeout=120)
                checked += 1
                need = f"an atomic {op} cannot be `{od}`"
                if c.returncode == 0 or need not in c.stdout + c.stderr:
                    bad += 1
                    print(f"`{call}` was not refused with `{need}`")

        # 5. The compiled atomics are atomic instructions.
        if platform.machine() in ("arm64", "aarch64") and shutil.which("otool"):
            ops = []
            for ty in INTS:
                ops += [("load", ty), ("store", ty), ("cas", ty), ("add", ty)]
            def body(atomic):
                out = ["fn main() / {io, unsafe, mem} {", "    unsafe {", "        let a = mem_alloc(64, 8)"]
                for i, (op, ty) in enumerate(ops):
                    p = f"a.offset({8 * (i % 8)})"
                    if atomic:
                        s = {"load": f"print(\"{{{p}.atomic_load_{ty}(Acquire)}}\")",
                             "store": f"{p}.atomic_store_{ty}(1, Release)",
                             "cas": f"print(\"{{{p}.atomic_cas_{ty}(1, 2, AcqRel)}}\")",
                             "add": f"print(\"{{{p}.atomic_add_{ty}(1, SeqCst)}}\")"}[op]
                    else:
                        s = {"load": f"print(\"{{{p}.load_{ty}()}}\")",
                             "store": f"{p}.store_{ty}(1)",
                             "cas": f"print(\"{{{p}.load_{ty}()}}\")",
                             "add": f"print(\"{{{p}.load_{ty}()}}\")"}[op]
                    out.append("        " + s)
                return "\n".join(out + ["        mem_free(a)", "    }", "}"]) + "\n"
            counts = []
            for atomic in (True, False):
                with open(path, "w") as fh:
                    fh.write(body(atomic))
                bp = build_native(GRASM, path, exe, "--native")
                if bp.returncode != 0:
                    bad += 1
                    print("the atomic instruction program did not build:\n" + bp.stderr[:1500])
                    break
                counts.append(disasm_counts(exe))
            if len(counts) == 2:
                diff = {k: counts[0][k] - counts[1][k] for k in counts[0]}
                per = len(INTS)
                want_d = {"ldar": per, "stlr": per, "cas": per, "ldadd": per, "ldaxr": 0}
                # Without LSE, Cranelift writes compare-exchange and add as
                # exclusive-access loops instead; one loop per operation.
                if diff["cas"] == 0 and diff["ldadd"] == 0:
                    want_d = {"ldar": per, "stlr": per, "cas": 0, "ldadd": 0, "ldaxr": 2 * per}
                checked += 1
                if diff != want_d:
                    bad += 1
                    print(f"atomic instructions per family: want {want_d}, the build has {diff}")
        else:
            print("not arm64: the atomic instruction check did not run")

        print(f"programs: {o.programs}   checked lines: {checked}   disagreed: {bad}   (SEED={o.seed})")
    finally:
        if not o.keep:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
