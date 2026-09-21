#!/usr/bin/env python3
"""Randomized differential testing over generics, function values and `inout`.

`fuzz_differential.py` builds programs out of structs, sums, matches, effects
and fallible calls. The seams it does not reach are the ones where the native
backend has to work out a type it was never told: a generic body instantiated
at two types, a function held in a binding and called later, a closure passed
through a helper, and a place written back through `inout`. Those are exactly
where the AOT backend's type probe has had to guess, and a guess is where the
two engines drift apart.

Every program is well-typed by construction — the generator tracks the type of
each expression as it builds it, so a rejected program is a generator bug and
is reported as loudly as a mismatch. Programs are total: no division, no index,
no clock, no environment. The only thing left to disagree about is codegen.

    scripts/fuzz_generics.py [--count N] [--seed N] [--keep]

A mismatch prints the seed, and re-running with that seed reproduces exactly
the same program.
"""

import argparse
import os
import random
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Overridable so the fuzzer can be pointed at a debug build, where Cranelift
# keeps its own assertions on and the runtime checks integer overflow — both
# of which a release build compiles out.
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
# One binary, rebuilt in place, instead of one file per program (see the module).
build_native = _lib("inplace").build


_spec = _ilu.spec_from_file_location(
    "grasm_fresh",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", "fresh.py"),
)
_fresh = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_fresh)
_fresh.fresh_or_stop(GRASM)


INT, STR, BOOL = "i64", "str", "bool"

# Generic and higher-order helpers, written once at the top of every program.
# They are fixed rather than generated: the point is to exercise how the
# backends *instantiate* and *call* them, not to invent new signatures.
PRELUDE = """\
type Box = { n: i64, tag: str }

fn ident[T](x: T) -> T {
    x
}

fn firstOr[T](xs: [T], d: T) -> T {
    if xs.len() > 0 {
        xs[0]
    } else {
        d
    }
}

fn count[T](xs: [T]) -> i64 {
    xs.len()
}

fn twice[T](f: fn(T) -> T, x: T) -> T {
    f(f(x))
}

fn apply_all[T](f: fn(T) -> T, xs: [T]) -> [T] {
    xs.map(f)
}

fn pick[T](c: bool, a: T, b: T) -> T {
    if c {
        a
    } else {
        b
    }
}

fn bump(inout n: i64) {
    n += 1
}

fn scale(inout n: i64, by: i64) {
    n = n * by
}

fn relabel(inout b: Box, s: str) {
    b.tag = b.tag + s
}

fn add1(n: i64) -> i64 {
    n + 1
}

fn dbl(n: i64) -> i64 {
    n * 2
}

fn shout(s: str) -> str {
    s + "!"
}

fn trim1(s: str) -> str {
    if s.len() > 0 {
        s
    } else {
        "e"
    }
}
"""

INT_FNS = ["add1", "dbl"]
STR_FNS = ["shout", "trim1"]


