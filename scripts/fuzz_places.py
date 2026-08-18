#!/usr/bin/env python3
"""Every shape of place, read and written, on both engines.

Three findings in `ProjectGold/FINDINGS.md` have the same shape. A rule was
added to the native backend at one site and the backend handles that kind of
thing at several, so the rule was added to *a spelling* rather than to the
language:

  * F17 — a method's return type was named in one table and not the other, so
    `byte_len` on a loop variable compiled into a pointer dereference of the
    integer 10.
  * F22 — `.value` was implemented where a place *starts* at a reference, and
    a reference reached through a list is met halfway down `write_place`.
  * F23 — the C pointer conversion was implemented at the direct call site,
    and `s.strlen()` and `[s].map(strlen)[0]` reach the same function two
    other ways.

The first two were found by writing a program that happened to hit the second
site. This enumerates the sites instead.

Each generated program builds a value at one place shape and then reads it,
writes it, compounds it, or calls a mutating method on it. The oracle is the
interpreter, which is normative. A native build that *declines* is reported and
not failed — a refusal is honest. A native build that answers **differently** is
the defect this exists for.

    scripts/fuzz_places.py [--keep]
"""

import argparse
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")

PRELUDE = """type Inner = { n: i64, xs: [i64], s: str }
type Outer = { one: Inner, many: [Inner], cell: ref i64, cells: [ref i64] }

fn fresh() -> Outer {
    var many: [Inner] = []
    for i in 0..3 {
        many.push(Inner{ n: i, xs: [i, i + 1], s: "s{i}" })
    }
    var cells: [ref i64] = []
    for i in 0..3 {
        cells.push(ref(i * 10))
    }
    Outer{
        one: Inner{ n: 7, xs: [1, 2, 3], s: "seven" },
        many: many,
        cell: ref(100),
        cells: cells,
    }
}
"""

# (name, place expression, type, a value to write)
PLACES = [
    ("local", "n", "i64", "42"),
    ("field", "o.one.n", "i64", "42"),
    ("field-str", "o.one.s", "str", '"written"'),
    ("index", "o.one.xs[1]", "i64", "42"),
    ("field-of-element", "o.many[1].n", "i64", "42"),
    ("index-of-field-of-element", "o.many[1].xs[0]", "i64", "42"),
    ("str-of-element", "o.many[2].s", "str", '"written"'),
    ("ref-at-root", "r.value", "i64", "42"),
    ("ref-in-field", "o.cell.value", "i64", "42"),
    ("ref-in-list", "o.cells[1].value", "i64", "42"),
    ("ref-in-list-last", "o.cells[2].value", "i64", "42"),
    ("local-list", "xs[0]", "i64", "42"),
]

# List-valued places, for the mutating methods. These reach `read_place_boxed`
# and the take-or-copy path rather than the plain read.
LIST_PLACES = [
    ("local-list", "xs"),
    ("field-list", "o.one.xs"),
    ("list-of-element", "o.many[1].xs"),
    ("list-of-last-element", "o.many[2].xs"),
]

LIST_OPS = [
    ("push", '{p}.push(99)\n    print("{{{p}.len()}} {{{p}[{p}.len() - 1]}}")'),
    ("sort", '{p}.push(0)\n    {p}.sort()\n    print("{{{p}[0]}} {{{p}.len()}}")'),
    ("sort-by", '{p}.push(0)\n    {p}.sort_by(|v| 0 - v)\n    print("{{{p}[0]}} {{{p}.len()}}")'),
    ("pop", '{p}.push(5)\n    let v = {p}.pop()\n    print("{{v ?? 0 - 1}} {{{p}.len()}}")'),
    ("remove", '{p}.push(77)\n    let v = {p}.remove(0)\n    print("{{v}} {{{p}.len()}}")'),
    ("inout", 'bump({p})\n    print("{{{p}.len()}} {{{p}[{p}.len() - 1]}}")'),
]

