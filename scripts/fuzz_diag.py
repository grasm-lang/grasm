#!/usr/bin/env python3
"""The two frontends' diagnostics on generated programs, byte for byte.

`diagdiff` holds `front check` -- the frontend in Grasm -- to `grasm check`
on every program in the repository and in the Rust test suites. Those were
written by people, and most of them stop at their first mistake. This makes
programs instead, and makes them wrong:

* the ten generators `scripts/fuzz_tir.py` uses (the differential, generics,
  fallible, integer-width, map-key, handler, backend, effect-row,
  package-row and module-graph fuzzers'), each well typed by construction,
  and one of fuzz_tir's edits -- a literal of another type, a name swapped
  or unknown, an argument dropped, `let` and `var` traded, a comparison
  where arithmetic was, a `try` on a call, a line twice -- in more than half
  of them; the package generators give the loader, the manifests and the
  capability ceilings their diagnostics;
* the programs `scripts/fuzz_front.py` makes for the parser: mutated
  windows of the repository's files, nesting at and past the cap, literals
  at their edges, Unicode where it does not belong, token soup.

Each program goes to both:

    grasm check P    front check P

and the exit status, stdout and stderr must be the same bytes: every
diagnostic in the same order, at the same line and column, quoting the same
line with the same carets and the same help, the same count line, the same
ceiling breach or the same `ok`.

Counted, never assumed: a Rust panic is a Rust bug and fails the run; every
engine must make programs and have some compared, the run must see enough
refusals, enough diagnostics and enough accepted programs, and the Grasm
side's output must have been read for every program counted as agreeing --
a fuzzer that skips is not a pass. `FUZZ_DIAG_SKIP=<engine>` and
`FUZZ_DIAG_FAULT=nocompare` (the Grasm side never run, every program taken
as agreeing) plant exactly those, for `scripts/diagcontrols.sh`.

    scripts/fuzz_diag.py [--programs N] [--seed S] [--keep DIR]

`GRASM=` names the Rust compiler; `FRONT=` a built frontend.
"""

import argparse
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")
os.environ["GRASM"] = GRASM

import importlib.util as _ilu


def _script(name):
    spec = _ilu.spec_from_file_location(name, os.path.join(ROOT, "scripts", f"{name}.py"))
    mod = _ilu.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


TIMEOUT = 120
DIAG = re.compile(rb"^.+:[0-9]+:[0-9]+: error: ", re.M)


def run(cmd):
    try:
        p = subprocess.run(cmd, capture_output=True, timeout=TIMEOUT, cwd=ROOT, stdin=subprocess.DEVNULL)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return None, b"", b"timeout"