class Gen:
    """Builds a well-typed program, tracking what is in scope and its type."""

    def __init__(self, rng):
        self.rng = rng
        self.scope = []      # (name, type)
        self.next_id = 0
        self.depth = 0

    def show(self, out, expr, ty):
        """Prints an expression by binding it first.

        A `"` inside a `"{...}"` interpolation ends the string, and a `{`
        inside one ends the interpolation — so nothing built here is written
        directly between the braces. The language reports both clearly; it is
        the generator that has to keep out of them.
        """
        name = self.fresh("p")
        out.append(f"    let {name} = {expr}")
        self.scope.append((name, ty))
        out.append(f'    print("{{{name}}}")')

    def fresh(self, prefix="v"):
        self.next_id += 1
        return f"{prefix}{self.next_id}"

    def in_scope(self, ty):
        return [n for n, t in self.scope if t == ty]

    # --- leaves ----------------------------------------------------------

    def lit(self, ty):
        if ty == INT:
            return str(self.rng.randint(-40, 40))
        if ty == STR:
            return '"' + self.rng.choice(["a", "bc", "", "zz", "q9"]) + '"'
        return self.rng.choice(["true", "false"])

    def atom(self, ty):
        have = self.in_scope(ty)
        if have and self.rng.random() < 0.6:
            return self.rng.choice(have)
        return self.lit(ty)

    # --- expressions -----------------------------------------------------

    def expr(self, ty):
        if self.depth > 3:
            return self.atom(ty)
        self.depth += 1
        try:
            choices = {INT: self.int_expr, STR: self.str_expr, BOOL: self.bool_expr}
            return choices[ty]()
        finally:
            self.depth -= 1

    def int_expr(self):
        pick = self.rng.randrange(9)
        if pick == 0:
            return f"({self.expr(INT)} + {self.expr(INT)})"
        if pick == 1:
            return f"({self.expr(INT)} * {self.rng.randint(-3, 3)})"
        if pick == 2:
            return f"ident({self.expr(INT)})"
        if pick == 3:
            return f"twice({self.rng.choice(INT_FNS)}, {self.expr(INT)})"
        if pick == 4:
            return f"count([{self.expr(INT)}, {self.expr(INT)}])"
        if pick == 5:
            return f"firstOr([{self.expr(INT)}], {self.expr(INT)})"
        if pick == 6:
            return f"pick({self.expr(BOOL)}, {self.expr(INT)}, {self.expr(INT)})"
        if pick == 7:
            # A closure built here and applied through a generic helper.
            p = self.fresh("c")
            return f"twice(|{p}| {p} + {self.rng.randint(-4, 4)}, {self.expr(INT)})"
        return self.atom(INT)

    def str_expr(self):
        pick = self.rng.randrange(7)
        if pick == 0:
            return f"({self.expr(STR)} + {self.expr(STR)})"
        if pick == 1:
            return f"ident({self.expr(STR)})"
        if pick == 2:
            return f"twice({self.rng.choice(STR_FNS)}, {self.expr(STR)})"
        if pick == 3:
            return f"firstOr([{self.expr(STR)}], {self.expr(STR)})"
        if pick == 4:
            return f"pick({self.expr(BOOL)}, {self.expr(STR)}, {self.expr(STR)})"
        if pick == 5:
            p = self.fresh("c")
            return f"twice(|{p}| {p} + \"x\", {self.expr(STR)})"
        return self.atom(STR)

    def bool_expr(self):
        pick = self.rng.randrange(5)
        if pick == 0:
            return f"({self.expr(INT)} > {self.expr(INT)})"
        if pick == 1:
            return f"({self.expr(STR)} == {self.expr(STR)})"
        if pick == 2:
            return f"(not {self.expr(BOOL)})"
        if pick == 3:
            return f"pick({self.atom(BOOL)}, {self.expr(BOOL)}, {self.expr(BOOL)})"
        return self.atom(BOOL)

    # --- statements ------------------------------------------------------

    def stmt(self, out):
        pick = self.rng.randrange(10)
        ty = self.rng.choice([INT, STR, BOOL])

        if pick == 0:
            name = self.fresh()
            out.append(f"    let {name} = {self.expr(ty)}")
            self.scope.append((name, ty))
            return

        if pick == 1:
            # `inout` on a plain local, which is the write-back path.
            name = self.fresh("m")
            out.append(f"    var {name} = {self.expr(INT)}")
            self.scope.append((name, INT))
            out.append(f"    bump({name})")
            if self.rng.random() < 0.5:
                out.append(f"    scale({name}, {self.rng.randint(-3, 3)})")
            return

        if pick == 2:
            # `inout` on a struct field's place.
            name = self.fresh("b")
            out.append(
                f'    var {name} = Box{{ n: {self.expr(INT)}, tag: {self.expr(STR)} }}'
            )
            out.append(f"    bump({name}.n)")
            out.append(f'    relabel({name}, "z")')
            out.append(f'    print("{{{name}.n}} {{{name}.tag}}")')
            self.scope.append((name, "Box"))
            return

        if pick == 3:
            # A function value held in a binding, then called.
            name = self.fresh("f")
            fn = self.rng.choice(INT_FNS)
            out.append(f"    let {name} = {fn}")
            self.show(out, f"{name}({self.expr(INT)})", INT)
            return

        if pick == 4:
            # A list of function values, applied through a generic helper.
            fns = ", ".join(self.rng.sample(INT_FNS, 2))
            xs = self.fresh("xs")
            out.append(f"    let {xs} = [{fns}]")
            self.show(out, f"{xs}.len()", INT)
            self.show(out, f"{xs}[0]({self.expr(INT)})", INT)
            return

        if pick == 5:
            xs = self.fresh("l")
            items = ", ".join(self.expr(INT) for _ in range(self.rng.randint(1, 3)))
            out.append(f"    let {xs} = [{items}]")
            name = self.fresh("q")
            out.append(f"    let {name} = apply_all({self.rng.choice(INT_FNS)}, {xs})")
            out.append(f'    print("{{{name}}}")')
            self.show(out, f"count({xs})", INT)
            return

        if pick == 6:
            xs = self.fresh("s")
            items = ", ".join(self.expr(STR) for _ in range(self.rng.randint(1, 3)))
            out.append(f"    let {xs} = [{items}]")
            p = self.fresh("c")
            grown = self.fresh("g")
            out.append(f'    let {grown} = {xs}.map(|{p}| {p} + "x")')
            self.show(out, f"{grown}.len()", INT)
            self.show(out, f'firstOr({xs}, "z")', STR)
            return

        if pick == 7:
            # A fold whose accumulator type is not the element type.
            xs = self.fresh("g")
            items = ", ".join(self.expr(INT) for _ in range(self.rng.randint(1, 3)))
            out.append(f"    let {xs} = [{items}]")
            a, b = self.fresh("a"), self.fresh("b")
            self.show(out, f"{xs}.fold(0, |{a}, {b}| {a} + {b})", INT)
            return

        if pick == 8:
            cond = self.expr(BOOL)
            out.append(f"    if {cond} {{")
            saved = len(self.scope)
            self.stmt(out)
            del self.scope[saved:]
            out.append("    } else {")
            self.stmt(out)
            del self.scope[saved:]
            out.append("    }")
            return

        self.show(out, self.expr(ty), ty)

    def program(self):
        body = []
        for _ in range(self.rng.randint(4, 9)):
            self.stmt(body)
        # One last line so a program that generated only bindings still prints.
        self.show(body, self.expr(INT), INT)
        self.show(body, self.expr(STR), STR)
        return PRELUDE + "\nfn main() {\n" + "\n".join(body) + "\n}\n"


