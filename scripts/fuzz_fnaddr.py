#!/usr/bin/env python3
"""Function addresses, `call_c` and `export fn`, against a Python model.

[T-FN-ADDR], [L-LIB-CALL-C] and [T-EXPORT] are about what crosses the C
calling convention: which bits of which argument arrive, at what width,
signed or not, and what comes back. Two engines that share a mistake agree
with each other, so every answer here is computed by the model instead.

Five parts, each its own way of being wrong:

1. Random programs, in both engines: functions with random C signatures over
   every layout type (the eight integer types, `f64`, `addr`) and every
   result, each called directly, through `mem_fn_addr` and `call_c`, through
   an address held in a list, and -- for some -- exported; addresses compared
   for identity; and C functions reached through their addresses. Every
   printed line is the model's, and the native build leaks nothing.
2. C calling Grasm, natively only, since `grasm run` cannot make machine code
   for a Grasm function: `qsort` with a generated comparator over a random
   typed array; each export found back by name with `dlsym`, which must be
   the address `mem_fn_addr` gives; and every narrow parameter reached
   through a wider signature with the bits above its width set, as a C caller
   may leave them -- only the C entry's re-extension gets that right. The
   same program under `grasm run` must refuse the first thing it would hand
   to C, in its exact words.
3. Misuse, under `grasm run` only ([E-MEM-UNCHECKED]): `call_c` by the wrong
   signature, through null, wild or memory addresses; loads, stores and
   `mem_free` through a function's address; a sandbox address handed to C.
   Each a `test` whose panic the model words exactly.
4. The checker: every way a function cannot be reached from C -- generic,
   fallible, `inout`, a type that does not cross, an effect that needs a
   handler -- refused for an address and for an export, with its sentence;
   names the toolchain reserves refused for an export; signatures `call_c`
   cannot take refused; and every layout signature accepted.
5. On arm64, a call through an address is an indirect branch (`blr`), not a
   direct one, and no code pointer is authenticated or stripped: the build
   holds no `blraa`/`braa`/`xpac` instruction, since `arm64` signs none.

    scripts/fuzz_fnaddr.py [--programs N] [--seed S] [--keep]
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

# name -> (bits, signed)
INTS = {
    "i8": (8, True), "i16": (16, True), "i32": (32, True), "i64": (64, True),
    "u8": (8, False), "u16": (16, False), "u32": (32, False), "u64": (64, False),
}
LAYOUT = list(INTS) + ["f64", "addr"]
# Finite, and printed by bits, so formatting is never in question.
FLOATS = [0.5, -2.25, 1024.0, 1.5, 100.125, -0.75, 3.0, -65536.5, 0.0, 7.0e5]


def wrap(t, v):
    bits, signed = INTS[t]
    v &= (1 << bits) - 1
    if signed and v >= 1 << (bits - 1):
        v -= 1 << bits
    return v


def rng_int(rng, t):
    bits, signed = INTS[t]
    lo, hi = (-(1 << (bits - 1)), (1 << (bits - 1)) - 1) if signed else (0, (1 << bits) - 1)
    edges = [lo, hi, 0, 1, -1 if signed else 2, lo + 1, hi - 1]
    v = rng.choice(edges) if rng.random() < 0.4 else rng.randint(lo, hi)
    return max(lo, min(hi, v))


def f64_to_int(x, t):
    """`x.as_t()`: toward zero, held to the type's range."""
    bits, signed = INTS[t]
    lo, hi = (-(1 << (bits - 1)), (1 << (bits - 1)) - 1) if signed else (0, (1 << bits) - 1)
    return max(lo, min(hi, int(x)))


def bits_of(x):
    return struct.unpack("<q", struct.pack("<d", x))[0]


