#!/usr/bin/env python3
"""Whether a program is in the subset the S3 backend compiles, asked of its TIR.

    s3_subset.py <program.tir>      prints `in`, or `out: <every reason>`

The rule, stated once and here (docs/self-hosting.md, S3; the backend's own
refusals in `compiler/mir/src/lower.gr` are held to it by
`scripts/s3subset.sh`, in both directions):

A program is in the subset when its TIR declares a `main`, and every function
and module-level constant that `main` reaches -- through `call` nodes,
functions used as values (`fnref`) and constant reads, transitively --
satisfies all of the following, after monomorphization.

Monomorphization. Every generic function the program uses is instantiated at
each list of type arguments it is used at, as a copy with its type
parameters replaced (`monomorphize`, the backend's `compiler/mir/src/mono.gr`
in Python); every use names its instance, and every `boundcall` -- an
interface method on a receiver of a generic type -- becomes a call of what
the receiver's `impl` provides, itself instantiated when that `impl` is
generic. What follows is asked of the instances. A generic function asking
for ever bigger instances of itself (polymorphic recursion) is outside.

Types. The *held* types are the scalars -- i64, i8, i16, i32, u8, u16, u32,
u64, f64, bool, unit (also written as the empty tuple), never -- and the
boxed types built from held types: str,
bytes, a list, a tuple, a map, a named struct or sum type with held type
arguments, a result `T ! E` (E may be `never`), a union of boxed types, and
a function type whose parameters and result are held (a function value).
References, ranges as values, builders and raw addresses are not held. An
f64 lives in a floating-point register and travels in d0..d7 (AAPCS64).

  1. Declarations. A function has held parameter types (any mode; past the
     eighth argument of each register class they travel on the stack) and a
     held return type. One that can fail fails with a boxed type. A constant
     has a held type. `main` takes no parameters and answers `unit`.

  2. Nodes. Its body holds only:
       int, bool, unit                     literals
       float                               a literal, which TIR writes as bits
       local, const                        of a held type
       str                                 a literal, or one interpolating
                                           held values other than `never`
       list, tuple                         of held element types
       index, tuplefield                   answering a held type
       struct                              of a struct type whose fields are
                                           all held, with no `..base`
       field                               answering a held type, of a named
                                           type
       variant                             of a sum type, every field held
       binary add sub mul div rem          on an integer type or f64
       binary pow                          on i64 or f64
       binary eq ne lt le gt ge            on an integer type, f64 or bool
       binary and or
       binary add                          on str or a list
       binary eq ne                        on any boxed type
       binary lt le gt ge                  on str or a tuple
       binary coalesce                     on an Option or a result, answering
                                           a held type
       unary neg                           on an integer type or f64
       unary not                           on bool
       call                                not generic, answering a held type;
                                           an argument for an `inout`
                                           parameter is a `local`
       builtin print, eprint               of a `str` literal interpolating
                                           held values other than `never`, or
                                           of such a value
       builtin assert                      of a bool
       builtin exit                        of an integer
       builtin panic                       of one piece of text, or of a str
       builtin fail                        of a boxed value
       builtin listed in RT_BUILTINS       of held values, answering a held
                                           type (one runtime call each)
       builtin pi, e                       of nothing, answering f64
       maplit                              of a held map type
       try                                 answering a held type
       wrapok                              of a held type
       method                              listed in METHODS below, on its
                                           receiver type
       if, block, blockexpr, match         of a held type
       match                               on a held type, with patterns pwild
                                           pint pbool pstr pbind por ptuple,
                                           pvariant (each field read held and
                                           not generic), pok and perr (of a
                                           held half), and guards
       let                                 of a held type
       lettuple                            every name of a held type
       assign set                          of a held type, to a local or to a
                                           path of struct fields (each held,
                                           not generic) and list elements
                                           rooted at a local
       assign add|sub|mul|div|rem          of an integer type or f64, to such a
                                           place
       assign add                          of str, to such a place
       return, break, continue, while
       for range                           over a range written in place
       for list                            over elements of a held type
       closure                             of a held function type, every
                                           variable it captures (reads and
                                           does not bind) of a held type
       fnref                               of a function with no `inout`
                                           parameter, as a held type
       callvalue                           of a held function type

A method listed in MUTATING changes its receiver, which must then be a
place: a local, or struct fields and list elements of held types rooted at
one. A method on a range (`map`, `filter`) is on a range written in place.

Everything else -- effects, raw memory, C functions, references, builders,
tasks -- is outside, for a later part.

The count of programs in and out is printed by the gate, never assumed.
"""
import json
import re
import sys