# What to do with it. `{p}` is the place, `{v}` the value to write.
OPS = [
    ("read", 'print("{{{p}}}")'),
    ("write-then-read", '{p} = {v}\n    print("{{{p}}}")'),
    ("compound", "{p} += 1\n    print(\"{{{p}}}\")"),
    ("read-twice", 'print("{{{p}}} {{{p}}}")'),
    ("write-twice", '{p} = {v}\n    {p} = {p}\n    print("{{{p}}}")'),
]

# `+=` is defined for numbers; a string place gets `+` with a string instead.
STR_COMPOUND = '{p} = {p} + "!"\n    print("{{{p}}}")'


def build(place, ty, value, op_body):
    body = op_body.format(p=place, v=value)
    return (
        PRELUDE
        + "\nfn bump(inout ys: [i64]) {\n    ys.push(1234)\n}\n"
    + "\nfn main() / {io} {\n"
        + "    var o = fresh()\n"
        + "    var n = 5\n"
        + "    var xs = [9, 8, 7]\n"
        + "    let r = ref(3)\n"
        + f"    {body}\n"
        + "    release(r)\n"
        + "    release(o.cell)\n"
        + "    for c in o.cells {\n        release(c)\n    }\n"
        + "}\n"
    )


def run(cmd, timeout=180):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="grasm_fuzz_places_")
    path = os.path.join(work, "p.gr")
    exe = os.path.join(work, "p.bin")
    checked, disagreed, declined, rejected = 0, [], [], []
    try:
        for pname, place, ty, value in PLACES:
            for oname, op in OPS:
                if oname == "compound" and ty == "str":
                    op = STR_COMPOUND
                src = build(place, ty, value, op)
                with open(path, "w") as f:
                    f.write(src)
                checked += 1
                label = f"{pname}/{oname}"
                rc, out, err = run([GRASM, "run", path])
                if rc != 0:
                    # The generator is meant to emit well-typed programs, so a
                    # rejection is a generator bug and is reported as loudly
                    # as a mismatch.
                    rejected.append(label)
                    print(f"REJECTED {label}: {(out + err).strip()}\n{src}")
                    continue
                want = out
                rc2, _, err2 = run([GRASM, "build", path, "--native", "-o", exe])
                if rc2 != 0:
                    declined.append(label)
                    continue
                rc3, got, err3 = run([exe])
                if rc3 != 0 or got != want:
                    disagreed.append(label)
                    print(
                        f"DISAGREES {label}:\n  interpreter: {want.strip()!r}\n"
                        f"  native:      {got.strip()!r} {err3.strip()}\n{src}"
                    )
        for pname, place in LIST_PLACES:
            for oname, op in LIST_OPS:
                src = build(place, "list", "[]", op)
                with open(path, "w") as f:
                    f.write(src)
                checked += 1
                label = f"{pname}/{oname}"
                rc, out, err = run([GRASM, "run", path])
                if rc != 0:
                    rejected.append(label)
                    print(f"REJECTED {label}: {(out + err).strip()}\n{src}")
                    continue
                want = out
                rc2, _, err2 = run([GRASM, "build", path, "--native", "-o", exe])
                if rc2 != 0:
                    declined.append(label)
                    continue
                rc3, got, err3 = run([exe])
                if rc3 != 0 or got != want:
                    disagreed.append(label)
                    print(
                        f"DISAGREES {label}:\n  interpreter: {want.strip()!r}\n"
                        f"  native:      {got.strip()!r} {err3.strip()}\n{src}"
                    )
        if declined:
            print(f"  native declined (not a defect): {', '.join(sorted(set(declined)))}")
        print(
            f"programs: {checked}   disagreed: {len(disagreed)}   "
            f"rejected: {len(rejected)}   declined: {len(declined)}"
        )
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if disagreed or rejected else 0)


if __name__ == "__main__":
    main()
