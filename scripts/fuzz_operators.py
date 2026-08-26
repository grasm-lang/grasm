#!/usr/bin/env python3
"""Every operator against every representation, and the answer pinned.

Two questions, and the second is the one nothing else asks.

*Do the engines agree where the operator is allowed?* That is what the other
generators do, and it is checked here for each accepted cell -- output, exit
code and diagnostics.

*Is the operator allowed on the same types it was allowed on yesterday?* The
checker decides that, so every engine moves together and no cross-engine gate
can see it. Which types compare with `<` was written down nowhere: it lived in
the code, and a change that widened or narrowed it silently would have looked
exactly like a change that did not.

So the table below is the answer, recorded. `==` and `!=` are defined on any
two values of one type. Ordering is defined on `i64`, `f64`, `str`, and on a
tuple whose every component is itself ordered -- which is why `(bool, i64)`
has none. `+` also concatenates a `str` and a list. Everything else is
refused, and the refusal is the language's, not a limitation of a backend.

A cell that changes is not necessarily wrong. It is a decision, and this is
what makes it one.

    scripts/fuzz_operators.py [--keep]
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")

import importlib.util as _ilu
_spec = _ilu.spec_from_file_location(
    "grasm_fresh",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", "fresh.py"),
)
_fresh = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_fresh)
_fresh.fresh_or_stop(GRASM)


PRELUDE = """type P = { n: i64 }
type B[T] = { v: T }
type Col = | Red | Green
"""

# (name, type, one value, another)
VALUES = [
    ("i64", "i64", "1", "2"),
    ("f64", "f64", "1.5", "2.5"),
    ("bool", "bool", "true", "false"),
    ("str", "str", '"a"', '"b"'),
    ("unit", "()", "()", "()"),
    ("bytes", "bytes", "bytes_of([1])", "bytes_of([2])"),
    ("list", "[i64]", "[1, 2]", "[3]"),
    ("tuple", "(i64, str)", '(1, "a")', '(2, "b")'),
    ("tuple-of-bool", "(bool, i64)", "(false, 9)", "(true, 0)"),
    ("nested-tuple", "((i64, i64), i64)", "((1, 2), 3)", "((1, 3), 0)"),
    ("option", "Option[i64]", "Some(1)", "None"),
    ("struct", "P", "P{ n: 1 }", "P{ n: 2 }"),
    ("generic", "B[i64]", "B{ v: 1 }", "B{ v: 2 }"),
    ("sum", "Col", "Red", "Green"),
    ("range", "range", "0..3", "1..4"),
    ("map", "Map[str, i64]", "map()", "map()"),
]

OPS = ["==", "!=", "<", "<=", ">", ">=", "+", "-", "*", "/", "%"]

# The recorded answer: which operators the checker accepts, per representation.
ACCEPTS = {
    "i64": set(OPS),
    "f64": set(OPS),
    "bool": {"==", "!="},
    "str": {"==", "!=", "<", "<=", ">", ">=", "+"},
    "unit": {"==", "!="},
    "bytes": {"==", "!="},
    "list": {"==", "!=", "+"},
    "tuple": {"==", "!=", "<", "<=", ">", ">="},
    # Ordering needs every component ordered, and `bool` is not.
    "tuple-of-bool": {"==", "!="},
    "nested-tuple": {"==", "!=", "<", "<=", ">", ">="},
    "option": {"==", "!="},
    "struct": {"==", "!="},
    "generic": {"==", "!="},
    "sum": {"==", "!="},
    "range": {"==", "!="},
    "map": {"==", "!="},
}

TRACE = re.compile(
    r"^(stack trace \(innermost first\):"
    r"|  [A-Za-z_][A-Za-z0-9_:]* at "
    r"|  \.\.\. [0-9]+ more frame\(s\))"
)


def strip_trace(text):
    return "\n".join(l for l in text.splitlines() if not TRACE.match(l))


def run(cmd, timeout=180):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="grasm_fuzz_operators_")
    path = os.path.join(work, "p.gr")
    exe = os.path.join(work, "p.bin")
    checked, bad, drifted = 0, [], []
    try:
        for name, ty, a, b in VALUES:
            for op in OPS:
                src = (
                    PRELUDE
                    + "fn main() {\n"
                    + f"    let x: {ty} = {a}\n"
                    + f"    let y: {ty} = {b}\n"
                    + f'    print("{{x {op} y}}")\n'
                    + "}\n"
                )
                with open(path, "w") as f:
                    f.write(src)
                checked += 1
                label = f"{name} {op}"
                rc, out, err = run([GRASM, "run", path])
                accepted = rc == 0
                want = op in ACCEPTS[name]
                if accepted != want:
                    drifted.append(
                        f"{label}: the checker {'accepts' if accepted else 'refuses'} it; "
                        f"this file records the opposite"
                    )
                    continue
                if not accepted:
                    continue
                rc2, o2, err2 = run([GRASM, "build", path, "--native", "-o", exe])
                if rc2 != 0:
                    first = (o2 + err2).strip().splitlines()
                    bad.append(f"{label}: {first[0] if first else '(silent)'}")
                    continue
                rc3, got, err3 = run([exe])
                if (out, rc, strip_trace(err)) != (got, rc3, strip_trace(err3)):
                    bad.append(
                        f"{label}: run={(out.strip(), rc)} native={(got.strip(), rc3)}"
                    )
        for d in drifted:
            print(f"DRIFTED {d}")
        for b_ in bad:
            print(f"DISAGREES {b_}")
        print(
            f"cells: {checked}   disagreed: {len(bad)}   drifted: {len(drifted)}"
        )
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad or drifted else 0)


if __name__ == "__main__":
    main()
