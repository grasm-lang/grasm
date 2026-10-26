#!/usr/bin/env python3
"""Tuple map keys, against a Python dict.

[T-MAP-KEY] lets a key be a tuple of key types, and [E-MAP-ORDER] says such
keys iterate component by component, each by its own order. What can go
wrong is where two keys meet: a hash or an encoding that lets `("ab", "c")`
and `("a", "bc")` be one key, an order that compares the wrong component or
compares a `u64` signed, a probe that finds an entry it should not. Two
engines that share a mistake agree with each other, so every answer comes
from a Python dict sorted by the same rule.

Two parts:

1. Random programs, in both engines: random tuple key types two and three
   wide, nested once, over every key type; keys drawn from small pools so
   that they collide often and differ only inside one component; sets,
   deletes, gets, `has`, `len`, `keys`, `values`, `entries`, map literals,
   equality and printing, every answer the model's. The native build leaks
   nothing.
2. The checker: a component with no order -- `f64`, a list, a struct, unit --
   refused with its sentence, and a type parameter inside a tuple key held to
   the rule where the function is called.

    scripts/fuzz_mapkeys.py [--programs N] [--seed S] [--keep]
"""

import argparse
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

INTS = {"i64": (64, True), "i8": (8, True), "i32": (32, True), "u8": (8, False),
        "u32": (32, False), "u64": (64, False)}
SCALARS = list(INTS) + ["str", "bool"]
# Strings that begin one another and concatenate alike, and characters past
# ASCII, which is where a byte order and a code point order part ways.
STRS = ["", "a", "ab", "b", "abc", "c", "bc", "é", "ß", "zz", "a b"]


def rand_type(rng, depth=0):
    if depth < 1 and rng.random() < 0.3:
        return ("tuple", [rand_type(rng, depth + 1) for _ in range(rng.randint(2, 3))])
    return rng.choice(SCALARS)


def show_type(t):
    if isinstance(t, tuple):
        return "(" + ", ".join(show_type(x) for x in t[1]) + ")"
    return t