SCALAR = {"i64", "i8", "i16", "i32", "u8", "u16", "u32", "u64", "f64", "bool", "unit", "never"}
INTS = {"i64", "i8", "i16", "i32", "u8", "u16", "u32", "u64"}
STMTS = {"let", "lettuple", "assign", "return", "break", "continue", "while", "for",
         "parallel", "spawn", "with"}


class Node(list):
    """A list from the TIR text, knowing how many of its elements are its
    head: the ones on its own line (keyword, type, span, operands). The rest
    are its children, each on a line of its own."""
    head = None


def parse(src):
    """The TIR text as nested lists; strings stay quoted, so a string is never
    mistaken for a keyword. Each list knows where its children begin."""
    toks = re.finditer(r'"(?:[^"\\]|\\.)*"|\(|\)|\n|[^\s()"]+', src)
    stack = [Node()]
    starts = [[]]
    saw_nl = False
    for m in toks:
        t = m.group(0)
        if t == "\n":
            saw_nl = True
            continue
        if t == "(":
            stack[-1].append(None)
            starts[-1].append(saw_nl)
            stack.append(Node())
            starts.append([])
        elif t == ")":
            x = stack.pop()
            st = starts.pop()
            h = 1
            while h < len(x) and not st[h]:
                h += 1
            x.head = h
            stack[-1][-1] = x
        else:
            stack[-1].append(t)
            starts[-1].append(saw_nl)
        saw_nl = False
    return stack[0]


def unq(s):
    return json.loads(s)


def span(e):
    for x in e:
        if isinstance(x, str) and x.startswith("@"):
            return x
    return ""


def kids(e):
    """A node's children: the elements after its head."""
    h = getattr(e, "head", None)
    return list(e[h if h is not None else len(e):])


def atom(t):
    """A type element as text when it is an atom; None for a compound type."""
    return t if isinstance(t, str) else None


def head(t):
    """The keyword of a compound type, or None."""
    return t[0] if isinstance(t, list) and t else None


def show(t):
    if isinstance(t, list):
        return "(" + " ".join(show(x) for x in t) + ")"
    return str(t)


# The built-in methods compiled, as `family.name/arguments` (the family is the
# receiver's type: str, bytes, list, map, option, int -- i64 and the narrow
# integers -- and u64). Each is lowered as the Cranelift build lowers it, to
# the same runtime function.
METHODS = {
    "str.len/0", "str.byte_len/0", "str.is_empty/0", "str.chars/0", "str.codes/0",
    "str.substr/2", "str.split/1", "str.contains/1", "str.starts_with/1",
    "str.ends_with/1", "str.index_of/1", "str.to_lower/0", "str.to_upper/0",
    "str.trim/0", "str.trim_start/0", "str.trim_end/0", "str.replace/2",
    "str.repeat/1", "str.pad_start/2", "str.pad_end/2", "str.parse_int/0",
    "str.parse_int_base/1", "str.parse_u64/0", "str.code_at/1", "str.lines/0",
    "str.words/0", "str.to_bytes/0",
    "list.len/0", "list.is_empty/0", "list.get/1", "list.contains/1", "list.join/1",
    "list.sum/0", "list.rev/0", "list.slice/2", "list.first/0", "list.last/0",
    "list.min/0", "list.max/0", "list.enumerate/0", "list.zip/1", "list.push/1",
    "list.sort/0", "list.pop/0", "list.remove/1",
    "bytes.len/0", "bytes.is_empty/0", "bytes.at/1", "bytes.slice/2", "bytes.to_str/0",
    "bytes.to_list/0", "bytes.concat/1", "bytes.index_of/2", "bytes.set/2",
    "bytes.fill/1", "bytes.copy_from/2",
    "map.len/0", "map.is_empty/0", "map.get/1", "map.has/1", "map.keys/0",
    "map.values/0", "map.entries/0", "map.set/2", "map.delete/1",
    "option.is_some/0", "option.is_none/0", "option.unwrap_or/1", "option.expect/1",
}
for _fam in ("int", "u64"):
    for _m in ("bit_and/1", "bit_or/1", "bit_xor/1", "bit_not/0", "shl/1", "shr/1",
               "ushr/1", "to_char/0", "to_str/0", "to_base/1", "abs/0", "min/1", "max/1",
               "as_f64/0", "cmp/1"):
        METHODS.add(f"{_fam}.{_m}")
    for _w in INTS:
        METHODS.add(f"{_fam}.as_{_w}/0")
