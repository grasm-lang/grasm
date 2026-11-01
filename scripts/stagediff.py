#!/usr/bin/env python3
"""Two stages of the Grasm compiler against each other on the corpus, every
artifact byte for byte (`scripts/stagediff.sh`, docs/self-hosting.md §6.14).

    stagediff.py <grasm> <A> <B> <files> <work>

`A` and `B` are directories each holding a compiler's `front`, `s3` and
`link` and the runtime object that compiler makes, `rt.o`. `files` lists the
`.gr` files to compare, one a line. For each:

  * `front tir` by both: the same exit status, stdout and stderr;
  * a program with a `main` whose TIR both accept (not a `//! expect: error`
    case): `s3` by both on that TIR -- the same exit status, output and, when
    it compiled, the same object; and then each object linked by its own
    compiler's `link` with its own runtime alone (`-interpose`), the same
    executable. A program that links a system library (`grasm links`; the
    Grasm frontend has no such command until S8) is set aside, counted.

Prints the counts and what differed, and each compiler's time per tool
(the processes' wall time added up, run `STAGEDIFF_JOBS` at a time, 4 by
default). Exit status 0 when nothing differed, the counts add up and
something was compared -- at least the floors `STAGEDIFF_FLOORS="<TIR>
<executables>"` names, when it names them.
"""
import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

LIMIT = 900  # seconds for one process; a hang is a failure, not a wait


def run(argv):
    t = time.monotonic()
    try:
        r = subprocess.run(argv, capture_output=True, timeout=LIMIT, stdin=subprocess.DEVNULL)
        res = (r.returncode, r.stdout, r.stderr)
    except subprocess.TimeoutExpired:
        res = ("timeout", b"", b"")
    return res, time.monotonic() - t


def first_line(b):
    s = b.decode("utf-8", "replace").strip().splitlines()
    return (s or [""])[0][:160]


class Job:
    def __init__(self, grasm, a, b, work):
        self.grasm, self.a, self.b, self.work = grasm, a, b, work

    def tir(self, k, src):
        """Both fronts on one file: (src, verdict, why, tir path or None, times)."""
        ra, ta = run([os.path.join(self.a, "front"), "tir", src])
        rb, tb = run([os.path.join(self.b, "front"), "tir", src])
        times = (ta, tb)
        if ra != rb:
            what = []
            if ra[0] != rb[0]:
                what.append(f"exit {ra[0]} against {rb[0]}")
            if ra[1] != rb[1]:
                what.append("the TIR differs")
            if ra[2] != rb[2]:
                what.append(f"stderr differs: {first_line(ra[2])!r} against {first_line(rb[2])!r}")
            return (src, "differ", "; ".join(what), None, times)
        if ra[0] != 0:
            return (src, "refused", "", None, times)
        tir = None
        with open(src, encoding="utf-8", errors="replace") as f:
            negative = any(line.rstrip("\n") == "//! expect: error" for line in f)
        if not negative and b"\n(main " in ra[1]:
            tir = os.path.join(self.work, f"{k}.tir")
            with open(tir, "wb") as f:
                f.write(ra[1])
        return (src, "same", "", tir, times)

    def program(self, k, src, tir):
        """Both backends and both linkers on one program."""
        links = subprocess.run([self.grasm, "links", src], capture_output=True, text=True)
        if links.stdout.split():
            return (src, "external", "", (0, 0, 0, 0))
        oa = os.path.join(self.work, f"{k}.a.o")
        ob = os.path.join(self.work, f"{k}.b.o")
        ra, sa = run([os.path.join(self.a, "s3"), tir, "-o", oa])
        rb, sb = run([os.path.join(self.b, "s3"), tir, "-o", ob])
        try:
            if ra != rb:
                return (src, "object differs", f"s3 exits {ra[0]} against {rb[0]}: {first_line(ra[2] or rb[2])}", (sa, sb, 0, 0))
            if ra[0] != 0:
                return (src, "refused", "", (sa, sb, 0, 0))
            with open(oa, "rb") as fa, open(ob, "rb") as fb:
                if fa.read() != fb.read():
                    return (src, "object differs", "the objects differ", (sa, sb, 0, 0))
            # One name in two directories: the code signature names the
            # file it was written as (its identifier, as ld's does), so two
            # links compare only under one name.
            xa = os.path.join(self.work, "a", str(k))
            xb = os.path.join(self.work, "b", str(k))
            la, ta = run([os.path.join(self.a, "link"), "-interpose", os.path.join(self.a, "rt.o"), oa, "-dead_strip", "-o", xa])
            lb, tb = run([os.path.join(self.b, "link"), "-interpose", os.path.join(self.b, "rt.o"), ob, "-dead_strip", "-o", xb])
            if la[0] != 0 or lb[0] != 0:
                return (src, "executable differs", f"a link fails: {first_line(la[1] + la[2] + lb[1] + lb[2])}", (sa, sb, ta, tb))
            with open(xa, "rb") as fa, open(xb, "rb") as fb:
                if fa.read() != fb.read():
                    return (src, "executable differs", "the executables differ", (sa, sb, ta, tb))
            return (src, "same", "", (sa, sb, ta, tb))
        finally:
            for p in (oa, ob, os.path.join(self.work, "a", str(k)), os.path.join(self.work, "b", str(k))):
                if os.path.exists(p):
                    os.remove(p)


