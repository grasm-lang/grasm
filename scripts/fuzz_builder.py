#!/usr/bin/env python3
"""Builders, against a Python model of what each one holds.

`StrBuilder` and `ByteBuilder` ([L-LIB-BUILDER]) append in place when nothing
else holds the buffer and copy when something does ([E-COW]). That is the
whole risk: an engine that appends through a shared buffer prints the right
text for the builder it was asked about and the wrong text for the copy, and
two engines that share the mistake agree with each other. So each program
here makes copies -- of a variable, into a struct field, into a list element,
through an `inout` parameter -- appends to both sides at random, and prints
every builder after every few steps; the answers come from Python lists and
strings, which have no sharing to get wrong.

Each program runs under `grasm run` and as a native build, both must print the
model's lines, and the native one must leak nothing.

    scripts/fuzz_builder.py [--programs N] [--seed S] [--keep]
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

# Text with one-, two-, three- and four-byte characters, so a length in
# characters and one in bytes differ.
PIECES = ["", "a", "xyz", "é", "日本", "😀", "a\\nb", "\\{\\}", "ü-ß"]


def grasm_str(p):
    """The source spelling of a piece, and the text it stands for."""
    return '"' + p + '"', p.replace("\\n", "\n").replace("\\{", "{").replace("\\}", "}")


def hexs(bs):
    return "0x" + "".join(f"{b:02x}" for b in bs)


def program(rng, idx):
    # Model: name -> ("s", str) or ("b", list of ints).
    lines = ["type Box = { s: StrBuilder, b: ByteBuilder }", "",
             "fn grow(inout b: ByteBuilder, x: i64) {", "    b.push_byte(x)", "}", "",
             "fn say(inout s: StrBuilder, t: str) {", "    s.push_str(t)", "}", "",
             "fn main() {"]
    want = []
    model = {}
    names = []
    for i in range(3):
        n = f"s{i}"
        lines.append(f"    var {n} = str_builder()")
        model[n] = ("s", "")
        names.append(n)
        n = f"b{i}"
        lines.append(f"    var {n} = byte_builder()")
        model[n] = ("b", [])
        names.append(n)
    lines.append("    var box = Box{ s: str_builder(), b: byte_builder() }")
    model["box.s"] = ("s", "")
    model["box.b"] = ("b", [])
    lines.append("    var bs = [byte_builder(), byte_builder()]")
    model["bs[0]"] = ("b", [])
    model["bs[1]"] = ("b", [])
    places = names + ["box.s", "box.b", "bs[0]", "bs[1]"]
    step = 0
    for _ in range(rng.randint(20, 60)):
        p = rng.choice(places)
        kind, val = model[p]
        r = rng.random()
        if r < 0.25:
            # A copy: the two must be independent from here on.
            q = rng.choice([x for x in places if model[x][0] == kind and x != p])
            lines.append(f"    {q} = {p}")
            model[q] = (kind, val if kind == "s" else list(val))
        elif kind == "s":
            src, text = grasm_str(rng.choice(PIECES))
            if rng.random() < 0.3 and "." not in p and "[" not in p:
                lines.append(f"    say({p}, {src})")
            else:
                lines.append(f"    {p}.push_str({src})")
            model[p] = ("s", val + text)
        else:
            c = rng.random()
            if c < 0.35:
                x = rng.randint(0, 255)
                if rng.random() < 0.3 and "." not in p and "[" not in p:
                    lines.append(f"    grow({p}, {x})")
                else:
                    lines.append(f"    {p}.push_byte({x})")
                model[p] = ("b", val + [x])
            elif c < 0.6:
                xs = [rng.randint(0, 255) for _ in range(rng.randint(0, 5))]
                lines.append(f"    {p}.push_bytes(bytes_of({xs}))")
                model[p] = ("b", val + xs)
            elif c < 0.8:
                src, text = grasm_str(rng.choice(PIECES))
                lines.append(f"    {p}.push_str({src})")
                model[p] = ("b", val + list(text.encode()))
            elif val:
                i = rng.randrange(len(val))
                x = rng.randint(0, 255)
                lines.append(f"    {p}.set({i}, {x})")
                nv = list(val)
                nv[i] = x
                model[p] = ("b", nv)
        step += 1
        if step % 5 == 0:
            for q in places:
                kind, val = model[q]
                if kind == "s":
                    lines.append(f'    print("{idx}.{step} {q} {{{q}.len()}} [{{{q}.to_str()}}]")')
                    want.append(f"{idx}.{step} {q} {len(val)} [{val}]")
                else:
                    at = f" {{{q}.at({len(val) - 1})}}" if val else ""
                    lines.append(f'    print("{idx}.{step} {q} {{{q}.len()}} {{{q}}}{at}")')
                    tail = f" {val[-1]}" if val else ""
                    want.append(f"{idx}.{step} {q} {len(val)} {hexs(val)}{tail}")
    # Equality is by contents, so two built the same way are equal.
    lines.append(f'    print("{idx}.eq {{s0 == s0}} {{b0 == b0}}")')
    want.append(f"{idx}.eq true true")
    lines.append("}")
    return "\n".join(lines) + "\n", "\n".join(want) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--programs", type=int, default=int(os.environ.get("BUILDER_PROGRAMS", "30")))
    ap.add_argument("--seed", type=int, default=int(os.environ.get("SEED", "0")) or random.randrange(1 << 30))
    ap.add_argument("--keep", action="store_true")
    o = ap.parse_args()
    rng = random.Random(o.seed)
    work = tempfile.mkdtemp(prefix="grasm_builder_")
    bad = 0
    checked = 0
    s3 = S3Engine(GRASM, work)
    try:
        for idx in range(o.programs):
            src, want = program(rng, idx)
            checked += 2 * len(want.splitlines())
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
            s3.check(path, exe, f"program {idx} (SEED={o.seed})", interp=(r.returncode, r.stdout))
            for engine, got in (("interpreter", r), ("native", n)):
                if got.returncode != 0 or got.stdout != want:
                    bad += 1
                    print(f"program {idx}: {engine} disagrees with the model (exit {got.returncode})")
                    wl, gl = want.splitlines(), got.stdout.splitlines()
                    shown = 0
                    for w, g in zip(wl, gl + [""] * len(wl)):
                        if w != g and shown < 4:
                            print(f"    want {w}\n    got  {g}")
                            shown += 1
                    if got.stderr:
                        print("    stderr: " + got.stderr.strip()[:300])
                    if o.keep:
                        shutil.copy(path, os.path.join(ROOT, f"builder_fail_{idx}.gr"))
            held = leaked(exe)
            if held:
                bad += 1
                print(f"program {idx}: LEAKS {held}")
        if not s3.report():
            bad += 1
        print(f"programs: {o.programs}   checked lines: {checked}   disagreed: {bad}   (SEED={o.seed})")
    finally:
        if not o.keep:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