class Fn:
    """A generated function: its signature, its body, and the model of it."""

    def __init__(self, rng, k, export):
        self.k = k
        self.name = f"fz{k}" if not export else f"fzx_{rng.randrange(1 << 30)}_{k}"
        self.export = export
        self.params = [rng.choice(LAYOUT) for _ in range(rng.randint(0, 4))]
        self.ret = rng.choice(LAYOUT + [None])
        self.consts = [rng.randint(0, 90) for _ in range(len(self.params) + 1)]
        self.fconsts = [rng.choice([0.5, 2.0, -1.25, 3.0, 0.125]) for _ in range(len(self.params) + 1)]

    def sig(self):
        r = f" -> {self.ret}" if self.ret else ""
        return f"fn({', '.join(self.params)}){r}"

    def term(self, i, p):
        """Parameter `i`, as the result type, in Grasm."""
        t, R = self.params[i], self.ret
        if R == "f64":
            if t == "f64":
                return f"{p} * {self.fconsts[i + 1]!r}"
            if t == "addr":
                return f"{p}.as_i64().as_f64() * {self.fconsts[i + 1]!r}"
            return f"{p}.as_i64().as_f64() * {self.fconsts[i + 1]!r}"
        R = "i64" if R in (None, "addr") else R
        conv = lambda e: e if R == "i64" else f"{e}.as_{R}()"
        if t == "f64":
            return f"{p}.as_i32().as_i64()" if R == "i64" else f"{p}.as_{R}()"
        if t == "addr":
            return conv(f"{p}.as_i64()")
        if t == R:
            src = p
        elif R == "i64":
            src = f"{p}.as_i64()"
        else:
            src = f"{p}.as_{R}()"
        return f"{src} * {self.consts[i + 1]}"

    def source(self):
        ps = ", ".join(f"p{i}: {t}" for i, t in enumerate(self.params))
        R = self.ret
        head = f"{'export ' if self.export else ''}fn {self.name}({ps})"
        if R:
            head += f" -> {R}"
        head += " / {io, unsafe, mem} {"
        lines = [head, "    unsafe {"]
        if R == "f64":
            acc = [f"{self.fconsts[0]!r}"] + [self.term(i, f"p{i}") for i in range(len(self.params))]
            lines.append("        " + " + ".join(acc))
        else:
            rt = "i64" if R in (None, "addr") else R
            lines.append(f"        let acc: {rt} = {self.consts[0]}")
            terms = ["acc"] + [self.term(i, f"p{i}") for i in range(len(self.params))]
            expr = " + ".join(terms)
            if R is None:
                lines.append(f"        let total = {expr}")
                lines.append(f'        print("u{self.k} {{total}}")')
            elif R == "addr":
                lines.append(f"        mem_addr({expr})")
            else:
                lines.append(f"        {expr}")
        lines += ["    }", "}"]
        return "\n".join(lines)

    def model(self, args):
        """The function's value for `args` (Python values per type)."""
        R = self.ret
        if R == "f64":
            acc = self.fconsts[0]
            for i, (t, a) in enumerate(zip(self.params, args)):
                c = self.fconsts[i + 1]
                if t == "f64":
                    acc = acc + a * c
                elif t == "u64":
                    acc = acc + float(wrap("i64", a)) * c
                else:
                    acc = acc + float(a) * c
            return acc
        rt = "i64" if R in (None, "addr") else R
        acc = self.consts[0]
        for i, (t, a) in enumerate(zip(self.params, args)):
            if t == "f64":
                v = f64_to_int(a, "i32") if rt == "i64" else f64_to_int(a, rt)
                acc += v
            elif t == "addr":
                acc += wrap(rt, a)
            else:
                acc += wrap(rt, a) * self.consts[i + 1] if t != rt else a * self.consts[i + 1]
        return wrap(rt, acc)


def lit(t, v):
    if t == "f64":
        return repr(float(v))
    if t == "addr":
        return f"mem_addr({v})"
    return str(v)


def rand_arg(rng, t):
    if t == "f64":
        return rng.choice(FLOATS)
    if t == "addr":
        return rng.randrange(1 << 40)
    return rng_int(rng, t)