def int_pool(rng, t):
    bits, signed = INTS[t]
    lo, hi = (-(1 << (bits - 1)), (1 << (bits - 1)) - 1) if signed else (0, (1 << bits) - 1)
    pool = {lo, hi, 0, 1, max(lo, -1), hi // 2}
    while len(pool) < 7:
        pool.add(rng.randint(lo, hi))
    return sorted(pool)


def pools(rng, t):
    if isinstance(t, tuple):
        return [pools(rng, x) for x in t[1]]
    if t == "str":
        return rng.sample(STRS, rng.randint(3, 6))
    if t == "bool":
        return [False, True]
    return rng.sample(int_pool(rng, t), rng.randint(3, 5))


def draw(rng, t, pool):
    if isinstance(t, tuple):
        return tuple(draw(rng, x, p) for x, p in zip(t[1], pool))
    return rng.choice(pool)


def lit(t, v):
    if isinstance(t, tuple):
        return "(" + ", ".join(lit(x, y) for x, y in zip(t[1], v)) + ")"
    if t == "str":
        return '"' + v + '"'
    if t == "bool":
        return "true" if v else "false"
    return str(v)


def shown(t, v):
    """How a key prints: a tuple's strings bare, as `print` writes them."""
    if isinstance(t, tuple):
        return "(" + ", ".join(shown(x, y) for x, y in zip(t[1], v)) + ")"
    if t == "bool":
        return "true" if v else "false"
    return str(v)


def program(rng, idx):
    kt = ("tuple", [rand_type(rng, 1) for _ in range(rng.randint(2, 3))])
    pool = pools(rng, kt)
    T = show_type(kt)
    lines = ["fn main() {"]
    want = []
    n = [0]

    def say(expr, value):
        n[0] += 1
        lines.append(f'    print("{idx}.{n[0]} {{{expr}}}")')
        want.append(f"{idx}.{n[0]} {value}")

    model = {}
    seed = [(draw(rng, kt, pool), rng.randint(-50, 50)) for _ in range(rng.randint(0, 4))]
    for k, v in seed:
        model[k] = v
    lines.append(f"    var m: Map[{T}, i64] = " + ("{" + ", ".join(f"{lit(kt, k)}: {v}" for k, v in seed) + "}"
                                                   if seed else "{}"))
    order = lambda: sorted(model)

    def render():
        return "{" + ", ".join(f"{shown(kt, k)}: {model[k]}" for k in order()) + "}"

    for _ in range(rng.randint(20, 60)):
        r = rng.random()
        k = draw(rng, kt, pool)
        if r < 0.35:
            v = rng.randint(-1000, 1000)
            lines.append(f"    m.set({lit(kt, k)}, {v})")
            model[k] = v
        elif r < 0.45:
            say(f"m.delete({lit(kt, k)})", "true" if k in model else "false")
            model.pop(k, None)
        elif r < 0.6:
            say(f"m.get({lit(kt, k)})", f"Some({model[k]})" if k in model else "None")
        elif r < 0.7:
            say(f"m.has({lit(kt, k)})", "true" if k in model else "false")
        elif r < 0.75:
            say("m.len()", len(model))
        elif r < 0.8:
            say("m.keys()", "[" + ", ".join(shown(kt, x) for x in order()) + "]")
        elif r < 0.85:
            say("m.values()", "[" + ", ".join(str(model[x]) for x in order()) + "]")
        elif r < 0.9:
            say("m.entries().len()", len(model))
        elif r < 0.95:
            say("m", render())
        else:
            # A copy compares equal until either is written.
            lines.append(f"    let c{n[0]} = m")
            say(f"c{n[0]} == m", "true")
    say("m", render())
    lines.append("}")
    return "\n".join(lines) + "\n", "\n".join(want) + "\n"


BAD = [
    ("fn f() {\n    var m: Map[(f64, i64), i64] = {}\n}\n", "not `(f64, i64)`"),
    ("fn f() {\n    var m: Map[(i64, [i64]), i64] = {}\n}\n", "not `(i64, [i64])`"),
    ("type P = { x: i64 }\n\nfn f() {\n    var m: Map[(str, P), i64] = {}\n}\n", "not `(str, P)`"),
    ("fn f() {\n    var m: Map[(i64, (bool, f64)), i64] = {}\n}\n", "not `(i64, (bool, f64))`"),
    ("fn f() {\n    var m = map()\n    m.set((1, 2.5), 3)\n}\n", "not `(i64, f64)`"),
    ("fn f() {\n    var m: Map[(), i64] = {}\n}\n", "map keys must be"),
    ("fn g[T](x: T) -> i64 {\n    var m: Map[(T, str), i64] = {}\n    m.set((x, \"k\"), 1)\n    m.len()\n}\n\nfn f() -> i64 {\n    g(1.5)\n}\n",
     "not `f64`"),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--programs", type=int, default=int(os.environ.get("MAPKEYS_PROGRAMS", "60")))
    ap.add_argument("--seed", type=int, default=int(os.environ.get("SEED", "0")) or random.randrange(1 << 30))
    ap.add_argument("--keep", action="store_true")
    o = ap.parse_args()
    rng = random.Random(o.seed)
    work = tempfile.mkdtemp(prefix="grasm_mapkeys_")
    bad = 0
    checked = 0
    s3 = S3Engine(GRASM, work)
    try:
        path = os.path.join(work, "p.gr")
        exe = os.path.join(work, "p.bin")
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
                continue
            n = subprocess.run([exe], capture_output=True, text=True, timeout=600)
            s3.check(path, exe, f"program {idx} (SEED={o.seed})", interp=(r.returncode, r.stdout))
            for engine, got in (("interpreter", r), ("native", n)):
                if got.returncode != 0 or got.stdout != want:
                    bad += 1
                    print(f"program {idx}: {engine} disagrees with the model (exit {got.returncode})")
                    wl, gl = want.splitlines(), got.stdout.splitlines()
                    shown_n = 0
                    for w, g in zip(wl, gl + [""] * len(wl)):
                        if w != g and shown_n < 4:
                            print(f"    want {w}\n    got  {g}")
                            shown_n += 1
                    if got.stderr:
                        print("    stderr: " + got.stderr.strip()[:300])
                    if o.keep:
                        with open(os.path.join(ROOT, f"mapkeys_fail_{idx}.gr"), "w") as fh:
                            fh.write(src)
            held = leaked(exe)
            if held:
                bad += 1
                print(f"program {idx}: LEAKS {held}")
        for src, need in BAD:
            with open(path, "w") as fh:
                fh.write(src + "\nfn main() {\n}\n")
            c = subprocess.run([GRASM, "check", path], capture_output=True, text=True, timeout=120)
            checked += 1
            if c.returncode == 0 or need not in c.stdout + c.stderr:
                bad += 1
                print(f"not refused with `{need}`:\n{src}\n{(c.stdout + c.stderr)[:600]}")
        if not s3.report():
            bad += 1
        print(f"programs: {o.programs}   checked lines: {checked}   disagreed: {bad}   (SEED={o.seed})")
    finally:
        if not o.keep:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