METHODS |= {"int.from_bits/0", "str.parse_float/0", "str.cmp/1", "tuple.cmp/1"}
# The higher-order methods: a loop calling the function value per element.
# A range's are on a range written in place.
METHODS |= {"list.map/1", "list.filter/1", "list.all/1", "list.any/1", "list.find/1",
            "list.fold/2", "list.sort_by/1", "range.map/1", "range.filter/1"}
# The methods of f64: four as instructions (abs, sqrt, floor, ceil), the
# conversions to integers (toward zero, saturating, NaN to 0), its bits, and
# the rest -- round, min, max, the math library, formatting -- by the runtime.
for _m in ("abs", "sqrt", "floor", "ceil", "round", "to_bits", "to_str", "is_nan",
           "is_finite", "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh",
           "tanh", "exp", "ln", "log2", "log10", "cbrt", "to_radians", "to_degrees"):
    METHODS.add(f"f64.{_m}/0")
for _m in ("min", "max", "atan2", "hypot", "log", "to_fixed", "cmp"):
    METHODS.add(f"f64.{_m}/1")
for _w in INTS:
    METHODS.add(f"f64.as_{_w}/0")

# The built-ins that are one runtime call, by how many arguments they take.
RT_BUILTINS = {
    "args": 0, "input": 0, "map": 0, "bytes_of": 1, "bytes_filled": 2, "read_file": 1,
    "read_bytes": 1, "list_dir": 1, "file_size": 1, "sync_file": 1, "remove_file": 1,
    "make_dir": 1, "make_dir_new": 1, "env": 1, "write_file": 2, "write_bytes": 2,
    "append_bytes": 2, "rename_file": 2, "file_exists": 1, "assert_eq": 2, "Ok": 1, "Err": 1,
}

# The methods that change their receiver, which must then be a place: a local,
# or a path of struct fields and list elements rooted at one.
MUTATING = {"list.push/1", "list.sort/0", "list.pop/0", "list.remove/1", "bytes.set/2",
            "bytes.fill/1", "bytes.copy_from/2", "map.set/2", "map.delete/1", "list.sort_by/1"}


