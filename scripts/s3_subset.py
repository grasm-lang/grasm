#!/usr/bin/env python3
"""Whether a program is in the subset S3 part one compiles, asked of its TIR.

    s3_subset.py <program.tir>      prints `in`, or `out: <every reason>`

The rule, stated once and here (docs/self-hosting.md, S3; the backend's own
refusals in `compiler/mir/src/lower.gr` are held to it by
`scripts/s3subset.sh`, in both directions):

A program is in the subset when its TIR declares a `main`, and every function
and module-level constant that `main` reaches -- through `call` nodes and
constant reads, transitively -- satisfies all of:

  1. Declarations. A function is not generic, cannot fail (its error type is
     `none`), has at most seven parameters, each of a scalar type in any mode
     (`let`, `own`, `inout`), and answers a scalar type. A constant has a
     scalar type. `main` takes no parameters and answers `unit`.
     The scalar types are i64, i8, i16, i32, u8, u16, u32, u64, bool, unit
     and never.

  2. Nodes. Its body holds only:
       int, bool, unit                     literals
       local, const                        of a scalar type
       binary add sub mul div rem          on an integer type
       binary pow                          on i64
       binary eq ne lt le gt ge            on an integer type or bool
       binary and or
       unary neg                           on an integer type
       unary not                           on bool
       call                                not generic, answering a scalar; an
                                           argument for an `inout` parameter
                                           is a `local`
       builtin print, eprint               of a `str` literal whose
                                           interpolated values are scalars, or
                                           of a scalar
       builtin assert                      of a bool
       builtin exit                        of an integer
       builtin panic                       of a `str` literal whose
                                           interpolated values are scalars
                                           other than u64
       if, block, blockexpr, match         of a scalar type
       match                               on an integer or a bool, with
                                           patterns pwild pint pbool pbind por,
                                           and guards
       let                                 of a scalar type
       assign set|add|sub|mul|div          of a scalar type, to a `local`
       return, break, continue, while
       for range                           over a range written in place

Everything else -- strings as values, lists, maps, structs, sum types,
closures, methods, effects, raw memory, C functions, fallible functions,
floats -- is heap values, RC and the rest, which is S3 part two.

The count of programs in and out is printed by the gate, never assumed.
"""
import json
import re
import sys

SCALAR = {"i64", "i8", "i16", "i32", "u8", "u16", "u32", "u64", "bool", "unit", "never"}
INTS = {"i64", "i8", "i16", "i32", "u8", "u16", "u32", "u64"}
STMTS = {"let", "lettuple", "assign", "return", "break", "continue", "while", "for",
         "parallel", "spawn", "with"}


def parse(src):
    """The TIR text as nested lists; strings stay quoted, so a string is never
    mistaken for a keyword."""
    toks = re.findall(r'"(?:[^"\\]|\\.)*"|\(|\)|[^\s()"]+', src)
    stack = [[]]
    for t in toks:
        if t == "(":
            stack.append([])
        elif t == ")":
            x = stack.pop()
            stack[-1].append(x)
        else:
            stack[-1].append(t)
    return stack[0]


def unq(s):
    return json.loads(s)


def ty(x):
    """A type element as text when it is an atom; None for a compound type."""
    return x if isinstance(x, str) else None


def scalar(x):
    return ty(x) in SCALAR


def span(e):
    for x in e:
        if isinstance(x, str) and x.startswith("@"):
            return x
    return ""


# How many elements of each node are its head (keyword, type, span and
# operands, written on the node's own line); the rest are its children. A
# head element may itself be a list -- a compound type, a call's `(targs)` --
# so children are found by position, not by being lists.
HEAD = {
    "binary": 5, "unary": 5, "call": 5, "builtin": 4, "if": 3, "block": 2,
    "blockexpr": 3, "match": 3, "arm": 2, "let": 5, "assign": 3, "return": 2,
    "while": 2, "for": 3, "range": 4, "str": 3, "por": 2, "else": 1, "elseif": 1,
    "value": 1, "guard": 1, "params": 1,
}


def kids(e):
    """A node's children: the elements after its head."""
    return list(e[HEAD.get(e[0], len(e)):])


