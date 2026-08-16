#!/usr/bin/env python3
"""Randomized soundness testing for effect rows, in both directions.

The capability ceiling rests on one property: a function value's effect row
travels with it, whatever carries it and whatever binds it. Five consecutive
cold-evaluation rounds found a hole in that property — `for` instead of `let`,
a `type` alias, a sum-variant payload, a struct field, `??`, a name shadowed by
a parameter — and each was one route out of a grid nobody had enumerated.

This enumerates it. A program is built from three independent choices:

    carrier   how the function value is held: a list, a tuple, a struct field,
              a sum payload, a map value, an `Option`, an alias, a bare value
    binder    how it is taken back out: `let`, `for`, a `match` payload, an
              index, a field read, `??`, a closure parameter
    site      where it is finally called: directly, through `map`, through
              `par_map`, or through a helper that takes a function

Each combination is generated twice, and the oracle is exact both ways:

  * with an **effectful** function inside, `grasm check` must *fail* — the
    enclosing function declares `/ {}`, so a row that arrives is an error;
  * with a **pure** one, it must *succeed* — refusing that is a false
    positive, which is the direction the tightening of the last rounds kept
    erring in.

A generated program that fails its oracle prints the whole source, since the
grid position alone does not say what to look at.

    scripts/fuzz_rows.py [--count N] [--seed N]
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

# Two functions of one type, one of which prints. Which one a program uses is
# the whole experiment.
FNS = """\
fn shout(n: i64) -> i64 {
    print("shout {n}")
    n
}

fn dbl(n: i64) -> i64 {
    n * 2
}
"""

# `held` is the expression that holds the function value; `taken` is the
# statement that binds it back out as `g`. The pair has to agree on shape,
# so they are chosen together.
CARRIERS = [
    # (name, declarations, how to build it from FN, how to bind `g` out of it)
    (
        "list",
        "",
        "let box: [fn(i64) -> i64 {row}] = [{fn}]",
        "let g = box[0]",
    ),
    (
        "list-for",
        "",
        "let box: [fn(i64) -> i64 {row}] = [{fn}]",
        "var g = dbl\n    for f in box {{\n        g = f\n    }}",
    ),
    (
        "tuple",
        "",
        "let box: (i64, fn(i64) -> i64 {row}) = (1, {fn})",
        "let g = box.1",
    ),
    (
        "struct",
        "type Box = {{ f: fn(i64) -> i64 {row} }}\n",
        "let box = Box{{ f: {fn} }}",
        "let g = box.f",
    ),
    (
        "struct-alias",
        "type Cb = fn(i64) -> i64 {row}\ntype Box = {{ f: Cb }}\n",
        "let box = Box{{ f: {fn} }}",
        "let g = box.f",
    ),
    (
        "sum",
        "type Holder = | Hold(f: fn(i64) -> i64 {row}) | Nil\n",
        "let box = Hold({fn})",
        "let g = match box {{\n        Hold(f) -> f\n        Nil -> dbl\n    }}",
    ),
    (
        "map",
        "",
        "let box: Map[str, fn(i64) -> i64 {row}] = {{\"k\": {fn}}}",
        "let g = box.get(\"k\") ?? dbl",
    ),
    (
        "option",
        "",
        "let box: Option[fn(i64) -> i64 {row}] = Some({fn})",
        "let g = box ?? dbl",
    ),
    (
        "option-match",
        "",
        "let box: Option[fn(i64) -> i64 {row}] = Some({fn})",
        "let g = match box {{\n        Some(f) -> f\n        None -> dbl\n    }}",
    ),
    (
        "alias",
        "type Cb = fn(i64) -> i64 {row}\n",
        "let box: [Cb] = [{fn}]",
        "let g = box[0]",
    ),
    (
        "bare",
        "",
        "let box: fn(i64) -> i64 {row} = {fn}",
        "let g = box",
    ),
    # A field read on the left of `??`. Only a bare identifier there was
    # marked unaccounted, so the pure default on the right looked like the
    # whole story.
    (
        "field-coalesce",
        "type Bx = {{ opt: Option[fn(i64) -> i64 {row}] }}\n",
        "let box = Bx{{ opt: Some({fn}) }}",
        "let g = box.opt ?? dbl",
    ),
    # A method call on the left, for the same reason.
    (
        "method-coalesce",
        "",
        "let box: [fn(i64) -> i64 {row}] = [{fn}]",
        "let g = box.first() ?? dbl",
    ),
    # A `??` with no binding at all between it and the call site.
    (
        "inline-coalesce",
        "",
        "let box: Option[fn(i64) -> i64 {row}] = Some({fn})",
        "let g = box ?? dbl",
    ),
]

# How `g` is finally called, inside a function declaring `/ {}`.
#
# Nothing here prints: `quiet` declares an empty row, so a `print` of its own
# would be the error every case reports and the experiment would measure
# nothing.
SITES = [
    ("direct", "    let out = g(1)"),
    ("map", "    let out = [1, 2].map(g)"),
    ("par_map", "    let out = [1, 2].par_map(g)"),
    ("helper", "    let out = call(g)"),
    ("closure", "    let out = [1, 2].map(|x| g(x))"),
]

HELPER = """\
fn call(f: fn(i64) -> i64) -> i64 {
    f(1)
}
"""


def build(carrier, site, fn):
    """One program: the carrier holds `fn`, the site calls what came out.

    The row written on the slot is part of the experiment. A *pure* function
    held in a slot declared `/ {io}` is charged `io` and rightly so — the
    written type is the contract ([T-FN-TYPE-ROW-CARRIED]) — so the two halves
    differ in the written row as well as in the value.
    """
    name, decls, held, taken = carrier
    site_name, body = site
    row = " / {io}" if fn == "shout" else ""
    src = FNS
    if site_name == "helper":
        src += "\n" + HELPER
    if decls:
        src += "\n" + decls.format(row=row)
    src += "\nfn quiet() / {} {\n    "
    src += held.format(fn=fn, row=row) + "\n    "
    src += taken.format(row=row) + "\n"
    src += body + "\n}\n"
    src += "\nfn main() {\n    quiet()\n}\n"
    return src


LIB_MANIFEST = """\
[package]
name = "lib"
version = "0.1.0"
capabilities = []
"""

APP_MANIFEST = """\
[package]
name = "app"
version = "0.1.0"
capabilities = ["io"]

