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
# The Grasm backend (compiler/s3), held to the Cranelift build on each program.
S3Engine = _lib("s3engine").Engine

_spec = _ilu.spec_from_file_location(
    "grasm_fresh", os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", "fresh.py")
)
_fresh = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_fresh)
_fresh.fresh_or_stop(GRASM)


INT, FLOAT, BOOL, STR = "i64", "f64", "bool", "str"


class Gen:
    """Builds a well-typed program, tracking what is in scope and its type."""

    def __init__(self, rng):
        self.rng = rng
        # A `{` inside `"{...}"` ends the interpolation, so block expressions
        # cannot be generated there. The language reports this clearly; it is
        # the generator that has to know.
        self.in_interp = False
        self.scope = []          # list of (name, type)
        self.next_id = 0
        self.depth = 0
        self.helpers = []        # (name, return type, [param types])
        self.fallibles = []      # (name, ok type) for `fn .. -> T ! Bad`

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
            if self.helpers:
                choices += [self.call_helper]
            if ty == INT:
                choices += [
                    self.int_arith, self.int_cond, self.int_from_list,
                    self.int_from_match, self.int_from_field, self.int_from_map,
                    self.int_from_concat, self.int_from_map_lit,
                    self.int_from_tuple,
                ]
                # Block-shaped generators only at the top of an expression.
                # Nested inside a `match` arm inside a list literal they build
                # shapes the native type *probe* cannot see through — an
                # honest refusal, but not what this is testing.
                if not self.in_interp and self.depth <= 1:
                    choices += [
                        self.int_from_sort_by, self.int_from_pairs,
                        self.int_from_remove,
                    ]
            elif ty == FLOAT:
                choices += [self.float_arith, self.float_cond, self.float_method]
            elif ty == BOOL:
                # There were two `elif ty == BOOL` branches here, the first of
                # them `pass`. Python takes the first, so `bool_cmp` and
                # `bool_logic` were unreachable and every boolean this fuzzer
                # ever generated was the literal `true` or `false`. Comparison
                # and `and`/`or` went untested for the whole life of the tool,
                # which is the kind of hole a generator hides best: it kept
                # producing valid programs and reporting no mismatches.
                choices += [self.bool_cmp, self.bool_logic]
            elif ty == STR:
                choices += [self.str_concat, self.str_method, self.str_words]
                if not self.in_interp and self.depth <= 1:
                    choices += [self.str_from_codes]
            return self.rng.choice(choices)(ty)
        finally:
            self.depth -= 1

    def interp_expr(self, ty):
        """An expression safe to place inside `"{...}"`."""
        self.in_interp = True
        try:
            return self.expr(ty)
        finally:
            self.in_interp = False

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

    # The math library. Both engines route these to the same Rust std
    # functions, so any difference in the last bit is a real one.
    MATH_UNARY = [
        "sin", "cos", "tan", "atan", "sinh", "cosh", "tanh", "exp",
        "cbrt", "to_radians", "to_degrees",
    ]
    MATH_BINARY = ["atan2", "hypot"]

    def float_method(self, _):
        r = self.rng.random()
        if r < 0.15:
            return self.rng.choice(["pi()", "e()"])
        if r < 0.4:
            # Domain-restricted: `sqrt`, `ln` and friends of a negative are
            # NaN, which both engines agree on but which then swallows every
            # later operation and makes a mismatch impossible to see.
            m = self.rng.choice(["sqrt", "ln", "log2", "log10"])
            return f"(({self.expr(FLOAT)}).abs() + 1.0).{m}()"
        if r < 0.55:
            m = self.rng.choice(["asin", "acos"])
            return f"(({self.expr(FLOAT)}).sin()).{m}()"
        if r < 0.8:
            m = self.rng.choice(self.MATH_UNARY)
            return f"({self.expr(FLOAT)}).{m}()"
        m = self.rng.choice(self.MATH_BINARY)
        return f"({self.expr(FLOAT)}).{m}({self.expr(FLOAT)})"

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
        m = self.rng.choice([
            "to_upper()", "to_lower()", "trim()", "trim_start()", "trim_end()",
        ])
        return f"{self.expr(STR)}.{m}"

    def int_from_list(self, _):
        """A list reduced to a scalar — no indexing, so nothing to go out of."""
        n = self.rng.randint(1, 4)
        items = ", ".join(self.expr(INT) for _ in range(n))
        m = self.rng.choice(["len()", "sum()", "min() ?? 0", "max() ?? 0"])
        # `??` binds looser than arithmetic, so the whole thing is wrapped:
        # `a + xs.min() ?? 0` would parse as `(a + xs.min()) ?? 0`.
        return f"([{items}].{m})"

    def int_from_tuple(self, _):
        """Tuple components by position, and the order they sort in."""
        a, b = self.expr(INT), self.expr(INT)
        mode = self.rng.random()
        if mode < 0.4:
            return f"(({a}, {b}).{self.rng.choice(['0', '1'])})"
        if mode < 0.7:
            c, d = self.expr(INT), self.expr(INT)
            op = self.rng.choice(["<", "<=", ">", ">="])
            return f"(if ({a}, {b}) {op} ({c}, {d}) {{ 1 }} else {{ 0 }})"
        return f"(({a}, {b}).0 + ({a}, {b}).1)"

    def int_from_remove(self, _):
        """`remove` on an index that is definitely in range."""
        n = self.rng.randint(1, 4)
        items = ", ".join(self.expr(INT) for _ in range(n))
        name = self.fresh("r")
        return (
            f"({{ var {name}: [i64] = [{items}]\n"
            f"{name}.remove({name}.len() - 1) }})"
        )

    def str_words(self, _):
        """`words` and `split` differ on runs of whitespace; both engines
        have to agree on which."""
        src = self.expr(STR)
        mode = self.rng.random()
        if mode < 0.5:
            return f'(({src}).words().len().to_str())'
        return f'(({src}).words().join("-"))'

    def int_from_concat(self, _):
        """`+` on lists, reduced to a scalar so nothing can go out of range."""
        a = ", ".join(self.expr(INT) for _ in range(self.rng.randint(0, 2)))
        b = ", ".join(self.expr(INT) for _ in range(self.rng.randint(0, 2)))
        # An empty literal needs a type, and `[] + [x]` has one from the right.
        lhs = f"[{a}]" if a else "[]"
        rhs = f"[{b}]" if b else "[]"
        if not a and not b:
            lhs = f"[{self.expr(INT)}]"
        m = self.rng.choice(["len()", "sum()"])
        return f"(({lhs} + {rhs}).{m})"

    def int_from_sort_by(self, _):
        """`sort_by` needs a `var`, so this builds one in a helper-free way."""
        n = self.rng.randint(1, 4)
        items = ", ".join(self.expr(INT) for _ in range(n))
        key = self.rng.choice(["n", "0 - n", "n * n"])
        name = self.fresh("s")
        # A block expression keeps this an expression while needing a `var`.
        return (
            f"({{ var {name}: [i64] = [{items}]\n"
            f"{name}.sort_by(|n| {key})\n"
            f"{name}.get(0) ?? 0 }})"
        )

    def int_from_pairs(self, _):
        """`enumerate`/`zip`, consumed through `for (a, b)` destructuring."""
        n = self.rng.randint(1, 3)
        items = ", ".join(self.expr(INT) for _ in range(n))
        # Not "p": helper parameters are named `p0`, `p1`, … and a local
        # minted with the same prefix collided with one. The scope map then
        # said `p1` was the parameter's type while the shadowing local had
        # another, so the generator emitted programs that do not typecheck —
        # noise that hides a real rejection.
        acc = self.fresh("acc")
        if self.rng.random() < 0.5:
            src = f"[{items}].enumerate()"
        else:
            other = ", ".join(self.expr(INT) for _ in range(self.rng.randint(1, 4)))
            src = f"[{items}].zip([{other}])"
        return (
            f"({{ var {acc}: i64 = 0\n"
            f"for (a, b) in {src} {{ {acc} = {acc} + a + b }}\n"
            f"{acc} }})"
        )

    def int_from_map_lit(self, _):
        """A map literal, read back through `get`."""
        keys = self.rng.sample(["a", "b", "c", "d"], self.rng.randint(1, 3))
        entries = ", ".join(f'"{k}": {self.expr(INT)}' for k in keys)
        want = self.rng.choice(keys + ["zz"])
        m = self.rng.choice([f'get("{want}") ?? {self.rng.randint(-9, 9)}', "len()"])
        return f"(({{{entries}}}).{m})"

    def str_from_codes(self, _):
        """Code points out and back — the round trip has to be exact."""
        src = self.expr(STR)
        mode = self.rng.random()
        if mode < 0.4:
            return f'(({src}).codes().len().to_str())'
        if mode < 0.7:
            acc = self.fresh("c")
            return (
                f'({{ var {acc}: str = ""\n'
                f"for c in ({src}).codes() {{ {acc} = {acc} + c.to_char() }}\n"
                f"{acc} }})"
            )
        # `code_at` needs an index that is definitely in range.
        return f'(if ({src}).len() > 0 {{ ({src}).code_at(0).to_char() }} else {{ "" }})'

    def call_helper(self, ty):
        """A call to one of the top-level functions this program declares."""
        candidates = [h for h in self.helpers if h[1] == ty]
        if not candidates:
            return self.atom(ty)
        name, _, params = self.rng.choice(candidates)
        args = ", ".join(self.expr(p) for p in params)
        return f"{name}({args})"

    def int_from_match(self, _):
        """Exhaustive match over the generated sum type."""
        v = self.rng.choice(["Small(3)", "Small(-8)", "Pair(2, 5)", "Nothing"])
        return (
            f"(match Shape.{v} {{ "
            f"Small(a) -> a "
            f"Pair(a, b) -> (a + b) "
            f"Nothing -> {self.expr(INT)} }})"
        )

    def int_from_field(self, _):
        f = self.rng.choice(["low", "high"])
        return f"(Span{{ low: {self.expr(INT)}, high: {self.expr(INT)} }}.{f})"

    def int_from_map(self, _):
        live = [n for n, t in self.scope if t == "map"]
        if not live:
            return self.atom(INT)
        k = self.rng.choice(['"a"', '"b"', '"missing"'])
        return f"({self.rng.choice(live)}.get({k}) ?? {self.rng.randint(-9, 9)})"

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
        elif kind < 0.60 and self.fallibles:
            # Consuming a fallible value: both halves, and `try` inside a
            # function that declares the same error type.
            name, ret = self.rng.choice(self.fallibles)
            v = self.fresh("r")
            out.append(f"    match {name}({self.expr(INT)}) {{")
            out.append(f'        Ok({v}) -> print("{{{v}}}")')
            out.append(f'        Err(e{v}) -> print("{{e{v}.why}}")')
            out.append("    }")
        elif kind < 0.64:
            # An effect with a handler, including one operation that takes a
            # parameter and a handler body that itself performs `io`.
            n = self.fresh("h")
            out.append(f"    var {n} = {self.expr(INT)}")
            self.scope.append((n, INT))
            out.append(
                f"    with Cfg handled_by CfgHandler{{ scale: || {n}, "
                f'label: |v| "L{{v}}" }} {{'
            )
            out.append('        print("{Cfg.scale()} {Cfg.label(2)}")')
            out.append("    }")
        elif kind < 0.68:
            # A while loop with a counter the body cannot make diverge.
            n = self.fresh("w")
            acc = self.fresh("wa")
            limit = self.rng.randint(1, 5)
            out.append(f"    var {n} = 0")
            out.append(f"    var {acc} = 0")
            self.scope.append((acc, INT))
            out.append(f"    while {n} < {limit} {{")
            out.append(f"        {acc} = {acc} + {self.expr(INT)}")
            out.append(f"        {n} = {n} + 1")
            out.append("    }")
        elif kind < 0.76:
            m = self.fresh("m")
            out.append(f"    var {m} = map()")
            out.append(f'    {m}.set("a", {self.expr(INT)})')
            out.append(f'    {m}.set("b", {self.expr(INT)})')
            self.scope.append((m, "map"))
            out.append(f'    print("{{{m}}}")')
        elif kind < 0.85:
            out.append(f"    if {self.expr(BOOL)} {{")
            out.append(f"        print(\"{{{self.interp_expr(INT)}}}\")")
            out.append("    } else {")
            out.append(f"        print(\"{{{self.interp_expr(STR)}}}\")")
            out.append("    }")
        else:
            ty = self.rng.choice([INT, FLOAT, BOOL, STR])
            out.append(f'    print("{{{self.interp_expr(ty)}}}")')

    def declarations(self):
        """Top-level types and functions the body may use.

        Fallible functions and an effect are here because every miscompile
        found by a human so far lived in exactly that surface, and a generator
        that only emits straight-line arithmetic will never reach it. The
        `unbox: not an f64` bug needed a `match` inside a `fn ... ! E` whose
        arms were built-in method calls — four features at once.
        """
        out = [
            "type Span = { low: i64, high: i64 }",
            "",
            "type Shape =",
            "    | Small(a: i64)",
            "    | Pair(a: i64, b: i64)",
            "    | Nothing",
            "",
            "type Bad = { why: str }",
            "",
            "effect Cfg {",
            "    fn scale() -> i64",
            "    fn label(n: i64) -> str",
            "}",
            "",
        ]
        # Fallible helpers: `try` at the call site, `fail` on a literal test.
        for i in range(self.rng.randint(1, 2)):
            name = f"risky{i}"
            ret = self.rng.choice([INT, FLOAT])
            guard = self.rng.randint(0, 4)
            saved = self.scope
            self.scope = [("p0", INT)]
            out.append(f"fn {name}(p0: i64) -> {ret} ! Bad {{")
            # Half the time the failure sits in the *then* branch of a tail
            # `if/else`, which is a different shape from a guard statement and
            # the one that miscompiled: the branch leaves, so the `if` is worth
            # what the else branch produced, and taking the type from the then
            # branch alone returned a zero ([E-IF-DIVERGE]).
            tail_if = self.rng.random() < 0.5
            if tail_if:
                out.append(f"    if p0 == {guard} {{")
                out.append(f'        fail(Bad{{ why: "p0 was {guard}" }})')
                out.append("    } else {")
            else:
                out.append(f"    if p0 == {guard} {{")
                out.append(f'        fail(Bad{{ why: "p0 was {guard}" }})')
                out.append("    }")
            # A match whose arms are method calls — the shape that miscompiled.
            if ret == FLOAT:
                out.append("    match p0 % 3 {")
                out.append(f"        0 -> {self.expr(FLOAT)}.abs()")
                out.append(f"        1 -> {self.expr(FLOAT)}.min({self.expr(FLOAT)})")
                out.append(f"        _ -> {self.expr(FLOAT)}.max({self.expr(FLOAT)})")
                out.append("    }")
            else:
                out.append(f"    {self.expr(INT)}")
            if tail_if:
                out.append("    }")
            out.append("}")
            out.append("")
            self.scope = saved
            self.fallibles.append((name, ret))
        for i in range(self.rng.randint(1, 3)):
            name = f"helper{i}"
            params = [self.rng.choice([INT, FLOAT, BOOL, STR])
                      for _ in range(self.rng.randint(1, 2))]
            ret = self.rng.choice([INT, FLOAT, BOOL, STR])
            sig = ", ".join(f"p{j}: {t}" for j, t in enumerate(params))
            saved = self.scope
            self.scope = [(f"p{j}", t) for j, t in enumerate(params)]
            out.append(f"fn {name}({sig}) -> {ret} {{")
            out.append(f"    {self.expr(ret)}")
            out.append("}")
            out.append("")
            self.scope = saved
            self.helpers.append((name, ret, params))
        return out

    def program(self):
        head = self.declarations()
        body = []
        for _ in range(self.rng.randint(3, 9)):
            self.stmt(body)
        # Every fallible helper is consumed at least once, on a value that
        # fails and one that does not. A generator that declares a shape and
        # never runs it tests compilation only — which is how the first
        # version of this missed the miscompile it was written for.
        for name, _ret in self.fallibles:
            for arg in (0, 1, 4, 7):
                v = self.fresh("chk")
                body.append(f"    match {name}({arg}) {{")
                body.append(f'        Ok({v}) -> print("{{{v}}}")')
                body.append(f'        Err(e{v}) -> print("{{e{v}.why}}")')
                body.append("    }")

        # Always print something, so an empty program cannot pass vacuously.
        for ty in (INT, STR, BOOL, FLOAT):
            body.append(f'    print("{{{self.interp_expr(ty)}}}")')
        return ("\n".join(head) + "\nfn main() {\n"
                + "\n".join(body) + "\n}\n")


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=60, **kw)


