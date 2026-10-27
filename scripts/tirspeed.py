#!/usr/bin/env python3
"""Source to TIR, the Grasm frontend against the Rust one: lines per second.

    scripts/tirspeed.py [--rounds N] [--files LIST]

Each round runs every program through both commands, alternating which goes
first program by program, and times each command's whole run:

    grasm tir P          the Rust frontend: load, check, emit
    front tir P          compiler/front built --native: the same

A program's lines are the lines of every module it loads, read from the
`file` lines its TIR begins with. The rate is all the programs' lines over
all their time; the table gives each round's and the median. Both sides are
timed as processes, start-up included, since the Rust side has no other
way to be timed without changing it. The load average is printed before and
after, because these numbers mean little without it.

The programs are the corpus programs both frontends accept with the same TIR
(`--files` names a list instead).
"""

import argparse
import os
import statistics
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")


def loadavg():
    return " ".join(f"{x:.2f}" for x in os.getloadavg())


def timed(cmd):
    t0 = time.perf_counter_ns()
    p = subprocess.run(cmd, capture_output=True, cwd=ROOT, stdin=subprocess.DEVNULL)
    return time.perf_counter_ns() - t0, p.returncode, p.stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--files", default=None)
    args = ap.parse_args()
    front = os.environ.get("FRONT")
    if not front:
        print("error: FRONT= must name compiler/front built --native")
        return 1
    if args.files:
        files = [l.strip() for l in open(args.files) if l.strip()]
    else:
        dirs = ["apps", "examples", "packages", "projects", "spec/conformance", "benchmarks/grasm"]
        files = sorted(
            os.path.join(d, f).replace(ROOT + "/", "")
            for top in dirs
            for d, _, fs in os.walk(os.path.join(ROOT, top))
            for f in fs if f.endswith(".gr"))
    print(f"load average at start: {loadavg()}")
    # Which programs count, and their lines: both accept, same TIR.
    progs = []
    lines_of = {}
    for f in files:
        _, rc, rout = timed([GRASM, "tir", f])
        _, gc, gout = timed([front, "tir", f])
        if rc != 0 or gc != 0 or rout != gout:
            continue
        n = 0
        for line in rout.decode().split("\n"):
            if not line.startswith("(file "):
                continue
            path = line.split('"')[5]
            if path not in lines_of:
                try:
                    with open(os.path.join(ROOT, path), "rb") as fh:
                        lines_of[path] = fh.read().count(b"\n")
                except OSError:
                    lines_of[path] = 0
            n += lines_of[path]
        progs.append((f, n))
    total_lines = sum(n for _, n in progs)
    print(f"programs: {len(progs)}   lines (every module each loads): {total_lines}")
    rust_rates = []
    grasm_rates = []
    for r in range(args.rounds):
        rust_ns = 0
        grasm_ns = 0
        for k, (f, _) in enumerate(progs):
            order = [(GRASM, "r"), (front, "g")]
            if (k + r) % 2:
                order.reverse()
            for exe, side in order:
                ns, rc, _ = timed([exe, "tir", f])
                if side == "r":
                    rust_ns += ns
                else:
                    grasm_ns += ns
        rr = total_lines / (rust_ns / 1e9)
        gr = total_lines / (grasm_ns / 1e9)
        rust_rates.append(rr)
        grasm_rates.append(gr)
        print(f"  round {r + 1}: grasm tir {rust_ns / 1e9:7.2f} s  {rr / 1000:7.1f}k lines/s   "
              f"front tir {grasm_ns / 1e9:7.2f} s  {gr / 1000:7.1f}k lines/s   "
              f"front/Rust time {rust_ns and grasm_ns / rust_ns:.3f}   load {loadavg()}")
    mr = statistics.median(rust_rates)
    mg = statistics.median(grasm_rates)
    print(f"median: grasm tir {mr / 1000:.1f}k lines/s   front tir {mg / 1000:.1f}k lines/s   "
          f"front/Rust time {mr / mg:.3f}")
    print(f"load average at end: {loadavg()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