[dependencies.lib]
path = "../lib"
"""

APP_SRC = """\
import lib

fn main() {
    print("{lib.quiet()}")
}
"""


def build_package(carrier, fn, root):
    """The same carrier, exported from a `capabilities = []` package.

    The single-file half asks whether a declared `/ {}` row holds. This asks
    the question the ceiling is actually for: a dependency that quietly starts
    performing an effect has to fail the build in its *consumer*.
    """
    name, decls, held, taken = carrier
    row = " / {io}" if fn == "shout" else ""
    src = FNS
    if decls:
        src += "\n" + decls.format(row=row)
    src += "\npub fn quiet() -> i64 {\n    "
    src += held.format(fn=fn, row=row) + "\n    "
    src += taken.format(row=row) + "\n"
    src += "    g(1)\n}\n"
    for pkg, manifest, entry in (
        ("lib", LIB_MANIFEST, src),
        ("app", APP_MANIFEST, APP_SRC),
    ):
        d = os.path.join(root, pkg, "src")
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(root, pkg, "grasm.toml"), "w") as f:
            f.write(manifest)
        with open(os.path.join(d, "main.gr"), "w") as f:
            f.write(entry)
    return src


def check_package(root):
    p = subprocess.run(
        [GRASM, "deps", "app"],
        cwd=root,
        capture_output=True,
        text=True,
        timeout=120,
    )
    return p.returncode == 0, (p.stdout + p.stderr).strip()


def check(src, path):
    with open(path, "w") as f:
        f.write(src)
    p = subprocess.run(
        [GRASM, "check", path], capture_output=True, text=True, timeout=120
    )
    return p.returncode == 0, (p.stdout + p.stderr).strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=0, help="0 runs the whole grid")
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument(
        "--packages",
        action="store_true",
        help="also run each carrier across a package boundary",
    )
    args = ap.parse_args()

    grid = [(c, s) for c in CARRIERS for s in SITES]
    if args.count:
        rng = random.Random(args.seed if args.seed is not None else 0)
        grid = [rng.choice(grid) for _ in range(args.count)]

    work = tempfile.mkdtemp(prefix="grasm_fuzz_rows_")
    path = os.path.join(work, "p.gr")
    leaks, false_positives, checked = [], [], 0
    try:
        for carrier, site in grid:
            where = f"{carrier[0]}/{site[0]}"

            # An effectful value must be refused.
            src = build(carrier, site, "shout")
            ok, out = check(src, path)
            checked += 1
            if ok:
                leaks.append((where, src))
                print(f"LEAK {where}: an `io` function reached a `/ {{}}` body\n{src}")

            # A pure one must be accepted.
            src = build(carrier, site, "dbl")
            ok, out = check(src, path)
            checked += 1
            if not ok:
                false_positives.append((where, src, out))
                print(f"FALSE POSITIVE {where}: {out.splitlines()[0]}\n{src}")

        # The same carriers across a package boundary, once each: the call
        # site does not vary the answer there, and a workspace costs more to
        # build than a file.
        if args.packages:
            for carrier, _ in {(c, None) for c, _ in grid}:
                where = f"{carrier[0]}/package"
                root = os.path.join(work, "ws")
                shutil.rmtree(root, ignore_errors=True)
                src = build_package(carrier, "shout", root)
                ok, out = check_package(root)
                checked += 1
                if ok:
                    leaks.append((where, src))
                    print(f"LEAK {where}: a `capabilities = []` package performs `io`\n{src}")
                shutil.rmtree(root, ignore_errors=True)
                src = build_package(carrier, "dbl", root)
                ok, out = check_package(root)
                checked += 1
                if not ok:
                    false_positives.append((where, src, out))
                    print(f"FALSE POSITIVE {where}: {out.splitlines()[0]}\n{src}")

        print(
            f"programs: {checked}   leaked: {len(leaks)}   "
            f"false positives: {len(false_positives)}"
        )
    finally:
        shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if leaks or false_positives else 0)


if __name__ == "__main__":
    main()