def show(fn, v):
    """How a result is printed, and what the model prints for it."""
    if fn.ret == "f64":
        return bits_of(v)
    return v


def program(rng, idx, fns):
    """Part 1: every function called every way, both engines."""
    lines = []
    for f in fns:
        lines.append(f"type S{f.k} = {f.sig()}")
    lines.append("")
    lines.append("extern fn labs(n: i64) -> i64")
    lines.append("extern fn abs(n: i32) -> i32")
    lines.append("extern fn fabs(x: f64) -> f64")
    lines.append("extern fn strlen(s: str) -> i64")
    lines.append("type CLong = fn(i64) -> i64")
    lines.append("type CInt = fn(i32) -> i32")
    lines.append("type CReal = fn(f64) -> f64")
    lines.append("type CStr = fn(str) -> i64")
    lines.append("")
    for f in fns:
        lines.append(f.source())
        lines.append("")
    lines.append("fn main() / {io, unsafe, mem} {")
    lines.append("    unsafe {")
    want = []
    n = [0]

    def say(expr, value):
        n[0] += 1
        lines.append(f'        print("{idx}.{n[0]} {{{expr}}}")')
        want.append(f"{idx}.{n[0]} {value}")

    lines.append("        let fs = [" + ", ".join(f"mem_fn_addr({f.name})" for f in fns) + "]")
    for f in fns:
        for way in rng.sample(["direct", "addr", "list"], rng.randint(1, 3)):
            args = [rand_arg(rng, t) for t in f.params]
            a = ", ".join(lit(t, v) for t, v in zip(f.params, args))
            call = {
                "direct": f"{f.name}({a})",
                "addr": f"mem_fn_addr({f.name}).call_c(S{f.k}{', ' if a else ''}{a})",
                "list": f"fs[{fns.index(f)}].call_c(S{f.k}{', ' if a else ''}{a})",
            }[way]
            v = f.model(args)
            n[0] += 1
            if f.ret is None:
                lines.append(f"        {call}")
                want.append(f"u{f.k} {v}")
                n[0] -= 1
            elif f.ret == "f64":
                lines.append(f"        let r{idx}_{n[0]} = {call}")
                lines.append(f'        print("{idx}.{n[0]} {{r{idx}_{n[0]}.to_bits()}}")')
                want.append(f"{idx}.{n[0]} {bits_of(v)}")
            elif f.ret == "addr":
                lines.append(f"        let r{idx}_{n[0]} = {call}")
                lines.append(f'        print("{idx}.{n[0]} {{r{idx}_{n[0]}.as_i64()}}")')
                want.append(f"{idx}.{n[0]} {v}")
            else:
                lines.append(f'        print("{idx}.{n[0]} {{{call}}}")')
                want.append(f"{idx}.{n[0]} {v}")
    # Identity: one function, one address; two functions, two.
    for _ in range(3):
        f, g = rng.choice(fns), rng.choice(fns)
        say(f"mem_fn_addr({f.name}) == fs[{fns.index(g)}]", "true" if f is g else "false")
    # C functions through their addresses.
    x = rng.randint(-10 ** 12, 10 ** 12)
    say(f"mem_fn_addr(labs).call_c(CLong, {x})", abs(x))
    y = rng.randint(-(1 << 31) + 1, (1 << 31) - 1)
    lines.append(f"        let i{idx}: i32 = {y}")
    say(f"mem_fn_addr(abs).call_c(CInt, i{idx})", abs(y))
    z = rng.choice(FLOATS)
    say(f"mem_fn_addr(fabs).call_c(CReal, {z!r}).to_bits()", bits_of(abs(z)))
    s = "".join(rng.choice("abcxyz ") for _ in range(rng.randint(0, 12)))
    say(f'mem_fn_addr(strlen).call_c(CStr, "{s}")', len(s))
    lines += ["    }", "}"]
    return "\n".join(lines) + "\n", "\n".join(want) + "\n"


