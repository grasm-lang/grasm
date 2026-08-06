#!/usr/bin/env python3
"""Randomized differential testing: native output must equal the interpreter's.

`difftest.sh` holds both engines to 129 programs somebody sat down and wrote.
That is a sample, and a sample only finds the disagreements someone thought
to look for. This generates programs instead, which finds the ones nobody did.

Every program is well-typed by construction — the generator tracks the type of
each expression as it builds it, so a rejected program is a generator bug and
is reported as loudly as a mismatch. Programs are also built to be *total*:
no division by an expression that could be zero, no index that could be out of
range, no clock, no environment. The only thing left to disagree about is
codegen.

    scripts/fuzz_differential.py [--count N] [--seed N] [--keep]

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
GRASM = os.path.join(ROOT, "target", "release", "grasm")

INT, FLOAT, BOOL, STR = "i64", "f64", "bool", "str"


class Gen:
    """Builds a well-typed program, tracking what is in scope and its type."""

    def __init__(self, rng):
        self.rng = rng
        self.scope = []          # list of (name, type)
        self.next_id = 0
        self.depth = 0

    def fresh(self, prefix="v"):
        self.next_id += 1
        return f"{prefix}{self.next_id}"

    def in_scope(self, ty):
        return [n for n, t in self.scope if t == ty]

    # --- expressions -----------------------------------------------------

    def expr(self, ty):
        """An expression of exactly `ty`."""
        if self.depth > 3:
            return self.atom(ty)
        self.depth += 1
        try:
            choices = [self.atom]
            if ty == INT:
                choices += [self.int_arith, self.int_cond, self.int_from_list]
            elif ty == FLOAT:
                choices += [self.float_arith, self.float_cond]
            elif ty == BOOL:
                choices += [self.bool_cmp, self.bool_logic]
            elif ty == STR:
                choices += [self.str_concat, self.str_method]
            return self.rng.choice(choices)(ty)
        finally:
            self.depth -= 1

    def atom(self, ty):
        live = self.in_scope(ty)
        if live and self.rng.random() < 0.5:
            return self.rng.choice(live)
        if ty == INT:
            return str(self.rng.randint(-1000, 1000))
        if ty == FLOAT:
            # Values that print without surprises on either engine.
            return f"{self.rng.randint(-1000, 1000)}.{self.rng.randint(0, 99):02d}"
        if ty == BOOL:
            return self.rng.choice(["true", "false"])
        return '"' + self.rng.choice(
            ["a", "bc", "hello", "", "x y", "Zz", "123"]
        ) + '"'

    def int_arith(self, _):
        op = self.rng.choice(["+", "-", "*", "%", "/"])
        lhs = self.expr(INT)
        if op in ("/", "%"):
            # A literal divisor keeps the program total: division by zero is a
            # panic, and a panic is a different experiment from this one.
            d = self.rng.choice([1, 2, 3, 7, 10, -3, -7])
            return f"({lhs} {op} {d})"
        return f"({lhs} {op} {self.expr(INT)})"

    def float_arith(self, _):
        op = self.rng.choice(["+", "-", "*"])
        return f"({self.expr(FLOAT)} {op} {self.expr(FLOAT)})"

    def int_cond(self, _):
        return f"(if {self.expr(BOOL)} {{ {self.expr(INT)} }} else {{ {self.expr(INT)} }})"

    def float_cond(self, _):
        return f"(if {self.expr(BOOL)} {{ {self.expr(FLOAT)} }} else {{ {self.expr(FLOAT)} }})"

    def bool_cmp(self, _):
        ty = self.rng.choice([INT, FLOAT, STR])
        op = self.rng.choice(["==", "!=", "<", "<=", ">", ">="])
        return f"({self.expr(ty)} {op} {self.expr(ty)})"

    def bool_logic(self, _):
        op = self.rng.choice(["and", "or"])
        return f"({self.expr(BOOL)} {op} {self.expr(BOOL)})"

    def str_concat(self, _):
        return f"({self.expr(STR)} + {self.expr(STR)})"

    def str_method(self, _):
        m = self.rng.choice(["to_upper()", "to_lower()", "trim()"])
        return f"{self.expr(STR)}.{m}"

    def int_from_list(self, _):
        """A list reduced to a scalar — no indexing, so nothing to go out of."""
        n = self.rng.randint(1, 4)
        items = ", ".join(self.expr(INT) for _ in range(n))
        m = self.rng.choice(["len()", "sum()", "min() ?? 0", "max() ?? 0"])
        # `??` binds looser than arithmetic, so the whole thing is wrapped:
        # `a + xs.min() ?? 0` would parse as `(a + xs.min()) ?? 0`.
        return f"([{items}].{m})"

    # --- statements ------------------------------------------------------

    def stmt(self, out):
        kind = self.rng.random()
        if kind < 0.30:
            ty = self.rng.choice([INT, FLOAT, BOOL, STR])
            name = self.fresh()
            out.append(f"    let {name} = {self.expr(ty)}")
            self.scope.append((name, ty))
        elif kind < 0.45:
            name = self.fresh("acc")
            out.append(f"    var {name} = {self.expr(INT)}")
            self.scope.append((name, INT))
            out.append(f"    {name} = {name} + {self.expr(INT)}")
        elif kind < 0.60:
            # A bounded loop over a range, accumulating.
            name = self.fresh("t")
            idx = self.fresh("i")
            lo = self.rng.randint(0, 3)
            hi = lo + self.rng.randint(1, 4)
            out.append(f"    var {name} = 0")
            self.scope.append((name, INT))
            out.append(f"    for {idx} in {lo}..{hi} {{")
            self.scope.append((idx, INT))
            out.append(f"        {name} = {name} + {self.expr(INT)}")
            self.scope.pop()
            out.append("    }")
        elif kind < 0.72:
            # Iterating a list, which exercises the pinning path.
            name = self.fresh("s")
            it = self.fresh("e")
            n = self.rng.randint(1, 4)
            items = ", ".join(self.expr(INT) for _ in range(n))
            out.append(f"    var {name} = 0")
            self.scope.append((name, INT))
            out.append(f"    let xs{name} = [{items}]")
            out.append(f"    for {it} in xs{name} {{")
            self.scope.append((it, INT))
            out.append(f"        {name} = {name} + {self.expr(INT)}")
            self.scope.pop()
            out.append("    }")
        elif kind < 0.85:
            out.append(f"    if {self.expr(BOOL)} {{")
            out.append(f"        print(\"{{{self.expr(INT)}}}\")")
            out.append("    } else {")
            out.append(f"        print(\"{{{self.expr(STR)}}}\")")
            out.append("    }")
        else:
            ty = self.rng.choice([INT, FLOAT, BOOL, STR])
            out.append(f'    print("{{{self.expr(ty)}}}")')

    def program(self):
        body = []
        for _ in range(self.rng.randint(3, 9)):
            self.stmt(body)
        # Always print something, so an empty program cannot pass vacuously.
        for ty in (INT, STR, BOOL, FLOAT):
            body.append(f'    print("{{{self.expr(ty)}}}")')
        return "fn main() {\n" + "\n".join(body) + "\n}\n"


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=60, **kw)


def check(src_path, bin_path):
    """(ok, detail). Compares stdout and exit code, as difftest does."""
    build = run([GRASM, "build", src_path, "--native", "-o", bin_path])
    if not os.path.exists(bin_path):
        return False, "native build failed:\n" + (build.stdout + build.stderr)[:600]
    interp = run([GRASM, "run", src_path])
    native = run([bin_path])
    if interp.stdout != native.stdout or interp.returncode != native.returncode:
        return False, (
            f"exit {interp.returncode} (interp) vs {native.returncode} (native)\n"
            f"--- interpreter ---\n{interp.stdout}"
            f"--- native ---\n{native.stdout}"
        )
    return True, ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=200)
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--keep", action="store_true", help="keep failing programs")
    args = ap.parse_args()

    if not os.path.exists(GRASM):
        print(f"error: {GRASM} missing — run: cargo build --release --workspace")
        return 1

    base = args.seed if args.seed is not None else random.randrange(1 << 30)
    work = tempfile.mkdtemp(prefix="grasm_fuzz_")
    failures = []
    rejected = []

    print(f"generating {args.count} programs from base seed {base}")
    for i in range(args.count):
        seed = base + i
        src = Gen(random.Random(seed)).program()
        path = os.path.join(work, f"p{seed}.gr")
        with open(path, "w") as f:
            f.write(src)

        # A program this generator emits must typecheck. If it does not, the
        # generator is wrong, and that is worth hearing about immediately.
        checked = run([GRASM, "check", path])
        if checked.returncode != 0:
            rejected.append((seed, checked.stdout + checked.stderr))
            continue

        ok, detail = check(path, os.path.join(work, f"p{seed}.bin"))
        if not ok:
            failures.append((seed, src, detail))
        if (i + 1) % 25 == 0:
            print(f"  {i + 1}/{args.count}  mismatches={len(failures)}")

    print()
    print("=" * 42)
    print(f"programs: {args.count}   mismatched: {len(failures)}   rejected: {len(rejected)}")
    print("=" * 42)

    for seed, out in rejected[:3]:
        print(f"\nGENERATOR EMITTED AN INVALID PROGRAM (seed {seed}):")
        print("  " + out.strip().replace("\n", "\n  ")[:500])
    for seed, src, detail in failures[:3]:
        print(f"\nMISMATCH (reproduce with --seed {seed} --count 1):")
        print(src)
        print(detail)

    if args.keep and (failures or rejected):
        print(f"\nprograms kept in {work}")
    else:
        shutil.rmtree(work, ignore_errors=True)

    return 1 if failures or rejected else 0


if __name__ == "__main__":
    sys.exit(main())
