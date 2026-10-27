#!/usr/bin/env python3
"""The two frontends on generated programs: the same TIR, or both refuse.

`tirdiff` holds `compiler/front` -- the frontend in Grasm -- to `grasm tir`
on every program in the repository, and `tirrefuse` on every program the
Rust frontend refuses there and in its own test suites. Those were written
by people. This makes programs instead, from the generators the other
fuzzers use (each builds programs that are well typed by construction), and
then makes near-misses of a share of them -- one edit that usually breaks
the types and keeps the syntax: a literal of another type, a name swapped
for another or for none, an argument dropped, `let` and `var` traded, a
comparison where arithmetic was, a `try` on a call, a line twice.

Each program goes to both:

    grasm tir P    front tir P

and they must agree: both print the TIR, the same bytes, or both refuse.
Which diagnostic is not compared here (S2 part three).

Counted, never assumed: a Rust panic is a Rust bug and fails the run; every
engine must make programs and have some compared, and the run must see
enough accepted by both and enough refused by both, or it fails -- a fuzzer
that skips is not a pass (`FUZZ_TIR_SKIP=<engine>` plants exactly that, for
the controls).

    scripts/fuzz_tir.py [--programs N] [--seed S] [--keep DIR]

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
# The generators are imported, and some build the compiler when imported
# without a named one; this run names it.
os.environ["GRASM"] = GRASM

import importlib.util as _ilu


def _script(name):
    spec = _ilu.spec_from_file_location(name, os.path.join(ROOT, "scripts", f"{name}.py"))
    mod = _ilu.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


TIMEOUT = 120


def first_text(x):
    """A generator's program: the source, whatever else it returns beside it."""
    if isinstance(x, str):
        return x
    if isinstance(x, (tuple, list)):
        for y in x:
            if isinstance(y, str):
                return y
    raise TypeError(f"no program in {type(x).__name__}")


def engines():
    diff = _script("fuzz_differential")
    gen = _script("fuzz_generics")
    fal = _script("fuzz_fallible")
    iw = _script("fuzz_intwidth")
    mk = _script("fuzz_mapkeys")
    hd = _script("fuzz_handlers")
    s3 = _script("fuzz_s3")
    rows = _script("fuzz_rows")
    mods = _script("fuzz_modules")

    def rows_package(rng, i, work):
        root = os.path.join(work, f"ws{i}")
        rows.build_package(rng.choice(rows.CARRIERS), rng.choice(["shout", "dbl"]), root)
        return {"entry": os.path.join(root, "app", "src", "main.gr")}

    def modules_tree(rng, i, work):
        name, decls, stmts = rng.choice(mods.POSITIONS)
        where = rng.choice(["direct", "through a library", "transitive"])
        return {"entry": mods.tree(os.path.join(work, f"tree{i}"), where, decls, stmts)}

    out = [
        ("differential", lambda rng, i: diff.Gen(rng).program()),
        ("generics", lambda rng, i: gen.Gen(rng).program()),
        ("fallible", lambda rng, i: fal.gen(rng.randrange(1 << 30))),
        ("intwidth", lambda rng, i: first_text(iw.program(rng, i))),
        ("mapkeys", lambda rng, i: first_text(mk.program(rng, i))),
        ("handlers", lambda rng, i: first_text(hd.build(rng))),
        # Heap values, doubles, closures and part four's constructs, through
        # the generator that combines them.
        ("s3", lambda rng, i: first_text(s3.Gen(rng).program())),
        # Effect rows through every carrier, held to a `/ {}` row.
        ("rows", lambda rng, i: rows.build(rng.choice(rows.CARRIERS), rng.choice(rows.SITES),
                                           rng.choice(["shout", "dbl"]))),
        # Packages: a `capabilities = []` library, and three-package graphs --
        # the loader, the manifests and the capability ceilings.
        ("rows-package", rows_package),
        ("modules", modules_tree),
    ]
    return out


INT_LIT = re.compile(r"(?<![\w.\"])(\d+)(?![\w.])")
WORD = re.compile(r"\b[a-z_][a-z_0-9]*\b")
CALL = re.compile(r"\b([a-z_][a-z_0-9]*)\(")
KEYWORDS = {"fn", "let", "var", "if", "else", "match", "for", "in", "while", "return",
            "true", "false", "and", "or", "not", "type", "pub", "import", "with",
            "handled_by", "effect", "try", "fail", "own", "inout", "break", "continue",
            "main", "print", "impl", "interface", "test", "bench", "ref", "spawn",
            "parallel", "region", "unsafe", "extern", "export"}


def code_lines(src):
    """Indexes of lines with no string literal, so an edit stays out of text."""
    return [i for i, l in enumerate(src.split("\n")) if '"' not in l and l.strip() and not l.strip().startswith("//")]