def native_program(rng, idx, fns):
    """Part 2: C calling Grasm. Returns the source, the native output it
    must print, and the panic `grasm run` must give."""
    t = rng.choice(list(INTS))
    bits, signed = INTS[t]
    xs = [rng_int(rng, t) for _ in range(rng.randint(2, 12))]
    desc = rng.random() < 0.5
    size = bits // 8
    lines = [
        "extern fn qsort(base: addr, n: i64, width: i64, cmp: addr)",
        "extern fn dlsym(handle: addr, name: str) -> addr",
        "",
    ]
    for f in fns:
        lines.append(f"type S{f.k} = {f.sig()}")
        wide = [("i64" if p in INTS and p not in ("i64", "u64") else p) for p in f.params]
        r = f" -> {f.ret}" if f.ret else ""
        lines.append(f"type W{f.k} = fn({', '.join(wide)}){r}")
    lines.append("")
    lt, gt = ("1", "-1") if desc else ("-1", "1")
    lines += [
        "fn cmp(a: addr, b: addr) -> i32 / {unsafe, mem} {",
        "    var r: i32 = 0",
        "    unsafe {",
        f"        let x = a.load_{t}()",
        f"        let y = b.load_{t}()",
        "        if x < y {",
        f"            r = {lt}",
        "        } else if x > y {",
        f"            r = {gt}",
        "        }",
        "    }",
        "    r",
        "}",
        "",
    ]
    for f in fns:
        lines.append(f.source())
        lines.append("")
    lines += ["fn main() / {io, unsafe, mem} {", "    unsafe {"]
    lines.append(f"        let base = mem_alloc({size * len(xs)}, {size})")
    for i, v in enumerate(xs):
        lines.append(f"        base.offset({size * i}).store_{t}({v})")
    lines.append(f"        qsort(base, {len(xs)}, {size}, mem_fn_addr(cmp))")
    lines.append(f"        var out: [{t}] = []")
    lines.append(f"        for i in 0..{len(xs)} {{")
    lines.append(f"            out.push(base.offset({size} * i).load_{t}())")
    lines.append("        }")
    lines.append('        print("{out}")')
    lines.append("        mem_free(base)")
    want = [str(sorted(xs, reverse=desc)).replace("'", "")]
    n = 0
    for f in fns:
        args = [rand_arg(rng, p) for p in f.params]
        if f.export:
            n += 1
            a = ", ".join(lit(p, v) for p, v in zip(f.params, args))
            lines.append(f'        let e{n} = dlsym(mem_addr(-2), "{f.name}")')
            lines.append(f'        print("{{e{n} == mem_fn_addr({f.name})}}")')
            want.append("true")
            call = f"e{n}.call_c(S{f.k}{', ' if a else ''}{a})"
            want.append(emit_call(lines, f, call, args, n))
        if any(p in INTS and p not in ("i64", "u64") for p in f.params):
            n += 1
            # Bits above each narrow argument's width, set as a C caller may
            # leave them; the C entry must cut them away.
            dirty = []
            for p, v in zip(f.params, args):
                if p in INTS and p not in ("i64", "u64"):
                    junk = rng.randrange(1, 1 << (63 - INTS[p][0])) << INTS[p][0]
                    dirty.append(str(wrap("i64", (v & ((1 << INTS[p][0]) - 1)) | junk)))
                else:
                    dirty.append(lit(p, v))
            a = ", ".join(dirty)
            call = f"mem_fn_addr({f.name}).call_c(W{f.k}{', ' if a else ''}{a})"
            want.append(emit_call(lines, f, call, args, n))
    lines += ["    }", "}"]
    want = [w for w in want if w is not None]
    panic = "`grasm run` cannot hand C an address from its own sandbox (argument 1 of `qsort`)"
    return "\n".join(lines) + "\n", "\n".join(want) + "\n", panic


