#!/usr/bin/env python3
"""Fixed-width integers, against an oracle that is not either engine.

`i8 i16 i32 u8 u16 u32 u64` are their own types ([T-INT-FIXED], [T-U64]):
every engine holds the value in 64 bits and wraps each result to the type's
width. `u64` is the width whose bits are not its value read as an `i64`, so
it also has to divide, compare, convert and print unsigned. Before
they were, the names were spellings of `i64`, and `let a: u8 = 255; a + 1`
printed 256 in both engines -- which agreed, so the agreement gates saw
nothing. Two engines agreeing is not evidence that either is right, so the
answers here come from Python, which computes each one exactly and then
wraps it by the rule as written:

  +  -  *  ^  neg  abs  shl     the exact result, wrapped to the width
  /  %                          truncating; `MIN / -1` wraps to MIN
  shr                           arithmetic, on the value
  ushr                          logical, on the value's bits at its width
  bit_and/or/xor/not            on the bits, wrapped
  as_T                          wrapped to T; from f64, toward zero and held
                                to T's range, NaN to 0
  try_T                         Some when it fits in T
  checked_op / saturating_op    the exact result: None / held to the range
  x op= y                       the same as x = x op y
  as_i64 / try_i64              the bits read signed / Some when it fits
  to_base(16), parse_u64        u64 only: unsigned digits, and back
  as_f64().as_u64()             u64 only: the nearest double, held to range
  i64 ^ e                       every bit of e, however large
  rotate_left/right(n)          the bits at the width, turned by n mod width
  count_ones, leading_zeros,    the bits at the width; zero counts the width
  trailing_zeros                ([L-LIB-BITS-WIDTH])
  swap_bytes                    the width's bytes reversed, read back

`i64` is one of the kinds as well, so its own answers are held to the oracle
rather than only to the other engine.

A `u64` literal past 2^63 is written in hex or in decimal at random, since
both reach the checker by different roads.

Each program is run by `grasm run` and by a native build, both must print
the oracle's lines, and the native one must leak nothing.

    scripts/fuzz_intwidth.py [--programs N] [--seed S] [--keep]
"""

import argparse
import math
import os
import random
import shutil
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
# The Grasm backend (compiler/s3), held to the Cranelift build on each program.
S3Engine = _lib("s3engine").Engine
_lib("fresh").fresh_or_stop(GRASM)

KINDS = {"i8": (8, True), "i16": (16, True), "i32": (32, True),
         "u8": (8, False), "u16": (16, False), "u32": (32, False),
         "u64": (64, False), "i64": (64, True)}


def lo(k):
    bits, signed = KINDS[k]
    return -(1 << (bits - 1)) if signed else 0


def hi(k):
    bits, signed = KINDS[k]
    return (1 << (bits - 1)) - 1 if signed else (1 << bits) - 1


def wrap(k, v):
    bits, signed = KINDS[k]
    v &= (1 << bits) - 1
    if signed and v >= 1 << (bits - 1):
        v -= 1 << bits
    return v


def tdiv(a, b):
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b >= 0) else -q


def opt(v):
    return "None" if v is None else f"Some({v})"


def pick(rng, k):
    # The ends of the range, the values next to them, zero, and anything.
    edges = [lo(k), lo(k) + 1, hi(k), hi(k) - 1, 0, 1, -1 if KINDS[k][1] else 2]
    if k == "u64":
        # Where reading the bits as an `i64` changes sign.
        edges += [1 << 63, (1 << 63) - 1, (1 << 63) + 1]
    return rng.choice(edges) if rng.random() < 0.4 else rng.randint(lo(k), hi(k))


def lit(v, rng=None):
    if v < 0:
        return f"({v})"
    # Past `i64` a `u64` may be written either way ([T-U64]).
    if v >= 1 << 63 and rng is not None and rng.random() < 0.5:
        return hex(v)
    return str(v)


def rot(k, v, n):
    w = KINDS[k][0]
    x = v & ((1 << w) - 1)
    n %= w
    return wrap(k, ((x << n) | (x >> (w - n))) if n else x)