def main():
    if len(sys.argv) != 6:
        print(__doc__)
        return 2
    grasm, a, b, files, work = sys.argv[1:]
    jobs = int(os.environ.get("STAGEDIFF_JOBS") or "4")
    os.makedirs(os.path.join(work, "a"), exist_ok=True)
    os.makedirs(os.path.join(work, "b"), exist_ok=True)
    srcs = [line.strip() for line in open(files) if line.strip()]
    job = Job(grasm, a, b, work)
    failed = []
    count = {"same": 0, "refused": 0, "differ": 0}
    progs = []
    t_front = [0.0, 0.0]
    with ThreadPoolExecutor(jobs) as ex:
        for k, (src, verdict, why, tir, times) in enumerate(ex.map(lambda kv: job.tir(*kv), enumerate(srcs))):
            count[verdict] += 1
            t_front[0] += times[0]
            t_front[1] += times[1]
            if verdict == "differ":
                failed.append(f"{src} — front tir: {why}")
            if tir:
                progs.append((k, src, tir))
    pc = {"same": 0, "refused": 0, "external": 0, "object differs": 0, "executable differs": 0}
    t_s3 = [0.0, 0.0]
    t_link = [0.0, 0.0]
    with ThreadPoolExecutor(jobs) as ex:
        for src, verdict, why, times in ex.map(lambda p: job.program(*p), progs):
            pc[verdict] += 1
            t_s3[0] += times[0]
            t_s3[1] += times[1]
            t_link[0] += times[2]
            t_link[1] += times[3]
            if verdict in ("object differs", "executable differs"):
                failed.append(f"{src} — {verdict}: {why}")

    print(f"files: {len(srcs)}   TIR the same: {count['same']}   refused alike: {count['refused']}   differ: {count['differ']}")
    print(f"programs with a main: {len(progs)}   objects the same: {pc['same'] + pc['executable differs']}   refused alike (outside s3's subset): {pc['refused']}   external set aside: {pc['external']}   objects differ: {pc['object differs']}")
    print(f"executables the same: {pc['same']}   differ: {pc['executable differs']}")

    def ratio(t):
        return f"{t[1] / t[0]:.2f}x" if t[0] > 0 else "-"

    print(f"time, A then B (processes' wall time added up, {jobs} at a time): front tir {t_front[0]:.0f} s, {t_front[1]:.0f} s ({ratio(t_front)}); "
          f"s3 {t_s3[0]:.0f} s, {t_s3[1]:.0f} s ({ratio(t_s3)}); link {t_link[0]:.0f} s, {t_link[1]:.0f} s ({ratio(t_link)})")
    if failed:
        print()
        print("FAILED:")
        for f in failed[:80]:
            print(f"  {f}")
        return 1
    if count["same"] + count["refused"] + count["differ"] != len(srcs):
        print("the file counts do not add up")
        return 1
    if sum(pc.values()) != len(progs):
        print("the program counts do not add up")
        return 1
    if count["same"] < 1 or pc["same"] < 1:
        print("nothing was compared")
        return 1
    floors = (os.environ.get("STAGEDIFF_FLOORS") or "").split()
    if floors and (count["same"] < int(floors[0]) or pc["same"] < int(floors[1])):
        print(f"too few compared: TIR {count['same']} (the floor is {floors[0]}), executables {pc['same']} (the floor is {floors[1]})")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