def run(cmd, cwd=None):
    p = subprocess.run(cmd, capture_output=True, text=True, cwd=cwd, timeout=120)
    return p.returncode, p.stdout, p.stderr


def check_one(src, work):
    """Returns None if the three engines agree, or a description of the drift."""
    path = os.path.join(work, "p.gr")
    with open(path, "w") as f:
        f.write(src)

    # `check` is not run up front. Both builds below typecheck this very same
    # program on their way to code, so a build that succeeded has already said
    # it was well-typed, and a check spawn ahead of them spent a seventh of the
    # run re-deriving it. It runs where the answer is actually in doubt: a
    # build that failed, which has to say whether the generator emitted an
    # invalid program or the compiler failed on a valid one.
    ir, io, ie = run([GRASM, "run", path])

    bundle = os.path.join(work, "p.bundle")
    bp = build_native(GRASM, path, bundle)
    rc, err = bp.returncode, bp.stderr
    if rc != 0:
        crc, _, cerr = run([GRASM, "check", path])
        if crc != 0:
            return ("rejected", cerr.strip())
        return ("bundle build failed", err.strip())
    br, bo, be = run([bundle])

    native = os.path.join(work, "p.native")
    bp = build_native(GRASM, path, native, "--native")
    rc, err = bp.returncode, bp.stderr
    if rc != 0:
        if "does not yet support" in err:
            # Counted, not swallowed. An unlabelled `None` here is
            # indistinguishable from a program that agreed on every engine, so a
            # change that made `--native` decline the whole corpus would read as
            # a clean run.
            return ("declined", err.strip())
        return ("native build failed", err.strip())
    nr, no, ne = run([native])
    # A generic instantiation boxes and unboxes at each member's own
    # representation, so a miscount is per-instantiation and invisible in the
    # output (F57).
    held = leaked(native) if nr == 0 else None
    if held:
        return ("native leaks", held)

    if (ir, io, ie) != (br, bo, be):
        return ("bundle disagrees", f"run={ir!r},{io!r},{ie!r}\nbundle={br!r},{bo!r},{be!r}")
    if (ir, io) != (nr, no):
        return ("native disagrees", f"run={ir!r},{io!r}\nnative={nr!r},{no!r}")
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=500)
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    base = args.seed if args.seed is not None else random.randrange(1 << 30)
    failures = []
    rejected = []
    declined = []
    work = tempfile.mkdtemp(prefix="grasm_fuzz_generics_")
    try:
        for i in range(args.count):
            seed = base + i
            src = Gen(random.Random(seed)).program()
            verdict = check_one(src, work)
            if verdict is None:
                continue
            kind, detail = verdict
            if kind == "declined":
                declined.append(detail)
            elif kind == "rejected":
                rejected.append((seed, detail))
            else:
                failures.append((seed, kind, detail))
                keep = os.path.join(work, f"fail_{seed}.gr")
                with open(keep, "w") as f:
                    f.write(src)
                print(f"MISMATCH seed={seed} ({kind})\n{detail}\n  kept: {keep}")
            if (i + 1) % 50 == 0:
                print(f"  {i + 1}/{args.count}  mismatches={len(failures)}")

        for seed, detail in rejected[:5]:
            print(f"REJECTED seed={seed}: {detail.splitlines()[0] if detail else ''}")
        print(
            f"programs: {args.count}   mismatched: {len(failures)}   "
            f"rejected: {len(rejected)}   declined: {len(declined)}"
        )
    finally:
        if not args.keep and not failures:
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
