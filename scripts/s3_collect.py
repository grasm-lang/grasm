#!/usr/bin/env python3
"""The corpus programs the S3 gates look at, with their TIR and the subset rule's
verdict on each (`scripts/lib/s3.sh`, `s3_collect`).

    s3_collect.py <grasm> <outdir> < programs

Reads corpus program paths, one a line, in the corpus's order. For each one
that is expected to compile, writes its TIR to `<outdir>/<k>.tir` and prints

    <path> TAB <tir file> TAB <verdict>            a program with a `main`
    NOTIR TAB <path> TAB <the emitter's first line> a program that emits no TIR

The verdict is `in`, `out: <reasons>`, or `external: <libraries>` for a
program that links a system library, which the S3 gates set aside.

in the input's order; library modules (no `(main` in their TIR) are left out.
The TIR is emitted eight programs at a time -- it is most of the gates' time --
and the verdict is `scripts/s3_subset.py`'s, computed in this process.
"""
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import s3_subset  # noqa: E402


def one(args):
    grasm, outdir, k, src = args
    with open(src, encoding="utf-8") as f:
        if any(line.rstrip("\n") == "//! expect: error" for line in f):
            return None
    r = subprocess.run([grasm, "tir", src], capture_output=True, text=True)
    if r.returncode != 0:
        first = (r.stderr.strip().splitlines() or [""])[0]
        return ("NOTIR", src, first)
    text = r.stdout
    if "\n(main " not in text:
        return None
    tir = os.path.join(outdir, f"{k}.tir")
    with open(tir, "w", encoding="utf-8") as f:
        f.write(text)
    # A program that calls into a system library (`grasm links`) is linked
    # where that library is, and `scripts/sdltest.sh` is the gate that does;
    # the S3 gates set it aside, counted, as `difftest` does. TIR does not
    # say which libraries a program links, so this asks the compiler.
    links = subprocess.run([grasm, "links", src], capture_output=True, text=True)
    libs = " ".join(links.stdout.split())
    if libs:
        return (src, tir, f"external: {libs}")
    reasons = s3_subset.classify(s3_subset.parse(text))
    verdict = "out: " + "; ".join(sorted(set(reasons))) if reasons else "in"
    return (src, tir, verdict)


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    grasm, outdir = sys.argv[1], sys.argv[2]
    os.makedirs(outdir, exist_ok=True)
    progs = [line.strip() for line in sys.stdin if line.strip()]
    jobs = [(grasm, outdir, k, src) for k, src in enumerate(progs)]
    with ThreadPoolExecutor(8) as ex:
        for res in ex.map(one, jobs):
            if res is not None:
                print("\t".join(x.replace("\t", " ").replace("\n", " ") for x in res))
    return 0


if __name__ == "__main__":
    sys.exit(main())