class Checker:
    def __init__(self, fns, consts):
        self.fns = fns
        self.consts = consts
        self.reasons = []

    def bad(self, msg):
        self.reasons.append(msg)

    def expr(self, e):
        if not isinstance(e, list) or not e:
            self.bad("a bare atom where an expression was expected")
            return
        k = e[0]
        if k in ("int", "bool", "unit"):
            if k == "int" and ty(e[1]) not in INTS:
                self.bad(f"an integer of type {e[1]}")
            return
        if k == "local":
            if not scalar(e[1]):
                self.bad(f"a local of a non-scalar type {span(e)}")
            return
        if k == "const":
            if not scalar(e[1]):
                self.bad(f"a constant of a non-scalar type {span(e)}")
            return
        if k == "binary":
            op, ot = e[3], ty(e[4])
            if op in ("and", "or"):
                pass
            elif op in ("eq", "ne", "lt", "le", "gt", "ge"):
                if ot not in INTS and ot != "bool":
                    self.bad(f"`{op}` on a non-scalar {span(e)}")
            elif op == "pow":
                if ot != "i64":
                    self.bad(f"`^` on {e[4]} {span(e)}")
            elif op in ("add", "sub", "mul", "div", "rem"):
                if ot not in INTS:
                    self.bad(f"`{op}` on {e[4]} {span(e)}")
            else:
                self.bad(f"operator `{op}` {span(e)}")
            for x in kids(e):
                self.expr(x)
            return
        if k == "unary":
            op, ot = e[3], ty(e[4])
            if not ((op == "neg" and ot in INTS) or (op == "not" and ot == "bool")):
                self.bad(f"`{op}` on {e[4]} {span(e)}")
            self.expr(kids(e)[0])
            return
        if k == "call":
            name = unq(e[3])
            targs = e[4]
            if len(targs) > 1:
                self.bad(f"a generic call of `{name}`")
            if not scalar(e[1]):
                self.bad(f"a call of `{name}` answering a non-scalar")
            f = self.fns.get(name)
            if f is None:
                self.bad(f"a call of `{name}`, which is not a function")
                return
            params = f[7][1:]
            args = kids(e)
            for i, a in enumerate(args):
                if i < len(params) and params[i][1] == "inout":
                    if not (isinstance(a, list) and a and a[0] == "local"):
                        self.bad(f"an `inout` argument that is not a local {span(e)}")
                else:
                    self.expr(a)
            return
        if k == "builtin":
            name = unq(e[3])
            args = kids(e)
            if name in ("print", "eprint") and len(args) == 1:
                self.printed(args[0], allow_u64=True)
                return
            if name == "assert" and len(args) == 1:
                if ty(args[0][1]) != "bool":
                    self.bad("`assert` of a non-bool")
                self.expr(args[0])
                return
            if name == "exit" and len(args) == 1:
                if ty(args[0][1]) not in INTS:
                    self.bad("`exit` of a non-integer")
                self.expr(args[0])
                return
            if name == "panic" and len(args) == 1:
                if args[0][0] != "str":
                    self.bad("a panic message that is not a string literal")
                else:
                    self.printed(args[0], allow_u64=False)
                return
            self.bad(f"built-in `{name}` {span(e)}")
            return
        if k == "if":
            if not scalar(e[1]):
                self.bad(f"an `if` of a non-scalar type {span(e)}")
            ks = kids(e)
            self.expr(ks[0])
            self.expr(ks[1])
            if len(ks) > 2:
                self.expr(ks[2][1])
            return
        if k == "block":
            for x in kids(e):
                self.expr(x)
            return
        if k == "blockexpr":
            if not scalar(e[1]):
                self.bad(f"a block of a non-scalar type {span(e)}")
            self.expr(kids(e)[0])
            return
        if k == "match":
            if not scalar(e[1]):
                self.bad(f"a `match` of a non-scalar type {span(e)}")
            ks = kids(e)
            scrut = ks[0]
            st = ty(scrut[1])
            if st not in INTS and st != "bool":
                self.bad(f"matching on a non-integer {span(e)}")
            self.expr(scrut)
            for arm in ks[1:]:
                parts = kids(arm)
                self.pattern(parts[0])
                if len(parts) == 3:
                    self.expr(parts[1][1])
                self.expr(parts[-1])
            return
        if k == "let":
            if not scalar(e[4]):
                self.bad(f"a binding of a non-scalar type {span(kids(e)[0])}")
            self.expr(kids(e)[0])
            return
        if k == "assign":
            ks = kids(e)
            if ks[0][0] != "local":
                self.bad(f"assignment to a `{ks[0][0]}`")
                return
            if not scalar(e[2]):
                self.bad("assignment of a non-scalar type")
                return
            if e[1] != "set" and ty(e[2]) not in INTS:
                self.bad(f"`{e[1]}=` on {e[2]}")
            self.expr(ks[1])
            return
        if k == "return":
            for x in kids(e):
                self.expr(x)
            return
        if k in ("break", "continue"):
            return
        if k == "while":
            ks = kids(e)
            self.expr(ks[0])
            self.expr(ks[1])
            return
        if k == "for":
            if e[1] != "range":
                self.bad(f"`for` over a {e[1]}")
                return
            ks = kids(e)
            it = ks[1]
            if it[0] != "range":
                self.bad("`for` over a range that is not written in place")
                return
            for x in kids(it):
                self.expr(x)
            self.expr(ks[2])
            return
        self.bad(f"`{k}` {span(e)}")

    def printed(self, arg, allow_u64):
        if isinstance(arg, list) and arg and arg[0] == "str":
            for pc in kids(arg):
                if pc[0] == "text":
                    continue
                x = pc[1]
                t = ty(x[1]) if isinstance(x, list) and len(x) > 1 else None
                if t not in SCALAR or t == "never" or (t == "u64" and not allow_u64):
                    self.bad(f"interpolating a value of type {x[1]}")
                self.expr(x)
            return
        t = ty(arg[1]) if isinstance(arg, list) and len(arg) > 1 else None
        if t not in SCALAR or t == "never":
            self.bad("printing a value that is not a scalar")
        self.expr(arg)

    def pattern(self, p):
        k = p[0]
        if k in ("pwild", "pint", "pbool", "pbind"):
            return
        if k == "por":
            for x in kids(p):
                self.pattern(x)
            return
        self.bad(f"pattern `{k}`")


