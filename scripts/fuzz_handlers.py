#!/usr/bin/env python3
"""Differential testing for effect handlers and pattern binders.

Both of this project's SIGSEGVs came from here, and neither was reachable by
the other fuzzers, whose grammars have no handlers and no pattern binders:

  * a handler operation declared `-> () ! E` whose body gave the bare `()`
    half returned a null pointer typed as a fallible, and the compiled binary
    died with no message at all;
  * a bare identifier in a pattern that happened to name a nullary variant of
    *another* type was compiled as a variant test against a scalar, so the
    arm's body read a name nothing had bound.

A third defect in the same area was a wrong answer rather than a crash: a
handler's operations captured at the `with` rather than where the literal was
written, so `grasm run` said 1 and the native binary said 100.

Each program is built from independent choices — where the handler literal is
written, whether a captured binding changes between that point and the `with`,
what the operation returns, and which pattern shapes the body matches — and
all three engines must agree on stdout, stderr and exit code.

    scripts/fuzz_handlers.py [--count N] [--seed N] [--keep]
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
        name,
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", f"{name}.py"),
    )
    mod = _ilu.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


leaked = _lib("leaks").leaked

_spec = _ilu.spec_from_file_location(
    "grasm_fresh",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", "fresh.py"),
)
_fresh = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_fresh)
_fresh.fresh_or_stop(GRASM)


PRELUDE = """\
type Bad = { why: str }

type Flag = | On | Off

type Wrap = | Hold(v: i64) | Empty

type Sub = { n: i64, tag: str }

type Pt = { x: i64, y: i64 }

