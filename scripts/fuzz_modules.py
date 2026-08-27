#!/usr/bin/env python3
"""Every position a name from another module can appear in.

No generator had ever built a program of more than one file. Everything else
here fuzzes one module, and the whole package system -- the thing that makes a
dependency's reach a compile-time fact -- was covered by four hand-written
examples.

The hole that finding one opened: `type Cb = fn(Thing) -> i64` was "unknown
type `Thing`" whenever `Thing` came from another module, while `fn f(t: Thing)`
on the next line resolved. [T-MODULE-NS] says types share one namespace across
the program and the compiler says so twice over -- `pub` on a type is refused
with the words "every type is already reachable from every module" -- so an
alias was one namespace with a hole in it (F68). It was found by hand, by
writing the first library in this repository that imports another one.

So the grid is: every position a type name can stand in, crossed with how far
away the module that declared it is. Each cell is a whole package tree, built
and run three ways.

The oracle is exact rather than self-consistent. Every program prints a number
that depends on the value crossing the boundary, so a cell that compiles and
answers `0` fails; "it printed something" is what a degenerate implementation
also does.

A second round checks the ceiling. A package that performs an effect its
manifest does not declare must fail to build **in its consumer**, and one that
declares it must build -- both directions, because a checker that refuses
everything satisfies the first alone.

    scripts/fuzz_modules.py [--keep]
"""

import argparse
import importlib.util as _ilu
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")


def _lib(name):
    spec = _ilu.spec_from_file_location(
        name, os.path.join(ROOT, "scripts", "lib", f"{name}.py")
    )
    mod = _ilu.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


leaked = _lib("leaks").leaked
_lib("fresh").fresh_or_stop(GRASM)


# What `far` declares. Everything the grid names comes from here.
FAR = """\
type Thing = { n: i64 }

type Colour = | Red | Green(shade: i64)

type Box[T] = { held: T }

effect Cfg {
    fn value() -> i64
}

pub fn make(n: i64) -> Thing {
    Thing{ n: n }
}

pub fn boxed(n: i64) -> Box[i64] {
    let b: Box[i64] = Box{ held: n }
    b
}

pub fn same[T](v: T) -> T {
    v
}

pub fn pair[A, B](a: A, b: B) -> (A, B) {
    (a, b)
}

pub fn total(xs: [i64]) -> i64 {
    xs.sum()
}

// Performed here, handled by whoever calls it. [T-CAPABILITY] counts what is
// left in a `pub` row for a consumer to handle, so `Cfg` is in this package's
// ceiling -- which the second round below checks in both directions.
pub fn deep() -> i64 / {Cfg} {
    Cfg.value()
}
"""

# Every position a name declared in another module can stand in.
#
# Each cell is (declarations, statements). The declarations go where
# declarations go -- a `fn` or a `type` may not be nested inside another
# function -- and the statements end by binding `answer`, which must come out
# 42. Splitting them is what lets the same cell be planted in an app's `main`
# and in a library's exported function without rewriting it.
POSITIONS = [
    ('parameter',
     """fn take(t: Thing) -> i64 {
    t.n * 2
}""",
     """let answer = take(far.make(21))"""),
    ('return type',
     """fn give(n: i64) -> Thing {
    far.make(n)
}""",
     """let answer = give(21).n * 2"""),
    ('let annotation',
     """""",
     """let t: Thing = far.make(21)
let answer = t.n * 2"""),
    ('list element',
     """""",
     """let ts: [Thing] = [far.make(21)]
let answer = ts[0].n * 2"""),
    ('map value',
     """""",
     """var m: Map[str, Thing] = map()
m.set("k", far.make(21))
let answer = (m.get("k") ?? far.make(0)).n * 2"""),
    ('tuple component',
     """""",
     """let pair: (Thing, i64) = (far.make(21), 2)
let answer = pair.0.n * pair.1"""),
    ('option payload',
     """""",
     """let o: Option[Thing] = Some(far.make(21))
let answer = (o ?? far.make(0)).n * 2"""),
    ('struct field',
     """type Holder = { it: Thing }""",
     """let h = Holder{ it: far.make(21) }
let answer = h.it.n * 2"""),
    ('sum payload',
     """type Wrap = | Empty | Full(t: Thing)""",
     """let w = Full(far.make(21))
let answer = match w {
    Empty -> 0
    Full(t) -> t.n * 2
}"""),
    ('alias to the type',
     """type Item = Thing

fn take_item(t: Item) -> i64 {
    t.n * 2
}""",
     """let answer = take_item(far.make(21))"""),
    ('alias to a function of it',
     """type Reader = fn(Thing) -> i64

fn read(t: Thing) -> i64 {
    t.n * 2
}""",
     """let f: Reader = read
let answer = f(far.make(21))"""),
    ('alias to a list of it',
     """type Batch = [Thing]""",
     """let b: Batch = [far.make(21)]
let answer = b[0].n * 2"""),
    ('alias to the handler type',
     """type Source = CfgHandler

fn ask_through(h: Source) -> i64 / {Cfg} {
    var got = 0
    with Cfg handled_by h {
        got = Cfg.value()
    }
    got
}""",
     """let answer = ask_through(CfgHandler{ value: || 42 })"""),
    ('generic argument',
     """""",
     """let b: Box[i64] = far.boxed(21)
let answer = b.held * 2"""),
    ('generic of it',
     """""",
     """let b: Box[Thing] = Box{ held: far.make(21) }
let answer = b.held.n * 2"""),
    ('sum from another module',
     """""",
     """let c: Colour = Green(21)
let answer = match c {
    Red -> 0
    Green(shade) -> shade * 2
}"""),
    ('fn-typed parameter',
     """fn apply(t: Thing, f: fn(Thing) -> i64) -> i64 {
    f(t)
}""",
     """let answer = apply(far.make(21), |t| t.n * 2)"""),
    ('closure capturing it',
     """""",
     """let t = far.make(21)
let f = || t.n * 2
let answer = f()"""),
    ('handler installed here',
     """fn ask() -> i64 / {Cfg} {
    Cfg.value()
}""",
     """var answer = 0
with Cfg handled_by CfgHandler{ value: || 42 } {
    answer = ask()
}"""),
    ('generic function',
     """""",
     """let answer = far.same(21) * 2"""),
    ('generic function, two parameters',
     """""",
     """let p = far.pair(21, "x")
let answer = p.0 * 2"""),
    ('generic function over the type',
     """""",
     """let t: Thing = far.same(far.make(21))
let answer = t.n * 2"""),
    ('generic call from another module, as a list element',
     """""",
     """let answer = far.total([far.same(21), far.same(0)]) * 2"""),
    ('effect performed in another module',
     """""",
     """var answer = 0
with Cfg handled_by CfgHandler{ value: || 42 } {
    answer = far.deep()
}"""),
    ('ref to it',
     """""",
     """let r = ref(far.make(21))
let answer = r.n * 2"""),
]


