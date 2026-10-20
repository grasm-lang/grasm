#!/usr/bin/env python3
"""Fixed-width integers, against an oracle that is not either engine.

`i8 i16 i32 u8 u16 u32` are their own types ([T-INT-FIXED]): every engine
holds the value in 64 bits and wraps each result to the type's width. Before
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
_lib("fresh").fresh_or_stop(GRASM)

KINDS = {"i8": (8, True), "i16": (16, True), "i32": (32, True),
         "u8": (8, False), "u16": (16, False), "u32": (32, False)}


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
    return rng.choice(edges) if rng.random() < 0.4 else rng.randint(lo(k), hi(k))


def lit(v):
    return f"({v})" if v < 0 else str(v)


def program(rng, idx):
    lines, want = [], []
    for case in range(6):
        k = rng.choice(list(KINDS))
        a, b = pick(rng, k), pick(rng, k)
        nz = b if b != 0 else 1
        n = rng.choice([0, 1, 3, 7, 8, 15, 16, 31, 33, 63, 64])
        p = rng.choice([0, 1, 2, 3, 5])
        big = rng.choice([rng.randint(-(1 << 40), 1 << 40), hi(k) + 1, lo(k) - 1, rng.randint(lo(k), hi(k))])
        f = rng.choice([300.7, -300.7, 1e12, -1e12, 0.5, -0.5, 65535.9, float("nan")])
        j = rng.choice(list(KINDS))
        tag = f"c{idx}_{case}"
        lines.append(f"    let a{case}: {k} = {lit(a)}")
        lines.append(f"    let b{case}: {k} = {lit(b)}")
        lines.append(f"    let z{case}: {k} = {lit(nz)}")
        lines.append(f"    var x{case}: {k} = {lit(a)}")
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
            (f"{A}.as_i64()", a),
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
        ]
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
    try:
        for idx in range(o.programs):
            src, want = program(rng, idx)
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
        lines = o.programs * 6 * 32
        print(f"programs: {o.programs}   checked lines: {lines}   disagreed: {bad}   (SEED={o.seed})")
    finally:
        if not o.keep:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
