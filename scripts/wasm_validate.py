#!/usr/bin/env python3
"""A WebAssembly validator for the modules the Grasm backend writes.

docs/self-hosting.md §6.24, decision (11): every module `s3 --target wasm32`
emits is checked by V8 (`new WebAssembly.Module`, which validates it whole)
and by this, a second validator independent of V8 -- the core
specification's validation algorithm (its appendix, "Validation Algorithm":
a value stack and a control stack, each instruction's types popped and
pushed) for every instruction the backend emits, and the module's structure
around the bodies: the sections in order, each read to its end, every index
in range, the types of imports, functions, the table, memory, the tag,
globals, exports, elements and data -- and, for a module built `--exports`
(the playground's, docs/self-hosting.md §6.25), the start function, the data
count, a passive data segment and `memory.init`.

An instruction it does not know is an error, not a pass: the backend emits
only what is below, and a module that holds anything else is not one of its.

    wasm_validate.py <module.wasm> ...

Prints, per module, `ok <path> bodies=<n> instructions=<m>`, or
`INVALID <path>: <why>`; exits 1 if any module is invalid. The counts are
what the gates assert, so it cannot pass by checking nothing.
"""

import sys

I32, I64, F32, F64, FUNCREF, EXNREF = 0x7F, 0x7E, 0x7D, 0x7C, 0x70, 0x69
NUM = {I32: "i32", I64: "i64", F32: "f32", F64: "f64"}
UNKNOWN = None  # the type of a value popped from an unreachable stack


class Invalid(Exception):
    pass


class Reader:
    def __init__(self, b, pos=0, end=None):
        self.b, self.p = b, pos
        self.end = len(b) if end is None else end

    def byte(self):
        if self.p >= self.end:
            raise Invalid("unexpected end")
        v = self.b[self.p]
        self.p += 1
        return v

    def uleb(self, bits=32):
        r, s = 0, 0
        while True:
            x = self.byte()
            r |= (x & 0x7F) << s
            s += 7
            if not x & 0x80:
                break
            if s >= bits + 7:
                raise Invalid("integer too long")
        if r >= 1 << bits:
            raise Invalid("integer too large")
        return r

    def sleb(self, bits):
        r, s = 0, 0
        while True:
            x = self.byte()
            r |= (x & 0x7F) << s
            s += 7
            if not x & 0x80:
                break
            if s >= bits + 7:
                raise Invalid("integer too long")
        if x & 0x40:
            r -= 1 << s
        if not -(1 << (bits - 1)) <= r < (1 << (bits - 1)):
            raise Invalid("integer out of range")
        return r

    def take(self, n):
        if self.p + n > self.end:
            raise Invalid("unexpected end")
        v = self.b[self.p:self.p + n]
        self.p += n
        return v

    def name(self):
        n = self.uleb()
        v = self.take(n)
        try:
            return v.decode("utf-8")
        except UnicodeDecodeError:
            raise Invalid("a name that is not UTF-8")

    def valtype(self):
        t = self.byte()
        if t not in (I32, I64, F32, F64, FUNCREF, EXNREF):
            raise Invalid(f"unknown value type 0x{t:02x}")
        return t


# Each plain instruction: its operands' types popped, its result pushed.
def _ops():
    t = {}
    for o in (0x45,):
        t[o] = ([I32], [I32])
    for o in range(0x46, 0x50):
        t[o] = ([I32, I32], [I32])
    t[0x50] = ([I64], [I32])
    for o in range(0x51, 0x5B):
        t[o] = ([I64, I64], [I32])
    for o in range(0x61, 0x67):
        t[o] = ([F64, F64], [I32])
    for o in (0x67, 0x68, 0x69):
        t[o] = ([I32], [I32])
    for o in range(0x6A, 0x79):
        t[o] = ([I32, I32], [I32])
    for o in (0x79, 0x7A, 0x7B):
        t[o] = ([I64], [I64])
    for o in range(0x7C, 0x8B):
        t[o] = ([I64, I64], [I64])
    for o in range(0x99, 0xA0):
        t[o] = ([F64], [F64])
    for o in range(0xA0, 0xA7):
        t[o] = ([F64, F64], [F64])
    t[0xA7] = ([I64], [I32])
    t[0xAC] = ([I32], [I64])
    t[0xAD] = ([I32], [I64])
    t[0xB9] = ([I64], [F64])
    t[0xBA] = ([I64], [F64])
    t[0xBD] = ([F64], [I64])
    t[0xBF] = ([I64], [F64])
    for o in (0xC2, 0xC3, 0xC4):
        t[o] = ([I64], [I64])
    return t