class Checker:
    def __init__(self, fns, consts, types):
        self.fns = fns
        self.consts = consts
        self.types = types
        self.reasons = []
        self.vars = {}

    def bad(self, msg):
        self.reasons.append(msg)

    # ----- types -------------------------------------------------------------

    def kind(self, t):
        """'s' for a scalar, 'b' for a held boxed type, None otherwise."""
        if isinstance(t, str):
            if t in SCALAR:
                return "s"
            if t in ("str", "bytes"):
                return "b"
            return None
        h = head(t)
        if h == "list":
            return "b" if self.kind(t[1]) else None
        if h == "fallible":
            return "b" if self.kind(t[1]) and self.kind(t[2]) else None
        if h == "union":
            return "b" if all(self.kind(x) == "b" for x in t[1:]) else None
        if h == "tuple" and len(t) == 1:
            return "s"
        if h == "fn":
            # (fn (P..) R row): a function value, boxed, when what it takes
            # and answers are held.
            ps = t[1] if isinstance(t[1], list) else []
            return "b" if all(self.kind(x) for x in ps) and self.kind(t[2]) else None
        if h in ("tuple", "map", "named"):
            rest = t[2:] if h == "named" else t[1:]
            return "b" if all(self.kind(x) for x in rest) else None
        return None

    def held(self, t):
        return self.kind(t) is not None

    def boxed(self, t):
        return self.kind(t) == "b"

    def body(self, t):
        """The declaration body (`struct` or `sum`) of named type `t`."""
        if head(t) != "named":
            return None
        d = self.types.get(unq(t[1]))
        if d is None:
            return None
        ks = kids(d)
        return ks[-1] if ks else None

    @staticmethod
    def has_generic(t):
        if not isinstance(t, list):
            return False
        if t and t[0] == "generic":
            return True
        return any(Checker.has_generic(x) for x in t[1:])

    @staticmethod
    def subst(ft, args):
        if head(ft) == "generic":
            n = int(ft[1])
            if 0 <= n < len(args):
                return args[n]
        return ft

    def field_held(self, ft, args):
        """A declared field type, under the type's arguments, is held."""
        x = self.subst(ft, args)
        if self.has_generic(x):
            return head(x) in ("list", "tuple", "map", "named")
        return self.held(x)

    # ----- expressions ---------------------------------------------------------

    def expr(self, e):
        if not isinstance(e, list) or not e:
            self.bad("a bare atom where an expression was expected")
            return
        k = e[0]
        if k in ("int", "bool", "unit"):
            if k == "int" and e[1] not in INTS:
                self.bad(f"an integer of type {show(e[1])}")
            return
        if k == "float":
            if e[1] != "f64":
                self.bad(f"a float literal of type {show(e[1])}")
            return
        if k in ("local", "const"):
            if not self.held(e[1]):
                self.bad(f"a {k} of type {show(e[1])} {span(e)}")
            return
        if k == "str":
            self.pieces(e)
            return
        if k == "binary":
            self.binary(e)
            return
        if k == "unary":
            op, ot = e[3], e[4]
            if not ((op == "neg" and (atom(ot) in INTS or ot == "f64")) or (op == "not" and ot == "bool")):
                self.bad(f"`{op}` on {show(ot)} {span(e)}")
            self.expr(kids(e)[0])
            return
        if k == "call":
            self.call(e)
            return
        if k == "builtin":
            self.builtin(e)
            return
        if k == "method":
            self.method(e)
            return
        if k == "if":
            if not self.held(e[1]):
                self.bad(f"an `if` of type {show(e[1])} {span(e)}")
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
            if not self.held(e[1]):
                self.bad(f"a block of type {show(e[1])} {span(e)}")
            self.expr(kids(e)[0])
            return
        if k == "match":
            self.match(e)
            return
        if k == "let":
            if not self.held(e[4]):
                self.bad(f"a binding of type {show(e[4])} {span(kids(e)[0])}")
            self.vars[int(e[1])] = e[4]
            self.expr(kids(e)[0])
            return
        if k == "lettuple":
            ks = kids(e)
            for vn in kids(ks[0]):
                self.vars[int(vn[1])] = vn[4]
                if not self.held(vn[4]):
                    self.bad(f"a binding of type {show(vn[4])}")
            self.expr(ks[1])
            return
        if k == "assign":
            self.assign(e)
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
            self.for_(e)
            return
        if k == "list":
            if not self.held(e[1]):
                self.bad(f"a list of {show(e[1])} {span(e)}")
            for x in kids(e):
                self.expr(x)
            return
        if k == "tuple":
            if not self.held(e[1]):
                self.bad(f"a tuple of {show(e[1])} {span(e)}")
            for x in kids(e):
                self.expr(x)
            return
        if k == "index":
            if not self.held(e[1]):
                self.bad(f"an element of type {show(e[1])} {span(e)}")
            for x in kids(e):
                self.expr(x)
            return
        if k == "tuplefield":
            if not self.held(e[1]):
                self.bad(f"a tuple element of type {show(e[1])} {span(e)}")
            self.expr(kids(e)[0])
            return
        if k == "field":
            recv = kids(e)[0]
            if not self.held(e[1]):
                self.bad(f"a field of type {show(e[1])} {span(e)}")
            elif head(recv[1]) != "named":
                self.bad(f"a field of {show(recv[1])} {span(e)}")
            self.expr(recv)
            return
        if k == "struct":
            self.struct(e)
            return
        if k == "variant":
            self.variant(e)
            return
        if k == "maplit":
            if not self.held(e[1]):
                self.bad(f"a map of {show(e[1])} {span(e)}")
            for en in kids(e):
                for x in kids(en):
                    self.expr(x)
            return
        if k == "try":
            if not self.held(e[1]):
                self.bad(f"`try` answering {show(e[1])} {span(e)}")
            self.expr(kids(e)[0])
            return
        if k == "wrapok":
            if not (head(e[1]) == "fallible" and self.held(e[1][1])):
                self.bad(f"`Ok` of type {show(e[1])} {span(e)}")
            self.expr(kids(e)[0])
            return
        if k == "closure":
            self.closure(e)
            return
        if k == "fnref":
            name = unq(e[3])
            f = self.fns.get(name)
            if f is None:
                self.bad(f"`{name}` as a value, which is not a function")
            elif len(e[4]) > 1:
                self.bad(f"a generic function `{name}` as a value {span(e)}")
            elif not self.held(e[1]):
                self.bad(f"a function value of type {show(e[1])} {span(e)}")
            elif any(p[1] == "inout" for p in kids(f[7])):
                self.bad(f"`{name}`, which has `inout` parameters, as a value {span(e)}")
            return
        if k == "callvalue":
            ks = kids(e)
            ft = ks[0][1]
            if head(ft) != "fn" or not self.held(ft):
                self.bad(f"calling a value of type {show(ft)} {span(e)}")
            for x in ks:
                self.expr(x)
            return
        self.bad(f"`{k}` {span(e)}")

    def closure(self, e):
        """A closure: of a held function type, every variable it captures
        held, its parameters bound for its body."""
        if not self.held(e[1]):
            self.bad(f"a closure of type {show(e[1])} {span(e)}")
            return
        bound, reads = set(), {}

        def walk(x):
            if isinstance(x, list) and x:
                if x[0] in ("var", "let") and len(x) > 1:
                    bound.add(int(x[1]))
                elif x[0] == "pbind" and len(x) > 2:
                    bound.add(int(x[2]))
                elif x[0] == "local" and len(x) >= 4:
                    reads.setdefault(int(x[3]), x[1])
                for y in x:
                    walk(y)

        walk(e)
        for vid, t in sorted(reads.items()):
            if vid not in bound and not self.held(t):
                self.bad(f"a closure capturing a value of type {show(t)} {span(e)}")
        ks = kids(e)
        for p in kids(ks[0]):
            self.vars[int(p[1])] = p[4]
            if not self.held(p[4]):
                self.bad(f"a closure parameter of type {show(p[4])}")
        self.expr(ks[1])

    def pieces(self, s):
        """A `str` node's interpolated values: held, and not `never`."""
        for pc in kids(s):
            if pc[0] == "text":
                continue
            x = pc[1]
            t = x[1] if isinstance(x, list) and len(x) > 1 else None
            if not self.held(t) or t == "never":
                self.bad(f"interpolating a value of type {show(t)}")
            self.expr(x)

    def binary(self, e):
        op, ot = e[3], e[4]
        ks = kids(e)
        if op in ("and", "or"):
            pass
        elif op == "coalesce":
            opt = head(ot) == "named" and unq(ot[1]) == "Option"
            if not self.held(e[1]) or not (opt or head(ot) == "fallible"):
                self.bad(f"`??` on {show(ot)} {span(e)}")
        elif self.boxed(ot):
            if op == "add" and (ot == "str" or head(ot) == "list"):
                pass
            elif op in ("eq", "ne"):
                pass
            elif op in ("lt", "le", "gt", "ge") and (ot == "str" or head(ot) == "tuple"):
                pass
            else:
                self.bad(f"`{op}` on {show(ot)} {span(e)}")
        elif op in ("eq", "ne", "lt", "le", "gt", "ge"):
            if atom(ot) not in INTS and ot != "bool" and ot != "f64":
                self.bad(f"`{op}` on {show(ot)} {span(e)}")
        elif op == "pow":
            if ot != "i64" and ot != "f64":
                self.bad(f"`^` on {show(ot)} {span(e)}")
        elif op in ("add", "sub", "mul", "div", "rem"):
            if atom(ot) not in INTS and ot != "f64":
                self.bad(f"`{op}` on {show(ot)} {span(e)}")
        else:
            self.bad(f"operator `{op}` {span(e)}")
        for x in ks:
            self.expr(x)

    def call(self, e):
        name = unq(e[3])
        if len(e[4]) > 1:
            self.bad(f"a generic call of `{name}`")
        if not self.held(e[1]):
            self.bad(f"a call of `{name}` answering {show(e[1])}")
        f = self.fns.get(name)
        if f is None:
            self.bad(f"a call of `{name}`, which is not a function")
            return
        params = kids(f[7])
        for i, a in enumerate(kids(e)):
            if i < len(params) and params[i][1] == "inout":
                # A local, or a place inside one: struct fields and list
                # elements of held types, read before the call and written
                # back after it.
                if not self.place_receiver(a):
                    self.bad(f"an `inout` argument that is not a place {span(e)}")
                elif a[0] != "local":
                    self.expr(a)
            else:
                self.expr(a)

    def builtin(self, e):
        name = unq(e[3])
        args = kids(e)
        if name in ("print", "eprint") and len(args) == 1:
            a = args[0]
            if a[0] == "str":
                self.pieces(a)
            else:
                if not self.held(a[1]) or a[1] == "never":
                    self.bad("printing a value of a type this backend does not hold")
                self.expr(a)
            return
        if name == "assert" and len(args) == 1:
            if args[0][1] != "bool":
                self.bad("`assert` of a non-bool")
            self.expr(args[0])
            return
        if name == "exit" and len(args) == 1:
            if atom(args[0][1]) not in INTS:
                self.bad("`exit` of a non-integer")
            self.expr(args[0])
            return
        if name == "panic" and len(args) == 1:
            a = args[0]
            if a[0] == "str" and len(kids(a)) == 1 and kids(a)[0][0] == "text":
                return
            if a[1] != "str":
                self.bad(f"a panic message of type {show(a[1])}")
                return
            self.expr(a)
            return
        if name == "fail" and len(args) == 1:
            if not self.boxed(args[0][1]):
                self.bad(f"failing with a value of type {show(args[0][1])}")
                return
            self.expr(args[0])
            return
        if name in ("pi", "e") and not args and e[1] == "f64":
            return
        if RT_BUILTINS.get(name) == len(args):
            if not self.held(e[1]):
                self.bad(f"built-in `{name}` answering {show(e[1])} {span(e)}")
                return
            for a in args:
                if not self.held(a[1]):
                    self.bad(f"built-in `{name}` of {show(a[1])} {span(e)}")
                    return
            for a in args:
                self.expr(a)
            return
        self.bad(f"built-in `{name}` {span(e)}")

    def method(self, e):
        name = unq(e[3])
        ks = kids(e)
        recv = ks[0]
        sig = f"{family(recv[1])}.{name}/{len(ks) - 1}"
        if sig not in METHODS or not self.held(e[1]):
            self.bad(f"method `{name}` on {show(recv[1])} {span(e)}")
            return
        if sig in MUTATING and not self.place_receiver(recv):
            self.bad(f"`{name}` on a receiver that is not a place {span(e)}")
            return
        if family(recv[1]) == "range":
            if recv[0] != "range":
                self.bad(f"`{name}` on a range that is not written in place {span(e)}")
                return
            ks = kids(recv) + ks[1:]
        for x in ks:
            self.expr(x)

    def place_receiver(self, r):
        """A receiver a method may change: struct fields and list elements,
        each of a held type, down to a local."""
        while isinstance(r, list) and r and r[0] in ("field", "index"):
            inner = kids(r)[0]
            ct = inner[1]
            if r[0] == "field":
                b = self.body(ct)
                if b is None or b[0] != "struct":
                    return False
            elif head(ct) != "list":
                return False
            if not self.held(r[1]):
                return False
            r = inner
        return isinstance(r, list) and bool(r) and r[0] == "local"

    def match(self, e):
        if not self.held(e[1]):
            self.bad(f"a `match` of type {show(e[1])} {span(e)}")
        ks = kids(e)
        scrut = ks[0]
        if not self.held(scrut[1]):
            self.bad(f"matching on {show(scrut[1])} {span(e)}")
        self.expr(scrut)
        for arm in ks[1:]:
            parts = kids(arm)
            self.pattern(parts[0], scrut[1])
            if len(parts) == 3:
                self.expr(parts[1][1])
            self.expr(parts[-1])

    def pattern(self, p, st):
        k = p[0]
        if k == "pbind":
            self.vars[int(p[2])] = st
            return
        if k in ("pwild", "pint", "pbool", "pstr"):
            return
        if k == "por":
            for x in kids(p):
                self.pattern(x, st)
            return
        if k == "ptuple":
            for j, sub in enumerate(kids(p)):
                et = st[j + 1] if head(st) == "tuple" and j + 1 < len(st) else None
                self.pattern(sub, et)
            return
        if k == "pvariant":
            b = self.body(st)
            idx = int(p[3])
            if b is None or b[0] != "sum" or idx >= len(kids(b)):
                self.bad(f"a variant pattern on {show(st)} {span(p)}")
                return
            vd = kids(b)[idx]
            fields = kids(vd)
            args = st[2:]
            for j, sub in enumerate(kids(p)):
                if sub[0] == "pwild":
                    continue
                ft = copy_sub(fields[j][2], args)
                if not self.field_held(fields[j][2], args) or self.has_generic(ft):
                    self.bad(f"a variant field of type {show(ft)} {span(p)}")
                    continue
                self.pattern(sub, ft)
            return
        if k in ("pok", "perr"):
            sub = kids(p)[0]
            if sub[0] == "pwild":
                return
            half = st[1] if k == "pok" else st[2]
            if head(st) != "fallible" or not self.held(half):
                self.bad(f"an `{k}` pattern on {show(st)} {span(p)}")
                return
            self.pattern(sub, half)
            return
        self.bad(f"pattern `{k}`")

    def place_type(self, p, e):
        """The type of assignment place `p`, or None (with a reason)."""
        if p[0] == "local":
            return self.vars.get(int(p[1]))
        if p[0] == "field":
            ct = self.place_type(kids(p)[0], e)
            if ct is None:
                return None
            b = self.body(ct)
            i = int(p[1])
            if b is None or b[0] != "struct" or i >= len(kids(b)):
                self.bad(f"an assignment to a field of {show(ct)}")
                return None
            ft = copy_sub(kids(b)[i][2], ct[2:])
            if self.has_generic(ft) or not self.held(ft):
                self.bad(f"an assignment to a field of type {show(ft)}")
                return None
            return ft
        if p[0] == "index":
            pk = kids(p)
            ct = self.place_type(pk[0], e)
            self.expr(pk[1])
            if ct is None:
                return None
            if head(ct) != "list" or not self.held(ct[1]):
                self.bad(f"an assignment to an element of {show(ct)}")
                return None
            return ct[1]
        self.bad(f"assignment through a `{p[0]}`")
        return None

    def assign(self, e):
        ks = kids(e)
        target = ks[0]
        if target[0] != "local":
            self.place_type(target, e)
        if not self.held(e[2]):
            self.bad(f"assignment of type {show(e[2])}")
            return
        op = e[1]
        if op != "set" and atom(e[2]) not in INTS and e[2] != "f64" and not (op == "add" and e[2] == "str"):
            self.bad(f"`{op}=` on {show(e[2])}")
        self.expr(ks[1])

    def for_(self, e):
        ks = kids(e)
        var = ks[0]
        self.vars[int(var[1])] = var[4]
        if e[1] == "list":
            if not self.held(var[4]):
                self.bad(f"`for` over elements of type {show(var[4])}")
                return
            self.expr(ks[1])
            self.expr(ks[2])
            return
        if e[1] != "range":
            self.bad(f"`for` over a {e[1]}")
            return
        it = ks[1]
        if it[0] != "range":
            self.bad("`for` over a range that is not written in place")
            return
        for x in kids(it):
            self.expr(x)
        self.expr(ks[2])

    def struct(self, e):
        t = e[1]
        b = self.body(t)
        if b is None or b[0] != "struct":
            self.bad(f"a literal of {show(t)} {span(e)}")
            return
        for f in kids(b):
            if not self.field_held(f[2], t[2:]):
                self.bad(f"a struct with a field of type {show(f[2])}")
        for f in kids(e):
            if f[0] != "field":
                self.bad(f"a struct literal with `..` {span(e)}")
                continue
            self.expr(f[2])

    def variant(self, e):
        t = e[1]
        b = self.body(t)
        idx = int(e[4])
        if b is None or b[0] != "sum" or idx >= len(kids(b)):
            self.bad(f"a variant of {show(t)} {span(e)}")
            return
        fields = kids(kids(b)[idx])
        for j, x in enumerate(kids(e)):
            if not self.field_held(fields[j][2], t[2:]):
                self.bad(f"a variant field of type {show(fields[j][2])}")
            self.expr(x)