def toml(name, deps, caps=None):
    out = f'[package]\nname = "{name}"\nversion = "1.0.0"\n'
    if name != "app":
        out += 'entry = "src/main.gr"\n'
    out += f"capabilities = {caps if caps is not None else []}\n".replace("'", '"')
    for dep in deps:
        out += f'\n[dependencies.{dep}]\npath = "../{dep}"\nversion = "^1.0.0"\n'
    return out


def write(work, name, deps, source, caps=None):
    d = os.path.join(work, name, "src")
    os.makedirs(d, exist_ok=True)
    open(os.path.join(work, name, "grasm.toml"), "w").write(toml(name, deps, caps))
    open(os.path.join(d, "main.gr"), "w").write(source)


def tree(work, where, decls, stmts):
    """A package tree with the case planted `where`.

    "direct": the app imports `far` and names the type itself.
    "through a library": the *library* names it, and the app only calls the
    library -- which is where F68 lived, because a library is the module a
    per-module pass reaches before its dependency.
    "transitive": the app names the type without importing the module that
    declared it, which is what one namespace across the program means.
    """
    shutil.rmtree(work, ignore_errors=True)
    write(work, "far", [], FAR, ["io", "Cfg"])
    ind = lambda t: "\n".join("    " + l if l.strip() else l for l in t.strip("\n").split("\n"))
    if where == "direct":
        write(work, "mid", ["far"], "import far\n\npub fn passthrough() -> i64 { 0 }\n", ["io"])
        write(work, "app", ["far", "mid"],
              "import far\nimport mid\n\n" + decls.strip("\n") + "\n\nfn main() {\n"
              + ind(stmts) + '\n    print("{answer}")\n}\n', ["io"])
    elif where == "through a library":
        write(work, "mid", ["far"],
              "import far\n\n" + decls.strip("\n") + "\n\npub fn compute() -> i64 {\n"
              + ind(stmts) + "\n    answer\n}\n", ["io"])
        write(work, "app", ["mid"],
              'import mid\n\nfn main() {\n    print("{mid.compute()}")\n}\n', ["io"])
    else:  # transitive
        write(work, "mid", ["far"],
              "import far\n\npub fn source(n: i64) -> Thing {\n    far.make(n)\n}\n"
              "pub fn boxed(n: i64) -> Box[i64] {\n    far.boxed(n)\n}\n"
              "pub fn same[T](v: T) -> T {\n    far.same(v)\n}\n"
              "pub fn pair[A, B](a: A, b: B) -> (A, B) {\n    far.pair(a, b)\n}\n"
              "pub fn total(xs: [i64]) -> i64 {\n    far.total(xs)\n}\n"
              "pub fn deep() -> i64 / {Cfg} {\n    far.deep()\n}\n", ["io", "Cfg"])
        reroute = lambda t: (t.replace("far.make", "mid.source").replace("far.boxed", "mid.boxed")
                             .replace("far.same", "mid.same").replace("far.pair", "mid.pair")
                             .replace("far.deep", "mid.deep")
                             .replace("far.total", "mid.total"))
        write(work, "app", ["mid"],
              "import mid\n\n" + reroute(decls.strip("\n")) + "\n\nfn main() {\n"
              + ind(reroute(stmts)) + '\n    print("{answer}")\n}\n', ["io"])
    return os.path.join(work, "app", "src", "main.gr")