def swap(k, v):
    w = KINDS[k][0]
    x = v & ((1 << w) - 1)
    return wrap(k, int.from_bytes(x.to_bytes(w // 8, "big"), "little"))


def signed64(v):
    v &= (1 << 64) - 1
    return v - (1 << 64) if v >= 1 << 63 else v


def program(rng, idx):
    lines, want = [], []
    for case in range(6):
        k = rng.choice(list(KINDS))
        a, b = pick(rng, k), pick(rng, k)
        nz = b if b != 0 else 1
        n = rng.choice([0, 1, 3, 7, 8, 15, 16, 31, 33, 63, 64])
        p = rng.choice([0, 1, 2, 3, 5])
        big = rng.choice([rng.randint(-(1 << 40), 1 << 40), hi(k) + 1, lo(k) - 1, rng.randint(lo(k), hi(k))])
        # `big` is an `i64`, which has no room for the top of a `u64`.
        big = max(-(1 << 63), min((1 << 63) - 1, big))
        f = rng.choice([300.7, -300.7, 1e12, -1e12, 0.5, -0.5, 65535.9, float("nan"),
                        1e19, 2e19, -1e19])
        j = rng.choice(list(KINDS))
        tag = f"c{idx}_{case}"
        lines.append(f"    let a{case}: {k} = {lit(a, rng)}")
        lines.append(f"    let b{case}: {k} = {lit(b, rng)}")
        lines.append(f"    let z{case}: {k} = {lit(nz, rng)}")
        lines.append(f"    var x{case}: {k} = {lit(a, rng)}")
        lines.append(f"    let big{case}: i64 = {lit(big)}")
        fl = "0.0 / 0.0" if math.isnan(f) else repr(f)
        lines.append(f"    let f{case}: f64 = {fl}")
        A, B, Z = f"a{case}", f"b{case}", f"z{case}"
        exprs = [
            (f"{A} + {B}", wrap(k, a + b)),
            (f"{A} - {B}", wrap(k, a - b)),
            (f"{A} * {B}", wrap(k, a * b)),
            (f"{A} / {Z}", wrap(k, tdiv(a, nz))),
            (f"{A} % {Z}", wrap(k, a - nz * tdiv(a, nz))),
            (f"{A} ^ {p}", wrap(k, a ** p)),
            (f"-{A}", wrap(k, -a)),
            (f"{A}.abs()", wrap(k, abs(a))),
            (f"{A}.min({B})", min(a, b)),
            (f"{A}.max({B})", max(a, b)),
            (f"{A}.bit_and({B})", wrap(k, a & b)),
            (f"{A}.bit_or({B})", wrap(k, a | b)),
            (f"{A}.bit_xor({B})", wrap(k, a ^ b)),
            (f"{A}.bit_not()", wrap(k, ~a)),
            (f"{A}.shl({n})", wrap(k, a << (n & 63))),
            (f"{A}.shr({n})", wrap(k, a >> (n & 63))),
            (f"{A}.ushr({n})", wrap(k, (a & ((1 << KINDS[k][0]) - 1)) >> (n & 63))),
            (f"{A}.as_{j}()", wrap(j, a)),
            (f"{A}.as_i64()", signed64(a)),
            (f"{A}.try_i64()", opt(a if a < 1 << 63 else None)),
            (f"big{case}.as_{k}()", wrap(k, big)),
            (f"big{case}.try_{k}()", opt(big if lo(k) <= big <= hi(k) else None)),
            (f"f{case}.as_{k}()", 0 if math.isnan(f) else max(lo(k), min(hi(k), math.trunc(f)))),
            (f"{A}.checked_add({B})", opt(a + b if lo(k) <= a + b <= hi(k) else None)),
            (f"{A}.checked_sub({B})", opt(a - b if lo(k) <= a - b <= hi(k) else None)),
            (f"{A}.checked_mul({B})", opt(a * b if lo(k) <= a * b <= hi(k) else None)),
            (f"{A}.saturating_add({B})", max(lo(k), min(hi(k), a + b))),
            (f"{A}.saturating_sub({B})", max(lo(k), min(hi(k), a - b))),
            (f"{A}.saturating_mul({B})", max(lo(k), min(hi(k), a * b))),
            (f"{A} < {B}", "true" if a < b else "false"),
            (f"{A} == {B}", "true" if a == b else "false"),
            (f"[{A}, {B}, {Z}].sum()", wrap(k, a + b + nz)),
            (f"[{A}, {B}, {Z}].max()", opt(max(a, b, nz))),
        ]
        w = KINDS[k][0]
        x = a & ((1 << w) - 1)
        r = rng.choice([0, 1, 3, 7, 8, 15, 31, 33, 63, 64, 65, -1, -9, rng.randint(-200, 200)])
        exprs += [
            (f"{A}.rotate_left({lit(r)})", rot(k, a, r)),
            (f"{A}.rotate_right({lit(r)})", rot(k, a, -r)),
            (f"{A}.count_ones()", bin(x).count("1")),
            (f"{A}.leading_zeros()", w - x.bit_length()),
            (f"{A}.trailing_zeros()", w if x == 0 else (x & -x).bit_length() - 1),
            (f"{A}.swap_bytes()", swap(k, a)),
        ]
        if k == "u64":
            exprs += [
                (f"{A}.to_base(16)", format(a, "x")),
                (f'"{a}".parse_u64()', opt(a)),
                (f'"{format(a, "x")}".parse_u64_base(16)', opt(a)),
                (f"{A}.as_f64().as_u64()", min(int(float(a)), hi(k))),
                (f"{A}.cmp({B})", (a > b) - (a < b)),
            ]
        # `i64`'s `^` with every bit of a large exponent ([E-ARITH-INT]).
        base, ex = rng.randint(-50, 50), rng.choice([rng.randint(0, 70), rng.randint(1 << 32, 1 << 40)])
        exprs.append((f"{lit(base)} ^ {ex}", signed64(pow(base, ex, 1 << 64))))
        for i, (e, v) in enumerate(exprs):
            lines.append(f'    print("{tag}.{i} {{{e}}}")')
            want.append(f"{tag}.{i} {v}")
        x = a
        for op, sym in [(rng.choice("+-*"), None) for _ in range(3)]:
            lines.append(f"    x{case} {op}= {B}")
            x = wrap(k, eval(f"x {op} b"))
        lines.append(f'    print("{tag}.x {{x{case}}}")')
        want.append(f"{tag}.x {x}")
    src = "fn main() {\n" + "\n".join(lines) + "\n}\n"
    return src, "\n".join(want) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--programs", type=int, default=int(os.environ.get("INTWIDTH_PROGRAMS", "40")))
    ap.add_argument("--seed", type=int, default=int(os.environ.get("SEED", "0")) or random.randrange(1 << 30))
    ap.add_argument("--keep", action="store_true")
    o = ap.parse_args()
    rng = random.Random(o.seed)
    work = tempfile.mkdtemp(prefix="grasm_intwidth_")
    bad = 0
    checked = 0
    s3 = S3Engine(GRASM, work)
    try:
        for idx in range(o.programs):
            src, want = program(rng, idx)
            checked += 2 * len(want.splitlines())
            path = os.path.join(work, "p.gr")
            exe = os.path.join(work, "p.bin")
            with open(path, "w") as fh:
                fh.write(src)
            r = subprocess.run([GRASM, "run", path], capture_output=True, text=True, timeout=600)
            bp = build_native(GRASM, path, exe, "--native")
            if bp.returncode != 0:
                print(f"program {idx}: the native build failed:\n{(bp.stdout + bp.stderr)[:2000]}")
                bad += 1
                continue
            n = subprocess.run([exe], capture_output=True, text=True, timeout=600)
            s3.check(path, exe, f"program {idx} (SEED={o.seed})", interp=(r.returncode, r.stdout))
            for engine, got in (("interpreter", r), ("native", n)):
                if got.returncode != 0 or got.stdout != want:
                    bad += 1
                    print(f"program {idx}: {engine} disagrees with the oracle (exit {got.returncode})")
                    wl, gl = want.splitlines(), got.stdout.splitlines()
                    shown = 0
                    for w, g in zip(wl, gl + [""] * len(wl)):
                        if w != g and shown < 6:
                            print(f"    want {w}\n    got  {g}")
                            shown += 1
                    if got.stderr:
                        print("    stderr: " + got.stderr.strip()[:300])
                    if o.keep:
                        shutil.copy(path, os.path.join(ROOT, f"intwidth_fail_{idx}.gr"))
            held = leaked(exe)
            if held:
                bad += 1
                print(f"program {idx}: LEAKS {held}")
        if not s3.report():
            bad += 1
        print(f"programs: {o.programs}   checked lines: {checked}   disagreed: {bad}   (SEED={o.seed})")
    finally:
        if not o.keep:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
