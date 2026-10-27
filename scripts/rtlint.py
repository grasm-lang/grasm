#!/usr/bin/env python3
"""rtlint: the runtime's lint ([T-RUNTIME], docs/self-hosting.md §5.6, §6.9).

A process cell is one per process and every thread sees it, so a plain load
or store of one is a data race unless the thread holds a lock that guards
it. This is the guard behind the owner's rule against unsound global mutable
state: Rust's `static X: AtomicU64`, never `static mut`. rtlint reads a
runtime package's TIR (`grasm tir`) and refuses, by function and line:

  plain     a non-atomic load or store (`load_T`, `store_T`) through an
            address derived from a process cell, outside a function marked
            as holding the cell's lock;
  handed    such an address handed to anything that may touch it
            non-atomically -- a C function, `call_c`, `ctx_switch`,
            `ctx_init`, `mem_free` -- outside such a function;
  escapes   such an address put where the lint cannot follow it: stored in
            memory, or returned to C by an `export fn`;
  caller    a call of a function marked as holding a lock from a function
            that neither holds that lock nor takes it (a compare-and-swap on
            the lock's own cell);
  marker    a mark that is not one rtlint reads, names no process cell of
            the module, or names a lock that is not an integer cell.

Marks are doc comments, so they sit in the source beside what they say:

    /// rtlint: guarded by POOL_LOCK
    process_local POOL_HEAD: addr

    /// rtlint: holds POOL_LOCK
    fn pool_pop() -> addr { ... }

Atomic access (`atomic_load_T`, `atomic_store_T`, `atomic_add_T`,
`atomic_cas_T`) is allowed everywhere. "Derived from a process cell" is
followed through every value computed from the address -- locals, `offset`,
`as_i64` and back through `mem_addr`, arithmetic, branches -- and across
calls in both directions, to a fixpoint over the whole program. Two kinds of
value are not the cell's address whatever they were computed from: one read
out of memory, and a `bool`.

    rtlint.py <entry.gr>...        lint each runtime package; exit 1 on any refusal
    rtlint.py --cases <entry.gr>   the lint's own conformance cases: each module
                                   says what it expects in `//! rtlint:` lines
                                   (`ok`, or a refusal's text), and every
                                   module must get exactly what it says
"""

import json
import os
import re
import subprocess
import sys

GRASM = os.environ.get("GRASM", "./target/release/grasm")
# A planted fault, for the controls in scripts/rtlint.sh: `store-atomic`
# takes a plain store for an atomic one.
FAULT = os.environ.get("RTLINT_FAULT", "")

# ----- TIR -------------------------------------------------------------------


def parse(text):
    """The TIR text as nested lists: an atom is a str, a string a (str,)."""
    out = []
    stack = [out]
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "(":
            stack.append([])
            i += 1
        elif c == ")":
            done = stack.pop()
            stack[-1].append(done)
            i += 1
        elif c == '"':
            j = i + 1
            while text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            stack[-1].append((json.loads(text[i : j + 1]),))
            i = j + 1
        elif c.isspace():
            i += 1
        else:
            j = i
            while j < n and not text[j].isspace() and text[j] not in '()"':
                j += 1
            stack[-1].append(text[i:j])
            i = j
    return out


def s(x):
    return x[0] if isinstance(x, tuple) else x


def kids(node):
    return [k for k in node[1:] if isinstance(k, list)]


def span_of(node):
    for k in node[1:]:
        if isinstance(k, str) and k.startswith("@"):
            return int(k[1:].split(":")[0])
    return -1


NONATOMIC = re.compile(r"^(load|store)_(i8|i16|i32|i64|u8|u16|u32|u64|f64|addr)$")
STORES = re.compile(r"^(store_|atomic_store_|atomic_add_|atomic_cas_)")
READS = re.compile(r"^(load_|atomic_load_|atomic_add_|atomic_cas_)")
HANDING_BUILTINS = {"ctx_switch", "ctx_init", "mem_free"}
NAMES = {"processname", "mstatic", "tlsname", "fnname", "type", "order", "sig", "targs"}
LEAVES = {"int", "float", "bool", "unit", "const", "fnref", "constref", "externref"}
LOCK_TYPES = {"i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"}
NONE = frozenset()

