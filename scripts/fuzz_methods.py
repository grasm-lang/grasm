#!/usr/bin/env python3
"""Every builtin method, on every shape of receiver, on both engines.

F17 is the reason this exists. `byte_len` was added to the native backend's
method table and *not* to the table beside it that says what each method
returns, whose last arm was a catch-all. The compiled program called `.len()`
on the integer 10 as though it were a pointer and took SIGSEGV, while
`grasm run` printed the right answer.

The unit test written alongside the feature passed, because a **literal**
receiver takes a different path in the backend. It took a real program with a
loop variable to expose it. So the grid here is method × *receiver shape*: the
same call written six ways, which is the axis that hid the defect.

One program per (type, shape), printing every method of that type on its own
line, compared against the interpreter. A native build that declines is
reported and not failed; one that answers differently is the defect.

    scripts/fuzz_methods.py [--keep]
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

# (type, a literal of that type, [method call written against `R`])
GROUPS = [
    (
        "str",
        '"Hello, World"',
        [
            "R.len()", "R.byte_len()", "R.is_empty()", "R.substr(1, 3)",
            "R.chars().len()", "R.lines().len()", 'R.split(",").len()',
            "R.words().len()", 'R.contains("World")', 'R.starts_with("He")',
            'R.ends_with("ld")', 'R.index_of("W") ?? 0 - 1', 'R.replace("l", "L")',
            "R.trim()", "R.trim_start()", "R.trim_end()", "R.to_upper()",
            "R.to_lower()", "R.repeat(2).len()", 'R.pad_start(20, "-")',
            'R.pad_end(20, "-")', "R.parse_int() ?? 0 - 1",
            "R.parse_int_base(16) ?? 0 - 1", "R.parse_float() ?? 0.0 - 1.0",
            "R.code_at(0)", "R.codes().len()", "R.to_bytes().len()",
        ],
    ),
    (
        "i64",
        "0 - 42",
        [
            "R.abs()", "R.as_f64()", "R.to_str()", "R.min(7)", "R.max(7)",
            "R.cmp(7)", "R.to_base(16)", "R.to_base(2)", "R.bit_and(12)",
            "R.bit_or(12)", "R.bit_xor(12)", "R.bit_not()", "R.shl(2)",
            "R.shr(2)", "R.ushr(2)", "R.abs().to_char()",
        ],
    ),
    (
        "f64",
        "2.25",
        [
            "R.abs()", "R.as_i64()", "R.to_str()", "R.sqrt()", "R.floor()",
            "R.ceil()", "R.round()", "R.is_nan()", "R.is_finite()",
            "R.to_fixed(3)", "R.ln()", "R.log10()", "R.log2()", "R.exp()",
            "R.sin()", "R.cos()", "R.tan()", "R.hypot(1.0)", "R.atan2(1.0)",
            "R.to_degrees()", "R.to_radians()", "R.cbrt()", "R.log(2.0)",
            "R.sinh()", "R.cosh()", "R.tanh()",
            "(R / 4.0).asin()", "(R / 4.0).acos()", "R.atan()",
        ],
    ),
    (
        "list",
        "[3, 1, 2]",
        [
            "R.len()", "R.is_empty()", "R.get(0) ?? 0 - 1", "R.first() ?? 0 - 1",
            "R.last() ?? 0 - 1", "R.rev().len()", "R.slice(0, 2).len()",
            "R.contains(3)", "R.sum()", "R.min() ?? 0 - 1", "R.max() ?? 0 - 1",
            "R.fold(0, |a, b| a + b)", "R.map(|v| v * 2).len()",
            "R.filter(|v| v > 1).len()", "R.any(|v| v > 2)", "R.all(|v| v > 0)",
            "R.find(|v| v > 1) ?? 0 - 1", "R.enumerate().len()",
            "R.zip([1, 2]).len()", "R.par_map(|v| v + 1).len()",
            "R.par_filter(|v| v > 1).len()",
            "R.map(|v| v.to_str()).join(\"-\")",
            "R.enumerate().len()",
        ],
    ),
    (
        "map",
        "built()",
        [
            "R.len()", "R.is_empty()", 'R.get("a") ?? 0 - 1', 'R.has("a")',
            "R.keys().len()", "R.values().len()", "R.entries().len()",
            'R.keys().join(",")',
        ],
    ),
    (
        "option",
        "Some(7)",
        [
            "R.is_some()", "R.is_none()", "R.unwrap_or(0 - 1)",
            'R.expect("must be there")', "R ?? 0 - 1",
        ],
    ),
    (
        "bytes",
        "bytes_of([1, 2, 255])",
        [
            "R.len()", "R.is_empty()", "R.at(2)", "R.slice(0, 2).len()",
            "R.concat(bytes_of([9])).len()", "R.to_list().len()",
            "R.to_str().len()",
        ],
    ),
]

# How the receiver is spelled. `{lit}` is the literal, `{calls}` the printing
# body written against `R`.
SHAPES = {
    "literal": "    let out = [\n{calls}\n    ]\n    print(out.join(\"|\"))\n",
    "local": "    let R = {lit}\n    let out = [\n{calls}\n    ]\n    print(out.join(\"|\"))\n",
    "var-local": "    var R = {lit}\n    let out = [\n{calls}\n    ]\n    print(out.join(\"|\"))\n",
    # The shape that exposed F17: nothing in the backend's fast paths sees a
    # literal here.
    "loop-var": "    for R in [{lit}] {{\n        let out = [\n{calls}\n        ]\n        print(out.join(\"|\"))\n    }}\n",
    "field": "    let holder = Box{{ v: {lit} }}\n    let R = holder.v\n    let out = [\n{calls}\n    ]\n    print(out.join(\"|\"))\n",
    "element": "    let holder = [{lit}]\n    let R = holder[0]\n    let out = [\n{calls}\n    ]\n    print(out.join(\"|\"))\n",
}


PRELUDE = """fn built() -> Map[str, i64] {
    var m: Map[str, i64] = map()
    m.set("a", 1)
    m.set("b", 2)
    m
}

