#!/usr/bin/env python3
"""The native backend's fusions, against the interpreter that has none.

Two shapes are compiled into a loop rather than through the general path:

  * `(a..b).map(f)`, `.filter(f)` and their chains — `fused_range_build`
    inlines the closure body into a `for`, so no list of the range is built
    and no closure is called;
  * `m.get(k) ?? d` — the map lookup and the default are fused, and the
    default must stay **lazy**: an eager one broke `??`'s laziness and was
    caught only by a conformance program whose default diverged.

An optimisation is a promise that two paths give the same answer, and the
interpreter has neither of these, so it is the oracle by construction. What
this adds over the existing gates is breadth: empty and single-element ranges,
negative bounds, every scalar map value type, present and absent keys, a
default with a side effect, and a closure that diverges part-way through.

    scripts/fuzz_fusions.py [--keep]
"""

import argparse
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")

RANGES = ["0..0", "0..1", "3..3", "0..5", "2..7", "0..20", "0 - 3..3", "0 - 5..0 - 2"]

RANGE_OPS = [
    "R.map(|v| v * 2)",
    "R.map(|v| v + k)",
    "R.map(|v| v * v)",
    "R.filter(|v| v % 2 == 0)",
    "R.filter(|v| v > k)",
    "R.map(|v| v * 3).filter(|v| v > 4)",
    "R.filter(|v| v % 2 == 0).map(|v| v + 1)",
    "R.map(|v| twice(v))",
    "R.to_list()",
    "R.len()",
    "R.sum()",
    "R.map(|v| v * 2).sum()",
    "R.filter(|v| v != k).len()",
    "R.map(|v| v.to_str()).join(\"-\")",
    "R.map(|v| v > k)",
]

# (declared type, a literal of it, a present key, a default expression)
MAP_KINDS = [
    ("i64", "7", '"a"', "0 - 1"),
    ("f64", "1.5", '"a"', "0.0 - 1.0"),
    ("bool", "true", '"a"', "false"),
    ("str", '"x"', '"a"', '"none"'),
]

PRELUDE = """fn twice(n: i64) -> i64 {
    n * 2
}

fn note(n: i64, log: ref [i64]) -> i64 {
    log.value = log.value + [n]
    n
}

"""


def range_program():
    lines = []
    for r in RANGES:
        for op in RANGE_OPS:
            expr = op.replace("R", f"({r})")
            lines.append(f'    print("{{{expr}}}")')
    return (
        PRELUDE
        + "fn main() / {io} {\n    let k = 2\n"
        + "\n".join(lines)
        + "\n}\n"
    )


def map_program(ty, lit, key, default):
    return (
        PRELUDE
        + "fn main() / {io} {\n"
        + f"    var m: Map[str, {ty}] = map()\n"
        + f"    m.set({key}, {lit})\n"
        + f'    m.set("b", {lit})\n'
        + f'    print("{{m.get({key}) ?? {default}}}")\n'
        + f'    print("{{m.get("zz") ?? {default}}}")\n'
        + f'    print("{{m.get("b") ?? {default}}}")\n'
        # An empty map: the fused path must find nothing and take the default.
        + f"    var e: Map[str, {ty}] = map()\n"
        + f'    print("{{e.get({key}) ?? {default}}}")\n'
        + "}\n"
    )


def lazy_program():
    """The default must not run when the key is present."""
    return (
        PRELUDE
        + "fn main() / {io} {\n"
        + "    var m: Map[str, i64] = map()\n"
        + '    m.set("a", 1)\n'
        + "    let log = ref([0])\n"
        + '    let hit = m.get("a") ?? note(99, log)\n'
        + '    print("present {hit} ran {log.value}")\n'
        + '    let miss = m.get("z") ?? note(98, log)\n'
        + '    print("absent {miss} ran {log.value}")\n'
        + "    release(log)\n}\n"
    )


def diverging_program():
    """A fused loop that panics part-way: same output before it, same status."""
    return (
        PRELUDE
        + "fn main() / {io} {\n"
        + '    print("before")\n'
        + "    print(\"{(0..4).map(|x| 10 / (x - 2))}\")\n"
        + '    print("unreachable")\n}\n'
    )


def run(cmd, timeout=240):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="grasm_fuzz_fusions_")
    path = os.path.join(work, "p.gr")
    exe = os.path.join(work, "p.bin")

    cases = [("range-grid", range_program()), ("lazy-default", lazy_program()),
             ("diverging-fused-map", diverging_program())]
    for ty, lit, key, default in MAP_KINDS:
        cases.append((f"map-get-or/{ty}", map_program(ty, lit, key, default)))

    checked, bad = 0, []
    try:
        for name, src in cases:
            with open(path, "w") as f:
                f.write(src)
            checked += 1
            rc, out, err = run([GRASM, "run", path])
            rc2, _, err2 = run([GRASM, "build", path, "--native", "-o", exe])
            if rc2 != 0:
                bad.append(name)
                print(f"WILL NOT BUILD {name}: {err2.strip().splitlines()[0]}")
                continue
            rc3, got, err3 = run([exe])
            # The exit code matters as much as the output: a fused loop that
            # panics must panic.
            if got != out or rc3 != rc:
                bad.append(name)
                print(f"DISAGREES {name}: exit {rc} vs {rc3}")
                w, g = out.splitlines(), got.splitlines()
                for i in range(max(len(w), len(g))):
                    a = w[i] if i < len(w) else "<nothing>"
                    b = g[i] if i < len(g) else "<nothing>"
                    if a != b:
                        print(f"  line {i + 1}: run={a!r} native={b!r}")
        print(f"programs: {checked}   disagreed: {len(bad)}")
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
