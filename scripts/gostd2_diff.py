"""More of Go's standard library, against Go, case for case.

`goprintf` against `fmt.Sprintf`, `Sprint`, `Sprintln` and
`strconv.FormatFloat`: random format strings built from every flag, width,
precision, `*` and `[n]` index form and verb, over operands of every kind
Go's `any` can hold here -- including the ones the verb does not suit, so
the `%!d(string=x)` forms are compared too. Floats are sent as bit patterns,
so both sides format exactly the same number.
"""
import json
import math
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
            print(ran.stderr[-2000:]); sys.exit(1)
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
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