"""


def build(ty, lit, calls, shape):
    # Every result is interpolated, so one program covers every method and a
    # disagreement names the position.
    lines = ",\n".join(f'        "{{{c}}}"' for c in calls)
    if shape == "literal":
        # Every mention of the receiver, not only the ones a `.` follows:
        # `(R / 4.0).asin()` and `R ?? 0 - 1` name it bare.
        lines = re.sub(r"\bR\b", f"({lit})", lines)
    body = SHAPES[shape].format(lit=lit, calls=lines)
    decl = f"type Box = {{ v: {ty_decl(ty)} }}\n\n" if shape == "field" else ""
    return PRELUDE + decl + "fn main() / {io} {\n" + body + "}\n"


def ty_decl(ty):
    return {
        "str": "str",
        "i64": "i64",
        "f64": "f64",
        "list": "[i64]",
        "bytes": "bytes",
        "map": "Map[str, i64]",
        "option": "Option[i64]",
    }[ty]


def run(cmd, timeout=180):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def spec_methods():
    """Every method the library spec's tables name, so the grid cannot drift."""
    text = open(os.path.join(ROOT, "spec", "library.md")).read()
    return {m.group(2) for m in re.finditer(r"`([a-z]+)\.([a-z_0-9]+)\(", text)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="grasm_fuzz_methods_")
    path = os.path.join(work, "p.gr")
    exe = os.path.join(work, "p.bin")
    checked, disagreed, declined, rejected = 0, [], [], []
    covered = set()
    try:
        for ty, lit, calls in GROUPS:
            for c in calls:
                covered.update(re.findall(r"\.([a-z_0-9]+)\(", c))
            for shape in SHAPES:
                src = build(ty, lit, calls, shape)
                with open(path, "w") as f:
                    f.write(src)
                checked += 1
                label = f"{ty}/{shape}"
                rc, out, err = run([GRASM, "run", path])
                if rc != 0:
                    rejected.append(label)
                    print(f"REJECTED {label}: {(out + err).strip()}\n{src}")
                    continue
                want = out
                rc2, _, err2 = run([GRASM, "build", path, "--native", "-o", exe])
                if rc2 != 0:
                    first = err2.strip().splitlines()[0] if err2.strip() else ""
                    # The backend distinguishes these itself: "does not yet
                    # support X" is a named gap, and "codegen bug" is a
                    # verifier failure the compiler is reporting against
                    # itself. Counting the second as a decline is how
                    # `b.is_empty()` failed to compile in every program in the
                    # language while this gate said nothing.
                    if "codegen bug" in first:
                        disagreed.append(label)
                        print(f"CODEGEN BUG {label}: {first}")
                    else:
                        declined.append(f"{label}: {first}")
                    continue
                rc3, got, err3 = run([exe])
                if rc3 != 0 or got != want:
                    disagreed.append(label)
                    # Name the position rather than dumping two long lines.
                    w, g = want.strip().split("|"), got.strip().split("|")
                    for i, (a, b) in enumerate(zip(w, g)):
                        if a != b:
                            which = calls[i] if i < len(calls) else "?"
                            print(f"DISAGREES {label} at {which}: run={a!r} native={b!r}")
                    if len(w) != len(g):
                        print(f"DISAGREES {label}: {len(w)} vs {len(g)} results")
        # The mutating methods are covered by `fuzz_places.py`, which crosses
        # them with the place shapes that matter for a write. `f` is the regex
        # reading `x.f` out of the spec's prose, and the rest are free
        # functions rather than methods.
        elsewhere = {
            "push", "pop", "remove", "set", "delete", "sort", "sort_by",
            "map", "ref", "release", "alive", "f",
        }
        missing = sorted(spec_methods() - covered - elsewhere)
        if missing:
            print(f"  NOT COVERED by this grid: {', '.join(missing)}")
        for d in declined:
            print(f"  native declined (not a defect): {d}")
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