PLAIN = _ops()
# Loads and stores: the type, the store's or the load's, the natural
# alignment's log2.
LOADS = {0x28: (I32, 2), 0x29: (I64, 3), 0x2B: (F64, 3), 0x30: (I64, 0), 0x31: (I64, 0),
         0x32: (I64, 1), 0x33: (I64, 1), 0x34: (I64, 2), 0x35: (I64, 2)}
STORES = {0x36: (I32, 2), 0x37: (I64, 3), 0x39: (F64, 3), 0x3C: (I64, 0), 0x3D: (I64, 1),
          0x3E: (I64, 2)}


class Module:
    def __init__(self):
        self.types = []          # (params, results)
        self.funcs = []          # type index per function, imports first
        self.nimported = 0
        self.tables = []
        self.mems = 0
        self.globals = []        # (type, mutable)
        self.tags = []           # type index
        self.body_count = 0
        self.inst_count = 0
        # The data count section's number, which `memory.init` needs.
        self.data_count = None


def blocktype(r, m):
    b = r.b[r.p]
    if b == 0x40:
        r.p += 1
        return [], []
    if b in (I32, I64, F32, F64):
        r.p += 1
        return [], [b]
    x = r.sleb(33)
    if x < 0 or x >= len(m.types):
        raise Invalid("a block type out of range")
    return list(m.types[x][0]), list(m.types[x][1])


