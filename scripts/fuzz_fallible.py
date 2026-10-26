#!/usr/bin/env python3
"""Randomized differential testing over the two halves of a fallible value.

`fuzz_differential.py` generates well-typed programs over plain types. This one
generates the shape that has produced the most miscompiles: a fallible function
whose value-position `if`/`match`/`return`/annotated-`let` mixes a bare ok half
with `Ok(..)`, `Err(..)`, `fail(..)` and a fallible call. Each merge position is
a separate conversion in the backend and each of them has been wrong at least
once.

    scripts/fuzz_fallible.py [count] [first-seed]

A mismatch prints the seed and the whole program; the same seed reproduces it.
"""
import os
import sys, random, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Overridable, like the shell gates: a debug build keeps Cranelift's own
# assertions and the runtime's overflow checks, which release compiles out.
G = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")

import importlib.util as _ilu


def _lib(name):
    spec = _ilu.spec_from_file_location(
        name,
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", f"{name}.py"),
    )
    mod = _ilu.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


leaked = _lib("leaks").leaked
# One binary, rebuilt in place, instead of one file per program (see the module).
build_native = _lib("inplace").build
# The Grasm backend (compiler/s3), held to the Cranelift build on each program.
S3Engine = _lib("s3engine").Engine


_spec = _ilu.spec_from_file_location(
    "grasm_fresh",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", "fresh.py"),
)
_fresh = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_fresh)
_fresh.fresh_or_stop(G)

OK_TYPES = [
    ("i64", ["3", "n * 2", "0 - 7"], "0 - 1"),
    ("f64", ["3", "4.5", "n.as_f64()"], "0.0"),
    ("str", ['"a"', '"bb"', '"c"'], '"-"'),
    ("bool", ["true", "false", "n > 1"], "false"),
    ("[i64]", ["[1]", "[n, 2]", "[]"], "[]"),
]

def half(rng, kind, bare):
    """A branch body: bare ok half, Ok(..), Err(..), fail(..), or a call."""
    pick = rng.random()
    if pick < 0.35: return bare
    if pick < 0.60: return f"Ok({bare})"
    if pick < 0.75: return "Err(Bad)"
    if pick < 0.85: return "fail(Bad)"
    return "g(n)"

def gen(seed):
    rng = random.Random(seed)
    kind, bares, dflt = rng.choice(OK_TYPES)
    n_branches = rng.choice([2, 2, 3, 4])
    form = rng.choice(["if", "match", "return_if", "let_ann"])
    bodies = [half(rng, kind, rng.choice(bares)) for _ in range(n_branches)]
    # at least one branch must give a value (not all diverging)
    if all(b in ("Err(Bad)", "fail(Bad)") for b in bodies):
        bodies[0] = rng.choice(bares)
    L = []
    L.append("type E = | Bad")
    L.append(f"fn g(n: i64) -> {kind} ! E {{")
    L.append(f"    if n < 0 {{ Err(Bad) }} else {{ Ok({bares[0]}) }}")
    L.append("}")
    L.append(f"fn f(n: i64) -> {kind} ! E {{")
    if form == "if":
        L.append(f"    if n == 0 {{")
        L.append(f"        {bodies[0]}")
        for i, b in enumerate(bodies[1:-1], start=1):
            L.append(f"    }} else if n == {i} {{")
            L.append(f"        {b}")
        L.append("    } else {")
        L.append(f"        {bodies[-1]}")
        L.append("    }")
    elif form == "match":
        L.append("    match n {")
        for i, b in enumerate(bodies[:-1]):
            L.append(f"        {i} -> {b}")
        L.append(f"        _ -> {bodies[-1]}")
        L.append("    }")
    elif form == "return_if":
        L.append(f"    return if n == 0 {{")
        L.append(f"        {bodies[0]}")
        L.append("    } else {")
        L.append(f"        {bodies[-1]}")
        L.append("    }")
    else:
        L.append(f"    let v: {kind} ! E = if n == 0 {{")
        L.append(f"        {bodies[0]}")
        L.append("    } else {")
        L.append(f"        {bodies[-1]}")
        L.append("    }")
        L.append("    v")
    L.append("}")
    L.append("fn main() {")
    L.append(f"    for i in 0..{n_branches + 1} {{")
    L.append(f'        print("{{f(i) ?? {dflt}}}")')
    L.append("    }")
    L.append("}")
    return "\n".join(L) + "\n"