def callees(e, acc):
    if isinstance(e, list):
        if e and e[0] == "call":
            acc.add(("fn", unq(e[3])))
        if e and e[0] == "const" and len(e) > 3 and isinstance(e[2], str) and e[2].startswith("@"):
            acc.add(("const", unq(e[3])))
        for x in e:
            callees(x, acc)


def classify(tree):
    fns, consts, main = {}, {}, None
    for d in tree:
        if not isinstance(d, list) or not d:
            continue
        if d[0] == "fn":
            fns[unq(d[1])] = d
        elif d[0] == "const":
            consts[unq(d[1])] = d
        elif d[0] == "main":
            main = unq(d[1])
    if main is None:
        return ["the program has no `main`"]
    if main not in fns:
        return [f"`main` names `{main}`, which is not a function"]
    ck = Checker(fns, consts)
    seen = set()
    work = [("fn", main)]
    while work:
        item = work.pop()
        if item in seen:
            continue
        seen.add(item)
        kind, name = item
        d = fns.get(name) if kind == "fn" else consts.get(name)
        if d is None:
            continue
        acc = set()
        callees(d, acc)
        work.extend(sorted(acc))
        before = len(ck.reasons)
        if kind == "const":
            if not scalar(d[2]):
                ck.bad(f"constant `{name}` of type {d[2]}")
            ck.expr(d[3])
        else:
            # (fn "name" (generics ..) (bounds ..) RET ERR (row ..) (params ..) body)
            if len(d[2]) > 1:
                ck.bad(f"generic function `{name}`")
            if d[5] != "none":
                ck.bad(f"`{name}` can fail")
            if not scalar(d[4]):
                ck.bad(f"`{name}` answers {d[4]}")
            params = d[7][1:]
            if len(params) > 7:
                ck.bad(f"`{name}` has more than seven parameters")
            for p in params:
                if not scalar(p[4]):
                    ck.bad(f"a parameter of a non-scalar type in `{name}`")
            if name == main:
                if params:
                    ck.bad("`main` takes parameters")
                if d[4] != "unit":
                    ck.bad(f"`main` answers {d[4]}")
            ck.expr(d[8])
        for k in range(before, len(ck.reasons)):
            ck.reasons[k] = f"{name}: {ck.reasons[k]}"
    return ck.reasons


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    tree = parse(open(sys.argv[1], encoding="utf-8").read())
    reasons = classify(tree)
    if reasons:
        print("out: " + "; ".join(sorted(set(reasons))))
    else:
        print("in")
    return 0


if __name__ == "__main__":
    sys.exit(main())