def mutate(rng, src):
    """One edit that usually breaks the types and keeps the syntax; the kind
    made, or None when nothing applied."""
    lines = src.split("\n")
    cands = code_lines(src)
    if not cands:
        return None, src
    for _ in range(12):
        kind = rng.choice(["literal", "name", "drop_arg", "let_var", "compare", "try", "dup_line", "unknown_name"])
        i = rng.choice(cands)
        line = lines[i]
        new = None
        if kind == "literal":
            ms = list(INT_LIT.finditer(line))
            if ms:
                m = rng.choice(ms)
                rep = rng.choice(['"x"', "1.5", "true", "[1]", "(1, 2)", "-1"])
                new = line[: m.start()] + rep + line[m.end():]
        elif kind in ("name", "unknown_name"):
            ms = [m for m in WORD.finditer(line) if m.group(0) not in KEYWORDS]
            if ms:
                m = rng.choice(ms)
                if kind == "unknown_name":
                    rep = "nowhere_" + m.group(0)
                else:
                    pool = [w for w in set(WORD.findall(src)) if w not in KEYWORDS and w != m.group(0)]
                    if not pool:
                        continue
                    rep = rng.choice(sorted(pool))
                new = line[: m.start()] + rep + line[m.end():]
        elif kind == "drop_arg":
            m = re.search(r",\s*[^,()]+\)", line)
            if m:
                new = line[: m.start()] + ")" + line[m.end():]
        elif kind == "let_var":
            if re.search(r"\blet\b", line):
                new = re.sub(r"\blet\b", "var", line, count=1)
            elif re.search(r"\bvar\b", line):
                new = re.sub(r"\bvar\b", "let", line, count=1)
        elif kind == "compare":
            for a, b in ((" + ", " == "), (" == ", " + "), (" < ", " - "), (" * ", " < "), (" and ", " + ")):
                if a in line:
                    new = line.replace(a, b, 1)
                    break
        elif kind == "try":
            ms = list(CALL.finditer(line))
            if ms:
                m = rng.choice(ms)
                new = line[: m.start()] + "try " + line[m.start():]
        elif kind == "dup_line":
            if not line.rstrip().endswith("{") and not line.strip().startswith("}"):
                lines.insert(i, line)
                return kind, "\n".join(lines)
        if new is not None and new != line:
            lines[i] = new
            return kind, "\n".join(lines)
    return None, src


def run(cmd):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=TIMEOUT, cwd=ROOT,
                           stdin=subprocess.DEVNULL)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return None, "", "timeout"


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
    ap.add_argument("--programs", type=int, default=700)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--keep", default=None)
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="fuzz_tir_")
    try:
        front = build_front(work)
        engs = engines()
        skip = os.environ.get("FUZZ_TIR_SKIP", "")
        rng = random.Random(args.seed)
        stats = {name: {"made": 0, "same": 0, "refused": 0, "differ": 0, "panic": 0, "timeout": 0,
                        "mutants": 0} for name, _ in engs}
        failures = []
        for i in range(args.programs):
            name, make = engs[i % len(engs)]
            if skip and name == skip:
                continue
            prng = random.Random(rng.randrange(1 << 60))
            try:
                if name in ("rows-package", "modules"):
                    src = make(prng, i, work)
                else:
                    src = make(prng, i)
            except Exception as e:  # noqa: BLE001 -- a generator bug is reported, not hidden
                failures.append((name, i, f"the generator failed: {type(e).__name__}: {e}", ""))
                continue
            st = stats[name]
            st["made"] += 1
            kind = None
            if isinstance(src, dict):
                # A tree on disk: the entry is mutated in place, sometimes.
                path = src["entry"]
                with open(path) as f:
                    src = f.read()
                if prng.random() < 0.3:
                    kind, src = mutate(prng, src)
                    if kind:
                        st["mutants"] += 1
                        with open(path, "w") as f:
                            f.write(src)
            else:
                if prng.random() < 0.45:
                    kind, src = mutate(prng, src)
                    if kind:
                        st["mutants"] += 1
                path = os.path.join(work, f"p{i}.gr")
                with open(path, "w") as f:
                    f.write(src)
            rc, rout, rerr = run([GRASM, "tir", path])
            gc, gout, gerr = run([front, "tir", path])
            if rc is None or gc is None:
                st["timeout"] += 1
                failures.append((name, i, f"timeout (grasm {rc}, front {gc})", src))
            elif "panicked at" in rerr:
                st["panic"] += 1
                failures.append((name, i, "the Rust frontend panics: " + rerr.strip().split("\n")[0], src))
            elif rc == 0 and gc == 0 and rout == gout:
                st["same"] += 1
            elif rc != 0 and gc != 0:
                st["refused"] += 1
            else:
                st["differ"] += 1
                why = "the TIR differs" if rc == 0 and gc == 0 else (
                    "Rust accepts, Grasm refuses: " + gerr.strip().split("\n")[0][:200] if rc == 0 else
                    "Rust refuses (" + (rerr.strip().split("\n") or [""])[0][:200] + "), Grasm accepts")
                failures.append((name, i, f"{why} (edit: {kind})", src))
        total = {k: sum(s[k] for s in stats.values()) for k in ("made", "same", "refused", "differ", "panic", "timeout", "mutants")}
        for name, _ in engs:
            s = stats[name]
            print(f"  {name:16} made {s['made']:4}  same TIR {s['same']:4}  both refuse {s['refused']:4}  "
                  f"disagree {s['differ']}  near-misses {s['mutants']}")
        print(f"programs: {total['made']}   same TIR: {total['same']}   both refuse: {total['refused']}   "
              f"disagree: {total['differ']}   Rust panics: {total['panic']}   timeouts: {total['timeout']}   "
              f"near-misses made: {total['mutants']}")
        for name, i, why, src in failures[:8]:
            print(f"\nFAILED ({name}, program {i}): {why}")
            if args.keep:
                os.makedirs(args.keep, exist_ok=True)
                with open(os.path.join(args.keep, f"{name}-{i}.gr"), "w") as f:
                    f.write(src)
        bad = False
        if failures:
            bad = True
        if total["same"] + total["refused"] + total["differ"] + total["panic"] + total["timeout"] != total["made"]:
            print("the counts do not add up")
            bad = True
        for name, _ in engs:
            s = stats[name]
            if s["made"] < 10 or s["same"] < 3:
                print(f"engine `{name}` made {s['made']} programs and {s['same']} were compared: an engine that skips is not a pass")
                bad = True
        if total["same"] < args.programs // 3 or total["refused"] < args.programs // 10:
            print(f"too few compared: {total['same']} accepted by both, {total['refused']} refused by both")
            bad = True
        return 1 if bad else 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