def check_body(m, r, end, params, results, nlocals_out):
    locals_ = list(params)
    n = r.uleb()
    total = len(params)
    for _ in range(n):
        c = r.uleb()
        t = r.valtype()
        total += c
        if total > 50000:
            raise Invalid("more than 50,000 locals")
        locals_.extend([t] * c)
    nlocals_out.append(total)
    vals = []
    # A control frame: kind, its label's types, its end's types, the value
    # stack's height at its start, whether the rest is unreachable.
    ctrls = [["func", list(results), list(results), 0, False]]

    def push(t):
        vals.append(t)

    def pop(expect=UNKNOWN):
        top = ctrls[-1]
        if len(vals) == top[3]:
            if top[4]:
                return expect
            raise Invalid("a value popped from an empty stack")
        v = vals.pop()
        if expect is not UNKNOWN and v is not UNKNOWN and v != expect:
            raise Invalid(f"a {NUM.get(v, hex(v or 0))} where a {NUM.get(expect, hex(expect))} was wanted")
        return v

    def pops(ts):
        for t in reversed(ts):
            pop(t)

    def unreachable():
        del vals[ctrls[-1][3]:]
        ctrls[-1][4] = True

    def label(d):
        if d >= len(ctrls):
            raise Invalid("a branch to a label that is not there")
        f = ctrls[-1 - d]
        return f[1]

    while True:
        if r.p >= end:
            raise Invalid("a body that does not end")
        o = r.byte()
        m.inst_count += 1
        if o == 0x0B:  # end
            f = ctrls[-1]
            pops(f[2])
            if len(vals) != f[3]:
                raise Invalid(f"{len(vals) - f[3]} value(s) left at the end of a {f[0]}")
            ctrls.pop()
            if f[0] == "if" and f[5] != f[2]:
                raise Invalid("an if without else whose results are not its parameters")
            if not ctrls:
                if r.p != end:
                    raise Invalid("bytes after the body's end")
                return
            for t in f[2]:
                push(t)
            continue
        if o in PLAIN:
            a, b = PLAIN[o]
            pops(a)
            for t in b:
                push(t)
        elif o == 0x00:
            unreachable()
        elif o in (0x02, 0x03):
            ps, rs = blocktype(r, m)
            pops(ps)
            ctrls.append(["block" if o == 0x02 else "loop", rs if o == 0x02 else ps, rs, len(vals), False])
            for t in ps:
                push(t)
        elif o == 0x04:
            ps, rs = blocktype(r, m)
            pop(I32)
            pops(ps)
            ctrls.append(["if", rs, rs, len(vals), False])
            ctrls[-1].append(ps)
            for t in ps:
                push(t)
        elif o == 0x05:
            f = ctrls[-1]
            if f[0] != "if":
                raise Invalid("else outside an if")
            pops(f[2])
            if len(vals) != f[3]:
                raise Invalid("values left at an else")
            f[0] = "else"
            f[4] = False
            for t in f[5]:
                push(t)
        elif o == 0x0C:
            d = r.uleb()
            pops(label(d))
            unreachable()
        elif o == 0x0D:
            d = r.uleb()
            pop(I32)
            ts = label(d)
            pops(ts)
            for t in ts:
                push(t)
        elif o == 0x0F:
            pops(results)
            unreachable()
        elif o == 0x10:
            x = r.uleb()
            if x >= len(m.funcs):
                raise Invalid(f"a call of function {x}, which is not there")
            ps, rs = m.types[m.funcs[x]]
            pops(ps)
            for t in rs:
                push(t)
        elif o == 0x11:
            x = r.uleb()
            tbl = r.uleb()
            if x >= len(m.types) or tbl >= len(m.tables):
                raise Invalid("call_indirect out of range")
            pop(I32)
            ps, rs = m.types[x]
            pops(ps)
            for t in rs:
                push(t)
        elif o == 0x1A:
            pop()
        elif o in (0x20, 0x21, 0x22):
            x = r.uleb()
            if x >= len(locals_):
                raise Invalid(f"local {x} of {len(locals_)}")
            if o == 0x20:
                push(locals_[x])
            else:
                pop(locals_[x])
                if o == 0x22:
                    push(locals_[x])
        elif o in (0x23, 0x24):
            x = r.uleb()
            if x >= len(m.globals):
                raise Invalid("a global that is not there")
            t, mut = m.globals[x]
            if o == 0x23:
                push(t)
            else:
                if not mut:
                    raise Invalid("a write to an immutable global")
                pop(t)
        elif o in LOADS or o in STORES:
            t, natural = LOADS.get(o) or STORES.get(o)
            align = r.uleb()
            r.uleb()
            if m.mems == 0:
                raise Invalid("a memory access with no memory")
            if align > natural:
                raise Invalid("an alignment past the natural one")
            if o in LOADS:
                pop(I32)
                push(t)
            else:
                pop(t)
                pop(I32)
        elif o in (0x3F, 0x40):
            if r.byte() != 0:
                raise Invalid("memory index not 0")
            if o == 0x40:
                pop(I32)
            push(I32)
        elif o == 0x41:
            r.sleb(32)
            push(I32)
        elif o == 0x42:
            r.sleb(64)
            push(I64)
        elif o == 0x44:
            r.take(8)
            push(F64)
        elif o == 0xFC:
            sub = r.uleb()
            if sub in (6, 7):
                pop(F64)
                push(I64)
            elif sub == 10:
                if r.byte() != 0 or r.byte() != 0:
                    raise Invalid("memory.copy's memories not 0")
                pop(I32)
                pop(I32)
                pop(I32)
            elif sub == 11:
                if r.byte() != 0:
                    raise Invalid("memory.fill's memory not 0")
                pop(I32)
                pop(I32)
                pop(I32)
            elif sub == 8:
                x = r.uleb()
                if m.data_count is None:
                    raise Invalid("memory.init with no data count section")
                if x >= m.data_count:
                    raise Invalid("memory.init of a data segment that is not there")
                if r.byte() != 0:
                    raise Invalid("memory.init's memory not 0")
                pop(I32)
                pop(I32)
                pop(I32)
            else:
                raise Invalid(f"an instruction 0xfc {sub} the backend does not emit")
        elif o == 0x08:
            x = r.uleb()
            if x >= len(m.tags):
                raise Invalid("a throw of a tag that is not there")
            ps, rs = m.types[m.tags[x]]
            pops(ps)
            unreachable()
        elif o == 0x1F:
            ps, rs = blocktype(r, m)
            n = r.uleb()
            for _ in range(n):
                kind = r.byte()
                if kind in (0, 1):
                    x = r.uleb()
                    if x >= len(m.tags):
                        raise Invalid("a catch of a tag that is not there")
                    tp = list(m.types[m.tags[x]][0]) + ([EXNREF] if kind == 1 else [])
                elif kind in (2, 3):
                    tp = [EXNREF] if kind == 3 else []
                else:
                    raise Invalid("an unknown catch clause")
                d = r.uleb()
                if d >= len(ctrls):
                    raise Invalid("a catch to a label that is not there")
                if label(d) != tp:
                    raise Invalid("a catch's values do not match its label")
            pops(ps)
            ctrls.append(["try_table", rs, rs, len(vals), False])
            for t in ps:
                push(t)
        else:
            raise Invalid(f"an instruction 0x{o:02x} the backend does not emit")


