#!/usr/bin/env python3
"""Random programs in the S3 subset, through the Grasm backend and Cranelift.

The corpus programs in the subset (`scripts/s3subset.sh`) are small, and
written to state one rule each: few of them have a function with more live
values than there are registers, a call inside a loop inside a match, or an
`inout` argument written back across a branch. This generates programs that
do, from the whole subset (`scripts/s3_subset.py`) and nothing outside it:

  * every integer type, each expression typed throughout, literals at the ends
    of their ranges as well as near zero;
  * `+ - * / %` with divisors that cannot be zero, `^` on i64, unary minus,
    every comparison, `and`/`or`/`not`, `if` and `match` (literal, or- and
    binding patterns, guards) as values and as statements, block values;
  * `let`, `var`, `=`, `+=`-style assignment, `while`, `for` over exclusive and
    inclusive ranges, `break` and `continue`;
  * functions that call earlier ones, with `inout` parameters, one that
    recurses, constants, `print` and `eprint` of interpolated values;
  * heap values (`HeapGen`): strings built and appended to, lists pushed,
    popped, indexed, assigned and walked with `break` and `continue`, a
    struct with a list in it changed through field and element paths and
    passed `inout`, a sum type matched with guards, maps, tuples and
    `let (a, b)`, options under `??`, and functions that can fail -- `fail`,
    `try` through each other, and their results matched as `Ok`/`Err` or
    under `??`;
  * doubles (`FloatGen`): literals at the edges and NaN and the infinities,
    arithmetic, every comparison as a value and as a branch with NaN among
    the operands, conversions to and from every integer type and the bits,
    the math methods, doubles in lists, structs, tuples, options, maps and
    results, a call with twelve arguments (two doubles on the stack), and
    twenty doubles live across calls;
  * closures and function values (`ClosureGen`): captures of every kind,
    closures stored and called, named functions as values, closures made by
    functions, the higher-order list methods, `sort_by`, closures answering
    `Ok`/`Err`, a closure making another;
  * part four's constructs (`Part4Gen`): references written and read through,
    a structure of them matched, released and asked after; ranges held as
    values; an effect handled, nested; `parallel` tasks; the parallel maps; a
    struct updated with `..`; a builder; the bit operations at a width;
  * now and then a deliberate panic, a failed `assert` or an `exit`.

Each program must be in the subset by the rule, compile with the backend (a
refusal is a failure, not a skip), link, and print and exit exactly as the
Cranelift build does -- and again with the allocator held to three registers,
so the same program runs mostly out of spill slots. One that returns from
`main` is run once more with `GRASM_LEAK_CHECK=1`, and must have nothing
live at exit.

    scripts/fuzz_s3.py [--programs N] [--seed S] [--keep]
"""

import argparse
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")
RUNTIME = os.environ.get("S3_RT") or os.path.join(ROOT, "target", "release", "libgrasm_rt_abi.a")

import importlib.util as _ilu