type Col = | Red | Green
"""

# What the effect operation answers, and how a caller consumes it. The `()`
# and `() ! Bad` rows are the shapes with no value to merge through.
RETURNS = [
    ("i64", "-> i64", "        let v = Src.get(1)\n        print(\"{v}\")"),
    ("str", "-> str", "        let v = Src.get(1)\n        print(\"{v}\")"),
    (
        "unit",
        "",
        "        Src.get(1)\n        print(\"after\")",
    ),
    (
        "fallible-i64",
        "-> i64 ! Bad",
        "        let v = Src.get(1) ?? 0 - 1\n        print(\"{v}\")",
    ),
    (
        "fallible-unit",
        "-> () ! Bad",
        "        let v = Src.get(1) ?? ()\n        print(\"after\")",
    ),
    (
        "fallible-match",
        "-> i64 ! Bad",
        "        match Src.get(1) {\n            Ok(v) -> print(\"ok {v}\")\n"
        "            Err(e) -> print(\"err {e.why}\")\n        }",
    ),
    # The representations the three above do not reach. An operation's result
    # crosses the same boundaries a value ever crosses -- an env slot, a return
    # register, a box -- and which of those it takes depends on what it is.
    # `unit` was here already and still hid F50, because every operation
    # returned a *literal*: the capture is the half with an address.
    ("f64", "-> f64", "        let v = Src.get(1)\n        print(\"{v}\")"),
    ("bool", "-> bool", "        let v = Src.get(1)\n        print(\"{v}\")"),
    ("bytes", "-> bytes", "        let v = Src.get(1)\n        print(\"{v}\")"),
    ("list", "-> [i64]", "        let v = Src.get(1)\n        print(\"{v}\")"),
    ("tuple", "-> (i64, str)", "        let v = Src.get(1)\n        print(\"{v}\")"),
    ("option", "-> Option[i64]", "        let v = Src.get(1)\n        print(\"{v}\")"),
    ("struct", "-> Pt", "        let v = Src.get(1)\n        print(\"{v}\")"),
    ("sum", "-> Col", "        let v = Src.get(1)\n        print(\"{v}\")"),
    # The result *discarded*. Every shape above binds or prints it, so none of
    # them could see F57: a `with` block's tail value was never released, and
    # the output is identical either way. A boxed result is the only kind that
    # can be lost this way.
    ("discarded", "-> [i64]", "        Src.get(1)"),
]

# The operation's body, by what its declared result allows.
BODIES = {
    "i64": ["n * 2", "n + cap", "if n > 0 { n } else { 0 - n }"],
    "str": ["\"got {n}\"", "\"{n}-{cap}\""],
    "unit": ["()", "{ let ignored = n }", "held_unit"],
    "fallible-i64": [
        "Ok(n * 2)",
        "if n > 0 { n * 2 } else { fail(Bad{ why: \"low\" }) }",
        "if n > 0 { Ok(n * 2) } else { Err(Bad{ why: \"low\" }) }",
        "n + cap",
    ],
    "fallible-unit": [
        "()",
        "if n > 0 { () } else { fail(Bad{ why: \"low\" }) }",
        "if n > 0 { Ok(()) } else { Err(Bad{ why: \"low\" }) }",
        "match n {\n        0 -> fail(Bad{ why: \"zero\" })\n        _ -> ()\n    }",
    ],
    "fallible-match": [
        "Ok(n * 2)",
        "if n > 1 { n * 3 } else { fail(Bad{ why: \"low\" }) }",
    ],
    # Each of these has one body that reads `n` or `cap` and one that returns
    # a captured binding -- `held`, declared beside `cap`. Returning a literal
    # never reads the environment, which is why an operation returning `()`
    # had been in this table all along and F50 was still there.
    "f64": ["n.as_f64() * 1.5", "held_f64"],
    "bool": ["n > 0", "held_bool"],
    "bytes": ["bytes_of([n, 2])", "held_bytes"],
    "list": ["[n, cap]", "held_list"],
    "tuple": ["(n, \"got {n}\")", "held_tuple"],
    "option": ["Some(n * 2)", "held_option"],
    "struct": ["Pt{ x: n, y: cap }", "held_struct"],
    "sum": ["if n > 0 { Red } else { Green }", "held_sum"],
    "discarded": ["[n, cap]", "held_list"],
}

# Where the handler literal is written, relative to the `with` that installs
# it — and whether the captured binding changes in between. A closure captures
# values at the moment it is created, so the answer must not depend on this.
PLACEMENTS = [
    ("inline", "    with Src handled_by SrcHandler{{ get: |n| {body} }} {{"),
    (
        "let-before",
        "    let h = SrcHandler{{ get: |n| {body} }}\n    with Src handled_by h {{",
    ),
    (
        "let-then-change",
        "    let h = SrcHandler{{ get: |n| {body} }}\n    cap = cap + 100\n"
        "    with Src handled_by h {{",
    ),
    (
        "two-handlers",
        "    let h = SrcHandler{{ get: |n| {body} }}\n    cap = cap + 5\n"
        "    let h2 = SrcHandler{{ get: |n| {body} }}\n    with Src handled_by h {{",
    ),
]

# Pattern shapes whose binders collide with a nullary variant of another type.
# `None` is in the prelude, so this needs no declaration of its own.
PATTERNS = [
    ("none-over-int", "    let a = match 7 {\n        None -> \"bound\"\n    }\n    print(a)"),
    (
        "flag-over-str",
        "    let b = match \"hi\" {\n        On -> \"bound-{On}\"\n    }\n    print(b)",
    ),
    (
        "nested-tuple",
        "    let c = match (1, 2) {\n        (On, Off) -> \"t {On} {Off}\"\n    }\n    print(c)",
    ),
    (
        "payload",
        "    let d = match Hold(5) {\n        Hold(On) -> \"w {On}\"\n        Empty -> \"e\"\n    }\n"
        "    print(d)",
    ),
    (
        "own-type",
        "    let e = match Flag.Off {\n        On -> \"on\"\n        Off -> \"off\"\n    }\n    print(e)",
    ),
    ("none", ""),
]


# One binding per representation, in scope for every body that names one. A
# captured *unit* is the shape that panicked inside Cranelift's frontend, and
# it is here for the same reason the others are: an operation returning a
# literal never reads its environment (F50).
HELD = (
    "    let held_unit = ()\n"
    "    let held_f64 = 2.5\n"
    "    let held_bool = true\n"
    "    let held_bytes = bytes_of([7])\n"
    "    let held_list = [7, 8]\n"
    "    let held_tuple = (7, \"held\")\n"
    "    let held_option = Some(7)\n"
    "    let held_struct = Pt{ x: 7, y: 8 }\n"
    "    let held_sum = Green\n"
)


def build(rng):
    ret_name, ret_sig, consume = rng.choice(RETURNS)
    body = rng.choice(BODIES[ret_name])
    place_name, place = rng.choice(PLACEMENTS)
    _, pattern = rng.choice(PATTERNS)

    src = PRELUDE
    src += "\neffect Src {\n    fn get(n: i64) " + ret_sig + "\n}\n"
    src += "\nfn main() {\n"
    src += "    var cap = 1\n"
    src += HELD
    src += place.format(body=body) + "\n"
    src += consume + "\n"
    src += "    }\n"
    src += "    print(\"cap {cap}\")\n"
    if pattern:
        src += pattern + "\n"
    # A struct field read through a capture, which is its own shape.
    src += "    var sub = Sub{ n: 5, tag: \"t\" }\n"
    src += "    let g = SrcHandler{ get: |n| " + body + " }\n"
    src += "    sub.n = 50\n"
    src += "    with Src handled_by g {\n"
    src += consume + "\n"
    src += "    }\n"
    src += "    print(\"{sub.n}\")\n"
    src += "}\n"
    return src


def run(cmd, cwd=None):
    p = subprocess.run(cmd, capture_output=True, text=True, cwd=cwd, timeout=120)
    return p.returncode, p.stdout, p.stderr


def strip_trace(text):
    """The one documented stderr difference ([E-PANIC-TRACE])."""
    keep = []
    for line in text.splitlines():
        if line == "stack trace (innermost first):":
            continue
        if line.startswith("  ") and " at " in line:
            continue
        keep.append(line)
    return "\n".join(keep)


def check_one(src, work):
    path = os.path.join(work, "p.gr")
    with open(path, "w") as f:
        f.write(src)

    rc, out, err = run([GRASM, "check", path])
    if rc != 0:
        return ("rejected", err.strip() or out.strip())

    ir, io_, ie = run([GRASM, "run", path])

    bundle = os.path.join(work, "p.bundle")
    rc, _, err = run([GRASM, "build", path, "-o", bundle])
    if rc != 0:
        return ("bundle build failed", err.strip())
    br, bo, be = run([bundle])
    os.remove(bundle)

    native = os.path.join(work, "p.native")
    rc, _, err = run([GRASM, "build", path, "--native", "-o", native])
    if rc != 0:
        if "does not yet support" in err:
            # Counted, not swallowed. An unlabelled `None` here is
            # indistinguishable from a program that agreed on every engine, so a
            # change that made `--native` decline the whole corpus would read as
            # a clean run.
            return ("declined", err.strip())
        return ("native build failed", err.strip())
    nr, no, ne = run([native])
    # Before the binary is removed: a handler's operations own an environment,
    # and getting its reference count wrong by one in either direction is
    # invisible in the output (F57).
    held = leaked(native) if nr == 0 else None
    os.remove(native)
    if held:
        return ("native leaks", held)

    if (ir, io_, strip_trace(ie)) != (br, bo, strip_trace(be)):
        return ("bundle disagrees", f"run={ir},{io_!r},{ie!r}\nbundle={br},{bo!r},{be!r}")
    if (ir, io_, strip_trace(ie)) != (nr, no, strip_trace(ne)):
        return ("native disagrees", f"run={ir},{io_!r},{ie!r}\nnative={nr},{no!r},{ne!r}")
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=200)
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    base = args.seed if args.seed is not None else random.randrange(1 << 30)
    failures, rejected, declined = [], [], []
    work = tempfile.mkdtemp(prefix="grasm_fuzz_handlers_")
    try:
        for i in range(args.count):
            seed = base + i
            src = build(random.Random(seed))
            verdict = check_one(src, work)
            if verdict is None:
                continue
            kind, detail = verdict
            if kind == "declined":
                declined.append(detail)
            elif kind == "rejected":
                rejected.append((seed, detail, src))
            else:
                failures.append((seed, kind, detail))
                print(f"MISMATCH seed={seed} ({kind})\n{detail}\n--- source ---\n{src}")
            if (i + 1) % 50 == 0:
                print(f"  {i + 1}/{args.count}  mismatches={len(failures)}")

        for seed, detail, src in rejected[:3]:
            first = detail.splitlines()[0] if detail else ""
            print(f"REJECTED seed={seed}: {first}\n--- source ---\n{src}")
        print(
            f"programs: {args.count}   mismatched: {len(failures)}   "
            f"rejected: {len(rejected)}   declined: {len(declined)}"
        )
    finally:
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)
        else:
            print(f"kept: {work}")
    # A decline is coverage lost, not an honest no. [§11] says the backend
    # compiles everything the interpreter accepts bar polymorphic recursion,
    # so a refusal here is either a new gap -- which belongs in
    # `scripts/nativegaps.sh` and in the tutorial's table -- or a defect.
    # Counted and not acted on, it reads as PASS while programs quietly stop
    # running: removing the F52 fix left this at `declined: 1` and exit 0.
    sys.exit(1 if failures or rejected or declined else 0)


if __name__ == "__main__":
    main()
