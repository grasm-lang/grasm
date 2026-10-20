"""The oracle for packages/pyrandom: Python 3.13's random.Random. Each
input line is a seed and calls on one generator, `name,arg,...`; floats
are given and answered as the hex of their bits."""
import math
import random
import struct
import sys


def f(h):
    return struct.unpack("<d", struct.pack("<Q", int(h, 16)))[0]


def fh(x):
    return "%x" % struct.unpack("<Q", struct.pack("<d", x))[0]


def call(r, c):
    name, *a = c.split(",")
    if name == "random":
        return fh(r.random())
    if name == "bits":
        return str(r.getrandbits(int(a[0])))
    if name == "range":
        return str(r.randrange(int(a[0]), int(a[1]), int(a[2])))
    if name == "range1":
        return str(r.randrange(int(a[0])))
    if name == "int":
        return str(r.randint(int(a[0]), int(a[1])))
    if name == "choice":
        return str(r.choice(range(int(a[0]))))
    if name == "shuffle":
        xs = list(range(int(a[0])))
        r.shuffle(xs)
        return "/".join(map(str, xs))
    if name == "sample":
        return "/".join(map(str, r.sample(range(int(a[0])), int(a[1]))))
    if name == "choices":
        return "/".join(map(str, r.choices(range(int(a[0])), k=int(a[1]))))
    if name in ("choicesw", "choicescw"):
        w = [f(x) for x in a[0].split("/")] if a[0] else []
        kw = {"weights": w} if name == "choicesw" else {"cum_weights": w}
        return "/".join(map(str, r.choices(range(len(w)), k=int(a[1]), **kw)))
    if name == "bytes":
        return r.randbytes(int(a[0])).hex() or "-"
    if name == "binomial":
        return str(r.binomialvariate(int(a[0]), f(a[1])))
    if name == "lgamma":
        return fh(math.lgamma(f(a[0])))
    if name == "triangular":
        return fh(r.triangular(f(a[0]), f(a[1]), None if a[2] == "~" else f(a[2])))
    fn = {"uniform": r.uniform, "normal": r.normalvariate, "gauss": r.gauss, "lognormal": r.lognormvariate, "expo": r.expovariate,
          "vonmises": r.vonmisesvariate, "gamma": r.gammavariate, "beta": r.betavariate, "pareto": r.paretovariate,
          "weibull": r.weibullvariate}[name]
    return fh(fn(*[f(x) for x in a]))


out = []
for line in sys.stdin.read().splitlines():
    if not line:
        continue
    seed, *calls = line.split(" ")
    kind, val = seed.split(":", 1)
    raw = b"" if val == "-" else bytes.fromhex(val) if kind != "i" else b""
    r = random.Random(int(val) if kind == "i" else raw.decode() if kind == "s" else raw)
    res = []
    for c in calls:
        try:
            res.append(call(r, c))
        except (ValueError, IndexError) as e:
            res.append("E" + str(e).encode().hex())
    out.append(" ".join(res))
print("\n".join(out))
