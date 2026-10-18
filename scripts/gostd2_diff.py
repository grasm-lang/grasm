"""More of Go's standard library, against Go, case for case.

`gotemplate` against `text/template`: random templates built from every
construct -- trim markers, comments, variables, pipelines, every builtin,
`if`/`else if`/`range`/`with`/`break`/`continue`, `define`, `template` and
`block` -- run on JSON documents decoded as Go decodes them, plus the same
templates damaged, so parse and execution errors are compared too.

`goprintf` against `fmt.Sprintf`, `Sprint`, `Sprintln` and
`strconv.FormatFloat`: random format strings built from every flag, width,
precision, `*` and `[n]` index form and verb, over operands of every kind
Go's `any` can hold here -- including the ones the verb does not suit, so
the `%!d(string=x)` forms are compared too. Floats are sent as bit patterns,
so both sides format exactly the same number.
"""
import json
import math
import re
import os
import random
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
rng = random.Random(20261018)
work = tempfile.mkdtemp()
harness = os.path.join(work, "gostdtest2")
oracle = os.path.join(work, "gostdoracle")

built = subprocess.run(["go", "build", "-o", oracle, "."],
                       cwd=os.path.join(ROOT, "scripts/lib/gostd"),
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
built = subprocess.run([GRASM, "build", "packages/gostdtest2/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
ENV = dict(os.environ, TZ="UTC")


def ask_go(cases):
    feed = "".join(f"{op} x{a.hex()} x{b.hex()}\n" for op, a, b in cases)
    ran = subprocess.run([oracle], input=feed, capture_output=True, text=True, env=ENV)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    return ran.stdout.split("\n")[:-1]


def ask_grasm(cases):
    argv = []
    for op, a, b in cases:
        argv += [op, "x" + a.hex(), "x" + b.hex()]
    lines = []
    for start in range(0, len(argv), 3 * 1000):
        ran = subprocess.run([harness] + argv[start:start + 3000],
                             capture_output=True, text=True)
        if ran.returncode != 0:
            # Find the one case that brought the harness down, so the
            # report names it rather than a chunk of a thousand.
            for k in range(start, min(start + 3000, len(argv)), 3):
                one = subprocess.run([harness] + argv[k:k + 3], capture_output=True, text=True)
                if one.returncode != 0:
                    op, a, b = cases[k // 3]
                    print(f"the harness crashed on {op} {a[:300]!r} {b[:200]!r}")
                    saved = os.path.join(work, "crash.json")
                    json.dump([op, a.hex(), b.hex()], open(saved, "w"))
                    print(f"the whole case is in {saved}")
                    print(one.stderr[-1000:])
                    break
            sys.exit(1)
        lines += ran.stdout.split("\n")[:-1]
    return lines


def bits(f):
    return struct.unpack(">Q", struct.pack(">d", f))[0]


FLOATS = [0.0, -0.0, 1.0, -1.0, 0.5, 1e6, 999999.0, 1e21, 1e20, 123456789.0,
          1e-4, 1e-5, 0.1, 0.125, 2.5, 3.5, 1.005, 9.999, 99.5, 5e-324,
          2.2250738585072014e-308, 1.7976931348623157e308, math.pi,
          float("inf"), float("-inf"), float("nan"), 100.0, 1e100, 0.000123]
STRINGS = ["", "a", "hello", "héllo", "日本語", "tab\tnl\n", "q\"uo'te", "`bt`",
           "\x00\x7f", " ", "﻿", "🙂", "100%", "  pad  "]


def rfloat():
    c = rng.random()
    if c < 0.4:
        return rng.choice(FLOATS)
    if c < 0.7:
        return struct.unpack(">d", struct.pack(">Q", rng.getrandbits(64)))[0]
    return rng.choice([1, -1]) * rng.random() * 10 ** rng.randrange(-8, 25)


def value(depth=0):
    kind = rng.choice("iiiuffffsssbtnlm" if depth < 2 else "iufsbtn")
    if kind == "i":
        v = rng.choice([0, 1, -1, 42, -42, 255, 1 << 31, -(1 << 63), (1 << 63) - 1,
                        rng.randrange(-10**6, 10**6), 0x1F600, 65, 0xD800, 0x110000])
        return ["i", str(v)]
    if kind == "u":
        return ["u", str(rng.choice([0, 7, 255, (1 << 64) - 1, 1 << 63,
                                     rng.getrandbits(64)]))]
    if kind == "f":
        return ["f", f"{bits(rfloat()):016x}"]
    if kind == "s":
        return ["s", rng.choice(STRINGS)]
    if kind == "b":
        return ["b", bytes(rng.randrange(0, 128) for _ in range(rng.randrange(0, 5))).hex()]
    if kind == "t":
        return ["t", rng.random() < 0.5]
    if kind == "n":
        return ["n"]
    if kind == "l":
        return ["l", [value(depth + 1) for _ in range(rng.randrange(0, 4))]]
    keys = rng.sample(["a", "b", "zz", "", "é", "10", "9"], rng.randrange(0, 4))
    return ["m", [[k, value(depth + 1)] for k in keys]]


VERBS = list("vdbcoOqxXUeEfFgGsTtw%") + ["é", "!", "z", "y"]


def directive():
    out = "%"
    for flag in "#0+- ":
        if rng.random() < 0.15:
            out += flag
    r = rng.random()
    if r < 0.08:
        out += f"[{rng.randrange(0, 5)}]"
    r = rng.random()
    if r < 0.3:
        out += str(rng.randrange(0, 15))
    elif r < 0.37:
        out += "*"
    elif r < 0.4:
        out += f"[{rng.randrange(1, 4)}]*"
    r = rng.random()
    if r < 0.3:
        out += "." + str(rng.randrange(0, 12))
    elif r < 0.35:
        out += ".*"
    elif r < 0.38:
        out += "."
    if rng.random() < 0.05:
        out += f"[{rng.randrange(1, 4)}]"
    if rng.random() < 0.02:
        return out
    return out + rng.choice(VERBS)


def format_string():
    parts = []
    for _ in range(rng.randrange(1, 4)):
        if rng.random() < 0.4:
            parts.append(rng.choice(["x", " ", "=", "é", "[", "]"]))
        parts.append(directive())
    return "".join(parts)


cases = []
for _ in range(20000):
    args = [value() for _ in range(rng.randrange(0, 5))]
    # Operands that are integers make `*` widths meaningful.
    if rng.random() < 0.3:
        args.insert(0, ["i", str(rng.randrange(-20, 20))])
    cases.append(("gpf", format_string().encode(), json.dumps(args).encode()))
for verb in VERBS:
    for v in [value() for _ in range(80)]:
        cases.append(("gpf", ("%" + verb).encode(), json.dumps([v]).encode()))
for _ in range(2000):
    args = [value() for _ in range(rng.randrange(0, 5))]
    cases.append(("gps", b"", json.dumps(args).encode()))
    cases.append(("gpl", b"", json.dumps(args).encode()))
for _ in range(4000):
    f = rfloat()
    for verb in "beEfgGxX":
        prec = rng.choice([-1, -1, 0, 1, 2, 3, 5, 6, 10, 15, 17, 18, 19, 25])
        cases.append(("gff", f"{bits(f):016x},{verb},{prec}".encode(), b""))


# --- gotemplate ---------------------------------------------------------
DOCS = [
    {"a": 1, "b": "x", "s": "a", "n": 1000000, "f": 2.5, "t": True, "z": 0, "e": "",
     "nul": None, "neg": -3, "big": 1e21,
     "l": [1, "two", None, [3], {"k": "v"}], "m": {"x": 1, "y": "why", "n": None},
     "lm": [{"name": "a", "v": 1}, {"name": "b", "v": 2}], "emp": [], "em": {},
     "html": "<a href='x'>&\"</a>", "uni": "héllo 日本 🙂", "nums": [3, 1, 2]},
    None, [1, 2, 3], "just a string", {"x": {"y": {"z": "deep"}}},
]
FIELDS = [".", ".a", ".b", ".s", ".n", ".f", ".t", ".z", ".e", ".nul", ".neg", ".big", ".l",
          ".m", ".m.x", ".m.y", ".m.n", ".m.missing", ".missing", ".missing.x", ".nul.x",
          ".lm", ".emp", ".em", ".html", ".uni", ".nums", ".x.y.z", ".a.b", ".l.x"]
LITS = ["1", "0", "-2", "3.5", "1e3", "0x10", "'a'", "1.0", "\"str\"", "`raw`", "true",
        "false", "nil", "\"\"", "\"<&>\"", "07", "1_000", "2"]
FUNCS = ["and", "or", "not", "len", "index", "slice", "print", "printf", "println", "html",
         "js", "urlquery", "eq", "ne", "lt", "le", "gt", "ge", "call"]


class Gen:
    def __init__(self, safe=False):
        self.safe = safe
        self.moved = False
        self.vars = ["$"]
        self.in_range = 0
        self.depth = 0
        self.defines = []
        self.in_define = False

    def operand(self):
        r = rng.random()
        if r < 0.4:
            return rng.choice(FIELDS)
        if r < 0.7:
            return rng.choice(LITS)
        if r < 0.85:
            v = rng.choice(self.vars)
            return v + rng.choice(["", "", ".a", ".x", ".m.x"]) if v == "$" else v
        if self.depth < 3:
            self.depth += 1
            p = "(" + self.pipeline(decl=False) + ")"
            self.depth -= 1
            return p
        return rng.choice(LITS)

    SAFE = ["len .l", "len .m", "len .s", "len .uni", "index .l 1", "index .l 0", "index .m \"x\"",
            "index .lm 1 \"name\"", "index .nums 2", "slice .l 1 3", "slice .s 0 1",
            "slice .uni 1", "eq .s \"a\"", "eq .a 1.0", "eq .t true", "ne .b \"y\"", "lt .neg 0.0",
            "le .f 2.5", "gt .n 1e5", "ge .z 0.0", "eq .s \"x\" \"a\"", "not .t", "not .e",
            "and .a .b", "or .e .z .s", "and .t .missing", "or .nul .b", "print .a .b .s",
            "print .l .m", "println .f .n", "printf \"%v|%5.1f|%s\" .a .f .s",
            "printf \"%d\" (len .l)", "printf \"%x\" .b", "printf \"%q\" .uni", "html .html",
            "js .html", "urlquery .uni .b", "html .a .nul", "len (slice .nums 1)", "print (index .l 3)",
            "eq (len .l) 5", "index $.l 4 \"k\""]

    def anchored(self, text):
        """Inside `range` and `with` the dot moves; reach the data by `$`."""
        if not self.moved:
            return text
        return re.sub(r'(?<![\w$)\]])\.(?=[a-z])', '$.', text)

    def command(self):
        if self.safe and rng.random() < 0.5:
            return self.anchored(rng.choice(self.SAFE))
        if self.safe:
            return self.anchored(rng.choice(FIELDS[:20] + ["$", "1", "\"lit\"", "true", "3.25"]))
        if rng.random() < 0.5:
            f = rng.choice(FUNCS)
            if f == "printf":
                fmt = rng.choice(["\"%v\"", "\"%d|%s\"", "\"%05.2f\"", "\"%q\"", "\"%T\""])
                return f + " " + fmt + "".join(" " + self.operand() for _ in range(rng.randrange(0, 3)))
            return f + "".join(" " + self.operand() for _ in range(rng.randrange(0, 4)))
        return self.operand()

    def pipeline(self, decl=True):
        out = ""
        if decl and rng.random() < 0.15:
            name = rng.choice(["$x", "$y", "$i"])
            if name in self.vars and rng.random() < 0.5:
                out = name + " = "
            else:
                out = name + " := "
                self.vars.append(name)
        cmds = [self.command()]
        while rng.random() < 0.25:
            f = rng.choice(["print", "html", "not", "printf \"%v!\"", "urlquery", "js",
                            "and 1", "or 0"] if self.safe else
                           ["print", "len", "html", "not", "printf \"%v!\"", "index .l",
                            "eq 1", "urlquery", "js", "and 1", "or 0"])
            cmds.append(f)
        return out + " | ".join(cmds)

    def open(self):
        return rng.choice(["{{", "{{", "{{- ", "{{ "])

    def close(self):
        return rng.choice(["}}", "}}", " -}}", " }}"])

    def body(self, n=None):
        n = rng.randrange(0, 4) if n is None else n
        return "".join(self.item() for _ in range(n))

    def item(self):
        r = rng.random()
        if r < 0.25:
            return rng.choice(["a", " ", "\n", "  x  ", "|", "\t"])
        if r < 0.55 or self.depth >= 3:
            return self.open() + self.pipeline() + self.close()
        if r < 0.6:
            # A comment must touch its delimiters: `{{ /*` is an error.
            return rng.choice(["{{", "{{- "]) + "/* c */" + rng.choice(["}}", " -}}"])
        saved = list(self.vars)
        self.depth += 1
        kind = rng.choice(["if", "if", "range", "range", "with", "template", "break"])
        if kind == "if":
            t = "{{if " + self.pipeline() + "}}" + self.body()
            if rng.random() < 0.3:
                t += "{{else if " + self.pipeline() + "}}" + self.body()
            if rng.random() < 0.5:
                t += "{{else}}" + self.body()
            t += "{{end}}"
        elif kind == "range":
            head = rng.choice(["", "$v := ", "$k, $v := "])
            if "$v" in head:
                self.vars.append("$v")
            if "$k" in head:
                self.vars.append("$k")
            target = rng.choice([".l", ".m", ".lm", ".emp", ".em", ".nums", "3", ".missing", "0"]
                                if self.safe else
                                [".l", ".m", ".lm", ".emp", ".em", ".nums", ".", "3", ".s",
                                 ".missing", ".nul", "0", ".f"])
            self.in_range += 1
            was = self.moved
            target = self.anchored(target)
            self.moved = True
            t = "{{range " + head + target + "}}" + self.body()
            self.moved = was
            self.in_range -= 1
            if rng.random() < 0.3:
                t += "{{else}}" + self.body()
            t += "{{end}}"
        elif kind == "with":
            head = self.pipeline()
            was = self.moved
            self.moved = True
            t = "{{with " + head + "}}" + self.body()
            self.moved = was
            if rng.random() < 0.4:
                t += "{{else}}" + self.body()
            t += "{{end}}"
        elif kind == "template" and self.defines and not self.in_define:
            t = "{{template \"" + rng.choice(self.defines) + "\" " + self.operand() + "}}"
        elif self.in_range:
            t = rng.choice(["{{break}}", "{{continue}}"])
        else:
            t = "{{.}}"
        self.vars = saved
        self.depth -= 1
        return t

    def template(self):
        out = ""
        for name in ["d1", "d2"][:rng.randrange(0, 3)]:
            g = Gen(self.safe)
            g.in_define = True
            out += "{{define \"" + name + "\"}}" + g.body(rng.randrange(1, 4)) + "{{end}}"
            self.defines.append(name)
        if rng.random() < 0.15:
            g = Gen(self.safe)
            g.in_define = True
            out += "{{block \"bl\" .}}" + g.body(2) + "{{end}}"
        return out + self.body(rng.randrange(1, 6))


templates = []
for k in range(9000):
    templates.append(Gen(safe=k % 2 == 0).template())
for t in list(templates[:3000]):
    m = list(t)
    for _ in range(rng.randrange(1, 3)):
        at = rng.randrange(len(m) + 1)
        if rng.random() < 0.5 and m:
            del m[min(at, len(m) - 1)]
        else:
            m.insert(at, rng.choice(["{{", "}}", "end", "$", ".", "(", ")", "|", "\"", "-",
                                     "{{else}}", "{{end}}", " "]))
    templates.append("".join(m))
for k, t in enumerate(templates):
    # The well-typed templates are written for the first document.
    doc = DOCS[0] if k < 9000 and k % 2 == 0 and rng.random() < 0.85 else rng.choice(DOCS)
    cases.append(("tpl", t.encode(), json.dumps(doc).encode()))

expected = ask_go(cases)
got = ask_grasm(cases)


def readable(text):
    try:
        return bytes.fromhex(text).decode("utf-8", "replace")
    except Exception:
        return text


families = {}
agree = differ = 0
for i, (op, a, b) in enumerate(cases):
    want, mine = expected[i], (got[i] if i < len(got) else "<missing>")
    fam = families.setdefault(op, [0, 0])
    if want == mine:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if differ <= 12:
            print(f"  {op} {a.decode('utf-8', 'replace')!r} {b.decode()[:90]}")
            print(f"    go     {readable(want)!r}")
            print(f"    grasm  {readable(mine)!r}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 4:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
# A template both sides refuse agrees trivially. Enough of them must render
# for the comparison to be about output, not about errors.
rendered = sum(1 for (op, _, _), want in zip(cases, expected) if op == "tpl" and want != "ERR")
refused = sum(1 for (op, _, _), want in zip(cases, expected) if op == "tpl" and want == "ERR")
print(f"templates rendered: {rendered}   refused: {refused}")
if os.environ.get("SHOW_REFUSED"):
    shown_r = 0
    for k, ((op, a, b), want) in enumerate(zip(cases, expected)):
        if op == "tpl" and want == "ERR" and shown_r < 12:
            t = a.decode()
            if k % 2 == 0:
                shown_r += 1
                print("REFUSED", repr(t[:220]))
if rendered < refused // 2:
    print("too few templates render for the comparison to mean much")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