# ----- the program -------------------------------------------------------------


class Program:
    def __init__(self, entry):
        r = subprocess.run([GRASM, "tir", entry], capture_output=True, text=True)
        if r.returncode != 0:
            raise SystemExit(f"rtlint: {entry} does not emit TIR:\n{r.stderr.strip()}")
        self.roots = parse(r.stdout)
        self.files = []  # (module, path, base, text)
        self.cells = {}  # qualified -> layout type
        self.fns = {}  # qualified -> node
        self.params = {}  # qualified -> [var id]
        self.exports = set()
        for root in self.roots:
            head = root[0]
            if head == "file":
                path = s(root[3])
                with open(path, encoding="utf-8") as f:
                    self.files.append((s(root[1]), path, int(root[4]), f.read()))
            elif head == "processlocal":
                self.cells[s(root[1])] = root[2]
            elif head == "fn":
                name = s(root[1])
                self.fns[name] = root
                ps = next(k for k in kids(root) if k[0] == "params")
                self.params[name] = [int(p[2]) for p in kids(ps)]
            elif head == "export":
                self.exports.add(s(root[1]))
        self.files.sort(key=lambda f: f[2])
        self.guard = {}  # cell -> lock cell
        self.holds = {}  # fn -> set of lock cells
        self.refusals = []  # (fn, where, kind, text)
        self.read_marks()

    def where(self, off):
        """`path:line` of a global source offset."""
        if off < 0:
            return "?"
        f = [x for x in self.files if x[2] <= off][-1]
        return f"{f[1]}:{f[3].count(chr(10), 0, off - f[2]) + 1}"

    def module_of(self, q):
        return q.rsplit("::", 1)[0]

    def read_marks(self):
        """The `/// rtlint:` doc comments above declarations."""
        for module, path, _, text in self.files:
            for m in re.finditer(
                r"((?:^[ \t]*///[^\n]*\n)+)[ \t]*(?:pub[ \t]+)?(?:export[ \t]+)?([a-z_]+)[ \t]+([A-Za-z_][A-Za-z0-9_]*)",
                text,
                re.M,
            ):
                doc, kind, name = m.group(1), m.group(2), m.group(3)
                q = f"{module}::{name}"
                line = text.count("\n", 0, m.start(2)) + 1
                at = f"{path}:{line}"
                for mark in re.findall(r"///[ \t]*rtlint:[ \t]*([^\n]*)", doc):
                    mark = mark.strip()
                    g = re.fullmatch(r"guarded by ([A-Za-z_][A-Za-z0-9_]*)", mark)
                    h = re.fullmatch(r"holds ([A-Za-z_][A-Za-z0-9_]*)", mark)
                    if kind == "process_local" and g:
                        lock = f"{module}::{g.group(1)}"
                        self.guard[q] = lock
                    elif kind == "fn" and h:
                        lock = f"{module}::{h.group(1)}"
                        self.holds.setdefault(q, set()).add(lock)
                    else:
                        self.refusals.append((q, at, "marker", f"`{mark}` is not a mark rtlint reads here"))
                        continue
                    if lock not in self.cells:
                        self.refusals.append((q, at, "marker", f"lock `{lock}` is not a process cell of the module"))
                    elif self.cells[lock] not in LOCK_TYPES:
                        self.refusals.append((q, at, "marker", f"lock `{lock}` is a `{self.cells[lock]}` cell, not an integer one"))

    # ----- taint ---------------------------------------------------------------

    def lint(self):
        self.ret = {f: NONE for f in self.fns}
        self.ptaint = {f: [NONE] * len(self.params[f]) for f in self.fns}
        for _ in range(64):
            self.changed = False
            for f in self.fns:
                self.function(f, report=False)
            if not self.changed:
                break
        else:
            raise SystemExit("rtlint: the taint did not settle")
        for f in self.fns:
            self.function(f, report=True)
        for f in sorted(self.exports):
            if self.ret.get(f):
                self.refusals.append((f, self.where(span_of(self.fns[f][-1])), "escapes",
                                      f"process cell {self.cells_text(self.ret[f])}'s address returned to C by an `export fn`"))

    def function(self, f, report):
        self.cur = f
        self.takes = set()
        env = {}
        for i, v in enumerate(self.params[f]):
            env[v] = self.ptaint[f][i]
        body = self.fns[f][-1]
        # Flow-insensitive: the body to a fixpoint over its own variables,
        # quietly, then once more saying what it finds.
        self.report = False
        for _ in range(64):
            before = dict(env)
            self.expr(body, env)
            if env == before:
                break
        self.report = report
        self.flow_ret(f, self.expr(body, env))

    def flow_ret(self, f, t):
        if not t <= self.ret[f]:
            self.ret[f] = self.ret[f] | t
            self.changed = True

    def refuse(self, node, kind, text):
        if self.report:
            self.refusals.append((self.cur, self.where(span_of(node)), kind, text))

    def held(self, cells):
        """The cells of `cells` this function may touch plainly."""
        h = self.holds.get(self.cur, set())
        return {c for c in cells if self.guard.get(c) in h}

    def bind(self, env, v, t):
        env[v] = env.get(v, NONE) | t

    def expr(self, node, env):
        t = self.value(node, env)
        # A `bool` is never an address.
        if len(node) > 1 and node[1] == "bool":
            return NONE
        return t

    def value(self, node, env):
        head = node[0]
        if head in NAMES or head in LEAVES:
            return NONE
        if head == "local":
            return env.get(int(node[-1]), NONE)
        if head == "let":
            self.bind(env, int(node[1]), self.expr(node[-1], env))
            return NONE
        if head == "lettuple":
            t = self.expr(node[-1], env)
            for v in self.binders(node[:-1], "var", 1):
                self.bind(env, v, t)
            return NONE
        if head == "for":
            # The loop's variable is derived from what it walks.
            ks = kids(node)
            var = next(k for k in ks if k[0] == "var")
            rest = [k for k in ks if k is not var]
            self.bind(env, int(var[1]), self.expr(rest[0], env))
            for k in rest[1:]:
                self.expr(k, env)
            return NONE
        if head == "match":
            # Whatever a pattern binds is derived from what is matched.
            ks = kids(node)
            t = self.expr(ks[0], env)
            got = NONE
            for arm in ks[1:]:
                parts = kids(arm)
                if not parts:
                    continue
                for v in self.binders(parts[0], "pbind", 2):
                    self.bind(env, v, t)
                for k in parts[1:]:
                    got = got | self.expr(k, env)
            return got
        if head == "assign":
            place, val = kids(node)[0], kids(node)[1]
            t = self.expr(val, env)
            root = place
            while root[0] != "local" and kids(root):
                root = kids(root)[0]
            if root[0] == "local":
                self.bind(env, int(root[1]), t)
            return NONE
        if head == "builtin":
            name = s(node[3])
            if name == "mem_process":
                return frozenset([s(kids(node)[0][1])])
            ts = [self.expr(k, env) for k in kids(node)]
            got = NONE.union(*ts)
            if name in HANDING_BUILTINS:
                bad = got - self.held(got)
                if bad:
                    self.refuse(node, "handed", f"process cell {self.cells_text(bad)}'s address handed to `{name}` outside a function holding its lock")
                return NONE
            return got
        if head == "method":
            name = s(node[3])
            args = [k for k in kids(node) if k[0] not in NAMES]
            recv = self.expr(args[0], env) if args else NONE
            rest = [self.expr(k, env) for k in args[1:]]
            plain = NONATOMIC.match(name) and not (FAULT == "store-atomic" and name.startswith("store_"))
            if plain:
                bad = recv - self.held(recv)
                if bad:
                    what = "load" if name.startswith("load_") else "store"
                    self.refuse(node, "plain", f"a plain {what} of process cell {self.cells_text(bad)} outside a function holding its lock")
            if STORES.match(name):
                stored = NONE.union(*rest)
                if stored:
                    self.refuse(node, "escapes", f"process cell {self.cells_text(stored)}'s address stored in memory, where the lint cannot follow it")
            if name.startswith("atomic_cas_"):
                self.takes |= recv
            if name == "call_c":
                hand = NONE.union(recv, *rest)
                bad = hand - self.held(hand)
                if bad:
                    self.refuse(node, "handed", f"process cell {self.cells_text(bad)}'s address handed to C outside a function holding its lock")
                return NONE
            if READS.match(name) or STORES.match(name):
                return NONE
            return NONE.union(recv, *rest)
        if head == "call":
            callee = s(node[3])
            ts = [self.expr(k, env) for k in kids(node) if k[0] not in NAMES]
            if callee in self.fns:
                pt = self.ptaint[callee]
                for i, t in enumerate(ts):
                    if i < len(pt) and not t <= pt[i]:
                        pt[i] = pt[i] | t
                        self.changed = True
                for lock in sorted(self.holds.get(callee, ())):
                    if lock not in self.holds.get(self.cur, set()) and lock not in self.takes:
                        self.refuse(node, "caller", f"`{callee}` holds `{lock}`, and `{self.cur}` neither holds nor takes it")
                return self.ret[callee]
            return NONE.union(*ts)
        if head == "callextern":
            callee = s(node[3])
            got = NONE.union(*[self.expr(k, env) for k in kids(node) if k[0] not in NAMES])
            bad = got - self.held(got)
            if bad:
                self.refuse(node, "handed", f"process cell {self.cells_text(bad)}'s address handed to C function `{callee}` outside a function holding its lock")
            return NONE
        if head == "return":
            ks = kids(node)
            if ks:
                self.flow_ret(self.cur, self.expr(ks[0], env))
            return NONE
        # Everything else: its value is computed from its parts.
        return NONE.union(*[self.expr(k, env) for k in kids(node)])

    def binders(self, node, head, at):
        """The variable ids of every `head` node under `node`, read at `at`."""
        out = []
        stack = [node]
        while stack:
            n = stack.pop()
            if isinstance(n, list):
                if n and n[0] == head and len(n) > at and isinstance(n[at], str) and n[at].lstrip("-").isdigit():
                    out.append(int(n[at]))
                stack.extend(k for k in n[1:] if isinstance(k, list))
        return out

    def cells_text(self, cells):
        return ", ".join(f"`{c}`" for c in sorted(cells))


