#!/usr/bin/env python3
"""The Grasm runtime against the Rust runtime, on the benchmarks.

docs/self-hosting.md's S5 asks for the benchmarks to pass "with the Grasm
runtime linked in place of libgrasm_rt_abi.a". The programs are the eight
`scripts/bench.sh` runs, each built once by `grasm build --native` and
linked twice from the same objects by `scripts/lib/rtswap_link`: as it
ships, with the Rust runtime archive, and with the Grasm runtime alone in
the archive's place (`compiler/link -interpose`, no archive on the line).
So the two binaries differ in the runtime and in nothing else. Each must
print what `grasm run` prints before either is timed: a wrong answer is not a
fast one.

The binaries are timed in alternation -- one run of each per round, the
order rotated every round -- because run-to-run drift on this machine is
about 2%, and timing all of one and then all of the other turned that drift
into a difference that was not there. Each runs once untimed first: macOS
charges every new executable ~450 ms on its first run. A run is the wall
clock of one spawn, as `bench.sh` takes it. The figure per binary is the
median of its rounds; the ratio is the Grasm runtime's median over the Rust
runtime's, so above 1 is slower; the summary is their geometric mean.

Like `bench.sh`, it refuses to time on a busy machine -- a load average
above 0.35 of the cores -- and prints the load before and after every
program, because the parallel rows (`parmap`, `parwords`) swing by about 10%
under load. `BENCH_IGNORE_LOAD=1` times anyway, for numbers not to publish.

    scripts/rtbench.py [--rounds N] [--programs a,b,..]

`RT_OBJ=<object>` times that runtime object (otherwise built as the gates
build it); `LINK=` names the linker (compiler/link).
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
LINK = os.environ.get("LINK") or os.path.join(ROOT, "target", "release", "grasm-link")
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


def runtime_object(work):
    """The Grasm runtime's object: RT_OBJ, or built as the gates build it."""
    if os.environ.get("RT_OBJ"):
        return os.environ["RT_OBJ"]
    s3 = os.environ.get("S3") or os.path.join(work, "s3")
    if not os.environ.get("S3"):
        p = subprocess.run([GRASM, "build", os.path.join(ROOT, "compiler/s3/src/main.gr"), "--native", "-o", s3],
                           capture_output=True, text=True, cwd=ROOT)
        if p.returncode != 0:
            raise SystemExit("compiler/s3 does not build:\n" + p.stdout + p.stderr)
    tir = os.path.join(work, "rt.tir")
    obj = os.path.join(work, "rt.o")
    p = subprocess.run([GRASM, "tir", os.path.join(ROOT, "compiler/rt/src/main.gr")], capture_output=True, text=True, cwd=ROOT)
    if p.returncode != 0:
        raise SystemExit("the Grasm runtime emits no TIR:\n" + p.stderr[:500])
    with open(tir, "w") as f:
        f.write(p.stdout)
    p = subprocess.run([s3, tir, "-o", obj, "--runtime"], capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit("the Grasm runtime does not compile:\n" + (p.stdout + p.stderr)[:500])
    return obj


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=21)
    ap.add_argument("--programs", default=",".join(PROGRAMS))
    args = ap.parse_args()
    one, cpus = load()
    if one > cpus * 0.35 and not os.environ.get("BENCH_IGNORE_LOAD"):
        print(f"refusing to benchmark: load average {one:.2f} on {cpus} cores")
        return 1
    work = tempfile.mkdtemp(prefix="rtbench_")
    try:
        rt = runtime_object(work)
        env = dict(os.environ, RTSWAP_LINK=LINK, RTSWAP_RT=rt,
                   GRASM_LINKER=os.path.join(ROOT, "scripts", "lib", "rtswap_link"))
        env.pop("RTSWAP_POISON", None)
        env.pop("RTSWAP_ONLY", None)
        ratios = []
        for prog in args.programs.split(","):
            src = os.path.join(ROOT, "benchmarks", "grasm", prog + ".gr")
            exe = os.path.join(work, prog)
            p = subprocess.run([GRASM, "build", src, "--native", "-o", exe], capture_output=True, text=True, env=env)
            if p.returncode != 0 or not os.path.exists(exe + ".swap"):
                print(f"{prog}: the native build fails: {(p.stdout + p.stderr)[:300]}")
                return 1
            with open(exe + ".swaplog") as f:
                if any(line.startswith("rust ") for line in f):
                    print(f"{prog}: the Grasm runtime's link took names from the Rust runtime")
                    return 1
            bins = [("Rust runtime", exe), ("Grasm runtime", exe + ".swap")]
            want = subprocess.run([GRASM, "run", src], capture_output=True, text=True).stdout
            for name, b in bins:
                got = subprocess.run([b], capture_output=True, text=True, stdin=subprocess.DEVNULL)
                if got.returncode != 0 or got.stdout != want:
                    print(f"{prog}: the {name} build prints {got.stdout[:120]!r}, `grasm run` {want[:120]!r}")
                    return 1
            before, _ = load()
            times = [[], []]
            for r in range(args.rounds):
                for q in range(2):
                    k = (q + r) % 2
                    times[k].append(time_us(bins[k][1]))
            after, _ = load()
            med = [statistics.median(t) for t in times]
            ratios.append(med[1] / med[0])
            print(f"{prog:9s} Rust runtime {med[0] / 1000:8.2f} ms   Grasm runtime {med[1] / 1000:8.2f} ms "
                  f"({med[1] / med[0]:.3f}x)   load {before:.2f} -> {after:.2f}", flush=True)
            for _name, b in bins:
                os.remove(b)
        print()
        print(f"programs: {len(ratios)}   rounds: {args.rounds}   Grasm runtime / Rust runtime geomean: "
              f"{geomean(ratios):.3f}   cores: {cpus}")
        return 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