def run(cmd, cwd=None):
    p = subprocess.run(cmd, capture_output=True, text=True, cwd=cwd, timeout=120)
    return p.returncode, p.stdout.strip(), p.stderr.strip()


WANT = "42"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="fuzz_modules_")
    programs = 0
    disagreed = []
    refused = []
    wrong = []
    leaks = []
    try:
        for where in ("direct", "through a library", "transitive"):
            for name, decls, stmts in POSITIONS:
                # The handler alias and the installed handler need `Cfg` in the
                # row of whatever holds them, which `compute` cannot declare
                # from here; those two are run where the row is `main`'s.
                # A handler's row belongs to whatever holds it, and
                # `compute` cannot declare `Cfg` from here -- those two cells
                # run where the row is `main`'s.
                if where == "through a library" and "Cfg" in (decls + stmts):
                    continue
                src = tree(work, where, decls, stmts)
                programs += 1
                label = f"{name} / {where}"

                rc, out, err = run([GRASM, "run", src])
                if rc != 0:
                    refused.append(label)
                    print(f"REFUSED {label}\n{err or out}")
                    continue
                if out != WANT:
                    wrong.append(label)
                    print(f"WRONG {label}: interpreter said {out!r}, wanted {WANT!r}")
                    continue

                exe = os.path.join(work, "out")
                rc2, o2, e2 = run([GRASM, "build", src, "--native", "-o", exe])
                if rc2 != 0:
                    refused.append(label + " / native")
                    print(f"NATIVE REFUSED {label}\n{e2 or o2}")
                    continue
                rc3, o3, e3 = run([exe])
                if rc3 != 0 or o3 != out:
                    disagreed.append(label + " / native")
                    print(f"DISAGREES {label}\n  interpreter: {out}\n  native: {o3} {e3}")
                    continue
                held = leaked(exe)
                if held:
                    leaks.append(label)
                    print(f"LEAKS {label}: {held}")

                bundle = os.path.join(work, "bundle")
                rc4, o4, e4 = run([GRASM, "build", src, "-o", bundle])
                if rc4 != 0:
                    refused.append(label + " / bundle")
                    print(f"BUNDLE REFUSED {label}\n{e4 or o4}")
                    continue
                rc5, o5, e5 = run([bundle])
                if rc5 != 0 or o5 != out:
                    disagreed.append(label + " / bundle")
                    print(f"DISAGREES {label}\n  interpreter: {out}\n  bundle: {o5} {e5}")

        # --- the ceiling ------------------------------------------------
        #
        # Both directions. A checker that refuses every package satisfies the
        # first case on its own, which is why the second is here.
        ceiling = 0
        holes = []
        WORLD = (
            'pub fn shout(msg: str) / {io} {\n    print("{msg}")\n}\n',
            'import far\n\nfn main() {\n    far.shout("42")\n}\n',
            "io",
        )
        # A user effect left in a `pub` row counts too: the consumer has to
        # supply a handler, and [T-CAPABILITY] charges the package for what it
        # leaves for the consumer to handle, not only for what the runtime
        # discharges.
        OWN = (
            "effect Cfg {\n    fn value() -> i64\n}\n\n"
            "pub fn deep() -> i64 / {Cfg} {\n    Cfg.value()\n}\n",
            "import far\n\nfn main() {\n    var n = 0\n"
            "    with Cfg handled_by CfgHandler{ value: || 42 } {\n"
            '        n = far.deep()\n    }\n    print("{n}")\n}\n',
            "Cfg",
        )
        for lib, app, effect in (WORLD, OWN):
            for caps, must_build in (([effect], True), ([], False)):
                shutil.rmtree(work, ignore_errors=True)
                write(work, "far", [], lib, caps)
                write(work, "app", ["far"], app, ["io"])
                src = os.path.join(work, "app", "src", "main.gr")
                rc, out, err = run([GRASM, "run", src])
                ceiling += 1
                built = rc == 0
                if built != must_build:
                    holes.append(f"{effect}/capabilities={caps}")
                    if must_build:
                        print(f"CEILING refuses a package that declares `{effect}`:\n{err or out}")
                    else:
                        print(f"CEILING lets a package perform `{effect}` it never declared")
                elif built and out != WANT:
                    holes.append(f"{effect}/capabilities={caps}/answer")
                    print(f"CEILING wrong answer for `{effect}`: {out!r}")

        print(
            f"programs: {programs}   disagreed: {len(disagreed)}   "
            f"refused: {len(refused)}   wrong: {len(wrong)}   leaked: {len(leaks)}   "
            f"ceiling cases: {ceiling}   holes: {len(holes)}"
        )
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if disagreed or refused or wrong or leaks or holes else 0)


if __name__ == "__main__":
    main()
