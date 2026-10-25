#!/usr/bin/env python3
"""Whether a program is in the subset the S3 backend compiles, asked of its TIR.

    s3_subset.py <program.tir>      prints `in`, or `out: <every reason>`

The rule, stated once and here (docs/self-hosting.md, S3; the backend's own
refusals in `compiler/mir/src/lower.gr` are held to it by
`scripts/s3subset.sh`, in both directions):

A program is in the subset when its TIR declares a `main`, and every function
and module-level constant that `main` reaches -- through `call` nodes,
functions used as values (`fnref`), functions whose address is taken
(`fnname`) and constant reads, transitively -- satisfies all of the
following, after monomorphization. A program that links a system library
(`grasm links`, which TIR does not record) is set aside by the gates before
this rule is asked (`scripts/s3_collect.py`).

Monomorphization. Every generic function the program uses is instantiated at
each list of type arguments it is used at, as a copy with its type
parameters replaced (`monomorphize`, the backend's `compiler/mir/src/mono.gr`
in Python); every use names its instance, and every `boundcall` -- an
interface method on a receiver of a generic type -- becomes a call of what
the receiver's `impl` provides, itself instantiated when that `impl` is
generic. What follows is asked of the instances. A generic function asking
for ever bigger instances of itself (polymorphic recursion) is outside.

Types. The *held* types are the scalars -- i64, i8, i16, i32, u8, u16, u32,
u64, f64, bool, unit (also written as the empty tuple), never, `addr` (a
word, [T-ADDR]), a channel (`Chan[T]`, a handle), and a reference to a held
type (`ref T`, one word naming a slot of the shared heap) -- and the boxed
types built from held types: str, bytes, the two builders, a list, a tuple,
a map, a named struct or sum type with held type arguments, a result
`T ! E` (E may be `never`), a union of boxed types, a function type whose
parameters and result are held (a function value), and a range held as a
value (the runtime's boxed `Range`). An f64 lives in a floating-point
register and travels in d0..d7 (AAPCS64).

  1. Declarations. A function has held parameter types (any mode; past the
     eighth argument of each register class they travel on the stack) and a
     held return type. One that can fail fails with a boxed type. A constant
     has a held type. `main` takes no parameters and answers `unit`. A C
     function (`extern`) takes integers, `f64`, `addr`, `str` or `bytes`, at
     most eight of each register class, and answers an integer, `f64`,
     `addr` or nothing (`extern_ok`).

  2. Nodes. Its body holds only:
       int, bool, unit                     literals
       float                               a literal, which TIR writes as bits
       local, const                        of a held type
       str                                 a literal, or one interpolating
                                           held values other than `never`
       list, tuple                         of held element types
       index, tuplefield                   answering a held type
       struct                              of a struct type whose fields are
                                           all held; a `..base` first, then
                                           the fields as written
       field                               answering a held type, of a named
                                           type or of one a reference points
                                           at
       deref                               `r.value`, of a held type
       variant                             of a sum type, every field held
       binary add sub mul div rem          on an integer type or f64
       binary pow                          on i64 or f64
       binary eq ne lt le gt ge            on an integer type, f64, bool or
                                           addr
       binary eq ne                        on a reference (identity), or unit
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
                                           parameter is a `local`, or struct
                                           fields and list elements rooted at
                                           one
       callextern                          of a C function as declaration 1 says
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
       builtin ref, release, alive         of a reference to a held type
       builtin net_workers                 of a held function of one argument
       builtin mem_alloc, mem_free,        raw memory; `mem_size_of` and
         mem_addr, mem_size_of,            `mem_align_of` of a layout type,
         mem_align_of, mem_tls,            `mem_tls` of a cell, `mem_fn_addr`
         mem_fn_addr                       of a C function or of a Grasm one
                                           with a C signature (`c_compatible`),
                                           each a name and not evaluated
       maplit                              of a held map type
       try                                 answering a held type
       wrapok                              of a held type
       method                              listed in METHODS below, on its
                                           receiver type; `call_c` on an addr,
                                           with a C signature
       if, block, blockexpr, match         of a held type
       region, unsafe                      blocks, of a held type
       match                               on a held type -- on what a
                                           reference points at, for a
                                           reference -- with patterns pwild
                                           pint pbool pstr pbind por ptuple,
                                           pvariant (each field read held and
                                           not generic), pok and perr (of a
                                           held half), and guards
       let                                 of a held type
       lettuple                            every name of a held type
       assign set                          of a held type, to a local or to a
                                           path of struct fields (each held,
                                           not generic), list elements and
                                           references (`r.value`, or a field
                                           or element of what one points at)
                                           rooted at a local
       assign add|sub|mul|div|rem          of an integer type or f64, to such a
                                           place
       assign add                          of str, to such a place
       return, break, continue, while
       for range                           over a range written in place, or
                                           held as a value
       for list                            over elements of a held type
       range                               a value, of `i64` bounds
       closure                             of a held function type, every
                                           variable it captures (reads and
                                           does not bind) of a held type; one
                                           that cannot fail itself but whose
                                           `fail` or `try` fails the function
                                           around it only as the argument of
                                           a method in INLINING, with no
                                           `return`
       fnref                               of a function with no `inout`
                                           parameter, or of a C function, as a
                                           held type
       callvalue                           of a held function type
       with                                of an effect whose every operation
                                           the handler, a held struct, has a
                                           function value for
       effectop                            answering a held type, of held
                                           arguments
       parallel, spawn                     a `spawn` directly in a `parallel`
                                           block of its own kind; a `go`
                                           (`spawn concurrent`) capturing held
                                           values

A method listed in MUTATING changes its receiver, which must then be a
place: a local, or struct fields, list elements and references of held types
rooted at one. A method on a range (`map`, `filter`, `to_list`, `len`, `sum`)
is on a range written in place or held as a value. An atomic method's last
argument is its ordering, a name.

What is left outside: a closure whose failure leaves the function around it
anywhere but there, an `inout` argument through a reference, and what the
checker refuses.

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
        # `Some(v)` when it fits ([L-LIB-INT-CONV]).
        METHODS.add(f"{_fam}.try_{_w}/0")
    # The bits at the receiver's width ([L-LIB-BITS-WIDTH]) and the checked
    # and saturating arithmetic, at its width by the runtime.
    for _m in ("count_ones/0", "leading_zeros/0", "trailing_zeros/0", "swap_bytes/0",
               "rotate_left/1", "rotate_right/1", "checked_add/1", "checked_sub/1",
               "checked_mul/1", "saturating_add/1", "saturating_sub/1", "saturating_mul/1"):
        METHODS.add(f"{_fam}.{_m}")
METHODS |= {"int.from_bits/0", "str.parse_float/0", "str.cmp/1", "tuple.cmp/1"}
# The higher-order methods: a loop calling the function value per element.
# A range's are on its elements, built first when it is held as a value.
METHODS |= {"list.map/1", "list.filter/1", "list.all/1", "list.any/1", "list.find/1",
            "list.fold/2", "list.sort_by/1", "range.map/1", "range.filter/1"}
# The parallel maps: the runtime's, calling a worker on threads.
METHODS |= {"list.par_map/1", "list.par_filter/1"}
# Raw memory on an `addr` ([L-LIB-MEM], [L-LIB-ATOMIC]): arithmetic, loads and
# stores of every layout type, and the atomics of every integer one, whose
# last argument is an ordering -- a name, counted here as an argument.
LAYOUTS = sorted(INTS | {"f64", "addr"})
METHODS |= {"addr.offset/1", "addr.diff/1", "addr.as_i64/0"}
for _t in LAYOUTS:
    METHODS |= {f"addr.load_{_t}/0", f"addr.store_{_t}/1"}
for _t in sorted(INTS):
    METHODS |= {f"addr.atomic_load_{_t}/1", f"addr.atomic_store_{_t}/2",
                f"addr.atomic_add_{_t}/2", f"addr.atomic_cas_{_t}/3"}
# The builders ([L-LIB-BUILDER]).
METHODS |= {"strbuilder.len/0", "bytebuilder.len/0", "strbuilder.is_empty/0",
            "bytebuilder.is_empty/0", "strbuilder.to_str/0", "bytebuilder.to_str/0",
            "bytebuilder.to_bytes/0", "bytebuilder.at/1", "strbuilder.push_str/1",
            "bytebuilder.push_str/1", "bytebuilder.push_byte/1", "bytebuilder.push_bytes/1",
            "bytebuilder.set/2"}
# A range's elements and their count, by the runtime.
METHODS |= {"range.to_list/0", "range.len/0", "range.sum/0"}
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
    # The clocks, sleeping, entropy, processes and the network: each argument
    # boxed and lent, as the Cranelift build passes them.
    "now_millis": 0, "monotonic_millis": 0, "monotonic_nanos": 0, "sleep_millis": 1,
    "random_bytes": 1, "read_bytes_at": 3, "proc_run": 2, "proc_pipeline": 2,
    "tcp_listen": 1, "tcp_accept": 1, "tcp_read": 1, "tcp_read_bytes": 1, "net_port": 1,
    "udp_bind": 1, "udp_recv": 1, "udp_recv_from": 1, "tcp_peer": 1, "net_close": 1,
    "tcp_connect": 2, "tcp_listen_on": 2, "net_wait": 2, "tcp_write": 2,
    "tcp_write_bytes": 2, "udp_send": 4,
    # Channels: a handle each, a value sent handed over, one received in an
    # `Option`. `chan_new` takes a capacity or none.
    "chan_send": 2, "chan_recv": 1, "chan_select": 1, "chan_close": 1,
    # An empty builder of each kind.
    "str_builder": 0, "byte_builder": 0,
}

# The methods that change their receiver, which must then be a place: a local,
# or a path of struct fields and list elements rooted at one.
MUTATING = {"list.push/1", "list.sort/0", "list.pop/0", "list.remove/1", "bytes.set/2",
            "bytes.fill/1", "bytes.copy_from/2", "map.set/2", "map.delete/1", "list.sort_by/1",
            "strbuilder.push_str/1", "bytebuilder.push_str/1", "bytebuilder.push_byte/1",
            "bytebuilder.push_bytes/1", "bytebuilder.set/2"}


class Checker:
    def __init__(self, fns, consts, types, effects=None, externs=None):
        self.fns = fns
        self.consts = consts
        self.types = types
        self.effects = effects or {}
        self.externs = externs or {}
        # The `parallel` blocks around the node being checked, innermost
        # last: whether each is a `concurrent` one.
        self.par = []
        # The closures lowered into the loop of the method they are written
        # in (`INLINING`), by identity.
        self.inline_ok = set()
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
            if t in ("str", "bytes", "range", "strbuilder", "bytebuilder"):
                return "b"
            if t == "addr":
                # An address is a word ([T-ADDR]).
                return "s"
            return None
        h = head(t)
        if h == "ref":
            # A reference to a held value: one word naming its slot.
            return "s" if self.kind(t[1]) else None
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
        if h == "named" and unq(t[1]) == "Chan":
            # A channel: a handle, one word.
            return "s"
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
        if k in ("region", "unsafe"):
            # A block whose allocations die with it ([T-REGION]), or one
            # the checker let do what `unsafe` allows: compiled as blocks.
            if not self.held(e[1]):
                self.bad(f"a `{k}` block of type {show(e[1])} {span(e)}")
            self.expr(kids(e)[0])
            return
        if k == "callextern":
            name = unq(e[3])
            d = self.externs.get(name)
            if d is None or not extern_ok(d):
                self.bad(f"a call of C function `{name}` {span(e)}")
                return
            for x in kids(e):
                self.expr(x)
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
            # Of a named type, or of one a reference points at.
            recv = kids(e)[0]
            if not self.held(e[1]):
                self.bad(f"a field of type {show(e[1])} {span(e)}")
            elif head(deref_type(recv[1])) != "named" or not self.held(recv[1]):
                self.bad(f"a field of {show(recv[1])} {span(e)}")
            self.expr(recv)
            return
        if k == "deref":
            # `r.value`: what a reference points at, read through the heap.
            inner = kids(e)[0]
            if not self.held(e[1]) or head(inner[1]) != "ref":
                self.bad(f"a `deref` of type {show(e[1])} {span(e)}")
            self.expr(inner)
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
            if f is None and name in self.externs:
                # A C function as a value, through a wrapper ([T-EXTERN-VALUE]).
                if not extern_ok(self.externs[name]) or not self.held(e[1]):
                    self.bad(f"C function `{name}` as a value {span(e)}")
            elif f is None:
                self.bad(f"`{name}` as a value, which is not a function")
            elif len(e[4]) > 1:
                self.bad(f"a generic function `{name}` as a value {span(e)}")
            elif not self.held(e[1]):
                self.bad(f"a function value of type {show(e[1])} {span(e)}")
            elif any(p[1] == "inout" for p in kids(f[7])):
                self.bad(f"`{name}`, which has `inout` parameters, as a value {span(e)}")
            return
        if k == "range":
            ks = kids(e)
            if any(x[1] != "i64" for x in ks):
                self.bad(f"a range of non-`i64` bounds {span(e)}")
            for x in ks:
                self.expr(x)
            return
        if k == "with":
            self.with_(e)
            return
        if k == "parallel":
            # `parallel`: its tasks run in order after its block; a
            # `concurrent` one hands closures to the runtime's scheduler.
            self.par.append(e[1] == "concurrent")
            self.expr(kids(e)[0])
            self.par.pop()
            return
        if k == "spawn":
            conc = e[1] == "concurrent"
            if not self.par or self.par[-1] != conc:
                self.bad(f"a `spawn` not directly in its own kind of block {span(e)}")
                return
            saved = self.par
            if conc:
                # A `go` is a closure of no arguments: its captures are held.
                self.captures_held(e)
                self.par = []
            else:
                # A task's body runs after its block, in the block's frame.
                self.par = saved[:-1]
            self.expr(kids(e)[0])
            self.par = saved
            return
        if k == "effectop":
            if not self.held(e[1]):
                self.bad(f"an effect operation answering {show(e[1])} {span(e)}")
            for x in kids(e):
                if not self.held(x[1]):
                    self.bad(f"an effect operation's argument of type {show(x[1])} {span(e)}")
                self.expr(x)
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

    def with_(self, e):
        """`with E handled_by h { body }`: the handler a held struct with a
        function value for every operation of the effect, by name."""
        name = unq(e[1])
        ks = kids(e)
        h = ks[0]
        decl = self.effects.get(name)
        b = self.body(h[1])
        if decl is None or b is None or b[0] != "struct" or not self.held(h[1]):
            self.bad(f"a `with {name}` whose handler is {show(h[1])} {span(e)}")
            return
        fields = {unq(f[1]): f[2] for f in kids(b)}
        for op in kids(decl):
            ft = fields.get(unq(op[1]))
            if ft is None or self.kind(ft) != "b":
                self.bad(f"a handler with no function for `{name}.{unq(op[1])}` {span(e)}")
                return
        self.expr(h)
        self.expr(ks[1])

    def closure(self, e):
        """A closure: of a held function type, every variable it captures
        held, its parameters bound for its body."""
        if not self.held(e[1]):
            self.bad(f"a closure of type {show(e[1])} {span(e)}")
            return
        if e[3] == "total" and fails_out(kids(e)[1]) and id(e) not in self.inline_ok:
            # Its `fail` or `try` fails the function around it ([E-PAR-FAIL]),
            # and it is not where it is lowered into a loop.
            self.bad(f"a closure whose failure leaves the function around it {span(e)}")
            return
        self.captures_held(e)
        ks = kids(e)
        for p in kids(ks[0]):
            self.vars[int(p[1])] = p[4]
            if not self.held(p[4]):
                self.bad(f"a closure parameter of type {show(p[4])}")
        # Its body is a function of its own: no `spawn` in it belongs to a
        # block around the closure.
        saved = self.par
        self.par = []
        self.expr(ks[1])
        self.par = saved

    def captures_held(self, e):
        """Every variable closure (or `go` task) `e` captures -- reads and
        does not bind -- is of a held type."""
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
        elif op in ("eq", "ne") and head(ot) == "ref" and self.held(ot):
            # [T-REF-EQ]: identity, the same word.
            pass
        elif op in ("eq", "ne") and ot == "unit":
            # `()` has one value.
            pass
        elif op in ("eq", "ne", "lt", "le", "gt", "ge") and ot == "addr":
            # An address compares as the word it is.
            pass
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
        if name == "ref" and len(args) == 1 and head(e[1]) == "ref" and self.held(e[1]):
            # [T-REF-GENERATIONAL]: the value boxed into the shared heap.
            self.expr(args[0])
            return
        if name in ("release", "alive") and len(args) == 1 and head(args[0][1]) == "ref" and self.held(args[0][1]):
            self.expr(args[0])
            return
        if name in ("mem_size_of", "mem_align_of") and len(args) == 1:
            # A layout type, as a name.
            if args[0][0] != "type" or args[0][1] not in LAYOUTS:
                self.bad(f"`{name}` of {show(args[0])} {span(e)}")
            return
        if name == "mem_fn_addr" and len(args) == 1:
            # [T-FN-ADDR]: a Grasm function's C entry, or a C function.
            a = args[0]
            target = unq(a[1]) if a[0] == "fnname" else ""
            f = self.fns.get(target)
            if f is not None:
                if not c_compatible(f):
                    self.bad(f"the address of `{target}`, which has no C signature {span(e)}")
            elif target not in self.externs:
                self.bad(f"the address of `{target}` {span(e)}")
            return
        if name == "mem_tls" and len(args) == 1:
            # [T-TLS]: a cell, as a name.
            if args[0][0] != "tlsname":
                self.bad(f"`mem_tls` of {show(args[0])} {span(e)}")
            return
        if name in ("mem_alloc", "mem_free", "mem_addr") and len(args) == (2 if name == "mem_alloc" else 1):
            for a in args:
                self.expr(a)
            return
        if name == "net_workers" and len(args) == 2:
            # `f(0)` .. `f(n - 1)` on a thread each, by the runtime.
            ft = args[1][1]
            if not self.held(e[1]) or head(ft) != "fn" or not self.held(ft) or len(ft[1]) != 1:
                self.bad(f"`net_workers` of {show(ft)} {span(e)}")
                return
            for a in args:
                self.expr(a)
            return
        if RT_BUILTINS.get(name) == len(args) or (name == "chan_new" and len(args) <= 1):
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
        if family(recv[1]) == "addr" and name == "call_c":
            # [L-LIB-CALL-C]: through an address by the C convention; the
            # signature is a name, `(sig (P..) R)`, not evaluated.
            s = ks[1] if len(ks) > 1 else None
            if s is None or s[0] != "sig" or not extern_ok(["extern", '""', s[1], s[2]]) or len(s[1]) != len(ks) - 2:
                self.bad(f"a `call_c` whose signature is not a C one {span(e)}")
                return
            self.expr(recv)
            for x in ks[2:]:
                self.expr(x)
            return
        sig = f"{family(recv[1])}.{name}/{len(ks) - 1}"
        if sig not in METHODS or not self.held(e[1]):
            self.bad(f"method `{name}` on {show(recv[1])} {span(e)}")
            return
        if sig in MUTATING and not self.place_receiver(recv, through_refs=True):
            self.bad(f"`{name}` on a receiver that is not a place {span(e)}")
            return
        if family(recv[1]) == "addr" and name.startswith("atomic_"):
            # The ordering is a name, not evaluated.
            if not ks or ks[-1][0] != "order":
                self.bad(f"an atomic without its ordering {span(e)}")
                return
            ks = ks[:-1]
        if sig in INLINING:
            # A closure written here that cannot fail itself but fails the
            # function around it ([E-PAR-FAIL]) is lowered into the loop.
            a = ks[2 if name == "fold" else 1]
            if inlined_closure(a):
                self.inline_ok.add(id(a))
        for x in ks:
            self.expr(x)

    def place_receiver(self, r, through_refs=False):
        """A receiver a method may change: struct fields and list elements,
        each of a held type, down to a local -- and, for a mutating method
        (`through_refs`), through references: a field or element of what one
        points at, or the referent itself (`r.value`)."""
        while isinstance(r, list) and r and r[0] in ("field", "index", "deref"):
            inner = kids(r)[0]
            ct = inner[1]
            if r[0] == "deref":
                if not through_refs or head(ct) != "ref" or not self.held(ct):
                    return False
                r = inner
                continue
            if through_refs:
                ct = deref_type(ct)
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
            # A match on a reference matches what it points at
            # ([T-REF-MATCH]).
            self.pattern(parts[0], deref_type(scrut[1]))
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
        if p[0] == "deref":
            # `r.value = x`: into what the reference points at.
            ct = self.place_type(kids(p)[0], e)
            if ct is None:
                return None
            if head(ct) != "ref" or not self.held(ct):
                self.bad(f"an assignment through a `deref` of {show(ct)}")
                return None
            return ct[1]
        if p[0] == "field":
            # A field of what a reference points at is written through it.
            ct = self.place_type(kids(p)[0], e)
            if ct is None:
                return None
            ct = deref_type(ct)
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
            ct = deref_type(ct)
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
        self.expr(ks[1])
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
        # `T{ ..b, f: x }`: the base first, then the fields as written.
        for f in kids(e):
            if f[0] == "base":
                self.expr(kids(f)[0])
        for f in kids(e):
            if f[0] == "field":
                self.expr(f[2])
            elif f[0] != "base":
                self.bad(f"a struct literal with `{f[0]}` {span(e)}")

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


def c_compatible(f):
    """A Grasm function `(fn "name" (generics ..) (bounds ..) RET ERR (row ..)
    (params ..) body)` with a C signature: `let` parameters of layout types,
    a layout result or none, not generic, not fallible ([T-FN-ADDR])."""
    layout = INTS | {"f64", "addr"}
    if len(f[2]) > 1 or f[5] != "none":
        return False
    for p in kids(f[7]):
        if p[1] != "let" or not isinstance(p[4], str) or p[4] not in layout:
            return False
    return f[4] == "unit" or (isinstance(f[4], str) and f[4] in layout)


def extern_ok(d):
    """An `extern` declaration `(extern "m::f" (P..) R)` whose parameters are
    integers, `f64`, `addr`, `str` or `bytes` -- at most eight of each
    register class -- and whose answer is an integer, `f64`, `addr` or
    nothing."""
    ps = d[2] if isinstance(d[2], list) else []
    nint = nfp = 0
    for p in ps:
        if not isinstance(p, str) or not (p in INTS or p in ("f64", "addr", "str", "bytes")):
            return False
        if p == "f64":
            nfp += 1
        else:
            nint += 1
    r = d[3]
    return nint <= 8 and nfp <= 8 and isinstance(r, str) and (r in INTS or r in ("f64", "addr", "unit"))


# The higher-order methods that lower a closure written as their argument
# into their loop when it fails the function around it.
INLINING = {"list.map/1", "list.filter/1", "list.all/1", "list.any/1", "list.find/1",
            "list.fold/2", "range.map/1", "range.filter/1", "list.par_map/1", "list.par_filter/1"}


def has_return(body):
    """Whether `body` has a `return` not inside a closure of its own."""
    stack = [body]
    while stack:
        x = stack.pop()
        if not isinstance(x, list) or not x:
            continue
        if x[0] == "return":
            return True
        if x[0] == "closure":
            continue
        stack.extend(y for y in x if isinstance(y, list))
    return False


def inlined_closure(a):
    """A closure that cannot fail itself, whose `fail` or `try` fails the
    function around it, and that has no `return`."""
    return (isinstance(a, list) and a and a[0] == "closure" and a[3] == "total"
            and fails_out(kids(a)[1]) and not has_return(kids(a)[1]))


def fails_out(body):
    """Whether `body` has a `fail` or a `try` that is not inside a closure of
    its own that can fail."""
    stack = [body]
    while stack:
        x = stack.pop()
        if not isinstance(x, list) or not x:
            continue
        if x[0] == "try" or (x[0] == "builtin" and len(x) > 3 and x[3] == '"fail"'):
            return True
        if x[0] == "closure" and len(x) > 3 and x[3] == "fallible":
            continue
        stack.extend(y for y in x if isinstance(y, list))
    return False


def deref_type(t):
    """What a reference type points at, through as many references as it has;
    any other type as it is."""
    while head(t) == "ref" and len(t) > 1:
        t = t[1]
    return t


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
        if e and e[0] == "fnname" and len(e) > 1:
            # A function whose address is taken ([T-FN-ADDR]) is reached.
            acc.add(("fn", unq(e[1])))
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
    fns, consts, types, effects, externs, main = {}, {}, {}, {}, {}, None
    for d in tree:
        if not isinstance(d, list) or not d:
            continue
        if d[0] == "fn":
            fns[unq(d[1])] = d
        elif d[0] == "const":
            consts[unq(d[1])] = d
        elif d[0] == "type":
            types[unq(d[1])] = d
        elif d[0] == "effect":
            effects[unq(d[1])] = d
        elif d[0] == "extern":
            externs[unq(d[1])] = d
        elif d[0] == "main":
            main = unq(d[1])
    if main is None:
        return ["the program has no `main`"]
    if main not in fns:
        return [f"`main` names `{main}`, which is not a function"]
    ck = Checker(fns, consts, types, effects, externs)
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