def check(src_path, bin_path, s3, label):
    """"ok", "reject" (a generator bug) or "fail" (a compiler one), and detail.

    A program this generator emits must typecheck, and one that does not is a
    generator bug worth hearing about immediately. Establishing that does not
    need its own `grasm check` run: `build --native` typechecks the very same
    program on its way to code, so a build that succeeded has already said the
    program was well-typed. A separate check spawn spent a third of this
    fuzzer's compile time re-deriving what the build was about to prove, so it
    runs only when the build failed — the one case where the answer is in
    doubt, and the only case that has to tell the two kinds of bug apart.
    """
    build = build_native(GRASM, src_path, bin_path, "--native")
    if not os.path.exists(bin_path):
        checked = run([GRASM, "check", src_path])
        if checked.returncode != 0:
            return "reject", checked.stdout + checked.stderr
        return "fail", "native build failed:\n" + (build.stdout + build.stderr)[:600]
    interp = run([GRASM, "run", src_path])
    native = run([bin_path])
    # Its verdict is the engine's to count; the comparison below is the
    # interpreter's against the Cranelift build's, as before.
    s3.check(src_path, bin_path, label, interp=(interp.returncode, interp.stdout))
    if interp.stdout != native.stdout or interp.returncode != native.returncode:
        return "fail", (
            f"exit {interp.returncode} (interp) vs {native.returncode} (native)\n"
            f"--- interpreter ---\n{interp.stdout}"
            f"--- native ---\n{native.stdout}"
        )
    # Whole random programs, which is the broadest sample of shapes there is.
    # They were compared for what they print and never for what they keep
    # (F57).
    held = leaked(bin_path) if native.returncode == 0 else None
    if held:
        return "fail", f"native leaks: {held}"
    return "ok", ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=500)
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
    s3 = S3Engine(GRASM, work)

    print(f"generating {args.count} programs from base seed {base}")
    for i in range(args.count):
        seed = base + i
        src = Gen(random.Random(seed)).program()
        path = os.path.join(work, f"p{seed}.gr")
        with open(path, "w") as f:
            f.write(src)

        status, detail = check(path, os.path.join(work, "p.bin"), s3, f"seed {seed}")
        if status == "reject":
            rejected.append((seed, detail))
        elif status == "fail":
            failures.append((seed, src, detail))
        if (i + 1) % 25 == 0:
            print(f"  {i + 1}/{args.count}  mismatches={len(failures)}")

    print()
    s3_ok = s3.report()
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

    return 1 if failures or rejected or not s3_ok else 0


if __name__ == "__main__":
    sys.exit(main())