def emit_call(lines, f, call, args, n):
    v = f.model(args)
    if f.ret is None:
        lines.append(f"        {call}")
        return f"u{f.k} {v}"
    if f.ret == "f64":
        lines.append(f"        let c{n} = {call}")
        lines.append(f'        print("{{c{n}.to_bits()}}")')
        return str(bits_of(v))
    if f.ret == "addr":
        lines.append(f"        let c{n} = {call}")
        lines.append(f'        print("{{c{n}.as_i64()}}")')
        return str(v)
    lines.append(f'        print("{{{call}}}")')
    return str(v)


def misuse_file(rng, count):
    """Part 3: the interpreter's checks, as tests."""
    lines = [
        "extern fn free(p: addr)",
        "extern fn labs(n: i64) -> i64",
        "",
        "fn one(n: i64) -> i64 {",
        "    n",
        "}",
        "",
        "fn pair(a: i8, b: f64) -> u32 {",
        "    7",
        "}",
        "",
    ]
    sigs = ["fn(i64) -> i64", "fn(i8, f64) -> u32", "fn(i64, i64) -> i64", "fn(i8, f64) -> i32",
            "fn(i8) -> u32", "fn(f64, i8) -> u32", "fn()", "fn(i64)", "fn(u8, f64) -> u32"]
    show_sig = {
        "fn(i64) -> i64": "(i64) -> i64", "fn(i8, f64) -> u32": "(i8, f64) -> u32",
        "fn(i64, i64) -> i64": "(i64, i64) -> i64", "fn(i8, f64) -> i32": "(i8, f64) -> i32",
        "fn(i8) -> u32": "(i8) -> u32", "fn(f64, i8) -> u32": "(f64, i8) -> u32",
        "fn()": "()", "fn(i64)": "(i64)", "fn(u8, f64) -> u32": "(u8, f64) -> u32",
    }
    own = {"one": "fn(i64) -> i64", "pair": "fn(i8, f64) -> u32"}
    vals = {"i64": "1", "i8": "1", "f64": "1.0", "u8": "1"}
    for i, s in enumerate(sigs):
        lines.append(f"type M{i} = {s}")
    lines.append("")
    want = []

    def args_for(s):
        inner = s[s.index("(") + 1:s.index(")")]
        return [vals[p.strip()] for p in inner.split(",") if p.strip()]

    for i in range(count):
        kind = rng.choice(["sig", "null", "wild", "memory", "load", "store", "free", "toc", "ok"])
        name = f"m{i}"
        body = []
        if kind in ("sig", "ok"):
            f = rng.choice(list(own))
            s = own[f] if kind == "ok" else rng.choice([x for x in sigs if x != own[f]])
            k = sigs.index(s)
            a = ", ".join(args_for(s))
            body.append(f"let _x = mem_fn_addr({f}).call_c(M{k}{', ' if a else ''}{a})")
            msg = None if kind == "ok" else (
                f"call_c through the address of `{f}`, which is {show_sig[own[f]]}, as {show_sig[s]}")
        elif kind in ("null", "wild", "memory"):
            s = rng.choice(sigs)
            k = sigs.index(s)
            a = ", ".join(args_for(s))
            at = {"null": "mem_addr(0)", "wild": f"mem_addr({(1 << 44) + rng.randrange(1 << 20)})",
                  "memory": f"mem_alloc({rng.choice([8, 16, 64])}, 8)"}[kind]
            body.append(f"let _x = {at}.call_c(M{k}{', ' if a else ''}{a})")
            msg = ("call_c through the null address" if kind == "null"
                   else "call_c through an address that holds no function")
        elif kind in ("load", "store"):
            t = rng.choice(LAYOUT)
            f = rng.choice(list(own))
            if kind == "load":
                body.append(f"let _x = mem_fn_addr({f}).load_{t}()")
            else:
                body.append(f"mem_fn_addr({f}).store_{t}({lit(t, 0) if t != 'addr' else 'mem_addr(8)'})")
            msg = f"{kind}_{t} through the address of a function"
        elif kind == "free":
            body.append(f"mem_free(mem_fn_addr({rng.choice(list(own))}))")
            msg = "mem_free of the address of a function"
        else:
            at = rng.choice([f"mem_alloc({rng.choice([1, 8, 40])}, 8)", "mem_fn_addr(one)"])
            body.append(f"free({at})")
            msg = "`grasm run` cannot hand C an address from its own sandbox (argument 1 of `free`)"
        lines.append(f'test "{name}" {{')
        lines.append("    unsafe {")
        lines += [f"        {b}" for b in body]
        lines += ["    }", "}", ""]
        want.append((name, msg))
    lines.append("fn main() {\n}")
    return "\n".join(lines) + "\n", want