def main():
    args = sys.argv[1:]
    cases = False
    if args and args[0] == "--cases":
        cases = True
        args = args[1:]
    if not args:
        print(__doc__)
        return 2
    total_cells = total_fns = total_refused = 0
    failed = False
    for entry in args:
        p = Program(entry)
        p.lint()
        total_cells += len(p.cells)
        total_fns += len(p.fns)
        total_refused += len(p.refusals)
        if not cases:
            for f, where, kind, text in sorted(p.refusals, key=lambda r: (r[1], r[0], r[3])):
                print(f"rtlint: {where}: {kind}: {text} (in `{f}`)")
            failed = failed or bool(p.refusals)
            continue
        # The cases: each module's refusals against what it says it expects.
        by_module = {}
        for f, _, kind, text in p.refusals:
            by_module.setdefault(p.module_of(f), []).append(f"{kind}: {text}")
        checked = agreed = 0
        for module, path, _, text in p.files:
            want = re.findall(r"^//! rtlint: (.*)$", text, re.M)
            if not want:
                continue
            checked += 1
            got = sorted(by_module.get(module, []))
            exp = [] if want == ["ok"] else want
            if len(got) == len(exp) and all(any(e in g for g in got) for e in exp):
                agreed += 1
            else:
                failed = True
                print(f"CASE {path}: expected {exp or 'no refusal'}, got {got or 'no refusal'}")
        print(f"cases: {checked}   as expected: {agreed}   refusals: {len(p.refusals)}")
        if checked == 0:
            print("no case says what it expects, so nothing was checked")
            failed = True
    if not cases:
        print(f"packages: {len(args)}   process cells: {total_cells}   functions: {total_fns}   refused: {total_refused}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