def family(t):
    """The receiver family a built-in method is looked up by."""
    if isinstance(t, str):
        if t in ("str", "bytes", "bool", "u64"):
            return t
        if t in INTS:
            return "int"
        return t
    h = head(t)
    if h == "named" and unq(t[1]) == "Option":
        return "option"
    return h


class Outside(Exception):
    """A program monomorphization finds outside the subset."""


def node(items, h):
    n = Node(items)
    n.head = h
    return n


def copy_sub(x, subst):
    """A copy of `x` with every `(generic n)` read as `subst[n]` (copied)."""
    if isinstance(x, list):
        if x and x[0] == "generic" and len(x) == 2:
            n = int(x[1])
            if 0 <= n < len(subst):
                return copy_sub(subst[n], [])
        return node([copy_sub(c, subst) for c in x], getattr(x, "head", None) if getattr(x, "head", None) is not None else len(x))
    return x


def has_generic(t):
    if not isinstance(t, list):
        return False
    if t and t[0] == "generic":
        return True
    return any(has_generic(x) for x in t[1:])


def impl_key(t):
    if isinstance(t, str):
        return t
    if head(t) == "named":
        return unq(t[1])
    if head(t) == "list":
        return "[]"
    return show(t)


def unify(p, c, binds):
    if isinstance(p, list) and p and p[0] == "generic" and len(p) == 2:
        n = int(p[1])
        if not 0 <= n < len(binds):
            return False
        if binds[n] is None:
            binds[n] = c
            return True
        return show(binds[n]) == show(c)
    if isinstance(p, list) != isinstance(c, list):
        return False
    if not isinstance(p, list):
        return p == c
    return len(p) == len(c) and all(unify(a, b, binds) for a, b in zip(p, c))