# Part 4: what the checker refuses, with its sentence.
BAD_FNS = [
    ("fn bad[T](x: T) -> T {\n    x\n}", "generic function `bad` cannot {what}"),
    ("fn bad(n: i64) -> i64 ! str {\n    n\n}", "fallible function `bad` cannot {what}"),
    ("fn bad(inout n: i64) {\n    n += 1\n}", "`bad` cannot {what}: it has an `inout` or `own` parameter"),
    ("fn bad(s: str) -> i64 {\n    1\n}", "`bad` cannot {what}: `str` does not cross to C"),
    ("fn bad(b: bool) -> i64 {\n    1\n}", "`bad` cannot {what}: `bool` does not cross to C"),
    ("fn bad(n: i64) -> [i64] {\n    [n]\n}", "`bad` cannot {what}: `[i64]` does not cross to C"),
    ("fn bad(n: i64) -> Option[i64] {\n    Some(n)\n}", "`bad` cannot {what}: `Option[i64]` does not cross to C"),
    ("fn bad(n: i64) -> i64 {\n    Ask.ask()\n}", "`bad` cannot {what}: it performs `Ask`"),
    ("fn bad(n: i64) -> i64 / {async} {\n    let c: Chan[i64] = chan_new()\n    chan_send(c, n)\n    n\n}",
     "`bad` cannot {what}: it performs `async`"),
]
BAD_SIGS = [
    ("fn(i64) -> str", "`str` has no C representation"),
    ("fn(bool)", "`bool` has no C representation"),
    ("fn([i64]) -> i64", "`[i64]` has no C representation"),
    ("fn(i64) -> bool", "`bool` has no C representation"),
]
RESERVED = ["main", "grasm_go", "g_main_x", "gc_y", "gp_z", "gpf_w", "gv_v", "gx_u"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--programs", type=int, default=int(os.environ.get("FNADDR_PROGRAMS", "40")))
    ap.add_argument("--seed", type=int, default=int(os.environ.get("SEED", "0")) or random.randrange(1 << 30))
    ap.add_argument("--keep", action="store_true")
    o = ap.parse_args()
    rng = random.Random(o.seed)
    work = tempfile.mkdtemp(prefix="grasm_fnaddr_")
    bad = 0
    checked = 0
    try:
        path = os.path.join(work, "p.gr")
        exe = os.path.join(work, "p.bin")

        def keep(src, tag):
            if o.keep:
                with open(os.path.join(ROOT, f"fnaddr_fail_{tag}.gr"), "w") as fh:
                    fh.write(src)

        def compare(engine, got, want, label, src):
            nonlocal bad
            if got.returncode != 0 or got.stdout != want:
                bad += 1
                print(f"{label}: {engine} disagrees with the model (exit {got.returncode})")
                wl, gl = want.splitlines(), got.stdout.splitlines()
                shown = 0
                for w, g in zip(wl, gl + [""] * len(wl)):
                    if w != g and shown < 4:
                        print(f"    want {w}\n    got  {g}")
                        shown += 1
                if got.stderr:
                    print("    stderr: " + got.stderr.strip()[:400])
                keep(src, label.replace(" ", "_"))

        for idx in range(o.programs):
            fns = [Fn(rng, k, rng.random() < 0.3) for k in range(rng.randint(2, 6))]
            # 1. Both engines, against the model.
            src, want = program(rng, idx, fns)
            checked += 2 * len(want.splitlines())
            with open(path, "w") as fh:
                fh.write(src)
            r = subprocess.run([GRASM, "run", path], capture_output=True, text=True, timeout=600)
            compare("interpreter", r, want, f"program {idx}", src)
            bp = build_native(GRASM, path, exe, "--native")
            if bp.returncode != 0:
                print(f"program {idx}: the native build failed:\n{(bp.stdout + bp.stderr)[:2000]}")
                bad += 1
                keep(src, idx)
                continue
            n = subprocess.run([exe], capture_output=True, text=True, timeout=600)
            compare("native", n, want, f"program {idx}", src)
            held = leaked(exe)
            if held:
                bad += 1
                print(f"program {idx}: LEAKS {held}")

            # 2. C calling Grasm: natively, and refused by `grasm run`.
            src, want, panic = native_program(rng, idx, fns)
            checked += len(want.splitlines()) + 1
            with open(path, "w") as fh:
                fh.write(src)
            bp = build_native(GRASM, path, exe, "--native")
            if bp.returncode != 0:
                print(f"native {idx}: the build failed:\n{(bp.stdout + bp.stderr)[:2000]}")
                bad += 1
                keep(src, f"native{idx}")
                continue
            n = subprocess.run([exe], capture_output=True, text=True, timeout=600)
            compare("native", n, want, f"C calling Grasm {idx}", src)
            r = subprocess.run([GRASM, "run", path], capture_output=True, text=True, timeout=600)
            first = (r.stderr.strip().splitlines() or [""])[0]
            if r.returncode != 101 or panic not in first:
                bad += 1
                print(f"C calling Grasm {idx}: `grasm run` gave exit {r.returncode}, `{first}`; want `{panic}`")
                keep(src, f"native{idx}")

        # 3. Misuse, in the interpreter.
        src, cases = misuse_file(rng, max(30, o.programs))
        with open(path, "w") as fh:
            fh.write(src)
        t = subprocess.run([GRASM, "test", path], capture_output=True, text=True, timeout=600)
        report = t.stdout + t.stderr
        for name, msg in cases:
            checked += 1
            line = next((l for l in report.splitlines() if l.startswith(f'test "{name}" ...')), None)
            ok = line == f'test "{name}" ... ok' if msg is None else (
                line is not None and line.startswith(f'test "{name}" ... FAILED: {msg}'))
            if not ok:
                bad += 1
                print(f"misuse {name}: want `{msg or 'ok'}`\n               got  `{line}`")
                keep(src, "misuse")

        # 4. The checker.
        effect = "effect Ask {\n    fn ask() -> i64\n}\n\n"
        for decl, need in BAD_FNS:
            for what, use in (("have its address taken",
                               "fn user() / {unsafe, mem} {\n    unsafe {\n        let a = mem_fn_addr(bad)\n    }\n}\n"),
                              ("be exported", None)):
                d = ("export " + decl) if use is None else decl
                src = effect + d + "\n\n" + (use or "") + "\nfn main() {\n}\n"
                with open(path, "w") as fh:
                    fh.write(src)
                c = subprocess.run([GRASM, "check", path], capture_output=True, text=True, timeout=120)
                checked += 1
                sentence = need.format(what=what)
                if c.returncode == 0 or sentence not in c.stdout + c.stderr:
                    bad += 1
                    print(f"not refused with `{sentence}`:\n{src}\n{(c.stdout + c.stderr)[:600]}")
        for sig, need in BAD_SIGS:
            src = f"type Sig = {sig}\n\nfn f(a: addr) / {{io, unsafe, mem}} {{\n    unsafe {{\n        let x = 1\n    }}\n}}\n\nfn g(a: addr) / {{io, unsafe, mem}} {{\n    unsafe {{\n        a.call_c(Sig{', 1' if '(i64' in sig or '([' in sig else (', true' if '(bool' in sig else '')})\n    }}\n}}\n\nfn main() {{\n}}\n"
            with open(path, "w") as fh:
                fh.write(src)
            c = subprocess.run([GRASM, "check", path], capture_output=True, text=True, timeout=120)
            checked += 1
            if c.returncode == 0 or need not in c.stdout + c.stderr:
                bad += 1
                print(f"`call_c` through `{sig}` not refused with `{need}`:\n{(c.stdout + c.stderr)[:600]}")
        for name in rng.sample(RESERVED, 4):
            with open(path, "w") as fh:
                fh.write(f"export fn {name}() {{\n}}\n\nfn other() {{\n}}\n")
            c = subprocess.run([GRASM, "check", path], capture_output=True, text=True, timeout=120)
            checked += 1
            need = f"`{name}` cannot be exported: the toolchain gives that name to its own code"
            if c.returncode == 0 or need not in c.stdout + c.stderr:
                bad += 1
                print(f"`export fn {name}` not refused with `{need}`")
        # Every layout type, in and out, crosses.
        ok = [f"export fn okx_{t}(a: {t}, b: {t}) -> {t} {{\n    a\n}}" for t in LAYOUT]
        with open(path, "w") as fh:
            fh.write("\n\n".join(ok) + "\n\nfn main() {\n}\n")
        c = subprocess.run([GRASM, "check", path], capture_output=True, text=True, timeout=120)
        checked += 1
        if c.returncode != 0:
            bad += 1
            print("a layout signature was refused:\n" + (c.stdout + c.stderr)[:1500])

        # 5. Indirect calls, and no pointer authentication.
        if platform.machine() in ("arm64", "aarch64") and shutil.which("otool"):
            body = ("type U = fn(i64) -> i64\n\nfn one(n: i64) -> i64 {\n    n + 1\n}\n\n"
                    "fn main() / {io, unsafe, mem} {\n    unsafe {\n        let a = mem_fn_addr(one)\n"
                    + "".join(f"        print(\"{{a.call_c(U, {k})}}\")\n" for k in range(5))
                    + "    }\n}\n")
            with open(path, "w") as fh:
                fh.write(body)
            bp = build_native(GRASM, path, exe, "--native")
            if bp.returncode != 0:
                bad += 1
                print("the indirect call program did not build:\n" + bp.stderr[:1500])
            else:
                p = subprocess.run(["otool", "-tv", exe], capture_output=True, text=True, timeout=300)
                mnem = [l.split("\t")[1].strip() for l in p.stdout.splitlines() if "\t" in l]
                blr = sum(1 for m in mnem if m == "blr")
                # Return addresses are signed with the B key, as Cranelift
                # does for every function on macOS; that must survive.
                signed = sum(1 for m in mnem if m == "pacibsp")
                # A code pointer on `arm64` is not signed, so authenticating
                # one would fault, and stripping one would mean something was.
                odd = [m for m in mnem if re.fullmatch(r"(blra|braa|brab|xpac)[a-z]*", m)]
                checked += 1
                if blr < 5 or signed == 0 or odd:
                    bad += 1
                    print(f"want at least 5 `blr`, return addresses signed, and no code pointer "
                          f"authenticated or stripped; the build has {blr} `blr`, {signed} `pacibsp`, {odd[:5]}")
        else:
            print("not arm64: the indirect call check did not run")

        print(f"programs: {o.programs}   checked lines: {checked}   disagreed: {bad}   (SEED={o.seed})")
    finally:
        if not o.keep:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