def run(args):
    p = subprocess.run(args, capture_output=True, text=True, timeout=60)
    return p.stdout + p.stderr, p.returncode

def main():
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 500
    base = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    d = tempfile.mkdtemp(prefix="merge_fuzz_")
    src, out = os.path.join(d, "p.gr"), os.path.join(d, "p")
    checked = mism = rejected = 0
    # Programs the backend refused. They were skipped by `continue` and counted
    # nowhere, so a change that made `--native` decline half the corpus looked
    # exactly like a clean run.
    declined = 0
    s3 = S3Engine(G, d)
    for s in range(base, base + count):
        open(src, "w").write(gen(s))
        # No `check` run ahead of the build: the build typechecks this same
        # program on its way to code, so one that succeeded has already said it
        # was well-typed, and checking first spent a third of the run proving
        # it twice. The check moves to the build-failure path, which is where
        # an invalid program and a compiler that failed on a valid one have to
        # be told apart — and telling them apart is what it was always for.
        bp = build_native(G, src, out, "--native")
        why, rc = bp.stdout + bp.stderr, bp.returncode
        if rc != 0:
            _, crc = run([G, "check", src])
            if crc != 0:
                rejected += 1
                continue
            first = why.strip().splitlines()[0] if why.strip() else "(silent)"
            reason = first.split(": ", 1)[-1]
            # "codegen bug" is the backend failing its own verifier, which is a
            # defect and not a gap. Counted as a decline it read as a number
            # that had always been 1, and hid `return if c { fail(e) } else {
            # fail(e) }` failing to compile at all (F46). The methods fuzzer
            # already draws this line; this one only counted.
            if "codegen bug" in first:
                mism += 1
                print(f"CODEGEN BUG seed={s}: {reason}\n--- source ---\n{gen(s)}")
                continue
            # Named, not just counted: a decline that prints only a number
            # cannot be told apart from a permanent gap, so a new one looks
            # like the one that was always there.
            declined += 1
            print(f"  declined seed={s}: {reason}")
            continue
        a, ra = run([G, "run", src])
        b, rb = run([out])
        s3.check(src, out, f"seed={s}")
        # An error path is the one a program takes least often and frees
        # least carefully (F57).
        held = leaked(out) if rb == 0 else None
        if held:
            mism += 1
            print(f"LEAKS seed={s}: {held}")
            continue
        checked += 1
        if (a, ra) != (b, rb):
            mism += 1
            print(f"MISMATCH seed={s}\n--- source ---\n{gen(s)}--- run ---\n{a}rc={ra}\n--- native ---\n{b}rc={rb}")
            if mism >= 3:
                break
        if checked % 50 == 0:
            print(f"  {checked} compared, {mism} mismatched, {rejected} rejected", flush=True)
    s3_ok = s3.report()
    s3.cleanup()
    print(
        f"compared: {checked}   mismatched: {mism}   "
        f"rejected: {rejected}   declined: {declined}"
    )
    # This printed MISMATCH and exited 0. Every other generator here ends with
    # `sys.exit(1 if ... else 0)`; this one ended with the print, so a run that
    # found a disagreement reported success — the finding was on stdout and
    # nothing acted on it.
    # A decline is coverage lost, not an honest no. [§11] says the backend
    # compiles everything the interpreter accepts bar polymorphic recursion,
    # so a refusal here is either a new gap -- which belongs in
    # `scripts/nativegaps.sh` and in the tutorial's table -- or a defect.
    # Counted and not acted on, it reads as PASS while programs quietly stop
    # running: removing the F52 fix left this at `declined: 1` and exit 0.
    sys.exit(1 if mism or declined or not s3_ok else 0)


main()