def _lib(name):
    spec = _ilu.spec_from_file_location(
        name, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", f"{name}.py"))
    mod = _ilu.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


build_native = _lib("inplace").build
_lib("fresh").fresh_or_stop(GRASM)
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import s3_subset  # noqa: E402

KINDS = {"i64": (64, True), "i32": (32, True), "i16": (16, True), "i8": (8, True),
         "u64": (64, False), "u32": (32, False), "u16": (16, False), "u8": (8, False)}
INT_WEIGHTS = ["i64"] * 6 + ["i32", "i16", "i8", "u64", "u32", "u16", "u8"]


def lo(k):
    bits, signed = KINDS[k]
    return -(1 << (bits - 1)) if signed else 0


def hi(k):
    bits, signed = KINDS[k]
    return (1 << (bits - 1)) - 1 if signed else (1 << bits) - 1


class Gen:
    """One program. A scope is a list of (name, type, mutable)."""

    def __init__(self, rng):
        self.rng = rng
        self.fns = []
        self.consts = []
        self.n = 0
        self.depth = 0

    def fresh(self, base):
        self.n += 1
        return f"{base}{self.n}"

    def ind(self):
        return "    " * self.depth

    # ----- expressions -------------------------------------------------------

    def literal(self, k):
        r = self.rng
        x = r.random()
        if x < 0.2:
            v = r.choice([lo(k), hi(k), lo(k) + 1, hi(k) - 1])
        elif x < 0.7:
            v = r.randint(max(lo(k), -20), min(hi(k), 20))
        else:
            v = r.randint(lo(k), hi(k))
        return f"({v})" if v < 0 else str(v)

    def vars_of(self, scope, k):
        return [n for (n, t, _m) in scope if t == k]

    def divisor(self, k, scope):
        """An expression of type k that is never zero."""
        r = self.rng
        vs = self.vars_of(scope, k)
        if r.random() < 0.5 or not vs:
            if KINDS[k][1]:
                v = r.choice([1, -1, 2, 3, 7, 10, 16, 100, -2, -7])
            else:
                v = r.choice([1, 2, 3, 7, 10, 16, 100])
            return f"({v})" if v < 0 else str(v)
        x = r.choice(vs)
        return f"({x} % 7 + 9)" if KINDS[k][1] else f"({x} % 7 + 1)"

    def anchored_kind(self, scope):
        """An integer type that has a variable in scope, so an expression of
        it is typed by that variable rather than by its literals."""
        ks = sorted(set(t for (_n, t, _m) in scope if t in KINDS))
        if ks and self.rng.random() < 0.85:
            return self.rng.choice(ks)
        return "i64"

    def anchored(self, k, scope, d):
        vs = self.vars_of(scope, k)
        if not vs:
            return self.int_expr(k, scope, d, slot=k == "i64")
        v = self.rng.choice(vs)
        if self.rng.random() < 0.4:
            return v
        op = self.rng.choice(["+", "-", "*"])
        return f"({v} {op} {self.int_expr(k, scope, d + 1)})"

    def has_anchor(self, k, scope):
        """Whether something of type k other than a literal is at hand: a
        literal of a narrow type takes that type only where the position or
        the other operand gives it one; on its own it is an i64."""
        return (k == "i64" or bool(self.vars_of(scope, k))
                or any(t == k for (_n, t) in self.consts)
                or any(f[2] == k and self.callable(f, scope) for f in self.fns))

    def strong(self, k, scope):
        """A leaf of type k that is not a literal (k must have an anchor)."""
        r = self.rng
        vs = self.vars_of(scope, k)
        cs = [n for (n, t) in self.consts if t == k]
        if vs and (r.random() < 0.8 or not cs):
            return r.choice(vs)
        if cs:
            return r.choice(cs)
        call = self.call_expr(k, scope, plain=True)
        if call:
            return call
        return self.literal(k)

    def int_expr(self, k, scope, d=0, ml=False, slot=True):
        """An expression of integer type k. `slot`: the position gives the
        type, so a bare literal is one; otherwise, for a narrow type, the
        expression is anchored by a variable, constant or call. Multi-line
        forms (`match`, block values) only where `ml` says a statement starts
        here."""
        r = self.rng
        narrow = k != "i64"
        anchor = self.has_anchor(k, scope)
        if narrow and not anchor:
            return self.literal(k)
        if d >= 3 or r.random() < 0.3:
            if narrow and not slot:
                return self.strong(k, scope)
            vs = self.vars_of(scope, k)
            cs = [n for (n, t) in self.consts if t == k]
            if vs and r.random() < 0.7:
                return r.choice(vs)
            if cs and r.random() < 0.3:
                return r.choice(cs)
            return self.literal(k)
        c = r.random()
        if ml and c < 0.15:
            return self.match_expr(k, scope)
        if ml and c < 0.22:
            t = self.fresh("b")
            inner = scope + [(t, k, False)]
            self.depth += 1
            first = f"{self.ind()}let {t}: {k} = {self.int_expr(k, scope, d + 1)}"
            last = f"{self.ind()}{self.int_expr(k, inner, d + 1, slot=False)}"
            self.depth -= 1
            return f"{{\n{first}\n{last}\n{self.ind()}}}"
        if c < 0.45:
            op = r.choice(["+", "-", "*", "+", "-"])
            return f"({self.int_expr(k, scope, d + 1, slot=False)} {op} {self.int_expr(k, scope, d + 1)})"
        if c < 0.55:
            op = r.choice(["/", "%"])
            return f"({self.int_expr(k, scope, d + 1, slot=False)} {op} {self.divisor(k, scope)})"
        if c < 0.60 and KINDS[k][1]:
            return f"(-{self.int_expr(k, scope, d + 1, slot=False)})"
        if c < 0.63 and k == "i64":
            return f"({self.int_expr(k, scope, d + 1)} ^ {r.randint(0, 4)})"
        if c < 0.72:
            return (f"(if {self.bool_expr(scope, d + 1)} {{ {self.int_expr(k, scope, d + 1, slot=False)} }}"
                    f" else {{ {self.int_expr(k, scope, d + 1, slot=False)} }})")
        if c < 0.88:
            call = self.call_expr(k, scope)
            if call:
                return call
        if narrow and not slot:
            return self.strong(k, scope)
        return self.literal(k)

    def bool_expr(self, scope, d=0):
        r = self.rng
        bs = [n for (n, t, _m) in scope if t == "bool"]
        if d >= 3 or r.random() < 0.2:
            if bs and r.random() < 0.6:
                return r.choice(bs)
            return r.choice(["true", "false"])
        c = r.random()
        if c < 0.6:
            k = self.anchored_kind(scope)
            op = r.choice(["<", "<=", ">", ">=", "==", "!="])
            return f"({self.anchored(k, scope, d + 1)} {op} {self.int_expr(k, scope, d + 1)})"
        if c < 0.8:
            op = r.choice(["and", "or"])
            return f"({self.bool_expr(scope, d + 1)} {op} {self.bool_expr(scope, d + 1)})"
        if c < 0.9:
            return f"(not {self.bool_expr(scope, d + 1)})"
        return f"({self.bool_expr(scope, d + 1)} == {self.bool_expr(scope, d + 1)})"

    def match_expr(self, k, scope):
        r = self.rng
        scrut = self.anchored(k, scope, 1)
        arms = []
        used = set()
        for _ in range(r.randint(1, 3)):
            v = r.randint(max(lo(k), -3), min(hi(k), 5))
            if v in used:
                continue
            used.add(v)
            pat = str(v)
            if r.random() < 0.3 and lo(k) <= v + 1 <= hi(k) and v + 1 not in used:
                used.add(v + 1)
                pat = f"{pat} | {v + 1}"
            arms.append(f"{pat} -> {self.int_expr(k, scope, 1, slot=False)}")
        if r.random() < 0.4:
            b = self.fresh("m")
            inner = scope + [(b, k, False)]
            arms.append(f"{b} if {self.bool_expr(inner, 1)} -> {self.int_expr(k, inner, 1, slot=False)}")
        if r.random() < 0.3:
            b = self.fresh("m")
            arms.append(f"{b} -> {self.int_expr(k, scope + [(b, k, False)], 1, slot=False)}")
        else:
            arms.append(f"_ -> {self.int_expr(k, scope, 1, slot=False)}")
        self.depth += 1
        body = "\n".join(f"{self.ind()}{a}" for a in arms)
        self.depth -= 1
        return f"match {scrut} {{\n{body}\n{self.ind()}}}"

    def call_expr(self, k, scope, plain=False):
        r = self.rng
        if k == "i64" and not plain and r.random() < 0.15:
            # Bounded, so both builds recurse well inside the depth the
            # language guarantees ([E-RECURSION-DEPTH]).
            return f"rec(({self.int_expr('i64', scope, 3)}) % 40, {self.int_expr('i64', scope, 3)})"
        cands = [f for f in self.fns if f[2] == k and self.callable(f, scope)]
        if not cands:
            return None
        name, params, _ret = r.choice(cands)
        args = []
        taken = set()
        for (mode, t) in params:
            if mode == "inout":
                # One place cannot be passed `inout` twice in one call.
                vs = [n for (n, tt, m) in scope if tt == t and m and n not in taken]
                v = r.choice(vs)
                taken.add(v)
                args.append(v)
            else:
                args.append(self.int_expr(t, scope, 3))
        return f"{name}({', '.join(args)})"

    def callable(self, f, scope):
        """Whether `f` can be called here: a distinct mutable local of the
        right type for each of its `inout` parameters."""
        need = {}
        for (mode, t) in f[1]:
            if mode == "inout":
                need[t] = need.get(t, 0) + 1
        for t, n in need.items():
            if len([1 for (_v, tt, m) in scope if tt == t and m]) < n:
                return False
        return True

    # ----- statements -----------------------------------------------------------

    def block(self, scope, n, loop=False, budget=2):
        """Statements one level in; answers the lines and the scope after them."""
        out = []
        scope = list(scope)
        self.depth += 1
        for _ in range(n):
            out.extend(self.stmt(scope, loop, budget))
        self.depth -= 1
        return out, scope

    def stmt(self, scope, loop, budget):
        r = self.rng
        i = self.ind()
        c = r.random()
        if c < 0.22 or not scope:
            k = r.choice(INT_WEIGHTS) if r.random() < 0.85 else "bool"
            name = self.fresh("v")
            mut = r.random() < 0.6
            e = self.bool_expr(scope) if k == "bool" else self.int_expr(k, scope, 0, True)
            scope.append((name, k, mut))
            return [f"{i}{'var' if mut else 'let'} {name}: {k} = {e}"]
        muts = [(n, t) for (n, t, m) in scope if m]
        if c < 0.40 and muts:
            name, k = r.choice(muts)
            if k == "bool":
                return [f"{i}{name} = {self.bool_expr(scope)}"]
            op = r.choice(["=", "+=", "-=", "*=", "/="])
            if op == "/=":
                return [f"{i}{name} /= {self.divisor(k, scope)}"]
            return [f"{i}{name} {op} {self.int_expr(k, scope, 0, op == '=')}"]
        if c < 0.55:
            return [self.print_stmt(scope)]
        if c < 0.65 and budget > 0:
            out = [f"{i}if {self.bool_expr(scope)} {{"]
            body, _ = self.block(scope, r.randint(1, 3), loop, budget - 1)
            out += body
            if r.random() < 0.6:
                out.append(f"{i}}} else {{")
                body, _ = self.block(scope, r.randint(1, 3), loop, budget - 1)
                out += body
            out.append(f"{i}}}")
            return out
        if c < 0.73 and budget > 0:
            ctr = self.fresh("w")
            lim = r.randint(0, 6)
            out = [f"{i}var {ctr}: i64 = 0", f"{i}while {ctr} < {lim} {{", f"{i}    {ctr} += 1"]
            body, _ = self.block(scope + [(ctr, "i64", False)], r.randint(1, 3), True, budget - 1)
            out += body
            out.append(f"{i}}}")
            return out
        if c < 0.81 and budget > 0:
            v = self.fresh("i")
            a = r.randint(-3, 3)
            b = a + r.randint(-1, 6)
            span = f"{a}..={b}" if r.random() < 0.4 else f"{a}..{b}"
            out = [f"{i}for {v} in {span} {{"]
            body, _ = self.block(scope + [(v, "i64", False)], r.randint(1, 3), True, budget - 1)
            out += body
            out.append(f"{i}}}")
            return out
        if c < 0.85 and loop:
            return [f"{i}if {self.bool_expr(scope)} {{", f"{i}    {r.choice(['break', 'continue'])}", f"{i}}}"]
        if c < 0.92:
            k = r.choice(INT_WEIGHTS)
            call = self.call_expr(k, scope)
            if call:
                name = self.fresh("r")
                scope.append((name, k, False))
                return [f"{i}let {name}: {k} = {call}"]
        if c < 0.97 and budget > 0:
            k = self.anchored_kind(scope)
            out = [f"{i}match {self.anchored(k, scope, 1)} {{"]
            self.depth += 1
            for v in sorted(set(r.randint(max(lo(k), -2), min(hi(k), 3)) for _ in range(2))):
                out.append(f"{self.ind()}{v} -> {{")
                body, _ = self.block(scope, 1, loop, budget - 1)
                out += body
                out.append(f"{self.ind()}}}")
            out.append(f"{self.ind()}_ -> {{")
            body, _ = self.block(scope, 1, loop, budget - 1)
            out += body
            out.append(f"{self.ind()}}}")
            self.depth -= 1
            out.append(f"{i}}}")
            return out
        return [self.print_stmt(scope)]

    def print_stmt(self, scope):
        r = self.rng
        parts = []
        for _ in range(r.randint(1, 3)):
            if r.random() < 0.8:
                k = self.anchored_kind(scope)
                parts.append(f"{{{self.anchored(k, scope, 2)}}}")
            else:
                parts.append(f"{{{self.bool_expr(scope, 2)}}}")
        word = r.choice(["print", "print", "print", "eprint"])
        return f"{self.ind()}{word}(\"{self.fresh('p')}: {' '.join(parts)}\")"

    # ----- the program -------------------------------------------------------------

    def function(self, idx):
        r = self.rng
        name = f"f{idx}"
        ret = r.choice(INT_WEIGHTS)
        params = []
        scope = []
        for j in range(r.randint(0, 5)):
            k = r.choice(INT_WEIGHTS)
            mode = "inout" if r.random() < 0.2 else "let"
            params.append((mode, k))
            scope.append((f"a{j}", k, mode == "inout"))
        self.depth = 0
        body, after = self.block(scope, r.randint(2, 7))
        sig = ", ".join(f"{'inout ' if m == 'inout' else ''}a{j}: {k}" for j, (m, k) in enumerate(params))
        self.depth = 1
        tail = self.int_expr(ret, after, 1)
        self.fns.append((name, params, ret))
        return [f"fn {name}({sig}) -> {ret} {{"] + body + [f"    {tail}", "}", ""]

    def program(self):
        r = self.rng
        lines = []
        for c in range(r.randint(0, 2)):
            k = r.choice(INT_WEIGHTS)
            name = f"K{c}"
            lines.append(f"let {name}: {k} = {self.literal(k)}")
            self.consts.append((name, k))
        if lines:
            lines.append("")
        # One function that reaches itself, so the stack check is exercised.
        lines += [
            "fn rec(n: i64, acc: i64) -> i64 {",
            "    if n <= 0 {",
            "        acc",
            "    } else {",
            "        rec(n - 1, acc * 3 + n)",
            "    }",
            "}",
            "",
        ]
        for idx in range(r.randint(2, 6)):
            lines += self.function(idx)
        # Heap values and fallible code, in functions of their own that
        # `main` calls among its scalar statements.
        hg = HeapGen(random.Random(r.random()))
        lines += HEAP_DECLS.split("\n")
        lines += hg.fallible_fns()
        nheap = r.randint(1, 3)
        for k in range(nheap):
            lines += hg.function(f"heap{k}")
        # Doubles, likewise.
        fg = FloatGen(random.Random(r.random()))
        lines += FLOAT_DECLS.split("\n")
        nflt = r.randint(1, 2)
        for k in range(nflt):
            lines += fg.function(f"flt{k}")
        # And closures.
        cg = ClosureGen(random.Random(r.random()))
        lines += CLOSURE_DECLS.split("\n")
        nclo = r.randint(1, 2)
        for k in range(nclo):
            lines += cg.function(f"clo{k}")
        # And part four's references, ranges, effects and tasks.
        pg = Part4Gen(random.Random(r.random()))
        lines += PART4_DECLS.split("\n")
        npart4 = r.randint(1, 2)
        for k in range(npart4):
            lines += pg.function(f"p4_{k}")
        self.depth = 0
        body, _ = self.block([], r.randint(6, 14), budget=2)
        # Between two of main's own statements: before a line that starts
        # one, or at the end.
        calls = ([f"heap{k}" for k in range(nheap)] + [f"flt{k}" for k in range(nflt)]
                 + [f"clo{k}" for k in range(nclo)] + [f"p4_{k}" for k in range(npart4)])
        for name in calls:
            spots = [i for i, line in enumerate(body)
                     if line.startswith("    ") and line[4:5] not in (" ", "}")] + [len(body)]
            body.insert(r.choice(spots), f"    {name}()")
        ending = []
        c = r.random()
        if c < 0.05:
            ending = ["    let z: i64 = 0", "    print(\"{1 / z}\")"]
        elif c < 0.08:
            ending = ["    assert(1 > 2)"]
        elif c < 0.12:
            ending = [f"    exit({r.randint(0, 300)})"]
        elif c < 0.15:
            ending = ["    panic(\"stopped on purpose\")"]
        lines += ["fn main() {"] + body + ending + ["}"]
        return "\n".join(lines) + "\n"


HEAP_DECLS = """type Pt = { a: i64, s: str, tags: [str] }

type Sh = | Ci(r: i64) | Sq(w: i64, h: i64) | Lbl(name: str, p: Pt) | Nothing

type Oops = | Small(n: i64) | Named(why: str)

fn describe(xs: [i64], label: str) -> str {
    var out = label
    for x in xs {
        out += "{x},"
    }
    out
}

fn first_long(xs: [str], k: i64) -> str {
    for x in xs {
        if x.len() > k {
            return x
        }
    }
    "none"
}

fn bump(inout p: Pt, by: i64) {
    p.a += by
    p.tags.push("b{by}")
}
"""

WORDS = ["a", "bc", "Dog", "xyz", "", "hello world", "q-r", "ZZ top", "e"]


class HeapGen:
    """Statements over strings, lists, structs, a sum type, maps, tuples,
    options and fallible functions -- every heap construct of the subset --
    each printing what it computed, so a wrong count or a wrong value shows.
    Loops `break` and `continue` and helpers `return` early with values live,
    which is where a missing or an extra release would hide."""

    def __init__(self, rng):
        self.rng = rng
        self.n = 0
        self.nfall = 0

    def fresh(self, base):
        self.n += 1
        return f"{base}{self.n}"

    def lit(self):
        return json.dumps(self.rng.choice(WORDS))

    def small(self):
        return str(self.rng.randint(-5, 9))

    def fallible_fns(self):
        """Functions that can fail, some through others' `try`."""
        r = self.rng
        out = []
        self.nfall = r.randint(1, 3)
        for k in range(self.nfall):
            m = r.randint(2, 5)
            out += [
                f"fn ck{k}(x: i64) -> i64 ! Oops {{",
                f"    if x % {m} == {r.randint(0, m - 1)} {{",
                "        fail(Small(x))",
                "    }",
                f"    if x < {r.randint(-9, 0)} {{",
                "        fail(Named(\"neg {x}\"))",
                "    }",
                f"    x * {r.randint(1, 4)} + {r.randint(-3, 3)}",
                "}",
                "",
                f"fn chain{k}(x: i64) -> str ! Oops {{",
                f"    let a = try ck{k}(x)",
                f"    let b = try ck{r.randint(0, k)}(a + {r.randint(1, 3)})",
                "    var parts: [str] = []",
                "    parts.push(\"{a}\")",
                "    parts.push(\"{b}\")",
                "    parts.join(\"-\")",
                "}",
                "",
            ]
        return out

    def function(self, name):
        r = self.rng
        lines = [f"fn {name}() {{"]
        self.vars = {"str": [], "list": [], "slist": [], "pt": [], "map": []}
        for _ in range(r.randint(8, 16)):
            lines += ["    " + x for x in self.stmt()]
        lines += ["}", ""]
        return lines

    def pick(self, kind):
        vs = self.vars[kind]
        return self.rng.choice(vs) if vs else None

    def stmt(self):
        r = self.rng
        c = r.random()
        if c < 0.12 or not self.vars["str"]:
            v = self.fresh("s")
            self.vars["str"].append(v)
            if r.random() < 0.5:
                return [f"var {v} = {self.lit()}"]
            return [f"var {v} = \"<{{{self.small()}}}|{{{self.lit()}}}>\""]
        if c < 0.20:
            v = self.pick("str")
            if r.random() < 0.5:
                return [f"{v} += {self.lit()}", f"print({v})"]
            o = self.pick("str")
            return [f"{v} = {v} + \"[\" + {o} + \"]\"", f"print({v})"]
        if c < 0.27:
            v = self.pick("str")
            return [f"print(\"{{{v}.len()}} {{{v}.to_upper()}} {{{v}.contains(\"o\")}} "
                    f"{{{v}.trim()}} {{{v}.replace(\"o\", \"0\")}} {{{v}.split(\" \").join(\"_\")}} "
                    f"{{{v}.pad_start(8, \".\")}} {{{v} == {self.lit()}}} {{{v} < {self.lit()}}}\")"]
        if c < 0.35 or not self.vars["list"]:
            v = self.fresh("xs")
            self.vars["list"].append(v)
            items = ", ".join(self.small() for _ in range(r.randint(1, 5)))
            return [f"var {v} = [{items}]"]
        if c < 0.43:
            v = self.pick("list")
            k = r.randint(0, 7)
            out = [f"{v}.push({self.small()})", f"{v}[{k} % {v}.len()] = {self.small()}"]
            if r.random() < 0.4:
                out += [f"if {v}.len() > 1 {{", f"    print(\"popped {{{v}.pop() ?? -100}}\")", "}"]
            if r.random() < 0.3:
                out += [f"{v}.sort()"]
            return out + [f"print(\"{{{v}}} {{{v}.sum()}} {{{v}.rev()}} {{{v}.slice(1, 3)}} "
                          f"{{{v}.contains(3)}} {{{v}.get({k}) ?? -1}} {{describe({v}, \"L\")}}\")"]
        if c < 0.50:
            v = self.pick("list")
            acc = self.fresh("acc")
            lim = r.randint(0, 6)
            return [f"var {acc} = \"\"",
                    f"for x in {v} {{",
                    f"    if x > {lim} {{",
                    f"        {r.choice(['break', 'continue'])}",
                    "    }",
                    "    let t = \"<{x}>\"",
                    f"    if x == {r.randint(-2, 4)} {{",
                    "        continue",
                    "    }",
                    f"    {acc} += t",
                    "}",
                    f"print({acc})"]
        if c < 0.58:
            v = self.fresh("p")
            self.vars["pt"].append(v)
            s = self.pick("str")
            return [f"var {v} = Pt{{ a: {self.small()}, s: {s}, tags: [{self.lit()}] }}",
                    f"{v}.a += {self.small()}",
                    f"{v}.tags.push({s})",
                    f"bump({v}, {self.small()})",
                    f"print({v})"]
        if c < 0.64 and self.vars["pt"]:
            p = self.pick("pt")
            ps = self.fresh("ps")
            return [f"var {ps}: [Pt] = [{p}]",
                    f"{ps}.push(Pt{{ a: 1, s: \"n\", tags: [] }})",
                    f"{ps}[0].a = {self.small()}",
                    f"{ps}[1].tags.push({self.lit()})",
                    f"print({ps})",
                    f"print({p}.s)"]
        if c < 0.72:
            sh = self.fresh("sh")
            s = self.pick("str")
            ctor = r.choice([f"Ci({self.small()})", f"Sq({self.small()}, {self.small()})",
                             f"Lbl({s}, Pt{{ a: 2, s: {s}, tags: [{s}] }})", "Nothing"])
            return [f"let {sh} = {ctor}",
                    f"match {sh} {{",
                    f"    Ci(r) if r > {r.randint(0, 5)} -> print(\"big circle {{r}}\")",
                    "    Ci(r) -> print(\"circle {r}\")",
                    "    Sq(w, h) -> print(\"square {w * h}\")",
                    "    Lbl(n, q) -> print(\"label {n} {q.a} {q.tags}\")",
                    "    Nothing -> print(\"nothing\")",
                    "}",
                    f"print({sh})"]
        if c < 0.80:
            m = self.fresh("m")
            s = self.pick("str")
            out = [f"var {m}: Map[str, i64] = {{}}"]
            for _ in range(r.randint(1, 4)):
                out.append(f"{m}.set({r.choice([s, self.lit()])}, {self.small()})")
            out += [f"print(\"{{{m}}} {{{m}.len()}} {{{m}.get({s}) ?? -1}} {{{m}.has({self.lit()})}} {{{m}.keys()}}\")",
                    f"print({m}.delete({self.lit()}))"]
            return out
        if c < 0.86:
            t = self.fresh("t")
            s = self.pick("str")
            return [f"let {t} = ({self.small()}, {s})",
                    f"let ({t}a, {t}b) = {t}",
                    f"print(\"{{{t}.0}} {{{t}.1}} {{{t}a}} {{{t}b}} {{{t}}}\")",
                    f"match {t} {{",
                    f"    ({r.randint(-1, 3)}, w) -> print(\"tuple {{w}}\")",
                    "    _ -> print(\"tuple other\")",
                    "}"]
        if c < 0.94 and self.nfall > 0:
            k = r.randrange(self.nfall)
            x = self.small()
            return [f"match ck{k}({x}) {{",
                    "    Ok(v) -> print(\"ok {v}\")",
                    "    Err(Small(n)) -> print(\"small {n}\")",
                    "    Err(Named(w)) -> print(\"named {w}\")",
                    "}",
                    f"print(\"{{ck{k}({self.small()}) ?? -7}}\")",
                    f"match chain{k}({x}) {{",
                    "    Ok(s) -> print(\"chain {s}\")",
                    "    Err(e) -> print(\"chain failed {e}\")",
                    "}"]
        sl = self.fresh("ws")
        s = self.pick("str")
        return [f"let {sl} = [{self.lit()}, {s}, {self.lit()}]",
                f"print(first_long({sl}, {r.randint(0, 5)}))",
                f"print(\"{{{sl}}} {{{sl}.len()}} {{{sl}.join(\"+\")}}\")"]


FLOAT_DECLS = """type Vec2 = { x: f64, y: f64, tag: i64 }

fn fnan() -> f64 {
    let z = 0.0
    z / z
}

fn finf() -> f64 {
    1.0 / 0.0
}

fn fmix(a: f64, b: i64, c: f64, d: f64, e: f64, f: f64, g: f64, h: f64, i: f64, j: f64, k: i64, l: f64) -> f64 {
    a * 1.5 + b.as_f64() - c / 2.0 + d * e - f + g * h + i - j + k.as_f64() * l
}

fn fdiv(x: f64, y: f64) -> f64 ! str {
    if y == 0.0 {
        fail("divide {x} by zero")
    }
    x / y
}

fn fchain(x: f64, y: f64) -> f64 ! str {
    let q = try fdiv(x, y)
    let r = try fdiv(q, x - 1.0)
    q + r
}

fn fscale(inout v: Vec2, by: f64) {
    v.x *= by
    v.y = v.y * by + 0.5
    v.tag += 1
}

fn fsum(xs: [f64]) -> f64 {
    var t = 0.0
    for x in xs {
        if x != x {
            continue
        }
        t += x
    }
    t
}
"""

FLOAT_LITS = ["0.0", "-0.0", "1.0", "-1.0", "0.5", "-2.5", "0.1", "3.0", "1e300", "-1e300",
              "2.5e-3", "1e-310", "123456.789", "9007199254740993.0", "4.9e-324", "255.9", "-128.5",
              "1.7976931348623157e308", "0.30000000000000004", "65535.5"]


class FloatGen:
    """Statements over `f64`: literals at the edges (both zeros, subnormals,
    the largest finite values) and NaN and the infinities computed; the four
    operations, `%` and `^`; every comparison, as a value and as a branch, NaN
    among the operands; conversions to and from every integer type and to the
    bits; the methods compiled as instructions and the ones the runtime does;
    doubles in lists, structs (one changed through an `inout` parameter),
    tuples, options, maps and results; a call with twelve arguments, ten of
    them doubles, so two travel on the stack; and twenty doubles live across
    calls, more than the registers a call preserves."""

    def __init__(self, rng):
        self.rng = rng
        self.n = 0

    def fresh(self, base):
        self.n += 1
        return f"{base}{self.n}"

    def lit(self):
        x = self.rng.choice(FLOAT_LITS)
        return f"({x})" if x.startswith("-") else x

    def leaf(self, fs):
        r = self.rng.random()
        if fs and r < 0.55:
            return self.rng.choice(fs)
        if r < 0.65:
            return self.rng.choice(["fnan()", "finf()", "(-finf())"])
        return self.lit()

    def fexpr(self, fs, d=0):
        r = self.rng
        if d >= 3 or r.random() < 0.3:
            return self.leaf(fs)
        c = r.random()
        if c < 0.45:
            op = r.choice(["+", "-", "*", "/", "+", "*"])
            return f"({self.fexpr(fs, d + 1)} {op} {self.fexpr(fs, d + 1)})"
        if c < 0.52:
            return f"({self.fexpr(fs, d + 1)} % {self.fexpr(fs, d + 1)})"
        if c < 0.56:
            return f"({self.fexpr(fs, d + 1)} ^ {r.choice(['2.0', '0.5', '-1.0', '3.0'])})"
        if c < 0.64:
            return f"(-{self.fexpr(fs, d + 1)})"
        if c < 0.80:
            m = r.choice(["abs", "sqrt", "floor", "ceil", "round", "sin", "exp", "ln"])
            return f"{self.fexpr(fs, d + 1)}.{m}()"
        if c < 0.86:
            m = r.choice(["min", "max", "atan2", "hypot"])
            return f"{self.fexpr(fs, d + 1)}.{m}({self.fexpr(fs, d + 1)})"
        if c < 0.92:
            return f"({r.randint(-9, 9)}).as_f64()"
        cmp = r.choice(["<", "<=", ">", ">=", "==", "!="])
        return (f"(if {self.fexpr(fs, d + 1)} {cmp} {self.fexpr(fs, d + 1)} "
                f"{{ {self.fexpr(fs, d + 1)} }} else {{ {self.fexpr(fs, d + 1)} }})")

    def function(self, name):
        r = self.rng
        lines = [f"fn {name}() {{"]
        fs = []
        for _ in range(r.randint(2, 4)):
            v = self.fresh("g")
            lines.append(f"    var {v}: f64 = {self.fexpr(fs)}")
            fs.append(v)
        for _ in range(r.randint(8, 14)):
            lines += ["    " + x for x in self.stmt(fs)]
        lines += ["}", ""]
        return lines

    def stmt(self, fs):
        r = self.rng
        c = r.random()
        a = r.choice(fs)
        b = r.choice(fs)
        if c < 0.10:
            v = self.fresh("g")
            line = f"var {v}: f64 = {self.fexpr(fs)}"
            fs.append(v)
            return [line]
        if c < 0.18:
            op = r.choice(["=", "+=", "-=", "*=", "/="])
            return [f"{a} {op} {self.fexpr(fs)}", f"print({a})"]
        if c < 0.26:
            return [f"print(\"{{{a} < {b}}} {{{a} <= {b}}} {{{a} > {b}}} {{{a} >= {b}}} {{{a} == {b}}} {{{a} != {b}}}\")"]
        if c < 0.34:
            cmp = r.choice(["<", "<=", ">", ">=", "==", "!="])
            neg = r.choice(["", "not "])
            return [f"if {neg}({a} {cmp} {b}) {{",
                    f"    print(\"{self.fresh('br')} taken {{{a}}}\")",
                    "} else {",
                    f"    print(\"{self.fresh('br')} not taken {{{b}}}\")",
                    "}"]
        if c < 0.42:
            return [f"print(\"{{{a}.as_i64()}} {{{a}.as_u64()}} {{{a}.as_u8()}} {{{a}.as_i8()}} {{{a}.as_i32()}} "
                    f"{{{a}.as_u16()}} {{{a}.to_bits()}} {{{a}.to_bits().from_bits()}} {{{a}.is_nan()}} {{{a}.is_finite()}}\")"]
        if c < 0.50:
            n = r.randint(-300, 300)
            u = r.randint(0, 2 ** 64 - 1)
            ci = self.fresh("ci")
            cu = self.fresh("cu")
            return [f"let {ci}: i32 = {n}",
                    f"let {cu}: u64 = {u}",
                    f"print(\"{{{ci}.as_f64()}} {{{cu}.as_f64() / 3.0}} {{{a}.to_str()}} {{{a}.to_fixed({r.randint(0, 4)})}} {{{a}.cmp({b})}}\")"]
        if c < 0.58:
            xs = self.fresh("fl")
            return [f"var {xs}: [f64] = [{a}, {self.fexpr(fs)}, {self.lit()}]",
                    f"{xs}.push({b})",
                    f"{xs}[{r.randint(0, 3)}] = {self.fexpr(fs)}",
                    f"{xs}[0] += 1.5",
                    f"print(\"{{{xs}}} {{{xs}.len()}} {{fsum({xs})}} {{{xs}[1]}}\")",
                    f"{xs}.sort()",
                    f"print(\"{{{xs}}} {{{xs}.contains({a})}}\")"]
        if c < 0.66:
            p = self.fresh("vv")
            return [f"var {p} = Vec2{{ x: {a}, y: {self.fexpr(fs)}, tag: {r.randint(-3, 3)} }}",
                    f"fscale({p}, {self.fexpr(fs)})",
                    f"{p}.y -= {b}",
                    f"var {p}s: [Vec2] = [{p}]",
                    f"{p}s[0].x = {self.lit()}",
                    f"fscale({p}s[0], 2.0)",
                    f"print(\"{{{p}}} {{{p}.x + {p}.y}} {{{p}s}}\")"]
        if c < 0.72:
            t = self.fresh("ft")
            return [f"let {t} = ({a}, {r.randint(0, 9)}, {b})",
                    f"let ({t}a, {t}n, {t}b) = {t}",
                    f"let {t}o: Option[f64] = if {t}n > 4 {{ Some({t}a) }} else {{ None }}",
                    f"var {t}m: Map[str, f64] = {{}}",
                    f"{t}m.set(\"k\", {t}b)",
                    f"print(\"{{{t}}} {{{t}.0}} {{{t}a * {t}b}} {{{t}o ?? -1.0}} {{{t}m.get(\"k\") ?? 0.0}} {{{t}m}}\")"]
        if c < 0.80:
            return [f"match fdiv({a}, {r.choice([b, '0.0', self.lit()])}) {{",
                    "    Ok(q) -> print(\"quotient {q}\")",
                    "    Err(e) -> print(\"failed: {e}\")",
                    "}",
                    f"print(\"{{fchain({a}, {b}) ?? -9.5}}\")"]
        if c < 0.88:
            args = [self.fexpr(fs, 2) if k not in (1, 10) else str(r.randint(-5, 5)) for k in range(12)]
            return [f"print(fmix({', '.join(args)}))"]
        # Twenty doubles live across a call.
        names = [self.fresh("h") for _ in range(20)]
        out = []
        for k, nm in enumerate(names):
            out.append(f"let {nm} = {r.choice(fs)} * {k + 1}.0 + {self.lit()}")
        margs = [("2" if k == 1 else "3" if k == 10 else names[k]) for k in range(12)]
        out.append(f"print(\"mid {{fsum([{names[0]}, {names[1]}])}} {{fmix({', '.join(margs)})}}\")")
        out.append(f"print({' + '.join(names)})")
        return out


CLOSURE_DECLS = """type Ops = { name: str, f: fn(i64) -> i64 }

fn cl_add(x: i64) -> i64 {
    x + 1
}

fn cl_adder(n: i64) -> fn(i64) -> i64 {
    |x| x + n
}

fn cl_apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn cl_twice(f: fn(i64) -> i64) -> fn(i64) -> i64 {
    |x| f(f(x))
}

fn cl_label(tag: str) -> fn(i64) -> str {
    |x| "{tag}{x}"
}
"""


class ClosureGen:
    """Closures and function values: closures capturing integers, strings,
    doubles, lists and structs by value, made, stored (in variables, a list,
    a struct's field) and called; named functions as values; closures
    answered by functions and closing over their parameters; the
    higher-order list methods with closure literals and named functions,
    `sort_by` through a struct's field; closures that answer `Ok` or `Err`;
    a closure making and calling another. Each prints what it computed."""

    def __init__(self, rng):
        self.rng = rng
        self.n = 0

    def fresh(self, base):
        self.n += 1
        return f"{base}{self.n}"

    def small(self):
        return str(self.rng.randint(-4, 9))

    def function(self, name):
        r = self.rng
        c = self.fresh("c")
        s = self.fresh("s")
        f = self.fresh("fl")
        xs = self.fresh("xs")
        ws = self.fresh("ws")
        items = ", ".join(self.small() for _ in range(r.randint(0, 6)))
        lines = [f"fn {name}() {{",
                 f"    let {c} = {self.small()}",
                 f"    let {s} = {json.dumps(r.choice(WORDS))}",
                 f"    let {f} = {r.choice(['0.5', '1.25', '-2.0', '3.0'])}",
                 f"    var {xs}: [i64] = [{items}]",
                 f"    let {ws}: [str] = [{', '.join(json.dumps(r.choice(WORDS)) for _ in range(r.randint(0, 4)))}]"]
        for _ in range(r.randint(6, 12)):
            lines += ["    " + x for x in self.stmt(c, s, f, xs, ws)]
        lines += ["}", ""]
        return lines

    def stmt(self, c, s, f, xs, ws):
        r = self.rng
        k = r.random()
        if k < 0.10:
            g = self.fresh("g")
            return [f"let {g}: fn(i64) -> i64 = |x| x * {c} + {self.small()}",
                    f"print(\"{{{g}(3)}} {{cl_apply({g}, {self.small()})}} {{cl_twice({g})(1)}}\")"]
        if k < 0.20:
            op = r.choice(["+", "-", "*"])
            return [f"print({xs}.map(|x| x {op} {c}))",
                    f"print({xs}.filter(|x| x % 2 == 0 or x > {c}))"]
        if k < 0.28:
            return [f"print(\"{{{xs}.fold(0, |a, x| a + x * {c})}} {{{xs}.any(|x| x > {c})}} "
                    f"{{{xs}.all(|x| x != {c})}} {{{xs}.find(|x| x > {c})}}\")"]
        if k < 0.36:
            return [f"print({ws}.map(|w| w + {s}))",
                    f"print({ws}.filter(|w| w.len() > {r.randint(0, 3)}))",
                    f"print({ws}.fold({s}, |a, w| a + \"|\" + w))"]
        if k < 0.42:
            return [f"print({xs}.map(|x| x.as_f64() * {f}))",
                    f"print({xs}.map(|x| \"<{{x}}{{{s}}}>\"))"]
        if k < 0.50:
            ys = self.fresh("ys")
            return [f"var {ys} = {xs}",
                    f"{ys}.push({self.small()})",
                    f"{ys}.sort_by(|x| {r.choice(['0 - x', 'x * x', 'x % 3'])})",
                    f"print({ys})"]
        if k < 0.56:
            h = self.fresh("h")
            return [f"let {h}: fn(i64) -> i64 = cl_add",
                    f"print({xs}.map(cl_add))",
                    f"print(\"{{{h}(41)}} {{cl_adder({c})(10)}} {{cl_label({s})(7)}} {{{h}}}\")"]
        if k < 0.64:
            o = self.fresh("ops")
            return [f"let {o} = [Ops{{ name: {s}, f: |x| x + {c} }}, Ops{{ name: \"sq\", f: |x| x * x }}, Ops{{ name: \"k\", f: cl_adder({self.small()}) }}]",
                    f"for op in {o} {{",
                    f"    print(\"{{op.name}} {{op.f({self.small()})}}\")",
                    "}",
                    f"print({o}.map(|op| op.f(2)))"]
        if k < 0.72:
            rs = self.fresh("rs")
            return [f"let {rs} = {xs}.map(|x| if x > {c} {{ Err(\"big {{x}}\") }} else {{ Ok(x * 2) }})",
                    f"print({rs})",
                    f"if {rs}.len() > 0 {{",
                    f"    print({rs}[0] ?? -1)",
                    "}"]
        if k < 0.80:
            o = self.fresh("outer")
            return [f"let {o}: fn(i64) -> i64 = |a| {{",
                    f"    let inner: fn(i64) -> i64 = |b| a + b + {c}",
                    "    inner(a * 2)",
                    "}",
                    f"print(\"{{{o}(1)}} {{{o}({self.small()})}}\")"]
        if k < 0.88:
            sq = self.fresh("pq")
            return [f"var {sq}: [Ops] = []",
                    f"for i in 0..{r.randint(0, 3)} {{",
                    f"    {sq}.push(Ops{{ name: \"n{{i}}\", f: cl_adder(i * {c}) }})",
                    "}",
                    f"print({sq}.map(|op| op.f({self.small()})))"]
        return [f"print((0..{r.randint(0, 5)}).map(|i| i * {c}))",
                f"print((1..={r.randint(1, 6)}).filter(|i| i % 2 == 1))"]


PART4_DECLS = """type RNode = | RLeaf(n: i64) | RPair(l: ref RNode, r: ref RNode)

type RCell = { v: i64, items: [i64], tag: str }

effect REmit {
    fn emit(n: i64) -> i64
}

fn rn_sum(t: ref RNode) -> i64 {
    match t {
        RLeaf(n) -> n
        RPair(l, r) -> rn_sum(l) * 2 + rn_sum(r)
    }
}

fn r_bump(c: ref RCell, k: i64) {
    c.v += k
    c.items.push(k)
    c.tag = c.tag + "+"
}

fn r_span(a: i64, n: i64) -> range {
    a..(a + n)
}

fn r_emit_twice(x: i64) -> i64 / {REmit} {
    REmit.emit(x) + REmit.emit(x + 1)
}
"""


class Part4Gen:
    """Part four's constructs: references -- cells written through `.value`
    and through fields and elements, a structure of them matched, a list of
    them, released and asked after -- ranges held as values and walked, an
    effect handled (nested, the innermost answering), `parallel` tasks in
    order, a parallel map, a struct updated with `..`, a builder, and the
    bit operations at a width. Each prints what it computed.

    A string field is appended to through a reference as `c.tag = c.tag +
    ..`, not `+=`: the Cranelift build, the reference here, crashes on the
    second `+=` (docs/known-issues.md, 2026-10-25)."""

    def __init__(self, rng):
        self.rng = rng
        self.n = 0

    def fresh(self, base):
        self.n += 1
        return f"{base}{self.n}"

    def small(self):
        return str(self.rng.randint(-4, 9))

    def function(self, name):
        lines = [f"fn {name}() {{"]
        for _ in range(self.rng.randint(4, 9)):
            lines += ["    " + x for x in self.stmt()]
        lines += ["}", ""]
        return lines

    def stmt(self):
        r = self.rng
        k = r.random()
        if k < 0.12:
            c = self.fresh("cell")
            return [f"let {c} = ref({self.small()})",
                    f"{c}.value += {self.small()}",
                    f"{c}.value = {c}.value * {self.small()}",
                    f"print(\"{{{c}.value}} {{alive({c})}}\")"]
        if k < 0.24:
            c = self.fresh("rc")
            return [f"let {c} = ref(RCell{{ v: {self.small()}, items: [], tag: {json.dumps(r.choice(WORDS))} }})",
                    f"r_bump({c}, {self.small()})",
                    f"r_bump({c}, {self.small()})",
                    f"{c}.items[{r.randint(0, 1)}] = {self.small()}",
                    f"{c}.items.push({c}.v)",
                    f"print(\"{{{c}.v}} {{{c}.items}} {{{c}.tag}}\")"]
        if k < 0.34:
            t = self.fresh("tree")
            a, b, c = self.small(), self.small(), self.small()
            return [f"let {t} = ref(RPair(ref(RLeaf({a})), ref(RPair(ref(RLeaf({b})), ref(RLeaf({c}))))))",
                    f"print(rn_sum({t}))"]
        if k < 0.44:
            rs = self.fresh("rs")
            tot = self.fresh("tot")
            return [f"let {rs} = [ref({self.small()}), ref({self.small()}), ref({self.small()})]",
                    f"{rs}[{r.randint(0, 2)}].value = {self.small()}",
                    f"var {tot} = 0",
                    f"for x in {rs} {{",
                    f"    {tot} += x.value",
                    "}",
                    f"release({rs}[2])",
                    f"print(\"{{{tot}}} {{{rs}[0] == {rs}[0]}} {{{rs}[0] == {rs}[1]}} {{alive({rs}[2])}}\")"]
        if k < 0.56:
            g = self.fresh("rg")
            a, n = self.small(), r.randint(0, 5)
            incl = r.choice(["..", "..="])
            return [f"let {g} = r_span({a}, {n})",
                    f"let {g}b = {a}{incl}{r.randint(-2, 8)}",
                    f"print(\"{{{g}}} {{{g}.len()}} {{{g}.sum()}} {{{g}.to_list()}} {{{g}b}} {{{g}b.len()}} {{{g} == {g}b}}\")",
                    f"print({g}.map(|i| i * {self.small()}))",
                    f"for i in {g}b {{",
                    "    print(i)",
                    "}"]
        if k < 0.66:
            m = self.small()
            return [f"with REmit handled_by REmitHandler{{ emit: |n| n * {m} }} {{",
                    f"    print(r_emit_twice({self.small()}))",
                    f"    with REmit handled_by REmitHandler{{ emit: |n| n + {self.small()} }} {{",
                    f"        print(REmit.emit({self.small()}))",
                    "    }",
                    f"    print(REmit.emit({self.small()}))",
                    "}"]
        if k < 0.74:
            lg = self.fresh("log")
            return [f"var {lg} = \"\"",
                    "parallel {",
                    "    spawn {",
                    f"        {lg} += \"a\"",
                    "    }",
                    "    spawn {",
                    f"        {lg} += \"{r.choice(WORDS)}\"",
                    "    }",
                    "}",
                    f"print({lg})"]
        if k < 0.82:
            xs = ", ".join(self.small() for _ in range(r.randint(0, 7)))
            return [f"print([{xs}].par_map(|x| x * {self.small()} + 1))",
                    f"print([{xs}].par_filter(|x| x % 2 == 0))"]
        if k < 0.88:
            c = self.fresh("up")
            return [f"let {c} = RCell{{ v: {self.small()}, items: [{self.small()}], tag: \"t\" }}",
                    f"let {c}b = RCell{{ ..{c}, v: {self.small()} }}",
                    f"print(\"{{{c}}} {{{c}b}}\")"]
        if k < 0.94:
            sb = self.fresh("sb")
            return [f"var {sb} = str_builder()",
                    f"{sb}.push_str({json.dumps(r.choice(WORDS))})",
                    f"{sb}.push_str(\"{{{self.small()}}}\")",
                    f"print(\"{{{sb}.len()}} {{{sb}.to_str()}}\")"]
        w = r.choice(["u8", "i8", "u16", "i32", "u64", "i64"])
        v = self.fresh("bv")
        return [f"let {v}: {w} = {r.randint(0, 120)}",
                f"print(\"{{{v}.count_ones()}} {{{v}.leading_zeros()}} {{{v}.trailing_zeros()}} "
                f"{{{v}.rotate_left({r.randint(0, 70)})}} {{{v}.rotate_right({r.randint(0, 70)})}} {{{v}.swap_bytes()}}\")"]


def run(cmd, timeout=20, env=None):
    try:
        p = subprocess.run(cmd, capture_output=True, timeout=timeout, stdin=subprocess.DEVNULL, env=env)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, b"", b"timeout"


def link_in_place(obj, out):
    """Links `obj` into `out`, keeping `out`'s inode (`lib/inplace.py`)."""
    stage = out + ".stage"
    p = subprocess.run(["cc", obj, RUNTIME, "-o", stage, "-Wl,-dead_strip"], capture_output=True, text=True)
    if p.returncode != 0:
        return p
    data = open(stage, "rb").read()
    if os.path.exists(out):
        with open(out, "r+b") as f:
            f.truncate(0)
            f.write(data)
        os.remove(stage)
    else:
        os.replace(stage, out)
    return p


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--programs", type=int, default=60)
    ap.add_argument("--seed", type=int, default=20261021)
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()
    work = tempfile.mkdtemp(prefix="fuzz_s3_")
    # `S3=<binary>` fuzzes that driver instead of one built here, with
    # `S3_EXTRA` added to each compilation, as the other S3 gates take them
    # (`scripts/s3controls.sh` points them at a deliberately broken one).
    s3 = os.environ.get("S3") or os.path.join(work, "s3")
    extra = (os.environ.get("S3_EXTRA") or "").split()
    if not os.environ.get("S3"):
        p = subprocess.run([GRASM, "build", os.path.join(ROOT, "compiler/s3/src/main.gr"), "--native", "-o", s3],
                           capture_output=True, text=True, cwd=ROOT)
        if p.returncode != 0:
            print("compiler/s3 does not build:")
            print(p.stdout + p.stderr)
            return 1
    rng = random.Random(args.seed)
    made = 0
    not_checked = 0
    agree = 0
    stressed = 0
    panicked = 0
    leak_checked = 0
    failures = []
    for idx in range(args.programs):
        g = Gen(random.Random(rng.random()))
        src_text = g.program()
        src = os.path.join(work, "p.gr")
        with open(src, "w") as f:
            f.write(src_text)
        tir = os.path.join(work, "p.tir")
        p = subprocess.run([GRASM, "tir", src], capture_output=True, text=True)
        if p.returncode != 0:
            # The generator wrote something the checker refuses: a generator
            # bug, counted and shown, never compared.
            not_checked += 1
            if not_checked <= 3:
                print(f"program {idx} does not check:\n{p.stderr[:600]}")
                if args.keep:
                    bad = os.path.join(work, f"unchecked_{idx}.gr")
                    with open(bad, "w") as f:
                        f.write(src_text)
                    print(f"  kept: {bad}")
            continue
        with open(tir, "w") as f:
            f.write(p.stdout)
        made += 1
        reasons = s3_subset.classify(s3_subset.parse(p.stdout))
        if reasons:
            failures.append((idx, "outside the subset: " + "; ".join(sorted(set(reasons)))[:300], src_text))
            continue
        ref = os.path.join(work, "ref")
        b = build_native(GRASM, src, ref, "--native")
        if b.returncode != 0 or not os.path.exists(ref):
            failures.append((idx, "the Cranelift build fails: " + (b.stderr or "")[:300], src_text))
            continue
        want = run([ref])
        if want[0] == 101:
            panicked += 1
        ok = True
        for regs in (None, 3):
            obj = os.path.join(work, "p.o")
            cmd = [s3, tir, "-o", obj] + ([] if regs is None else ["--max-regs", str(regs)]) + extra
            c = run(cmd)
            if c[0] != 0:
                failures.append((idx, f"the backend answered {c[0]} (regs {regs}): {c[2][:300]!r}", src_text))
                ok = False
                break
            exe = os.path.join(work, "s3exe")
            ln = link_in_place(obj, exe)
            if ln.returncode != 0:
                failures.append((idx, f"the object does not link: {ln.stderr[:300]}", src_text))
                ok = False
                break
            got = run([exe])
            if got != want:
                failures.append((idx, f"regs {regs}: Cranelift exit {want[0]}, s3 exit {got[0]}\n"
                                      f"  Cranelift: {want[1][-300:]!r} {want[2][-200:]!r}\n"
                                      f"  s3:        {got[1][-300:]!r} {got[2][-200:]!r}", src_text))
                ok = False
                break
            # Its reference counting: nothing live when `main` returns
            # (`scripts/s3rc.sh` checks the corpus the same way).
            if regs is None and want[0] == 0:
                lk = run([exe], env=dict(os.environ, GRASM_LEAK_CHECK="1"))
                if lk[0] != 0 or b"leak check: nothing live at exit" not in lk[2]:
                    failures.append((idx, f"the leak check: exit {lk[0]}, {lk[2][-200:]!r}", src_text))
                    ok = False
                    break
                leak_checked += 1
            if regs is not None:
                stressed += 1
        if ok:
            agree += 1
    for idx, why, text in failures[:5]:
        print(f"FAILED program {idx}: {why}")
        if args.keep:
            path = os.path.join(work, f"failed_{idx}.gr")
            with open(path, "w") as f:
                f.write(text)
            print(f"  kept: {path}")
    if not args.keep:
        shutil.rmtree(work, ignore_errors=True)
    print(f"programs: {args.programs}   compared: {made}   identical: {agree}   stressed: {stressed}   "
          f"leak-checked: {leak_checked}   panicked: {panicked}   failed: {len(failures)}   "
          f"not checked: {not_checked}")
    if failures:
        return 1
    # Most programs end by returning from `main`, and every one of those was
    # leak-checked; a fuzzer whose programs all panicked checked no counts.
    if leak_checked < agree // 2:
        print(f"only {leak_checked} of {agree} programs were leak-checked")
        return 1
    # A generator whose programs mostly fail to check is testing nothing.
    if not_checked * 10 > args.programs or agree < args.programs * 8 // 10:
        print("too few programs were compared for this to mean anything")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