def const_expr(r, want, m):
    o = r.byte()
    if o == 0x41 and want == I32:
        r.sleb(32)
    elif o == 0x42 and want == I64:
        r.sleb(64)
    elif o == 0x44 and want == F64:
        r.take(8)
    else:
        raise Invalid("a constant expression the backend does not write")
    if r.byte() != 0x0B:
        raise Invalid("a constant expression that does not end")


def validate(b):
    if b[:8] != b"\x00asm\x01\x00\x00\x00":
        raise Invalid("not a wasm module of version 1")
    m = Module()
    r = Reader(b, 8)
    order = [1, 2, 3, 4, 5, 13, 6, 7, 8, 9, 12, 10, 11]
    last = -1
    nfuncs_declared = None
    data_count = None
    seen_code = False
    nlocals = []
    while r.p < len(b):
        sid = r.byte()
        size = r.uleb()
        end = r.p + size
        if end > len(b):
            raise Invalid("a section past the module's end")
        s = Reader(b, r.p, end)
        if sid != 0:
            if sid not in order:
                raise Invalid(f"an unknown section {sid}")
            k = order.index(sid)
            if k <= last:
                raise Invalid(f"section {sid} out of order")
            last = k
        if sid == 0:
            s.name()
            s.p = end
        elif sid == 1:
            for _ in range(s.uleb()):
                if s.byte() != 0x60:
                    raise Invalid("a type that is not a function's")
                ps = [s.valtype() for _ in range(s.uleb())]
                rs = [s.valtype() for _ in range(s.uleb())]
                m.types.append((ps, rs))
        elif sid == 2:
            for _ in range(s.uleb()):
                s.name()
                s.name()
                kind = s.byte()
                if kind != 0:
                    raise Invalid("an import that is not a function")
                x = s.uleb()
                if x >= len(m.types):
                    raise Invalid("an import of a type that is not there")
                m.funcs.append(x)
                m.nimported += 1
        elif sid == 3:
            nfuncs_declared = s.uleb()
            for _ in range(nfuncs_declared):
                x = s.uleb()
                if x >= len(m.types):
                    raise Invalid("a function of a type that is not there")
                m.funcs.append(x)
        elif sid == 4:
            for _ in range(s.uleb()):
                if s.byte() != FUNCREF:
                    raise Invalid("a table not of functions")
                flags = s.byte()
                lo = s.uleb()
                hi = s.uleb() if flags & 1 else None
                if hi is not None and hi < lo:
                    raise Invalid("a table's maximum below its minimum")
                m.tables.append(lo)
        elif sid == 5:
            n = s.uleb()
            if n != 1:
                raise Invalid("not one memory")
            flags = s.byte()
            if flags not in (0, 1):
                raise Invalid("a memory that is shared or 64-bit")
            lo = s.uleb()
            if flags & 1:
                if s.uleb() < lo:
                    raise Invalid("a memory's maximum below its minimum")
            if lo > 65536:
                raise Invalid("a memory past 4 GiB")
            m.mems = 1
        elif sid == 13:
            for _ in range(s.uleb()):
                if s.byte() != 0:
                    raise Invalid("a tag that is not an exception")
                x = s.uleb()
                if x >= len(m.types) or m.types[x][1]:
                    raise Invalid("a tag of a type that answers something")
                m.tags.append(x)
        elif sid == 6:
            for _ in range(s.uleb()):
                t = s.valtype()
                mut = s.byte()
                if mut not in (0, 1):
                    raise Invalid("a global's mutability not 0 or 1")
                const_expr(s, t, m)
                m.globals.append((t, mut == 1))
        elif sid == 7:
            names = set()
            for _ in range(s.uleb()):
                nm = s.name()
                if nm in names:
                    raise Invalid(f"export `{nm}` twice")
                names.add(nm)
                kind = s.byte()
                x = s.uleb()
                limit = {0: len(m.funcs), 1: len(m.tables), 2: m.mems, 3: len(m.globals), 4: len(m.tags)}.get(kind)
                if limit is None or x >= limit:
                    raise Invalid(f"export `{nm}` of something that is not there")
        elif sid == 9:
            for _ in range(s.uleb()):
                flags = s.uleb()
                if flags != 0:
                    raise Invalid("an element segment the backend does not write")
                if not m.tables:
                    raise Invalid("elements with no table")
                const_expr(s, I32, m)
                for _ in range(s.uleb()):
                    if s.uleb() >= len(m.funcs):
                        raise Invalid("an element of a function that is not there")
        elif sid == 8:
            x = s.uleb()
            if x >= len(m.funcs):
                raise Invalid("a start function that is not there")
            if m.types[m.funcs[x]] != ([], []):
                raise Invalid("a start function that takes or answers something")
        elif sid == 12:
            data_count = s.uleb()
            m.data_count = data_count
        elif sid == 10:
            seen_code = True
            n = s.uleb()
            if n != (nfuncs_declared or 0):
                raise Invalid(f"{n} bodies for {nfuncs_declared} functions")
            for k in range(n):
                size_b = s.uleb()
                body_end = s.p + size_b
                if body_end > end:
                    raise Invalid("a body past its section's end")
                ps, rs = m.types[m.funcs[m.nimported + k]]
                try:
                    check_body(m, Reader(b, s.p, body_end), body_end, ps, rs, nlocals)
                except Invalid as e:
                    raise Invalid(f"function {m.nimported + k}: {e}")
                m.body_count += 1
                s.p = body_end
        elif sid == 11:
            n = s.uleb()
            if data_count is not None and n != data_count:
                raise Invalid("data segments not as counted")
            for _ in range(n):
                flags = s.uleb()
                if flags not in (0, 1) or not m.mems:
                    raise Invalid("a data segment the backend does not write")
                if flags == 0:
                    const_expr(s, I32, m)
                s.take(s.uleb())
        if s.p != end:
            raise Invalid(f"section {sid} not read to its end")
        r.p = end
    if nfuncs_declared and not seen_code:
        raise Invalid("functions with no code")
    return m


def main():
    bad = 0
    for path in sys.argv[1:]:
        with open(path, "rb") as f:
            b = f.read()
        try:
            m = validate(b)
            print(f"ok {path} bodies={m.body_count} instructions={m.inst_count}")
        except Invalid as e:
            bad += 1
            print(f"INVALID {path}: {e}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