def monomorphize(tree):
    """Every generic function the program uses, instantiated at the type
    arguments it is used at, as a non-generic copy named `name[T1,T2]`
    (`compiler/mir/src/mono.gr`, which the backend runs): each use rewritten
    to name its instance, and each `boundcall` to a call of the function its
    receiver's `impl` provides -- an instance itself when that `impl` is
    generic. The subset is then decided on the instances."""
    fns, impls, start = {}, {}, []
    for d in list(tree):
        if not isinstance(d, list) or not d:
            continue
        if d[0] == "fn":
            fns[unq(d[1])] = d
            if len(d[2]) <= 1:
                start.append(d)
        elif d[0] == "const":
            start.append(d)
        elif d[0] == "impl":
            for meth in kids(d):
                if isinstance(meth, list) and meth and meth[0] == "method":
                    impls[(unq(d[1]), unq(d[2]), unq(meth[1]))] = unq(meth[2])
    made = {}
    work = []

    def instance(name, targs):
        iname = name + "[" + ",".join(show(a) for a in targs) + "]"
        if iname in made:
            return iname
        if len(made) >= 4000 or len(iname) > 600:
            raise Outside(f"polymorphic recursion in `{name}`")
        g = fns.get(name)
        if g is None:
            raise Outside(f"a call of `{name}`, which is not a function")
        c = copy_sub(g, targs)
        c[1] = json.dumps(iname)
        c[2] = node(["generics"], 1)
        c[3] = node(["bounds"], 1)
        tree.append(c)
        fns[iname] = c
        made[iname] = c
        work.append(c)
        return iname

    def rewrite(r):
        stack = [r]
        while stack:
            e = stack.pop()
            if not isinstance(e, list) or not e:
                continue
            if e[0] in ("call", "fnref") and len(e) > 4 and isinstance(e[4], list) and len(e[4]) > 1:
                e[3] = json.dumps(instance(unq(e[3]), list(e[4][1:])))
                e[4] = node(["targs"], 1)
            elif e[0] == "boundcall":
                iface, meth = unq(e[3]), unq(e[4])
                rt = kids(e)[0][1]
                if has_generic(rt):
                    raise Outside(f"a `boundcall` of `{meth}` on {show(rt)}")
                f = impls.get((iface, impl_key(rt), meth))
                if f is None or f not in fns:
                    raise Outside(f"no `impl {iface}` for {show(rt)} to call `{meth}` on")
                callee = f
                ng = len(fns[f][2]) - 1
                if ng > 0:
                    ps = kids(kids(fns[f])[0])
                    binds = [None] * ng
                    if not ps or not unify(ps[0][4], rt, binds) or any(b is None for b in binds):
                        raise Outside(f"`{f}` does not take {show(rt)}")
                    callee = instance(f, binds)
                e[0] = "call"
                e[3] = json.dumps(callee)
                e[4] = node(["targs"], 1)
            for x in reversed(e):
                if isinstance(x, list):
                    stack.append(x)

    for r in start:
        rewrite(r)
    while work:
        rewrite(work.pop(0))