def build_front(work):
    front = os.environ.get("FRONT")
    if front:
        return front
    front = os.path.join(work, "front")
    p = subprocess.run([GRASM, "build", "compiler/front/src/main.gr", "--native", "-o", front],
                       capture_output=True, text=True, cwd=ROOT)
    if p.returncode != 0:
        print("error: compiler/front does not build:\n" + p.stdout + p.stderr)
        sys.exit(1)
    return front


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--programs", type=int, default=2200)
    ap.add_argument("--seed", type=int, default=3)
    ap.add_argument("--keep", default=None)
    args = ap.parse_args()

    tir = _script("fuzz_tir")
    ff = _script("fuzz_front")
    engs = tir.engines()
    files = ff.seed_files()
    if not files:
        print("no .gr files to mutate: nothing would be compared")
        return 1
    names = [n for n, _ in engs] + ["parser"]
    skip = os.environ.get("FUZZ_DIAG_SKIP", "")
    nocompare = os.environ.get("FUZZ_DIAG_FAULT") == "nocompare"

    work = tempfile.mkdtemp(prefix="fuzz_diag_")
    try:
        front = build_front(work)
        rng = random.Random(args.seed)
        stats = {n: {"made": 0, "same": 0, "refused": 0, "ok": 0, "differ": 0, "panic": 0, "timeout": 0,
                     "edits": 0} for n in names}
        diags = helps = ceilings = 0
        rust_bytes = front_bytes = 0
        failures = []
        for i in range(args.programs):
            name = names[i % len(names)]
            if skip and name == skip:
                continue
            st = stats[name]
            if name == "parser":
                prng = random.Random(rng.randrange(1 << 60))
                kind, src = ff.generate(prng, files)
                path = os.path.join(work, f"q{i}.gr")
                with open(path, "w", encoding="utf-8") as f:
                    f.write(src)
            else:
                _, path, kind, src = tir.make_program(engs, i, rng, work, rate=0.6, tree_rate=0.5,
                                                      which=names.index(name))
                if path is None:
                    failures.append((name, i, kind, ""))
                    continue
                if kind:
                    st["edits"] += 1
            st["made"] += 1
            rc, rout, rerr = run([GRASM, "check", path])
            if nocompare:
                gc, gout, gerr = rc, rout, rerr
            else:
                gc, gout, gerr = run([front, "check", path])
                front_bytes += len(gout) + len(gerr)
            if rc is None or gc is None:
                st["timeout"] += 1
                failures.append((name, i, f"timeout (grasm {rc}, front {gc})", src))
            elif rc not in (0, 1) or b"panicked at" in rerr:
                st["panic"] += 1
                failures.append((name, i, "the Rust frontend panics: " + rerr.decode("utf-8", "replace").strip().split("\n")[0], src))
            elif (rc, rout, rerr) == (gc, gout, gerr):
                st["same"] += 1
                rust_bytes += len(rout) + len(rerr)
                if rc == 0:
                    st["ok"] += 1
                else:
                    st["refused"] += 1
                    diags += len(DIAG.findall(rerr))
                    helps += rerr.count(b"\n  help: ") + (1 if rerr.startswith(b"  help: ") else 0)
                    ceilings += len(re.findall(rb"^error: package `[^`]*` performs ", rerr, re.M))
            else:
                st["differ"] += 1
                ra = rerr.decode("utf-8", "replace").split("\n")
                ga = gerr.decode("utf-8", "replace").split("\n")
                at = next((k for k in range(max(len(ra), len(ga)))
                           if k >= len(ra) or k >= len(ga) or ra[k] != ga[k]), 0)
                why = (f"exit {rc} against {gc}" if rc != gc else
                       "stdout differs" if rout != gout else
                       f"stderr line {at + 1}: rust {ra[at] if at < len(ra) else '<end>'!r:.160} "
                       f"front {ga[at] if at < len(ga) else '<end>'!r:.160}")
                failures.append((name, i, f"{why} (edit: {kind})", src))
        total = {k: sum(s[k] for s in stats.values()) for k in
                 ("made", "same", "refused", "ok", "differ", "panic", "timeout", "edits")}
        for n in names:
            s = stats[n]
            print(f"  {n:16} made {s['made']:4}  same {s['same']:4} (refused {s['refused']:4}, ok {s['ok']:4})  "
                  f"differ {s['differ']}  edited {s['edits']}")
        print(f"programs: {total['made']}   same output: {total['same']}   refused: {total['refused']}   "
              f"accepted: {total['ok']}   differ: {total['differ']}   Rust panics: {total['panic']}   "
              f"timeouts: {total['timeout']}   seed: {args.seed}")
        print(f"diagnostics compared: {diags}   help lines: {helps}   ceiling breaches: {ceilings}   "
              f"bytes compared: {rust_bytes}")
        for name, i, why, src in failures[:8]:
            print(f"\nFAILED ({name}, program {i}): {why}")
            if args.keep:
                os.makedirs(args.keep, exist_ok=True)
                with open(os.path.join(args.keep, f"{name}-{i}.gr"), "w") as f:
                    f.write(src)
        bad = bool(failures)
        if total["same"] + total["differ"] + total["panic"] + total["timeout"] != total["made"]:
            print("the counts do not add up")
            bad = True
        if front_bytes < rust_bytes or rust_bytes == 0:
            print(f"the Grasm side's output was not read for every program counted as agreeing "
                  f"({front_bytes} bytes read of {rust_bytes} compared)")
            bad = True
        for n in names:
            s = stats[n]
            if s["made"] < 10 or s["same"] < 3:
                print(f"engine `{n}` made {s['made']} programs and {s['same']} were compared: an engine that skips is not a pass")
                bad = True
        n = args.programs
        if total["refused"] < n // 4 or total["ok"] < n // 10 or diags < n // 4 or helps < n // 20:
            print(f"too few compared: {total['refused']} refused, {total['ok']} accepted, {diags} diagnostics, {helps} help lines")
            bad = True
        return 1 if bad else 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
