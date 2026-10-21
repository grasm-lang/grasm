#!/usr/bin/env python3
"""Appending N pieces: the old idioms against the builders, in both engines.

[L-LIB-BUILDER] promises amortized constant-time appends, so emitting an
output is linear in its size. A promise about cost is checked by measuring it
at more than one size: doubling N should roughly double a linear loop's time
and quadruple a quadratic one's. This builds each loop at every N, runs it
under `grasm run` and as a native binary, interleaving the runs and keeping the
best of `--reps` (a loaded machine only ever makes a run slower), and prints
seconds and the growth from one N to the next.

    scripts/builder_bench.py [--sizes 50000,100000,200000] [--reps 3]

Exit 1 when a builder loop grows faster than 3x per doubling -- quadratic
growth is 4x -- or does not beat the old idiom it replaces at the largest N
in the interpreter. Timings are measurements, not a gate: run it on a quiet
machine and read the table.
"""

import argparse
import os
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")

LOOPS = {
    # The idiom known-issues measured at 2.4 s for 400,000 appends.
    "str  s = s + x": ('var s = ""', 's = s + "ab"', "s.len()"),
    "str  StrBuilder": ("var s = str_builder()", 's.push_str("ab")', "s.len()"),
    # `bytes` cannot grow, so before a builder `concat` was the only way.
    "bytes  b = b.concat(x)": ("var b = bytes_of([])\n    let one = bytes_of([7])", "b = b.concat(one)", "b.len()"),
    "bytes  ByteBuilder": ("var b = byte_builder()", "b.push_byte(7)", "b.len()"),
}


def source(setup, step, result, n):
    return (f"fn main() {{\n    {setup}\n    var i = 0\n    while i < {n} {{\n"
            f"        {step}\n        i += 1\n    }}\n    print({result})\n}}\n")


def timed(cmd):
    t = time.perf_counter()
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=3600)
    if r.returncode != 0:
        sys.exit(f"{' '.join(cmd)} failed: {r.stderr.strip()[:300]}")
    return time.perf_counter() - t, r.stdout.strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sizes", default="50000,100000,200000")
    ap.add_argument("--reps", type=int, default=3)
    o = ap.parse_args()
    sizes = [int(x) for x in o.sizes.split(",")]
    work = tempfile.mkdtemp(prefix="grasm_builder_bench_")
    progs = {}
    for name, (setup, step, result) in LOOPS.items():
        for n in sizes:
            path = os.path.join(work, f"p{len(progs)}.gr")
            with open(path, "w") as fh:
                fh.write(source(setup, step, result, n))
            exe = path[:-3]
            b = subprocess.run([GRASM, "build", "--native", path, "-o", exe], capture_output=True, text=True)
            if b.returncode != 0:
                sys.exit(f"{name} at {n} does not build: {b.stderr.strip()[:300]}")
            # The first run of a new binary pays macOS's first-exec check.
            subprocess.run([exe], capture_output=True)
            progs[(name, n)] = (path, exe)
    best = {}
    for _ in range(o.reps):
        for (name, n), (path, exe) in progs.items():
            for engine, cmd in (("run", [GRASM, "run", path]), ("native", [exe])):
                t, out = timed(cmd)
                if out != str(n * (2 if name.startswith("str") else 1)):
                    sys.exit(f"{name} at {n} under {engine} printed {out!r}")
                key = (name, n, engine)
                best[key] = min(best.get(key, t), t)
    bad = []
    for engine in ("run", "native"):
        print(f"\n{engine:6}  " + "".join(f"{n:>12}" for n in sizes) + "   growth per doubling")
        for name in LOOPS:
            ts = [best[(name, n, engine)] for n in sizes]
            growth = [ts[i + 1] / ts[i] for i in range(len(ts) - 1)]
            print(f"  {name:24}" + "".join(f"{t:>11.3f}s" for t in ts)
                  + "   " + " ".join(f"{g:.1f}x" for g in growth))
            if "Builder" in name and max(growth) > 3.0:
                bad.append(f"{name} under {engine} grows {max(growth):.1f}x per doubling")
    big = sizes[-1]
    for old, new in (("str  s = s + x", "str  StrBuilder"), ("bytes  b = b.concat(x)", "bytes  ByteBuilder")):
        if best[(new, big, "run")] >= best[(old, big, "run")]:
            bad.append(f"{new} is not faster than {old} at {big} under grasm run")
    for b in bad:
        print("NOT LINEAR: " + b)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
