#!/usr/bin/env python3
"""The Grasm backend's code against the Cranelift build's, on the benchmarks.

docs/self-hosting.md's S3 asks for "benchmarks measured against the Cranelift
build". The programs are the ones `scripts/bench.sh` runs, each built by
`grasm build --native` (Cranelift) and by `grasm tir` through `compiler/s3`,
linked with `cc` and the same runtime as `scripts/lib/s3.sh` links the
corpus -- once per variant of the driver's arguments asked for (`--variant
NAME=ARGS`; by default one, the driver's defaults). Every binary must print
what `grasm run` prints before any is timed: a wrong answer is not a fast
one.

The binaries are timed in alternation -- one run of each per round, the
order rotated every round so each goes first as often -- because run-to-run
drift on this machine is about 2%, and timing all of one and then all of the
other turned that drift into a difference that was not there (an A/B of
identical code read 1.036x). Each binary runs once untimed first: macOS
charges every new executable ~450 ms on its first run. A run is the wall
clock of one spawn, the way `bench.sh` takes it. The figure per binary is
the median of its rounds; a ratio is a variant's median over Cranelift's, so
above 1 is slower; the summary is the geometric mean of the ratios. With two
variants, the second's median over the first's is printed too: the change
between them, measured in the same rounds.

Like `bench.sh`, it refuses to time on a busy machine -- a load average above
0.35 of the cores -- and prints the load before and after every program,
because the parallel rows (`parmap`, `parwords`) swing by about 10% under
load.

    scripts/s3bench.py [--rounds N] [--programs a,b,..] [--variant NAME=ARGS]...

`S3=<binary>` times that driver; `S3_RT=` names the runtime.
"""

import argparse
import math
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")
RUNTIME = os.environ.get("S3_RT") or os.path.join(ROOT, "target", "release", "libgrasm_rt_abi.a")
PROGRAMS = ["fib", "sieve", "matmul", "wordfreq", "tree", "sort", "parmap", "parwords"]


def load():
    return os.getloadavg()[0], os.cpu_count() or 1


def time_us(exe):
    started = time.perf_counter()
    p = subprocess.run([exe], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
    elapsed = time.perf_counter() - started
    if p.returncode != 0:
        raise SystemExit(f"{exe} exited {p.returncode} while being timed")
    return elapsed * 1e6


def geomean(xs):
    return math.exp(sum(math.log(x) for x in xs) / len(xs))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=21)
    ap.add_argument("--programs", default=",".join(PROGRAMS))
    ap.add_argument("--variant", action="append", default=[],
                    help="NAME=ARGS: a Grasm-backend build with these driver arguments")
    args = ap.parse_args()
    variants = []
    for v in args.variant or ["s3="]:
        name, _, extra = v.partition("=")
        variants.append((name, extra.split()))
    one, cpus = load()
    if one > cpus * 0.35 and not os.environ.get("BENCH_IGNORE_LOAD"):
        print(f"refusing to benchmark: load average {one:.2f} on {cpus} cores")
        return 1
    work = tempfile.mkdtemp(prefix="s3bench_")
    try:
        s3 = os.environ.get("S3") or os.path.join(work, "s3")
        if not os.environ.get("S3"):
            p = subprocess.run([GRASM, "build", os.path.join(ROOT, "compiler/s3/src/main.gr"), "--native", "-o", s3],
                               capture_output=True, text=True, cwd=ROOT)
            if p.returncode != 0:
                print("compiler/s3 does not build:\n" + p.stdout + p.stderr)
                return 1
        ratios = {name: [] for name, _ in variants}
        change = []
        for prog in args.programs.split(","):
            src = os.path.join(ROOT, "benchmarks", "grasm", prog + ".gr")
            tir = os.path.join(work, prog + ".tir")
            cl = os.path.join(work, prog + ".cl")
            p = subprocess.run([GRASM, "build", src, "--native", "-o", cl], capture_output=True, text=True)
            if p.returncode != 0:
                print(f"{prog}: the Cranelift build fails: {(p.stdout + p.stderr)[:300]}")
                return 1
            p = subprocess.run([GRASM, "tir", src], capture_output=True, text=True)
            if p.returncode != 0:
                print(f"{prog}: no TIR: {p.stderr[:300]}")
                return 1
            with open(tir, "w") as f:
                f.write(p.stdout)
            bins = [("Cranelift", cl)]
            for name, extra in variants:
                obj = os.path.join(work, f"{prog}.{name}.o")
                exe = os.path.join(work, f"{prog}.{name}")
                p = subprocess.run([s3, tir, "-o", obj] + extra, capture_output=True, text=True)
                if p.returncode != 0:
                    print(f"{prog}: the Grasm backend ({name}) answered {p.returncode}: {p.stderr[:300]}")
                    return 1
                p = subprocess.run(["cc", obj, RUNTIME, "-o", exe, "-Wl,-dead_strip"], capture_output=True, text=True)
                if p.returncode != 0:
                    print(f"{prog}: the object ({name}) does not link: {p.stderr[:300]}")
                    return 1
                os.remove(obj)
                bins.append((name, exe))
            want = subprocess.run([GRASM, "run", src], capture_output=True, text=True).stdout
            for name, exe in bins:
                got = subprocess.run([exe], capture_output=True, text=True, stdin=subprocess.DEVNULL)
                if got.returncode != 0 or got.stdout != want:
                    print(f"{prog}: {name} prints {got.stdout[:120]!r}, `grasm run` {want[:120]!r}")
                    return 1
            before, _ = load()
            times = [[] for _ in bins]
            n = len(bins)
            for r in range(args.rounds):
                for q in range(n):
                    k = (q + r) % n
                    times[k].append(time_us(bins[k][1]))
            after, _ = load()
            med = [statistics.median(t) for t in times]
            cells = [f"{bins[0][0]} {med[0] / 1000:8.2f} ms"]
            for k in range(1, n):
                ratios[bins[k][0]].append(med[k] / med[0])
                cells.append(f"{bins[k][0]} {med[k] / 1000:8.2f} ms ({med[k] / med[0]:.3f}x)")
            if n == 3:
                change.append(med[2] / med[1])
                cells.append(f"{bins[2][0]}/{bins[1][0]} {med[2] / med[1]:.3f}")
            print(f"{prog:9s} " + "   ".join(cells) + f"   load {before:.2f} -> {after:.2f}", flush=True)
            for _name, exe in bins:
                os.remove(exe)
            os.remove(tir)
        print()
        summary = [f"programs: {len(args.programs.split(','))}", f"rounds: {args.rounds}"]
        for name, _ in variants:
            summary.append(f"{name} geomean: {geomean(ratios[name]):.3f}")
        if change:
            summary.append(f"{variants[1][0]}/{variants[0][0]} geomean: {geomean(change):.3f}")
        summary.append(f"cores: {cpus}")
        print("   ".join(summary))
        return 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