def callees(e, acc):
    if isinstance(e, list):
        if e and e[0] in ("call", "fnref"):
            acc.add(("fn", unq(e[3])))
        if e and e[0] == "const" and len(e) > 3 and isinstance(e[2], str) and e[2].startswith("@"):
            acc.add(("const", unq(e[3])))
        for x in e:
            callees(x, acc)


def classify(tree):
    # Expression chains nest hundreds deep in the corpus, and this walk
    # recurses once per node.
    sys.setrecursionlimit(max(sys.getrecursionlimit(), 20000))
    try:
        monomorphize(tree)
    except Outside as why:
        return [str(why)]
    fns, consts, types, main = {}, {}, {}, None
    for d in tree:
        if not isinstance(d, list) or not d:
            continue
        if d[0] == "fn":
            fns[unq(d[1])] = d
        elif d[0] == "const":
            consts[unq(d[1])] = d
        elif d[0] == "type":
            types[unq(d[1])] = d
        elif d[0] == "main":
            main = unq(d[1])
    if main is None:
        return ["the program has no `main`"]
    if main not in fns:
        return [f"`main` names `{main}`, which is not a function"]
    ck = Checker(fns, consts, types)
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
        ck.vars = {}
        if kind == "const":
            if not ck.held(d[2]):
                ck.bad(f"constant `{name}` of type {show(d[2])}")
            ck.expr(d[3])
        else:
            # (fn "name" (generics ..) (bounds ..) RET ERR (row ..) (params ..) body)
            if len(d[2]) > 1:
                ck.bad(f"generic function `{name}`")
            if not ck.held(d[4]):
                ck.bad(f"`{name}` answers {show(d[4])}")
            params = kids(d[7])
            if d[5] != "none" and not ck.boxed(d[5]):
                ck.bad(f"`{name}` fails with {show(d[5])}")
            for p in params:
                ck.vars[int(p[2])] = p[4]
                if not ck.held(p[4]):
                    ck.bad(f"a parameter of type {show(p[4])} in `{name}`")
            if name == main:
                if params:
                    ck.bad("`main` takes parameters")
                if d[4] != "unit":
                    ck.bad(f"`main` answers {show(d[4])}")
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
